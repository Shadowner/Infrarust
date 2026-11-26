import { connectBots, settled } from './bots.js';
import { AUTHENTICATED, DockerProxy, Fleet, Pulse, RESUMED, restartAndWait } from './harness.js';
import { releases, sh, sleep } from './remote.js';
import { PORTS, REMOTE_ROOT, route, routes } from './topology.js';

const MIXED_FLEET = [
  { mode: 'pt', version: '1.8.9' },
  { mode: 'pt', version: '1.12.2' },
  { mode: 'zc', version: '1.19.4' },
  { mode: 'so', version: '1.20.1' },
  { mode: 'pt', version: '1.21.11' },
  { mode: 'of', version: '1.21.4' },
  { mode: 'co', version: '1.21.4' },
  { mode: 'co', version: '1.20.1' },
];

const SMALL_FLEET = [
  { mode: 'pt', version: '1.21.4' },
  { mode: 'zc', version: '1.20.1' },
  { mode: 'pt', version: '1.8.9' },
  { mode: 'of', version: '1.21.4' },
];

const RESUME_FLEET = [
  { mode: 'pt', version: '1.21.4' },
  { mode: 'of', version: '1.20.6' },
  { mode: 'of', version: '1.21.4' },
  { mode: 'co', version: '1.21.4' },
  { mode: 'ofv', version: '1.21.4' },
  { mode: 'cov', version: '1.21.4' },
  { mode: 'co', version: '1.21.11' },
  { mode: 'of', version: '1.19.4' },
];

