use uuid::Uuid;

use crate::codec::{McBufReadExt, McBufWriteExt, VarInt};
use crate::packets::play::common::{read_nested_text_component, write_text_component};
use crate::version::ProtocolVersion;

const PROMPT_ADDED: ProtocolVersion = ProtocolVersion::V1_17;
const HASH_REMOVED: ProtocolVersion = ProtocolVersion(210);
const PACK_ID_ADDED: ProtocolVersion = ProtocolVersion::V1_20_3;

pub const MAX_RESOURCE_PACK_URL: usize = 32767;

pub const MAX_RESOURCE_PACK_HASH: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourcePackResult {
    SuccessfullyLoaded,
    Declined,
    FailedDownload,
    Accepted,
    Downloaded,
    InvalidUrl,
    FailedReload,
    Discarded,
    Unknown(i32),
}

impl ResourcePackResult {
    pub const fn from_id(id: i32) -> Self {
        match id {
            0 => Self::SuccessfullyLoaded,
            1 => Self::Declined,
            2 => Self::FailedDownload,
            3 => Self::Accepted,
            4 => Self::Downloaded,
            5 => Self::InvalidUrl,
            6 => Self::FailedReload,
            7 => Self::Discarded,
            other => Self::Unknown(other),
        }
    }

    pub const fn id(self) -> i32 {
        match self {
            Self::SuccessfullyLoaded => 0,
            Self::Declined => 1,
            Self::FailedDownload => 2,
            Self::Accepted => 3,
            Self::Downloaded => 4,
            Self::InvalidUrl => 5,
            Self::FailedReload => 6,
            Self::Discarded => 7,
            Self::Unknown(id) => id,
        }
    }

    pub const fn is_final(self) -> bool {
        !matches!(self, Self::Accepted | Self::Downloaded | Self::Unknown(_))
    }
}

fn read_prompt(r: &mut &[u8], version: ProtocolVersion) -> crate::ProtocolResult<Option<Vec<u8>>> {
    if r.read_bool()? {
        Ok(Some(read_nested_text_component(r, version)?))
    } else {
        Ok(None)
    }
}

fn write_prompt(
    mut w: &mut (impl std::io::Write + ?Sized),
    prompt: Option<&[u8]>,
    version: ProtocolVersion,
    packet_name: &str,
) -> crate::ProtocolResult<()> {
    w.write_bool(prompt.is_some())?;
    if let Some(prompt) = prompt {
        write_text_component(&mut w, prompt, version, packet_name, "prompt")?;
    }
    Ok(())
}

define_twin_packets! {
    states: {
        #[derive(PartialEq, Eq)]
        CResourcePack: Play / Clientbound = ids![
            V1_8                => 0x48,
            V1_9                => 0x32,
            V1_12               => 0x33,
            V1_12_1             => 0x34,
            V1_13               => 0x37,
            V1_14               => 0x39,
            V1_15               => 0x3A,
            V1_16               => 0x39,
            V1_16_2             => 0x38,
            V1_17               => 0x3C,
            V1_19               => 0x3A,
            V1_19_1             => 0x3D,
            V1_19_3             => 0x3C,
            V1_19_4             => 0x40,
            V1_20_2 ..= V1_20_2 => 0x42,
        ],
        #[derive(PartialEq, Eq)]
        CConfigResourcePack: Config / Clientbound = ids![
            V1_20_2 ..= V1_20_2 => 0x06,
        ],
    },
    encode_only: true,
    fields: {
        pub url: String,
        pub hash: String,
        pub forced: bool,
        pub prompt: Option<Vec<u8>>,
    },
    shared_impl: {},
    decode(r, version): {
        let url = r.read_string_bounded(MAX_RESOURCE_PACK_URL)?;
        let hash = r.read_string_bounded(MAX_RESOURCE_PACK_HASH)?;
        let (forced, prompt) = if version.no_less_than(PROMPT_ADDED) {
            (r.read_bool()?, read_prompt(r, version)?)
        } else {
            (false, None)
        };
        Ok(Self { url, hash, forced, prompt })
    },
    encode(self, w, version): {
        w.write_string(&self.url)?;
        w.write_string(&self.hash)?;
        if version.no_less_than(PROMPT_ADDED) {
            w.write_bool(self.forced)?;
            write_prompt(w, self.prompt.as_deref(), version, Self::NAME)?;
        }
        Ok(())
    },
}

