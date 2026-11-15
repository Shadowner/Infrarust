use std::borrow::Cow;

use serde_json::{Map, Number, Value};

use super::MAX_NBT_DEPTH;
use super::encode::Ir;

const MAX_NBT_NODES: usize = 262_144;
const MAX_NBT_STRING_BYTES: usize = 65_535;

pub(super) fn write_payload(ir: &Ir, out: &mut Vec<u8>) {
    match ir {
        Ir::Bool(b) => out.push(u8::from(*b)),
        Ir::Int(i) => out.extend_from_slice(&i.to_be_bytes()),
        Ir::Long(l) => out.extend_from_slice(&l.to_be_bytes()),
        Ir::Double(d) => out.extend_from_slice(&d.to_be_bytes()),
        Ir::Str(s) => write_mutf8(s, out),
        Ir::IntArray(ints) => {
            write_len(ints.len(), out);
            for i in ints {
                out.extend_from_slice(&i.to_be_bytes());
            }
        }
        Ir::List(items) => write_list(items, out),
        Ir::Compound(fields) => write_compound(fields, out),
    }
}

fn write_len(len: usize, out: &mut Vec<u8>) {
    out.extend_from_slice(&i32::try_from(len).unwrap_or(i32::MAX).to_be_bytes());
}

fn write_compound(fields: &[(Cow<'static, str>, Ir)], out: &mut Vec<u8>) {
    for (key, value) in fields {
        out.push(value.tag_id());
        write_mutf8(key, out);
        write_payload(value, out);
    }
    out.push(0);
}

fn is_wrapper_compound(ir: &Ir) -> bool {
    matches!(ir, Ir::Compound(fields) if fields.len() == 1 && fields[0].0.is_empty())
}

fn write_list(items: &[Ir], out: &mut Vec<u8>) {
    let Some(first) = items.first() else {
        out.push(0);
        write_len(0, out);
        return;
    };
    let element = first.tag_id();
    let homogeneous = items.iter().all(|item| item.tag_id() == element);
    if homogeneous && !(element == 10 && items.iter().any(is_wrapper_compound)) {
        out.push(element);
        write_len(items.len(), out);
        for item in items {
            write_payload(item, out);
        }
        return;
    }
    out.push(10);
    write_len(items.len(), out);
    for item in items {
        match item {
            Ir::Compound(fields) if !is_wrapper_compound(item) => write_compound(fields, out),
            other => {
                out.push(other.tag_id());
                write_mutf8("", out);
                write_payload(other, out);
                out.push(0);
            }
        }
    }
}

fn mutf8_unit_len(unit: u16) -> usize {
    match unit {
        0x0001..=0x007F => 1,
        0x0000 | 0x0080..=0x07FF => 2,
        _ => 3,
    }
}

fn write_mutf8(value: &str, out: &mut Vec<u8>) {
    let start = out.len();
    out.extend_from_slice(&[0, 0]);
    let mut written = 0usize;
    let mut units = [0u16; 2];
    for ch in value.chars() {
        let encoded = ch.encode_utf16(&mut units);
        let needed: usize = encoded.iter().map(|u| mutf8_unit_len(*u)).sum();
        if written + needed > MAX_NBT_STRING_BYTES {
            break;
        }
        for unit in encoded.iter().copied() {
            match mutf8_unit_len(unit) {
                1 => out.push(unit.to_be_bytes()[1]),
                2 => {
                    out.push(0xC0 | ((unit >> 6) & 0x1F).to_be_bytes()[1]);
                    out.push(0x80 | (unit & 0x3F).to_be_bytes()[1]);
                }
                _ => {
                    out.push(0xE0 | ((unit >> 12) & 0x0F).to_be_bytes()[1]);
                    out.push(0x80 | ((unit >> 6) & 0x3F).to_be_bytes()[1]);
                    out.push(0x80 | (unit & 0x3F).to_be_bytes()[1]);
                }
            }
        }
        written += needed;
    }
    let len = u16::try_from(written).unwrap_or(u16::MAX).to_be_bytes();
    out[start] = len[0];
    out[start + 1] = len[1];
}

fn decode_mutf8(bytes: &[u8]) -> String {
    let continuation = |index: usize| bytes.get(index).is_some_and(|b| b & 0xC0 == 0x80);
    let low = |index: usize| u16::from(bytes.get(index).copied().unwrap_or(0) & 0x3F);
    let mut units: Vec<u16> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&lead) = bytes.get(i) {
        if lead < 0x80 {
            units.push(u16::from(lead));
            i += 1;
        } else if lead & 0xE0 == 0xC0 && continuation(i + 1) {
            units.push((u16::from(lead & 0x1F) << 6) | low(i + 1));
            i += 2;
        } else if lead & 0xF0 == 0xE0 && continuation(i + 1) && continuation(i + 2) {
            units.push((u16::from(lead & 0x0F) << 12) | (low(i + 1) << 6) | low(i + 2));
            i += 3;
        } else if lead & 0xF8 == 0xF0
            && continuation(i + 1)
            && continuation(i + 2)
            && continuation(i + 3)
        {
            let code = (u32::from(lead & 0x07) << 18)
                | (u32::from(low(i + 1)) << 12)
                | (u32::from(low(i + 2)) << 6)
                | u32::from(low(i + 3));
            match char::from_u32(code) {
                Some(ch) => {
                    let mut buf = [0u16; 2];
                    units.extend_from_slice(ch.encode_utf16(&mut buf));
                }
                None => units.push(0xFFFD),
            }
            i += 4;
        } else {
            units.push(0xFFFD);
            i += 1;
        }
    }
    String::from_utf16_lossy(&units)
}

pub(super) struct NbtReader<'a> {
    pub(super) buf: &'a [u8],
    pub(super) pos: usize,
    pub(super) nodes: usize,
}

