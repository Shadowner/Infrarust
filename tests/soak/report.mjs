import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

const dir = process.argv[2];
if (!dir) {
  process.stderr.write('usage: node report.mjs <run dir>\n');
  process.exit(2);
}

function jsonl(name) {
  const p = join(dir, name);
  if (!existsSync(p)) return [];
  return readFileSync(p, 'utf8')
    .split('\n')
    .filter(Boolean)
    .map((l) => {
      try {
        return JSON.parse(l);
      } catch {
        return null;
      }
    })
    .filter(Boolean);
}

function json(name) {
  const p = join(dir, name);
  if (!existsSync(p)) return null;
  try {
    return JSON.parse(readFileSync(p, 'utf8'));
  } catch {
    return null;
  }
}

function keyvals(name) {
  const p = join(dir, name);
  if (!existsSync(p)) return null;
  const result = {};
  for (const line of readFileSync(p, 'utf8').split('\n')) {
    const [k, v] = line.split('=');
    if (k && v !== undefined) result[k.trim()] = Number(v);
  }
  return result;
}

function slope(points) {
  if (points.length < 3) return null;
  const n = points.length;
  const mx = points.reduce((s, p) => s + p[0], 0) / n;
  const my = points.reduce((s, p) => s + p[1], 0) / n;
  let num = 0;
  let den = 0;
  for (const [x, y] of points) {
    num += (x - mx) * (y - my);
    den += (x - mx) ** 2;
  }
  return den === 0 ? null : num / den;
}

function pad(v, w) {
  const s = v === null || v === undefined ? '-' : String(v);
  return s.length >= w ? s : ' '.repeat(w - s.length) + s;
}

function table(headers, rows) {
  const widths = headers.map((h, i) => Math.max(h.length, ...rows.map((r) => String(r[i] ?? '-').length)));
  const line = (cells) => cells.map((c, i) => pad(c, widths[i])).join('  ');
  return [line(headers), ...rows.map(line)].join('\n');
}

const out = [];
const say = (s = '') => out.push(s);

const samples = jsonl('samples.jsonl');
const bots = jsonl('bots.jsonl');
const burst = jsonl('burst.jsonl');
const attackers = jsonl('attackers.jsonl');
const attackersSummary = json('attackers-summary.json');
const post = jsonl('post.jsonl');
const postSummary = json('post-summary.json');
const shutdown = json('shutdown.json');
const botSummary = json('bots-summary.json');
const burstSummary = json('burst-summary.json');
const backend = json('backend.json');
const witness = keyvals('witness-final.txt') ?? keyvals('witness-before-shutdown.txt');

say(`# Soak report for ${dir}`);
say();

