import { mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { BACKENDS, SCENARIOS } from './scenarios.js';

export const PROXY_BIND = '127.0.0.1:25565';
export const WEB_BIND = '127.0.0.1:25599';
export const API_KEY = 'infrarust-e2e-api-key-0123456789';

export const FORWARDING_SECRET = 'e2eSecretForVelocityForwarding01';

function quote(s) {
  return JSON.stringify(String(s));
}

export function writeLab(dir, { sessionUrl } = {}) {
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(join(dir, 'servers'), { recursive: true });
  mkdirSync(join(dir, 'plugins'), { recursive: true });

  writeFileSync(join(dir, 'forwarding.secret'), FORWARDING_SECRET, { mode: 0o600 });

  const proxy = [
    `bind = ${quote(PROXY_BIND)}`,
    `servers_dir = ${quote(join(dir, 'servers'))}`,
    `plugins_dir = ${quote(join(dir, 'plugins'))}`,
    'connect_timeout = "3s"',
    'connect_max_attempts = 1',
    '',
    '[rate_limit]',
    'enabled = false',
    '',
    '[active_health]',
    'enabled = true',
    'unhealthy_interval = "1s"',
    'timeout = "1s"',
    '',
    '[forwarding]',
    'mode = "velocity"',
    `secret_file = ${quote(join(dir, 'forwarding.secret'))}`,
    '',
  ];

  if (sessionUrl) {
    proxy.push('[auth]', `session_url = ${quote(sessionUrl)}`, '');
  }

  proxy.push(
    '[web]',
    'enable_api = true',
    'enable_webui = false',
    `bind = ${quote(WEB_BIND)}`,
    `api_key = ${quote(API_KEY)}`,
    '',
    '[web.rate_limit]',
    'requests_per_minute = 1000000',
    '',
  );

  writeFileSync(join(dir, 'infrarust.toml'), proxy.join('\n'));

  for (const scenario of SCENARIOS) {
    const backend = BACKENDS[scenario.backend];
    const server = [
      `name = ${quote(scenario.id)}`,
      `domains = [${quote(scenario.domain)}]`,
      `addresses = [${quote(`127.0.0.1:${backend.port}`)}]`,
      `proxy_mode = ${quote(scenario.proxyMode)}`,
      `forwarding_mode = ${quote(scenario.forwardingMode)}`,
      '',
    ].join('\n');
    writeFileSync(join(dir, 'servers', `${scenario.id}.toml`), server);
  }

  return { configPath: join(dir, 'infrarust.toml') };
}
