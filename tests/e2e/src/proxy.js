import { spawn } from 'node:child_process';
import { once } from 'node:events';
import net from 'node:net';

import { API_KEY, WEB_BIND } from './lab.js';

const READY_LINE = 'listener bound';

export class Proxy {
  constructor(binary, configPath, { cwd, logLevel = 'info' } = {}) {
    this.binary = binary;
    this.configPath = configPath;
    this.cwd = cwd;
    this.logLevel = logLevel;
    this.child = null;
    this.lines = [];
  }

  async start({ timeoutMs = 20000 } = {}) {
    this.child = spawn(this.binary, ['--config', this.configPath, '--log-level', this.logLevel], {
      cwd: this.cwd,
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    this.child.stdin.end();

    let ready = false;
    const onData = (chunk) => {
      for (const line of chunk.toString().split('\n')) {
        if (line.trim()) this.lines.push(line);
      }
    };
    this.child.stdout.on('data', onData);
    this.child.stderr.on('data', onData);

    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      if (this.child.exitCode !== null) {
        throw new Error(`proxy exited early (code ${this.child.exitCode}):\n${this.tail(40)}`);
      }
      if (this.lines.some((l) => l.includes(READY_LINE))) {
        ready = true;
        break;
      }
      await sleep(100);
    }
    if (!ready) {
      throw new Error(`proxy did not report "${READY_LINE}" in ${timeoutMs}ms:\n${this.tail(40)}`);
    }
    return this;
  }

  mark() {
    return this.lines.length;
  }

  since(mark) {
    return this.lines.slice(mark);
  }

  tail(n = 20) {
    return this.lines.slice(-n).join('\n');
  }

  async stop() {
    if (!this.child || this.child.exitCode !== null) return;
    this.child.kill('SIGTERM');
    const exited = Promise.race([once(this.child, 'exit'), sleep(5000).then(() => 'timeout')]);
    if ((await exited) === 'timeout') this.child.kill('SIGKILL');
  }
}

export async function players() {
  const res = await fetch(`http://${WEB_BIND}/api/v1/players`, {
    headers: { Authorization: `Bearer ${API_KEY}` },
  });
  if (!res.ok) throw new Error(`admin API /players returned ${res.status}`);
  const body = await res.json();
  return body.data ?? [];
}

export async function playerDetail(idOrUsername) {
  const res = await fetch(`http://${WEB_BIND}/api/v1/players/${encodeURIComponent(idOrUsername)}`, {
    headers: { Authorization: `Bearer ${API_KEY}` },
  });
  if (res.status === 404) return null;
  if (!res.ok) throw new Error(`admin API /players/${idOrUsername} returned ${res.status}`);
  const body = await res.json();
  return body.data ?? null;
}

export async function waitForPlayer(username, { timeoutMs = 3000 } = {}) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const player = await playerDetail(username);
    if (player) return player;
    if (Date.now() > deadline) return null;
    await sleep(150);
  }
}

export async function waitForPort(port, { host = '127.0.0.1', timeoutMs = 10000 } = {}) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const ok = await new Promise((resolve) => {
      const socket = net.connect({ port, host });
      const done = (v) => {
        socket.removeAllListeners();
        socket.destroy();
        resolve(v);
      };
      socket.once('connect', () => done(true));
      socket.once('error', () => done(false));
      socket.setTimeout(1000, () => done(false));
    });
    if (ok) return;
    if (Date.now() > deadline) throw new Error(`port ${port} never accepted a connection`);
    await sleep(100);
  }
}

export function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
