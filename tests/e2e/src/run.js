#!/usr/bin/env node

import { mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { startBackend } from './backend-mock.js';
import { connectOnce } from './client.js';
import { evaluate, KNOWN_FINDINGS } from './expectations.js';
import { FORWARDING_SECRET, writeLab } from './lab.js';
import { Proxy, sleep, waitForPlayer, waitForPort } from './proxy.js';
import { writeReport } from './report.js';
import { BACKENDS, SCENARIOS, skipReason } from './scenarios.js';
import { allVersions, TIER_B_ANCHORS } from './versions.js';
import { Yggdrasil } from './yggdrasil.js';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, '..');
const REPO = resolve(ROOT, '../..');

function parseArgs(argv) {
  const args = { tier: 'a', versions: null, scenarios: null, labDir: null, binary: null, sessionServer: null, outDir: null, backends: 'mock', caseDelayMs: null };
  for (let i = 0; i < argv.length; i++) {
    const [key, inline] = argv[i].split('=');
    const value = () => inline ?? argv[++i];
    switch (key) {
      case '--tier': args.tier = value(); break;
      case '--version': args.versions = value().split(','); break;
      case '--scenario': args.scenarios = value().split(','); break;
      case '--lab-dir': args.labDir = value(); break;
      case '--binary': args.binary = value(); break;
      case '--session-server': args.sessionServer = value(); break;
      case '--out': args.outDir = value(); break;
      case '--backends': args.backends = value(); break;
      case '--case-delay': args.caseDelayMs = Number(value()); break;
      case '--help': case '-h': usage(); process.exit(0); break;
      default: throw new Error(`unknown argument: ${key}`);
    }
  }
  return args;
}

