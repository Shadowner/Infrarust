#!/usr/bin/env node

import { mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { Yggdrasil } from '../yggdrasil.js';
import { openDisplay } from './display.js';
import { proxiedScenarios, targetFor, writeLab } from './lab.js';
import { buildLaunch, runClient, setReleaseOrder } from './launch.js';
import { cacheRoot, installClient, releasesFrom, resolveJava } from './mojang.js';
import { ProxyProcess, portInUse } from './proxy.js';
import { BACKENDS, SCENARIOS, skipReason } from './scenarios.js';
import { fetchPaperServer, prepareServer, sleep } from './server.js';
import { fetchAuthlibInjector, offlineSession, onlineSession } from './session.js';
import { renderReport } from './report.js';

const ANCHORS = [
  '1.7.10',
  '1.8.9',
  '1.10.2',
  '1.11.2',
  '1.12.2',
  '1.13.2',
  '1.14.4',
  '1.15.2',
  '1.16.5',
  '1.17.1',
  '1.18.2',
  '1.19.4',
  '1.20.1',
  '1.20.2',
  '1.20.6',
  '1.21.4',
  '1.21.11',
];

function parseArgs(argv) {
  const opts = {
    versions: null,
    set: 'anchors',
    scenarios: null,
    binary: null,
    cache: null,
    labDir: null,
    out: null,
    sessionServer: null,
    donorMc: null,
    timeoutMs: 210000,
    settleMs: 1500,
    keepGoing: true,
    keepWorlds: false,
  };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    const next = () => argv[++i];
    switch (arg) {
      case '--versions': opts.versions = next().split(',').map((s) => s.trim()).filter(Boolean); break;
      case '--set': opts.set = next(); break;
      case '--scenario': opts.scenarios = next().split(',').map((s) => s.trim()).filter(Boolean); break;
      case '--binary': opts.binary = next(); break;
      case '--cache': opts.cache = next(); break;
      case '--lab-dir': opts.labDir = next(); break;
      case '--out': opts.out = next(); break;
      case '--session-server': opts.sessionServer = next(); break;
      case '--donor-mc': opts.donorMc = next(); break;
      case '--timeout': opts.timeoutMs = Number(next()) * 1000; break;
      case '--stop-on-fail': opts.keepGoing = false; break;
      case '--keep-worlds': opts.keepWorlds = true; break;
      default: throw new Error(`unknown option ${arg}`);
    }
  }
  return opts;
}

