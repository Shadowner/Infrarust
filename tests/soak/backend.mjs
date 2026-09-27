import { createRequire } from 'node:module';
import { writeFileSync } from 'node:fs';

const require = createRequire(new URL('../e2e/package.json', import.meta.url));
const mc = require('minecraft-protocol');
const mcData = require('minecraft-data');

function arg(name, fallback) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : fallback;
}

const version = arg('version', '1.18.2');
const host = arg('host', '127.0.0.1');
const ports = arg('ports', '41566,41567').split(',').map(Number);
const statsFile = arg('stats', null);
const data = mcData(version);

const totals = {};

function startServer(port) {
  const stats = { port, connections: 0, joins: 0, chats: 0, commandsLeaked: 0, errors: 0, online: 0, errorKinds: {} };
  const noteError = (err) => {
    stats.errors += 1;
    const key = String(err?.code ?? err?.message ?? err).slice(0, 80);
    stats.errorKinds[key] = (stats.errorKinds[key] ?? 0) + 1;
  };
  totals[port] = stats;
  const server = mc.createServer({
    'online-mode': false,
    version,
    port,
    host,
    motd: `soak backend ${port}`,
    maxPlayers: 100000,
    hideErrors: true,
    keepAlive: true,
    kickTimeout: 30000,
  });

  server.on('connection', (client) => {
    stats.connections += 1;
    client.on('error', noteError);
  });

  server.on('playerJoin', (client) => {
    stats.joins += 1;
    stats.online += 1;
    client.once('end', () => {
      stats.online -= 1;
    });
    try {
      client.write('login', {
        ...data.loginPacket,
        entityId: client.id,
        isHardcore: false,
        gameMode: 0,
        previousGameMode: 1,
        hashedSeed: [0, 0],
        maxPlayers: 100000,
        viewDistance: 2,
        simulationDistance: 2,
        reducedDebugInfo: false,
        enableRespawnScreen: true,
        isDebug: false,
        isFlat: true,
      });
      client.write('position', { x: 0, y: 64, z: 0, yaw: 0, pitch: 0, flags: 0, teleportId: 1, dismountVehicle: false });
    } catch (err) {
      noteError(err);
    }

    client.on('chat', (packet) => {
      const message = String(packet.message ?? '');
      if (message.startsWith('/')) {
        stats.commandsLeaked += 1;
        return;
      }
      stats.chats += 1;
      try {
        client.write('chat', {
          message: JSON.stringify({ text: `echo@${port} ${message}` }),
          position: 1,
          sender: '00000000-0000-0000-0000-000000000000',
        });
      } catch (err) {
        noteError(err);
      }
    });
  });

  server.on('error', (err) => {
    process.stderr.write(`backend ${port} error: ${err.message}\n`);
  });
  return server;
}

const servers = ports.map(startServer);

if (statsFile) {
  setInterval(() => {
    try {
      writeFileSync(statsFile, JSON.stringify({ t: Date.now(), backends: totals }));
    } catch {
    }
  }, 2000).unref();
}

function shutdown() {
  for (const server of servers) {
    try {
      server.close();
    } catch {
    }
  }
  if (statsFile) {
    try {
      writeFileSync(statsFile, JSON.stringify({ t: Date.now(), backends: totals }));
    } catch {
    }
  }
  setTimeout(() => process.exit(0), 300);
}

process.on('SIGTERM', shutdown);
process.on('SIGINT', shutdown);
process.stdout.write(`backends listening on ${ports.join(',')} (${version})\n`);
