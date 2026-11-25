export const REMOTE = process.env.LAB_REMOTE ?? 'shado@192.168.50.226';
export const REMOTE_ROOT = process.env.LAB_REMOTE_ROOT ?? 'infrarust-lab';

export const API_KEY = 'infrarust-lab-api-key-0123456789';
export const FORWARDING_SECRET = 'labSecretForVelocityForwarding01';
export const DOMAIN_SUFFIX = '127.0.0.1.sslip.io';

export const PORTS = {
  yggdrasil: 25585,
  releases: 8900,
  native: { game: 25565, admin: 8080 },
  docker: { game: 25566, admin: 8081 },
  backendBase: 25700,
};

export const YGGDRASIL_URL = `http://127.0.0.1:${PORTS.yggdrasil}`;
export const RELEASES_URL = `http://127.0.0.1:${PORTS.releases}`;

export const VERSIONS = [
  { id: '1.8.9', key: '189', server: '1.8.8', image: 'java8', velocity: false, transfers: false },
  { id: '1.12.2', key: '1122', server: '1.12.2', image: 'java8', velocity: false, transfers: false },
  { id: '1.19.4', key: '1194', server: '1.19.4', image: 'java17', velocity: false, transfers: false },
  { id: '1.20.1', key: '1201', server: '1.20.1', image: 'java17', velocity: true, transfers: false },
  { id: '1.21.4', key: '1214', server: '1.21.4', image: 'java21', velocity: true, transfers: true },
  { id: '1.21.11', key: '12111', server: '1.21.11', image: 'java21', velocity: false, transfers: true },
  { id: '1.20.6', key: '1206', server: '1.20.6', image: 'java21', velocity: false, transfers: true },
];

export const MODES = [
  { key: 'pt', proxyMode: 'passthrough', forwarding: 'none', backend: 'plain', survives: true },
  { key: 'zc', proxyMode: 'zero_copy', forwarding: 'none', backend: 'plain', survives: true },
  { key: 'so', proxyMode: 'server_only', forwarding: 'none', backend: 'plain', survives: true },
  { key: 'of', proxyMode: 'offline', forwarding: 'none', backend: 'plain', survives: false },
  { key: 'co', proxyMode: 'client_only', forwarding: 'none', backend: 'plain', survives: false, auth: true },
  { key: 'ofv', proxyMode: 'offline', forwarding: 'velocity', backend: 'velocity', survives: false },
  { key: 'cov', proxyMode: 'client_only', forwarding: 'velocity', backend: 'velocity', survives: false, auth: true },
];

export function version(id) {
  const found = VERSIONS.find((v) => v.id === id);
  if (!found) throw new Error(`the lab has no backend for Minecraft ${id}`);
  return found;
}

export function mode(key) {
  const found = MODES.find((m) => m.key === key);
  if (!found) throw new Error(`unknown lab mode ${key}`);
  return found;
}

export function backends() {
  const out = [];
  VERSIONS.forEach((v, index) => {
    out.push({ name: `mc-${v.key}`, version: v, flavour: 'plain', port: PORTS.backendBase + index * 2 });
    if (v.velocity) {
      out.push({ name: `mc-${v.key}-v`, version: v, flavour: 'velocity', port: PORTS.backendBase + index * 2 + 1 });
    }
  });
  return out;
}

export function backendFor(versionId, flavour) {
  const found = backends().find((b) => b.version.id === versionId && b.flavour === flavour);
  if (!found) throw new Error(`no ${flavour} backend for ${versionId}`);
  return found;
}

export function routes() {
  const out = [];
  for (const v of VERSIONS) {
    for (const m of MODES) {
      if (m.backend === 'velocity' && !v.velocity) continue;
      out.push({
        id: `${m.key}${v.key}`,
        domain: `${m.key}${v.key}.${DOMAIN_SUFFIX}`,
        mode: m,
        version: v,
        backend: backendFor(v.id, m.backend),
      });
    }
  }
  return out;
}

export function route(modeKey, versionId) {
  const found = routes().find((r) => r.mode.key === modeKey && r.version.id === versionId);
  if (!found) throw new Error(`no route for ${modeKey} on ${versionId}`);
  return found;
}

export function container(backend) {
  return `lab-${backend.name}`;
}