define_twin_packets! {
    states: {
        #[derive(PartialEq, Eq)]
        CResourcePackPush: Play / Clientbound = ids![
            V1_20_3 => 0x44,
            V1_20_5 => 0x46,
            V1_21_2 => 0x4B,
            V1_21_5 => 0x4A,
            V1_21_9 => 0x4F,
            V26_1   => 0x51,
        ],
        #[derive(PartialEq, Eq)]
        CConfigResourcePackPush: Config / Clientbound = ids![
            V1_20_3 => 0x07,
            V1_20_5 => 0x09,
        ],
    },
    encode_only: true,
    fields: {
        pub id: Uuid,
        pub url: String,
        pub hash: String,
        pub forced: bool,
        pub prompt: Option<Vec<u8>>,
    },
    shared_impl: {},
    decode(r, version): {
        let id = r.read_uuid()?;
        let url = r.read_string_bounded(MAX_RESOURCE_PACK_URL)?;
        let hash = r.read_string_bounded(MAX_RESOURCE_PACK_HASH)?;
        let forced = r.read_bool()?;
        let prompt = read_prompt(r, version)?;
        Ok(Self { id, url, hash, forced, prompt })
    },
    encode(self, w, version): {
        w.write_uuid(&self.id)?;
        w.write_string(&self.url)?;
        w.write_string(&self.hash)?;
        w.write_bool(self.forced)?;
        write_prompt(w, self.prompt.as_deref(), version, Self::NAME)
    },
}

define_twin_packets! {
    states: {
        #[derive(PartialEq, Eq)]
        CResourcePackPop: Play / Clientbound = ids![
            V1_20_3 => 0x43,
            V1_20_5 => 0x45,
            V1_21_2 => 0x4A,
            V1_21_5 => 0x49,
            V1_21_9 => 0x4E,
            V26_1   => 0x50,
        ],
        #[derive(PartialEq, Eq)]
        CConfigResourcePackPop: Config / Clientbound = ids![
            V1_20_3 => 0x06,
            V1_20_5 => 0x08,
        ],
    },
    encode_only: true,
    fields: {
        pub id: Option<Uuid>,
    },
    shared_impl: {},
    decode(r, _version): {
        let id = if r.read_bool()? { Some(r.read_uuid()?) } else { None };
        Ok(Self { id })
    },
    encode(self, w, _version): {
        w.write_bool(self.id.is_some())?;
        if let Some(id) = &self.id {
            w.write_uuid(id)?;
        }
        Ok(())
    },
}

