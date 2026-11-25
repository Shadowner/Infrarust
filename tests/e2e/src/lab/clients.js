import { execFile, spawn } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { Yggdrasil } from '../yggdrasil.js';
import { existsSync } from 'node:fs';

import { Display } from '../real/display.js';
import { buildLaunch, setReleaseOrder } from '../real/launch.js';
import { cacheRoot, installClient, releasesFrom, resolveJava } from '../real/mojang.js';
import { fetchAuthlibInjector, offlineSession, onlineSession } from '../real/session.js';
import { YGGDRASIL_URL } from './topology.js';
import { sleep } from './remote.js';

const OPTIONS = [
  'version:100',
  'soundCategory_master:0.0',
  'soundCategory_music:0.0',
  'renderDistance:2',
  'simulationDistance:5',
  'graphicsMode:0',
  'fancyGraphics:false',
  'maxFps:20',
  'enableVsync:false',
  'guiScale:1',
  'pauseOnLostFocus:false',
  'skipMultiplayerWarning:true',
  'onboardAccessibility:false',
  'joinedFirstServer:true',
  'tutorialStep:none',
  'narrator:0',
  'realmsNotifications:false',
  'telemetryOptInExtra:false',
  '',
].join('\n');

let displayQueue = Promise.resolve();
let nextDisplay = 140;

function ownDisplay() {
  const claim = displayQueue.then(async () => {
    for (let tries = 0; tries < 60; tries++) {
      const number = nextDisplay++;
      if (existsSync(`/tmp/.X11-unix/X${number}`)) continue;
      const display = new Display(number);
      try {
        await display.start();
      } catch {
        continue;
      }
      await sleep(300);
      if (display.child.exitCode === null) return display;
    }
    throw new Error('no free X display for a client');
  });
  displayQueue = claim.catch(() => {});
  return claim;
}

function run(command, args, env) {
  return new Promise((resolve) => {
    execFile(command, args, { env: { ...process.env, ...env } }, (error, stdout, stderr) => {
      resolve({ ok: !error, stdout: stdout.toString(), stderr: stderr.toString() });
    });
  });
}

export class ClientFactory {
  constructor({ cache, donorMc = null }) {
    this.cache = cache;
    this.paths = cacheRoot(cache);
    this.donorMc = donorMc;
    this.installs = new Map();
    this.yggdrasil = null;
    this.injectorJar = null;
    this.ready = null;
  }

  async prepare() {
    if (!this.ready) {
      this.ready = (async () => {
        for (const dir of Object.values(this.paths)) mkdirSync(dir, { recursive: true });
        mkdirSync(join(this.cache, 'tools'), { recursive: true });
        const releases = await releasesFrom(this.paths, '1.7.10');
        setReleaseOrder(releases.map((release) => release.id));
        this.yggdrasil = new Yggdrasil(YGGDRASIL_URL);
        await this.yggdrasil.ready();
        this.injectorJar = await fetchAuthlibInjector(this.paths);
      })();
    }
    return this.ready;
  }

  async install(versionId) {
    await this.prepare();
    if (!this.installs.has(versionId)) {
      const install = await installClient(this.paths, versionId, {
        donorAssets: this.donorMc ? join(this.donorMc, 'assets') : undefined,
        donorLibraries: this.donorMc ? join(this.donorMc, 'libraries') : undefined,
      });
      const { java } = await resolveJava(this.paths, install.vjson);
      this.installs.set(versionId, { install, java });
    }
    return this.installs.get(versionId);
  }

  async client({ username, route, port, shotsDir }) {
    const { install, java } = await this.install(route.version.id);
    const session = route.mode.auth
      ? await onlineSession(this.yggdrasil, username, { injectorJar: this.injectorJar })
      : offlineSession(username);
    const gameDir = join(this.cache, 'gamedirs', `lab-${username}`);
    const launch = buildLaunch(install, { gameDir, session, target: { host: route.domain, port }, java });
    return new RealClient({ username: session.username, route, launch, gameDir, shotsDir });
  }
}

export class RealClient {
  constructor({ username, route, launch, gameDir, shotsDir }) {
    Object.assign(this, { username, route, launch, gameDir, shotsDir });
    this.lines = [];
    this.child = null;
    this.display = null;
    this.exit = null;
  }

