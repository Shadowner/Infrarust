export const BACKENDS = {
  plain: { port: 25700, kind: 'plain' },
  velocity: { port: 25701, kind: 'velocity' },
  bungee: { port: 25702, kind: 'bungee' },
};

const VELOCITY_MIN_PROTOCOL = 393;

export const SCENARIOS = [
  {
    id: 'passthrough',
    domain: 'pt.test',
    proxyMode: 'passthrough',
    forwardingMode: 'none',
    backend: 'plain',
    expect: { reachesPlay: true, interceptedByProxy: false },
  },
  {
    id: 'zerocopy',
    domain: 'zc.test',
    proxyMode: 'zero_copy',
    forwardingMode: 'none',
    backend: 'plain',
    linuxOnly: true,
    expect: { reachesPlay: true, interceptedByProxy: false },
  },
  {
    id: 'server-only',
    domain: 'so.test',
    proxyMode: 'server_only',
    forwardingMode: 'none',
    backend: 'plain',
    expect: { reachesPlay: true, interceptedByProxy: false },
  },
  {
    id: 'offline',
    domain: 'off.test',
    proxyMode: 'offline',
    forwardingMode: 'none',
    backend: 'plain',
    expect: { reachesPlay: true, interceptedByProxy: true },
  },
  {
    id: 'offline-velocity',
    domain: 'off-velo.test',
    proxyMode: 'offline',
    forwardingMode: 'velocity',
    backend: 'velocity',
    minProtocol: VELOCITY_MIN_PROTOCOL,
    expect: { reachesPlay: true, interceptedByProxy: true, velocitySigned: true },
  },
  {
    id: 'offline-bungeecord',
    domain: 'off-bungee.test',
    proxyMode: 'offline',
    forwardingMode: 'bungee_cord',
    backend: 'bungee',
    expect: { reachesPlay: true, interceptedByProxy: true, bungeeInjected: true },
  },
  {
    id: 'offline-bungeeguard',
    domain: 'off-bguard.test',
    proxyMode: 'offline',
    forwardingMode: 'bungee_guard',
    backend: 'bungee',
    expect: { reachesPlay: true, interceptedByProxy: true, bungeeInjected: true, bungeeGuardToken: true },
  },
  {
    id: 'client-only',
    domain: 'co.test',
    proxyMode: 'client_only',
    forwardingMode: 'none',
    backend: 'plain',
    needsAuth: true,
    expect: { reachesPlay: true, interceptedByProxy: true, encrypted: true },
  },
  {
    id: 'client-only-velocity',
    domain: 'co-velo.test',
    proxyMode: 'client_only',
    forwardingMode: 'velocity',
    backend: 'velocity',
    needsAuth: true,
    minProtocol: VELOCITY_MIN_PROTOCOL,
    expect: { reachesPlay: true, interceptedByProxy: true, encrypted: true, velocitySigned: true },
  },
  {
    id: 'passthrough-velocity-fallback',
    domain: 'pt-velo.test',
    proxyMode: 'passthrough',
    forwardingMode: 'velocity',
    backend: 'bungee',
    expect: { reachesPlay: true, interceptedByProxy: false, bungeeInjected: true },
  },
  {
    id: 'full-fallback',
    domain: 'full.test',
    proxyMode: 'full',
    forwardingMode: 'none',
    backend: 'plain',
    expect: { reachesPlay: true, interceptedByProxy: false },
  },
];

export function skipReason(scenario, protocol, opts = {}) {
  if (scenario.linuxOnly && process.platform !== 'linux') {
    return 'zero_copy is Linux-only';
  }
  if (scenario.minProtocol && protocol < scenario.minProtocol) {
    return `velocity forwarding needs protocol >= ${scenario.minProtocol} (login plugin request is 1.13+)`;
  }
  if (scenario.needsAuth && !opts.authAvailable) {
    return 'no session server configured';
  }
  return null;
}
