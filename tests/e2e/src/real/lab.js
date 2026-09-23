import { mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { ADMIN_PORT_BASE, BACKENDS, PROXY_PORT_BASE, SCENARIOS } from './scenarios.js';

export const API_KEY = 'infrarust-e2e-api-key-0123456789';
export const FORWARDING_SECRET = 'e2eSecretForVelocityForwarding01';

function quote(s) {
  return JSON.stringify(String(s));
}

export function proxiedScenarios() {
  const out = [];
  let index = 0;
  for (const scenario of SCENARIOS) {
    if (!scenario.proxy) continue;
    out.push({
      ...scenario,
      port: PROXY_PORT_BASE + index,
      adminPort: ADMIN_PORT_BASE + index,
    });
    index++;
  }
  return out;
}

export function targetFor(scenario) {
  if (scenario.proxy) return { host: '127.0.0.1', port: scenario.port };
  return { host: '127.0.0.1', port: BACKENDS[scenario.backend].port };
}

export function writeLab(dir, { sessionUrl } = {}) {
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });

  const configs = [];
  for (const scenario of proxiedScenarios()) {
    const home = join(dir, scenario.id);
    mkdirSync(join(home, 'servers'), { recursive: true });
    mkdirSync(join(home, 'plugins'), { recursive: true });

    const forwards = scenario.proxy.forwardingMode !== 'none';
    if (forwards) {
      writeFileSync(join(home, 'forwarding.secret'), scenario.proxy.secret ?? FORWARDING_SECRET, { mode: 0o600 });
    }

    const lines = [
      `bind = ${quote(`127.0.0.1:${scenario.port}`)}`,
      `servers_dir = ${quote(join(home, 'servers'))}`,
      `plugins_dir = ${quote(join(home, 'plugins'))}`,
      'connect_timeout = "5s"',
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
    ];

    if (forwards) {
      lines.push(
        '[forwarding]',
        `mode = ${quote(scenario.proxy.forwardingMode)}`,
        `secret_file = ${quote(join(home, 'forwarding.secret'))}`,
        '',
      );
    }

    if (sessionUrl) lines.push('[auth]', `session_url = ${quote(sessionUrl)}`, '');

    lines.push(
      '[web]',
      'enable_api = true',
      'enable_webui = false',
      `bind = ${quote(`127.0.0.1:${scenario.adminPort}`)}`,
      `api_key = ${quote(API_KEY)}`,
      '',
      '[web.rate_limit]',
      'requests_per_minute = 1000000',
      '',
    );

    writeFileSync(join(home, 'infrarust.toml'), lines.join('\n'));

    const backend = BACKENDS[scenario.backend];
    writeFileSync(
      join(home, 'servers', `${scenario.id}.toml`),
      [
        `name = ${quote(scenario.id)}`,
        'domains = ["*"]',
        `addresses = [${quote(`127.0.0.1:${backend.port}`)}]`,
        `proxy_mode = ${quote(scenario.proxy.proxyMode)}`,
        `forwarding_mode = ${quote(scenario.proxy.forwardingMode)}`,
        '',
      ].join('\n'),
    );

    configs.push({ scenario, configPath: join(home, 'infrarust.toml') });
  }
  return configs;
}
