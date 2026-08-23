import mc from 'minecraft-protocol';

import { MAX_FORWARDING_VERSION, VELOCITY_CHANNEL, verifyAndDecode } from './velocity.js';

function installVelocityExchange(client, observation, secret) {
  const nmpListeners = client.listeners('login_start');
  client.removeAllListeners('login_start');

  client.once('login_start', (packet) => {
    const messageId = 1;
    observation.velocityRequested = true;

    client.once('login_plugin_response', (response) => {
      observation.velocityResponded = true;
      observation.velocityMessageIdEcho = response.messageId;

      if (!response.data || response.data.length === 0) {
        observation.velocity = { ok: false, error: 'proxy answered "not understood" (empty data)' };
      } else {
        observation.velocity = verifyAndDecode(response.data, secret);
      }

      for (const listener of nmpListeners) listener.call(client, packet);
    });

    client.write('login_plugin_request', {
      messageId,
      channel: VELOCITY_CHANNEL,
      data: Buffer.from([MAX_FORWARDING_VERSION]),
    });
  });
}

export function startBackend({ version, port, kind, secret, host = '127.0.0.1' }) {
  return new Promise((resolve, reject) => {
    const state = { last: null, connections: 0 };

    const server = mc.createServer({
      'online-mode': false,
      version,
      port,
      host,
      motd: `infrarust-e2e ${kind}`,
      maxPlayers: 20,
      hideErrors: true,
      kickTimeout: 30000,
      beforeLogin(client) {
        if (state.last) state.last.username = client.username;
      },
    });

    server.on('connection', (client) => {
      state.connections += 1;
      const observation = { kind, handshakeHost: null, protocolVersion: null, reachedLogin: false, reachedPlay: false };
      state.last = observation;

      client.once('set_protocol', (packet) => {
        observation.handshakeHost = packet.serverHost;
        observation.protocolVersion = packet.protocolVersion;
        observation.nextState = packet.nextState;
      });

      if (kind === 'velocity') installVelocityExchange(client, observation, secret);

      client.on('error', (err) => {
        observation.error = observation.error ?? String(err.message ?? err);
      });
    });

    server.on('login', (client) => {
      if (state.last) {
        state.last.reachedLogin = true;
        state.last.username = client.username;
        state.last.uuid = client.uuid;
      }
    });

    server.on('playerJoin', () => {
      if (state.last) state.last.reachedPlay = true;
    });

    server.on('error', (err) => reject(err));
    server.once('listening', () =>
      resolve({
        kind,
        port,
        server,
        reset() {
          state.last = null;
        },
        observation() {
          return state.last;
        },
        close() {
          return new Promise((done) => {
            server.once('close', done);
            try {
              server.close();
            } catch {
              done();
            }
            setTimeout(done, 3000);
          });
        },
      }),
    );
  });
}
