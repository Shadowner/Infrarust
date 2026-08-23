import { createHash } from 'node:crypto';
import {
  chmodSync,
  createWriteStream,
  existsSync,
  linkSync,
  mkdirSync,
  readFileSync,
  renameSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import { Readable } from 'node:stream';
import { pipeline } from 'node:stream/promises';
import { dirname, join, resolve } from 'node:path';
import { spawn } from 'node:child_process';

const MANIFEST_URL = 'https://piston-meta.mojang.com/mc/game/version_manifest_v2.json';

const JAVA_RUNTIME_URL =
  'https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json';

const OS_NAME = 'linux';
const OS_ARCH = 'x86_64';

export function cacheRoot(base) {
  base = resolve(base);
  return {
    root: base,
    versions: join(base, 'versions'),
    libraries: join(base, 'libraries'),
    assets: join(base, 'assets'),
    natives: join(base, 'natives'),
    runtimes: join(base, 'runtimes'),
    servers: join(base, 'servers'),
    meta: join(base, 'meta'),
  };
}

async function fetchJson(url) {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`GET ${url} -> ${res.status}`);
  return res.json();
}

function digest(path, algorithm) {
  return createHash(algorithm).update(readFileSync(path)).digest('hex');
}

export async function download(url, dest, { sha1: expectedSha1, sha256: expectedSha256, mode, headers } = {}) {
  const algorithm = expectedSha256 ? 'sha256' : 'sha1';
  const expected = expectedSha256 ?? expectedSha1;

  if (existsSync(dest)) {
    if (!expected || digest(dest, algorithm) === expected) {
      if (mode) chmodQuiet(dest, mode);
      return dest;
    }
  }
  mkdirSync(dirname(dest), { recursive: true });
  const res = await fetch(url, { headers });
  if (!res.ok) throw new Error(`GET ${url} -> ${res.status}`);
  const tmp = `${dest}.part`;
  await pipeline(Readable.fromWeb(res.body), createWriteStream(tmp));
  if (expected && digest(tmp, algorithm) !== expected) {
    throw new Error(`checksum mismatch for ${url}`);
  }
  renameSync(tmp, dest);
  if (mode) chmodQuiet(dest, mode);
  return dest;
}

function chmodQuiet(path, mode) {
  try {
    chmodSync(path, mode);
  } catch {
  }
}

async function cachedJson(url, dest) {
  if (existsSync(dest) && statSync(dest).size > 2) return JSON.parse(readFileSync(dest, 'utf8'));
  const body = await fetchJson(url);
  mkdirSync(dirname(dest), { recursive: true });
  writeFileSync(dest, JSON.stringify(body));
  return body;
}

export async function versionManifest(paths) {
  return cachedJson(MANIFEST_URL, join(paths.meta, 'version_manifest_v2.json'));
}

export async function releasesFrom(paths, from = '1.7.10') {
  const manifest = await versionManifest(paths);
  const releases = manifest.versions.filter((v) => v.type === 'release').reverse();
  const start = releases.findIndex((v) => v.id === from);
  if (start < 0) throw new Error(`${from} is not a release in the manifest`);
  return releases.slice(start);
}

export async function versionJson(paths, id) {
  const manifest = await versionManifest(paths);
  const entry = manifest.versions.find((v) => v.id === id);
  if (!entry) throw new Error(`unknown version ${id}`);
  return cachedJson(entry.url, join(paths.versions, id, `${id}.json`));
}

export function rulesAllow(rules, features = {}) {
  if (!rules || rules.length === 0) return true;
  let allowed = false;
  for (const rule of rules) {
    let matches = true;
    if (rule.os) {
      if (rule.os.name && rule.os.name !== OS_NAME) matches = false;
      if (rule.os.arch && rule.os.arch !== OS_ARCH && rule.os.arch !== 'x86_64') matches = false;
      if (rule.os.version && !new RegExp(rule.os.version).test(process.version)) {
        matches = false;
      }
    }
    if (rule.features) {
      for (const [name, want] of Object.entries(rule.features)) {
        if (Boolean(features[name]) !== Boolean(want)) matches = false;
      }
    }
    if (matches) allowed = rule.action === 'allow';
  }
  return allowed;
}

