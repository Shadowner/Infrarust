import { spawn } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import net from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { performance } from 'node:perf_hooks';
import { parseArgs } from 'node:util';

const READY = 'infrarust is ready, accepting connections';
const PROTOCOL = 769;

const { values: args } = parseArgs({
  options: {
    binary: { type: 'string' },
    sessions: { type: 'string', default: '1000' },
    'zero-copy': { type: 'string', default: '0.2' },
    tick: { type: 'string', default: '10' },
    settle: { type: 'string', default: '2000' },
    json: { type: 'boolean', default: false },
    keep: { type: 'boolean', default: false },
  },
});

const SESSIONS = Number(args.sessions);
const SPLICED = Math.round(SESSIONS * Number(args['zero-copy']));
const TICK_MS = Number(args.tick);
const SETTLE_MS = Number(args.settle);
const TIMEOUT_MS = 30000;

if (!args.binary) {
  console.error('error: --binary <path to infrarust> is required');
  process.exit(1);
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

function varint(value) {
  const out = [];
  do {
    let byte = value & 0x7f;
    value >>>= 7;
    if (value !== 0) byte |= 0x80;
    out.push(byte);
  } while (value !== 0);
  return Buffer.from(out);
}

function string(text) {
  const bytes = Buffer.from(text, 'utf8');
  return Buffer.concat([varint(bytes.length), bytes]);
}

function frame(body) {
  return Buffer.concat([varint(body.length), body]);
}

function hello(domain, name) {
  const port = Buffer.alloc(2);
  port.writeUInt16BE(25565);
  const handshake = Buffer.concat([varint(0), varint(PROTOCOL), string(domain), port, varint(2)]);
  const login = Buffer.concat([varint(0), string(name), Buffer.alloc(16, 7)]);
  return Buffer.concat([frame(handshake), frame(login)]);
}

function readVarint(buffer, offset) {
  let value = 0;
  for (let shift = 0; shift < 35; shift += 7) {
    if (offset >= buffer.length) return null;
    const byte = buffer[offset++];
    value |= (byte & 0x7f) << shift;
    if ((byte & 0x80) === 0) return { value, offset };
  }
  throw new Error('varint too long');
}

function splitHello(buffer) {
  let offset = 0;
  const frames = [];
  for (let i = 0; i < 2; i++) {
    const len = readVarint(buffer, offset);
    if (!len || len.offset + len.value > buffer.length) return null;
    frames.push(buffer.subarray(len.offset, len.offset + len.value));
    offset = len.offset + len.value;
  }
  const login = frames[1];
  const nameLen = readVarint(login, 1);
  const name = login.subarray(nameLen.offset, nameLen.offset + nameLen.value).toString('utf8');
  return { name, rest: buffer.subarray(offset) };
}

function number(n) {
  const bytes = Buffer.alloc(8);
  bytes.writeBigUInt64BE(BigInt(n));
  return bytes;
}

class Receiver {
  constructor(label) {
    this.label = label;
    this.expected = 0;
    this.pending = Buffer.alloc(0);
    this.lastAt = null;
    this.gap = { ms: 0, from: null, to: null };
    this.broken = null;
  }

  feed(chunk) {
    const now = performance.now();
    this.pending = this.pending.length ? Buffer.concat([this.pending, chunk]) : chunk;
    const whole = this.pending.length - (this.pending.length % 8);
    for (let offset = 0; offset < whole; offset += 8) {
      const n = Number(this.pending.readBigUInt64BE(offset));
      if (n !== this.expected && !this.broken) {
        this.broken = `${this.label}: expected ${this.expected}, got ${n}`;
      }
      this.expected = n + 1;
    }
    this.pending = this.pending.subarray(whole);
    if (whole === 0) return;
    if (this.lastAt !== null && now - this.lastAt > this.gap.ms) {
      this.gap = { ms: now - this.lastAt, from: this.lastAt, to: now };
    }
    this.lastAt = now;
  }
}

class Session {
  constructor(name, server) {
    this.name = name;
    this.server = server;
    this.sent = 0;
    this.up = new Receiver(`${name} up`);
    this.down = new Receiver(`${name} down`);
    this.client = null;
    this.backend = null;
    this.closed = [];
  }

  tick() {
    const bytes = number(this.sent++);
    this.client.write(bytes);
    this.backend.write(bytes);
  }

  settled() {
    return this.up.expected === this.sent && this.down.expected === this.sent;
  }
}

class Backend {
  constructor() {
    this.waiting = new Map();
    this.server = net.createServer((socket) => this.accept(socket));
  }

  async listen() {
    await new Promise((resolve) => this.server.listen(0, '127.0.0.1', resolve));
    return this.server.address().port;
  }

  accept(socket) {
    socket.setNoDelay(true);
    let buffer = Buffer.alloc(0);
    const onData = (chunk) => {
      buffer = Buffer.concat([buffer, chunk]);
      const parsed = splitHello(buffer);
      if (!parsed) return;
      socket.off('data', onData);
      const resolve = this.waiting.get(parsed.name);
      this.waiting.delete(parsed.name);
      resolve?.({ socket, rest: parsed.rest });
    };
    socket.on('data', onData);
  }

  expect(name) {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`the backend never saw ${name}`)), TIMEOUT_MS);
      this.waiting.set(name, (value) => {
        clearTimeout(timer);
        resolve(value);
      });
    });
  }
}