  async start() {
    mkdirSync(this.gameDir, { recursive: true });
    writeFileSync(join(this.gameDir, 'options.txt'), OPTIONS);
    this.display = await ownDisplay();
    const args = this.launch.args.map((arg) => (arg === '-Xmx2G' ? '-Xmx1G' : arg));
    this.child = spawn(this.launch.java, args, {
      cwd: this.gameDir,
      env: {
        ...process.env,
        DISPLAY: this.display.name,
        WAYLAND_DISPLAY: '',
        LIBGL_ALWAYS_SOFTWARE: '1',
        GALLIUM_DRIVER: 'llvmpipe',
        LP_NUM_THREADS: '2',
        ALSOFT_DRIVERS: 'null',
        __GLX_VENDOR_LIBRARY_NAME: 'mesa',
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let rest = '';
    const onData = (chunk) => {
      const parts = (rest + chunk.toString()).split('\n');
      rest = parts.pop();
      for (const text of parts) if (text.trim()) this.lines.push({ at: Date.now(), text });
    };
    this.child.stdout.on('data', onData);
    this.child.stderr.on('data', onData);
    this.child.once('exit', (code, signal) => {
      this.exit = { code, signal, at: Date.now() };
    });
    return this;
  }

  get alive() {
    return this.child !== null && this.exit === null;
  }

  mark() {
    return this.lines.length;
  }

  since(mark) {
    return this.lines.slice(mark);
  }

  async waitFor(pattern, { since = 0, timeoutMs = 30000 } = {}) {
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      const found = this.lines.slice(since).find((line) => pattern.test(line.text));
      if (found) return found;
      if (!this.alive || Date.now() > deadline) return null;
      await sleep(50);
    }
  }

  xdo(...args) {
    return run('xdotool', args, { DISPLAY: this.display.name });
  }

  async focus() {
    const found = await this.xdo('search', '--onlyvisible', '--name', 'Minecraft');
    const window = found.stdout.trim().split('\n').filter(Boolean).pop();
    if (!window) return null;
    await this.xdo('windowfocus', window);
    return window;
  }

  async key(...keys) {
    await this.focus();
    return this.xdo('key', '--delay', '60', ...keys);
  }

  async press(key, holdMs = 150) {
    await this.xdo('keydown', key);
    await sleep(holdMs);
    await this.xdo('keyup', key);
  }

  async chat(text) {
    await this.focus();
    await this.press('t');
    await sleep(700);
    await this.xdo('type', '--delay', '70', text);
    await sleep(400);
    await this.press('Return');
  }

  async hold(key, ms) {
    await this.focus();
    await this.xdo('keydown', key);
    await sleep(ms);
    await this.xdo('keyup', key);
  }

  async click(x, y) {
    await this.focus();
    await this.xdo('mousemove', String(x), String(y));
    await sleep(150);
    await this.xdo('click', '1');
  }

  async screenshot(name) {
    if (!this.shotsDir) return null;
    mkdirSync(this.shotsDir, { recursive: true });
    const path = join(this.shotsDir, `${name}.png`);
    const shot = await run('import', ['-display', this.display.name, '-window', 'root', path], {});
    return shot.ok ? path : null;
  }

  saveLog(name) {
    if (!this.shotsDir) return null;
    mkdirSync(this.shotsDir, { recursive: true });
    const path = join(this.shotsDir, `${name}.log`);
    writeFileSync(path, this.lines.map((line) => `${new Date(line.at).toISOString()} ${line.text}`).join('\n'));
    return path;
  }

  async screenText(name) {
    const shot = await this.screenshot(name);
    if (!shot) return '';
    const prepared = `${shot}.ocr.png`;
    const made = await run('magick', [shot, '-resize', '400%', '-colorspace', 'Gray', '-threshold', '60%', '-negate', prepared], {});
    if (!made.ok) return '';
    const langs = await run('tesseract', ['--list-langs'], {});
    const known = langs.stdout.split('\n').slice(1).map((l) => l.trim()).filter((l) => l && l !== 'osd');
    const lang = known.includes('eng') ? 'eng' : known[0];
    if (!lang) return '';
    const read = await run('tesseract', [prepared, '-', '-l', lang, '--psm', '6'], {});
    await run('rm', ['-f', prepared], {});
    return read.stdout.replace(/\s+/g, ' ').trim();
  }

  async stop() {
    if (this.child && this.exit === null) {
      this.child.kill('SIGKILL');
      await sleep(200);
    }
    if (this.display) await this.display.stop();
  }
}
