#!/usr/bin/env node

import assert from 'node:assert/strict';
import { createHmac } from 'node:crypto';

import { startBackend } from './backend-mock.js';
import { connectOnce } from './client.js';
import { evaluate } from './expectations.js';
import { BACKEND_COMPRESSION_THRESHOLD, COMPRESSION_THRESHOLD } from './lab.js';
import { BIG_PACKET_CHANNEL, LOGIN_QUERIES } from './modded.js';
import { SCENARIOS } from './scenarios.js';
import { verifyAndDecode, writeVarInt } from './velocity.js';

const SECRET = 'e2eSecretForVelocityForwarding01';
const results = [];

function check(name, fn) {
  return Promise.resolve()
    .then(fn)
    .then(
      () => results.push({ name, ok: true }),
      (err) => results.push({ name, ok: false, error: err.message }),
    );
}

function velocityPayload({ version = 4, ip = '10.0.0.1', uuid = '069a79f444e94726a5befca90e38aaf5', username = 'Notch' } = {}) {
  const str = (s) => {
    const b = Buffer.from(s, 'utf8');
    return Buffer.concat([writeVarInt(b.length), b]);
  };
  return Buffer.concat([writeVarInt(version), str(ip), Buffer.from(uuid, 'hex'), str(username), writeVarInt(0)]);
}

function signed(payload, secret) {
  return Buffer.concat([createHmac('sha256', secret).update(payload).digest(), payload]);
}

await check('velocity: a correctly signed payload verifies and decodes', () => {
  const r = verifyAndDecode(signed(velocityPayload(), SECRET), SECRET);
  assert.equal(r.ok, true, r.error);
  assert.equal(r.decoded.version, 4);
  assert.equal(r.decoded.username, 'Notch');
  assert.equal(r.decoded.ip, '10.0.0.1');
  assert.equal(r.decoded.trailingBytes, 0);
});

await check('velocity: a payload signed with the wrong secret is rejected', () => {
  const r = verifyAndDecode(signed(velocityPayload(), 'not-the-right-secret'), SECRET);
  assert.equal(r.ok, false);
  assert.match(r.error, /HMAC mismatch/);
});

await check('velocity: a tampered payload is rejected', () => {
  const buf = signed(velocityPayload(), SECRET);
  buf[40] ^= 0xff;
  const r = verifyAndDecode(buf, SECRET);
  assert.equal(r.ok, false);
  assert.match(r.error, /HMAC mismatch/);
});

await check('velocity: an unsigned payload is rejected', () => {
  const r = verifyAndDecode(velocityPayload(), SECRET);
  assert.equal(r.ok, false);
});

await check('expectations: a scenario fails when the client never reaches play', () => {
  const scenario = SCENARIOS.find((s) => s.id === 'passthrough');
  const { status } = evaluate(scenario, {
    client: { reachedPlay: false, error: 'boom' },
    backend: {},
    proxyPlayer: { is_active: false },
  });
  assert.equal(status, 'fail');
});

await check('expectations: a broken admin API is not reported as an absent player', () => {
  const scenario = SCENARIOS.find((s) => s.id === 'passthrough');
  const { checks } = evaluate(scenario, {
    client: { reachedPlay: true, onPlayError: 'returned 429' },
    backend: {},
    proxyPlayer: null,
  });
  const c = checks.find((x) => x.name === 'interceptedByProxy');
  assert.match(c.detail, /could not read the proxy/);
});

await check('expectations: a known finding downgrades a failure but does not hide it', () => {
  const scenario = SCENARIOS.find((s) => s.id === 'full-fallback');
  const { status, checks } = evaluate(scenario, {
    client: { reachedPlay: false, error: 'connection ended before reaching play' },
    backend: null,
    proxyPlayer: null,
  });
  assert.equal(status, 'known');
  assert.ok(checks.some((c) => c.known === 'full-mode-is-refused-at-validation'));
});

await check('expectations: a known finding that stops reproducing is reported as fixed', () => {
  const scenario = SCENARIOS.find((s) => s.id === 'full-fallback');
  const { status } = evaluate(scenario, {
    client: { reachedPlay: true },
    backend: {},
    proxyPlayer: { is_active: false },
  });
  assert.equal(status, 'fixed');
});

const only = (name, expected = true) => ({ id: `selftest-${name}`, expect: { [name]: expected } });
const relayedQueries = () => ({
  client: {
    loginQueries: LOGIN_QUERIES.map((q) => ({ messageId: q.messageId, channel: q.channel, data: q.request.toString('hex'), at: 1 })),
    loginSuccessAt: 3,
  },
  backend: {
    loginQueries: {
      complete: true,
      responses: LOGIN_QUERIES.map((q) => ({ messageId: q.messageId, data: q.answer ? q.answer.toString('hex') : null })),
    },
    loginSuccessSentAt: 2,
  },
});

await check('expectations: login queries relayed unchanged pass', () => {
  assert.equal(evaluate(only('loginQueriesRelayed'), relayedQueries()).status, 'pass');
  assert.equal(evaluate(only('loginSuccessAfterBackend'), relayedQueries()).status, 'pass');
});

