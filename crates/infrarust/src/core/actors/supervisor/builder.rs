use infrarust_config::models::server::ProxyModeEnum;
use tokio::sync::oneshot;

use crate::{Connection, server::ServerResponse};

use super::{ActorSupervisor, actor_pair::ActorPair};

pub struct ActorPairBuilder<'a> {
    supervisor: &'a ActorSupervisor,
    config_id: Option<&'a str>,
    client_conn: Option<Connection>,
    proxy_mode: Option<ProxyModeEnum>,
    oneshot_request_receiver: Option<oneshot::Receiver<ServerResponse>>,
    is_login: bool,
    username: String,
    domain: String,
}

impl<'a> ActorPairBuilder<'a> {
    pub fn new(supervisor: &'a ActorSupervisor) -> Self {
        Self {
            supervisor,
            config_id: None,
            client_conn: None,
            proxy_mode: None,
            oneshot_request_receiver: None,
            is_login: false,
            username: String::new(),
            domain: String::new(),
        }
    }

    pub fn config_id(mut self, config_id: &'a str) -> Self {
        self.config_id = Some(config_id);
        self
    }

    pub fn client_conn(mut self, conn: Connection) -> Self {
        self.client_conn = Some(conn);
        self
    }

    pub fn proxy_mode(mut self, mode: ProxyModeEnum) -> Self {
        self.proxy_mode = Some(mode);
        self
    }

    pub fn oneshot_receiver(mut self, receiver: oneshot::Receiver<ServerResponse>) -> Self {
        self.oneshot_request_receiver = Some(receiver);
        self
    }

    pub fn is_login(mut self, is_login: bool) -> Self {
        self.is_login = is_login;
        self
    }

    pub fn username(mut self, username: impl Into<String>) -> Self {
        self.username = username.into();
        self
    }

    pub fn domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = domain.into();
        self
    }

    pub async fn build(self) -> Option<ActorPair> {
        let config_id = self.config_id?;
        let client_conn = self.client_conn?;
        let proxy_mode = self.proxy_mode?;
        let oneshot_request_receiver = self.oneshot_request_receiver?;

        Some(
            self.supervisor
                .create_actor_pair(
                    config_id,
                    client_conn,
                    proxy_mode,
                    oneshot_request_receiver,
                    self.is_login,
                    self.username,
                    &self.domain,
                )
                .await,
        )
    }
}