function nativeClassifier(library) {
  const key = library.natives?.[OS_NAME];
  if (!key) return null;
  return key.replace('${arch}', '64');
}

export function resolveLibraries(paths, vjson) {
  const classpath = [];
  const nativeArchives = [];

  for (const library of vjson.libraries) {
    if (!rulesAllow(library.rules)) continue;

    const artifact = library.downloads?.artifact;
    if (artifact) {
      classpath.push({
        path: join(paths.libraries, artifact.path),
        url: artifact.url,
        sha1: artifact.sha1,
      });
    } else if (!library.natives && library.name) {
      const path = mavenPath(library.name);
      classpath.push({
        path: join(paths.libraries, path),
        url: `${library.url ?? 'https://libraries.minecraft.net/'}${path}`,
        sha1: null,
      });
    }

    const classifier = nativeClassifier(library);
    if (classifier) {
      const native = library.downloads?.classifiers?.[classifier];
      if (native) {
        nativeArchives.push({
          path: join(paths.libraries, native.path),
          url: native.url,
          sha1: native.sha1,
          exclude: library.extract?.exclude ?? [],
        });
      }
    }
  }
  return { classpath, nativeArchives };
}

function mavenPath(name) {
  const [group, artifact, version] = name.split(':');
  return `${group.replace(/\./g, '/')}/${artifact}/${version}/${artifact}-${version}.jar`;
}

export async function installAssets(paths, vjson, { donorAssets, concurrency = 16, onProgress } = {}) {
  const index = vjson.assetIndex;
  if (!index) return { assetsDir: paths.assets, indexId: 'legacy', downloaded: 0, linked: 0 };

  const indexPath = join(paths.assets, 'indexes', `${index.id}.json`);
  await download(index.url, indexPath, { sha1: index.sha1 });
  const objects = JSON.parse(readFileSync(indexPath, 'utf8')).objects;

  const entries = Object.values(objects);
  let downloaded = 0;
  let linked = 0;
  let done = 0;

  const queue = entries.slice();
  const worker = async () => {
    for (;;) {
      const entry = queue.pop();
      if (!entry) return;
      const rel = join(entry.hash.slice(0, 2), entry.hash);
      const dest = join(paths.assets, 'objects', rel);
      done++;
      if (existsSync(dest)) continue;
      if (donorAssets) {
        const donor = join(donorAssets, 'objects', rel);
        if (existsSync(donor)) {
          mkdirSync(dirname(dest), { recursive: true });
          try {
            linkSync(donor, dest);
            linked++;
            continue;
          } catch {
          }
        }
      }
      await download(`https://resources.download.minecraft.net/${rel}`, dest, { sha1: entry.hash });
      downloaded++;
      if (onProgress && downloaded % 200 === 0) onProgress({ done, total: entries.length, downloaded, linked });
    }
  };
  await Promise.all(Array.from({ length: concurrency }, worker));

  return { assetsDir: paths.assets, indexId: index.id, downloaded, linked, total: entries.length };
}

const SYSTEM_JAVA = {
  8: '/usr/lib/jvm/java-8-openjdk/bin/java',
  11: '/usr/lib/jvm/java-11-openjdk/bin/java',
  17: '/usr/lib/jvm/java-17-openjdk/bin/java',
  21: '/usr/lib/jvm/java-21-openjdk/bin/java',
};

