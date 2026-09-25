#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum JavaIoError {
    #[error("the message ends before its {0}")]
    Truncated(&'static str),
    #[error("a string of {0} bytes does not fit writeUTF")]
    TooLong(usize),
    #[error("malformed modified UTF-8")]
    Malformed,
}

pub(crate) fn encode_modified_utf8(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    for unit in text.encode_utf16() {
        match unit {
            0x0001..=0x007F => out.push(unit as u8),
            0x0000 | 0x0080..=0x07FF => {
                out.push(0xC0 | ((unit >> 6) & 0x1F) as u8);
                out.push(0x80 | (unit & 0x3F) as u8);
            }
            _ => {
                out.push(0xE0 | ((unit >> 12) & 0x0F) as u8);
                out.push(0x80 | ((unit >> 6) & 0x3F) as u8);
                out.push(0x80 | (unit & 0x3F) as u8);
            }
        }
    }
    out
}

pub(crate) fn decode_modified_utf8(bytes: &[u8]) -> Result<String, JavaIoError> {
    let mut units = Vec::with_capacity(bytes.len());
    let mut rest = bytes;
    while let Some((&first, tail)) = rest.split_first() {
        let first = u16::from(first);
        let (unit, tail) = if first & 0x80 == 0 {
            (first, tail)
        } else if first & 0xE0 == 0xC0 {
            let [second, tail @ ..] = tail else {
                return Err(JavaIoError::Malformed);
            };
            if second & 0xC0 != 0x80 {
                return Err(JavaIoError::Malformed);
            }
            (((first & 0x1F) << 6) | u16::from(second & 0x3F), tail)
        } else if first & 0xF0 == 0xE0 {
            let [second, third, tail @ ..] = tail else {
                return Err(JavaIoError::Malformed);
            };
            if second & 0xC0 != 0x80 || third & 0xC0 != 0x80 {
                return Err(JavaIoError::Malformed);
            }
            (
                ((first & 0x0F) << 12) | (u16::from(second & 0x3F) << 6) | u16::from(third & 0x3F),
                tail,
            )
        } else {
            return Err(JavaIoError::Malformed);
        };
        units.push(unit);
        rest = tail;
    }
    Ok(char::decode_utf16(units)
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect())
}

pub(crate) struct JavaReader<'a> {
    data: &'a [u8],
}

impl<'a> JavaReader<'a> {
    pub(crate) const fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    fn take(&mut self, len: usize, what: &'static str) -> Result<&'a [u8], JavaIoError> {
        if self.data.len() < len {
            return Err(JavaIoError::Truncated(what));
        }
        let (head, tail) = self.data.split_at(len);
        self.data = tail;
        Ok(head)
    }

    pub(crate) fn read_u16(&mut self) -> Result<u16, JavaIoError> {
        let bytes = self.take(2, "short")?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    pub(crate) fn read_utf(&mut self) -> Result<String, JavaIoError> {
        let len = usize::from(self.read_u16()?);
        decode_modified_utf8(self.take(len, "string")?)
    }

    pub(crate) fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], JavaIoError> {
        self.take(len, "data")
    }
}

#[derive(Default)]
pub(crate) struct JavaWriter {
    buf: Vec<u8>,
}

impl JavaWriter {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn write_utf(&mut self, text: &str) -> Result<&mut Self, JavaIoError> {
        let encoded = encode_modified_utf8(text);
        let len = u16::try_from(encoded.len()).map_err(|_| JavaIoError::TooLong(encoded.len()))?;
        self.buf.extend_from_slice(&len.to_be_bytes());
        self.buf.extend_from_slice(&encoded);
        Ok(self)
    }

    pub(crate) fn write_i32(&mut self, value: i32) -> &mut Self {
        self.buf.extend_from_slice(&value.to_be_bytes());
        self
    }

    pub(crate) fn write_u16(&mut self, value: u16) -> &mut Self {
        self.buf.extend_from_slice(&value.to_be_bytes());
        self
    }

    pub(crate) fn write_bytes(&mut self, data: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(data);
        self
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.buf
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn ascii_is_one_byte_per_char() {
        assert_eq!(encode_modified_utf8("Connect"), b"Connect");
    }

    #[test]
    fn nul_is_two_bytes() {
        assert_eq!(encode_modified_utf8("a\0b"), [b'a', 0xC0, 0x80, b'b']);
        assert_eq!(
            decode_modified_utf8(&[b'a', 0xC0, 0x80, b'b']).unwrap(),
            "a\0b"
        );
    }

    #[test]
    fn two_and_three_byte_chars_match_java() {
        assert_eq!(encode_modified_utf8("é"), [0xC3, 0xA9]);
        assert_eq!(encode_modified_utf8("€"), [0xE2, 0x82, 0xAC]);
        assert_eq!(decode_modified_utf8(&[0xE2, 0x82, 0xAC]).unwrap(), "€");
    }

    #[test]
    fn supplementary_chars_are_surrogate_pairs() {
        let encoded = encode_modified_utf8("😀");
        assert_eq!(encoded, [0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80]);
        assert_ne!(encoded, "😀".as_bytes());
        assert_eq!(decode_modified_utf8(&encoded).unwrap(), "😀");
    }

    #[test]
    fn a_lone_surrogate_decodes_to_the_replacement_char() {
        assert_eq!(
            decode_modified_utf8(&[0xED, 0xA0, 0xBD]).unwrap(),
            "\u{FFFD}"
        );
    }

    #[test]
    fn malformed_input_is_refused() {
        assert_eq!(decode_modified_utf8(&[0xC3]), Err(JavaIoError::Malformed));
        assert_eq!(
            decode_modified_utf8(&[0xE2, 0x82]),
            Err(JavaIoError::Malformed)
        );
        assert_eq!(
            decode_modified_utf8(&[0xF0, 0x9F]),
            Err(JavaIoError::Malformed)
        );
        assert_eq!(
            decode_modified_utf8(&[0xC3, 0x29]),
            Err(JavaIoError::Malformed)
        );
    }

    #[test]
    fn write_utf_prefixes_the_encoded_length() {
        let mut out = JavaWriter::new();
        out.write_utf("a😀").unwrap().write_i32(-2).write_u16(25565);
        assert_eq!(
            out.into_bytes(),
            [
                0x00, 0x07, b'a', 0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80, 0xFF, 0xFF, 0xFF, 0xFE, 0x63,
                0xDD
            ]
        );
    }

    #[test]
    fn write_utf_refuses_strings_over_65535_bytes() {
        let long = "é".repeat(40_000);
        assert_eq!(
            JavaWriter::new().write_utf(&long).err(),
            Some(JavaIoError::TooLong(80_000))
        );
    }

    #[test]
    fn the_reader_walks_fields_and_keeps_the_rest() {
        let mut out = JavaWriter::new();
        out.write_utf("Forward").unwrap().write_utf("ALL").unwrap();
        out.write_bytes(&[1, 2, 3]);
        let bytes = out.into_bytes();
        let mut input = JavaReader::new(&bytes);
        assert_eq!(input.read_utf().unwrap(), "Forward");
        assert_eq!(input.read_utf().unwrap(), "ALL");
        assert_eq!(input.read_bytes(3).unwrap(), [1, 2, 3]);
        assert_eq!(input.read_utf(), Err(JavaIoError::Truncated("short")));
    }
}
