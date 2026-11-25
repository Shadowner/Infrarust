import { mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { API_KEY, FORWARDING_SECRET, PORTS, RELEASES_URL, YGGDRASIL_URL, backends, container, routes } from './topology.js';

const quote = (value) => JSON.stringify(String(value));

function proxyConfig({ home, game, admin }) {
  return [
    `bind = ${quote(`127.0.0.1:${game}`)}`,
    `servers_dir = ${quote(`${home}/servers`)}`,
    `plugins_dir = ${quote(`${home}/plugins`)}`,
    'connect_timeout = "5s"',
    'max_connections_per_ip = 0',
    '',
    '[rate_limit]',
    'enabled = false',
    '',
    '[forwarding]',
    'mode = "velocity"',
    `secret_file = ${quote(`${home}/forwarding.secret`)}`,
    '',
    '[auth]',
    `session_url = ${quote(`${YGGDRASIL_URL}/authlib-injector/sessionserver/session/minecraft/hasJoined`)}`,
    '',
    '[web]',
    'enable_api = true',
    'enable_webui = true',
    `bind = ${quote(`127.0.0.1:${admin}`)}`,
    `api_key = ${quote(API_KEY)}`,
    '',
    '[web.rate_limit]',
    'requests_per_minute = 1000000',
    '',
    '[update]',
    'check = true',
    'channel = "prerelease"',
    'interval = "1h"',
    `url = ${quote(`${RELEASES_URL}/releases`)}`,
    '',
  ].join('\n');
}

function serverConfig(entry) {
  return [
    `name = ${quote(entry.id)}`,
    `domains = [${quote(entry.domain)}]`,
    `addresses = [${quote(`127.0.0.1:${entry.backend.port}`)}]`,
    `proxy_mode = ${quote(entry.mode.proxyMode)}`,
    `forwarding_mode = ${quote(entry.mode.forwarding)}`,
    '',
  ].join('\n');
}

function writeProxy(dir, home, ports) {
  mkdirSync(join(dir, 'servers'), { recursive: true });
  mkdirSync(join(dir, 'plugins'), { recursive: true });
  writeFileSync(join(dir, 'infrarust.toml'), proxyConfig({ home, ...ports }));
  writeFileSync(join(dir, 'forwarding.secret'), FORWARDING_SECRET);
  for (const entry of routes()) writeFileSync(join(dir, 'servers', `${entry.id}.toml`), serverConfig(entry));
}

function backendService(backend) {
  const modern = !['1.8.8', '1.12.2'].includes(backend.version.server);
  return [
    `  ${backend.name}:`,
    `    image: docker.io/itzg/minecraft-server:${backend.version.image}`,
    `    container_name: ${container(backend)}`,
    '    tty: true',
    '    stdin_open: true',
    '    restart: "no"',
    '    environment:',
    '      EULA: "TRUE"',
    '      TYPE: "PAPER"',
    `      VERSION: ${quote(backend.version.server)}`,
    '      ONLINE_MODE: "FALSE"',
    '      GUI: "FALSE"',
    `      MEMORY: ${quote(modern ? '2G' : '1G')}`,
    '      LEVEL_TYPE: "FLAT"',
    '      VIEW_DISTANCE: "4"',
    '      SIMULATION_DISTANCE: "4"',
    '      SPAWN_PROTECTION: "0"',
    '      SPAWN_ANIMALS: "FALSE"',
    '      SPAWN_MONSTERS: "FALSE"',
    '      SPAWN_NPCS: "FALSE"',
    '      SYNC_CHUNK_WRITES: "FALSE"',
    '      MAX_PLAYERS: "600"',
    '      MODE: "creative"',
    '      ENABLE_RCON: "TRUE"',
    '      ENFORCE_SECURE_PROFILE: "FALSE"',
    ...(backend.flavour === 'velocity'
      ? [
          `      JVM_OPTS: "-javaagent:/data/authlib-injector.jar=${YGGDRASIL_URL}/authlib-injector"`,
          `      SERVER_PORT: "${backend.port}"`,
          `      RCON_PORT: "${backend.port + 1000}"`,
        ]
      : []),
    '      RCON_PASSWORD: "infrarust-lab"',
    '    volumes:',
    `      - ./backends/${backend.name}:/data`,
    ...(backend.flavour === 'velocity'
      ? ['    network_mode: host']
      : ['    ports:', `      - "127.0.0.1:${backend.port}:25565"`]),
    '    healthcheck:',
    '      test: ["CMD", "mc-health"]',
    '      start_period: 30s',
    '      interval: 5s',
    '      retries: 80',
    '',
  ].join('\n');
}

function compose() {
  return [
    'name: infrarust-lab',
    'services:',
    '  drasl:',
    '    image: docker.io/unmojang/drasl:3.4.4',
    '    container_name: lab-drasl',
    '    restart: "no"',
    '    volumes:',
    '      - ./drasl/config:/etc/drasl',
    '      - ./drasl/data:/var/lib/drasl',
    '    ports:',
    `      - "127.0.0.1:${PORTS.yggdrasil}:${PORTS.yggdrasil}"`,
    '',
    '  releases:',
    '    image: docker.io/library/node:24.21.0-alpine3.24',
    '    container_name: lab-releases',
    '    restart: "no"',
    '    network_mode: host',
    '    environment:',
    `      LAB_PORT: "${PORTS.releases}"`,
    '    volumes:',
    '      - ./release-server.mjs:/srv/release-server.mjs:ro',
    '      - ../releases:/data:ro',
    '    command: ["node", "/srv/release-server.mjs"]',
    '',
    '  proxy:',
    '    profiles: ["docker-proxy"]',
    '    image: infrarust-lab:docker',
    '    container_name: lab-proxy',
    '    restart: "no"',
    '    network_mode: host',
    '    stdin_open: true',
    '    tty: false',
    '    volumes:',
    '      - ./proxy-docker:/app/config',
    '',
    ...backends().map(backendService),
  ].join('\n');
}

function drasl() {
  return [
    'Domain = "127.0.0.1"',
    `BaseURL = ${quote(YGGDRASIL_URL)}`,
    `ListenAddress = "0.0.0.0:${PORTS.yggdrasil}"`,
    'DefaultAdmins = ["admin"]',
    '',
    'PlayerUUIDGeneration = "offline"',
    'MinPasswordLength = 4',
    '',
    '[RegistrationNewPlayer]',
    'Allow = true',
    'RequireInvite = false',
    '',
  ].join('\n');
}

function seedBackend(dir, backend) {
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, 'bukkit.yml'), 'settings:\n  connection-throttle: -1\n');
  if (backend.flavour !== 'velocity') return;
  mkdirSync(join(dir, 'config'), { recursive: true });
  writeFileSync(
    join(dir, 'config', 'paper-global.yml'),
    ['proxies:', '  velocity:', '    enabled: true', '    online-mode: true', `    secret: '${FORWARDING_SECRET}'`, ''].join('\n'),
  );
}

