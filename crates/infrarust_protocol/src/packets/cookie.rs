use crate::codec::{McBufReadExt, McBufWriteExt};

pub const MAX_COOKIE_PAYLOAD: usize = 5120;

define_twin_packets! {
    states: {
        #[derive(PartialEq, Eq)]
        CStoreCookie: Play / Clientbound = ids![
            V1_20_5 => 0x6B,
            V1_21_2 => 0x72,
            V1_21_5 => 0x71,
            V1_21_9 => 0x76,
            V26_1   => 0x78,
        ],
        #[derive(PartialEq, Eq)]
        CConfigStoreCookie: Config / Clientbound = ids![
            V1_20_5 => 0x0A,
        ],
    },
    encode_only: true,
    fields: {
        pub key: String,
        pub payload: Vec<u8>,
    },
    shared_impl: {},
    decode(r, _version): {
        let key = r.read_string()?;
        let payload = r.read_byte_array(MAX_COOKIE_PAYLOAD)?;
        Ok(Self { key, payload })
    },
    encode(self, w, _version): {
        w.write_string(&self.key)?;
        w.write_byte_array(&self.payload)?;
        Ok(())
    },
}

define_twin_packets! {
    states: {
        #[derive(PartialEq, Eq)]
        CCookieRequest: Play / Clientbound = ids![
            V1_20_5 => 0x16,
            V1_21_5 => 0x15,
        ],
        #[derive(PartialEq, Eq)]
        CConfigCookieRequest: Config / Clientbound = ids![
            V1_20_5 => 0x00,
        ],
        #[derive(PartialEq, Eq)]
        CLoginCookieRequest: Login / Clientbound = ids![
            V1_20_5 => 0x05,
        ],
    },
    encode_only: true,
    fields: {
        pub key: String,
    },
    shared_impl: {},
    decode(r, _version): {
        Ok(Self { key: r.read_string()? })
    },
    encode(self, w, _version): {
        w.write_string(&self.key)?;
        Ok(())
    },
}

define_twin_packets! {
    states: {
        #[derive(PartialEq, Eq)]
        SCookieResponse: Play / Serverbound = ids![
            V1_20_5 => 0x11,
            V1_21_2 => 0x13,
            V1_21_6 => 0x14,
            V26_1   => 0x15,
        ],
        #[derive(PartialEq, Eq)]
        SConfigCookieResponse: Config / Serverbound = ids![
            V1_20_5 => 0x01,
        ],
        #[derive(PartialEq, Eq)]
        SLoginCookieResponse: Login / Serverbound = ids![
            V1_20_5 => 0x04,
        ],
    },
    encode_only: false,
    fields: {
        pub key: String,
        pub payload: Option<Vec<u8>>,
    },
    shared_impl: {},
    decode(r, _version): {
        let key = r.read_string()?;
        let payload = if r.read_bool()? {
            Some(r.read_byte_array(MAX_COOKIE_PAYLOAD)?)
        } else {
            None
        };
        Ok(Self { key, payload })
    },
    encode(self, w, _version): {
        w.write_string(&self.key)?;
        w.write_bool(self.payload.is_some())?;
        if let Some(payload) = &self.payload {
            w.write_byte_array(payload)?;
        }
        Ok(())
    },
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::packets::{Packet, round_trip};
    use crate::version::ProtocolVersion;

    const V: ProtocolVersion = ProtocolVersion::V1_20_5;

    #[test]
    fn store_cookie_round_trips_in_both_states() {
        let play = CStoreCookie {
            key: "infrarust:session".into(),
            payload: vec![1, 2, 3],
        };
        assert_eq!(round_trip(&play, V), play);
        let config = CConfigStoreCookie {
            key: play.key.clone(),
            payload: play.payload.clone(),
        };
        assert_eq!(round_trip(&config, V), config);
    }

    #[test]
    fn requests_round_trip_in_every_state() {
        let key = "minecraft:token".to_string();
        assert_eq!(round_trip(&CCookieRequest { key: key.clone() }, V).key, key);
        assert_eq!(
            round_trip(&CConfigCookieRequest { key: key.clone() }, V).key,
            key
        );
        assert_eq!(
            round_trip(&CLoginCookieRequest { key: key.clone() }, V).key,
            key
        );
    }

    #[test]
    fn responses_carry_an_optional_payload() {
        for payload in [None, Some(vec![]), Some(vec![7; MAX_COOKIE_PAYLOAD])] {
            let play = SCookieResponse {
                key: "a:b".into(),
                payload: payload.clone(),
            };
            assert_eq!(round_trip(&play, V), play);
            let config = SConfigCookieResponse {
                key: "a:b".into(),
                payload: payload.clone(),
            };
            assert_eq!(round_trip(&config, V), config);
            let login = SLoginCookieResponse {
                key: "a:b".into(),
                payload,
            };
            assert_eq!(round_trip(&login, V), login);
        }
    }

    #[test]
    fn oversized_payloads_are_rejected() {
        let mut buf = Vec::new();
        SCookieResponse {
            key: "a:b".into(),
            payload: Some(vec![0; MAX_COOKIE_PAYLOAD + 1]),
        }
        .encode(&mut buf, V)
        .unwrap();
        assert!(SCookieResponse::decode(&mut buf.as_slice(), V).is_err());
    }
}
