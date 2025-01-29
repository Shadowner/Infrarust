use std::{
    collections::HashMap,
    time::{Duration, SystemTime},
};

use log::debug;

use crate::{
    network::{
        packet::Packet,
        proxy_protocol::{errors::ProxyProtocolError, ProtocolResult},
    },
    server::motd::{generate_motd, MotdConfig},
    version::Version,
    ServerConnection, CONFIG,
};

use super::{backend::Server, ServerRequest};

pub struct StatusCache {
    ttl: Duration,
    entries: HashMap<u64, CacheEntry>,
}

struct CacheEntry {
    expires_at: SystemTime,
    response: Packet,
}

impl StatusCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: HashMap::new(),
        }
    }

    pub async fn get_status_response(
        &mut self,
        server: &Server,
        req: &ServerRequest,
    ) -> ProtocolResult<Packet> {
        let key = self.cache_key(server, req.protocol_version);

        if let Some(entry) = self.entries.get(&key) {
            if entry.expires_at > SystemTime::now() {
                return Ok(entry.response.clone());
            }
        }

        let response = match server.dial().await {
            Ok(mut conn) => self.fetch_status(&mut conn, req).await?,
            Err(e) => {
                let guard = CONFIG.read();

                if guard.motds.unreachable.is_some() {
                    let motd = guard.motds.unreachable.clone().unwrap();

                    if motd.enabled && !motd.is_empty() {
                        return generate_motd(&motd, true);
                    } else if motd.enabled {
                        return generate_motd(&MotdConfig::default_unreachable(), true);
                    }
                }

                debug!("Failed to connect to server: {}", e);

                return Err(ProxyProtocolError::Other(format!(
                    "Failed to connect to server: {}",
                    e
                )));
            }
        };

        if let Some(motd) = &server.config.motd {
            let response_packet = generate_motd(motd, false)?;

            self.entries.insert(
                key,
                CacheEntry {
                    expires_at: SystemTime::now() + self.ttl,
                    response: response_packet.clone(),
                },
            );

            return Ok(response_packet);
        }

        debug!("Caching status response for {:?}", server.config.addresses);
        self.entries.insert(
            key,
            CacheEntry {
                expires_at: SystemTime::now() + self.ttl,
                response: response.clone(),
            },
        );

        Ok(response)
    }

    pub async fn fetch_status(
        &self,
        conn: &mut ServerConnection,
        req: &ServerRequest,
    ) -> ProtocolResult<Packet> {
        debug!("ReadPacket: {:?}", req.read_packets[0]);
        debug!("ReadPacket: {:?}", req.read_packets[1]);
        conn.write_packet(&req.read_packets[0].clone()).await?;
        conn.write_packet(&req.read_packets[1].clone()).await?;
        conn.read_packet().await
    }

    fn cache_key(&self, server: &Server, version: Version) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        server.config.addresses[0].hash(&mut hasher);
        version.hash(&mut hasher);
        hasher.finish()
    }
}
