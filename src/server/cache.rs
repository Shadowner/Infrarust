use std::{
    collections::HashMap,
    time::{Duration, SystemTime},
};

use tracing::{Instrument, debug, debug_span, instrument};

use crate::{
    CONFIG, ServerConnection,
    network::{
        packet::Packet,
        proxy_protocol::{ProtocolResult, errors::ProxyProtocolError},
    },
    server::motd::{MotdConfig, generate_motd},
    version::Version,
};

use crate::telemetry::TELEMETRY;

use super::{ServerRequest, backend::Server};

pub struct StatusCache {
    ttl: Duration,
    entries: HashMap<u64, CacheEntry>,
}

#[derive(Debug)]
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

    #[instrument(name = "get_status_response", skip(self, server), fields(
        server_addr = %server.config.addresses.first().unwrap_or(&String::new()),
        protocol_version = ?req.protocol_version
    ))]
    pub async fn get_status_response(
        &mut self,
        server: &Server,
        req: &ServerRequest,
    ) -> ProtocolResult<Packet> {
        match self.try_get_status_response(server, req).await {
            Ok(response) => Ok(response),
            Err(e) => {
                TELEMETRY.record_protocol_error(
                    "status_fetch_failed",
                    &e.to_string(),
                    req.session_id,
                );
                Err(e)
            }
        }
    }

    #[instrument(name = "try_get_status_response", skip(self, server), fields(
        server_addr = %server.config.addresses.first().unwrap_or(&String::new()),
        protocol_version = ?req.protocol_version
    ))]
    pub async fn try_get_status_response(
        &mut self,
        server: &Server,
        req: &ServerRequest,
    ) -> ProtocolResult<Packet> {
        let key = self.cache_key(server, req.protocol_version);

        if let Some(entry) = self.entries.get(&key) {
            if entry.expires_at > SystemTime::now() {
                debug!("Cache hit, returning cached status response");
                return Ok(entry.response.clone());
            }
            debug!("Cache expired");
        } else {
            debug!("Cache miss");
        }

        let use_proxy_protocol = server.config.send_proxy_protocol.unwrap_or(false);

        let response = match if use_proxy_protocol {
            debug!("Using proxy protocol for status connection");
            server
                .dial_with_proxy_protocol(req.session_id, req.client_addr)
                .instrument(debug_span!("backend_server_connect_with_proxy"))
                .await
        } else {
            debug!("Using standard connection for status");
            server
                .dial(req.session_id)
                .instrument(debug_span!("backend_server_connect"))
                .await
        } {
            Ok(mut conn) => {
                self.fetch_status(&mut conn, req)
                    .instrument(debug_span!("fetch_server_status"))
                    .await?
            }
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

        debug!("Caching new status response");
        self.entries.insert(
            key,
            CacheEntry {
                expires_at: SystemTime::now() + self.ttl,
                response: response.clone(),
            },
        );

        Ok(response)
    }

    #[instrument(skip(self, conn), fields(
        packets_count = %req.read_packets.len()
    ))]
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
