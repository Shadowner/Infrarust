import { spawn } from 'node:child_process';

import { API_KEY, PORTS, REMOTE, REMOTE_ROOT, RELEASES_URL, container } from './topology.js';

const SOCKET = `${process.env.XDG_RUNTIME_DIR ?? '/tmp'}/infrarust-lab-ssh`;
const SSH_ARGS = ['-o', 'BatchMode=yes', '-o', 'ControlMaster=auto', '-o', `ControlPath=${SOCKET}`, '-o', 'ControlPersist=600'];

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

export function sh(command, { input, allowFailure = false, timeoutMs = 120000 } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn('ssh', [...SSH_ARGS, REMOTE, command], { stdio: ['pipe', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    const timer = setTimeout(() => child.kill('SIGKILL'), timeoutMs);
    child.stdout.on('data', (chunk) => (stdout += chunk));
    child.stderr.on('data', (chunk) => (stderr += chunk));
    child.stdin.end(input ?? '');
    child.once('error', reject);
    child.once('exit', (code) => {
      clearTimeout(timer);
      if (code === 0 || allowFailure) resolve({ code, stdout, stderr });
      else reject(new Error(`remote command failed (${code}): ${command}\n${stderr || stdout}`));
    });
  });
}

export function lab(args, opts) {
  return sh(`bash ${REMOTE_ROOT}/src/tests/e2e/lab/remote.sh ${args}`, opts);
}

export class Tunnel {
  constructor(ports = [PORTS.native.game, PORTS.native.admin, PORTS.docker.game, PORTS.docker.admin, PORTS.yggdrasil, PORTS.releases]) {
    this.ports = ports;
    this.child = null;
  }

  async open() {
    const forwards = this.ports.flatMap((port) => ['-L', `127.0.0.1:${port}:127.0.0.1:${port}`]);
    this.child = spawn(
      'ssh',
      ['-o', 'BatchMode=yes', '-o', 'ExitOnForwardFailure=yes', '-o', 'ServerAliveInterval=15', '-N', ...forwards, REMOTE],
      { stdio: ['ignore', 'ignore', 'pipe'] },
    );
    let stderr = '';
    this.child.stderr.on('data', (chunk) => (stderr += chunk));
    const deadline = Date.now() + 15000;
    for (;;) {
      if (this.child.exitCode !== null) throw new Error(`the SSH tunnel did not open: ${stderr.trim()}`);
      try {
        await fetch(`${RELEASES_URL}/_control`);
        return this;
      } catch {
      }
      if (Date.now() > deadline) throw new Error('the SSH tunnel never carried a request');
      await sleep(200);
    }
  }

  close() {
    if (this.child && this.child.exitCode === null) this.child.kill('SIGTERM');
  }
}

export class Console {
  constructor(backend) {
    this.backend = backend;
    this.child = null;
  }

  open() {
    this.child = spawn('ssh', [...SSH_ARGS, REMOTE, `sudo -n docker exec -i ${container(this.backend)} rcon-cli`], {
      stdio: ['pipe', 'ignore', 'ignore'],
    });
    return this;
  }

  send(command) {
    if (!this.child || this.child.exitCode !== null) this.open();
    this.child.stdin.write(`${command}\n`);
  }

  close() {
    if (this.child && this.child.exitCode === null) {
      this.child.stdin.end();
      this.child.kill('SIGTERM');
    }
  }
}

export class LogTail {
  constructor(command) {
    this.command = command;
    this.lines = [];
    this.child = null;
  }

  open() {
    this.child = spawn('ssh', [...SSH_ARGS, REMOTE, this.command], { stdio: ['ignore', 'pipe', 'pipe'] });
    let rest = '';
    const onData = (chunk) => {
      const parts = (rest + chunk.toString()).split('\n');
      rest = parts.pop();
      for (const text of parts) if (text.trim()) this.lines.push({ at: Date.now(), text });
    };
    this.child.stdout.on('data', onData);
    this.child.stderr.on('data', onData);
    return this;
  }

  mark() {
    return this.lines.length;
  }

  since(mark) {
    return this.lines.slice(mark);
  }

  async waitFor(pattern, { since = 0, timeoutMs = 30000 } = {}) {
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      const found = this.lines.slice(since).find((line) => pattern.test(line.text));
      if (found) return found;
      if (Date.now() > deadline) return null;
      await sleep(50);
    }
  }

  close() {
    if (this.child && this.child.exitCode === null) this.child.kill('SIGTERM');
  }
}

export function backendLog(backend) {
  return new LogTail(`sudo -n docker logs -f --tail 0 ${container(backend)}`).open();
}

export function nativeLog() {
  return new LogTail('sudo -n journalctl -u infrarust-lab -f -n 0 -o cat').open();
}

export function dockerProxyLog() {
  return new LogTail('sudo -n docker logs -f --tail 0 lab-proxy').open();
}

export class Admin {
  constructor(port) {
    this.base = `http://127.0.0.1:${port}/api/v1`;
  }

  async request(method, path, body) {
    const res = await fetch(`${this.base}${path}`, {
      method,
      headers: { Authorization: `Bearer ${API_KEY}`, ...(body === undefined ? {} : { 'Content-Type': 'application/json' }) },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const text = await res.text();
    let json = null;
    try {
      json = JSON.parse(text);
    } catch {
    }
    return { status: res.status, body: json, text };
  }

  get(path) {
    return this.request('GET', path);
  }

  post(path, body) {
    return this.request('POST', path, body);
  }

  async players() {
    const all = [];
    for (let page = 1; ; page++) {
      const res = await this.get(`/players?per_page=100&page=${page}`);
      if (res.status !== 200) throw new Error(`GET /players returned ${res.status}: ${res.text}`);
      all.push(...(res.body.data ?? []));
      if (page >= (res.body.meta?.total_pages ?? 1)) return all;
    }
  }

  async alive() {
    try {
      const res = await this.get('/proxy/update');
      return res.status === 200;
    } catch {
      return false;
    }
  }

  async waitAlive({ timeoutMs = 20000 } = {}) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      if (await this.alive()) return true;
      await sleep(200);
    }
    return false;
  }
}

export async function releases(patch) {
  const res = await fetch(`${RELEASES_URL}/_control`, {
    method: patch ? 'POST' : 'GET',
    headers: patch ? { 'Content-Type': 'application/json' } : {},
    body: patch ? JSON.stringify(patch) : undefined,
  });
  return res.json();
}
