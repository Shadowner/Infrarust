use infrarust_api::types::Component;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};
use serde_json::Value;

use crate::error::{HarnessError, HarnessResult};

const MAX_NBT_DEPTH: usize = 512;

pub fn uses_nbt_components(version: ProtocolVersion) -> bool {
    version.no_less_than(ProtocolVersion::V1_20_3)
}

#[derive(Debug, Clone, PartialEq)]
pub struct DisconnectInfo {
    pub state: ConnectionState,
    pub raw: Vec<u8>,
    pub text: String,
    pub json: Option<Value>,
}

impl DisconnectInfo {
    pub fn from_json_string(state: ConnectionState, reason: &str) -> Self {
        let (text, json) = json_component_text(reason);
        Self {
            state,
            raw: reason.as_bytes().to_vec(),
            text,
            json,
        }
    }

    pub fn from_component(state: ConnectionState, raw: Vec<u8>, version: ProtocolVersion) -> Self {
        let (text, json) = decode_component(&raw, version);
        Self {
            state,
            raw,
            text,
            json,
        }
    }
}

pub fn component_text(raw: &[u8], version: ProtocolVersion) -> String {
    decode_component(raw, version).0
}

pub fn decode_component(raw: &[u8], version: ProtocolVersion) -> (String, Option<Value>) {
    if uses_nbt_components(version) {
        let text = nbt_text(raw).unwrap_or_else(|_| String::from_utf8_lossy(raw).into_owned());
        return (text, None);
    }
    json_component_text(&String::from_utf8_lossy(raw))
}

pub fn json_component_text(json: &str) -> (String, Option<Value>) {
    match serde_json::from_str::<Value>(json) {
        Ok(value) => (json_text(&value), Some(value)),
        Err(_) => (json.to_string(), None),
    }
}

pub fn encode_component_json(json: &str, version: ProtocolVersion) -> HarnessResult<Vec<u8>> {
    if !uses_nbt_components(version) {
        return Ok(json.as_bytes().to_vec());
    }
    let component =
        Component::from_json(json).map_err(|e| HarnessError::Unexpected(e.to_string()))?;
    Ok(component.to_nbt_network())
}

pub fn json_text(value: &Value) -> String {
    let mut out = String::new();
    push_json_text(value, &mut out);
    out
}

