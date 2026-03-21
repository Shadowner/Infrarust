//! Title-related packets (Clientbound).
//!
//! Since 1.17, titles are split into three separate packets:
//! - [`CSetTitle`] — main title text
//! - [`CSetSubtitle`] — subtitle text
//! - [`CSetTitleTimes`] — fade-in, stay, fade-out timings
//!
//! Before 1.17, all title operations used a single [`CTitleLegacy`] packet
//! with an action discriminator.

use crate::codec::{McBufReadExt, McBufWriteExt, VarInt};
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

    /// Creates a title packet from pre-encoded NBT bytes (1.20.3+).
    pub fn from_nbt(nbt: Vec<u8>) -> Self {
        Self { text: nbt }
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

    /// Creates a subtitle packet from pre-encoded NBT bytes (1.20.3+).
    pub fn from_nbt(nbt: Vec<u8>) -> Self {
        Self { text: nbt }
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

#[derive(Debug, Clone)]
pub enum CTitleLegacy {
    SetTitle(String),
    SetSubtitle(String),
    SetTimes {
        fade_in: i32,
        stay: i32,
        fade_out: i32,
    },
}

impl CTitleLegacy {
    fn action_id(&self, version: ProtocolVersion) -> i32 {
        match self {
            Self::SetTitle(_) => 0,
            Self::SetSubtitle(_) => 1,
            Self::SetTimes { .. } => {
                // Pre-1.11 (< V1_12): action 2 = times
                // 1.11+ (>= V1_12): action 3 = times
                if version.less_than(ProtocolVersion::V1_12) {
                    2
                } else {
                    3
                }
            }
        }
    }
}

impl Packet for CTitleLegacy {
    const NAME: &'static str = "CTitleLegacy";

    fn state() -> ConnectionState {
        ConnectionState::Play
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let action = r.read_var_int()?.0;
        let times_action = if version.less_than(ProtocolVersion::V1_12) { 2 } else { 3 };

        if action == 0 {
            Ok(Self::SetTitle(r.read_string()?))
        } else if action == 1 {
            Ok(Self::SetSubtitle(r.read_string()?))
        } else if action == times_action {
            let fade_in = r.read_i32_be()?;
            let stay = r.read_i32_be()?;
            let fade_out = r.read_i32_be()?;
            Ok(Self::SetTimes { fade_in, stay, fade_out })
        } else {
            Err(crate::error::ProtocolError::invalid(
                format!("CTitleLegacy: unknown action {action}"),
            ))
        }
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_var_int(&VarInt(self.action_id(version)))?;
        match self {
            Self::SetTitle(json) | Self::SetSubtitle(json) => {
                w.write_string(json)?;
            }
            Self::SetTimes { fade_in, stay, fade_out } => {
                w.write_i32_be(*fade_in)?;
                w.write_i32_be(*stay)?;
                w.write_i32_be(*fade_out)?;
            }
        }
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

    #[test]
    fn test_legacy_title_set_title() {
        let pkt = CTitleLegacy::SetTitle(r#"{"text":"Welcome!"}"#.to_string());
        let decoded = round_trip(&pkt, ProtocolVersion::V1_12);
        match decoded {
            CTitleLegacy::SetTitle(text) => assert_eq!(text, r#"{"text":"Welcome!"}"#),
            _ => panic!("expected SetTitle"),
        }
    }

    #[test]
    fn test_legacy_title_set_subtitle() {
        let pkt = CTitleLegacy::SetSubtitle(r#"{"text":"Sub"}"#.to_string());
        let decoded = round_trip(&pkt, ProtocolVersion::V1_14);
        match decoded {
            CTitleLegacy::SetSubtitle(text) => assert_eq!(text, r#"{"text":"Sub"}"#),
            _ => panic!("expected SetSubtitle"),
        }
    }

    #[test]
    fn test_legacy_title_set_times_post_1_11() {
        let pkt = CTitleLegacy::SetTimes { fade_in: 10, stay: 70, fade_out: 20 };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_12);
        match decoded {
            CTitleLegacy::SetTimes { fade_in, stay, fade_out } => {
                assert_eq!(fade_in, 10);
                assert_eq!(stay, 70);
                assert_eq!(fade_out, 20);
            }
            _ => panic!("expected SetTimes"),
        }
    }

    #[test]
    fn test_legacy_title_set_times_pre_1_11() {
        let pkt = CTitleLegacy::SetTimes { fade_in: 5, stay: 40, fade_out: 10 };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_9);
        match decoded {
            CTitleLegacy::SetTimes { fade_in, stay, fade_out } => {
                assert_eq!(fade_in, 5);
                assert_eq!(stay, 40);
                assert_eq!(fade_out, 10);
            }
            _ => panic!("expected SetTimes"),
        }
    }
}
