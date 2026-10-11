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
