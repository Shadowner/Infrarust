import mc from 'minecraft-protocol';

import { sleep } from './remote.js';

const TRANSFER_INTENT = 3;

export class Bot {
  constructor({ username, route, port, follow = false }) {
    Object.assign(this, { username, route, port, follow });
    this.client = null;
    this.playAt = null;
    this.endedAt = null;
    this.kick = null;
    this.error = null;
    this.lastPacketAt = null;
    this.maxGapMs = 0;
    this.gapWindow = null;
    this.packets = 0;
    this.cookies = new Map();
    this.sentBack = [];
    this.returns = 0;
    this.cookieRequests = 0;
    this.leaving = null;
  }

  connect({ transferred = false } = {}) {
    const client = mc.createClient({
      host: '127.0.0.1',
      port: this.port,
      username: this.username,
      version: this.route.version.server,
      fakeHost: this.route.domain,
      hideErrors: true,
      auth: 'offline',
    });
    this.client = client;
    if (transferred) {
      const write = client.write.bind(client);
      client.write = (name, params) => write(name, name === 'set_protocol' ? { ...params, nextState: TRANSFER_INTENT } : params);
    }
    client.on('cookie_request', (packet) => {
      if (client.state !== 'login') return;
      this.cookieRequests++;
      client.write('cookie_response', { key: packet.key, value: this.cookies.get(packet.key) });
    });
    client.on('state', (state) => {
      if (state !== 'play') return;
      if (this.playAt === null) this.playAt = Date.now();
      if (transferred) this.returns++;
    });
    client.on('packet', (data, meta) => {
      const now = Date.now();
      if (meta.state === 'play') {
        if (this.gapWindow && this.lastPacketAt !== null) {
          this.gapWindow.maxGapMs = Math.max(this.gapWindow.maxGapMs, now - this.lastPacketAt);
        }
        this.lastPacketAt = now;
        this.packets++;
      }
      if (meta.name === 'kick_disconnect' || meta.name === 'disconnect') {
        this.kick = typeof data.reason === 'string' ? data.reason : JSON.stringify(data.reason);
      }
      if (meta.name === 'store_cookie') {
        this.cookies.set(data.key, data.value);
        this.leaving = { state: meta.state, key: data.key, valueLength: data.value?.length ?? null };
      }
      if (meta.name === 'transfer' && this.follow) {
        this.sentBack.push({ ...this.leaving, at: now, host: data.host, port: data.port, transferState: meta.state });
        this.leaving = null;
        this.connect({ transferred: true });
        client.end();
      }
    });
    client.on('error', (err) => {
      this.error = err.message;
    });
    client.on('end', () => {
      if (this.client === client && this.endedAt === null) this.endedAt = Date.now();
    });
    return this;
  }

  get inPlay() {
    return this.playAt !== null && this.endedAt === null && this.client?.state === 'play';
  }

  watchGap() {
    this.gapWindow = { maxGapMs: 0 };
    return this.gapWindow;
  }

  close() {
    try {
      this.client?.end();
    } catch {
    }
  }
}

export async function connectBots({ count, prefix, routes, port, ratePerSecond = 40, settleMs = 15000, follow = false }) {
  const bots = [];
  for (let i = 0; i < count; i++) {
    const route = routes[i % routes.length];
    bots.push(new Bot({ username: `${prefix}${String(i).padStart(4, '0')}`, route, port, follow }).connect());
    if ((i + 1) % ratePerSecond === 0) await sleep(1000);
  }
  const deadline = Date.now() + settleMs;
  while (Date.now() < deadline && bots.some((bot) => bot.playAt === null && bot.endedAt === null)) await sleep(100);
  return bots;
}

export async function settled(bots, { timeoutMs = 30000 } = {}) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline && bots.some((bot) => !bot.inPlay)) await sleep(100);
  return bots.filter((bot) => bot.inPlay).length;
}
