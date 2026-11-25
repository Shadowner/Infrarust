#!/usr/bin/env node

import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const minecraftData = require('minecraft-data');

const SNAPSHOT = new URL(
  '../../../crates/infrarust_protocol/src/registry/testdata/registry_snapshot.txt',
  import.meta.url,
);

const PACKETS = {
  CStoreCookie: 'store_cookie',
  CConfigStoreCookie: 'store_cookie',
  CTransfer: 'transfer',
  CConfigTransfer: 'transfer',
  CCookieRequest: 'cookie_request',
  CConfigCookieRequest: 'cookie_request',
  CLoginCookieRequest: 'cookie_request',
  SCookieResponse: 'cookie_response',
  SConfigCookieResponse: 'cookie_response',
  SLoginCookieResponse: 'cookie_response',
  CEncryptionRequest: 'encryption_begin',
  SEncryptionResponse: 'encryption_begin',
};

const STATES = { config: 'configuration', login: 'login', play: 'play' };
const DIRECTIONS = { clientbound: 'toClient', serverbound: 'toServer' };
const FIRST_PROTOCOL = 766;

function reference(protocol) {
  const versions = minecraftData.postNettyVersionsByProtocolVersion.pc[protocol] ?? [];
  const usable = versions.find((version) => minecraftData(version.minecraftVersion)?.protocol);
  return usable?.minecraftVersion ?? null;
}

function idsOf(minecraftVersion, state, direction) {
  const packet = minecraftData(minecraftVersion).protocol[state]?.[direction]?.types?.packet;
  const mappings = packet?.[1]?.[0]?.type?.[1]?.mappings ?? {};
  return Object.fromEntries(Object.entries(mappings).map(([id, name]) => [name, Number(id)]));
}

const lines = readFileSync(SNAPSHOT, 'utf8').split('\n');
const checked = [];
const mismatches = [];
const unknown = new Set();

for (const line of lines) {
  const [kind, state, direction, protocolText, name, idText] = line.split(' ');
  if (kind !== 'ENC' || !(name in PACKETS)) continue;
  const protocol = Number(protocolText);
  if (protocol < FIRST_PROTOCOL) continue;
  const minecraftVersion = reference(protocol);
  if (!minecraftVersion) {
    unknown.add(protocol);
    continue;
  }
  const expected = idsOf(minecraftVersion, STATES[state], DIRECTIONS[direction])[PACKETS[name]];
  const actual = Number(idText);
  checked.push({ protocol, minecraftVersion, state, name, actual });
  if (expected !== actual) {
    mismatches.push(
      `${name} (${state}, ${direction}) on ${minecraftVersion} (${protocol}): the registry says 0x${actual.toString(16)}, minecraft-data says ${
        expected === undefined ? 'no such packet' : `0x${expected.toString(16)}`
      }`,
    );
  }
}

const protocols = [...new Set(checked.map((entry) => entry.protocol))].sort((a, b) => a - b);
console.log(`minecraft-data ${require('minecraft-data/package.json').version}`);
console.log(`${checked.length} packet ids checked on protocols ${protocols.join(', ')}`);
if (unknown.size) {
  console.log(`not in minecraft-data, skipped: protocols ${[...unknown].sort((a, b) => a - b).join(', ')}`);
}
if (checked.length === 0) {
  console.error('nothing was checked: the snapshot or the packet table is out of date');
  process.exit(1);
}
for (const mismatch of mismatches) console.error(`MISMATCH ${mismatch}`);
process.exit(mismatches.length ? 1 : 0);
