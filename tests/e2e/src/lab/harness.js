import { join } from 'node:path';

import { Admin, Console, backendLog, dockerProxyLog, nativeLog, sh, sleep } from './remote.js';
import { PORTS, REMOTE_ROOT, route } from './topology.js';

export const COMMITTED = /handover committed/;
export const RESUMED = /player resumed after the restart/;
export const AUTHENTICATED = /client authenticated/;
export const ROLLED_BACK = /handing the sessions back to the previous one|rolling the hot restart back/;
export const FAILED = /hot restart failed|restart request is refused|hot restart is unavailable/;

export class NativeProxy {
  constructor() {
    this.kind = 'native';
    this.port = PORTS.native.game;
    this.admin = new Admin(PORTS.native.admin);
    this.log = nativeLog();
    this.home = `${REMOTE_ROOT}/native`;
  }

  async pid() {
    const out = await sh('systemctl show -p MainPID --value infrarust-lab');
    return Number(out.stdout.trim());
  }

  async active() {
    const out = await sh('systemctl is-active infrarust-lab', { allowFailure: true });
    return out.stdout.trim();
  }

  signal(name) {
    return sh(`sudo -n systemctl kill -s ${name} --kill-who=main infrarust-lab`);
  }

  reload() {
    return sh('sudo -n systemctl reload infrarust-lab');
  }

  async install(name) {
    await sh(`bash ${REMOTE_ROOT}/src/tests/e2e/lab/remote.sh install ${name}`);
    if (!(await this.admin.waitAlive())) throw new Error(`the native proxy did not come up with build ${name}`);
  }

  cli(args, opts = {}) {
    return sh(`cd ${this.home} && ./infrarust --config infrarust.toml ${args} < /dev/null`, { allowFailure: true, ...opts });
  }

  async binarySha() {
    const out = await sh(`sha256sum ${this.home}/infrarust | cut -d' ' -f1`);
    return out.stdout.trim();
  }

  async stats() {
    const pid = await this.pid();
    if (!pid) return { pid, fds: 0, rssKb: 0, threads: 0 };
    const out = await sh(
      `sudo -n sh -c 'ls /proc/${pid}/fd | wc -l; grep -E "^(VmRSS|Threads)" /proc/${pid}/status'`,
      { allowFailure: true },
    );
    const lines = out.stdout.trim().split('\n');
    return {
      pid,
      fds: Number(lines[0]),
      rssKb: Number((lines.find((l) => l.startsWith('VmRSS')) ?? '').replace(/\D+/g, '')),
      threads: Number((lines.find((l) => l.startsWith('Threads')) ?? '').replace(/\D+/g, '')),
    };
  }

  close() {
    this.log.close();
  }
}

export class DockerProxy {
  constructor() {
    this.kind = 'docker';
    this.port = PORTS.docker.game;
    this.admin = new Admin(PORTS.docker.admin);
    this.log = null;
  }

  async up() {
    await sh(`cd ${REMOTE_ROOT}/stack && sudo -n docker compose --profile docker-proxy up -d --force-recreate proxy`);
    this.log = dockerProxyLog();
    if (!(await this.admin.waitAlive())) throw new Error('the Docker proxy did not come up');
  }

  async pid() {
    const out = await sh("sudo -n docker inspect -f '{{.State.Pid}} {{.State.StartedAt}} {{.RestartCount}}' lab-proxy");
    return out.stdout.trim();
  }

  signal(name) {
    return sh(`sudo -n docker kill --signal=${name} lab-proxy`);
  }

  console(line) {
    return sh(
      `(printf '%s\\n' ${JSON.stringify(line)}; sleep 3) | sudo -n timeout -s KILL 6 docker attach --sig-proxy=false lab-proxy > /dev/null 2>&1`,
      { allowFailure: true, timeoutMs: 20000 },
    );
  }

  async down() {
    await sh(`cd ${REMOTE_ROOT}/stack && sudo -n docker compose --profile docker-proxy rm -sf proxy`, { allowFailure: true });
    this.log?.close();
  }

  close() {
    this.log?.close();
  }
}

export class Backends {
  constructor() {
    this.logs = new Map();
    this.consoles = new Map();
  }

