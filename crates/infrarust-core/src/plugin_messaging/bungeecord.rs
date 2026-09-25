use std::sync::Arc;

use bytes::Bytes;
use infrarust_api::events::connection::ConnectCause;
use infrarust_api::messaging::ChannelId;
use infrarust_api::player::Player;
use infrarust_api::types::{Component, LEGACY_SECTION, ProtocolVersion as ApiVersion, ServerId};
use infrarust_config::ServerConfig;
use infrarust_protocol::version::ConnectionState;

use super::channels::{self, MessageIds};
use super::java_io::{JavaIoError, JavaReader, JavaWriter};
use super::messenger::send_through_carriers;
use super::router::Scope;
use crate::error::CoreError;
use crate::player::PlayerSession;
use crate::session::backend_bridge::BackendBridge;
use crate::session::server_switch::validation::validate_switch_allowed;

const ALL: &str = "ALL";
const ONLINE: &str = "ONLINE";

struct Network<'a> {
    scope: &'a Scope<'a>,
    origin: &'a ServerConfig,
}

impl Network<'_> {
    fn contains(&self, config: &ServerConfig) -> bool {
        validate_switch_allowed(self.origin, config).is_ok()
    }

    fn server(&self, name: &str) -> Option<Arc<ServerConfig>> {
        self.scope
            .services
            .domain_router
            .find_by_server_id(name)
            .filter(|config| self.contains(config))
    }

    fn servers(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .scope
            .services
            .domain_router
            .list_all()
            .into_iter()
            .filter(|(_, config)| self.contains(config))
            .map(|(_, config)| config.effective_id())
            .collect();
        names.sort();
        names.dedup();
        names
    }

    fn holds(&self, session: &PlayerSession) -> bool {
        session.counted_server().is_some_and(|server| {
            server == *self.scope.server || self.server(server.as_str()).is_some()
        })
    }

    fn player(&self, name: &str) -> Option<Arc<PlayerSession>> {
        self.scope
            .services
            .connection_registry
            .find_by_username(name)
            .filter(|session| self.holds(session))
    }

    fn players(&self) -> Vec<Arc<PlayerSession>> {
        let mut players: Vec<Arc<PlayerSession>> = self
            .scope
            .services
            .connection_registry
            .all()
            .into_iter()
            .filter(|session| self.holds(session))
            .collect();
        players.sort_by(|a, b| a.profile().username.cmp(&b.profile().username));
        players
    }

    fn players_on(&self, server: &str) -> Vec<Arc<PlayerSession>> {
        let mut players = self
            .scope
            .services
            .connection_registry
            .find_by_server(server);
        players.sort_by(|a, b| a.profile().username.cmp(&b.profile().username));
        players
    }
}

