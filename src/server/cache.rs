use std::{
    collections::HashMap,
    time::{Duration, Instant, SystemTime},
};

use tracing::{Instrument, debug, debug_span, instrument};

use crate::{
    CONFIG, ServerConnection,
    network::{
        packet::Packet,
        proxy_protocol::{ProtocolResult, errors::ProxyProtocolError},
    },
    server::{
        ServerRequest,
        backend::Server,
        motd::{MotdConfig, generate_motd},
    },
    version::Version,
};

#[derive(Debug)]
pub struct StatusCache {
    ttl: Duration,
    entries: HashMap<u64, CacheEntry>,
}

#[derive(Debug, Clone)]
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
        let key = self.cache_key(server, req.protocol_version);
        debug!(
            "Status lookup for domain: {}, cache key: {}",
            req.domain, key
        );

        if let Some(cached) = self.check_cache(key) {
            return Ok(cached);
        }

        debug!("Cache miss for {}, fetching from server", req.domain);
        match self.fetch_from_server(server, req).await {
            Ok(response) => {
                debug!("Server fetch successful for {}", req.domain);
                self.update_cache(key, response.clone());
                Ok(response)
            }
            Err(e) => {
                debug!("Server fetch failed for {}: {}", req.domain, e);
                self.handle_server_fetch_error(server, req).await
            }
        }
    }

    fn check_cache(&self, key: u64) -> Option<Packet> {
        if let Some(entry) = self.entries.get(&key) {
            if entry.expires_at > SystemTime::now() {
                debug!("Cache hit for key: {}", key);
                return Some(entry.response.clone());
            }
            debug!("Cache entry expired for key: {}", key);
        }
        None
    }

    fn update_cache(&mut self, key: u64, response: Packet) {
        self.entries.insert(
            key,
            CacheEntry {
                expires_at: SystemTime::now() + self.ttl,
                response,
            },
        );
        debug!(
            "Cache updated for key: {}, cache size: {}",
            key,
            self.entries.len()
        );
    }

    #[instrument(name = "fetch_from_server", skip(self, server), fields(
        server_addr = %server.config.addresses.first().unwrap_or(&String::new()),
        domain = %req.domain
    ))]
    async fn fetch_from_server(
        &self,
        server: &Server,
        req: &ServerRequest,
    ) -> ProtocolResult<Packet> {
        let use_proxy_protocol = server.config.send_proxy_protocol.unwrap_or(false);
        let start_time = Instant::now();

        debug!(
            "Connecting to server for domain: {} (proxy protocol: {})",
            req.domain, use_proxy_protocol
        );

        let connect_result = if use_proxy_protocol {
            server
                .dial_with_proxy_protocol(req.session_id, req.client_addr)
                .instrument(debug_span!("connect_with_proxy"))
                .await
        } else {
            server
                .dial(req.session_id)
                .instrument(debug_span!("connect_standard"))
                .await
        };

        match connect_result {
            Ok(mut conn) => {
                debug!("Connected to server after {:?}", start_time.elapsed());

                let fetch_start = Instant::now();
                match self.fetch_status(&mut conn, req).await {
                    Ok(packet) => {
                        debug!("Status fetched in {:?}", fetch_start.elapsed());
                        Ok(packet)
                    }
                    Err(e) => {
                        debug!("Status fetch failed: {}", e);
                        Err(e)
                    }
                }
            }
            Err(e) => {
                debug!("Connection failed: {}", e);
                Err(e)
            }
        }
    }

    async fn handle_server_fetch_error(
        &self,
        server: &Server,
        req: &ServerRequest,
    ) -> ProtocolResult<Packet> {
        debug!("Generating fallback MOTD for {}", req.domain);

        if let Some(motd) = &server.config.motd {
            debug!("Using server-specific MOTD for {}", req.domain);
            return generate_motd(motd, false);
        }

        let guard = CONFIG.read();
        if let Some(motd) = guard.motds.unreachable.clone() {
            if motd.enabled {
                if !motd.is_empty() {
                    debug!("Using global 'unreachable' MOTD");
                    return generate_motd(&motd, true);
                }
                debug!("Using default 'unreachable' MOTD");
                return generate_motd(&MotdConfig::default_unreachable(), true);
            }
        }

        Err(ProxyProtocolError::Other(format!(
            "Failed to connect to server for domain: {}",
            req.domain
        )))
    }

    #[instrument(skip(self, conn), fields(
        domain = %req.domain,
        session_id = %req.session_id
    ))]
    async fn fetch_status(
        &self,
        conn: &mut ServerConnection,
        req: &ServerRequest,
    ) -> ProtocolResult<Packet> {
        if let Err(e) = conn.write_packet(&req.read_packets[0].clone()).await {
            debug!("Failed to send handshake: {}", e);
            return Err(e);
        }

        if let Err(e) = conn.write_packet(&req.read_packets[1].clone()).await {
            debug!("Failed to send status request: {}", e);
            return Err(e);
        }

        let start = Instant::now();
        let result = conn.read_packet().await;
        let elapsed = start.elapsed();

        match &result {
            Ok(_) => debug!("Got status response in {:?}", elapsed),
            Err(e) => debug!(
                "Failed to read status response: {} (after {:?})",
                e, elapsed
            ),
        }

        result
    }

    fn cache_key(&self, server: &Server, version: Version) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        if !server.config.addresses.is_empty() {
            server.config.addresses[0].hash(&mut hasher);
        }
        version.hash(&mut hasher);
        hasher.finish()
    }

    pub async fn check_cache_only(
        &mut self,
        server: &Server,
        req: &ServerRequest,
    ) -> ProtocolResult<Option<Packet>> {
        let key = self.cache_key(server, req.protocol_version);
        debug!(
            "Quick cache check for domain: {} (key: {})",
            req.domain, key
        );

        Ok(self.check_cache(key))
    }

    pub async fn update_cache_for(
        &mut self,
        server: &Server,
        req: &ServerRequest,
        response: Packet,
    ) {
        let key = self.cache_key(server, req.protocol_version);
        debug!("Updating cache for domain: {} (key: {})", req.domain, key);
        self.update_cache(key, response);
    }
}
