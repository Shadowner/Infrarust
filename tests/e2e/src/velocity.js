import { createHmac, timingSafeEqual } from 'node:crypto';

export const VELOCITY_CHANNEL = 'velocity:player_info';

export const MAX_FORWARDING_VERSION = 4;

class Reader {
  constructor(buf) {
    this.buf = buf;
    this.off = 0;
  }

  varint() {
    let value = 0;
    let shift = 0;
    for (;;) {
      if (this.off >= this.buf.length) throw new Error('varint runs past end of buffer');
      const byte = this.buf[this.off++];
      value |= (byte & 0x7f) << shift;
      if ((byte & 0x80) === 0) break;
      shift += 7;
      if (shift > 35) throw new Error('varint is too long');
    }
    return value | 0;
  }

  string() {
    const len = this.varint();
    if (len < 0 || this.off + len > this.buf.length) throw new Error('string length is out of range');
    const s = this.buf.toString('utf8', this.off, this.off + len);
    this.off += len;
    return s;
  }

  bool() {
    if (this.off >= this.buf.length) throw new Error('bool runs past end of buffer');
    return this.buf[this.off++] !== 0;
  }

  uuid() {
    if (this.off + 16 > this.buf.length) throw new Error('uuid runs past end of buffer');
    const hex = this.buf.toString('hex', this.off, this.off + 16);
    this.off += 16;
    return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
  }

  get remaining() {
    return this.buf.length - this.off;
  }
}

export function writeVarInt(value) {
  const bytes = [];
  let v = value >>> 0;
  for (;;) {
    if ((v & ~0x7f) === 0) {
      bytes.push(v);
      break;
    }
    bytes.push((v & 0x7f) | 0x80);
    v >>>= 7;
  }
  return Buffer.from(bytes);
}

export function verifyAndDecode(data, secret) {
  const buf = Buffer.isBuffer(data) ? data : Buffer.from(data);
  if (buf.length < 33) {
    return { ok: false, error: `payload too short (${buf.length} bytes, need > 32)` };
  }

  const signature = buf.subarray(0, 32);
  const payload = buf.subarray(32);
  const expected = createHmac('sha256', secret).update(payload).digest();

  if (!timingSafeEqual(signature, expected)) {
    return {
      ok: false,
      error: `HMAC mismatch: got ${signature.toString('hex')}, expected ${expected.toString('hex')}`,
    };
  }

  const r = new Reader(payload);
  const decoded = { version: r.varint(), ip: r.string(), uuid: r.uuid(), username: r.string() };

  const propertyCount = r.varint();
  if (propertyCount < 0 || propertyCount > 64) {
    return { ok: false, error: `implausible property count ${propertyCount}` };
  }
  decoded.properties = [];
  for (let i = 0; i < propertyCount; i++) {
    const name = r.string();
    const value = r.string();
    const signed = r.bool();
    decoded.properties.push({ name, value, signature: signed ? r.string() : null });
  }

  decoded.trailingBytes = r.remaining;

  return { ok: true, decoded };
}
