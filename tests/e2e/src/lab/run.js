#!/usr/bin/env node

import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { ClientFactory } from './clients.js';
import { Backends, Checks, NativeProxy } from './harness.js';
import { Tunnel, releases } from './remote.js';
import { SCENARIOS } from './scenarios.js';

function parseArgs(argv) {
  const opts = { only: null, skip: [], cache: null, donorMc: null, out: null, list: false };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    const next = () => argv[++i];
    switch (arg) {
      case '--only': opts.only = next().split(','); break;
      case '--skip': opts.skip = next().split(','); break;
      case '--cache': opts.cache = next(); break;
      case '--donor-mc': opts.donorMc = next(); break;
      case '--out': opts.out = next(); break;
      case '--list': opts.list = true; break;
      default: throw new Error(`unknown option ${arg}`);
    }
  }
  return opts;
}

function render(results, startedAt) {
  const lines = ['# Remote lab: hot restart and update with real clients', ''];
  lines.push(`Run of ${new Date(startedAt).toISOString()}, ${Math.round((Date.now() - startedAt) / 1000)} s.`, '');
  lines.push('| Scenario | Checks | Failed | Time |', '|---|---|---|---|');
  for (const result of results) {
    const total = result.checks.filter((c) => c.ok !== null).length;
    const failed = result.checks.filter((c) => c.ok === false).length;
    const state = result.error ? '💥' : failed ? '❌' : '✅';
    lines.push(`| ${state} \`${result.id}\` ${result.title} | ${total} | ${failed} | ${Math.round(result.elapsedMs / 1000)} s |`);
  }
  for (const result of results) {
    lines.push('', `## ${result.id}: ${result.title}`, '');
    if (result.error) lines.push(`Crashed: \`${result.error}\``, '');
    for (const check of result.checks) {
      const mark = check.ok === null ? 'ℹ️' : check.ok ? '✅' : '❌';
      lines.push(`- ${mark} ${check.name}${check.detail ? `: ${check.detail}` : ''}`);
    }
  }
  return `${lines.join('\n')}\n`;
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.list) {
    for (const scenario of SCENARIOS) console.log(`${scenario.id.padEnd(22)} ${scenario.title}`);
    return;
  }
  const benchRoot = new URL('../../', import.meta.url).pathname;
  const outDir = opts.out ?? join(benchRoot, 'reports');
  mkdirSync(outDir, { recursive: true });
  const shotsDir = join(outDir, 'lab-shots');

  let selected = SCENARIOS;
  if (opts.only) selected = SCENARIOS.filter((s) => opts.only.includes(s.id));
  selected = selected.filter((s) => !opts.skip.includes(s.id));
  if (selected.length === 0) throw new Error('no scenario selected');

  const tunnel = await new Tunnel().open();
  const factory = new ClientFactory({ cache: opts.cache ?? join(benchRoot, '.mccache'), donorMc: opts.donorMc });
  await factory.prepare();
  const backends = new Backends();
  const native = new NativeProxy();

  const startedAt = Date.now();
  const results = [];
  const cleanup = [];
  const shutdown = async () => {
    for (const fn of cleanup.splice(0).reverse()) await fn().catch(() => {});
    backends.close();
    native.close();
    tunnel.close();
  };
  for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, () => shutdown().finally(() => process.exit(130)));

  try {
    for (const scenario of selected) {
      console.log(`\n== ${scenario.id}: ${scenario.title}`);
      const checks = new Checks();
      const began = Date.now();
      let error = null;
      const local = [];
      try {
        await releases({ visible: [], fault: null, tamper: null, clearHits: true });
        await scenario.run({ checks, factory, backends, native, shotsDir: join(shotsDir, scenario.id), defer: (fn) => local.push(fn) });
      } catch (err) {
        error = `${err.stack ?? err.message}${err.cause ? ` (cause: ${err.cause.message ?? err.cause})` : ""}`;
        console.log(`   [ CRASH ] ${error}`);
      } finally {
        for (const fn of local.reverse()) await fn().catch((err) => console.log(`   cleanup: ${err.message}`));
      }
      results.push({ id: scenario.id, title: scenario.title, checks: checks.items, error, elapsedMs: Date.now() - began });
      writeFileSync(join(outDir, 'lab.md'), render(results, startedAt));
      writeFileSync(join(outDir, 'lab.json'), JSON.stringify({ startedAt, results }, null, 2));
    }
  } finally {
    await shutdown();
  }

  const failed = results.filter((r) => r.error || r.checks.some((c) => c.ok === false));
  console.log(`\n${results.length - failed.length}/${results.length} scenarios green · report: ${join(outDir, 'lab.md')}`);
  process.exit(failed.length ? 1 : 0);
}

main().catch((err) => {
  console.error(err.stack ?? err.message);
  process.exit(1);
});