const ANNOUNCE_FLEET = [
  { mode: 'pt', version: '1.21.4' },
  { mode: 'co', version: '1.21.4' },
  { mode: 'of', version: '1.20.1' },
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

async function swapBinary(proxy, build) {
  await sh(
    `cp ${REMOTE_ROOT}/dist/${build}/infrarust ${proxy.home}/.infrarust.swap && mv ${proxy.home}/.infrarust.swap ${proxy.home}/infrarust`,
  );
}

const token = (prefix) => `${prefix}${Math.random().toString(36).slice(2, 8).toUpperCase()}`;

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

export async function restartRound(
  ctx,
  fleet,
  label,
  trigger,
  { expect = 'committed', proxy = ctx.native, sendsBack = true, timeoutMs = 45000 } = {},
) {
  const { checks } = ctx;
  const survivors = fleet.survivors.slice();
  const intercepted = fleet.intercepted.slice();
  const returning = sendsBack ? fleet.returning.slice() : [];
  const kicked = sendsBack ? fleet.kicked.slice() : intercepted;
  const backendMarks = new Map(fleet.members.map((m) => [m, m.backendLog.mark()]));
  const clientMarks = new Map(fleet.members.map((m) => [m, m.client.mark()]));
  const proxyMark = proxy.log.mark();
  const pidBefore = await proxy.pid();
  const pulse = new Pulse(survivors).start();
  await sleep(2000);

  const result = await restartAndWait(proxy, trigger, { timeoutMs });
  checks.add(`${label}: the restart ends as ${expect}`, result.outcome === expect, result.line ?? 'no outcome line in the log');
  const trips = expect === 'refused' ? [] : await Promise.all(returning.map((member) => member.returned(backendMarks.get(member))));
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

  if (expect === 'refused') {
    for (const member of intercepted) {
      const tag = `${label}: ${member.username} (${member.route.mode.proxyMode}, ${member.route.version.id})`;
      checks.add(`${tag} is untouched`, !member.left(backendMarks.get(member)) && member.client.alive);
    }
    return result;
  }

  const proxyLines = proxy.log.since(proxyMark).map((line) => line.text);
  const away = [];
  const lostOnTheWay = [];
  for (const [index, member] of returning.entries()) {
    const tag = `${label}: ${member.username} (${member.route.mode.proxyMode}${member.route.mode.forwarding === 'none' ? '' : ' + velocity'}, ${member.route.version.id})`;
    const trip = trips[index];
    const back = Boolean(trip.back) && member.client.alive;
    checks.add(
      `${tag} comes back on its own, in the same client`,
      back,
      trip.back ? `away from its backend for ${trip.awayMs} ms` : trip.left ? 'left and did not come back' : 'never left its backend',
    );
    if (!back) {
      await member.client.screenshot(`${label}-not-back`);
      member.client.saveLog(`${label}-not-back`);
      lostOnTheWay.push(member);
      continue;
    }
    away.push(trip.awayMs);
    const resumed = proxyLines.find((line) => RESUMED.test(line) && line.includes(member.username));
    checks.add(`${tag} is resumed from its ticket`, Boolean(resumed), resumed?.slice(0, 200) ?? 'no resume line in the proxy log');
    if (member.route.mode.auth) {
      const again = proxyLines.find((line) => AUTHENTICATED.test(line) && line.includes(member.username));
      checks.add(`${tag} is not sent through the session server again`, !again, again?.slice(0, 200) ?? '');
    }
    const shown = member.disconnectReason(clientMarks.get(member));
    checks.add(`${tag} never sees a disconnect screen`, !/restart|kicked/i.test(shown ?? ''), shown ?? '');
    const token = await member.token('r');
    checks.add(`${tag} hears the server again`, token.ok, token.ms === null ? '' : `${token.ms} ms`);
    const talks = await member.chat('r');
    if (!talks) {
      await member.client.screenshot(`${label}-back-mute`);
      member.client.saveLog(`${label}-back-mute`);
    }
    checks.add(`${tag} talks to the server again`, talks);
  }
  if (away.length) checks.note(`${label}: time away from the backend for the players sent back`, `${away.join(', ')} ms`);

  const players = await proxy.admin.players();
  for (const member of survivors.concat(returning.filter((m) => !lostOnTheWay.includes(m)))) {
    checks.add(`${label}: the proxy still lists ${member.username}`, players.some((p) => p.username === member.username));
  }

  for (const member of kicked) {
    const tag = `${label}: ${member.username} (${member.route.mode.proxyMode}, ${member.route.version.id})`;
    const logged = member.disconnectReason(clientMarks.get(member));
    const screen = await member.client.screenText(`${label}-kicked`);
    const told = /restart/i.test(logged ?? '') ? `log: ${logged}` : /restart/i.test(screen) ? `screen: ${screen}` : null;
    checks.add(`${tag} is told the proxy restarts`, Boolean(told), told ?? `log: ${logged ?? 'nothing'} | screen: ${screen || 'unreadable'}`);
    checks.add(`${tag} left its backend`, Boolean(member.left(backendMarks.get(member))));
  }
  const replaced = kicked.concat(lostOnTheWay);
  for (const member of replaced) await fleet.remove(member);
  if (replaced.length) {
    const again = await fleet.add(replaced.map((m) => ({ mode: m.route.mode.key, version: m.route.version.id })));
    checks.add(
      `${label}: the players who were disconnected get back in afterwards`,
      again.failures.length === 0,
      `${again.added.length}/${replaced.length}`,
    );
    for (const failure of again.failures) await failure.member.client.stop();
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
    id: 'resume',
    title: 'players on 1.20.5 and later come back on their own, through new and older binaries',
    async run(ctx) {
      const { checks, native } = ctx;
      await ensureNative(ctx);
      const fleet = await fleetOf(ctx, RESUME_FLEET);
      const botRoutes = [route('of', '1.20.6'), route('of', '1.21.4'), route('of', '1.21.11')];
      const bots = await connectBots({ count: 6, prefix: 'Rs', routes: botRoutes, port: native.port, follow: true, settleMs: 30000 });
      ctx.defer(async () => bots.forEach((bot) => bot.close()));
      const scripted = await settled(bots);
      checks.add('the six scripted clients are in play', scripted === 6, `${scripted}/6`);

      await restartRound(ctx, fleet, 'SIGUSR2', () => native.signal('USR2'));
      await settled(bots);
      for (const bot of bots) {
        const sent = bot.sentBack[0];
        checks.add(
          `${bot.username} (${bot.route.version.id}) decodes a 32 byte cookie and a transfer to where it came from`,
          Boolean(sent) &&
            sent.key === 'infrarust:resume' &&
            sent.valueLength === 32 &&
            sent.host === bot.route.domain &&
            sent.port === native.port &&
            sent.state === 'play' &&
            sent.transferState === 'play',
          JSON.stringify(sent ?? null),
        );
        checks.add(
          `${bot.username} is asked for its cookie once and plays again`,
          bot.cookieRequests === 1 && bot.returns === 1 && bot.inPlay,
          `cookie requests ${bot.cookieRequests}, returns ${bot.returns}, ${bot.kick ?? bot.error ?? 'no error'}`,
        );
      }

      await restartRound(ctx, fleet, 'second trip', () => native.admin.post('/proxy/restart'));
      await settled(bots);
      checks.add(
        'the scripted clients make the second trip with a new ticket',
        bots.every((bot) => bot.returns === 2 && bot.inPlay && bot.sentBack.length === 2),
        bots.map((bot) => `${bot.returns}/${bot.sentBack.length}`).join(', '),
      );

      await swapBinary(native, 'legacy');
      await restartRound(ctx, fleet, 'into a binary that cannot resume', () => native.signal('USR2'), { sendsBack: false });
      checks.add('the older binary runs', (await proxyVersion(native)) === '2.0.0-beta.6', await proxyVersion(native));
      checks.add(
        'the scripted clients are told to reconnect instead of being sent back',
        bots.every((bot) => bot.sentBack.length === 2 && /restarting/i.test(bot.kick ?? '')),
        bots.map((bot) => `${bot.sentBack.length}:${bot.kick ?? bot.error ?? 'nothing'}`).join(' | ').slice(0, 300),
      );

      await swapBinary(native, 'n');
      await restartRound(ctx, fleet, 'back from the older binary', () => native.signal('USR2'), { sendsBack: false });
      checks.add('the current binary runs again', (await proxyVersion(native)) === '2.0.0-beta.5', await proxyVersion(native));
      await restartRound(ctx, fleet, 'after the round trip', () => native.signal('USR2'));
    },
  },
  {
    id: 'announce',
    title: 'a restart is announced in chat, can be called off, and waits for its delay',
    async run(ctx) {
      const { checks, native } = ctx;
      await ensureNative(ctx);
      const fleet = await fleetOf(ctx, ANNOUNCE_FLEET);
      const [quiet] = fleet.survivors;
      const [modern] = fleet.returning;
      const [older] = fleet.kicked;
      const readers = [modern, older].filter(Boolean);
      const marks = new Map(fleet.members.map((m) => [m, m.client.mark()]));
      const backendMarks = new Map(fleet.members.map((m) => [m, m.backendLog.mark()]));
      const pid = await native.pid();

      const first = token('LAB');
      const asked = await native.admin.post('/proxy/restart', { delay_seconds: 600, message: `&e${first} in {delay} ({seconds})` });
      checks.add(
        'a restart with a delay is accepted and says when',
        asked.status === 202 && /600 seconds/.test(asked.body?.data?.message ?? ''),
        `${asked.status} ${asked.text.slice(0, 140)}`,
      );
      for (const member of readers) {
        const seen = await member.client.waitFor(new RegExp(`${first} in 10m \\(600\\)`), { since: marks.get(member), timeoutMs: 20000 });
        checks.add(`${member.username} (${member.route.version.id}) reads the notice in chat`, Boolean(seen), seen?.text.slice(-120) ?? '');
      }
      const waiting = (await native.admin.get('/proxy')).body?.data ?? {};
      checks.add(
        'the status shows the countdown',
        waiting.restart_in_seconds > 540 && waiting.restart_in_seconds <= 600 && typeof waiting.restart_scheduled_at === 'string',
        `${waiting.restart_in_seconds} s, ${waiting.restart_scheduled_at}`,
      );
      const twice = await native.admin.post('/proxy/restart');
      checks.add('a second request during the countdown is a conflict', twice.status === 409, `${twice.status} ${twice.text.slice(0, 120)}`);

      const called = await native.admin.request('DELETE', '/proxy/restart');
      checks.add('the restart is called off', called.status === 200, `${called.status} ${called.text.slice(0, 120)}`);
      for (const member of readers) {
        const seen = await member.client.waitFor(/The restart is cancelled/, { since: marks.get(member), timeoutMs: 20000 });
        checks.add(`${member.username} is told the restart is cancelled`, Boolean(seen), seen?.text.slice(-120) ?? '');
      }
      await sleep(1500);
      const idle = (await native.admin.get('/proxy')).body?.data ?? {};
      checks.add('the countdown is gone from the status', !('restart_in_seconds' in idle) && !('restart_scheduled_at' in idle), JSON.stringify(idle).slice(0, 200));
      checks.add('the proxy did not restart', (await native.pid()) === pid && idle.uptime_seconds > 0, `uptime ${idle.uptime_seconds} s`);
      for (const member of fleet.members) {
        checks.add(`${member.username} is still on its backend after the cancellation`, !member.left(backendMarks.get(member)) && member.client.alive);
      }
      const nothing = await native.admin.request('DELETE', '/proxy/restart');
      checks.add('cancelling again is a conflict', nothing.status === 409, `${nothing.status} ${nothing.text.slice(0, 120)}`);
      const tooLong = await native.admin.post('/proxy/restart', { delay_seconds: 7200 });
      checks.add('a delay over an hour is refused with 400', tooLong.status === 400, `${tooLong.status} ${tooLong.text.slice(0, 140)}`);

      const second = token('LAB');
      const result = await restartRound(
        ctx,
        fleet,
        'announced restart',
        () => native.admin.post('/proxy/restart', { delay_seconds: 12, message: `${second} {seconds}` }),
        { timeoutMs: 90000 },
      );
      checks.add('the restart waited for its delay', result.doneAt - result.startedAt >= 12000, `${result.doneAt - result.startedAt} ms from the request to the commit`);
      for (const member of readers) {
        const said = member.client
          .since(marks.get(member))
          .map((line) => line.text.match(new RegExp(`${second} (\\d+)`)))
          .filter(Boolean)
          .map((match) => Number(match[1]));
        checks.add(
          `${member.username} (${member.route.version.id}) reads the notice and its reminders`,
          [12, 10, 5, 1].every((left) => said.includes(left)),
          `seconds announced: ${said.join(', ')}`,
        );
      }
      if (quiet) {
        const heard = quiet.client
          .since(marks.get(quiet))
          .some((line) => line.text.includes(first) || line.text.includes(second) || /The restart is cancelled/.test(line.text));
        checks.add(`${quiet.username} (${quiet.route.mode.proxyMode}) is not told anything, its traffic is only relayed`, !heard);
      }
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
    title: '360 sessions and four real clients through ten restarts',
    async run(ctx) {
      const { checks, native } = ctx;
      await ensureNative(ctx);
      const fleet = await fleetOf(ctx, SMALL_FLEET.filter((spec) => route(spec.mode, spec.version).mode.survives).concat([{ mode: 'zc', version: '1.21.11' }]));
      const botRoutes = [route('pt', '1.21.4'), route('zc', '1.21.4'), route('pt', '1.20.1'), route('zc', '1.20.1'), route('so', '1.19.4'), route('zc', '1.19.4')];
      const bots = await connectBots({ count: 300, prefix: 'Ld', routes: botRoutes, port: native.port, ratePerSecond: 25, settleMs: 30000 });
      ctx.defer(async () => bots.forEach((bot) => bot.close()));
      const inPlay = bots.filter((bot) => bot.inPlay).length;
      checks.add('the 300 sessions are in play', inPlay === 300, `${inPlay}/300`);
      const comerRoutes = [route('of', '1.21.4'), route('of', '1.21.11'), route('of', '1.20.6')];
      const comers = await connectBots({ count: 60, prefix: 'Lr', routes: comerRoutes, port: native.port, ratePerSecond: 20, settleMs: 30000, follow: true });
      ctx.defer(async () => comers.forEach((bot) => bot.close()));
      const comersIn = await settled(comers);
      checks.add('60 more sessions are in play on offline routes', comersIn === 60, `${comersIn}/60`);
      const resumeMark = native.log.mark();
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
        await settled(comers, { timeoutMs: 20000 });
      }
      await sleep(35000);
      await pulse.stop();
      checks.add('the ten restarts are committed', outcomes.every((o) => o === 'committed'), outcomes.join(', '));
      checks.note('signal to commit, per restart', `${pauses.join(', ')} ms`);
      const alive = bots.filter((bot) => bot.inPlay).length;
      const lost = bots.filter((bot) => !bot.inPlay);
      checks.add('every session is still in play 35 s after the last restart', alive === inPlay, `${alive}/${inPlay}; ${lost.slice(0, 5).map((b) => `${b.username}:${b.kick ?? b.error ?? 'eof'}`).join(', ')}`);
      const refused = comers.filter((bot) => !bot.inPlay && /throttled/i.test(bot.kick ?? ''));
      const steady = comers.filter((bot) => !refused.includes(bot));
      const trips = steady.map((bot) => bot.returns);
      checks.add(
        'the offline sessions come back after each of the ten restarts',
        steady.every((bot) => bot.returns === 10 && bot.inPlay) && refused.length <= 3,
        `trips per session: min ${Math.min(...trips)}, max ${Math.max(...trips)}; in play ${steady.filter((bot) => bot.inPlay).length}/${steady.length}; ${steady
          .filter((bot) => !bot.inPlay)
          .slice(0, 4)
          .map((bot) => `${bot.username}:${bot.kick ?? bot.error ?? 'eof'}`)
          .join(', ')}`,
      );
      if (refused.length) {
        checks.note(
          'sessions the backend refused after the proxy resumed them',
          `${refused.length}/60 (${refused.map((bot) => `${bot.username} on ${bot.route.version.id} after ${bot.returns} trips`).join(', ')}): ${refused[0].kick}`,
        );
      }
      const made = comers.reduce((sum, bot) => sum + bot.returns, 0);
      const resumedLines = native.log.since(resumeMark).filter((line) => RESUMED.test(line.text)).length;
      checks.add(
        'the proxy resumed each of them from a ticket every time',
        resumedLines >= made && resumedLines <= made + refused.length,
        `${resumedLines} resume lines for ${made} trips made, ${refused.length} refused by the backend`,
      );
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
      checks.add('the proxy lists as many players as before', playersAfter === playersBefore - refused.length, `${playersBefore} -> ${playersAfter}, ${refused.length} refused by the backend`);
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