export function writeStage(dir, { remoteHome, releaseServer }) {
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });

  writeProxy(join(dir, 'proxy-native'), `${remoteHome}/native`, PORTS.native);
  writeProxy(join(dir, 'proxy-docker'), '/app/config', PORTS.docker);

  mkdirSync(join(dir, 'drasl', 'config'), { recursive: true });
  writeFileSync(join(dir, 'drasl', 'config', 'config.toml'), drasl());

  for (const backend of backends()) seedBackend(join(dir, 'backend-seed', backend.name), backend);

  writeFileSync(join(dir, 'compose.yml'), compose());
  writeFileSync(join(dir, 'release-server.mjs'), releaseServer);
  writeFileSync(
    join(dir, 'infrarust-lab.service'),
    [
      '[Unit]',
      'Description=Infrarust e2e lab proxy',
      'After=network-online.target',
      '',
      '[Service]',
      'Type=simple',
      'User=shado',
      `WorkingDirectory=${remoteHome}/native`,
      `ExecStart=${remoteHome}/native/infrarust --config ${remoteHome}/native/infrarust.toml`,
      'ExecReload=/bin/kill -USR2 $MAINPID',
      'Restart=no',
      'TimeoutStopSec=45',
      'LimitNOFILE=1048576',
      '',
    ].join('\n'),
  );
}
