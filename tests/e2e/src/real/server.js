import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { download, resolveJava, versionJson } from './mojang.js';

const PAPER_API = 'https://fill.papermc.io/v3/projects/paper';
const PAPER_HEADERS = { 'User-Agent': 'infrarust-e2e/1.0 (+https://github.com/shadowner/infrarust)' };

function properties({ port, onlineMode, extra = {} }) {
  const props = {
    'server-port': port,
    'server-ip': '127.0.0.1',
    'online-mode': onlineMode ? 'true' : 'false',
    'level-type': 'flat',
    'generate-structures': 'false',
    'spawn-protection': '0',
    'view-distance': '3',
    'simulation-distance': '3',
    'spawn-animals': 'false',
    'spawn-monsters': 'false',
    'spawn-npcs': 'false',
    'sync-chunk-writes': 'false',
    'enable-command-block': 'false',
    'network-compression-threshold': '256',
    'max-tick-time': '-1',
    motd: 'infrarust e2e',
    ...extra,
  };
  return Object.entries(props)
    .map(([k, v]) => `${k}=${v}`)
    .join('\n');
}

export async function fetchVanillaServer(paths, id) {
  const vjson = await versionJson(paths, id);
  const server = vjson.downloads?.server;
  if (!server) throw new Error(`${id} publishes no server jar`);
  const jar = join(paths.servers, 'jars', `vanilla-${id}.jar`);
  await download(server.url, jar, { sha1: server.sha1 });
  return { jar, vjson };
}

export async function fetchPaperServer(paths, id) {
  const res = await fetch(`${PAPER_API}/versions/${id}/builds/latest`, { headers: PAPER_HEADERS });
  if (!res.ok) return null;
  const build = await res.json();
  const artifact = build.downloads?.['server:default'];
  if (!artifact) return null;
  const jar = join(paths.servers, 'jars', artifact.name);
  await download(artifact.url, jar, { sha256: artifact.checksums?.sha256, headers: PAPER_HEADERS });
  return { jar, build: build.id };
}

export class Server {
  constructor({ jar, java, dir, port, onlineMode = false, extraProps = {}, jvmArgs = [] }) {
    Object.assign(this, { jar, java, dir, port, onlineMode, extraProps, jvmArgs });
    this.lines = [];
    this.child = null;
  }

  async start({ timeoutMs = 180000 } = {}) {
    mkdirSync(this.dir, { recursive: true });
    rmSync(join(this.dir, 'world', 'session.lock'), { force: true });
    writeFileSync(join(this.dir, 'eula.txt'), 'eula=true\n');
    writeFileSync(
      join(this.dir, 'server.properties'),
      properties({ port: this.port, onlineMode: this.onlineMode, extra: this.extraProps }),
    );

    this.child = spawn(this.java, [...this.jvmArgs, '-Xms512M', '-Xmx1200M', '-jar', this.jar, 'nogui'], {
      cwd: this.dir,
      stdio: ['pipe', 'pipe', 'pipe'],
    });

    const onData = (chunk) => {
      for (const line of chunk.toString().split('\n')) if (line.trim()) this.lines.push(line);
    };
    this.child.stdout.on('data', onData);
    this.child.stderr.on('data', onData);

    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      if (this.child.exitCode !== null) {
        throw new Error(`server exited ${this.child.exitCode} before it was ready:\n${this.tail(30)}`);
      }
      if (this.lines.some((l) => /Done \(/.test(l))) return this;
      await sleep(200);
    }
    throw new Error(`server never reported Done in ${timeoutMs}ms:\n${this.tail(30)}`);
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

  async quiesce({ quietMs = 600, timeoutMs = 8000 } = {}) {
    const deadline = Date.now() + timeoutMs;
    let last = this.lines.length;
    let stableSince = Date.now();
    while (Date.now() < deadline) {
      await sleep(150);
      if (this.lines.length !== last) {
        last = this.lines.length;
        stableSince = Date.now();
      } else if (Date.now() - stableSince >= quietMs) {
        return;
      }
    }
  }

  async waitForJoin(username, { since = 0, timeoutMs = 90000, otherPlayers = /\bE\d\ds\d\d\b/ } = {}) {
    const joinedLine = `${username} joined the game`;
    const loggedIn = new RegExp(`\\b${username}\\b\\[[^\\]]*\\] logged in with entity id`);
    const deadline = Date.now() + timeoutMs;
    const mine = (line) => {
      const named = line.match(otherPlayers);
      return !named || named[0] === username;
    };
    for (;;) {
      const window = this.lines.slice(since);
      const joined = window.find((l) => l.includes(joinedLine)) ?? window.find((l) => loggedIn.test(l));
      if (joined) return { joined: true, line: joined, window };
      const refused = window
        .filter(mine)
        .find((l) => /Disconnecting|Failed to verify|Internal Exception|lost connection|Kicking/i.test(l));
      if (refused) return { joined: false, line: refused, window };
      if (Date.now() > deadline) return { joined: false, line: null, window };
      await sleep(200);
    }
  }

  hasVelocityForwarding() {
    return this.lines.some((l) => /Velocity/.test(l) && !/(compression|cipher) from Velocity/.test(l));
  }

  say(message) {
    try {
      this.child.stdin.write(`say ${message}\n`);
      return true;
    } catch {
      return false;
    }
  }

  async stop({ timeoutMs = 20000 } = {}) {
    if (!this.child || this.child.exitCode !== null) return;
    try {
      this.child.stdin.write('stop\n');
    } catch {
    }
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline && this.child.exitCode === null) await sleep(200);
    if (this.child.exitCode === null) this.child.kill('SIGKILL');
  }
}

export async function prepareServer(paths, id, { flavour = 'vanilla', port, dir, onlineMode = false, velocitySecret } = {}) {
  const vjson = await versionJson(paths, id);
  const { java } = await resolveJava(paths, vjson);

  let jar;
  if (flavour === 'paper') {
    const paper = await fetchPaperServer(paths, id);
    if (!paper) throw new Error(`Paper publishes no build for ${id}`);
    jar = paper.jar;
  } else {
    ({ jar } = await fetchVanillaServer(paths, id));
  }

  const server = new Server({ jar, java, dir, port, onlineMode });

  if (flavour === 'paper' && velocitySecret) {
    writeVelocityConfig(dir, id, velocitySecret);
  }
  return server;
}

function writeVelocityConfig(dir, id, secret) {
  const block = ['proxies:', '  velocity:', '    enabled: true', '    online-mode: true', `    secret: ${secret}`, ''].join('\n');
  mkdirSync(join(dir, 'config'), { recursive: true });
  writeFileSync(join(dir, 'config', 'paper-global.yml'), block);
  writeFileSync(join(dir, 'paper.yml'), ['settings:', '  velocity-support:', '    enabled: true', '    online-mode: true', `    secret: ${secret}`, ''].join('\n'));
}

export function writeBungeeConfig(dir) {
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, 'spigot.yml'), ['settings:', '  bungeecord: true', ''].join('\n'));
}

export function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}
