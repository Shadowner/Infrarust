use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::cookie::{
    CConfigCookieRequest, CConfigStoreCookie, CCookieRequest, CLoginCookieRequest, CStoreCookie,
    SConfigCookieResponse, SCookieResponse, SLoginCookieResponse,
};
use infrarust_protocol::packets::resource_pack::{
    CConfigResourcePack, CConfigResourcePackPush, CResourcePack, CResourcePackPush,
    ResourcePackResult, SConfigResourcePackResponse, SResourcePackResponse,
};
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};
use uuid::Uuid;

use crate::error::HarnessResult;
use crate::wire;

#[derive(Debug, Clone, Default)]
pub struct CookieJar {
    cookies: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl CookieJar {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Vec<u8>>> {
        self.cookies.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[must_use]
    pub fn with(self, key: impl Into<String>, value: impl Into<Vec<u8>>) -> Self {
        self.insert(key, value);
        self
    }

    pub fn insert(&self, key: impl Into<String>, value: impl Into<Vec<u8>>) {
        self.lock().insert(key.into(), value.into());
    }

    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.lock().get(key).cloned()
    }

    pub fn snapshot(&self) -> HashMap<String, Vec<u8>> {
        self.lock().clone()
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ClientReplies {
    pub(crate) jar: CookieJar,
    pub(crate) pack_results: Vec<ResourcePackResult>,
}

struct Pushed {
    id: Option<Uuid>,
    hash: String,
}

impl ClientReplies {
    pub(crate) fn answer(
        &self,
        frame: &PacketFrame,
        state: ConnectionState,
        version: ProtocolVersion,
    ) -> HarnessResult<Vec<PacketFrame>> {
        match state {
            ConnectionState::Login if wire::is::<CLoginCookieRequest>(frame, version) => {
                let key = wire::decode::<CLoginCookieRequest>(frame, version)?.key;
                let payload = self.jar.get(&key);
                Ok(vec![wire::encode(
                    &SLoginCookieResponse { key, payload },
                    version,
                )?])
            }
            ConnectionState::Config => self.answer_config(frame, version),
            ConnectionState::Play => self.answer_play(frame, version),
            _ => Ok(Vec::new()),
        }
    }

    fn answer_config(
        &self,
        frame: &PacketFrame,
        version: ProtocolVersion,
    ) -> HarnessResult<Vec<PacketFrame>> {
        if wire::is::<CConfigStoreCookie>(frame, version) {
            let cookie = wire::decode::<CConfigStoreCookie>(frame, version)?;
            self.jar.insert(cookie.key, cookie.payload);
            return Ok(Vec::new());
        }
        if wire::is::<CConfigCookieRequest>(frame, version) {
            let key = wire::decode::<CConfigCookieRequest>(frame, version)?.key;
            let payload = self.jar.get(&key);
            return Ok(vec![wire::encode(
                &SConfigCookieResponse { key, payload },
                version,
            )?]);
        }
        let pushed = if wire::is::<CConfigResourcePackPush>(frame, version) {
            let push = wire::decode::<CConfigResourcePackPush>(frame, version)?;
            Pushed {
                id: Some(push.id),
                hash: push.hash,
            }
        } else if wire::is::<CConfigResourcePack>(frame, version) {
            let pack = wire::decode::<CConfigResourcePack>(frame, version)?;
            Pushed {
                id: None,
                hash: pack.hash,
            }
        } else {
            return Ok(Vec::new());
        };
        self.pack_results
            .iter()
            .map(|result| {
                wire::encode(
                    &SConfigResourcePackResponse {
                        id: pushed.id,
                        hash: Some(pushed.hash.clone()),
                        result: *result,
                    },
                    version,
                )
            })
            .collect()
    }

    fn answer_play(
        &self,
        frame: &PacketFrame,
        version: ProtocolVersion,
    ) -> HarnessResult<Vec<PacketFrame>> {
        if wire::is::<CStoreCookie>(frame, version) {
            let cookie = wire::decode::<CStoreCookie>(frame, version)?;
            self.jar.insert(cookie.key, cookie.payload);
            return Ok(Vec::new());
        }
        if wire::is::<CCookieRequest>(frame, version) {
            let key = wire::decode::<CCookieRequest>(frame, version)?.key;
            let payload = self.jar.get(&key);
            return Ok(vec![wire::encode(
                &SCookieResponse { key, payload },
                version,
            )?]);
        }
        let pushed = if wire::is::<CResourcePackPush>(frame, version) {
            let push = wire::decode::<CResourcePackPush>(frame, version)?;
            Pushed {
                id: Some(push.id),
                hash: push.hash,
            }
        } else if wire::is::<CResourcePack>(frame, version) {
            let pack = wire::decode::<CResourcePack>(frame, version)?;
            Pushed {
                id: None,
                hash: pack.hash,
            }
        } else {
            return Ok(Vec::new());
        };
        self.pack_results
            .iter()
            .map(|result| {
                wire::encode(
                    &SResourcePackResponse {
                        id: pushed.id,
                        hash: Some(pushed.hash.clone()),
                        result: *result,
                    },
                    version,
                )
            })
            .collect()
    }
}
