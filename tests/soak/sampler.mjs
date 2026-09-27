import { appendFileSync, closeSync, openSync, readFileSync, readSync, readdirSync, readlinkSync, statSync } from 'node:fs';

function arg(name, fallback) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : fallback;
}

const pid = Number(arg('pid', '0'));
const logFile = arg('log', null);
const out = arg('out', null);
const everyMs = Number(arg('every', '5')) * 1000;
const witnessFile = arg('witness', null);
const backendFile = arg('backend', null);
const api = arg('api', null);
const apiKey = arg('api-key', '');
const watchFile = arg('watch', null);
const lastAux = new Map();

function readAux(now) {
  if (!watchFile) return null;
  let entries;
  try {
    entries = readFileSync(watchFile, 'utf8').split('\n').filter(Boolean).map((l) => l.split('='));
  } catch {
    return null;
  }
  const result = {};
  for (const [name, pidText] of entries) {
    try {
      const stat = readFileSync(`/proc/${pidText}/stat`, 'utf8');
      const fields = stat.slice(stat.lastIndexOf(')') + 2).split(' ');
      const cpuS = (Number(fields[11]) + Number(fields[12])) / 100;
      const prev = lastAux.get(name);
      lastAux.set(name, { cpuS, at: now });
      result[name] = prev ? Math.round(((cpuS - prev.cpuS) / ((now - prev.at) / 1000)) * 1000) / 10 : null;
    } catch {
      result[name] = null;
    }
  }
  return result;
}

const PATTERNS = {
  instanceFailed: /wasm plugin instance failed; discarding it/,
  recovered: /wasm plugin recovered: a fresh instance is enabled/,
  quarantined: /wasm plugin quarantined: it kept failing/,
  queueFull: /wasm plugin call queue is full/,
  expiredQueued: /deadline passed while it waited/,
  codecTrap: /wasm codec filter trapped; passing through/,
  codecCreateFailed: /codec instance create failed/,
  cpuBudget: /exceeded CPU budget/,
  panicked: /panicked at/,
  slowListener: /slow (event )?(listener|handler)|took longer than/i,
  listenerTimeout: /listener timed out|handler timed out|timed out after/i,
  taskAbnormal: /ended abnormally/,
};

const startedAt = Date.now();
let logOffset = 0;
let partial = '';
const cumulative = { lines: 0, warn: 0, error: 0 };
for (const k of Object.keys(PATTERNS)) cumulative[k] = 0;
let lastCpu = null;

function readProc() {
  try {
    const status = readFileSync(`/proc/${pid}/status`, 'utf8');
    const field = (name) => {
      const m = new RegExp(`^${name}:\\s+(\\d+)`, 'm').exec(status);
      return m ? Number(m[1]) : null;
    };
    const stat = readFileSync(`/proc/${pid}/stat`, 'utf8');
    const fields = stat.slice(stat.lastIndexOf(')') + 2).split(' ');
    const cpu = (Number(fields[11]) + Number(fields[12])) / 100;
    let fds = null;
    try {
      fds = readdirSync(`/proc/${pid}/fd`).length;
    } catch {
    }
    let sockets = null;
    try {
      sockets = readdirSync(`/proc/${pid}/fd`).filter((fd) => {
        try {
          return readlinkSync(`/proc/${pid}/fd/${fd}`).startsWith('socket:');
        } catch {
          return false;
        }
      }).length;
    } catch {
    }
    return {
      alive: true,
      rssKb: field('VmRSS'),
      hwmKb: field('VmHWM'),
      vmKb: field('VmSize'),
      threads: field('Threads'),
      fds,
      sockets,
      cpuS: cpu,
    };
  } catch {
    return { alive: false };
  }
}

