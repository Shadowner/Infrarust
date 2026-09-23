#!/usr/bin/env node

import assert from 'node:assert/strict';
import { createHmac } from 'node:crypto';

import { startBackend } from './backend-mock.js';
import { connectOnce } from './client.js';
import { evaluate } from './expectations.js';
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
  const scenario = SCENARIOS.find((s) => s.id === 'offline-bungeecord');
  const { status, checks } = evaluate(scenario, {
    client: { reachedPlay: true },
    backend: { handshakeHost: 'off-bungee.test' },
    proxyPlayer: { is_active: true },
  });
  assert.equal(status, 'known');
  assert.ok(checks.some((c) => c.known === 'bungee-forwarding-is-a-noop-in-offline-mode'));
});

await check('expectations: a known finding that stops reproducing is reported as fixed', () => {
  const scenario = SCENARIOS.find((s) => s.id === 'offline-bungeecord');
  const injected = ['off-bungee.test', '127.0.0.1', 'abc', '[{"name":"bungeeguard-token"}]'].join('\u0000');
  const { status } = evaluate(scenario, {
    client: { reachedPlay: true },
    backend: { handshakeHost: injected },
    proxyPlayer: { is_active: true },
  });
  assert.equal(status, 'fixed');
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
