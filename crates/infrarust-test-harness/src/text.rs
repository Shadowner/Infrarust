use infrarust_api::types::Component;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};
use serde_json::{Map, Number, Value};

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
        return match read_network_nbt(raw) {
            Ok(root) => (root_text(&root), Some(nbt_to_json(&root))),
            Err(_) => (String::from_utf8_lossy(raw).into_owned(), None),
        };
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
    Ok(component.to_nbt_for(infrarust_api::types::ProtocolVersion::new(version.0)))
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
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
    String(String),
    List(Vec<Nbt>),
    Compound(Vec<(String, Nbt)>),
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
    Ok(root_text(&read_network_nbt(bytes)?))
}

pub fn nbt_component_json(bytes: &[u8]) -> HarnessResult<Value> {
    Ok(nbt_to_json(&read_network_nbt(bytes)?))
}

pub fn component_json(raw: &[u8], version: ProtocolVersion) -> HarnessResult<Value> {
    if uses_nbt_components(version) {
        return nbt_component_json(raw);
    }
    serde_json::from_slice(raw)
        .map_err(|e| HarnessError::Unexpected(format!("component JSON: {e}")))
}

pub fn nbt_to_json(node: &Nbt) -> Value {
    match node {
        Nbt::Byte(v) => Value::from(*v),
        Nbt::Short(v) => Value::from(*v),
        Nbt::Int(v) => Value::from(*v),
        Nbt::Long(v) => Value::from(*v),
        Nbt::Float(v) => Number::from_f64(f64::from(*v)).map_or(Value::Null, Value::Number),
        Nbt::Double(v) => Number::from_f64(*v).map_or(Value::Null, Value::Number),
        Nbt::ByteArray(items) => items.iter().copied().map(Value::from).collect(),
        Nbt::IntArray(items) => items.iter().copied().map(Value::from).collect(),
        Nbt::LongArray(items) => items.iter().copied().map(Value::from).collect(),
        Nbt::String(s) => Value::String(s.clone()),
        Nbt::List(items) => items.iter().map(nbt_to_json).collect(),
        Nbt::Compound(fields) => Value::Object(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), nbt_to_json(v)))
                .collect::<Map<String, Value>>(),
        ),
    }
}

fn root_text(root: &Nbt) -> String {
    let mut out = String::new();
    push_nbt_text(root, &mut out);
    out
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
        Nbt::Byte(_)
        | Nbt::Short(_)
        | Nbt::Int(_)
        | Nbt::Long(_)
        | Nbt::Float(_)
        | Nbt::Double(_)
        | Nbt::ByteArray(_)
        | Nbt::IntArray(_)
        | Nbt::LongArray(_) => {}
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

    fn array<const N: usize>(&mut self) -> HarnessResult<[u8; N]> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn len(&mut self) -> HarnessResult<usize> {
        let n = i32::from_be_bytes(self.array()?);
        usize::try_from(n).map_err(|_| malformed("negative NBT length"))
    }

    fn string(&mut self) -> HarnessResult<String> {
        let len = usize::from(self.u16()?);
        Ok(String::from_utf8_lossy(self.take(len)?).into_owned())
    }

    fn elements<T, const N: usize>(
        &mut self,
        decode: impl Fn([u8; N]) -> T,
    ) -> HarnessResult<Vec<T>> {
        let n = self.len()?;
        if n.checked_mul(N).is_none_or(|bytes| bytes > self.buf.len()) {
            return Err(malformed("NBT array longer than its payload"));
        }
        (0..n).map(|_| self.array().map(&decode)).collect()
    }

    fn payload(&mut self, tag: u8, depth: usize) -> HarnessResult<Nbt> {
        if depth > MAX_NBT_DEPTH {
            return Err(malformed("NBT nested too deeply"));
        }
        match tag {
            1 => Ok(Nbt::Byte(i8::from_be_bytes(self.array()?))),
            2 => Ok(Nbt::Short(i16::from_be_bytes(self.array()?))),
            3 => Ok(Nbt::Int(i32::from_be_bytes(self.array()?))),
            4 => Ok(Nbt::Long(i64::from_be_bytes(self.array()?))),
            5 => Ok(Nbt::Float(f32::from_be_bytes(self.array()?))),
            6 => Ok(Nbt::Double(f64::from_be_bytes(self.array()?))),
            7 => self.elements(i8::from_be_bytes).map(Nbt::ByteArray),
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
            11 => self.elements(i32::from_be_bytes).map(Nbt::IntArray),
            12 => self.elements(i64::from_be_bytes).map(Nbt::LongArray),
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
        let bytes = component.to_nbt_for(infrarust_api::types::ProtocolVersion::new(774));
        assert_eq!(nbt_text(&bytes).unwrap(), "Kicked: bye");
    }

    #[test]
    fn nbt_to_json_keeps_every_tag() {
        let mut bytes = vec![0x0A];
        bytes.extend_from_slice(&[0x08, 0x00, 0x04]);
        bytes.extend_from_slice(b"text");
        bytes.extend_from_slice(&[0x00, 0x02]);
        bytes.extend_from_slice(b"hi");
        bytes.extend_from_slice(&[0x01, 0x00, 0x04]);
        bytes.extend_from_slice(b"bold");
        bytes.push(0x01);
        bytes.extend_from_slice(&[0x03, 0x00, 0x01, b'i']);
        bytes.extend_from_slice(&(-7i32).to_be_bytes());
        bytes.extend_from_slice(&[0x0B, 0x00, 0x01, b'a']);
        bytes.extend_from_slice(&2i32.to_be_bytes());
        bytes.extend_from_slice(&1i32.to_be_bytes());
        bytes.extend_from_slice(&(-1i32).to_be_bytes());
        bytes.extend_from_slice(&[0x09, 0x00, 0x05]);
        bytes.extend_from_slice(b"extra");
        bytes.push(0x08);
        bytes.extend_from_slice(&1i32.to_be_bytes());
        bytes.extend_from_slice(&[0x00, 0x01, b'!']);
        bytes.push(0x00);
        assert_eq!(
            nbt_component_json(&bytes).unwrap(),
            serde_json::json!({"text": "hi", "bold": 1, "i": -7, "a": [1, -1], "extra": ["!"]})
        );
        assert_eq!(nbt_text(&bytes).unwrap(), "hi!");
    }

    #[test]
    fn nbt_arrays_cannot_claim_more_than_the_payload() {
        let mut bytes = vec![0x0B];
        bytes.extend_from_slice(&i32::MAX.to_be_bytes());
        assert!(read_network_nbt(&bytes).is_err());
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