function readLog() {
  const counts = { lines: 0, warn: 0, error: 0 };
  for (const k of Object.keys(PATTERNS)) counts[k] = 0;
  if (!logFile) return counts;
  let size;
  try {
    size = statSync(logFile).size;
  } catch {
    return counts;
  }
  if (size <= logOffset) return counts;
  const fd = openSync(logFile, 'r');
  const length = size - logOffset;
  const buffer = Buffer.alloc(length);
  readSync(fd, buffer, 0, length, logOffset);
  closeSync(fd);
  logOffset = size;
  const text = partial + buffer.toString('utf8');
  const lines = text.split('\n');
  partial = lines.pop() ?? '';
  for (const raw of lines) {
    const line = raw.replace(/\x1b\[[0-9;]*m/g, '');
    counts.lines += 1;
    if (/\bWARN\b/.test(line)) counts.warn += 1;
    if (/\bERROR\b/.test(line)) counts.error += 1;
    for (const [k, re] of Object.entries(PATTERNS)) if (re.test(line)) counts[k] += 1;
  }
  for (const k of Object.keys(counts)) cumulative[k] += counts[k];
  return counts;
}

let lastHostCpu = null;
function readHost() {
  try {
    const load = readFileSync('/proc/loadavg', 'utf8').split(' ');
    const cpuLine = readFileSync('/proc/stat', 'utf8').split('\n')[0].trim().split(/\s+/).slice(1).map(Number);
    const idle = cpuLine[3] + cpuLine[4];
    const total = cpuLine.reduce((a, b) => a + b, 0);
    let busyPct = null;
    if (lastHostCpu) {
      const dt = total - lastHostCpu.total;
      busyPct = dt > 0 ? Math.round((1 - (idle - lastHostCpu.idle) / dt) * 1000) / 10 : null;
    }
    lastHostCpu = { idle, total };
    return { load1: Number(load[0]), busyPct };
  } catch {
    return null;
  }
}

function readWitness() {
  if (!witnessFile) return null;
  try {
    const text = readFileSync(witnessFile, 'utf8');
    const result = {};
    for (const line of text.split('\n')) {
      const [k, v] = line.split('=');
      if (k && v !== undefined) result[k.trim()] = Number(v);
    }
    return result;
  } catch {
    return null;
  }
}

function readBackend() {
  if (!backendFile) return null;
  try {
    return JSON.parse(readFileSync(backendFile, 'utf8')).backends;
  } catch {
    return null;
  }
}

async function readApi() {
  if (!api) return null;
  const get = async (path) => {
    const res = await fetch(`${api}${path}`, { headers: { Authorization: `Bearer ${apiKey}` }, signal: AbortSignal.timeout(2000) });
    if (!res.ok) return null;
    return (await res.json()).data ?? null;
  };
  try {
    const plugins = await get('/api/v1/plugins');
    const count = await get('/api/v1/players/count');
    return {
      plugins: plugins ? Object.fromEntries(plugins.map((p) => [p.id, p.state])) : null,
      players: count && typeof count === 'object' ? (count.count ?? count.online ?? count.total ?? JSON.stringify(count)) : count,
    };
  } catch (err) {
    return { error: String(err?.message ?? err) };
  }
}

async function sample() {
  const now = Date.now();
  const proc = readProc();
  let cpuPct = null;
  if (proc.alive && lastCpu) cpuPct = Math.round(((proc.cpuS - lastCpu.cpuS) / ((now - lastCpu.at) / 1000)) * 1000) / 10;
  if (proc.alive) lastCpu = { cpuS: proc.cpuS, at: now };
  const log = readLog();
  const record = {
    t: Math.round((now - startedAt) / 1000),
    host: readHost(),
    aux: readAux(now),
    proc: { ...proc, cpuPct },
    log,
    witness: readWitness(),
    backend: readBackend(),
    api: await readApi(),
  };
  const line = JSON.stringify(record);
  if (out) appendFileSync(out, `${line}\n`);
  else process.stdout.write(`${line}\n`);
  return proc.alive;
}

let stopped = false;
async function loop() {
  while (!stopped) {
    const alive = await sample();
    if (!alive) break;
    await new Promise((resolve) => setTimeout(resolve, everyMs));
  }
  process.exit(0);
}

process.on('SIGTERM', () => {
  stopped = true;
});
process.on('SIGINT', () => {
  stopped = true;
});
loop();
