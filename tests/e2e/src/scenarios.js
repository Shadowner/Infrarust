import { BIG_PACKET_CHANNEL, BIG_PACKET_MIN_PROTOCOL, LOGIN_QUERY_MIN_PROTOCOL } from './modded.js';

export const BACKENDS = {
  plain: { port: 25700, kind: 'plain' },
  velocity: { port: 25701, kind: 'velocity', velocity: true },
  bungee: { port: 25702, kind: 'bungee' },
  query: { port: 25703, kind: 'query', loginQueries: true },
  queryVelocity: { port: 25704, kind: 'query-velocity', velocity: true, loginQueries: true },
  bigPacket: { port: 25705, kind: 'big-packet', bigPacket: true },
};

const VELOCITY_MIN_PROTOCOL = 393;
const LOGIN_QUERY_REASON = `login plugin requests need protocol >= ${LOGIN_QUERY_MIN_PROTOCOL} (1.13+)`;

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
    expect: { reachesPlay: true, interceptedByProxy: true, clientCompression: 'backend' },
  },
  {
    id: 'offline-velocity',
    domain: 'off-velo.test',
    proxyMode: 'offline',
    forwardingMode: 'velocity',
    backend: 'velocity',
    minProtocol: VELOCITY_MIN_PROTOCOL,
    expect: { reachesPlay: true, interceptedByProxy: true, velocitySigned: true, clientCompression: 'proxy' },
  },
  {
    id: 'offline-bungeecord',
    domain: 'off-bungee.test',
    proxyMode: 'offline',
    forwardingMode: 'bungee_cord',
    backend: 'bungee',
    expect: { reachesPlay: true, interceptedByProxy: true, bungeeInjected: true, clientCompression: 'proxy' },
  },
  {
    id: 'offline-bungeeguard',
    domain: 'off-bguard.test',
    proxyMode: 'offline',
    forwardingMode: 'bungee_guard',
    backend: 'bungee',
    expect: { reachesPlay: true, interceptedByProxy: true, bungeeInjected: true, bungeeGuardToken: true, clientCompression: 'proxy' },
  },
  {
    id: 'client-only',
    domain: 'co.test',
    proxyMode: 'client_only',
    forwardingMode: 'none',
    backend: 'plain',
    needsAuth: true,
    expect: { reachesPlay: true, interceptedByProxy: true, encrypted: true, clientCompression: 'proxy' },
  },
  {
    id: 'client-only-velocity',
    domain: 'co-velo.test',
    proxyMode: 'client_only',
    forwardingMode: 'velocity',
    backend: 'velocity',
    needsAuth: true,
    minProtocol: VELOCITY_MIN_PROTOCOL,
    expect: { reachesPlay: true, interceptedByProxy: true, encrypted: true, velocitySigned: true, clientCompression: 'proxy' },
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
  {
    id: 'login-query-offline',
    domain: 'lq-off.test',
    proxyMode: 'offline',
    forwardingMode: 'none',
    backend: 'query',
    mockOnly: true,
    minProtocol: LOGIN_QUERY_MIN_PROTOCOL,
    protocolReason: LOGIN_QUERY_REASON,
    client: { answerLoginQueries: true },
    expect: {
      reachesPlay: true,
      interceptedByProxy: true,
      loginQueriesRelayed: true,
      loginSuccessAfterBackend: true,
      clientCompression: 'backend',
    },
  },
  {
    id: 'login-query-velocity',
    domain: 'lq-velo.test',
    proxyMode: 'offline',
    forwardingMode: 'velocity',
    backend: 'queryVelocity',
    mockOnly: true,
    minProtocol: LOGIN_QUERY_MIN_PROTOCOL,
    protocolReason: LOGIN_QUERY_REASON,
    client: { answerLoginQueries: true },
    expect: {
      reachesPlay: true,
      interceptedByProxy: true,
      velocitySigned: true,
      loginQueriesRelayed: true,
      loginSuccessAfterBackend: true,
      clientCompression: 'proxy',
    },
  },
  {
    id: 'login-query-client-only',
    domain: 'lq-co.test',
    proxyMode: 'client_only',
    forwardingMode: 'none',
    backend: 'query',
    needsAuth: true,
    mockOnly: true,
    minProtocol: LOGIN_QUERY_MIN_PROTOCOL,
    protocolReason: LOGIN_QUERY_REASON,
    client: { answerLoginQueries: true },
    expect: {
      reachesPlay: true,
      interceptedByProxy: true,
      encrypted: true,
      loginQueriesRelayed: true,
      loginSuccessAfterBackend: true,
      clientCompression: 'proxy',
    },
  },
  {
    id: 'big-packet-client-only',
    domain: 'big-co.test',
    proxyMode: 'client_only',
    forwardingMode: 'none',
    backend: 'bigPacket',
    needsAuth: true,
    mockOnly: true,
    minProtocol: BIG_PACKET_MIN_PROTOCOL,
    protocolReason: `packet compression needs protocol >= ${BIG_PACKET_MIN_PROTOCOL} (1.8+); without it no packet may exceed 2 MiB`,
    client: { awaitPayload: BIG_PACKET_CHANNEL },
    expect: { reachesPlay: true, interceptedByProxy: true, encrypted: true, clientCompression: 'proxy', bigPacketIntact: true },
  },
  {
    id: 'forge-rewrite',
    domain: 'forge-rw.test',
    proxyMode: 'passthrough',
    forwardingMode: 'none',
    backend: 'plain',
    domainRewrite: 'forge-backend.internal',
    forgeMarker: true,
    expect: { reachesPlay: true, interceptedByProxy: false, forgeMarker: { rewrite: 'forge-backend.internal' } },
  },
  {
    id: 'forge-bungeecord',
    domain: 'forge-bungee.test',
    proxyMode: 'offline',
    forwardingMode: 'bungee_cord',
    backend: 'bungee',
    forgeMarker: true,
    expect: { reachesPlay: true, interceptedByProxy: true, forgeMarker: { properties: true } },
  },
];

export function skipReason(scenario, protocol, opts = {}) {
  if (scenario.linuxOnly && process.platform !== 'linux') {
    return 'zero_copy is Linux-only';
  }
  if (scenario.mockOnly && opts.realBackends) {
    return 'needs the mock backend, which is the side that drives this scenario';
  }
  if (scenario.minProtocol && protocol < scenario.minProtocol) {
    return scenario.protocolReason ?? `velocity forwarding needs protocol >= ${scenario.minProtocol} (login plugin request is 1.13+)`;
  }
  if (scenario.needsAuth && !opts.authAvailable) {
    return 'no session server configured';
  }
  return null;
}
