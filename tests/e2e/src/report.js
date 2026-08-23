import { writeFileSync } from 'node:fs';

const GLYPH = { pass: '✅', fail: '❌', known: '⚠️', fixed: '🎉', skip: '⏭️' };

export function writeReport(dir, run) {
  writeFileSync(`${dir}/report.json`, `${JSON.stringify(run, null, 2)}\n`);
  writeFileSync(`${dir}/report.md`, renderMarkdown(run));
}

export function renderMarkdown(run) {
  const scenarioIds = run.scenarios;
  const out = [];

  out.push('# Infrarust end-to-end connection matrix', '');
  out.push(`- Proxy: \`${run.binary}\``);
  out.push(`- Tier: ${run.tier}`);
  out.push(`- Backends: ${run.backendKind}`);
  out.push(`- Started: ${run.startedAt}`);
  out.push(`- Duration: ${(run.durationMs / 1000).toFixed(1)}s`);
  out.push('');

  const t = run.totals;
  out.push(
    `**${t.pass} passed, ${t.fail} failed, ${t.known} known findings, ${t.fixed} newly fixed, ${t.skip} skipped** ` +
      `across ${run.results.length} cases.`,
    '',
  );

  out.push(`| Version | Protocol | ${scenarioIds.join(' | ')} |`);
  out.push(`|---|---:|${scenarioIds.map(() => '---').join('|')}|`);
  for (const v of run.versions) {
    const cells = scenarioIds.map((id) => {
      const r = run.results.find((x) => x.version === v.version && x.scenario === id);
      return r ? GLYPH[r.status] ?? '?' : '·';
    });
    const flag = v.inInfrarustTable ? '' : ' ⚑';
    out.push(`| ${v.label}${flag} | ${v.protocol} | ${cells.join(' | ')} |`);
  }
  out.push('');
  out.push('⚑ = this protocol number is absent from `ProtocolVersion::SUPPORTED`.');
  out.push('');

  const failures = run.results.filter((r) => r.status === 'fail');
  if (failures.length) {
    out.push('## Failures', '');
    for (const f of failures) {
      out.push(`### ${f.version} · ${f.scenario}`);
      for (const c of f.checks.filter((c) => !c.ok)) out.push(`- **${c.name}**: ${c.detail}`);
      if (f.proxyLog?.length) {
        out.push('', '```');
        out.push(...f.proxyLog.slice(0, 8));
        out.push('```');
      }
      out.push('');
    }
  }

  if (run.knownFindings?.length) {
    out.push('## Known findings', '');
    for (const finding of run.knownFindings) {
      const hits = run.results.filter((r) => r.checks.some((c) => c.known === finding.id));
      const fixedHits = run.results.filter((r) => r.checks.some((c) => c.finding === finding.id && c.ok));
      out.push(`### ${finding.id}`);
      out.push('', finding.summary, '', finding.detail, '');
      out.push(`Reproduces in ${hits.length} case(s).`);
      if (fixedHits.length) {
        out.push('', `> ${fixedHits.length} case(s) no longer reproduce this. If that is intended, delete the entry from \`KNOWN_FINDINGS\`.`);
      }
      out.push('');
    }
  }

  const skipped = run.results.filter((r) => r.status === 'skip');
  if (skipped.length) {
    const reasons = new Map();
    for (const s of skipped) reasons.set(s.reason, (reasons.get(s.reason) ?? 0) + 1);
    out.push('## Skipped', '');
    for (const [reason, count] of reasons) out.push(`- ${reason} — ${count} case(s)`);
    out.push('');
  }

  return out.join('\n');
}
