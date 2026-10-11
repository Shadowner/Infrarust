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
};

export function evaluate(scenario, observations, { backendObservable = true } = {}) {
  const checks = [];
  let failed = false;
  let known = false;
  let fixed = false;

  const BACKEND_CHECKS = new Set(['velocitySigned', 'bungeeInjected', 'bungeeGuardToken']);

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
        checks.push({ name, ok: true, detail: result.detail });
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
