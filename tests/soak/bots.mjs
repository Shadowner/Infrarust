import { createRequire } from 'node:module';
import { appendFileSync, writeFileSync } from 'node:fs';
import net from 'node:net';

const require = createRequire(new URL('../e2e/package.json', import.meta.url));
const mc = require('minecraft-protocol');

function arg(name, fallback) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : fallback;
}

const cfg = {
  host: arg('host', '127.0.0.1'),
  port: Number(arg('port', '41565')),
  domain: arg('domain', 'soak-a.local'),
  switchTo: arg('switch-to', 'soak-b'),
  version: arg('version', '1.18.2'),
  bots: Number(arg('bots', '50')),
  duration: Number(arg('duration', '60')) * 1000,
  rampMs: Number(arg('ramp', '10')) * 1000,
  sessionMin: Number(arg('session-min', '8')) * 1000,
  sessionMax: Number(arg('session-max', '30')) * 1000,
  chatMs: Number(arg('chat-ms', '1500')),
  cmdMs: Number(arg('cmd-ms', '4000')),
  switchChance: Number(arg('switch-chance', '0.4')),
  abruptChance: Number(arg('abrupt-chance', '0.1')),
  abortChance: Number(arg('abort-chance', '0.03')),
  commands: arg('commands', 'wping,count,hello,fcmd,qcmd').split(',').filter(Boolean),
  windowMs: Number(arg('window', '10')) * 1000,
  out: arg('out', null),
  summary: arg('summary', null),
  tag: arg('tag', 'b'),
  replyTimeoutMs: Number(arg('reply-timeout', '5000')),
  loginTimeoutMs: Number(arg('login-timeout', '20000')),
  localAddress: arg('local-address', null),
};

const REPLY_MATCHERS = {
  wping: (text, token) => text === `wpong ${token}`,
  count: (text) => /^Online: \d+$/.test(text),
  hello: (text) => text.startsWith('Hello from Infrarust!'),
  fcmd: (text, token) => text === `fcmd-ok ${token}`,
  qcmd: (text, token) => text === `qcmd-ok ${token}`,
  wswitch: (text, token) => text.startsWith(`wswitched ${token} `),
};

function newWindow() {
  return {
    loginOk: 0,
    loginFail: 0,
    loginFailReasons: {},
    loginMs: [],
    aborted: 0,
    chatSent: 0,
    chatEchoed: 0,
    chatLost: 0,
    chatCut: 0,
    chatLate: 0,
    chatMs: [],
    cmd: {},
    switchSent: 0,
    switchOk: 0,
    switchErr: 0,
    switchErrReasons: {},
    switchConfirmed: 0,
    drops: 0,
    dropReasons: {},
    sessionsDone: 0,
    abrupt: 0,
  };
}

let win = newWindow();
const totals = newWindow();
let active = 0;
let running = true;
const startedAt = Date.now();

function bump(map, key) {
  const k = String(key ?? 'unknown').slice(0, 120);
  map[k] = (map[k] ?? 0) + 1;
}

function both(fn) {
  fn(win);
  fn(totals);
}

function cmdSlot(w, kind) {
  if (!w.cmd[kind]) w.cmd[kind] = { sent: 0, replied: 0, timeout: 0, cut: 0, ms: [] };
  return w.cmd[kind];
}

function pct(values, p) {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const i = Math.min(sorted.length - 1, Math.max(0, Math.ceil((p / 100) * sorted.length) - 1));
  return Math.round(sorted[i] * 10) / 10;
}

