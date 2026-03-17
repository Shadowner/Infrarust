//! Title-related packets (Clientbound, 1.17+).
//!
//! Since 1.17, titles are split into three separate packets:
//! - [`CSetTitle`] — main title text
//! - [`CSetSubtitle`] — subtitle text
//! - [`CSetTitleTimes`] — fade-in, stay, fade-out timings

use crate::codec::{McBufReadExt, McBufWriteExt};
use crate::error::ProtocolResult;
use crate::packets::Packet;
use crate::version::{ConnectionState, Direction, ProtocolVersion};

/// Sets the main title text displayed on the player's screen.
///
/// Content format varies by version:
/// - Pre-1.20.3: JSON text component (String)
/// - 1.20.3+: NBT compound (binary)
#[derive(Debug, Clone)]
pub struct CSetTitle {
    pub text: Vec<u8>,
}

impl CSetTitle {
    /// Creates a title packet from a JSON text component string.
    pub fn from_json(json: &str) -> Self {
        Self {
            text: json.as_bytes().to_vec(),
        }
    }
}

impl Packet for CSetTitle {
    const NAME: &'static str = "CSetTitle";

    fn state() -> ConnectionState {
        ConnectionState::Play
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let text = if version.less_than(ProtocolVersion::V1_20_3) {
            r.read_string()?.into_bytes()
        } else {
            r.read_remaining()?
        };
        Ok(Self { text })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        if version.less_than(ProtocolVersion::V1_20_3) {
            let json = std::str::from_utf8(&self.text).map_err(|_| {
                crate::error::ProtocolError::invalid(
                    "CSetTitle text is not valid UTF-8 for JSON version",
                )
            })?;
            w.write_string(json)?;
        } else {
            w.write_all(&self.text)?;
        }
        Ok(())
    }
}

/// Sets the subtitle text displayed below the title.
///
/// Same encoding rules as [`CSetTitle`].
#[derive(Debug, Clone)]
pub struct CSetSubtitle {
    pub text: Vec<u8>,
}

impl CSetSubtitle {
    /// Creates a subtitle packet from a JSON text component string.
    pub fn from_json(json: &str) -> Self {
        Self {
            text: json.as_bytes().to_vec(),
        }
    }
}

impl Packet for CSetSubtitle {
    const NAME: &'static str = "CSetSubtitle";

    fn state() -> ConnectionState {
        ConnectionState::Play
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let text = if version.less_than(ProtocolVersion::V1_20_3) {
            r.read_string()?.into_bytes()
        } else {
            r.read_remaining()?
        };
        Ok(Self { text })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        if version.less_than(ProtocolVersion::V1_20_3) {
            let json = std::str::from_utf8(&self.text).map_err(|_| {
                crate::error::ProtocolError::invalid(
                    "CSetSubtitle text is not valid UTF-8 for JSON version",
                )
            })?;
            w.write_string(json)?;
        } else {
            w.write_all(&self.text)?;
        }
        Ok(())
    }
}

/// Sets the title display timings (fade-in, stay, fade-out) in ticks.
#[derive(Debug, Clone)]
pub struct CSetTitleTimes {
    pub fade_in: i32,
    pub stay: i32,
    pub fade_out: i32,
}

impl Packet for CSetTitleTimes {
    const NAME: &'static str = "CSetTitleTimes";

    fn state() -> ConnectionState {
        ConnectionState::Play
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

    fn decode(r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        let fade_in = r.read_i32_be()?;
        let stay = r.read_i32_be()?;
        let fade_out = r.read_i32_be()?;
        Ok(Self {
            fade_in,
            stay,
            fade_out,
        })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_i32_be(self.fade_in)?;
        w.write_i32_be(self.stay)?;
        w.write_i32_be(self.fade_out)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn round_trip<P: Packet>(packet: &P, version: ProtocolVersion) -> P {
        let mut buf = Vec::new();
        packet.encode(&mut buf, version).unwrap();
        P::decode(&mut buf.as_slice(), version).unwrap()
    }

    #[test]
    fn test_title_round_trip_json() {
        let pkt = CSetTitle::from_json(r#"{"text":"Welcome!"}"#);
        let decoded = round_trip(&pkt, ProtocolVersion::V1_19);
        assert_eq!(
            std::str::from_utf8(&decoded.text).unwrap(),
            r#"{"text":"Welcome!"}"#
        );
    }

    #[test]
    fn test_subtitle_round_trip_json() {
        let pkt = CSetSubtitle::from_json(r#"{"text":"Enjoy your stay"}"#);
        let decoded = round_trip(&pkt, ProtocolVersion::V1_19);
        assert_eq!(
            std::str::from_utf8(&decoded.text).unwrap(),
            r#"{"text":"Enjoy your stay"}"#
        );
    }

    #[test]
    fn test_title_times_round_trip() {
        let pkt = CSetTitleTimes {
            fade_in: 10,
            stay: 70,
            fade_out: 20,
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_21);
        assert_eq!(decoded.fade_in, 10);
        assert_eq!(decoded.stay, 70);
        assert_eq!(decoded.fade_out, 20);
    }
}
