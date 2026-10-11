import { BACKEND_COMPRESSION_THRESHOLD, COMPRESSION_THRESHOLD } from './lab.js';
import { LOGIN_QUERIES } from './modded.js';

const COMPRESSION_MIN_PROTOCOL = 47;

export const KNOWN_FINDINGS = [
  {
    id: 'full-mode-is-refused-at-validation',
    scenarios: ['full-fallback'],
    relaxes: ['reachesPlay', 'interceptedByProxy'],
    summary: 'proxy_mode = "full" no longer falls back to passthrough: the server file is refused when the proxy loads it.',
    detail:
      'Since 60329025 (fix(config): reject the reserved full proxy mode at validation) the file provider logs ' +
      '"skipping invalid config ... proxy_mode = \\"full\\" is reserved and not implemented", so full.test is an ' +
      'unknown domain and domain_router disconnects the client. This is intended behaviour that postdates the ' +
      'scenario, which still asserts the old fallback; it should become a refusal check or be dropped.',
  },
  {
    id: 'velocity-on-passthrough-is-refused-at-validation',
    scenarios: ['passthrough-velocity-fallback'],
    relaxes: ['reachesPlay', 'interceptedByProxy', 'bungeeInjected'],
    summary:
      'forwarding_mode = "velocity" on a passthrough server no longer falls back to BungeeCord: the server file is refused when the proxy loads it.',
    detail:
      'Since 79e333cd (fix(config): reject velocity forwarding on forwarding-mode servers), now ' +
      'ConfigError::VelocityOnForwardingServer, the file provider skips the server ("Passthrough mode does not ' +
      'parse the login, but ... is velocity"), so pt-velo.test is an unknown domain and the client is ' +
      'disconnected before any backend is contacted. Intended behaviour that postdates the scenario, which still ' +
      'asserts the documented fallback to BungeeCord.',
  },
];

function findingFor(scenarioId, check, { backendObservable }) {
  return KNOWN_FINDINGS.find(
    (f) =>
      f.scenarios.includes(scenarioId) &&
      (f.relaxes.includes(check) || (!backendObservable && f.relaxesWithRealBackend?.includes(check))),
  );
}