export async function resolveJava(paths, vjson, { allowDownload = true } = {}) {
  const component = vjson.javaVersion?.component ?? 'jre-legacy';
  const major = vjson.javaVersion?.majorVersion ?? 8;

  const system = SYSTEM_JAVA[major];
  if (system && existsSync(system)) return { java: system, source: `system java ${major}` };

  const installed = join(paths.runtimes, component, 'bin', 'java');
  if (existsSync(installed)) return { java: installed, source: `mojang ${component}` };
  if (!allowDownload) throw new Error(`no java ${major} available for ${vjson.id}`);

  await installMojangRuntime(paths, component);
  if (!existsSync(installed)) throw new Error(`mojang runtime ${component} did not yield ${installed}`);
  return { java: installed, source: `mojang ${component}` };
}

export async function installMojangRuntime(paths, component) {
  const all = await cachedJson(JAVA_RUNTIME_URL, join(paths.meta, 'java-runtime-all.json'));
  const platform = all['linux'] ?? all['linux-i386'];
  const variants = platform?.[component];
  if (!variants || variants.length === 0) {
    throw new Error(`mojang publishes no linux build of ${component}`);
  }
  const manifest = await fetchJson(variants[0].manifest.url);
  const target = join(paths.runtimes, component);

  const files = Object.entries(manifest.files);
  const queue = files.slice();
  const worker = async () => {
    for (;;) {
      const next = queue.pop();
      if (!next) return;
      const [rel, spec] = next;
      const dest = join(target, rel);
      if (spec.type === 'directory') {
        mkdirSync(dest, { recursive: true });
      } else if (spec.type === 'link') {
        mkdirSync(dirname(dest), { recursive: true });
        if (!existsSync(dest)) {
          try {
            symlinkSync(spec.target, dest);
          } catch {
          }
        }
      } else if (spec.type === 'file') {
        const raw = spec.downloads.raw;
        await download(raw.url, dest, { sha1: raw.sha1, mode: spec.executable ? 0o755 : 0o644 });
      }
    }
  };
  await Promise.all(Array.from({ length: 12 }, worker));
  return target;
}

export async function installClient(paths, id, { donorAssets, donorLibraries, onProgress } = {}) {
  const vjson = await versionJson(paths, id);

  const clientJar = join(paths.versions, id, `${id}.jar`);
  await download(vjson.downloads.client.url, clientJar, { sha1: vjson.downloads.client.sha1 });

  const { classpath, nativeArchives } = resolveLibraries(paths, vjson);
  for (const lib of [...classpath, ...nativeArchives]) {
    if (existsSync(lib.path)) continue;
    if (donorLibraries) {
      const donor = lib.path.replace(paths.libraries, donorLibraries);
      if (existsSync(donor) && (!lib.sha1 || digest(donor, 'sha1') === lib.sha1)) {
        mkdirSync(dirname(lib.path), { recursive: true });
        try {
          linkSync(donor, lib.path);
          continue;
        } catch {
        }
      }
    }
    await download(lib.url, lib.path, { sha1: lib.sha1 });
  }

  const nativesDir = join(paths.natives, id);
  if (nativeArchives.length > 0 && !existsSync(join(nativesDir, '.complete'))) {
    mkdirSync(nativesDir, { recursive: true });
    for (const archive of nativeArchives) {
      await unzipInto(archive.path, nativesDir, archive.exclude);
    }
    writeFileSync(join(nativesDir, '.complete'), '');
  }

  const assets = await installAssets(paths, vjson, { donorAssets, onProgress });

  return {
    id,
    vjson,
    clientJar,
    classpath: [...classpath.map((l) => l.path), clientJar],
    nativesDir,
    assets,
  };
}

async function unzipInto(zipPath, dir, exclude = []) {
  const args = ['-o', '-q', zipPath, '-d', dir];
  for (const pattern of exclude) args.push('-x', `${pattern}*`);
  await new Promise((resolve, reject) => {
    const child = spawn('unzip', args, { stdio: 'ignore' });
    child.once('error', reject);
    child.once('exit', (code) => (code === 0 ? resolve() : reject(new Error(`unzip ${zipPath} exited ${code}`))));
  });
}
