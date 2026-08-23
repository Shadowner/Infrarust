const GLYPH = { pass: '✅', fail: '❌', skip: '⏭️', untrusted: '⚠️' };

function evidence(obs) {
  if (!obs) return null;
  if (obs.proxyGaveUp || obs.stalledOut) return obs.proxyWindow?.slice(-10) ?? obs.clientTail?.slice(-8);
  if (obs.serverWindow?.length) return obs.serverWindow.slice(-8);
  return obs.clientTail?.slice(-8);
}

export function renderReport(results, scenarios) {
  const counts = { pass: 0, fail: 0, skip: 0, untrusted: 0 };
  for (const row of results) for (const cell of row.cells) counts[cell.status] = (counts[cell.status] ?? 0) + 1;

  const header = ['version', ...scenarios.map((s) => s.id)];
  const lines = [
    '# Real-client connection matrix',
    '',
    'One column per scenario, one row per Minecraft release. Every cell is the',
    'actual game client launched headless, joining a real server of the same',
    'version; a green cell means that server logged the player into the world.',
    '',
    `| ${header.join(' | ')} |`,
    `|${header.map(() => '---').join('|')}|`,
  ];

  for (const row of results) {
    const cells = scenarios.map((s) => {
      const cell = row.cells.find((c) => c.scenario === s.id);
      return GLYPH[cell?.status] ?? '·';
    });
    lines.push(`| ${row.version} | ${cells.join(' | ')} |`);
  }

  const failures = [];
  for (const row of results) {
    for (const cell of row.cells) {
      if (cell.status === 'fail') failures.push({ version: row.version, ...cell });
    }
  }

  if (failures.length > 0) {
    lines.push('', '## Failures', '');
    for (const failure of failures) {
      lines.push(`### ${failure.version} · ${failure.scenario}`, '', failure.detail, '');
      const tail = evidence(failure.observations);
      if (tail?.length) lines.push('```', ...tail, '```', '');
    }
  }

  const skipped = new Map();
  for (const row of results) {
    for (const cell of row.cells) {
      if (cell.status !== 'skip') continue;
      const key = `${cell.scenario}: ${cell.detail}`;
      skipped.set(key, (skipped.get(key) ?? 0) + 1);
    }
  }
  const untrusted = results.filter((r) => r.cells.some((c) => c.status === 'untrusted'));
  if (untrusted.length > 0) {
    lines.push('## Untrusted', '');
    lines.push('These releases could not reach a server even without the proxy, so nothing');
    lines.push('their proxied cells say is attributable to Infrarust.', '');
    for (const row of untrusted) lines.push(`- ${row.version}: ${row.notes.join('; ')}`);
    lines.push('');
  }

  if (skipped.size > 0) {
    lines.push('## Skipped', '');
    for (const [reason, count] of skipped) lines.push(`- ${reason} (${count})`);
    lines.push('');
  }

  const summary =
    `${counts.pass ?? 0} passed · ${counts.fail ?? 0} failed · ` +
    `${counts.untrusted ?? 0} untrusted · ${counts.skip ?? 0} skipped`;
  lines.push('', summary, '');

  return { markdown: lines.join('\n'), summary, counts };
}