const CHECKS = {
  reachesPlay: ({ client }) => ({
    ok: client.reachedPlay === true,
    detail: client.reachedPlay ? 'reached play' : `did not reach play (${client.error ?? client.kick ?? 'no reason'})`,
  }),

  interceptedByProxy: ({ proxyPlayer, client }, expected) => {
    if (client.onPlayError) {
      return { ok: false, detail: `could not read the proxy's view: ${client.onPlayError}` };
    }
    const actual = proxyPlayer?.is_active;
    return {
      ok: actual === expected,
      detail: proxyPlayer ? `admin API is_active=${actual}` : 'proxy reported no player',
    };
  },

  encrypted: ({ client }) => ({
    ok: client.encrypted === true,
    detail: client.encrypted ? 'client<->proxy leg encrypted' : 'no encryption negotiated',
  }),

  velocitySigned: ({ backend, client }) => {
    const v = backend?.velocity;
    if (!backend?.velocityRequested) return { ok: false, detail: 'backend never sent a velocity request' };
    if (!backend.velocityResponded) return { ok: false, detail: 'proxy never answered the velocity request' };
    if (!v?.ok) return { ok: false, detail: `payload rejected: ${v?.error ?? 'unknown'}` };

    const d = v.decoded;
    const problems = [];
    if (d.username !== client.username) problems.push(`username ${d.username} != ${client.username}`);
    if (!d.ip) problems.push('empty forwarded ip');
    if (!/^[0-9a-f-]{36}$/.test(d.uuid)) problems.push(`malformed uuid ${d.uuid}`);
    if (d.trailingBytes !== 0) problems.push(`${d.trailingBytes} unparsed trailing bytes`);
    return {
      ok: problems.length === 0,
      detail: problems.length ? problems.join('; ') : `v${d.version} signed, ip=${d.ip}, uuid=${d.uuid}`,
    };
  },

  bungeeInjected: ({ backend }) => {
    const host = backend?.handshakeHost ?? '';
    const fields = host.split('\0');
    return {
      ok: fields.length >= 3,
      detail:
        fields.length >= 3
          ? `handshake carries ${fields.length} fields (ip=${fields[1]}, uuid=${fields[2]})`
          : `handshake carries no injected identity (${JSON.stringify(host)})`,
    };
  },

  bungeeGuardToken: ({ backend }) => {
    const fields = (backend?.handshakeHost ?? '').split('\0');
    const props = fields[3] ?? '';
    return {
      ok: props.includes('bungeeguard-token'),
      detail: props.includes('bungeeguard-token') ? 'bungeeguard-token present' : 'no bungeeguard-token property',
    };
  },

  clientCompression: ({ client, protocol }, source) => {
    const login = client.loginPackets ?? [];
    const sequence = login.join('>') || 'nothing';
    if (protocol < COMPRESSION_MIN_PROTOCOL) {
      return {
        ok: client.setCompression == null,
        skipped: true,
        detail: client.setCompression == null ? 'no compression before 1.8' : `SetCompression(${client.setCompression}) sent to a pre-1.8 client`,
      };
    }
    const expected = source === 'proxy' ? COMPRESSION_THRESHOLD : BACKEND_COMPRESSION_THRESHOLD;
    const compressions = login.filter((name) => name === 'compress').length;
    const problems = [];
    if (compressions === 0) problems.push('no SetCompression during login');
    if (compressions > 1) problems.push(`${compressions} SetCompression packets`);
    if (compressions && login.includes('success') && login.indexOf('compress') > login.indexOf('success')) {
      problems.push('SetCompression after LoginSuccess');
    }
    if (compressions && client.setCompression !== expected) {
      problems.push(`threshold ${client.setCompression}, expected the ${source}'s ${expected}`);
    }
    return {
      ok: problems.length === 0,
      detail: problems.length ? `${problems.join('; ')} (login: ${sequence})` : `SetCompression(${expected}) from the ${source} before LoginSuccess (login: ${sequence})`,
    };
  },

  loginQueriesRelayed: ({ client, backend }) => {
    const sent = backend?.loginQueries;
    if (!sent) return { ok: false, detail: 'the backend never sent its login queries' };
    const problems = [];

    const seen = client.loginQueries ?? [];
    for (const query of LOGIN_QUERIES) {
      const got = seen.filter((r) => r.channel === query.channel);
      if (got.length !== 1) {
        problems.push(`client saw ${query.channel} ${got.length} times`);
        continue;
      }
      if (got[0].messageId !== query.messageId) problems.push(`${query.channel} reached the client as id ${got[0].messageId}, sent as ${query.messageId}`);
      if (got[0].data !== query.request.toString('hex')) problems.push(`${query.channel} payload changed on the way to the client (${got[0].data})`);
    }
    const foreign = seen.filter((r) => !LOGIN_QUERIES.some((q) => q.channel === r.channel));
    if (foreign.length) problems.push(`client also saw ${foreign.map((r) => r.channel).join(', ')}`);

    for (const query of LOGIN_QUERIES) {
      const got = sent.responses.filter((r) => r.messageId === query.messageId);
      const want = query.answer ? query.answer.toString('hex') : null;
      if (got.length !== 1) {
        problems.push(`backend got ${got.length} responses for id ${query.messageId}`);
        continue;
      }
      if (got[0].data !== want) problems.push(`id ${query.messageId} reached the backend as ${got[0].data ?? 'not understood'}, client sent ${want ?? 'not understood'}`);
    }
    const stray = sent.responses.filter((r) => !LOGIN_QUERIES.some((q) => q.messageId === r.messageId));
    if (stray.length) problems.push(`backend got responses for ids it never used: ${stray.map((r) => r.messageId).join(', ')}`);

    return {
      ok: problems.length === 0,
      detail: problems.length
        ? problems.join('; ')
        : `${LOGIN_QUERIES.length} queries (ids ${LOGIN_QUERIES.map((q) => q.messageId).join(', ')}) relayed and answered byte-for-byte`,
    };
  },

  loginSuccessAfterBackend: ({ client, backend }) => {
    if (client.loginSuccessAt == null) return { ok: false, detail: 'the client never received LoginSuccess' };
    if (backend?.loginSuccessSentAt == null) return { ok: false, detail: 'the client got LoginSuccess but the backend never sent its own' };
    const lag = client.loginSuccessAt - backend.loginSuccessSentAt;
    const lateQueries = (client.loginQueries ?? []).filter((q) => q.at > client.loginSuccessAt);
    if (lag < 0) return { ok: false, detail: `the client got LoginSuccess ${(-lag).toFixed(1)} ms before the backend sent its own` };
    if (lateQueries.length) return { ok: false, detail: `${lateQueries.length} login queries reached the client after its LoginSuccess` };
    return { ok: true, detail: `client LoginSuccess ${lag.toFixed(1)} ms after the backend's` };
  },
};

export function evaluate(scenario, observations, { backendObservable = true } = {}) {
  const checks = [];
  let failed = false;
  let known = false;
  let fixed = false;

  const BACKEND_CHECKS = new Set([
    'velocitySigned',
    'bungeeInjected',
    'bungeeGuardToken',
    'loginQueriesRelayed',
    'loginSuccessAfterBackend',
  ]);

  for (const [name, expected] of Object.entries(scenario.expect)) {
    if (!backendObservable && BACKEND_CHECKS.has(name)) {
      checks.push({ name, ok: true, skipped: true, detail: 'not observable against a real backend' });
      continue;
    }
    const check = CHECKS[name];
    if (!check) {
      checks.push({ name, ok: false, detail: `no such check: ${name}` });
      failed = true;
      continue;
    }
    const result = check(observations, expected);
    const finding = findingFor(scenario.id, name, { backendObservable });

    if (result.ok) {
      if (finding) {
        fixed = true;
        checks.push({ name, ok: true, detail: `${result.detail} — known finding "${finding.id}" no longer reproduces`, finding: finding.id });
      } else {
        checks.push({ name, ok: true, detail: result.detail, ...(result.skipped ? { skipped: true } : {}) });
      }
    } else if (finding) {
      known = true;
      checks.push({ name, ok: false, known: finding.id, detail: result.detail });
    } else {
      failed = true;
      checks.push({ name, ok: false, detail: result.detail });
    }
  }

  let status = 'pass';
  if (failed) status = 'fail';
  else if (fixed) status = 'fixed';
  else if (known) status = 'known';
  return { status, checks };
}
