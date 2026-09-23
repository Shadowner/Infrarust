export const KNOWN_FINDINGS = [
  {
    id: 'bungee-forwarding-is-a-noop-in-offline-mode',
    scenarios: ['offline-bungeecord', 'offline-bungeeguard'],
    relaxes: ['bungeeInjected', 'bungeeGuardToken'],
    relaxesWithRealBackend: ['reachesPlay', 'interceptedByProxy'],
    summary:
      'BungeeCord and BungeeGuard forwarding inject nothing into the handshake when proxy_mode = offline.',
    detail:
      'handler/intercepted/initial_connect.rs applies handler.apply_handshake() only when login_completed is true, ' +
      'and requires_proxy_completed_login() forces login completion for Velocity alone. In offline mode the proxy ' +
      'therefore sends a plain handshake, the backend never learns the real IP, and the operator still sees the ' +
      '"BungeeCord legacy mode is insecure" warning that suggests forwarding is active. The same config under ' +
      'passthrough injects all four fields, which is what makes this a silent inconsistency rather than a design ' +
      'choice. Confirmed against Paper 1.21.4 with settings.bungeecord = true, which rejects the login with ' +
      '"If you wish to use IP forwarding, please enable it in your BungeeCord config as well!" - so this is not ' +
      'merely a missing field, it stops players connecting at all.',
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
