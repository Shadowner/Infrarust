mod exe;
mod probe;

use std::ffi::OsString;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub use exe::{ExeIdentity, ExeSnapshot};
pub use probe::{ConfigCheck, ProbeError, check_config, probe};

pub const HANDOVER_PROTOCOL: u32 = 1;
pub const HANDOVER_ACCEPTS: &[u32] = &[HANDOVER_PROTOCOL];
pub const PROBE_SCHEMA: u32 = 1;
pub const PROBE_FLAG: &str = "--handover-probe";
pub const CHECK_CONFIG_COMMAND: &str = "check-config";

const HOT_RESTART_AVAILABLE: bool = true;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildInfo {
    pub version: &'static str,
    pub target: &'static str,
    pub official: bool,
    pub docker: bool,
}

impl BuildInfo {
    pub fn probe(&self) -> HandoverProbe {
        HandoverProbe {
            probe: PROBE_SCHEMA,
            version: self.version.to_string(),
            target: self.target.to_string(),
            official: self.official,
            docker: self.docker,
            handover: HandoverSupport::current(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoverSupport {
    pub protocol: u32,
    pub accepts: Vec<u32>,
}

impl HandoverSupport {
    pub fn current() -> Option<Self> {
        (cfg!(target_os = "linux") && HOT_RESTART_AVAILABLE).then(|| Self {
            protocol: HANDOVER_PROTOCOL,
            accepts: HANDOVER_ACCEPTS.to_vec(),
        })
    }

    pub fn accepts(&self, protocol: u32) -> bool {
        self.accepts.contains(&protocol)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoverProbe {
    pub probe: u32,
    pub version: String,
    pub target: String,
    pub official: bool,
    pub docker: bool,
    pub handover: Option<HandoverSupport>,
}

impl HandoverProbe {
    pub fn can_inherit_from(&self, running: Option<&HandoverSupport>) -> bool {
        match (running, &self.handover) {
            (Some(running), Some(successor)) => successor.accepts(running.protocol),
            _ => false,
        }
    }
}

#[derive(Debug)]
pub struct RunningBinary {
    pub build: BuildInfo,
    pub exe: ExeSnapshot,
    pub args: Vec<OsString>,
}

impl RunningBinary {
    pub fn new(build: BuildInfo, exe: ExeSnapshot, args: Vec<OsString>) -> Arc<Self> {
        Arc::new(Self { build, exe, args })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    const BUILD: BuildInfo = BuildInfo {
        version: "2.0.0-beta.6",
        target: "x86_64-unknown-linux-musl",
        official: true,
        docker: false,
    };

    #[test]
    fn probe_round_trips_through_json() {
        let probe = BUILD.probe();
        let line = serde_json::to_string(&probe).unwrap();
        let back: HandoverProbe = serde_json::from_str(&line).unwrap();
        assert_eq!(back, probe);
        assert_eq!(back.probe, PROBE_SCHEMA);
        assert_eq!(back.version, "2.0.0-beta.6");
    }

    #[test]
    fn probe_without_handover_serialises_null() {
        let probe = HandoverProbe {
            handover: None,
            ..BUILD.probe()
        };
        let value: serde_json::Value = serde_json::to_value(&probe).unwrap();
        assert!(value["handover"].is_null());
    }

    #[test]
    fn inheritance_needs_both_sides_and_an_accepted_protocol() {
        let support = |protocol, accepts: &[u32]| HandoverSupport {
            protocol,
            accepts: accepts.to_vec(),
        };
        let successor = HandoverProbe {
            handover: Some(support(2, &[1, 2])),
            ..BUILD.probe()
        };
        assert!(successor.can_inherit_from(Some(&support(1, &[1]))));
        assert!(!successor.can_inherit_from(Some(&support(3, &[3]))));
        assert!(!successor.can_inherit_from(None));
        let legacy = HandoverProbe {
            handover: None,
            ..BUILD.probe()
        };
        assert!(!legacy.can_inherit_from(Some(&support(1, &[1]))));
    }
}
