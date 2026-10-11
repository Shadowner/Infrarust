import net from 'node:net';

import mc from 'minecraft-protocol';

import { BIG_PACKET_CHANNEL, bigPacketData, LOGIN_QUERIES, LOGIN_QUERY_TIMEOUT_MS, RAW_REPLY, RAW_TAIL } from './modded.js';
import { MAX_FORWARDING_VERSION, VELOCITY_CHANNEL, verifyAndDecode } from './velocity.js';

function velocityExchange(client, observation, secret) {
  return new Promise((resolve) => {
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
      resolve();
    });

    client.write('login_plugin_request', {
      messageId,
      channel: VELOCITY_CHANNEL,
      data: Buffer.from([MAX_FORWARDING_VERSION]),
    });
  });
}

function loginQueries(client, observation) {
  const queries = { sent: LOGIN_QUERIES.map((q) => q.messageId), responses: [], complete: false };
  observation.loginQueries = queries;

  return new Promise((resolve) => {
    let timer = null;
    const done = () => {
      clearTimeout(timer);
      resolve();
    };
    client.on('login_plugin_response', (response) => {
      queries.responses.push({
        messageId: response.messageId,
        data: response.data ? response.data.toString('hex') : null,
      });
      if (queries.responses.length === LOGIN_QUERIES.length) {
        queries.complete = true;
        done();
      }
    });
    timer = setTimeout(done, LOGIN_QUERY_TIMEOUT_MS);

    client.socket.cork();
    for (const q of LOGIN_QUERIES) {
      client.write('login_plugin_request', { messageId: q.messageId, channel: q.channel, data: q.request });
    }
    setImmediate(() => client.socket.uncork());
  });
}

function holdLoginStart(client, observation, steps) {
  const nmpListeners = client.listeners('login_start');
  client.removeAllListeners('login_start');

  client.once('login_start', async (packet) => {
    for (const step of steps) await step();
    observation.loginSuccessSentAt = performance.now();
    for (const listener of nmpListeners) listener.call(client, packet);
  });
}

export function startBackend({ version, port, kind, secret, velocity = false, loginQueries: queries = false, bigPacket = false, raw = false, host = '127.0.0.1' }) {
  if (raw) return startRawBackend({ port, kind, host });

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

      const steps = [];
      if (velocity) steps.push(() => velocityExchange(client, observation, secret));
      if (queries) steps.push(() => loginQueries(client, observation));
      if (steps.length) holdLoginStart(client, observation, steps);

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

    server.on('playerJoin', (client) => {
      if (state.last) state.last.reachedPlay = true;
      if (bigPacket) {
        client.write('custom_payload', { channel: BIG_PACKET_CHANNEL, data: bigPacketData() });
        if (state.last) state.last.bigPacketSent = true;
      }
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

function startRawBackend({ port, kind, host }) {
  return new Promise((resolve, reject) => {
    const state = { last: null };
    const sockets = new Set();

    const server = net.createServer((socket) => {
      sockets.add(socket);
      socket.on('close', () => sockets.delete(socket));
      const chunks = [];
      let observation = null;

      socket.on('data', (chunk) => {
        chunks.push(chunk);
        const received = Buffer.concat(chunks);
        if (!observation) {
          observation = { kind, replied: false };
          state.last = observation;
        }
        observation.received = received.toString('hex');
        observation.receivedLength = received.length;
        if (!observation.replied && received.length >= RAW_TAIL.length && received.subarray(-RAW_TAIL.length).equals(RAW_TAIL)) {
          observation.replied = true;
          socket.write(RAW_REPLY);
        }
      });
      socket.on('error', (err) => {
        if (observation) observation.error = observation.error ?? String(err.message ?? err);
      });
    });

    server.on('error', (err) => reject(err));
    server.listen(port, host, () =>
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
            for (const socket of sockets) socket.destroy();
            server.close(() => done());
            setTimeout(done, 3000);
          });
        },
      }),
    );
  });
}