if (samples.length > 0) {
  const perMinute = new Map();
  for (const s of samples) {
    const m = Math.floor(s.t / 60);
    if (!perMinute.has(m)) perMinute.set(m, []);
    perMinute.get(m).push(s);
  }
  const rows = [];
  for (const [m, list] of [...perMinute.entries()].sort((a, b) => a[0] - b[0])) {
    const alive = list.filter((s) => s.proc.alive);
    const last = alive[alive.length - 1];
    const sum = (k) => list.reduce((acc, s) => acc + (s.log?.[k] ?? 0), 0);
    const cpu = alive.filter((s) => s.proc.cpuPct !== null).map((s) => s.proc.cpuPct);
    rows.push([
      m,
      last ? Math.round(last.proc.rssKb / 1024) : null,
      last ? Math.round(last.proc.vmKb / 1024) : null,
      last?.proc.threads,
      last?.proc.fds,
      last?.proc.sockets,
      cpu.length ? Math.round(cpu.reduce((a, b) => a + b, 0) / cpu.length) : null,
      (() => {
        const busy = list.map((s) => s.host?.busyPct).filter((v) => typeof v === 'number');
        return busy.length ? Math.round(busy.reduce((a, b) => a + b, 0) / busy.length) : null;
      })(),
      Math.max(...list.map((s) => s.host?.load1 ?? 0)),
      last?.api?.players,
      sum('warn'),
      sum('error'),
      sum('instanceFailed'),
      sum('recovered'),
      sum('quarantined'),
      sum('queueFull'),
      sum('codecTrap'),
      sum('cpuBudget'),
      sum('panicked'),
    ]);
  }
  say('## Proxy process, per minute (last sample of the minute; log counts summed)');
  say();
  say('```');
  say(table(['min', 'rssMB', 'vmMB', 'thr', 'fds', 'socks', 'cpu%', 'host%', 'load', 'players', 'warn', 'error', 'failed', 'recov', 'quar', 'qfull', 'codec', 'cpu', 'panic'], rows));
  say('```');
  say();
  const alive = samples.filter((s) => s.proc.alive && s.proc.rssKb);
  const steady = alive.filter((s) => s.t >= 120);
  const use = steady.length >= 5 ? steady : alive;
  const rssSlope = slope(use.map((s) => [s.t / 3600, s.proc.rssKb / 1024]));
  const fdSlope = slope(use.map((s) => [s.t / 3600, s.proc.fds]));
  const thrSlope = slope(use.map((s) => [s.t / 3600, s.proc.threads]));
  const first = use[0];
  const last = use[use.length - 1];
  say(`RSS trend after t=120s: ${rssSlope?.toFixed(1)} MB/hour (from ${Math.round(first.proc.rssKb / 1024)} MB at t=${first.t}s to ${Math.round(last.proc.rssKb / 1024)} MB at t=${last.t}s, peak VmHWM ${Math.round(last.proc.hwmKb / 1024)} MB)`);
  say(`fd trend: ${fdSlope?.toFixed(1)} fds/hour, thread trend: ${thrSlope?.toFixed(1)} threads/hour`);
  const lastApi = [...samples].reverse().find((s) => s.api?.plugins);
  if (lastApi) say(`plugin states (admin API, last sample): ${JSON.stringify(lastApi.api.plugins)}`);
  say();
}

function botTable(title, windows) {
  if (windows.length === 0) return;
  const perMinute = new Map();
  for (const w of windows) {
    const m = Math.floor((w.t - 1) / 60);
    if (!perMinute.has(m)) perMinute.set(m, []);
    perMinute.get(m).push(w);
  }
  const rows = [];
  for (const [m, list] of [...perMinute.entries()].sort((a, b) => a[0] - b[0])) {
    const s = (f) => list.reduce((acc, w) => acc + (f(w) ?? 0), 0);
    const mx = (f) => Math.max(...list.map((w) => f(w) ?? 0));
    const cmdKinds = new Set(list.flatMap((w) => Object.keys(w.cmd)));
    const cmdCells = [...cmdKinds].sort().map((k) => {
      const sent = s((w) => w.cmd[k]?.sent);
      const replied = s((w) => w.cmd[k]?.replied);
      const p99 = mx((w) => w.cmd[k]?.p99);
      return `${k}:${replied}/${sent}@${Math.round(p99)}`;
    });
    rows.push([
      m,
      mx((w) => w.active),
      s((w) => w.login.ok),
      s((w) => w.login.fail),
      Math.round(mx((w) => w.login.p50)),
      Math.round(mx((w) => w.login.p99)),
      Math.round(mx((w) => w.login.max)),
      s((w) => w.chat.sent),
      s((w) => w.chat.lost),
      Math.round(mx((w) => w.chat.p50) * 10) / 10,
      Math.round(mx((w) => w.chat.p99)),
      `${s((w) => w.switch.ok)}/${s((w) => w.switch.sent)}`,
      s((w) => w.drops),
      cmdCells.join(' '),
    ]);
  }
  say(`## ${title}, per minute (latencies in ms; p-values are the worst 10 s window of the minute)`);
  say();
  say('```');
  say(table(['min', 'active', 'login', 'lfail', 'l.p50', 'l.p99', 'l.max', 'chats', 'lost', 'c.p50', 'c.p99', 'switch', 'drops', 'commands replied/sent@p99'], rows));
  say('```');
  say();
}

botTable('Bots', bots);
botTable('Burst bots', burst);
botTable('Attacker bots (127.0.0.2, faulty codec targets)', attackers);
botTable('Post-load probe bots', post);

