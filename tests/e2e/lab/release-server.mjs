import { createHash } from 'node:crypto';
import { createReadStream, existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { createServer } from 'node:http';
import { join } from 'node:path';

const DATA = process.env.LAB_RELEASES ?? '/data';
const PORT = Number(process.env.LAB_PORT ?? 8900);
const HOST = process.env.LAB_HOST ?? '127.0.0.1';
const BASE = process.env.LAB_BASE ?? `http://127.0.0.1:${PORT}`;

const state = { visible: [], fault: null, tamper: null, slow: false, hits: [] };

function releases() {
  return state.visible
    .filter((tag) => existsSync(join(DATA, tag)))
    .map((tag) => {
      const dir = join(DATA, tag);
      const manifest = JSON.parse(readFileSync(join(dir, 'manifest.json'), 'utf8'));
      return {
        tag_name: tag,
        draft: false,
        prerelease: tag.includes('-'),
        published_at: manifest.published_at,
        html_url: `${BASE}/notes/${tag}`,
        assets: readdirSync(dir).map((name) => ({
          name,
          browser_download_url: `${BASE}/dl/${tag}/${name}`,
          size: statSync(join(dir, name)).size,
        })),
      };
    });
}

function send(res, status, body, headers = {}) {
  const payload = typeof body === 'string' || Buffer.isBuffer(body) ? body : JSON.stringify(body);
  res.writeHead(status, { 'content-type': 'application/json', ...headers });
  res.end(payload);
}

function fault(res) {
  const reset = String(Math.floor(Date.now() / 1000) + 600);
  switch (state.fault) {
    case '403-quota':
      send(res, 403, { message: 'API rate limit exceeded' }, { 'x-ratelimit-remaining': '0', 'x-ratelimit-reset': reset });
      return true;
    case '403-plain':
      send(res, 403, { message: 'Forbidden' }, { 'x-ratelimit-remaining': '41' });
      return true;
    case '429':
      send(res, 429, { message: 'Too many requests' }, { 'retry-after': '120' });
      return true;
    case '500':
      send(res, 500, { message: 'Server error' });
      return true;
    default:
      return false;
  }
}

function tampered(name, bytes) {
  if (state.tamper === 'manifest' && name === 'manifest.json') {
    return Buffer.from(bytes.toString('utf8').replace(/"security":\s*false/, '"security": true'));
  }
  if (state.tamper === 'signature' && name === 'manifest.json.minisig') {
    const lines = bytes.toString('utf8').split('\n');
    const raw = Buffer.from(lines[1], 'base64');
    raw[raw.length - 1] ^= 0xff;
    lines[1] = raw.toString('base64');
    return Buffer.from(lines.join('\n'));
  }
  if (state.tamper === 'archive' && name.endsWith('.tar.gz')) {
    const copy = Buffer.from(bytes);
    copy[copy.length - 9] ^= 0xff;
    return copy;
  }
  return bytes;
}

function readBody(req) {
  return new Promise((resolve) => {
    const chunks = [];
    req.on('data', (chunk) => chunks.push(chunk));
    req.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')));
  });
}

const server = createServer(async (req, res) => {
  const url = new URL(req.url, BASE);

  if (url.pathname === '/_control') {
    if (req.method === 'POST') {
      const patch = JSON.parse((await readBody(req)) || '{}');
      for (const key of ['visible', 'fault', 'tamper', 'slow']) if (key in patch) state[key] = patch[key];
      if (patch.clearHits) state.hits = [];
    }
    send(res, 200, state);
    return;
  }

  state.hits.push({ at: Date.now(), path: url.pathname, agent: req.headers['user-agent'] ?? null });
  if (state.hits.length > 500) state.hits.shift();

  if (url.pathname === '/releases') {
    if (fault(res)) return;
    const body = JSON.stringify(releases());
    const etag = `"${createHash('sha256').update(body).digest('hex').slice(0, 32)}"`;
    if (req.headers['if-none-match'] === etag) {
      res.writeHead(304, { etag });
      res.end();
      return;
    }
    send(res, 200, body, { etag });
    return;
  }

  const download = url.pathname.match(/^\/dl\/([^/]+)\/([^/]+)$/);
  if (download) {
    const [, tag, name] = download;
    const path = join(DATA, tag, name);
    if (!state.visible.includes(tag) || !existsSync(path)) {
      send(res, 404, { message: 'Not Found' });
      return;
    }
    if (state.tamper) {
      const bytes = tampered(name, readFileSync(path));
      res.writeHead(200, { 'content-type': 'application/octet-stream', 'content-length': bytes.length });
      res.end(bytes);
      return;
    }
    res.writeHead(200, { 'content-type': 'application/octet-stream', 'content-length': statSync(path).size });
    if (state.slow && name.endsWith('.tar.gz')) {
      const bytes = readFileSync(path);
      const step = Math.ceil(bytes.length / 100);
      for (let offset = 0; offset < bytes.length && !res.destroyed; offset += step) {
        res.write(bytes.subarray(offset, offset + step));
        await new Promise((resolve) => setTimeout(resolve, 150));
      }
      res.end();
      return;
    }
    createReadStream(path).pipe(res);
    return;
  }

  send(res, 404, { message: 'Not Found' });
});

server.listen(PORT, HOST, () => {
  console.log(`release server on ${HOST}:${PORT}, data ${DATA}`);
});