function summarize(w, now) {
  const cmd = {};
  for (const [kind, s] of Object.entries(w.cmd)) {
    cmd[kind] = { sent: s.sent, replied: s.replied, timeout: s.timeout, cut: s.cut, p50: pct(s.ms, 50), p99: pct(s.ms, 99), max: pct(s.ms, 100) };
  }
  return {
    t: Math.round((now - startedAt) / 1000),
    active,
    login: { ok: w.loginOk, fail: w.loginFail, p50: pct(w.loginMs, 50), p95: pct(w.loginMs, 95), p99: pct(w.loginMs, 99), max: pct(w.loginMs, 100), reasons: w.loginFailReasons },
    aborted: w.aborted,
    chat: { sent: w.chatSent, echoed: w.chatEchoed, lost: w.chatLost, cut: w.chatCut, late: w.chatLate, p50: pct(w.chatMs, 50), p99: pct(w.chatMs, 99), max: pct(w.chatMs, 100) },
    cmd,
    switch: { sent: w.switchSent, ok: w.switchOk, err: w.switchErr, confirmed: w.switchConfirmed, reasons: w.switchErrReasons },
    drops: w.drops,
    dropReasons: w.dropReasons,
    sessions: w.sessionsDone,
    abrupt: w.abrupt,
  };
}

