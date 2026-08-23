import mc from 'minecraft-protocol';

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
