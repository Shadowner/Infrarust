import { createHash } from 'node:crypto';

import { writeVarInt } from './velocity.js';

export const LOGIN_QUERY_MIN_PROTOCOL = 393;

export const LOGIN_QUERIES = [
  {
    messageId: 7,
    channel: 'e2e:mod_list',
    request: Buffer.from('which mods?'),
    answer: Buffer.from('e2e-mod@1.0.0;core@2.1'),
    answerDelayMs: 40,
  },
  {
    messageId: -101,
    channel: 'automodpack:handshake',
    request: Buffer.from([0xc3, 0x28, 0xff, 0xfe, 0x00, 0x80]),
    answer: Buffer.from([0xa0, 0xa1, 0xed, 0xa0, 0x80, 0x00, 0xff]),
  },
  {
    messageId: 2147483647,
    channel: 'e2e:declined',
    request: Buffer.alloc(0),
    answer: null,
  },
];

export const LOGIN_QUERY_TIMEOUT_MS = 5000;

export function forgeMarkerFor(protocol) {
  if (protocol >= 764) return '\0FORGE';
  if (protocol >= 757) return '\0FML3\0';
  if (protocol >= 393) return '\0FML2\0';
  return '\0FML\0';
}

export const BIG_PACKET_MIN_PROTOCOL = 47;
export const BIG_PACKET_CHANNEL = 'e2e:big';
export const BIG_PACKET_SIZE = 3 * 1024 * 1024;

let bigPacket = null;
let bigPacketHash = null;

export function bigPacketData() {
  if (!bigPacket) {
    bigPacket = Buffer.alloc(BIG_PACKET_SIZE);
    for (let i = 0; i < BIG_PACKET_SIZE; i++) bigPacket[i] = (i * 31 + (i >>> 13)) & 0xff;
  }
  return bigPacket;
}

export function bigPacketDigest() {
  bigPacketHash ??= sha256(bigPacketData());
  return bigPacketHash;
}

export function sha256(buf) {
  return createHash('sha256').update(buf).digest('hex');
}

export const HOLEPUNCH_PROTOCOLS = { min: 759, max: 763 };
export const RAW_TAIL = Buffer.from('\x16\x03\x01\x00\x05hello', 'latin1');
export const RAW_REPLY = Buffer.from('e2e-raw-backend-reply');

function frame(id, payload) {
  const body = Buffer.concat([writeVarInt(id), payload]);
  return Buffer.concat([writeVarInt(body.length), body]);
}

function string(s) {
  const b = Buffer.from(s, 'utf8');
  return Buffer.concat([writeVarInt(b.length), b]);
}

function offlineUuid(username) {
  const md5 = createHash('md5').update(`OfflinePlayer:${username}`).digest();
  md5[6] = (md5[6] & 0x0f) | 0x30;
  md5[8] = (md5[8] & 0x3f) | 0x80;
  return md5;
}

export function modernLoginStartOpening({ protocol, host, port, username }) {
  const portBytes = Buffer.alloc(2);
  portBytes.writeUInt16BE(port);
  const handshake = frame(0x00, Buffer.concat([writeVarInt(protocol), string(host), portBytes, writeVarInt(2)]));
  const loginStart = frame(0x00, Buffer.concat([string(username), offlineUuid(username)]));
  return Buffer.concat([handshake, loginStart, RAW_TAIL]);
}
