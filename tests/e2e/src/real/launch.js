import { spawn } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { rulesAllow } from './mojang.js';

export const QUICK_PLAY_FROM = '1.20';

let releaseOrder = [];
export function setReleaseOrder(ids) {
  releaseOrder = ids;
}

export function usesQuickPlay(id) {
  const here = releaseOrder.indexOf(id);
  const boundary = releaseOrder.indexOf(QUICK_PLAY_FROM);
  if (here < 0 || boundary < 0) throw new Error(`release order does not contain ${id} and ${QUICK_PLAY_FROM}`);
  return here >= boundary;
}

function substitute(value, vars) {
  return value.replace(/\$\{([a-zA-Z0-9_]+)\}/g, (whole, name) => (name in vars ? vars[name] : whole));
}

function flatten(list, vars, features) {
  const out = [];
  for (const entry of list ?? []) {
    if (typeof entry === 'string') {
      out.push(substitute(entry, vars));
    } else if (rulesAllow(entry.rules, features)) {
      const values = Array.isArray(entry.value) ? entry.value : [entry.value];
      for (const value of values) out.push(substitute(value, vars));
    }
  }
  return out;
}

export function buildLaunch(install, { gameDir, session, target, java, width = 854, height = 480 }) {
  const { vjson, classpath, nativesDir, assets } = install;

  const vars = {
    auth_player_name: session.username,
    version_name: vjson.id,
    game_directory: gameDir,
    assets_root: assets.assetsDir,
    game_assets: assets.assetsDir,
    assets_index_name: assets.indexId,
    auth_uuid: session.uuid,
    auth_access_token: session.accessToken,
    auth_session: `token:${session.accessToken}:${session.uuid}`,
    user_type: session.userType ?? 'msa',
    user_properties: '{}',
    version_type: 'release',
    clientid: '00000000-0000-0000-0000-000000000000',
    auth_xuid: '0',
    natives_directory: nativesDir,
    launcher_name: 'infrarust-e2e',
    launcher_version: '1',
    classpath: classpath.join(':'),
    classpath_separator: ':',
    library_directory: install.librariesDir ?? '',
    resolution_width: String(width),
    resolution_height: String(height),
  };

  const quickPlay = usesQuickPlay(vjson.id);
  const features = {
    has_custom_resolution: true,
    is_demo_user: false,
    has_quick_plays_support: quickPlay,
    is_quick_play_multiplayer: quickPlay,
    is_quick_play_singleplayer: false,
    is_quick_play_realms: false,
  };

  if (quickPlay) {
    vars.quickPlayMultiplayer = `${target.host}:${target.port}`;
    vars.quickPlayPath = join(gameDir, 'quickplay.json');
  }

  const jvm = vjson.arguments
    ? flatten(vjson.arguments.jvm, vars, features)
    : [`-Djava.library.path=${nativesDir}`, '-cp', vars.classpath];

  const game = vjson.arguments
    ? flatten(vjson.arguments.game, vars, features)
    : substitute(vjson.minecraftArguments, vars).split(' ').filter(Boolean);

  if (!quickPlay) {
    game.push('--server', target.host, '--port', String(target.port));
  }

  const args = [
    ...jvm,
    '-Xms512M',
    '-Xmx2G',
    '-XX:+UseG1GC',
    '-Dfile.encoding=UTF-8',
    ...(session.javaagent ? [session.javaagent] : []),
    ...(session.jvmExtra ?? []),
    vjson.mainClass,
    ...game,
  ];

  return { java, args, vars, quickPlay };
}

export function runClient({ java, args, gameDir, display, timeoutMs = 180000, onLine, isDone, stopWhen }) {
  mkdirSync(gameDir, { recursive: true });
  writeFileSync(join(gameDir, 'options.txt'), OPTIONS);

  return new Promise((resolve) => {
    const started = Date.now();
    const lines = [];
    let settled = false;

    const child = spawn(java, args, {
      cwd: gameDir,
      env: {
        ...process.env,
        DISPLAY: display,
        LIBGL_ALWAYS_SOFTWARE: '1',
        GALLIUM_DRIVER: 'llvmpipe',
        ALSOFT_DRIVERS: 'null',
        __GLX_VENDOR_LIBRARY_NAME: 'mesa',
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    });

    const finish = (outcome) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      try {
        child.kill('SIGKILL');
      } catch {
      }
      resolve({ ...outcome, elapsedMs: Date.now() - started, lines });
    };

    const timer = setTimeout(() => finish({ reason: 'timeout' }), timeoutMs);

    if (stopWhen) stopWhen.then((value) => finish({ reason: 'stopped', value }), () => {});

    const onData = (chunk) => {
      for (const line of chunk.toString().split('\n')) {
        if (!line.trim()) continue;
        lines.push(line);
        if (onLine) onLine(line);
        if (!settled && isDone && isDone(line)) finish({ reason: 'signal', line });
      }
    };
    child.stdout.on('data', onData);
    child.stderr.on('data', onData);

    child.once('error', (err) => finish({ reason: 'spawn-failed', error: err.message }));
    child.once('exit', (code, signal) => {
      if (settled) return;
      finish({ reason: 'exited', exitCode: code, signal });
    });
  });
}

const OPTIONS = [
  'version:100',
  'soundCategory_master:0.0',
  'soundCategory_music:0.0',
  'renderDistance:2',
  'simulationDistance:5',
  'graphicsMode:0',
  'fancyGraphics:false',
  'ao:false',
  'maxFps:30',
  'enableVsync:false',
  'guiScale:1',
  'skipMultiplayerWarning:true',
  'onboardAccessibility:false',
  'joinedFirstServer:true',
  'tutorialStep:none',
  'narrator:0',
  'realmsNotifications:false',
  'telemetryOptInExtra:false',
  '',
].join('\n');
