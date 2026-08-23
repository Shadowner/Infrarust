//! [`LoadBalancerService`] implementation — backend visibility and drain control.

use std::sync::Arc;

use infrarust_api::services::load_balancer::{BackendStatus, LbError, LoadBalancerService};
use infrarust_api::types::{ServerAddress as ApiServerAddress, ServerId};
use infrarust_config::ServerAddress;

use crate::loadbalancer::{
    AddressConnectionCount, BackendCandidate, PassiveBackendHealth, SlowStartConfig,
    effective_weight,
};
use crate::routing::DomainRouter;

pub struct LoadBalancerServiceImpl {
    router: Arc<DomainRouter>,
    health: Arc<PassiveBackendHealth>,
    counts: Arc<dyn AddressConnectionCount>,
}

impl LoadBalancerServiceImpl {
    pub fn new(
        router: Arc<DomainRouter>,
        health: Arc<PassiveBackendHealth>,
        counts: Arc<dyn AddressConnectionCount>,
    ) -> Self {
        Self {
            router,
            health,
            counts,
        }
    }

    /// Resolves an address against the server that is supposed to own it, so
    /// a typo can never drain something the caller did not name.
    fn backend_of(
        &self,
        server: &ServerId,
        addr: &ApiServerAddress,
    ) -> Result<ServerAddress, LbError> {
        let config = self
            .router
            .find_by_server_id(server.as_str())
            .ok_or_else(|| LbError::UnknownServer(server.clone()))?;
        let wanted = ServerAddress {
            host: addr.host.clone(),
            port: addr.port,
        };
        if config.addresses.iter().any(|wa| wa.address == wanted) {
            Ok(wanted)
        } else {
            Err(LbError::UnknownAddress {
                server: server.clone(),
                address: addr.clone(),
            })
        }
    }
}

impl infrarust_api::services::load_balancer::private::Sealed for LoadBalancerServiceImpl {}

impl LoadBalancerService for LoadBalancerServiceImpl {
    fn strategy(&self, server: &ServerId) -> Option<String> {
        self.router
            .find_route_by_server_id(server.as_str())
            .map(|(_, lb)| lb.name().to_string())
    }

    fn backends(&self, server: &ServerId) -> Vec<BackendStatus> {
        let Some(config) = self.router.find_by_server_id(server.as_str()) else {
            return Vec::new();
        };
        let balance = config.balance_config();
        let slow_start = balance.slow_start.map(|window| SlowStartConfig {
            window,
            aggression: balance.slow_start_aggression,
        });

        config
            .addresses
            .iter()
            .map(|wa| {
                let report = self.health.report(&wa.address);
                let active_connections = self.counts.active_connections_for_address(&wa.address);
                let candidate = BackendCandidate {
                    address: wa.address.clone(),
                    weight: wa.weight,
                    active_connections,
                    state: report.state,
                    healthy_since: report.healthy_since,
                    probe: false,
                };
                BackendStatus {
                    address: ApiServerAddress {
                        host: wa.address.host.clone(),
                        port: wa.address.port,
                    },
                    weight: wa.weight,
                    effective_weight: effective_weight(&candidate, slow_start.as_ref()).round()
                        as u32,
                    state: report.state.to_api(),
                    active_connections,
                    healthy_since_secs: report.healthy_since.map(|at| at.elapsed().as_secs()),
                    ejections: report.ejections,
                    last_failure_secs_ago: report.last_failure.map(|at| at.elapsed().as_secs()),
                }
            })
            .collect()
    }

    fn set_drained(
        &self,
        server: &ServerId,
        addr: &ApiServerAddress,
        drained: bool,
    ) -> Result<(), LbError> {
        self.health
            .set_drained(&self.backend_of(server, addr)?, drained);
        Ok(())
    }

