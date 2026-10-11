import { createHash } from 'node:crypto';

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
