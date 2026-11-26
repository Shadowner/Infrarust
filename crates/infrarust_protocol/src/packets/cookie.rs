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
            V26_3   => 0x7A,
        ],
        #[derive(PartialEq, Eq)]
        CConfigStoreCookie: Config / Clientbound = ids![
            V1_20_5 => 0x0A,
            V26_3   => 0x0B,
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
    use crate::packets::{Packet, round_trip, wire};
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
        let oversized = SCookieResponse {
            key: "a:b".into(),
            payload: Some(vec![0; MAX_COOKIE_PAYLOAD + 1]),
        };
        let buf = wire(&oversized, V);
        assert!(SCookieResponse::decode(&mut buf.as_slice(), V).is_err());
    }

    #[test]
    fn store_cookie_is_a_key_then_a_length_prefixed_payload() {
        let expected = [&[16][..], b"infrarust:resume", &[4, 0xde, 0xad, 0xbe, 0xef]].concat();
        let key = "infrarust:resume".to_string();
        let payload = vec![0xde, 0xad, 0xbe, 0xef];
        assert_eq!(
            wire(
                &CStoreCookie {
                    key: key.clone(),
                    payload: payload.clone()
                },
                V
            ),
            expected
        );
        assert_eq!(wire(&CConfigStoreCookie { key, payload }, V), expected);
    }

    #[test]
    fn a_payload_longer_than_one_length_byte_gets_a_two_byte_varint() {
        let bytes = wire(
            &CStoreCookie {
                key: "a:b".into(),
                payload: vec![9; 300],
            },
            V,
        );
        assert_eq!(&bytes[..6], &[3, b'a', b':', b'b', 0xac, 0x02]);
        assert_eq!(bytes.len(), 6 + 300);
    }

    #[test]
    fn a_cookie_request_is_only_its_key() {
        let expected = [&[16][..], b"infrarust:resume"].concat();
        let key = "infrarust:resume".to_string();
        assert_eq!(wire(&CLoginCookieRequest { key: key.clone() }, V), expected);
        assert_eq!(
            wire(&CConfigCookieRequest { key: key.clone() }, V),
            expected
        );
        assert_eq!(wire(&CCookieRequest { key }, V), expected);
    }

    #[test]
    fn a_cookie_response_flags_its_payload() {
        let with = [&[3][..], b"a:b", &[1, 2, 0xca, 0xfe]].concat();
        let decoded = SLoginCookieResponse::decode(&mut with.as_slice(), V).unwrap();
        assert_eq!(decoded.key, "a:b");
        assert_eq!(decoded.payload, Some(vec![0xca, 0xfe]));
        assert_eq!(wire(&decoded, V), with);

        let without = [&[3][..], b"a:b", &[0]].concat();
        let decoded = SLoginCookieResponse::decode(&mut without.as_slice(), V).unwrap();
        assert_eq!(decoded.payload, None);
        assert_eq!(wire(&decoded, V), without);
    }

    #[test]
    fn a_truncated_cookie_response_is_rejected() {
        let truncated = [&[3][..], b"a:b", &[1, 4, 0xca]].concat();
        assert!(SLoginCookieResponse::decode(&mut truncated.as_slice(), V).is_err());
        let no_flag = [&[3][..], b"a:b"].concat();
        assert!(SLoginCookieResponse::decode(&mut no_flag.as_slice(), V).is_err());
    }
}
