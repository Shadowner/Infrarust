import { spawn } from 'node:child_process';
import { once } from 'node:events';

import net from 'node:net';

import { API_KEY } from './lab.js';
import { sleep } from './server.js';

const READY_LINE = 'listener bound';

export async function portInUse(port, { host = '127.0.0.1' } = {}) {
  return new Promise((resolve) => {
    const socket = net.connect({ port, host });
    const done = (value) => {
      socket.removeAllListeners();
      socket.destroy();
      resolve(value);
    };
    socket.once('connect', () => done(true));
    socket.once('error', () => done(false));
    socket.setTimeout(500, () => done(false));
  });
}

export class ProxyProcess {
  constructor(binary, configPath, { id, port, adminPort, logLevel = 'info', cwd } = {}) {
    Object.assign(this, { binary, configPath, id, port, adminPort, logLevel, cwd });
    this.child = null;
    this.lines = [];
  }

  async start({ timeoutMs = 20000 } = {}) {
    if (await portInUse(this.port)) {
      throw new Error(
        `port ${this.port} is already in use, so the ${this.id} proxy cannot start. ` +
          'A previous run probably left its proxies behind: pkill -f target/release/infrarust',
      );
    }
    this.child = spawn(this.binary, ['--config', this.configPath, '--log-level', this.logLevel], {
      cwd: this.cwd,
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    this.child.stdin.end();

    const onData = (chunk) => {
      for (const line of chunk.toString().split('\n')) if (line.trim()) this.lines.push(line);
    };
    this.child.stdout.on('data', onData);
    this.child.stderr.on('data', onData);

    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      if (this.child.exitCode !== null) {
        throw new Error(`proxy ${this.id} exited ${this.child.exitCode}:\n${this.tail(30)}`);
      }
      if (this.lines.some((l) => l.includes(READY_LINE))) return this;
      await sleep(100);
    }
    throw new Error(`proxy ${this.id} never bound in ${timeoutMs}ms:\n${this.tail(30)}`);
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

  async player(username) {
    const res = await fetch(`http://127.0.0.1:${this.adminPort}/api/v1/players/${encodeURIComponent(username)}`, {
      headers: { Authorization: `Bearer ${API_KEY}` },
    });
    if (res.status === 404) return null;
    if (!res.ok) throw new Error(`admin API returned ${res.status}`);
    const body = await res.json();
    return body.data ?? null;
  }

  async waitForPlayer(username, { timeoutMs = 5000 } = {}) {
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      const found = await this.player(username);
      if (found) return found;
      if (Date.now() > deadline) return null;
      await sleep(150);
    }
  }

  async waitForError(mark, { timeoutMs = 60000, quietMs = 1500 } = {}) {
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      const line = this.since(mark).find((l) => /\bERROR\b/.test(l));
      if (line) {
        await sleep(quietMs);
        return line;
      }
      if (Date.now() > deadline) return null;
      await sleep(200);
    }
  }

  async stop() {
    if (!this.child || this.child.exitCode !== null) return;
    this.child.kill('SIGTERM');
    const exited = await Promise.race([once(this.child, 'exit').then(() => 'exited'), sleep(5000).then(() => 'timeout')]);
    if (exited === 'timeout') this.child.kill('SIGKILL');
  }
}
