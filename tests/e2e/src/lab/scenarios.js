import { connectBots } from './bots.js';
import { DockerProxy, Fleet, Pulse, restartAndWait } from './harness.js';
import { releases, sh, sleep } from './remote.js';
import { PORTS, REMOTE_ROOT, route, routes } from './topology.js';

const MIXED_FLEET = [
  { mode: 'pt', version: '1.8.9' },
  { mode: 'pt', version: '1.12.2' },
  { mode: 'zc', version: '1.19.4' },
  { mode: 'so', version: '1.20.1' },
  { mode: 'zc', version: '1.21.4' },
  { mode: 'pt', version: '1.21.11' },
  { mode: 'of', version: '1.21.4' },
  { mode: 'co', version: '1.20.1' },
];

const SMALL_FLEET = [
  { mode: 'pt', version: '1.21.4' },
  { mode: 'zc', version: '1.20.1' },
  { mode: 'pt', version: '1.8.9' },
  { mode: 'of', version: '1.21.4' },
];

async function updateStatus(proxy) {
  const res = await proxy.admin.get('/proxy/update');
  return res.body?.data ?? null;
}

async function checkNow(proxy) {
  const res = await proxy.admin.post('/proxy/update/check');
  return { status: res.status, data: res.body?.data ?? null, text: res.text };
}

async function installViaApi(proxy, body, { timeoutMs = 90000 } = {}) {
  const res = await proxy.admin.post('/proxy/update/install', body);
  if (res.status !== 202) return { accepted: false, http: res.status, text: res.text, status: await updateStatus(proxy) };
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const status = await updateStatus(proxy);
    if (status && status.install.state !== 'running' && status.install.state !== 'idle') {
      return { accepted: true, http: 202, text: res.text, status };
    }
    if (Date.now() > deadline) return { accepted: true, http: 202, text: res.text, status, timedOut: true };
    await sleep(250);
  }
}

async function proxyVersion(proxy) {
  const res = await proxy.admin.get('/proxy');
  return res.body?.data?.version ?? null;
}

async function leftovers(proxy) {
  const out = await sh(`ls -A ${proxy.home} | grep -E '^\\.infrarust\\.(new|link)|^\\.infrarust\\.new' || true`);
  return out.stdout.trim();
}

async function ensureNative(ctx, build = 'n') {
  await ctx.native.install(build);
}

async function fleetOf(ctx, specs, proxy = ctx.native) {
  const fleet = new Fleet({ factory: ctx.factory, backends: ctx.backends, proxy, shotsDir: ctx.shotsDir });
  ctx.defer(() => fleet.stop());
  const { failures } = await fleet.add(specs);
  for (const failure of failures) {
    ctx.checks.add(`${failure.member.username} joins`, false, failure.tail.slice(-3).join(' | '));
  }
  ctx.checks.add('every client of the fleet is in its world', failures.length === 0, `${fleet.members.length}/${specs.length}`);
  return fleet;
}