    fn reset_backend(&self, server: &ServerId, addr: &ApiServerAddress) -> Result<(), LbError> {
        self.health.reset(&self.backend_of(server, addr)?);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    use infrarust_api::services::load_balancer::BackendState as ApiBackendState;

    use crate::loadbalancer::{BackendLoad, PendingRegistry};
    use crate::provider::ProviderId;

    fn service(addresses: &str) -> (LoadBalancerServiceImpl, Arc<PassiveBackendHealth>) {
        let router = Arc::new(DomainRouter::new());
        router.add(
            ProviderId::file("lobby"),
            toml::from_str(&format!(
                "name = \"lobby\"\ndomains = [\"lobby.test\"]\naddresses = [{addresses}]\n"
            ))
            .unwrap(),
        );
        let health = Arc::new(PassiveBackendHealth::with_threshold(1));
        let service =
            LoadBalancerServiceImpl::new(router, Arc::clone(&health), Arc::new(BackendLoad::new()));
        (service, health)
    }

    fn api_addr(host: &str, port: u16) -> ApiServerAddress {
        ApiServerAddress {
            host: host.to_string(),
            port,
        }
    }

    #[test]
    fn reports_every_configured_address() {
        let (service, _) = service("\"10.0.0.1:25565\", \"10.0.0.2:25566\"");
        let backends = service.backends(&ServerId::new("lobby"));
        assert_eq!(backends.len(), 2);
        assert_eq!(backends[0].address.to_string(), "10.0.0.1:25565");
        assert_eq!(backends[0].state, ApiBackendState::Healthy);
        assert_eq!(backends[0].effective_weight, 1);
    }

    #[test]
    fn active_connections_leave_out_the_logins_still_negotiating() {
        let router = Arc::new(DomainRouter::new());
        router.add(
            ProviderId::file("lobby"),
            toml::from_str(
                "name = \"lobby\"\ndomains = [\"lobby.test\"]\naddresses = [\"10.0.0.1:25565\"]\n",
            )
            .unwrap(),
        );
        let attached = Arc::new(BackendLoad::new());
        let pending = Arc::new(PendingRegistry::new(
            Arc::clone(&attached) as _,
            std::time::Duration::from_secs(60),
        ));
        let service = LoadBalancerServiceImpl::new(
            router,
            Arc::new(PassiveBackendHealth::new()),
            Arc::clone(&attached) as _,
        );
        let address = ServerAddress {
            host: "10.0.0.1".into(),
            port: 25565,
        };

        attached.acquire(&address);
        let _ticket = pending.reserve(&address);

        assert_eq!(
            service.backends(&ServerId::new("lobby"))[0].active_connections,
            1,
            "a reservation is not an active connection"
        );
    }

    #[test]
    fn drain_shows_up_in_the_report() {
        let (service, _) = service("\"10.0.0.1:25565\"");
        let server = ServerId::new("lobby");
        service
            .set_drained(&server, &api_addr("10.0.0.1", 25565), true)
            .unwrap();
        assert_eq!(
            service.backends(&server)[0].state,
            ApiBackendState::Draining
        );
    }

    #[test]
    fn reset_clears_the_ejection_count() {
        let (service, health) = service("\"10.0.0.1:25565\"");
        let server = ServerId::new("lobby");
        let address = ServerAddress {
            host: "10.0.0.1".into(),
            port: 25565,
        };
        health.record_failure(&address);
        assert_eq!(service.backends(&server)[0].ejections, 1);

        service
            .reset_backend(&server, &api_addr("10.0.0.1", 25565))
            .unwrap();
        let backend = &service.backends(&server)[0];
        assert_eq!(backend.ejections, 0);
        assert_eq!(backend.state, ApiBackendState::Healthy);
        assert!(backend.last_failure_secs_ago.is_none());
    }

    #[test]
    fn unknown_server_and_address_are_rejected() {
        let (service, _) = service("\"10.0.0.1:25565\"");
        assert!(matches!(
            service.set_drained(&ServerId::new("ghost"), &api_addr("10.0.0.1", 25565), true),
            Err(LbError::UnknownServer(_))
        ));
        assert!(matches!(
            service.reset_backend(&ServerId::new("lobby"), &api_addr("10.0.0.9", 25565)),
            Err(LbError::UnknownAddress { .. })
        ));
    }

    #[test]
    fn strategy_names_the_router_instance() {
        let (service, _) = service("\"10.0.0.1:25565\"");
        assert_eq!(
            service.strategy(&ServerId::new("lobby")).as_deref(),
            Some("first_available")
        );
        assert!(service.strategy(&ServerId::new("ghost")).is_none());
    }
}