await check('expectations: login queries the proxy answers itself are caught', () => {
  const obs = relayedQueries();
  obs.client.loginQueries = [];
  obs.backend.loginQueries.responses = LOGIN_QUERIES.map((q) => ({ messageId: q.messageId, data: null }));
  const { status, checks } = evaluate(only('loginQueriesRelayed'), obs);
  assert.equal(status, 'fail');
  assert.match(checks[0].detail, /client saw .* 0 times/);
});

await check('expectations: a renumbered login query is caught', () => {
  const obs = relayedQueries();
  obs.client.loginQueries[1].messageId = 0;
  assert.equal(evaluate(only('loginQueriesRelayed'), obs).status, 'fail');
});

await check('expectations: a LoginSuccess the proxy sends before the backend is caught', () => {
  const obs = relayedQueries();
  obs.client.loginSuccessAt = 1;
  assert.equal(evaluate(only('loginSuccessAfterBackend'), obs).status, 'fail');
});

await check("expectations: the backend's threshold on a login the proxy finishes is caught", () => {
  const client = { loginPackets: ['compress', 'success'], setCompression: BACKEND_COMPRESSION_THRESHOLD };
  assert.equal(evaluate(only('clientCompression', 'proxy'), { client, protocol: 769 }).status, 'fail');
  client.setCompression = COMPRESSION_THRESHOLD;
  assert.equal(evaluate(only('clientCompression', 'proxy'), { client, protocol: 769 }).status, 'pass');
  client.loginPackets = ['success', 'compress'];
  assert.equal(evaluate(only('clientCompression', 'proxy'), { client, protocol: 769 }).status, 'fail');
  assert.equal(evaluate(only('clientCompression', 'proxy'), { client: { loginPackets: ['success'] }, protocol: 769 }).status, 'fail');
});

await check('expectations: a forge marker lost by a rewrite or by bungeecord forwarding is caught', () => {
  const client = { domain: 'forge.test\u0000FML3\u0000' };
  const rewrite = only('forgeMarker', { rewrite: 'backend.internal' });
  assert.equal(evaluate(rewrite, { client, backend: { handshakeHost: 'backend.internal\u0000FML3\u0000' } }).status, 'pass');
  assert.equal(evaluate(rewrite, { client, backend: { handshakeHost: 'backend.internal' } }).status, 'fail');
  const props = only('forgeMarker', { properties: true });
  const forwarded = (properties) => ({ client, backend: { handshakeHost: ['forge.test', '127.0.0.1', 'abc', JSON.stringify(properties)].join('\u0000') } });
  const good = [{ name: 'forgeClient', value: 'true' }, { name: 'extraData', value: '\u0001FML3\u0001', signature: '' }];
  assert.equal(evaluate(props, forwarded(good)).status, 'pass');
  assert.equal(evaluate(props, forwarded([])).status, 'fail');
  assert.equal(evaluate(props, forwarded([good[0], { name: 'extraData', value: '\u0000FML3\u0000' }])).status, 'fail');
});

await check('loopback: the query backend and client agree with no proxy involved', async () => {
  const port = 26120;
  const backend = await startBackend({ version: '1.20.1', port, kind: 'query', loginQueries: true, secret: SECRET });
  try {
    const client = await connectOnce({ port, version: '1.20.1', domain: 'selftest.local', username: 'SelfTest', answerLoginQueries: true });
    assert.equal(client.reachedPlay, true, `did not reach play: ${client.error ?? client.kick}`);
    const { status, checks } = evaluate(
      { id: 'selftest-loopback', expect: { loginQueriesRelayed: true, loginSuccessAfterBackend: true } },
      { client, backend: backend.observation() },
    );
    assert.equal(status, 'pass', checks.map((c) => c.detail).join('; '));
  } finally {
    await backend.close();
  }
});

await check('loopback: the big packet reaches a client with no proxy involved', async () => {
  const port = 26121;
  const backend = await startBackend({ version: '1.21.4', port, kind: 'big-packet', bigPacket: true, secret: SECRET });
  try {
    const client = await connectOnce({ port, version: '1.21.4', domain: 'selftest.local', username: 'SelfTest', awaitPayload: BIG_PACKET_CHANNEL });
    const { status, checks } = evaluate(only('bigPacketIntact'), { client, backend: backend.observation() });
    assert.equal(status, 'pass', checks.map((c) => c.detail).join('; '));
  } finally {
    await backend.close();
  }
});

let selfPort = 26100;
for (const version of ['1.7', '1.12.2', '1.20.2', '26.1']) {
  const port = selfPort++;
  await check(`loopback: ${version} reaches play with no proxy involved`, async () => {
    const backend = await startBackend({ version, port, kind: 'plain', secret: SECRET });
    try {
      const obs = await connectOnce({ port, version, domain: 'selftest.local', username: 'SelfTest' });
      assert.equal(obs.reachedPlay, true, `did not reach play: ${obs.error ?? obs.kick}`);
      assert.equal(backend.observation()?.handshakeHost, 'selftest.local', 'fakeHost did not reach the backend');
    } finally {
      await backend.close();
    }
  });
}

let failed = 0;
for (const r of results) {
  console.log(`${r.ok ? 'ok  ' : 'FAIL'} ${r.name}${r.ok ? '' : `\n     ${r.error}`}`);
  if (!r.ok) failed++;
}
console.log(`\n${results.length - failed}/${results.length} harness checks passed`);
process.exit(failed ? 1 : 0);