export async function restartRound(ctx, fleet, label, trigger, { expect = 'committed', proxy = ctx.native } = {}) {
  const { checks } = ctx;
  const survivors = fleet.survivors.slice();
  const intercepted = fleet.intercepted.slice();
  const backendMarks = new Map(fleet.members.map((m) => [m, m.backendLog.mark()]));
  const clientMarks = new Map(fleet.members.map((m) => [m, m.client.mark()]));
  const pidBefore = await proxy.pid();
  const pulse = new Pulse(survivors).start();
  await sleep(2000);

  const result = await restartAndWait(proxy, trigger);
  checks.add(`${label}: the restart ends as ${expect}`, result.outcome === expect, result.line ?? 'no outcome line in the log');
  await sleep(4000);
  await pulse.stop();

  const pidAfter = await proxy.pid();
  checks.add(`${label}: the process is the same`, String(pidBefore) === String(pidAfter), `${pidBefore} -> ${pidAfter}`);
  if (expect === 'committed') {
    checks.add(
      `${label}: the uptime starts again`,
      result.after && result.before && result.after.uptime_seconds <= result.before.uptime_seconds,
      `${result.before?.uptime_seconds}s -> ${result.after?.uptime_seconds}s`,
    );
  }

  const gaps = [];
  for (const member of survivors) {
    const tag = `${label}: ${member.username} (${member.route.mode.proxyMode}, ${member.route.version.id})`;
    const left = member.left(backendMarks.get(member));
    checks.add(`${tag} stays on its backend`, !left && member.client.alive, left?.text ?? '');
    const report = pulse.report(member, { from: result.startedAt, to: result.doneAt + 4000 });
    gaps.push(report.maxGapMs);
    checks.add(
      `${tag} loses no server message`,
      report.missing === 0 && !report.outOfOrder && report.received > 0,
      `${report.received}/${report.sent} received, longest silence ${report.maxGapMs} ms`,
    );
    const token = await member.token('a');
    checks.add(`${tag} still hears the server`, token.ok, token.ms === null ? '' : `${token.ms} ms`);
    const talks = await member.chat('a');
    if (!talks) {
      await member.client.screenshot(`${label}-mute`);
      member.client.saveLog(`${label}-mute`);
    }
    checks.add(`${tag} still talks to the server`, talks);
  }
  if (gaps.length) checks.note(`${label}: longest silence seen by a real client`, `${Math.max(...gaps)} ms`);

  const players = await proxy.admin.players();
  for (const member of survivors) {
    checks.add(
      `${label}: the proxy still lists ${member.username}`,
      players.some((p) => p.username === member.username),
    );
  }

  if (expect !== 'refused') {
    for (const member of intercepted) {
      const tag = `${label}: ${member.username} (${member.route.mode.proxyMode}, ${member.route.version.id})`;
      const logged = member.disconnectReason(clientMarks.get(member));
      const screen = await member.client.screenText(`${label}-kicked`);
      const told = /restart/i.test(logged ?? '') ? `log: ${logged}` : /restart/i.test(screen) ? `screen: ${screen}` : null;
      checks.add(`${tag} is told the proxy restarts`, Boolean(told), told ?? `log: ${logged ?? 'nothing'} | screen: ${screen || 'unreadable'}`);
      checks.add(`${tag} left its backend`, Boolean(member.left(backendMarks.get(member))));
      await fleet.remove(member);
    }
    if (intercepted.length) {
      const again = await fleet.add(intercepted.map((m) => ({ mode: m.route.mode.key, version: m.route.version.id })));
      checks.add(
        `${label}: the intercepted players get back in afterwards`,
        again.failures.length === 0,
        `${again.added.length}/${intercepted.length}`,
      );
      for (const failure of again.failures) await failure.member.client.stop();
    }
  } else {
    for (const member of intercepted) {
      const tag = `${label}: ${member.username} (${member.route.mode.proxyMode}, ${member.route.version.id})`;
      checks.add(`${tag} is untouched`, !member.left(backendMarks.get(member)) && member.client.alive);
    }
  }
  return result;
}

