use std::ffi::OsString;
use std::os::fd::RawFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::hot_restart::HandoverError;
use crate::hot_restart::state::{self, HandoverState};

pub const MAGIC: [u8; 8] = *b"IRLNXENV";
pub const VERSION: u32 = 1;
const PREFIX_LEN: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvelopeHeader {
    pub handles: Vec<RawFd>,
    pub image: Option<RawFd>,
    #[serde(serialize_with = "path_bytes", deserialize_with = "bytes_path")]
    pub exe: PathBuf,
    pub version: String,
}

#[derive(Debug)]
pub struct Envelope {
    pub header: EnvelopeHeader,
    pub state: HandoverState,
    pub tails: Vec<u8>,
}

pub fn encode(
    header: &EnvelopeHeader,
    state: &HandoverState,
    tails: &[u8],
) -> Result<Vec<u8>, HandoverError> {
    check(header, state)?;
    let json = serde_json::to_vec(header).map_err(|e| malformed(e.to_string()))?;
    let json_len = u32::try_from(json.len())
        .map_err(|_| malformed("the envelope header exceeds 4 GiB".into()))?;
    let body = state::encode(state, tails)?;
    let mut out = Vec::with_capacity(PREFIX_LEN + json.len() + body.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&json_len.to_le_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(&body);
    Ok(out)
}

pub fn decode(bytes: &[u8]) -> Result<Envelope, HandoverError> {
    let prefix = bytes
        .get(..PREFIX_LEN)
        .ok_or_else(|| malformed("truncated envelope".into()))?;
    if prefix[..8] != MAGIC {
        return Err(malformed("bad envelope magic".into()));
    }
    let version = u32::from_le_bytes([prefix[8], prefix[9], prefix[10], prefix[11]]);
    if version != VERSION {
        return Err(malformed(format!(
            "envelope version {version} is not supported, expected {VERSION}"
        )));
    }
    let json_len = u32::from_le_bytes([prefix[12], prefix[13], prefix[14], prefix[15]]) as usize;
    let rest = &bytes[PREFIX_LEN..];
    let json = rest
        .get(..json_len)
        .ok_or_else(|| malformed("truncated envelope header".into()))?;
    let header: EnvelopeHeader =
        serde_json::from_slice(json).map_err(|e| malformed(e.to_string()))?;
    let (state, tails) = state::decode(&rest[json_len..])?;
    check(&header, &state)?;
    Ok(Envelope {
        header,
        state,
        tails,
    })
}

fn check(header: &EnvelopeHeader, state: &HandoverState) -> Result<(), HandoverError> {
    let count = header.handles.len();
    let mut seen = vec![false; count];
    let referenced = state.listener.iter().map(|listener| listener.handle).chain(
        state
            .sessions
            .iter()
            .flat_map(|session| [session.client.handle, session.backend.handle]),
    );
    let mut total = 0;
    for handle in referenced {
        let slot = seen.get_mut(handle.0 as usize).ok_or_else(|| {
            malformed(format!(
                "handle {} is outside the {count} handles of the envelope",
                handle.0
            ))
        })?;
        if std::mem::replace(slot, true) {
            return Err(malformed(format!(
                "handle {} is referenced twice",
                handle.0
            )));
        }
        total += 1;
    }
    if total != count {
        return Err(malformed(format!(
            "the envelope carries {count} handles but the state references {total}"
        )));
    }
    let mut fds: Vec<RawFd> = header.handles.iter().copied().chain(header.image).collect();
    if let Some(fd) = fds.iter().find(|fd| **fd < 0) {
        return Err(malformed(format!("fd {fd} is not a valid descriptor")));
    }
    fds.sort_unstable();
    if let Some(pair) = fds.windows(2).find(|pair| pair[0] == pair[1]) {
        return Err(malformed(format!("fd {} is listed twice", pair[0])));
    }
    Ok(())
}

const fn malformed(message: String) -> HandoverError {
    HandoverError::Malformed(message)
}

fn path_bytes<S: Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(path.as_os_str().as_bytes())
}