  log(backend) {
    if (!this.logs.has(backend.name)) this.logs.set(backend.name, backendLog(backend));
    return this.logs.get(backend.name);
  }

  console(backend) {
    if (!this.consoles.has(backend.name)) this.consoles.set(backend.name, new Console(backend).open());
    return this.consoles.get(backend.name);
  }

  close() {
    for (const log of this.logs.values()) log.close();
    for (const con of this.consoles.values()) con.close();
  }
}

export class Member {
  constructor({ client, backends }) {
    this.client = client;
    this.route = client.route;
    this.username = client.username;
    this.backendLog = backends.log(this.route.backend);
    this.console = backends.console(this.route.backend);
    this.joinedAt = null;
    this.tokens = [];
  }

  async join({ timeoutMs = 240000 } = {}) {
    const mark = this.backendLog.mark();
    await this.client.start();
    const joined = await this.backendLog.waitFor(new RegExp(`\\b${this.username}\\b.*(joined the game|logged in with entity id)`), {
      since: mark,
      timeoutMs,
    });
    this.joinedAt = joined?.at ?? null;
    if (joined) await sleep(2500);
    return Boolean(joined);
  }

  async token(label, { timeoutMs = 15000 } = {}) {
    const token = `TK${label}${Math.random().toString(36).slice(2, 8).toUpperCase()}`;
    const mark = this.client.mark();
    const sentAt = Date.now();
    this.console.send(`say ${token}`);
    const seen = await this.client.waitFor(new RegExp(token), { since: mark, timeoutMs });
    return { ok: Boolean(seen), ms: seen ? seen.at - sentAt : null };
  }

  async chat(label, { attempts = 4 } = {}) {
    for (let attempt = 0; attempt < attempts; attempt++) {
      const text = `c${label}${Math.random().toString(36).slice(2, 8)}z`;
      const mark = this.backendLog.mark();
      await this.client.chat(text);
      if (await this.backendLog.waitFor(new RegExp(text), { since: mark, timeoutMs: 6000 })) return true;
      if (!this.client.alive) return false;
      await sleep(500);
    }
    return false;
  }

  left(since) {
    return this.backendLog
      .since(since)
      .find((line) => new RegExp(`\\b${this.username}\\b.*(left the game|lost connection)`).test(line.text));
  }

  async returned(since, { timeoutMs = 60000 } = {}) {
    const name = `\\b${this.username}\\b`;
    const left = await this.backendLog.waitFor(new RegExp(`${name}.*(left the game|lost connection)`), { since, timeoutMs });
    if (!left) return { left: null, back: null, awayMs: null };
    const after = this.backendLog.lines.indexOf(left) + 1;
    const back = await this.backendLog.waitFor(new RegExp(`${name}.*(joined the game|logged in with entity id)`), {
      since: after,
      timeoutMs,
    });
    if (back) await sleep(2500);
    return { left, back, awayMs: back ? back.at - left.at : null };
  }

  disconnectReason(since) {
    const line = this.client
      .since(since)
      .find((l) => /disconnect|Proxy is restarting|Connection Lost|kicked/i.test(l.text));
    return line?.text ?? null;
  }
}

export class Fleet {
  constructor({ factory, backends, proxy, shotsDir }) {
    Object.assign(this, { factory, backends, proxy, shotsDir });
    this.members = [];
    this.serial = 0;
  }

  async add(specs, { concurrency = 4 } = {}) {
    const queue = specs.slice();
    const added = [];
    const failures = [];
    const worker = async () => {
      for (;;) {
        const spec = queue.shift();
        if (!spec) return;
        const r = route(spec.mode, spec.version);
        const username = spec.username ?? `L${r.mode.key}${r.version.key}n${this.serial++}`.slice(0, 16);
        const client = await this.factory.client({
          username,
          route: r,
          port: this.proxy.port,
          shotsDir: join(this.shotsDir, username),
        });
        const member = new Member({ client, backends: this.backends });
        if (await member.join()) {
          this.members.push(member);
          added.push(member);
        } else {
          await client.screenshot('join-failed');
          failures.push({ member, tail: client.lines.slice(-15).map((l) => l.text) });
          await client.stop();
        }
      }
    };
    await Promise.all(Array.from({ length: concurrency }, worker));
    return { added, failures };
  }