async function freePort() {
  const server = net.createServer();
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const { port } = server.address();
  await new Promise((resolve) => server.close(resolve));
  return port;
}

class Proxy {
  constructor(binary, dir, config) {
    this.lines = [];
    const env = { ...process.env, NO_COLOR: '1' };
    delete env.RUST_LOG;
    this.child = spawn(binary, ['-c', config, '-l', 'info'], {
      cwd: dir,
      env,
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    let partial = '';
    const onData = (chunk) => {
      const text = partial + chunk.toString();
      const parts = text.split('\n');
      partial = parts.pop();
      for (const line of parts) if (line.trim()) this.lines.push(line);
    };
    this.child.stdout.on('data', onData);
    this.child.stderr.on('data', onData);
  }

  count(needle) {
    return this.lines.filter((line) => line.includes(needle)).length;
  }

  async waitFor(needle, times) {
    const deadline = Date.now() + TIMEOUT_MS;
    while (this.count(needle) < times) {
      if (this.child.exitCode !== null) throw new Error(`the proxy exited while waiting for ${needle}:\n${this.tail()}`);
      if (Date.now() > deadline) throw new Error(`timed out waiting for ${needle} x${times}:\n${this.tail()}`);
      await sleep(5);
    }
  }

  find(needle) {
    return this.lines.filter((line) => line.includes(needle));
  }

  tail(n = 40) {
    return this.lines.slice(-n).join('\n');
  }

  async stop() {
    if (this.child.exitCode !== null) return;
    this.child.kill('SIGTERM');
    const deadline = Date.now() + TIMEOUT_MS;
    while (this.child.exitCode === null && Date.now() < deadline) await sleep(20);
    if (this.child.exitCode === null) this.child.kill('SIGKILL');
  }
}

function site(backendPort, proxyPort) {
  const dir = mkdtempSync(join(tmpdir(), 'infrarust-cutover-'));
  const servers = join(dir, 'servers');
  const plugins = join(dir, 'plugins');
  mkdirSync(servers);
  mkdirSync(plugins);
  for (const [id, mode] of [['pipe', 'passthrough'], ['splice', 'zero_copy']]) {
    writeFileSync(
      join(servers, `${id}.toml`),
      `domains = ["${id}.test"]\naddresses = ["127.0.0.1:${backendPort}"]\nproxy_mode = "${mode}"\n`,
    );
  }
  const config = join(dir, 'infrarust.toml');
  writeFileSync(
    config,
    [
      `bind = "127.0.0.1:${proxyPort}"`,
      `servers_dir = ${JSON.stringify(servers)}`,
      `plugins_dir = ${JSON.stringify(plugins)}`,
      'max_connections_per_ip = 0',
      '',
      '[rate_limit]',
      'enabled = false',
      '',
      '[update]',
      'check = false',
      '',
      '[ban]',
      `file = ${JSON.stringify(join(dir, 'bans.json'))}`,
      '',
    ].join('\n'),
  );
  return { dir, config };
}

async function connectSession(session, proxyPort, backend) {
  const arrival = backend.expect(session.name);
  const client = net.connect({ host: '127.0.0.1', port: proxyPort });
  client.setNoDelay(true);
  await new Promise((resolve, reject) => {
    client.once('connect', resolve);
    client.once('error', reject);
  });
  client.write(hello(`${session.server}.test`, session.name));
  const { socket, rest } = await arrival;
  session.client = client;
  session.backend = socket;
  client.on('data', (chunk) => session.down.feed(chunk));
  socket.on('data', (chunk) => session.up.feed(chunk));
  if (rest.length) session.up.feed(rest);
  for (const [side, stream] of [['client', client], ['backend', socket]]) {
    stream.on('close', () => session.closed.push(side));
    stream.on('error', (error) => session.closed.push(`${side}: ${error.message}`));
  }
}

function percentile(sorted, p) {
  if (!sorted.length) return NaN;
  const index = Math.min(sorted.length - 1, Math.ceil((p / 100) * sorted.length) - 1);
  return sorted[Math.max(0, index)];
}

function summary(gaps) {
  const sorted = gaps.map((gap) => gap.ms).sort((a, b) => a - b);
  return {
    min: sorted[0],
    p50: percentile(sorted, 50),
    p95: percentile(sorted, 95),
    p99: percentile(sorted, 99),
    max: sorted[sorted.length - 1],
  };
}

function format(stats) {
  return ['min', 'p50', 'p95', 'p99', 'max'].map((key) => `${key} ${stats[key].toFixed(1)}`).join('  ');
}

async function main() {
  const backend = new Backend();
  const backendPort = await backend.listen();
  const proxyPort = await freePort();
  const { dir, config } = site(backendPort, proxyPort);
  const proxy = new Proxy(args.binary, dir, config);
  let ticker = null;
  try {
    await proxy.waitFor(READY, 1);
    const sessions = [];
    for (let i = 0; i < SESSIONS; i++) {
      sessions.push(new Session(`S${i}`, i < SESSIONS - SPLICED ? 'pipe' : 'splice'));
    }
    for (let i = 0; i < sessions.length; i += 50) {
      await Promise.all(sessions.slice(i, i + 50).map((session) => connectSession(session, proxyPort, backend)));
    }

    ticker = setInterval(() => {
      for (const session of sessions) session.tick();
    }, TICK_MS);
    await sleep(SETTLE_MS);

    const signalledAt = performance.now();
    proxy.child.kill('SIGUSR2');
    await proxy.waitFor('handover committed', 1);
    await proxy.waitFor(READY, 2);
    const readyAt = performance.now();
    await sleep(SETTLE_MS);
    clearInterval(ticker);
    ticker = null;

    const deadline = Date.now() + TIMEOUT_MS;
    while (!sessions.every((session) => session.settled())) {
      if (Date.now() > deadline) throw new Error('some numbers never arrived after the restart');
      await sleep(10);
    }

    const streams = sessions.flatMap((session) => [session.up, session.down]);
    const broken = streams.map((stream) => stream.broken).filter(Boolean);
    const closed = sessions.filter((session) => session.closed.length).map((session) => `${session.name}: ${session.closed.join(', ')}`);
    const up = sessions.map((session) => session.up.gap);
    const down = sessions.map((session) => session.down.gap);
    const all = [...up, ...down];
    const lastOld = Math.max(...all.map((gap) => gap.from));
    const firstNew = Math.min(...all.map((gap) => gap.to));
    const result = {
      binary: args.binary,
      sessions: SESSIONS,
      passthrough: SESSIONS - SPLICED,
      zero_copy: SPLICED,
      tick_ms: TICK_MS,
      restart_ms: readyAt - signalledAt,
      proxy: {
        package: proxy.find('the handover package is ready').at(-1) ?? null,
        resumed: proxy.find('inherited sessions resumed').at(-1) ?? null,
      },
      cutover_ms: {
        client_to_backend: summary(up),
        backend_to_client: summary(down),
        every_stream_paused_ms: firstNew - lastOld,
        pause_window_ms: Math.max(...all.map((gap) => gap.to)) - Math.min(...all.map((gap) => gap.from)),
      },
      numbers_per_stream: sessions[0].sent,
      broken,
      closed,
    };
    if (args.json) {
      console.log(JSON.stringify(result, null, 2));
    } else {
      console.log(`binary      ${result.binary}`);
      console.log(`sessions    ${SESSIONS} (${result.passthrough} passthrough, ${result.zero_copy} zero_copy), one u64 each way every ${TICK_MS} ms`);
      console.log(`restart     SIGUSR2 to the second "ready" line: ${result.restart_ms.toFixed(1)} ms`);
      console.log(`package     ${result.proxy.package}`);
      console.log(`resumed     ${result.proxy.resumed}`);
      console.log(`cutover up  ${format(result.cutover_ms.client_to_backend)} ms (largest gap per stream)`);
      console.log(`cutover dn  ${format(result.cutover_ms.backend_to_client)} ms`);
      console.log(`all paused  ${result.cutover_ms.every_stream_paused_ms.toFixed(1)} ms (last byte before the gap on any stream to the first byte after it on any stream)`);
      console.log(`window      ${result.cutover_ms.pause_window_ms.toFixed(1)} ms (first stream paused to last stream resumed)`);
      console.log(`integrity   ${broken.length} broken sequences, ${closed.length} closed sessions, ${result.numbers_per_stream} numbers per stream`);
    }
    const failed = broken.length > 0 || closed.length > 0;
    await proxy.stop();
    for (const session of sessions) {
      session.client.destroy();
      session.backend.destroy();
    }
    backend.server.close();
    process.exitCode = failed ? 1 : 0;
  } catch (error) {
    console.error(String(error?.stack ?? error));
    process.exitCode = 1;
  } finally {
    if (ticker) clearInterval(ticker);
    await proxy.stop();
    backend.server.close();
    if (!args.keep) rmSync(dir, { recursive: true, force: true });
    setTimeout(() => process.exit(process.exitCode ?? 0), 100).unref();
  }
}

main();