fn push_json_text(value: &Value, out: &mut String) {
    match value {
        Value::String(s) => out.push_str(s),
        Value::Array(items) => items.iter().for_each(|item| push_json_text(item, out)),
        Value::Object(map) => {
            if let Some(text) = map.get("text") {
                push_json_text(text, out);
            }
            if let Some(Value::Array(extra)) = map.get("extra") {
                extra.iter().for_each(|item| push_json_text(item, out));
            }
        }
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Null => {}
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Nbt {
    String(String),
    List(Vec<Nbt>),
    Compound(Vec<(String, Nbt)>),
    Scalar,
}

impl Nbt {
    pub fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Compound(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

pub fn read_network_nbt(bytes: &[u8]) -> HarnessResult<Nbt> {
    let mut reader = NbtReader { buf: bytes };
    let tag = reader.u8()?;
    reader.payload(tag, 0)
}

pub fn nbt_text(bytes: &[u8]) -> HarnessResult<String> {
    let root = read_network_nbt(bytes)?;
    let mut out = String::new();
    push_nbt_text(&root, &mut out);
    Ok(out)
}

fn push_nbt_text(node: &Nbt, out: &mut String) {
    match node {
        Nbt::String(s) => out.push_str(s),
        Nbt::List(items) => items.iter().for_each(|item| push_nbt_text(item, out)),
        Nbt::Compound(_) => {
            if let Some(text) = node.get("text").or_else(|| node.get("")) {
                push_nbt_text(text, out);
            }
            if let Some(Nbt::List(extra)) = node.get("extra") {
                extra.iter().for_each(|item| push_nbt_text(item, out));
            }
        }
        Nbt::Scalar => {}
    }
}

struct NbtReader<'a> {
    buf: &'a [u8],
}

impl<'a> NbtReader<'a> {
    fn take(&mut self, n: usize) -> HarnessResult<&'a [u8]> {
        if self.buf.len() < n {
            return Err(malformed("unexpected end of NBT"));
        }
        let (head, tail) = self.buf.split_at(n);
        self.buf = tail;
        Ok(head)
    }

    fn u8(&mut self) -> HarnessResult<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> HarnessResult<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn len(&mut self) -> HarnessResult<usize> {
        let b = self.take(4)?;
        let n = i32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        Ok(usize::try_from(n.max(0)).unwrap_or(0))
    }

    fn string(&mut self) -> HarnessResult<String> {
        let len = usize::from(self.u16()?);
        Ok(String::from_utf8_lossy(self.take(len)?).into_owned())
    }

    fn skip_array(&mut self, element_size: usize) -> HarnessResult<Nbt> {
        let n = self.len()?;
        let bytes = n
            .checked_mul(element_size)
            .ok_or_else(|| malformed("NBT array length overflow"))?;
        self.take(bytes)?;
        Ok(Nbt::Scalar)
    }

    fn payload(&mut self, tag: u8, depth: usize) -> HarnessResult<Nbt> {
        if depth > MAX_NBT_DEPTH {
            return Err(malformed("NBT nested too deeply"));
        }
        match tag {
            1 => self.take(1).map(|_| Nbt::Scalar),
            2 => self.take(2).map(|_| Nbt::Scalar),
            3 | 5 => self.take(4).map(|_| Nbt::Scalar),
            4 | 6 => self.take(8).map(|_| Nbt::Scalar),
            7 => self.skip_array(1),
            8 => self.string().map(Nbt::String),
            9 => {
                let element = self.u8()?;
                let n = self.len()?;
                let mut items = Vec::with_capacity(n.min(64));
                for _ in 0..n {
                    items.push(self.payload(element, depth + 1)?);
                }
                Ok(Nbt::List(items))
            }
            10 => {
                let mut fields = Vec::new();
                loop {
                    let field_tag = self.u8()?;
                    if field_tag == 0 {
                        return Ok(Nbt::Compound(fields));
                    }
                    let name = self.string()?;
                    let value = self.payload(field_tag, depth + 1)?;
                    fields.push((name, value));
                }
            }
            11 => self.skip_array(4),
            12 => self.skip_array(8),
            other => Err(malformed(&format!("unknown NBT tag {other}"))),
        }
    }
}

fn malformed(context: &str) -> HarnessError {
    HarnessError::Unexpected(format!("malformed NBT: {context}"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn json_text_concatenates_text_and_extra() {
        let value: Value = serde_json::from_str(
            r#"{"text":"Hello ","extra":[{"text":"big ","extra":["wide"]}," world"]}"#,
        )
        .unwrap();
        assert_eq!(json_text(&value), "Hello big wide world");
    }

    #[test]
    fn nbt_text_reads_component_nbt_from_api() {
        let component = Component::text("Kicked: ")
            .color("red")
            .append(Component::text("bye").bold());
        assert_eq!(
            nbt_text(&component.to_nbt_network()).unwrap(),
            "Kicked: bye"
        );
    }

    #[test]
    fn nbt_text_reads_bare_string_root() {
        let mut bytes = vec![0x08, 0x00, 0x05];
        bytes.extend_from_slice(b"hello");
        assert_eq!(nbt_text(&bytes).unwrap(), "hello");
    }

    #[test]
    fn nbt_text_rejects_truncated_input() {
        assert!(nbt_text(&[0x0A, 0x08, 0x00]).is_err());
    }

    #[test]
    fn encode_component_json_round_trips_per_version() {
        let json = r#"{"text":"Server closed"}"#;
        for version in [ProtocolVersion::V1_8, ProtocolVersion::V1_20_3] {
            let raw = encode_component_json(json, version).unwrap();
            assert_eq!(component_text(&raw, version), "Server closed");
        }
    }
}