export const SCENARIOS = [
  {
    id: 'join-matrix',
    title: 'every client version enters every proxy mode',
    async run(ctx) {
      await ensureNative(ctx);
      const queue = routes().slice();
      let serial = 0;
      const worker = async () => {
        for (;;) {
          const entry = queue.shift();
          if (!entry) return;
          const fleet = new Fleet({ factory: ctx.factory, backends: ctx.backends, proxy: ctx.native, shotsDir: ctx.shotsDir });
          const tag = `${entry.version.id} ${entry.mode.proxyMode}${entry.mode.forwarding === 'none' ? '' : ' + velocity'}`;
          try {
            const { added, failures } = await fleet.add([
              { mode: entry.mode.key, version: entry.version.id, username: `J${entry.mode.key}${entry.version.key}x${serial++}` },
            ]);
            if (failures.length) {
              ctx.checks.add(`${tag}: joins`, false, failures[0].tail.slice(-2).join(' | '));
              continue;
            }
            const member = added[0];
            const token = await member.token('j');
            const chat = await member.chat('j');
            const player = (await ctx.native.admin.players()).find((p) => p.username === member.username);
            const active = player?.is_active === !entry.mode.survives;
            if (!(token.ok && chat && player && active)) {
              await member.client.screenshot('cell-failed');
              member.client.saveLog('cell-failed');
            }
            ctx.checks.add(
              `${tag}: joins, hears and talks`,
              token.ok && chat && Boolean(player) && active,
              `server->client ${token.ok}, client->server ${chat}, listed ${Boolean(player)}, is_active ${player?.is_active}`,
            );
          } finally {
            await fleet.stop();
          }
        }
      };
      await Promise.all(Array.from({ length: 4 }, worker));
    },
  },
  {
    id: 'restart-triggers',
    title: 'one fleet lives through SIGUSR2, systemctl reload and the admin API',
    async run(ctx) {
      await ensureNative(ctx);
      const fleet = await fleetOf(ctx, MIXED_FLEET);
      await restartRound(ctx, fleet, 'SIGUSR2', () => ctx.native.signal('USR2'));
      await restartRound(ctx, fleet, 'systemctl reload', () => ctx.native.reload());
      await restartRound(ctx, fleet, 'admin API', async () => {
        const res = await ctx.native.admin.post('/proxy/restart');
        ctx.checks.add('admin API: POST /proxy/restart answers 202', res.status === 202, `${res.status} ${res.text.slice(0, 120)}`);
      });
      const status = await ctx.native.admin.get('/proxy');
      ctx.checks.add('the proxy reports that it can restart in place', status.body?.data?.restart_available === true);
    },
  },
  {
    id: 'update-chain',
    title: 'check, install, restart onto the new release, roll back, restart again',
    async run(ctx) {
      const { checks, native } = ctx;
      await ensureNative(ctx);
      await releases({ visible: ['v2.0.0-beta.6'] });
      const fleet = await fleetOf(ctx, SMALL_FLEET);
      const shaBefore = await native.binarySha();

      const check = await checkNow(native);
      checks.add('the check offers 2.0.0-beta.6', check.data?.available?.version === '2.0.0-beta.6', check.text.slice(0, 200));
      checks.add('the release can take over in place', check.data?.available?.restart === 'hot', check.data?.available?.restart);
      const cli = await native.cli('update check --json');
      checks.add('infrarust update check exits 100 when an update exists', cli.code === 100, `exit ${cli.code}`);

      const install = await installViaApi(native);
      checks.add('the install ends as installed', install.status?.install?.state === 'installed', JSON.stringify(install.status?.install));
      const shaNew = await native.binarySha();
      checks.add('the binary on disk changed', shaNew !== shaBefore);
      const old = await sh(`sha256sum ${native.home}/infrarust.old | cut -d' ' -f1`, { allowFailure: true });
      checks.add('the previous binary is kept as infrarust.old', old.stdout.trim() === shaBefore);
      checks.add('no temporary file is left', (await leftovers(native)) === '', await leftovers(native));
      checks.add('the proxy still runs the old version', (await proxyVersion(native)) === '2.0.0-beta.5');
      const changed = (await updateStatus(native))?.binary_changed;
      checks.add('the status says the binary changed', changed === true, String(changed));

      await restartRound(ctx, fleet, 'onto beta.6', () => native.admin.post('/proxy/restart'));
      checks.add('the proxy now runs 2.0.0-beta.6', (await proxyVersion(native)) === '2.0.0-beta.6', await proxyVersion(native));

      const rollback = await native.cli('update rollback --yes');
      checks.add('infrarust update rollback succeeds', rollback.code === 0, (rollback.stdout + rollback.stderr).trim().split('\n').slice(-2).join(' | '));
      checks.add('the binary on disk is the first one again', (await native.binarySha()) === shaBefore);

      await restartRound(ctx, fleet, 'back onto beta.5', () => native.signal('USR2'));
      checks.add('the proxy runs 2.0.0-beta.5 again', (await proxyVersion(native)) === '2.0.0-beta.5', await proxyVersion(native));
    },
  },
  {
    id: 'update-refusals',
    title: 'a forged, damaged or unreachable release never replaces the binary',
    async run(ctx) {
      const { checks, native } = ctx;
      await ensureNative(ctx);
      const sha = await native.binarySha();
      const unchanged = async (label) => {
        checks.add(`${label}: the binary is untouched`, (await native.binarySha()) === sha);
        checks.add(`${label}: no temporary file is left`, (await leftovers(native)) === '', await leftovers(native));
      };
      await releases({ visible: ['v2.0.0-beta.6'] });

      for (const tamper of ['signature', 'manifest']) {
        await releases({ tamper });
        const mark = native.log.mark();
        const check = await checkNow(native);
        const warned = await native.log.waitFor(/release skipped.*signature/i, { since: mark, timeoutMs: 5000 });
        checks.add(
          `forged ${tamper}: the check offers nothing and the log says why`,
          !check.data?.available && Boolean(warned),
          warned?.text.slice(0, 220) ?? check.text.slice(0, 160),
        );
        const install = await installViaApi(native, { version: '2.0.0-beta.6' });
        checks.add(
          `forged ${tamper}: the install fails on the signature`,
          install.status?.install?.state === 'failed' && /signature/i.test(install.status.install.reason ?? ''),
          JSON.stringify(install.status?.install ?? install.text),
        );
        await unchanged(`forged ${tamper}`);
      }

      await releases({ tamper: 'archive' });
      const damaged = await installViaApi(native, { version: '2.0.0-beta.6' });
      checks.add(
        'damaged archive: the install fails on the checksum',
        damaged.status?.install?.state === 'failed' && /sha256|checksum|hash/i.test(damaged.status.install.reason ?? ''),
        JSON.stringify(damaged.status?.install ?? damaged.text),
      );
      await unchanged('damaged archive');
      await releases({ tamper: null });

      const faults = [
        ['403-quota', /rate limit/i, true],
        ['429', /rate limit/i, true],
        ['403-plain', /403/, false],
        ['500', /500/, false],
      ];
      for (const [fault, pattern, limited] of faults) {
        await releases({ fault });
        const check = await checkNow(native);
        const error = check.data?.last_error ?? '';
        checks.add(
          `feed answers ${fault}: the check fails with ${limited ? 'a rate limit' : 'the status'}`,
          pattern.test(error) && (limited || !/rate limit/i.test(error)),
          error || check.text.slice(0, 160),
        );
      }
      await releases({ fault: null });

      const bad = await native.admin.post('/proxy/update/install', { version: 'latest' });
      checks.add('a version that is not a number is refused with 400', bad.status === 400, `${bad.status} ${bad.text.slice(0, 120)}`);
      const typo = await native.admin.post('/proxy/update/install', { versoin: '2.0.0-beta.6' });
      checks.add('an unknown field is refused', typo.status >= 400 && typo.status < 500, `${typo.status}`);
      const missing = await installViaApi(native, { version: '9.9.9' });
      checks.add(
        'a release that does not exist fails cleanly',
        !missing.accepted || missing.status?.install?.state === 'failed',
        JSON.stringify(missing.status?.install ?? missing.text).slice(0, 200),
      );
      await unchanged('unknown release');

      await releases({ slow: true });
      const first = await native.admin.post('/proxy/update/install', { version: '2.0.0-beta.6' });
      await sleep(1500);
      const second = await native.admin.post('/proxy/update/install', { version: '2.0.0-beta.6' });
      checks.add('a second install during a running one is refused with 409', first.status === 202 && second.status === 409, `${first.status} then ${second.status} ${second.text.slice(0, 120)}`);
      const mark = native.log.mark();
      const restart = await native.admin.post('/proxy/restart');
      const refused = await native.log.waitFor(/install/i, { since: mark, timeoutMs: 8000 });
      checks.add(
        'a restart asked during an install is refused',
        restart.status === 409 || Boolean(refused),
        `${restart.status} ${restart.text.slice(0, 100)} | ${refused?.text.slice(0, 160) ?? 'no log line'}`,
      );
      const deadline = Date.now() + 60000;
      let status = await updateStatus(native);
      while (status?.install?.state === 'running' && Date.now() < deadline) {
        await sleep(500);
        status = await updateStatus(native);
      }
      await releases({ slow: false });
      checks.add('the slow install still completes', status?.install?.state === 'installed', JSON.stringify(status?.install));
      checks.add('the proxy kept running the old version through all of it', (await proxyVersion(native)) === '2.0.0-beta.5');
    },
  },
  {
    id: 'bad-release',
    title: 'a release that cannot take over hands the players back',
    async run(ctx) {
      const { checks, native } = ctx;
      await ensureNative(ctx);
      await releases({ visible: ['v2.0.0-beta.7'] });
      const fleet = await fleetOf(ctx, SMALL_FLEET);
      const sha = await native.binarySha();
      const install = await installViaApi(native, { version: '2.0.0-beta.7' });
      checks.add('the faulty release installs, it is signed and probes fine', install.status?.install?.state === 'installed', JSON.stringify(install.status?.install));

      const result = await restartRound(ctx, fleet, 'faulty release', () => native.signal('USR2'), { expect: 'rolled-back' });
      checks.add('the proxy still runs 2.0.0-beta.5', (await proxyVersion(native)) === '2.0.0-beta.5', await proxyVersion(native));
      checks.add('the log says why the new image gave up', /envelope version/.test(result.line ?? ''), (result.line ?? '').slice(0, 260));

      const second = await restartRound(ctx, fleet, 'faulty release, second try', () => native.signal('USR2'), { expect: 'rolled-back' });
      checks.note('second attempt outcome line', second.line ?? '');

      const rollback = await native.cli('update rollback --yes');
      checks.add('update rollback puts the good binary back', rollback.code === 0 && (await native.binarySha()) === sha, `exit ${rollback.code}`);
      await restartRound(ctx, fleet, 'after the rollback', () => native.signal('USR2'));
    },
  },
  {
    id: 'incompatible-release',
    title: 'a release with another handover protocol asks for a full restart',
    async run(ctx) {
      const { checks, native } = ctx;
      await ensureNative(ctx);
      await releases({ visible: ['v2.0.0-beta.8'] });
      const fleet = await fleetOf(ctx, SMALL_FLEET);
      const check = await checkNow(native);
      checks.add('the check announces a full restart', check.data?.available?.restart === 'full', JSON.stringify(check.data?.available));
      const install = await installViaApi(native);
      checks.add('the release installs', install.status?.install?.state === 'installed', JSON.stringify(install.status?.install));
      const result = await restartRound(ctx, fleet, 'incompatible release', () => native.admin.post('/proxy/restart'), { expect: 'refused' });
      checks.note('refusal', result.line ?? JSON.stringify(result.triggered?.text ?? ''));
      checks.add('the proxy still runs 2.0.0-beta.5', (await proxyVersion(native)) === '2.0.0-beta.5');
      await fleet.stop();
      await sh('sudo -n systemctl restart infrarust-lab');
      await native.admin.waitAlive();
      checks.add('a full restart runs 2.0.0-beta.8', (await proxyVersion(native)) === '2.0.0-beta.8', await proxyVersion(native));
    },
  },
  {
    id: 'broken-config',
    title: 'a configuration the binary rejects cancels the restart',
    async run(ctx) {
      const { checks, native } = ctx;
      await ensureNative(ctx);
      const fleet = await fleetOf(ctx, SMALL_FLEET);
      const config = `${native.home}/infrarust.toml`;
      await sh(`cp ${config} ${config}.good && printf '\\nthis is = = not toml\\n' >> ${config}`);
      ctx.defer(() => sh(`test -f ${config}.good && mv ${config}.good ${config} || true`));
      const result = await restartRound(ctx, fleet, 'broken config', () => native.signal('USR2'), { expect: 'refused' });
      checks.note('refusal', result.line ?? '');
      await sh(`mv ${config}.good ${config}`);
      await restartRound(ctx, fleet, 'config repaired', () => native.signal('USR2'));
    },
  },
  {
    id: 'bind-change',
    title: 'a new bind address is applied while forwarding players stay',
    async run(ctx) {
      const { checks, native } = ctx;
      await ensureNative(ctx);
      const fleet = await fleetOf(ctx, SMALL_FLEET.filter((spec) => route(spec.mode, spec.version).mode.survives));
      const config = `${native.home}/infrarust.toml`;
      await sh(`cp ${config} ${config}.good && sed -i 's/^bind = "127.0.0.1:${PORTS.native.game}"/bind = "127.0.0.1:25567"/' ${config}`);
      ctx.defer(() => sh(`test -f ${config}.good && mv ${config}.good ${config} || true`));
      await restartRound(ctx, fleet, 'bind moved', () => native.signal('USR2'));
      const listening = await sh("ss -ltnH | awk '{print $4}' | grep -E ':(25565|25567)$' | sort | tr '\\n' ' '");
      checks.add('the proxy listens on the new address only', listening.stdout.trim() === '127.0.0.1:25567', listening.stdout.trim());
      await sh(`mv ${config}.good ${config}`);
      await restartRound(ctx, fleet, 'bind moved back', () => native.signal('USR2'));
      const back = await sh("ss -ltnH | awk '{print $4}' | grep -E ':(25565|25567)$' | sort | tr '\\n' ' '");
      checks.add('the proxy listens on the first address again', back.stdout.trim() === `127.0.0.1:${PORTS.native.game}`, back.stdout.trim());
    },
  },
  {
    id: 'stop-during-restart',
    title: 'SIGTERM right behind SIGUSR2 stops the proxy',
    async run(ctx) {
      const { checks, native } = ctx;
      for (const delay of [0, 0.05, 0.3, 1]) {
        await ensureNative(ctx);
        const fleet = await fleetOf(ctx, [{ mode: 'pt', version: '1.21.4' }]);
        const mark = native.log.mark();
        await sh(
          `sudo -n systemctl kill -s USR2 --kill-who=main infrarust-lab; sleep ${delay}; sudo -n systemctl kill -s TERM --kill-who=main infrarust-lab`,
        );
        const deadline = Date.now() + 50000;
        let state = await native.active();
        while (state !== 'inactive' && state !== 'failed' && Date.now() < deadline) {
          await sleep(500);
          state = await native.active();
        }
        const result = await sh('systemctl show -p Result -p ExecMainStatus --value infrarust-lab');
        checks.add(
          `TERM ${delay}s after USR2: the proxy stops cleanly`,
          state === 'inactive',
          `${state}, ${result.stdout.trim().replace(/\n/g, ' ')}, ${Math.round((Date.now() - (deadline - 50000)) / 1000)} s`,
        );
        const lines = native.log.since(mark).map((l) => l.text);
        checks.note(`TERM ${delay}s after USR2: last log lines`, lines.slice(-3).join(' | ').slice(0, 300));
        await fleet.stop();
      }
    },
  },
  {
    id: 'logins-during-restart',
    title: 'players who log in while the proxy restarts are served or told why',
    async run(ctx) {
      const { checks, native } = ctx;
      await ensureNative(ctx);
      const targets = [route('pt', '1.21.4'), route('of', '1.21.4'), route('zc', '1.20.1'), route('of', '1.19.4')];
      const bots = [];
      let running = true;
      const spawner = (async () => {
        let n = 0;
        while (running) {
          const batch = await connectBots({ count: 1, prefix: `Lg${n++}x`, routes: [targets[n % targets.length]], port: native.port, settleMs: 0 });
          bots.push(...batch.map((bot) => Object.assign(bot, { startedAt: Date.now() })));
          await sleep(120);
        }
      })();
      ctx.defer(async () => {
        running = false;
        for (const bot of bots) bot.close();
      });
      await sleep(3000);
      const results = [];
      for (let i = 0; i < 3; i++) {
        results.push(await restartAndWait(native, () => native.signal('USR2')));
        await sleep(3000);
      }
      running = false;
      await spawner;
      await sleep(12000);
      checks.add('the three restarts are committed', results.every((r) => r.outcome === 'committed'), results.map((r) => r.outcome).join(', '));
      const served = bots.filter((bot) => bot.playAt !== null);
      const told = bots.filter((bot) => bot.playAt === null && bot.kick);
      const hung = bots.filter((bot) => bot.playAt === null && bot.endedAt === null);
      const dropped = bots.filter((bot) => bot.playAt === null && bot.endedAt !== null && !bot.kick);
      checks.note('logins attempted', `${bots.length}: ${served.length} served, ${told.length} told why, ${dropped.length} closed without a word, ${hung.length} left hanging`);
      checks.add('no login is left hanging', hung.length === 0, hung.slice(0, 5).map((b) => `${b.username}@${b.route.id}`).join(', '));
      checks.add('no login is closed without a message', dropped.length === 0, dropped.slice(0, 8).map((b) => `${b.username}@${b.route.id}:${b.error ?? 'eof'}`).join(', '));
      const reasons = [...new Set(told.map((bot) => bot.kick))];
      checks.note('kick reasons seen', reasons.join(' | ').slice(0, 400));
      const servedForwarding = served.filter((bot) => bot.route.mode.survives);
      const lost = servedForwarding.filter((bot) => bot.endedAt !== null);
      checks.add('a forwarding player who got in stays in', lost.length === 0, `${lost.length}/${servedForwarding.length} lost: ${lost.slice(0, 5).map((b) => `${b.username}:${b.kick ?? b.error ?? 'eof'}`).join(', ')}`);
    },
  },
  {
    id: 'load',
    title: 'three hundred sessions and four real clients through ten restarts',
    async run(ctx) {
      const { checks, native } = ctx;
      await ensureNative(ctx);
      const fleet = await fleetOf(ctx, SMALL_FLEET.filter((spec) => route(spec.mode, spec.version).mode.survives).concat([{ mode: 'zc', version: '1.21.11' }]));
      const botRoutes = [route('pt', '1.21.4'), route('zc', '1.21.4'), route('pt', '1.20.1'), route('zc', '1.20.1'), route('so', '1.19.4'), route('zc', '1.19.4')];
      const bots = await connectBots({ count: 300, prefix: 'Ld', routes: botRoutes, port: native.port, ratePerSecond: 25, settleMs: 30000 });
      ctx.defer(async () => bots.forEach((bot) => bot.close()));
      const inPlay = bots.filter((bot) => bot.inPlay).length;
      checks.add('the 300 sessions are in play', inPlay === 300, `${inPlay}/300`);
      await sleep(5000);
      const before = await native.stats();
      const playersBefore = (await native.admin.players()).length;
      const windows = bots.map((bot) => bot.watchGap());
      const pulse = new Pulse(fleet.members).start();
      const startedAt = Date.now();
      const outcomes = [];
      const pauses = [];
      for (let i = 0; i < 10; i++) {
        const result = await restartAndWait(native, () => native.signal('USR2'));
        outcomes.push(result.outcome);
        pauses.push(result.doneAt - result.startedAt);
        await sleep(2500);
      }
      await sleep(35000);
      await pulse.stop();
      checks.add('the ten restarts are committed', outcomes.every((o) => o === 'committed'), outcomes.join(', '));
      checks.note('signal to commit, per restart', `${pauses.join(', ')} ms`);
      const alive = bots.filter((bot) => bot.inPlay).length;
      const lost = bots.filter((bot) => !bot.inPlay);
      checks.add('every session is still in play 35 s after the last restart', alive === inPlay, `${alive}/${inPlay}; ${lost.slice(0, 5).map((b) => `${b.username}:${b.kick ?? b.error ?? 'eof'}`).join(', ')}`);
      const gaps = windows.map((w) => w.maxGapMs).sort((a, b) => a - b);
      checks.note('longest silence per session', `median ${gaps[Math.floor(gaps.length / 2)]} ms, p99 ${gaps[Math.floor(gaps.length * 0.99)]} ms, max ${gaps[gaps.length - 1]} ms`);
      for (const member of fleet.members) {
        const report = pulse.report(member, { from: startedAt, to: Date.now() });
        checks.add(
          `${member.username} (${member.route.mode.proxyMode}, ${member.route.version.id}) loses no server message over ten restarts`,
          report.missing === 0 && !report.outOfOrder && member.client.alive,
          `${report.received}/${report.sent}, longest silence ${report.maxGapMs} ms`,
        );
        checks.add(`${member.username} still talks to the server`, await member.chat('l'));
      }
      const after = await native.stats();
      const playersAfter = (await native.admin.players()).length;
      checks.add('the proxy lists as many players as before', playersAfter === playersBefore, `${playersBefore} -> ${playersAfter}`);
      checks.add('no descriptor leaks', Math.abs(after.fds - before.fds) <= 8, `${before.fds} -> ${after.fds}`);
      checks.note('memory and threads', `RSS ${Math.round(before.rssKb / 1024)} MiB -> ${Math.round(after.rssKb / 1024)} MiB, threads ${before.threads} -> ${after.threads}`);
      checks.add('the process is the same after ten restarts', before.pid === after.pid, `${before.pid} -> ${after.pid}`);
    },
  },
  {
    id: 'docker',
    title: 'the Docker image restarts in place and refuses to update itself',
    async run(ctx) {
      const { checks } = ctx;
      const proxy = new DockerProxy();
      await proxy.up();
      ctx.defer(() => proxy.down());
      await releases({ visible: ['v2.0.0-beta.6'] });
      const fleet = await fleetOf(ctx, SMALL_FLEET, proxy);
      await restartRound(ctx, fleet, 'docker kill USR2', () => proxy.signal('USR2'), { proxy });
      await restartRound(ctx, fleet, 'console restart', () => proxy.console('restart'), { proxy });
      const check = await checkNow(proxy);
      checks.add('the image still sees the new release', check.data?.available?.version === '2.0.0-beta.6', check.text.slice(0, 160));
      const install = await proxy.admin.post('/proxy/update/install');
      checks.add('the image refuses to install it', install.status === 409 && /docker|image|container/i.test(install.text), `${install.status} ${install.text.slice(0, 200)}`);
      await fleet.stop();
      const stopped = await sh('sudo -n docker stop -t 45 lab-proxy && sudo -n docker inspect -f "{{.State.ExitCode}}" lab-proxy', { allowFailure: true });
      checks.add('docker stop ends the proxy with exit code 0', stopped.stdout.trim().endsWith('0'), stopped.stdout.trim().replace(/\n/g, ' '));
    },
  },
];