fn names(players: &[Arc<PlayerSession>]) -> String {
    players
        .iter()
        .map(|p| p.profile().username.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn undashed(session: &PlayerSession) -> String {
    session.profile().uuid.simple().to_string()
}

fn forwarded(input: &mut JavaReader<'_>) -> Result<Vec<u8>, JavaIoError> {
    let channel = input.read_utf()?;
    let len = input.read_u16()?;
    let data = input.read_bytes(usize::from(len))?;
    let mut out = JavaWriter::new();
    out.write_utf(&channel)?.write_u16(len).write_bytes(data);
    Ok(out.into_bytes())
}

enum Reply {
    None,
    Data(Vec<u8>),
}

pub(crate) async fn handle(
    scope: &Scope<'_>,
    origin: &ServerConfig,
    data: &[u8],
    backend: &mut BackendBridge,
    state: ConnectionState,
) -> Result<(), CoreError> {
    let network = Network { scope, origin };
    let mut input = JavaReader::new(data);
    let reply = match input.read_utf() {
        Ok(subchannel) => match respond(&network, &subchannel, &mut input).await {
            Ok(reply) => reply,
            Err(e) => {
                tracing::debug!(%subchannel, server = %scope.server, "ignoring a malformed BungeeCord request: {e}");
                Reply::None
            }
        },
        Err(e) => {
            tracing::debug!(server = %scope.server, "ignoring a BungeeCord message without a subchannel: {e}");
            Reply::None
        }
    };
    let Reply::Data(reply) = reply else {
        return Ok(());
    };
    let ids = MessageIds::resolve(&scope.services.packet_registry, scope.version);
    let Some(id) = ids.serverbound(state) else {
        return Ok(());
    };
    let channel = ChannelId::bungeecord();
    let channel = channel.wire_name(ApiVersion::new(scope.version.0));
    backend.queue_frame(&channels::build(id, channel, &reply, scope.version))
}

#[allow(clippy::too_many_lines)]
async fn respond(
    network: &Network<'_>,
    subchannel: &str,
    input: &mut JavaReader<'_>,
) -> Result<Reply, JavaIoError> {
    let scope = network.scope;
    let allowed = scope.services.plugin_messaging.permissions();
    let requester = scope.session;
    let mut out = JavaWriter::new();
    match subchannel {
        "Connect" if allowed.connect => {
            let target = input.read_utf()?;
            if network.server(&target).is_some() {
                switch(requester, target);
            }
            Ok(Reply::None)
        }
        "ConnectOther" if allowed.connect_other => {
            let name = input.read_utf()?;
            let target = input.read_utf()?;
            if let Some(player) = network.player(&name)
                && network.server(&target).is_some()
            {
                switch(&player, target);
            }
            Ok(Reply::None)
        }
        "IP" if allowed.ip => {
            let addr = requester.remote_addr();
            out.write_utf("IP")?
                .write_utf(&addr.ip().to_string())?
                .write_i32(i32::from(addr.port()));
            Ok(Reply::Data(out.into_bytes()))
        }
        "IPOther" if allowed.ip_other => {
            let Some(player) = network.player(&input.read_utf()?) else {
                return Ok(Reply::None);
            };
            let addr = player.remote_addr();
            out.write_utf("IPOther")?
                .write_utf(&player.profile().username)?
                .write_utf(&addr.ip().to_string())?
                .write_i32(i32::from(addr.port()));
            Ok(Reply::Data(out.into_bytes()))
        }
        "PlayerCount" if allowed.player_count => {
            let target = input.read_utf()?;
            let (name, count) = if target == ALL {
                (ALL.to_string(), network.players().len())
            } else if let Some(config) = network.server(&target) {
                let name = config.effective_id();
                let count = network.players_on(&name).len();
                (name, count)
            } else {
                return Ok(Reply::None);
            };
            out.write_utf("PlayerCount")?
                .write_utf(&name)?
                .write_i32(i32::try_from(count).unwrap_or(i32::MAX));
            Ok(Reply::Data(out.into_bytes()))
        }
        "PlayerList" if allowed.player_list => {
            let target = input.read_utf()?;
            let (name, players) = if target == ALL {
                (ALL.to_string(), network.players())
            } else if let Some(config) = network.server(&target) {
                let name = config.effective_id();
                let players = network.players_on(&name);
                (name, players)
            } else {
                return Ok(Reply::None);
            };
            out.write_utf("PlayerList")?
                .write_utf(&name)?
                .write_utf(&names(&players))?;
            Ok(Reply::Data(out.into_bytes()))
        }
        "GetServers" if allowed.get_servers => {
            out.write_utf("GetServers")?
                .write_utf(&network.servers().join(", "))?;
            Ok(Reply::Data(out.into_bytes()))
        }
        "GetServer" if allowed.get_server => {
            out.write_utf("GetServer")?
                .write_utf(scope.server.as_str())?;
            Ok(Reply::Data(out.into_bytes()))
        }
        "GetPlayerServer" if allowed.get_player_server => {
            let Some(player) = network.player(&input.read_utf()?) else {
                return Ok(Reply::None);
            };
            let Some(server) = player.current_server() else {
                return Ok(Reply::None);
            };
            out.write_utf("GetPlayerServer")?
                .write_utf(&player.profile().username)?
                .write_utf(server.as_str())?;
            Ok(Reply::Data(out.into_bytes()))
        }
        "UUID" if allowed.uuid => {
            out.write_utf("UUID")?.write_utf(&undashed(requester))?;
            Ok(Reply::Data(out.into_bytes()))
        }
        "UUIDOther" if allowed.uuid_other => {
            let Some(player) = network.player(&input.read_utf()?) else {
                return Ok(Reply::None);
            };
            out.write_utf("UUIDOther")?
                .write_utf(&player.profile().username)?
                .write_utf(&undashed(&player))?;
            Ok(Reply::Data(out.into_bytes()))
        }
        "ServerIP" if allowed.server_ip => {
            let Some(config) = network.server(&input.read_utf()?) else {
                return Ok(Reply::None);
            };
            let Some(address) = config.addresses.first().map(|a| &a.address) else {
                return Ok(Reply::None);
            };
            out.write_utf("ServerIP")?
                .write_utf(&config.effective_id())?
                .write_utf(&address.host)?
                .write_u16(address.port);
            Ok(Reply::Data(out.into_bytes()))
        }
        "Message" if allowed.message => {
            let target = input.read_utf()?;
            let message = Component::from_legacy_with(&input.read_utf()?, LEGACY_SECTION);
            tell(network, &target, &message);
            Ok(Reply::None)
        }
        "MessageRaw" if allowed.message_raw => {
            let target = input.read_utf()?;
            let raw = input.read_utf()?;
            match Component::from_json(&raw) {
                Ok(message) => tell(network, &target, &message),
                Err(e) => tracing::debug!("ignoring a MessageRaw with an invalid component: {e}"),
            }
            Ok(Reply::None)
        }
        "KickPlayer" if allowed.kick_player => {
            let name = input.read_utf()?;
            let reason = Component::from_legacy_with(&input.read_utf()?, LEGACY_SECTION);
            if let Some(player) = network.player(&name) {
                player.disconnect(reason).await;
            }
            Ok(Reply::None)
        }
        "KickPlayerRaw" if allowed.kick_player_raw => {
            let name = input.read_utf()?;
            let raw = input.read_utf()?;
            if let Some(player) = network.player(&name) {
                match Component::from_json(&raw) {
                    Ok(reason) => player.disconnect(reason).await,
                    Err(e) => {
                        tracing::debug!("ignoring a KickPlayerRaw with an invalid component: {e}")
                    }
                }
            }
            Ok(Reply::None)
        }
        "Forward" if allowed.forward => {
            let target = input.read_utf()?;
            let payload = Bytes::from(forwarded(input)?);
            let targets: Vec<String> = if target == ALL || target == ONLINE {
                network
                    .servers()
                    .into_iter()
                    .filter(|name| name != scope.server.as_str())
                    .collect()
            } else {
                network
                    .server(&target)
                    .map(|config| config.effective_id())
                    .into_iter()
                    .collect()
            };
            let channel = ChannelId::bungeecord();
            for server in targets {
                let sent = send_through_carriers(
                    &scope.services.connection_registry,
                    &ServerId::new(server.as_str()),
                    &channel,
                    &payload,
                );
                if sent == 0 {
                    tracing::debug!(%server, "a BungeeCord Forward found no player to carry it");
                }
            }
            Ok(Reply::None)
        }
        "ForwardToPlayer" if allowed.forward_to_player => {
            let name = input.read_utf()?;
            let payload = Bytes::from(forwarded(input)?);
            if let Some(player) = network.player(&name)
                && let Err(e) =
                    player.send_plugin_message_to_backend(&ChannelId::bungeecord(), payload)
            {
                tracing::debug!(player = %name, "a BungeeCord ForwardToPlayer was not delivered: {e}");
            }
            Ok(Reply::None)
        }
        other => {
            tracing::debug!(subchannel = %other, server = %scope.server, "ignoring a BungeeCord subchannel that is unknown or not allowed");
            Ok(Reply::None)
        }
    }
}

fn switch(player: &PlayerSession, target: String) {
    if let Err(e) = player.request_switch(ServerId::new(target), ConnectCause::PluginMessage) {
        tracing::debug!(player = %player.profile().username, "a BungeeCord connect was not queued: {e}");
    }
}

fn tell(network: &Network<'_>, target: &str, message: &Component) {
    let players = if target == ALL {
        network.players()
    } else {
        network.player(target).into_iter().collect()
    };
    for player in players {
        if let Err(e) = player.send_message(message.clone()) {
            tracing::debug!(player = %player.profile().username, "a BungeeCord message was not delivered: {e}");
        }
    }
}