  get survivors() {
    return this.members.filter((m) => m.route.mode.survives);
  }

  get intercepted() {
    return this.members.filter((m) => !m.route.mode.survives);
  }

  get returning() {
    return this.intercepted.filter((m) => m.route.version.transfers);
  }

  get kicked() {
    return this.intercepted.filter((m) => !m.route.version.transfers);
  }

  async remove(member) {
    this.members = this.members.filter((m) => m !== member);
    await member.client.stop();
  }

  async stop() {
    await Promise.all(this.members.map((m) => m.client.stop()));
    this.members = [];
  }
}

export class Pulse {
  constructor(members, { everyMs = 200 } = {}) {
    this.members = members;
    this.everyMs = everyMs;
    this.timer = null;
    this.seq = 0;
    this.prefix = `PL${Math.random().toString(36).slice(2, 6).toUpperCase()}x`;
    this.marks = new Map(members.map((m) => [m, m.client.mark()]));
  }

  start() {
    const consoles = new Set(this.members.map((m) => m.console));
    this.timer = setInterval(() => {
      this.seq++;
      for (const con of consoles) con.send(`say ${this.prefix}${this.seq}x`);
    }, this.everyMs);
    return this;
  }

  async stop() {
    clearInterval(this.timer);
    await sleep(1500);
  }

  report(member, { from, to }) {
    const pattern = new RegExp(`${this.prefix}(\\d+)x`);
    const arrivals = member.client
      .since(this.marks.get(member))
      .map((line) => ({ at: line.at, match: line.text.match(pattern) }))
      .filter((entry) => entry.match)
      .map((entry) => ({ at: entry.at, seq: Number(entry.match[1]) }));
    const seen = new Set(arrivals.map((a) => a.seq));
    let maxGapMs = 0;
    let gapAt = null;
    for (let i = 1; i < arrivals.length; i++) {
      const gap = arrivals[i].at - arrivals[i - 1].at;
      if (arrivals[i].at >= from && arrivals[i - 1].at <= to && gap > maxGapMs) {
        maxGapMs = gap;
        gapAt = arrivals[i - 1].at;
      }
    }
    const missing = [];
    for (let seq = 1; seq <= this.seq; seq++) if (!seen.has(seq)) missing.push(seq);
    const outOfOrder = arrivals.some((a, i) => i > 0 && a.seq <= arrivals[i - 1].seq);
    return { received: arrivals.length, sent: this.seq, missing: missing.length, outOfOrder, maxGapMs, gapAt };
  }
}

export class Checks {
  constructor() {
    this.items = [];
  }

  add(name, ok, detail = '') {
    this.items.push({ name, ok: Boolean(ok), detail: String(detail ?? '') });
    const mark = ok ? '  ok  ' : ' FAIL ';
    console.log(`   [${mark}] ${name}${detail ? ` — ${detail}` : ''}`);
    return Boolean(ok);
  }

  note(name, detail) {
    this.items.push({ name, ok: null, detail: String(detail ?? '') });
    console.log(`   [ note ] ${name} — ${detail}`);
  }

  get failed() {
    return this.items.filter((item) => item.ok === false).length;
  }
}

export async function restartAndWait(proxy, trigger, { timeoutMs = 45000 } = {}) {
  const mark = proxy.log.mark();
  const before = await proxy.admin.get('/proxy');
  const startedAt = Date.now();
  const triggered = await trigger();
  const line = await proxy.log.waitFor(new RegExp(`${COMMITTED.source}|${ROLLED_BACK.source}|${FAILED.source}`), {
    since: mark,
    timeoutMs,
  });
  const doneAt = line?.at ?? Date.now();
  await proxy.admin.waitAlive({ timeoutMs: 15000 });
  const after = await proxy.admin.get('/proxy');
  return {
    triggered,
    startedAt,
    doneAt,
    outcome: !line ? 'timeout' : COMMITTED.test(line.text) ? 'committed' : ROLLED_BACK.test(line.text) ? 'rolled-back' : 'refused',
    line: line?.text ?? null,
    before: before.body?.data ?? null,
    after: after.body?.data ?? null,
    log: proxy.log.since(mark).map((l) => l.text),
  };
}