impl NbtReader<'_> {
    fn take(&mut self, len: usize) -> Result<&[u8], String> {
        let end = self
            .pos
            .checked_add(len)
            .filter(|end| *end <= self.buf.len())
            .ok_or_else(|| "unexpected end of input".to_owned())?;
        let slice = self
            .buf
            .get(self.pos..end)
            .ok_or_else(|| "unexpected end of input".to_owned())?;
        self.pos = end;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let slice = self.take(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(slice);
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.array::<1>()?[0])
    }

    fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_be_bytes(self.array()?))
    }

    fn len(&mut self) -> Result<usize, String> {
        let len = self.i32()?;
        usize::try_from(len).map_err(|_| format!("negative length {len}"))
    }

    fn string(&mut self) -> Result<String, String> {
        let len = usize::from(u16::from_be_bytes(self.array()?));
        let bytes = self.take(len)?;
        Ok(decode_mutf8(bytes))
    }

    fn account(&mut self, nodes: usize) -> Result<(), String> {
        self.nodes = self.nodes.saturating_add(nodes);
        if self.nodes > MAX_NBT_NODES {
            return Err("NBT has too many elements".to_owned());
        }
        Ok(())
    }

    fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    pub(super) fn root(&mut self) -> Result<Value, String> {
        let tag = self.u8()?;
        match tag {
            8..=10 => self.payload(tag, 0),
            other => Err(format!("unsupported root tag {other}")),
        }
    }

    fn payload(&mut self, tag: u8, depth: usize) -> Result<Value, String> {
        if depth > MAX_NBT_DEPTH {
            return Err("NBT nested too deeply".to_owned());
        }
        self.account(1)?;
        Ok(match tag {
            1 => Value::from(i8::from_be_bytes(self.array()?)),
            2 => Value::from(i16::from_be_bytes(self.array()?)),
            3 => Value::from(self.i32()?),
            4 => Value::from(i64::from_be_bytes(self.array()?)),
            5 => float_value(f64::from(f32::from_be_bytes(self.array()?))),
            6 => float_value(f64::from_be_bytes(self.array()?)),
            7 => {
                let len = self.len()?;
                self.account(len)?;
                let bytes = self.take(len)?;
                Value::Array(
                    bytes
                        .iter()
                        .map(|b| Value::from(i8::from_ne_bytes([*b])))
                        .collect(),
                )
            }
            8 => Value::String(self.string()?),
            9 => {
                let element = self.u8()?;
                let len = self.len()?;
                if element == 0 && len > 0 {
                    return Err("non-empty list of TAG_End".to_owned());
                }
                let mut items = Vec::with_capacity(len.min(self.remaining()));
                for _ in 0..len {
                    let item = self.payload(element, depth + 1)?;
                    items.push(if element == 10 {
                        unwrap_list_wrapper(item)
                    } else {
                        item
                    });
                }
                Value::Array(items)
            }
            10 => {
                let mut map = Map::new();
                loop {
                    let field = self.u8()?;
                    if field == 0 {
                        break;
                    }
                    let name = self.string()?;
                    let value = self.payload(field, depth + 1)?;
                    map.insert(name, value);
                }
                Value::Object(map)
            }
            11 => {
                let len = self.len()?;
                self.account(len)?;
                let bytes = self.take(len.checked_mul(4).ok_or("array too long")?)?;
                Value::Array(
                    bytes
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|c| Value::from(i32::from_be_bytes(*c)))
                        .collect(),
                )
            }
            12 => {
                let len = self.len()?;
                self.account(len)?;
                let bytes = self.take(len.checked_mul(8).ok_or("array too long")?)?;
                Value::Array(
                    bytes
                        .as_chunks::<8>()
                        .0
                        .iter()
                        .map(|c| Value::from(i64::from_be_bytes(*c)))
                        .collect(),
                )
            }
            other => return Err(format!("unknown tag type {other}")),
        })
    }
}

fn float_value(value: f64) -> Value {
    Number::from_f64(value).map_or(Value::Null, Value::Number)
}

fn unwrap_list_wrapper(value: Value) -> Value {
    match value {
        Value::Object(mut map) if map.len() == 1 && map.contains_key("") => {
            map.remove("").unwrap_or(Value::Null)
        }
        other => other,
    }
}