function summaryBlock(title, s) {
  if (!s) return;
  say(`## ${title} totals`);
  say();
  say('```');
  say(`logins ok ${s.login.ok}, failed ${s.login.fail}, login p50 ${s.login.p50} ms p95 ${s.login.p95} ms p99 ${s.login.p99} ms max ${s.login.max} ms`);
  if (Object.keys(s.login.reasons).length) say(`login failure reasons: ${JSON.stringify(s.login.reasons)}`);
  say(`chats sent ${s.chat.sent}, echoed ${s.chat.echoed}, lost ${s.chat.lost}, cut ${s.chat.cut}, late ${s.chat.late}, rtt p50 ${s.chat.p50} p99 ${s.chat.p99} max ${s.chat.max}`);
  for (const [k, c] of Object.entries(s.cmd)) say(`/${k}: sent ${c.sent}, replied ${c.replied}, timeout ${c.timeout}, cut ${c.cut}, p50 ${c.p50} p99 ${c.p99} max ${c.max}`);
  say(`switches sent ${s.switch.sent}, ok ${s.switch.ok}, err ${s.switch.err}, confirmed by the other backend ${s.switch.confirmed}; reasons ${JSON.stringify(s.switch.reasons)}`);
  say(`unplanned drops ${s.drops}: ${JSON.stringify(s.dropReasons)}`);
  say(`sessions ${s.sessions}, aborted logins ${s.aborted}, abrupt closes ${s.abrupt}`);
  say('```');
  say();
}

summaryBlock('Bot', botSummary);
summaryBlock('Burst', burstSummary);
summaryBlock('Attacker', attackersSummary);
summaryBlock('Post-load probe', postSummary);

if (witness) {
  say('## Healthy WASM witness counters (guest memory, written every 2 s)');
  say();
  say('```');
  say(JSON.stringify(witness));
  say('```');
  say();
}

if (backend) {
  say('## Backends');
  say();
  say('```');
  say(JSON.stringify(backend));
  say('```');
  say();
}

const benchPath = join(dir, 'bench.log');
if (existsSync(benchPath)) {
  const text = readFileSync(benchPath, 'utf8');
  const windows = text.split('=== window ').slice(1);
  const rows = windows.map((w) => {
    const n = w.split(' ')[0];
    const num = (re) => {
      const m = re.exec(w);
      return m ? m[1] : null;
    };
    return [
      n,
      num(/connections:\s+(\d+) ok/),
      num(/(\d+) failed/),
      num(/pings sent:\s+(\d+)/),
      num(/echoes:\s+(\d+)/),
      num(/p50\s+([\d.]+)/),
      num(/p99\s+([\d.]+)/),
      num(/p99\.9\s+([\d.]+)/),
      num(/max\s+([\d.]+)/),
    ];
  });
  if (rows.length) {
    say('## mc-bench packet round trips through the codec filters (microseconds)');
    say();
    say('```');
    say(table(['win', 'ok', 'fail', 'sent', 'echoes', 'p50', 'p99', 'p99.9', 'max'], rows));
    say('```');
    say();
  }
}

if (shutdown) {
  say(`## Shutdown: ${JSON.stringify(shutdown)}`);
  say();
}

const logPath = join(dir, 'proxy.log');
if (existsSync(logPath)) {
  const groups = new Map();
  for (const raw of readFileSync(logPath, 'utf8').split('\n')) {
    const line = raw.replace(/\x1b\[[0-9;]*m/g, '');
    const level = /\b(WARN|ERROR)\b/.exec(line);
    if (!level) continue;
    const body = line
      .slice(level.index)
      .replace(/\b[0-9a-f]{8}-[0-9a-f-]{27}\b/g, '<uuid>')
      .replace(/\d+(\.\d+)?(ms|µs|s)\b/g, '<dur>')
      .replace(/\b[a-z]\d+_\d+\b/g, '<bot>')
      .replace(/\d+/g, 'N')
      .slice(0, 260);
    groups.set(body, (groups.get(body) ?? 0) + 1);
  }
  const top = [...groups.entries()].sort((a, b) => b[1] - a[1]).slice(0, 25);
  say('## WARN/ERROR lines grouped (digits folded)');
  say();
  say('```');
  for (const [body, count] of top) say(`${pad(count, 7)}  ${body}`);
  say('```');
}

process.stdout.write(`${out.join('\n')}\n`);