fn bytes_path<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PathBuf, D::Error> {
    Vec::<u8>::deserialize(deserializer).map(|bytes| PathBuf::from(OsString::from_vec(bytes)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::ffi::OsStr;

    use super::*;
    use crate::hot_restart::platform::linux::tests::{Peers, frozen_package};

    fn header(handles: Vec<RawFd>) -> EnvelopeHeader {
        EnvelopeHeader {
            handles,
            image: Some(40),
            exe: PathBuf::from(OsStr::from_bytes(b"/opt/infra\xffrust/infrarust")),
            version: "2.0.0-beta.6".into(),
        }
    }

    fn sample() -> (HandoverState, Vec<u8>, Peers) {
        let (package, peers) = frozen_package(2);
        (package.state, package.tails, peers)
    }

    #[test]
    fn an_envelope_round_trips() {
        let (state, tails, _peers) = sample();
        let header = header(vec![10, 11, 12, 13, 14]);

        let bytes = encode(&header, &state, &tails).unwrap();
        let envelope = decode(&bytes).unwrap();

        assert_eq!(&bytes[..8], b"IRLNXENV");
        assert_eq!(envelope.header, header);
        assert_eq!(envelope.state, state);
        assert_eq!(envelope.tails, tails);
    }

    #[test]
    fn an_envelope_without_handles_or_image_round_trips() {
        let (package, _peers) = frozen_package(0);
        let mut state = package.state;
        state.listener = None;
        let header = EnvelopeHeader {
            image: None,
            ..header(Vec::new())
        };

        let envelope = decode(&encode(&header, &state, &[]).unwrap()).unwrap();

        assert_eq!(envelope.header, header);
        assert!(envelope.tails.is_empty());
    }

    #[test]
    fn a_bad_magic_is_rejected() {
        let (state, tails, _peers) = sample();
        let mut bytes = encode(&header(vec![1, 2, 3, 4, 5]), &state, &tails).unwrap();
        bytes[3] ^= 0xff;
        assert!(
            matches!(decode(&bytes), Err(HandoverError::Malformed(reason)) if reason.contains("magic"))
        );
    }

    #[test]
    fn an_unknown_version_is_rejected() {
        let (state, tails, _peers) = sample();
        let mut bytes = encode(&header(vec![1, 2, 3, 4, 5]), &state, &tails).unwrap();
        bytes[8..12].copy_from_slice(&2u32.to_le_bytes());
        assert!(
            matches!(decode(&bytes), Err(HandoverError::Malformed(reason)) if reason.contains("version 2"))
        );
    }

    #[test]
    fn a_truncated_envelope_is_rejected() {
        let (state, tails, _peers) = sample();
        let bytes = encode(&header(vec![1, 2, 3, 4, 5]), &state, &tails).unwrap();
        for cut in [0, 7, 15, 16, 40, bytes.len() - 1] {
            assert!(
                matches!(decode(&bytes[..cut]), Err(HandoverError::Malformed(_))),
                "a cut at {cut} was accepted"
            );
        }
    }

    #[test]
    fn a_handle_count_that_does_not_match_the_state_is_rejected() {
        let (state, tails, _peers) = sample();
        for handles in [vec![1, 2, 3, 4], vec![1, 2, 3, 4, 5, 6], Vec::new()] {
            assert!(matches!(
                encode(&header(handles.clone()), &state, &tails),
                Err(HandoverError::Malformed(_))
            ));
            let mut forged = encode(&header(vec![1, 2, 3, 4, 5]), &state, &tails).unwrap();
            let json = serde_json::to_vec(&header(handles)).unwrap();
            let original_len =
                u32::from_le_bytes([forged[12], forged[13], forged[14], forged[15]]) as usize;
            forged.splice(PREFIX_LEN..PREFIX_LEN + original_len, json.iter().copied());
            forged[12..16].copy_from_slice(&u32::try_from(json.len()).unwrap().to_le_bytes());
            assert!(
                matches!(decode(&forged), Err(HandoverError::Malformed(reason)) if reason.contains("handles")),
            );
        }
    }

    #[test]
    fn a_descriptor_listed_twice_or_negative_is_rejected() {
        let (state, tails, _peers) = sample();
        assert!(matches!(
            encode(&header(vec![1, 2, 3, 4, 1]), &state, &tails),
            Err(HandoverError::Malformed(reason)) if reason.contains("twice")
        ));
        assert!(matches!(
            encode(&header(vec![1, 2, 3, 4, 40]), &state, &tails),
            Err(HandoverError::Malformed(reason)) if reason.contains("twice")
        ));
        assert!(matches!(
            encode(&header(vec![1, 2, -3, 4, 5]), &state, &tails),
            Err(HandoverError::Malformed(reason)) if reason.contains("-3")
        ));
    }
}
