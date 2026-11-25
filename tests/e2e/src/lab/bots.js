import mc from 'minecraft-protocol';

import { sleep } from './remote.js';

export class Bot {
  constructor({ username, route, port }) {
    Object.assign(this, { username, route, port });
    this.client = null;
    this.playAt = null;
    this.endedAt = null;
    this.kick = null;
    this.error = null;
    this.lastPacketAt = null;
    this.maxGapMs = 0;
    this.gapWindow = null;
    this.packets = 0;
  }

  connect() {
    this.client = mc.createClient({
      host: '127.0.0.1',
      port: this.port,
      username: this.username,
      version: this.route.version.server,
      fakeHost: this.route.domain,
      hideErrors: true,
      auth: 'offline',
    });
    this.client.on('state', (state) => {
      if (state === 'play' && this.playAt === null) this.playAt = Date.now();
    });
    this.client.on('packet', (data, meta) => {
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
    });
    this.client.on('error', (err) => {
      this.error = err.message;
    });
    this.client.on('end', () => {
      if (this.endedAt === null) this.endedAt = Date.now();
    });
    return this;
  }

  get inPlay() {
    return this.playAt !== null && this.endedAt === null;
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

export async function connectBots({ count, prefix, routes, port, ratePerSecond = 40, settleMs = 15000 }) {
  const bots = [];
  for (let i = 0; i < count; i++) {
    const route = routes[i % routes.length];
    bots.push(new Bot({ username: `${prefix}${String(i).padStart(4, '0')}`, route, port }).connect());
    if ((i + 1) % ratePerSecond === 0) await sleep(1000);
  }
  const deadline = Date.now() + settleMs;
  while (Date.now() < deadline && bots.some((bot) => bot.playAt === null && bot.endedAt === null)) await sleep(100);
  return bots;
}