function usage() {
  console.log(`Usage: node src/run.js [options]

  --tier a|b            a = mock backends, whole matrix (default). b = anchor versions only.
  --version <list>      comma-separated client versions, e.g. 1.7,1.20.1
  --scenario <list>     comma-separated scenario ids, e.g. offline-velocity
  --binary <path>       infrarust binary (default: target/release/infrarust)
  --backends mock|real  mock = node-minecraft-protocol backends (default).
                        real = expect Java servers already listening on the
                        scenario ports; the harness starts nothing.
  --case-delay <ms>     Pause between cases. Defaults to 0 for mock backends and
                        4500 for real ones, because Bukkit-derived servers refuse
                        repeat connections from one IP within connection-throttle
                        (bukkit.yml, 4000 ms by default).
  --session-server <url>  base URL of a Yggdrasil session server; enables the client_only scenarios
  --lab-dir <path>      where to render the throwaway proxy config
  --out <path>          where to write report.json / report.md
`);
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const binary = args.binary ?? resolve(REPO, 'target/release/infrarust');
  const labDir = args.labDir ?? resolve(ROOT, '.lab');
  const outDir = args.outDir ?? resolve(ROOT, 'reports');
  mkdirSync(outDir, { recursive: true });

  const authAvailable = Boolean(args.sessionServer);

  let versions = allVersions();
  if (args.tier === 'b') versions = versions.filter((v) => TIER_B_ANCHORS.includes(v.version));
  if (args.versions) versions = versions.filter((v) => args.versions.includes(v.version));
  let scenarios = SCENARIOS;
  if (args.scenarios) scenarios = scenarios.filter((s) => args.scenarios.includes(s.id));

  if (!versions.length) throw new Error('no versions selected');
  if (!scenarios.length) throw new Error('no scenarios selected');

  let yggdrasil = null;
  if (args.sessionServer) {
    yggdrasil = new Yggdrasil(args.sessionServer);
    await yggdrasil.ready();
  }
  const { configPath } = writeLab(labDir, { sessionUrl: yggdrasil?.hasJoinedUrl ?? null });

  const proxy = new Proxy(binary, configPath, { cwd: labDir, logLevel: 'debug' });
  console.log(`starting ${binary}`);
  await proxy.start();
  console.log(`proxy ready · ${versions.length} versions × ${scenarios.length} scenarios`);

  const useMocks = args.backends === 'mock';
  const caseDelayMs = args.caseDelayMs ?? (useMocks ? 0 : 4500);

  const startedAt = new Date().toISOString();
  const t0 = Date.now();
  const results = [];

  try {
    for (const v of versions) {
      const backends = {};
      if (useMocks) {
        for (const [name, cfg] of Object.entries(BACKENDS)) {
          backends[name] = await startBackend({ version: v.version, port: cfg.port, kind: cfg.kind, secret: FORWARDING_SECRET });
        }
      } else {
        for (const [name] of Object.entries(BACKENDS)) {
          backends[name] = { reset() {}, observation: () => null, close: async () => {} };
        }
      }
      for (const cfg of Object.values(BACKENDS)) await waitForPort(cfg.port, { timeoutMs: 120000 });

      const line = [];
      for (const scenario of scenarios) {
        const reason = skipReason(scenario, v.protocol, { authAvailable });
        if (reason) {
          results.push({ version: v.version, protocol: v.protocol, scenario: scenario.id, status: 'skip', reason, checks: [] });
          line.push('⏭️');
          continue;
        }

        if (caseDelayMs) await sleep(caseDelayMs);
        const backend = backends[scenario.backend];
        backend.reset();
        const mark = proxy.mark();

        const username = `E${versions.indexOf(v)}_${scenario.id.replace(/[^a-zA-Z0-9]/g, '')}`.slice(0, 16);

        let proxyPlayer = null;
        const client = await connectOnce({
          version: v.version,
          domain: scenario.domain,
          username,
          session: scenario.needsAuth ? await yggdrasil.session(username) : null,
          sessionServer: scenario.needsAuth ? yggdrasil.sessionServer : null,
          onPlay: async () => {
            proxyPlayer = await waitForPlayer(username);
          },
        });

        const observations = { client, backend: backend.observation(), proxyPlayer };
        const { status, checks } = evaluate(scenario, observations, { backendObservable: useMocks });
        results.push({
          version: v.version,
          protocol: v.protocol,
          scenario: scenario.id,
          status,
          checks,
          client,
          backend: observations.backend,
          proxyPlayer,
          proxyLog: status === 'fail' ? proxy.since(mark).filter((l) => /WARN|ERROR|error|refused|timed out/i.test(l)) : [],
        });
        line.push({ pass: '✅', fail: '❌', known: '⚠️', fixed: '🎉' }[status] ?? '?');
      }

      console.log(`${v.label.padEnd(9)} ${String(v.protocol).padStart(4)}${v.inInfrarustTable ? ' ' : '⚑'} ${line.join(' ')}`);
      for (const b of Object.values(backends)) await b.close();
    }
  } finally {
    await proxy.stop();
  }

  const totals = { pass: 0, fail: 0, known: 0, fixed: 0, skip: 0 };
  for (const r of results) totals[r.status] = (totals[r.status] ?? 0) + 1;

  const run = {
    binary,
    tier: args.tier,
    backendKind: useMocksLabel(args.backends),
    caseDelayMs,
    startedAt,
    durationMs: Date.now() - t0,
    versions,
    scenarios: scenarios.map((s) => s.id),
    knownFindings: KNOWN_FINDINGS,
    totals,
    results,
  };
  writeReport(outDir, run);

  console.log(
    `\n${totals.pass} passed · ${totals.fail} failed · ${totals.known} known · ${totals.fixed} newly fixed · ${totals.skip} skipped`,
  );
  console.log(`report: ${outDir}/report.md`);

  if (totals.fail > 0) process.exit(1);
  if (totals.fixed > 0) process.exit(2);
  process.exit(0);
}

function useMocksLabel(kind) {
  return kind === 'mock' ? 'mock (node-minecraft-protocol)' : 'real Java servers';
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
