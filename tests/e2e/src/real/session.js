import { createHash } from 'node:crypto';
import { existsSync } from 'node:fs';
import { join } from 'node:path';

import { download } from './mojang.js';

const AUTHLIB_INJECTOR_VERSION = '1.2.8';
const AUTHLIB_INJECTOR_URL =
  `https://github.com/yushijinhun/authlib-injector/releases/download/v${AUTHLIB_INJECTOR_VERSION}/authlib-injector-${AUTHLIB_INJECTOR_VERSION}.jar`;

export async function fetchAuthlibInjector(paths) {
  const jar = join(paths.root, 'tools', `authlib-injector-${AUTHLIB_INJECTOR_VERSION}.jar`);
  if (existsSync(jar)) return jar;
  await download(AUTHLIB_INJECTOR_URL, jar);
  return jar;
}

const DEAD_API = 'http://127.0.0.1:1';
const MOJANG_HOSTS_OFF = [
  `-Dminecraft.api.auth.host=${DEAD_API}`,
  `-Dminecraft.api.account.host=${DEAD_API}`,
  `-Dminecraft.api.session.host=${DEAD_API}`,
  `-Dminecraft.api.services.host=${DEAD_API}`,
];

export function offlineSession(username) {
  return {
    username,
    uuid: offlineUuid(username),
    accessToken: '0',
    userType: 'msa',
    jvmExtra: MOJANG_HOSTS_OFF,
  };
}

export function offlineUuid(username) {
  const md5 = createHash('md5').update(`OfflinePlayer:${username}`).digest();
  md5[6] = (md5[6] & 0x0f) | 0x30;
  md5[8] = (md5[8] & 0x3f) | 0x80;
  return md5.toString('hex');
}

export async function onlineSession(yggdrasil, username, { injectorJar }) {
  const session = await yggdrasil.session(username);
  return {
    ...session,
    userType: 'msa',
    javaagent: `-javaagent:${injectorJar}=${yggdrasil.apiRoot}`,
    jvmExtra: [
      '-Dauthlibinjector.noShowServerName',
    ],
  };
}
