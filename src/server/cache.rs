use std::{
    collections::HashMap,
    time::{Duration, SystemTime},
};

use crate::{
    network::{packet::Packet, proxy_protocol::ProtocolResult},
    version::Version,
    ServerConnection,
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

        let mut conn = server.dial().await?;
        let response = self.fetch_status(&mut conn, req).await?;

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
        conn.write_packet(&req.read_packets[0]).await?;
        conn.write_packet(&req.read_packets[1]).await?;
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