function usernameFor(versionIndex, scenarioIndex) {
  return `E${String(versionIndex).padStart(2, '0')}s${String(scenarioIndex).padStart(2, '0')}`;
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  const here = new URL('.', import.meta.url).pathname;
  const benchRoot = join(here, '..', '..');

  const cache = opts.cache ?? join(benchRoot, '.mccache');
  const paths = cacheRoot(cache);
  for (const dir of Object.values(paths)) mkdirSync(dir, { recursive: true });
  mkdirSync(join(cache, 'tools'), { recursive: true });

  const outDir = opts.out ?? join(benchRoot, 'reports');
  mkdirSync(outDir, { recursive: true });

  const releases = await releasesFrom(paths, '1.7.10');
  const order = releases.map((r) => r.id);
  setReleaseOrder(order);
  const atLeastFor = (id) => (min) => order.indexOf(id) >= order.indexOf(min);

  let versions;
  if (opts.versions) {
    versions = opts.versions;
    for (const v of versions) if (!order.includes(v)) throw new Error(`${v} is not a release >= 1.7.10`);
  } else if (opts.set === 'all') {
    versions = order;
  } else {
    versions = ANCHORS;
  }

  let scenarios = SCENARIOS;
  if (opts.scenarios) scenarios = SCENARIOS.filter((s) => opts.scenarios.includes(s.id));
  if (scenarios.length === 0) throw new Error('no scenario selected');

  let yggdrasil = null;
  let injectorJar = null;
  if (opts.sessionServer) {
    yggdrasil = new Yggdrasil(opts.sessionServer);
    await yggdrasil.ready();
    injectorJar = await fetchAuthlibInjector(paths);
  }
  const authAvailable = Boolean(yggdrasil);

  const needed = [
    ...proxiedScenarios()
      .filter((s) => scenarios.some((x) => x.id === s.id))
      .flatMap((s) => [
        { port: s.port, what: `the ${s.id} proxy` },
        { port: s.adminPort, what: `the ${s.id} admin API` },
      ]),
    ...Object.entries(BACKENDS)
      .filter(([name]) => scenarios.some((s) => s.backend === name))
      .map(([name, b]) => ({ port: b.port, what: `the ${name} backend` })),
  ];
  const taken = [];
  for (const entry of needed) if (await portInUse(entry.port)) taken.push(entry);
  if (taken.length > 0) {
    const list = taken.map((t) => `  ${t.port} (${t.what})`).join('\n');
    throw new Error(
      `these ports are already in use:\n${list}\n\n` +
        'A previous run was probably killed rather than stopped. Clear it with:\n' +
        "  pkill -f target/release/infrarust; pkill -f '\\-jar .*\\.mccache/servers'",
    );
  }

  const labDir = opts.labDir ?? join(benchRoot, '.lab-real');
  writeLab(labDir, { sessionUrl: yggdrasil?.injectorHasJoinedUrl });

  const binary = opts.binary ?? join(benchRoot, '..', '..', 'target', 'release', 'infrarust');
  const proxies = new Map();
  for (const scenario of proxiedScenarios()) {
    if (!scenarios.some((s) => s.id === scenario.id)) continue;
    const proxy = new ProxyProcess(binary, join(labDir, scenario.id, 'infrarust.toml'), {
      id: scenario.id,
      port: scenario.port,
      adminPort: scenario.adminPort,
      cwd: join(labDir, scenario.id),
    });
    await proxy.start();
    proxies.set(scenario.id, proxy);
  }
  const ported = new Map(proxiedScenarios().map((s) => [s.id, s]));

  console.log(
    `real-client bench · ${versions.length} versions × ${scenarios.length} scenarios · ` +
      `${proxies.size} proxies · auth ${authAvailable ? 'on' : 'off'}`,
  );

  const liveServers = new Set();
  const shutdown = async () => {
    for (const server of liveServers) await server.stop();
    for (const proxy of proxies.values()) await proxy.stop();
  };
  for (const signal of ['SIGINT', 'SIGTERM']) {
    process.once(signal, () => {
      shutdown().finally(() => process.exit(130));
    });
  }

  const results = [];
  let exitCode = 0;

  try {
    for (const [versionIndex, version] of versions.entries()) {
      const row = await runVersion({
        liveServers,
        version,
        versionIndex,
        scenarios,
        paths,
        cache,
        opts,
        proxies,
        ported,
        yggdrasil,
        injectorJar,
        authAvailable,
        atLeast: atLeastFor(version),
      });
      results.push(row);
      printRow(row, scenarios);
      writeOut(outDir, versions, results, scenarios);
      if (row.cells.some((c) => c.status === 'fail') && !opts.keepGoing) break;
    }
  } finally {
    await shutdown();
  }

  const report = writeOut(outDir, versions, results, scenarios);

  console.log(`\n${report.summary}`);
  console.log(`report: ${join(outDir, 'real-client.md')}`);

  if (results.some((r) => r.cells.some((c) => c.status === 'fail'))) exitCode = 1;
  process.exit(exitCode);
}

function writeOut(outDir, versions, results, scenarios) {
  const report = renderReport(results, scenarios);
  writeFileSync(join(outDir, 'real-client.md'), report.markdown);
  writeFileSync(join(outDir, 'real-client.json'), JSON.stringify({ versions, results }, null, 2));
  return report;
}

