import net from 'node:net';

import { modernLoginStartOpening, RAW_REPLY } from './modded.js';

export function rawLoginStart({ host = '127.0.0.1', port = 25565, protocol, domain, username, timeoutMs = 10000, onRelayed = null }) {
  return new Promise((resolve) => {
    const sent = modernLoginStartOpening({ protocol, host: domain, port, username });
    const obs = { protocol, domain, username, sent: sent.toString('hex'), reply: null, replied: false, error: null };
    const chunks = [];
    let settled = false;

    const socket = net.connect({ host, port });
    const finish = () => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      socket.destroy();
      resolve(obs);
    };
    const timer = setTimeout(() => {
      obs.error = obs.error ?? `timed out after ${timeoutMs}ms waiting for the backend's reply`;
      finish();
    }, timeoutMs);

    socket.once('connect', () => socket.write(sent));
    socket.on('data', (chunk) => {
      chunks.push(chunk);
      const received = Buffer.concat(chunks);
      obs.reply = received.toString('hex');
      if (obs.replied || received.length < RAW_REPLY.length || !received.subarray(0, RAW_REPLY.length).equals(RAW_REPLY)) return;
      obs.replied = true;
      Promise.resolve()
        .then(() => onRelayed?.(obs))
        .catch((err) => {
          obs.onPlayError = String(err?.message ?? err);
        })
        .finally(finish);
    });
    socket.on('error', (err) => {
      obs.error = obs.error ?? String(err?.message ?? err);
      if (!obs.replied) finish();
    });
    socket.on('close', () => {
      if (obs.replied) return;
      obs.error = obs.error ?? 'the proxy closed the connection before the backend replied';
      finish();
    });
  });
}
