use std::collections::HashMap;
use std::sync::Mutex;

use crate::services::load_balancer::{BackendState, BackendStatus, LbError, LoadBalancerService};
use crate::types::{ServerAddress, ServerId};

use super::lock;

#[derive(Debug, Clone)]
struct Pool {
    strategy: String,
    backends: Vec<BackendStatus>,
}

#[derive(Debug, Default)]
pub struct MockLoadBalancerService {
    servers: Mutex<HashMap<ServerId, Pool>>,
}

impl MockLoadBalancerService {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_server(
        self,
        server: impl Into<ServerId>,
        strategy: impl Into<String>,
        addresses: impl IntoIterator<Item = ServerAddress>,
    ) -> Self {
        let backends = addresses.into_iter().map(healthy).collect();
        lock(&self.servers).insert(
            server.into(),
            Pool {
                strategy: strategy.into(),
                backends,
            },
        );
        self
    }

    pub fn set_status(&self, server: &ServerId, status: BackendStatus) -> Result<(), LbError> {
        let mut servers = lock(&self.servers);
        let backend = find(&mut servers, server, &status.address)?;
        *backend = status;
        Ok(())
    }
}

fn healthy(address: ServerAddress) -> BackendStatus {
    BackendStatus {
        address,
        weight: 1,
        effective_weight: 1,
        state: BackendState::Healthy,
        active_connections: 0,
        healthy_since_secs: None,
        ejections: 0,
        last_failure_secs_ago: None,
    }
}

fn find<'a>(
    servers: &'a mut HashMap<ServerId, Pool>,
    server: &ServerId,
    addr: &ServerAddress,
) -> Result<&'a mut BackendStatus, LbError> {
    let pool = servers
        .get_mut(server)
        .ok_or_else(|| LbError::UnknownServer(server.clone()))?;
    pool.backends
        .iter_mut()
        .find(|backend| &backend.address == addr)
        .ok_or_else(|| LbError::UnknownAddress {
            server: server.clone(),
            address: addr.clone(),
        })
}

impl crate::services::load_balancer::private::Sealed for MockLoadBalancerService {}

impl LoadBalancerService for MockLoadBalancerService {
    fn strategy(&self, server: &ServerId) -> Option<String> {
        lock(&self.servers)
            .get(server)
            .map(|pool| pool.strategy.clone())
    }

    fn backends(&self, server: &ServerId) -> Vec<BackendStatus> {
        lock(&self.servers)
            .get(server)
            .map(|pool| pool.backends.clone())
            .unwrap_or_default()
    }

    fn set_drained(
        &self,
        server: &ServerId,
        addr: &ServerAddress,
        drained: bool,
    ) -> Result<(), LbError> {
        let mut servers = lock(&self.servers);
        let backend = find(&mut servers, server, addr)?;
        backend.state = if drained {
            BackendState::Draining
        } else {
            BackendState::Healthy
        };
        Ok(())
    }

    fn reset_backend(&self, server: &ServerId, addr: &ServerAddress) -> Result<(), LbError> {
        let mut servers = lock(&self.servers);
        let backend = find(&mut servers, server, addr)?;
        backend.ejections = 0;
        backend.last_failure_secs_ago = None;
        if backend.state != BackendState::Draining {
            backend.state = BackendState::Healthy;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn address(port: u16) -> ServerAddress {
        ServerAddress {
            host: "10.0.0.1".to_owned(),
            port,
        }
    }

    fn service() -> MockLoadBalancerService {
        MockLoadBalancerService::new().with_server(
            "lobby",
            "least_conn",
            [address(25565), address(25566)],
        )
    }

    #[test]
    fn a_configured_server_reports_its_strategy_and_healthy_backends() {
        let lb = service();
        let lobby = ServerId::new("lobby");
        assert_eq!(lb.strategy(&lobby).as_deref(), Some("least_conn"));
        let backends = lb.backends(&lobby);
        assert_eq!(backends.len(), 2);
        assert!(
            backends
                .iter()
                .all(|backend| backend.state == BackendState::Healthy)
        );
        assert_eq!(lb.strategy(&ServerId::new("nowhere")), None);
        assert!(lb.backends(&ServerId::new("nowhere")).is_empty());
    }

    #[test]
    fn draining_and_undraining_changes_only_that_backend() {
        let lb = service();
        let lobby = ServerId::new("lobby");
        lb.set_drained(&lobby, &address(25565), true).unwrap();
        let states: Vec<_> = lb.backends(&lobby).iter().map(|b| b.state).collect();
        assert_eq!(states, [BackendState::Draining, BackendState::Healthy]);
        lb.set_drained(&lobby, &address(25565), false).unwrap();
        assert_eq!(lb.backends(&lobby)[0].state, BackendState::Healthy);
    }

    #[test]
    fn resetting_clears_the_failure_history_but_keeps_a_drain() {
        let lb = service();
        let lobby = ServerId::new("lobby");
        let mut ejected = lb.backends(&lobby)[1].clone();
        ejected.state = BackendState::Unhealthy;
        ejected.ejections = 3;
        ejected.last_failure_secs_ago = Some(4);
        lb.set_status(&lobby, ejected).unwrap();
        lb.reset_backend(&lobby, &address(25566)).unwrap();
        let reset = &lb.backends(&lobby)[1];
        assert_eq!(reset.state, BackendState::Healthy);
        assert_eq!(reset.ejections, 0);
        assert_eq!(reset.last_failure_secs_ago, None);

        lb.set_drained(&lobby, &address(25565), true).unwrap();
        lb.reset_backend(&lobby, &address(25565)).unwrap();
        assert_eq!(lb.backends(&lobby)[0].state, BackendState::Draining);
    }

    #[test]
    fn unknown_servers_and_addresses_are_errors() {
        let lb = service();
        assert!(matches!(
            lb.set_drained(&ServerId::new("nowhere"), &address(25565), true),
            Err(LbError::UnknownServer(_))
        ));
        assert!(matches!(
            lb.reset_backend(&ServerId::new("lobby"), &address(1)),
            Err(LbError::UnknownAddress { .. })
        ));
    }
}
