import mc from 'minecraft-protocol';

import { LOGIN_QUERIES } from './modded.js';

const DEFAULT_TIMEOUT_MS = 20000;

function sessionAuth({ username, uuid, accessToken }) {
  return (client, options) => {
    client.session = {
      accessToken,
      clientToken: 'infrarust-e2e-client-token',
      selectedProfile: { id: uuid, name: username },
      availableProfiles: [{ id: uuid, name: username }],
    };
    client.username = username;
    options.accessToken = accessToken;
    options.haveCredentials = true;
    options.connect(client);
  };
}

export function connectOnce({
  host = '127.0.0.1',
  port = 25565,
  version,
  domain,
  username,
  timeoutMs = DEFAULT_TIMEOUT_MS,
  session = null,
  sessionServer = null,
  onPlay = null,
  answerLoginQueries = false,
}) {
  return new Promise((resolve) => {
    const obs = {
      version,
      domain,
      username,
      reachedPlay: false,
      states: [],
      compressionThreshold: null,
      encrypted: false,
      loginUuid: null,
      loginPackets: [],
      setCompression: null,
      loginSuccessAt: null,
      loginQueries: [],
      error: null,
      kick: null,
    };

    let settled = false;
    let client;
    let timer;

    const finish = () => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      try {
        client?.end();
      } catch {
      }
      setTimeout(() => resolve(obs), 80);
    };

    const options = {
      host,
      port,
      username,
      version,
      fakeHost: domain,
      hideErrors: true,
      auth: session ? sessionAuth(session) : 'offline',
    };
    if (sessionServer) options.sessionServer = sessionServer;

    try {
      client = mc.createClient(options);
    } catch (err) {
      obs.error = `createClient: ${err.message}`;
      resolve(obs);
      return;
    }

    client.on('packet', (data, meta) => {
      if (meta.state !== 'login') return;
      obs.loginPackets.push(meta.name);
      if (meta.name === 'compress') obs.setCompression = data.threshold;
      else if (meta.name === 'success') obs.loginSuccessAt = performance.now();
      else if (meta.name === 'login_plugin_request') {
        obs.loginQueries.push({
          messageId: data.messageId,
          channel: data.channel,
          data: data.data ? data.data.toString('hex') : '',
          at: performance.now(),
        });
      }
    });

    if (answerLoginQueries) {
      client.removeAllListeners('login_plugin_request');
      client.on('login_plugin_request', (packet) => {
        const query = LOGIN_QUERIES.find((q) => q.channel === packet.channel);
        const response = query?.answer ? { messageId: packet.messageId, data: query.answer } : { messageId: packet.messageId };
        if (query?.answerDelayMs) setTimeout(() => client.write('login_plugin_response', response), query.answerDelayMs);
        else client.write('login_plugin_response', response);
      });
    }

    client.on('state', (newState) => {
      obs.states.push(newState);
      if (newState !== 'play') return;
      obs.reachedPlay = true;
      obs.compressionThreshold = client.compressionThreshold ?? null;
      if (!onPlay) {
        finish();
        return;
      }
      Promise.resolve()
        .then(() => onPlay(obs))
        .catch((err) => {
          obs.onPlayError = String(err?.message ?? err);
        })
        .finally(finish);
    });

    client.on('login', (packet) => {
      obs.loginUuid = packet?.uuid ?? client.uuid ?? null;
    });

    const realSetEncryption = client.setEncryption?.bind(client);
    if (realSetEncryption) {
      client.setEncryption = (secret) => {
        obs.encrypted = true;
        return realSetEncryption(secret);
      };
    }

    client.on('error', (err) => {
      obs.error = obs.error ?? String(err?.message ?? err);
      finish();
    });
    client.on('kick_disconnect', (packet) => {
      obs.kick = extractText(packet?.reason);
      finish();
    });
    client.on('disconnect', (packet) => {
      obs.kick = obs.kick ?? extractText(packet?.reason);
      finish();
    });
    client.on('end', () => {
      obs.error = obs.error ?? (obs.reachedPlay ? null : 'connection ended before reaching play');
      finish();
    });

    timer = setTimeout(() => {
      obs.error = obs.error ?? `timed out after ${timeoutMs}ms (states: ${obs.states.join('->') || 'none'})`;
      finish();
    }, timeoutMs);
  });
}

function extractText(reason) {
  if (reason == null) return null;
  if (typeof reason === 'string') {
    try {
      const parsed = JSON.parse(reason);
      return parsed?.text ?? parsed?.translate ?? reason;
    } catch {
      return reason;
    }
  }
  if (typeof reason === 'object') return reason.text ?? reason.translate ?? JSON.stringify(reason).slice(0, 200);
  return String(reason);
}
