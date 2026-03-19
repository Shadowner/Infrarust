//! Multi-version registry data cache for limbo login.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::version::ProtocolVersion;

use crate::error::CoreError;
use crate::registry_data::RegistryDataProvider;

pub struct RegistryCodecCache {
    captured: RwLock<HashMap<ProtocolVersion, CapturedFrames>>,
    provider: Arc<dyn RegistryDataProvider>,
}

struct CapturedFrames {
    registry_frames: Vec<PacketFrame>,
    known_packs_frame: Option<PacketFrame>,
    finalized: bool,
}

impl RegistryCodecCache {
    pub(crate) fn new(provider: Arc<dyn RegistryDataProvider>) -> Self {
        Self {
            captured: RwLock::new(HashMap::new()),
            provider,
        }
    }

    pub fn collect_registry_frame(&self, version: ProtocolVersion, frame: PacketFrame) {
        let mut map = self.captured.write().expect("registry cache lock poisoned");
        let entry = map.entry(version).or_insert_with(|| CapturedFrames {
            registry_frames: Vec::new(),
            known_packs_frame: None,
            finalized: false,
        });
        if !entry.finalized {
            entry.registry_frames.push(frame);
        }
    }

    pub fn collect_known_packs_frame(&self, version: ProtocolVersion, frame: PacketFrame) {
        let mut map = self.captured.write().expect("registry cache lock poisoned");
        let entry = map.entry(version).or_insert_with(|| CapturedFrames {
            registry_frames: Vec::new(),
            known_packs_frame: None,
            finalized: false,
        });
        if !entry.finalized {
            entry.known_packs_frame = Some(frame);
        }
    }

    pub fn finalize(&self, version: ProtocolVersion) {
        let mut map = self.captured.write().expect("registry cache lock poisoned");
        if let Some(entry) = map.get_mut(&version) {
            entry.finalized = true;
            tracing::info!(
                protocol = version.0,
                frames = entry.registry_frames.len(),
                "Registry data captured and finalized"
            );
        }
    }

    pub fn get_registry_frames(
        &self,
        version: ProtocolVersion,
    ) -> Result<Vec<PacketFrame>, CoreError> {
        {
            let map = self.captured.read().expect("registry cache lock poisoned");
            if let Some(entry) = map.get(&version) {
                if entry.finalized && !entry.registry_frames.is_empty() {
                    return Ok(entry.registry_frames.clone());
                }
            }
        }

        self.provider.registry_frames(version)
    }

    pub fn get_known_packs_frame(
        &self,
        version: ProtocolVersion,
    ) -> Result<Option<PacketFrame>, CoreError> {
        {
            let map = self.captured.read().expect("registry cache lock poisoned");
            if let Some(entry) = map.get(&version) {
                if entry.finalized {
                    return Ok(entry.known_packs_frame.clone());
                }
            }
        }

        self.provider.known_packs_frame(version)
    }

    pub fn has_captured(&self, version: ProtocolVersion) -> bool {
        let map = self.captured.read().expect("registry cache lock poisoned");
        map.get(&version).is_some_and(|e| e.finalized)
    }

    pub fn supports_version(&self, version: ProtocolVersion) -> bool {
        self.has_captured(version) || self.provider.supports_version(version)
    }
}