async function runVersion(ctx) {
  const { version, versionIndex, scenarios, paths, cache, opts, proxies, ported, atLeast } = ctx;
  const started = Date.now();
  const row = { version, cells: [], notes: [] };

  let install;
  let java;
  try {
    install = await installClient(paths, version, {
      donorAssets: opts.donorMc ? join(opts.donorMc, 'assets') : undefined,
      donorLibraries: opts.donorMc ? join(opts.donorMc, 'libraries') : undefined,
    });
    ({ java } = await resolveJava(paths, install.vjson));
  } catch (err) {
    row.notes.push(`install failed: ${err.message}`);
    for (const scenario of scenarios) {
      row.cells.push({ scenario: scenario.id, status: 'fail', detail: `client install failed: ${err.message}` });
    }
    return row;
  }

  const paperAvailable = scenarios.some((s) => BACKENDS[s.backend].flavour === 'paper')
    ? Boolean(await fetchPaperServer(paths, version).catch(() => null))
    : false;

  const wanted = new Set(
    scenarios
      .filter((s) => !skipReason(s, { atLeast, authAvailable: ctx.authAvailable, paperAvailable }))
      .map((s) => s.backend),
  );

  const servers = new Map();
  try {
    for (const name of wanted) {
      const backend = BACKENDS[name];
      const server = await prepareServer(paths, version, {
        flavour: backend.flavour,
        port: backend.port,
        dir: join(cache, 'worlds', `${version}-${name}`),
        velocitySecret: name === 'velocity' ? 'e2eSecretForVelocityForwarding01' : undefined,
      });
      await server.start();
      servers.set(name, server);
      ctx.liveServers.add(server);
    }
  } catch (err) {
    row.notes.push(`backend failed: ${err.message}`);
    for (const server of servers.values()) {
      await server.stop();
      ctx.liveServers.delete(server);
    }
    for (const scenario of scenarios) {
      row.cells.push({ scenario: scenario.id, status: 'fail', detail: `backend failed: ${err.message}` });
    }
    return row;
  }

  let velocityEnforced = null;

  try {
    for (const [scenarioIndex, scenario] of scenarios.entries()) {
      const skip = skipReason(scenario, {
        atLeast,
        authAvailable: ctx.authAvailable,
        paperAvailable,
        velocityBackendReady: velocityEnforced,
      });
      if (skip) {
        row.cells.push({ scenario: scenario.id, status: 'skip', detail: skip });
        continue;
      }
      const cell = await runCase({
        ...ctx,
        scenario,
        scenarioIndex,
        install,
        java,
        server: servers.get(scenario.backend),
        proxy: scenario.proxy ? proxies.get(scenario.id) : null,
        target: scenario.proxy ? targetFor(ported.get(scenario.id)) : targetFor(scenario),
        username: usernameFor(versionIndex, scenarioIndex),
      });
      row.cells.push(cell);

      if (scenario.id === 'direct' && cell.status === 'fail' && !cell.observations?.connected) {
        for (const rest of scenarios.slice(scenarioIndex + 1)) {
          row.cells.push({
            scenario: rest.id,
            status: 'untrusted',
            detail: 'not run: this release cannot start a client here, so nothing it did could be attributed to the proxy',
          });
        }
        row.notes.push('the client never opened a socket; the rest of the row was not run');
        break;
      }

      if (scenario.gatesVelocity) {
        const obs = cell.observations;
        if (!obs || obs.stalledOut || obs.clientCrashed || obs.clientReason === 'spawn-failed') {
          velocityEnforced = null;
        } else if (obs.joined) {
          velocityEnforced = false;
          row.notes.push('the Paper build for this release does not enforce Velocity forwarding');
          cell.status = 'untrusted';
          cell.detail =
            'this Paper build lets a direct connection in, so it does not enforce Velocity ' +
            'forwarding and no Velocity result is possible for this release';
        } else {
          velocityEnforced = true;
        }
      }
    }
  } finally {
    for (const server of servers.values()) {
      await server.stop();
      ctx.liveServers.delete(server);
    }
    if (!opts.keepWorlds) {
      for (const name of servers.keys()) {
        rmSync(join(cache, 'worlds', `${version}-${name}`), { recursive: true, force: true });
      }
    }
  }

  const control = row.cells.find((c) => c.scenario === 'direct');
  if (control && control.status === 'fail') {
    row.notes.push('control run failed: proxied results for this version are not attributable to the proxy');
    for (const cell of row.cells) {
      if (cell.scenario !== 'direct' && cell.status === 'fail') cell.status = 'untrusted';
    }
  }

  for (const cell of row.cells) {
    const seen = cell.observations?.proxyPlayer?.protocol_version;
    if (seen != null) {
      row.protocol = seen;
      break;
    }
  }

  row.elapsedMs = Date.now() - started;
  return row;
}

