import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';

import { sleep } from './server.js';

export class Display {
  constructor(number, { width = 854, height = 480 } = {}) {
    this.number = number;
    this.width = width;
    this.height = height;
    this.child = null;
  }

  get name() {
    return `:${this.number}`;
  }

  async start({ timeoutMs = 10000 } = {}) {
    this.child = spawn(
      'Xvfb',
      [this.name, '-screen', '0', `${this.width}x${this.height}x24`, '-nolisten', 'tcp', '-noreset'],
      { stdio: ['ignore', 'ignore', 'pipe'] },
    );
    let stderr = '';
    this.child.stderr.on('data', (c) => (stderr += c.toString()));

    const socket = `/tmp/.X11-unix/X${this.number}`;
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      if (this.child.exitCode !== null) throw new Error(`Xvfb ${this.name} exited: ${stderr.trim()}`);
      if (existsSync(socket)) return this;
      await sleep(100);
    }
    throw new Error(`Xvfb ${this.name} never created ${socket}: ${stderr.trim()}`);
  }

  async stop() {
    if (!this.child || this.child.exitCode !== null) return;
    this.child.kill('SIGTERM');
    await sleep(100);
    if (this.child.exitCode === null) this.child.kill('SIGKILL');
  }
}

export async function openDisplay(opts = {}) {
  for (let n = 90; n < 130; n++) {
    if (existsSync(`/tmp/.X11-unix/X${n}`)) continue;
    try {
      return await new Display(n, opts).start();
    } catch {
    }
  }
  throw new Error('no free X display between :90 and :130');
}