define_twin_packets! {
    states: {
        #[derive(PartialEq, Eq)]
        SResourcePackResponse: Play / Serverbound = ids![
            V1_8    => 0x19,
            V1_9    => 0x16,
            V1_12   => 0x18,
            V1_13   => 0x1D,
            V1_14   => 0x1F,
            V1_16   => 0x20,
            V1_16_2 => 0x21,
            V1_19   => 0x23,
            V1_19_1 => 0x24,
            V1_20_2 => 0x27,
            V1_20_3 => 0x28,
            V1_20_5 => 0x2B,
            V1_21_2 => 0x2D,
            V1_21_4 => 0x2F,
            V1_21_6 => 0x30,
            V26_1   => 0x31,
        ],
        #[derive(PartialEq, Eq)]
        SConfigResourcePackResponse: Config / Serverbound = ids![
            V1_20_2 => 0x05,
            V1_20_5 => 0x06,
        ],
    },
    encode_only: false,
    fields: {
        pub id: Option<Uuid>,
        pub hash: Option<String>,
        pub result: ResourcePackResult,
    },
    shared_impl: {},
    decode(r, version): {
        let id = if version.no_less_than(PACK_ID_ADDED) {
            Some(r.read_uuid()?)
        } else {
            None
        };
        let hash = if version.less_than(HASH_REMOVED) {
            Some(r.read_string()?)
        } else {
            None
        };
        let result = ResourcePackResult::from_id(r.read_var_int()?.0);
        Ok(Self { id, hash, result })
    },
    encode(self, w, version): {
        if version.no_less_than(PACK_ID_ADDED) {
            w.write_uuid(&self.id.unwrap_or_default())?;
        }
        if version.less_than(HASH_REMOVED) {
            w.write_string(self.hash.as_deref().unwrap_or_default())?;
        }
        w.write_var_int(&VarInt(self.result.id()))?;
        Ok(())
    },
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::packets::{Packet, round_trip};

    const HASH: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn result_ids_round_trip() {
        for id in 0..8 {
            assert_eq!(ResourcePackResult::from_id(id).id(), id);
        }
        assert_eq!(
            ResourcePackResult::from_id(42),
            ResourcePackResult::Unknown(42)
        );
        assert!(!ResourcePackResult::Accepted.is_final());
        assert!(!ResourcePackResult::Downloaded.is_final());
        assert!(ResourcePackResult::Declined.is_final());
        assert!(ResourcePackResult::SuccessfullyLoaded.is_final());
    }

    #[test]
    fn single_pack_drops_the_prompt_before_1_17() {
        let packet = CResourcePack {
            url: "https://example.com/pack.zip".into(),
            hash: HASH.into(),
            forced: true,
            prompt: Some(br#"{"text":"please"}"#.to_vec()),
        };
        let old = round_trip(&packet, ProtocolVersion::V1_16_4);
        assert_eq!(old.url, packet.url);
        assert_eq!(old.hash, packet.hash);
        assert!(!old.forced);
        assert_eq!(old.prompt, None);
        assert_eq!(round_trip(&packet, ProtocolVersion::V1_17), packet);
        assert_eq!(round_trip(&packet, ProtocolVersion::V1_20_2), packet);
    }

    #[test]
    fn push_keeps_an_nbt_prompt() {
        let packet = CResourcePackPush {
            id: Uuid::from_u128(9),
            url: "https://example.com/pack.zip".into(),
            hash: String::new(),
            forced: false,
            prompt: Some(vec![0x08, 0x00, 0x02, b'o', b'k']),
        };
        assert_eq!(round_trip(&packet, ProtocolVersion::V1_21), packet);
        let config = CConfigResourcePackPush {
            id: packet.id,
            url: packet.url.clone(),
            hash: packet.hash.clone(),
            forced: true,
            prompt: None,
        };
        assert_eq!(round_trip(&config, ProtocolVersion::V1_21), config);
    }

    #[test]
    fn pop_round_trips_with_and_without_an_id() {
        for id in [None, Some(Uuid::from_u128(3))] {
            let packet = CResourcePackPop { id };
            assert_eq!(round_trip(&packet, ProtocolVersion::V1_20_3), packet);
            let config = CConfigResourcePackPop { id };
            assert_eq!(round_trip(&config, ProtocolVersion::V1_20_5), config);
        }
    }

    #[test]
    fn response_fields_follow_the_version() {
        let packet = SResourcePackResponse {
            id: None,
            hash: Some(HASH.into()),
            result: ResourcePackResult::Accepted,
        };
        assert_eq!(round_trip(&packet, ProtocolVersion::V1_9_4), packet);

        let mut buf = Vec::new();
        packet.encode(&mut buf, ProtocolVersion::V1_12_2).unwrap();
        assert_eq!(buf, [3]);

        let modern = SResourcePackResponse {
            id: Some(Uuid::from_u128(5)),
            hash: None,
            result: ResourcePackResult::Discarded,
        };
        assert_eq!(round_trip(&modern, ProtocolVersion::V1_20_3), modern);
        let config = SConfigResourcePackResponse {
            id: None,
            hash: None,
            result: ResourcePackResult::Declined,
        };
        assert_eq!(round_trip(&config, ProtocolVersion::V1_20_2), config);
    }
}