async function runCase(ctx) {
  const { scenario, install, java, server, proxy, target, username, opts, cache, yggdrasil, injectorJar } = ctx;
  const started = Date.now();

  let session;
  try {
    session = scenario.needsAuth
      ? await onlineSession(yggdrasil, username, { injectorJar })
      : offlineSession(username);
  } catch (err) {
    return { scenario: scenario.id, status: 'fail', detail: `could not obtain a session: ${err.message}` };
  }

  const gameDir = join(cache, 'gamedirs', `${install.id}-${scenario.id}`);
  const launch = buildLaunch(install, { gameDir, session, target, java });

  await server.quiesce();
  const serverMark = server.mark();
  const proxyMark = proxy?.mark() ?? 0;

  const budgetMs = scenario.expect.joined ? opts.timeoutMs : Math.min(opts.timeoutMs, 120000);
  const joinPromise = server.waitForJoin(session.username, { since: serverMark, timeoutMs: budgetMs });

  const STALL_MS = 60000;
  const connect = { at: null };
  const over = { yet: false };
  const stalled = (async () => {
    while (!over.yet) {
      await sleep(500);
      if (connect.at && Date.now() - connect.at > STALL_MS) {
        return { joined: false, line: null, stalled: true };
      }
    }
    return new Promise(() => {});
  })();

  const settled = proxy
    ? Promise.race([
        joinPromise,
        stalled,
        proxy.waitForError(proxyMark, { timeoutMs: budgetMs }).then((line) =>
          line ? { joined: false, line, fromProxy: true } : new Promise(() => {}),
        ),
      ])
    : Promise.race([joinPromise, stalled]);

  const chatToken = `BENCH-${session.username}`;

  const probe = { proxyPlayer: null, proxyError: null, chatSaid: false, chatSeen: false };

  const verdict = settled.then(async (outcome) => {
    if (proxy) {
      try {
        probe.proxyPlayer = await proxy.waitForPlayer(session.username, {
          timeoutMs: outcome.joined ? 5000 : 1000,
        });
      } catch (err) {
        probe.proxyError = err.message;
      }
    }
    if (outcome.joined) {
      probe.chatSaid = server.say(chatToken);
      const deadline = Date.now() + 15000;
      while (!probe.chatSeen && Date.now() < deadline) await sleep(200);
    }
    return outcome;
  });

  const display = await openDisplay();
  let run;
  try {
    run = await runClient({
      java: launch.java,
      args: launch.args,
      gameDir,
      display: display.name,
      timeoutMs: budgetMs + 20000,
      onLine: (line) => {
        if (connect.at === null && /Connecting to /.test(line)) connect.at = Date.now();
      },
      isDone: (line) => {
        if (!line.includes(chatToken)) return false;
        probe.chatSeen = true;
        return true;
      },
      stopWhen: verdict,
    });
  } finally {
    await display.stop();
  }
  const joined = await verdict;
  over.yet = true;
  const proxyGaveUp = Boolean(joined.fromProxy);
  const stalledOut = Boolean(joined.stalled);

  const observations = {
    joined: joined.joined,
    serverLine: joined.line,
    proxyGaveUp,
    stalledOut,
    serverWindow: server.since(serverMark).slice(-40),
    proxyWindow: proxy ? proxy.since(proxyMark).slice(-40) : [],
    proxyPlayer: probe.proxyPlayer,
    proxyError: probe.proxyError,
    chatSaid: probe.chatSaid,
    chatRelayed: run.lines.some((l) => l.includes(chatToken)),
    connected: connect.at !== null,
    clientCrashed: run.lines.some((l) =>
      /Reported exception thrown!|A fatal error has been detected|Minecraft has crashed/.test(l),
    ),
    clientReason: run.reason,
    clientTail: run.lines.slice(-30),
    elapsedMs: Date.now() - started,
  };

  return { scenario: scenario.id, ...evaluate(scenario, observations), observations };
}

