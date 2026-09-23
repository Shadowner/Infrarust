export const BACKENDS = {
  plain: { port: 25700, flavour: 'vanilla', label: 'vanilla, offline-mode' },
  velocity: { port: 25701, flavour: 'paper', label: 'paper, velocity forwarding' },
};

export const PROXY_PORT_BASE = 25601;
export const ADMIN_PORT_BASE = 25631;

const VELOCITY_MIN_VERSION = '1.13';

export const SCENARIOS = [
  {
    id: 'direct',
    label: 'client -> server',
    proxy: null,
    backend: 'plain',
    expect: { joined: true, chatRelayed: true },
  },
  {
    id: 'passthrough',
    label: 'passthrough',
    proxy: { proxyMode: 'passthrough', forwardingMode: 'none' },
    backend: 'plain',
    expect: { joined: true, chatRelayed: true, proxySawPlayer: true, proxyActive: false },
  },
  {
    id: 'zerocopy',
    label: 'zero_copy',
    proxy: { proxyMode: 'zero_copy', forwardingMode: 'none' },
    backend: 'plain',
    linuxOnly: true,
    expect: { joined: true, chatRelayed: true, proxySawPlayer: true, proxyActive: false },
  },
  {
    id: 'server-only',
    label: 'server_only',
    proxy: { proxyMode: 'server_only', forwardingMode: 'none' },
    backend: 'plain',
    expect: { joined: true, chatRelayed: true, proxySawPlayer: true, proxyActive: false },
  },
  {
    id: 'offline',
    label: 'offline (intercepted)',
    proxy: { proxyMode: 'offline', forwardingMode: 'none' },
    backend: 'plain',
    expect: { joined: true, chatRelayed: true, proxySawPlayer: true, proxyActive: true },
  },
  {
    id: 'client-only',
    label: 'client_only (real auth)',
    proxy: { proxyMode: 'client_only', forwardingMode: 'none' },
    backend: 'plain',
    needsAuth: true,
    expect: { joined: true, chatRelayed: true, proxySawPlayer: true, proxyActive: true },
  },
  {
    id: 'velocity-direct-refused',
    label: 'velocity backend, no proxy (must refuse)',
    gatesVelocity: true,
    proxy: null,
    backend: 'velocity',
    minVersion: VELOCITY_MIN_VERSION,
    expect: { joined: false, rejection: /connect with Velocity/i },
  },
  {
    id: 'offline-velocity',
    label: 'offline + velocity',
    proxy: { proxyMode: 'offline', forwardingMode: 'velocity' },
    backend: 'velocity',
    minVersion: VELOCITY_MIN_VERSION,
    expect: { joined: true, chatRelayed: true, proxySawPlayer: true, proxyActive: true },
  },
  {
    id: 'client-only-velocity',
    label: 'client_only + velocity',
    proxy: { proxyMode: 'client_only', forwardingMode: 'velocity' },
    backend: 'velocity',
    needsAuth: true,
    minVersion: VELOCITY_MIN_VERSION,
    expect: { joined: true, chatRelayed: true, proxySawPlayer: true, proxyActive: true },
  },
  {
    id: 'offline-wrong-secret',
    label: 'offline + velocity, mismatched secret (must refuse)',
    proxy: { proxyMode: 'offline', forwardingMode: 'velocity', secret: 'wrongSecretForVelocityForwarding0' },
    backend: 'velocity',
    minVersion: VELOCITY_MIN_VERSION,
    expect: { joined: false, rejection: /Unable to verify player details/i },
  },
];

export function skipReason(scenario, { atLeast, authAvailable, paperAvailable, velocityBackendReady = null }) {
  if (scenario.linuxOnly && process.platform !== 'linux') return 'zero_copy is Linux-only';
  if (scenario.minVersion && !atLeast(scenario.minVersion)) {
    return `velocity forwarding needs ${scenario.minVersion}+ (no login plugin request before it)`;
  }
  if (scenario.needsAuth && !authAvailable) return 'no session server';
  if (BACKENDS[scenario.backend].flavour === 'paper' && !paperAvailable) return 'no Paper build for this version';
  if (scenario.backend === 'velocity' && !scenario.gatesVelocity && velocityBackendReady === false) {
    return 'this Paper build does not enforce Velocity forwarding';
  }
  return null;
}