function flatten(component) {
  if (component == null) return '';
  if (typeof component === 'string') {
    try {
      return flatten(JSON.parse(component));
    } catch {
      return component;
    }
  }
  if (Array.isArray(component)) return component.map(flatten).join('');
  let text = component.text ?? '';
  if (component.translate) text += component.translate;
  if (Array.isArray(component.extra)) text += component.extra.map(flatten).join('');
  return text;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function jitter(ms) {
  return ms * (0.6 + Math.random() * 0.8);
}

function pickCommand() {
  return cfg.commands[Math.floor(Math.random() * cfg.commands.length)];
}

async function session(slot, gen) {
  const username = `${cfg.tag}${slot}_${gen % 100000}`;
  const t0 = performance.now();
  const plannedMs = cfg.sessionMin + Math.random() * (cfg.sessionMax - cfg.sessionMin);
  const willAbort = Math.random() < cfg.abortChance;
  const willAbrupt = Math.random() < cfg.abruptChance;
  const willSwitch = Math.random() < cfg.switchChance;

  return new Promise((resolve) => {
    let client;
    let joined = false;
    let finished = false;
    let plannedEnd = false;
    let chatSeq = 0;
    let cmdSeq = 0;
    let homePort = null;
    let switched = false;
    let switchSentAt = null;
    const pendingChats = new Map();
    const goneChats = new Map();
    const pendingCmds = [];
    const timers = [];

    const finish = (why) => {
      if (finished) return;
      finished = true;
      for (const t of timers) clearTimeout(t), clearInterval(t);
      if (joined) active -= 1;
      if (joined && !plannedEnd) {
        process.stderr.write(`DROP ${username} ${why} t=${Math.round((Date.now() - startedAt) / 1000)}\n`);
        both((w) => {
          w.drops += 1;
          bump(w.dropReasons, why);
        });
      }
      const cutChats = pendingChats.size;
      for (const [token, sentAt] of pendingChats) goneChats.set(token, { sentAt, why: 'cut' });
      pendingChats.clear();
      const cutCmds = pendingCmds.splice(0);
      both((w) => {
        if (plannedEnd) w.chatCut += cutChats;
        else w.chatLost += cutChats;
        for (const pending of cutCmds) {
          if (plannedEnd) cmdSlot(w, pending.kind).cut += 1;
          else cmdSlot(w, pending.kind).timeout += 1;
        }
      });
      both((w) => {
        w.sessionsDone += 1;
      });
      try {
        client?.end();
      } catch {
      }
      setTimeout(resolve, 50);
    };

    try {
      const options = {
        host: cfg.host,
        port: cfg.port,
        username,
        version: cfg.version,
        fakeHost: cfg.domain,
        auth: 'offline',
        hideErrors: true,
        keepAlive: true,
      };
      if (cfg.localAddress) {
        options.stream = net.connect({ host: cfg.host, port: cfg.port, localAddress: cfg.localAddress });
      }
      client = mc.createClient(options);
    } catch (err) {
      both((w) => {
        w.loginFail += 1;
        bump(w.loginFailReasons, `createClient: ${err.message}`);
      });
      finish('create');
      return;
    }

    const loginTimer = setTimeout(() => {
      if (!joined) {
        both((w) => {
          w.loginFail += 1;
          bump(w.loginFailReasons, 'login timeout');
        });
        plannedEnd = true;
        finish('login timeout');
      }
    }, cfg.loginTimeoutMs);
    timers.push(loginTimer);

    if (willAbort) {
      timers.push(
        setTimeout(() => {
          if (!joined) {
            plannedEnd = true;
            both((w) => {
              w.aborted += 1;
            });
            try {
              client.socket?.destroy();
            } catch {
            }
            finish('aborted');
          }
        }, 30 + Math.random() * 250),
      );
    }

    const sendChat = () => {
      if (finished || !joined) return;
      chatSeq += 1;
      const token = `c${slot}.${gen}.${chatSeq}`;
      pendingChats.set(token, performance.now());
      both((w) => {
        w.chatSent += 1;
      });
      try {
        client.write('chat', { message: token });
      } catch {
      }
      timers.push(
        setTimeout(() => {
          if (pendingChats.has(token)) {
            goneChats.set(token, { sentAt: pendingChats.get(token), why: 'timeout' });
            pendingChats.delete(token);
            const switchState = switched ? 'done' : switchSentAt === null ? 'none' : `pending ${Math.round(performance.now() - switchSentAt)}ms`;
            process.stderr.write(`LOST chat ${username} ${token} t=${Math.round((Date.now() - startedAt) / 1000)} switch=${switchState}\n`);
            both((w) => {
              w.chatLost += 1;
            });
          }
        }, cfg.replyTimeoutMs),
      );
    };

    const sendCommand = (kind, argsText) => {
      if (finished || !joined) return;
      cmdSeq += 1;
      const token = `k${slot}.${gen}.${cmdSeq}`;
      const pending = { kind, token, at: performance.now() };
      pendingCmds.push(pending);
      both((w) => {
        cmdSlot(w, kind).sent += 1;
      });
      let line;
      if (kind === 'wping' || kind === 'fcmd' || kind === 'qcmd') line = `/${kind} ${token}`;
      else if (kind === 'wswitch') line = `/wswitch ${argsText} ${token}`;
      else line = `/${kind}`;
      try {
        client.write('chat', { message: line });
      } catch {
      }
      timers.push(
        setTimeout(() => {
          const i = pendingCmds.indexOf(pending);
          if (i >= 0) {
            pendingCmds.splice(i, 1);
            if (kind !== 'qcmd' && kind !== 'fcmd') process.stderr.write(`LOST cmd ${kind} ${username} ${token} t=${Math.round((Date.now() - startedAt) / 1000)}\n`);
            both((w) => {
              cmdSlot(w, kind).timeout += 1;
            });
          }
        }, cfg.replyTimeoutMs),
      );
    };

    const scheduleLoop = (baseMs, fn) => {
      const tick = () => {
        if (finished) return;
        fn();
        timers.push(setTimeout(tick, jitter(baseMs)));
      };
      timers.push(setTimeout(tick, jitter(baseMs)));
    };

    client.on('login', () => {
      if (joined || finished) return;
      joined = true;
      active += 1;
      clearTimeout(loginTimer);
      const ms = performance.now() - t0;
      both((w) => {
        w.loginOk += 1;
        w.loginMs.push(ms);
      });
      scheduleLoop(cfg.chatMs, sendChat);
      if (cfg.commands.length > 0) scheduleLoop(cfg.cmdMs, () => sendCommand(pickCommand()));
      if (willSwitch && cfg.switchTo) {
        timers.push(
          setTimeout(() => {
            both((w) => {
              w.switchSent += 1;
            });
            switchSentAt = performance.now();
            sendCommand('wswitch', cfg.switchTo);
          }, plannedMs * (0.3 + Math.random() * 0.3)),
        );
      }
      timers.push(
        setTimeout(() => {
          plannedEnd = true;
          if (willAbrupt) {
            both((w) => {
              w.abrupt += 1;
            });
            try {
              client.socket?.destroy();
            } catch {
            }
          }
          finish('planned');
        }, plannedMs),
      );
    });

    client.on('chat', (packet) => {
      if (finished) return;
      const text = flatten(packet.message);
      const echo = /^echo@(\d+) (\S+)$/.exec(text);
      if (echo) {
        const port = Number(echo[1]);
        if (homePort === null) homePort = port;
        else if (port !== homePort && !switched) {
          switched = true;
          both((w) => {
            w.switchConfirmed += 1;
          });
        }
        const sentAt = pendingChats.get(echo[2]);
        if (sentAt !== undefined) {
          pendingChats.delete(echo[2]);
          const ms = performance.now() - sentAt;
          both((w) => {
            w.chatEchoed += 1;
            w.chatMs.push(ms);
          });
        } else {
          const gone = goneChats.get(echo[2]);
          const delay = gone ? `${Math.round(performance.now() - gone.sentAt)}ms after send (${gone.why})` : 'unknown token';
          process.stderr.write(`LATE chat ${username} ${echo[2]} from ${port} ${delay} finished=${finished} t=${Math.round((Date.now() - startedAt) / 1000)}\n`);
          both((w) => {
            w.chatLate += 1;
          });
        }
        return;
      }
      for (let i = 0; i < pendingCmds.length; i += 1) {
        const pending = pendingCmds[i];
        const matcher = REPLY_MATCHERS[pending.kind];
        if (matcher && matcher(text, pending.token)) {
          pendingCmds.splice(i, 1);
          const ms = performance.now() - pending.at;
          both((w) => {
            const s = cmdSlot(w, pending.kind);
            s.replied += 1;
            s.ms.push(ms);
            if (pending.kind === 'wswitch') {
              if (text.endsWith(' ok')) w.switchOk += 1;
              else {
                w.switchErr += 1;
                bump(w.switchErrReasons, text.slice(0, 100));
              }
            }
          });
          return;
        }
      }
    });

    client.on('kick_disconnect', (packet) => {
      if (!joined) {
        both((w) => {
          w.loginFail += 1;
          bump(w.loginFailReasons, `kick: ${flatten(packet?.reason)}`);
        });
        plannedEnd = true;
      }
      finish(`kick: ${flatten(packet?.reason)}`);
    });
    client.on('disconnect', (packet) => {
      if (!joined) {
        both((w) => {
          w.loginFail += 1;
          bump(w.loginFailReasons, `disconnect: ${flatten(packet?.reason)}`);
        });
        plannedEnd = true;
      }
      finish(`disconnect: ${flatten(packet?.reason)}`);
    });
    client.on('error', (err) => {
      if (!joined && !finished) {
        both((w) => {
          w.loginFail += 1;
          bump(w.loginFailReasons, `error: ${err?.code ?? err?.message}`);
        });
        plannedEnd = true;
      }
      finish(`error: ${err?.code ?? err?.message}`);
    });
    client.on('end', (reason) => {
      if (!joined && !finished) {
        both((w) => {
          w.loginFail += 1;
          bump(w.loginFailReasons, `end before join: ${reason ?? ''}`);
        });
        plannedEnd = true;
      }
      finish(`end: ${reason ?? ''}`);
    });
  });
}

async function botLoop(slot) {
  await sleep((slot / Math.max(1, cfg.bots)) * cfg.rampMs);
  let gen = 0;
  while (running) {
    gen += 1;
    await session(slot, gen);
    await sleep(Math.random() * 400);
  }
}

function flush() {
  const now = Date.now();
  const line = JSON.stringify(summarize(win, now));
  win = newWindow();
  if (cfg.out) appendFileSync(cfg.out, `${line}\n`);
  else process.stdout.write(`${line}\n`);
}

const flusher = setInterval(flush, cfg.windowMs);

const loops = [];
for (let slot = 0; slot < cfg.bots; slot += 1) loops.push(botLoop(slot));

const stop = async () => {
  if (!running) return;
  running = false;
  await Promise.race([Promise.all(loops), sleep(cfg.sessionMax + 10000)]);
  clearInterval(flusher);
  flush();
  const summary = summarize(totals, Date.now());
  if (cfg.summary) writeFileSync(cfg.summary, JSON.stringify(summary, null, 2));
  process.stdout.write(`SUMMARY ${JSON.stringify(summary)}\n`);
  process.exit(0);
};

setTimeout(stop, cfg.duration);
process.on('SIGTERM', stop);
process.on('SIGINT', stop);