function evaluate(scenario, obs) {
  const checks = [];
  const want = scenario.expect;

  if (obs.clientCrashed && obs.joined) {
    return {
      status: 'untrusted',
      checks: [],
      detail: 'the client reached the world and then crashed on its own (see its log); nothing here is the proxy\'s doing',
    };
  }

  if (want.joined === false && (obs.stalledOut || obs.proxyGaveUp)) {
    return {
      status: 'untrusted',
      checks: [],
      detail:
        'the control could not be exercised: the connection never reached the backend ' +
        `(${obs.proxyGaveUp ? obs.serverLine : 'the proxy stopped talking mid-handshake'})`,
    };
  }

  checks.push({
    name: 'joined',
    ok: obs.joined === want.joined,
    detail: obs.joined
      ? obs.serverLine
      : obs.proxyGaveUp
        ? `the proxy failed the connection: ${obs.serverLine}`
        : obs.stalledOut
          ? 'the client connected and then nothing happened: no join, no kick, no error — ' +
            'the proxy stopped talking mid-handshake'
        : `the server never logged a join (client ${obs.clientReason})${obs.serverLine ? `; last relevant line: ${obs.serverLine}` : ''}`,
  });

  if (want.chatRelayed !== undefined) {
    checks.push({
      name: 'chatRelayed',
      ok: obs.chatRelayed === want.chatRelayed,
      detail: obs.chatRelayed
        ? 'the client received a message the server sent after the join'
        : obs.chatSaid
          ? 'the server said the token but the client never logged it'
          : 'the token was never sent',
    });
  }

  if (want.rejection) {
    const refused = [...obs.serverWindow, ...obs.clientTail].some((l) => want.rejection.test(l));
    checks.push({
      name: 'rejected for the right reason',
      ok: refused,
      detail: refused ? 'the backend refused with the expected message' : `no line matched ${want.rejection}`,
    });
  }

  if (scenario.proxy) {
    if (want.proxySawPlayer === undefined) {
    } else if (obs.proxyError) {
      checks.push({ name: 'proxySawPlayer', ok: false, detail: `could not read the proxy's view: ${obs.proxyError}` });
    } else {
      const saw = Boolean(obs.proxyPlayer);
      checks.push({
        name: 'proxySawPlayer',
        ok: saw === want.proxySawPlayer,
        detail: saw
          ? `proxy reports ${obs.proxyPlayer.username ?? '?'} on protocol ${obs.proxyPlayer.protocol_version ?? '?'}`
          : 'proxy reports no player',
      });
      if (saw && want.proxyActive !== undefined) {
        checks.push({
          name: 'proxyActive',
          ok: Boolean(obs.proxyPlayer.is_active) === want.proxyActive,
          detail: `is_active=${obs.proxyPlayer.is_active}, expected ${want.proxyActive}`,
        });
      }
    }
  }

  const failed = checks.filter((c) => !c.ok);
  return {
    status: failed.length === 0 ? 'pass' : 'fail',
    checks,
    detail: failed.map((c) => `${c.name}: ${c.detail}`).join(' | '),
  };
}

const GLYPH = { pass: '✅', fail: '❌', skip: '⏭️ ', untrusted: '⚠️ ' };

function printRow(row, scenarios) {
  const cells = scenarios.map((s) => {
    const cell = row.cells.find((c) => c.scenario === s.id);
    return GLYPH[cell?.status ?? 'skip'] ?? '?';
  });
  const secs = row.elapsedMs ? `${(row.elapsedMs / 1000).toFixed(0)}s` : '';
  const proto = row.protocol != null ? String(row.protocol).padStart(4) : '   ?';
  console.log(`${row.version.padEnd(9)} ${proto}  ${cells.join(' ')}  ${secs}`);
  for (const cell of row.cells) {
    if (cell.status === 'fail') console.log(`  ${cell.scenario}: ${cell.detail}`);
  }
}

await main();
