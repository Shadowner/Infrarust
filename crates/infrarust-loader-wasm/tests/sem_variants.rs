#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use infrarust_api::event::bus::{EventBus, EventBusExt};
use infrarust_api::event::{EventPriority, ResultedEvent};
use infrarust_api::events::ban::{BanIssuedEvent, BanRevokedEvent};
use infrarust_api::events::chat::{ChatMessageEvent, ChatMessageResult};
use infrarust_api::events::client::{PlayerChannelRegisterEvent, PlayerSettingsChangedEvent};
use infrarust_api::events::command::{CommandExecuteEvent, CommandExecuteResult};
use infrarust_api::events::connection::{
    ConnectCause, KickCause, KickedFromServerEvent, KickedFromServerResult, ServerPreConnectEvent,
};
use infrarust_api::events::handshake::{
    ConnectionHandshakeEvent, ConnectionHandshakeResult, ConnectionRejectedEvent, HandshakeIntent,
    RejectReason,
};
use infrarust_api::events::lifecycle::{
    DisconnectCause, DisconnectEvent, GameProfileRequestEvent, PermissionsSetupEvent,
    PermissionsSetupResult, PostLoginEvent,
};
use infrarust_api::events::limbo::{LimboEnterEvent, LimboExitEvent, LimboExitReason};
use infrarust_api::events::messaging::PluginMessageEvent;
use infrarust_api::events::named::NamedEvent;
use infrarust_api::events::packet::PacketDirection;
use infrarust_api::events::proxy::{
    BackendHealthEvent, PingResponse, ProxyPingEvent, ServerStateChangeEvent,
};
use infrarust_api::events::resource_pack::{PlayerResourcePackStatusEvent, ResourcePackOrigin};
use infrarust_api::events::transfer::{PreTransferEvent, TransferOrigin};
use infrarust_api::limbo::context::LimboEntryContext;
use infrarust_api::loader::PluginLoader;
use infrarust_api::messaging::{ChannelId, Endpoint, MessagePhase};
use infrarust_api::permissions::PermissionMap;
use infrarust_api::player::{
    ChatMode, ClientSettings, MainHand, ParticleStatus, Player, ResourcePackStatus, SkinParts,
};
use infrarust_api::plugin::Plugin;
use infrarust_api::services::ban_service::{BanEntry, BanSource, BanTarget};
use infrarust_api::services::load_balancer::BackendState;
use infrarust_api::services::server_manager::ServerState;
use infrarust_api::types::{
    Component, GameProfile, ProfileProperty, ProtocolVersion, ServerAddress, ServerId,
};
use infrarust_core::event_bus::EventBusImpl;
use infrarust_loader_wasm::WasmPluginLoader;

use support::{
    EnvOptions, TestEnv, load_enabled, loader_from_toml, make_env_with, read_log, stage,
};

const PROBE: &str = "sem-probe";

struct Dump {
    _tmp: tempfile::TempDir,
    data: PathBuf,
    env: TestEnv,
    _loader: WasmPluginLoader,
    _plugin: Box<dyn Plugin>,
}

impl Dump {
    async fn start() -> Self {
        Self::with("dump", |_| {}).await
    }

    async fn with(config: &str, before_load: impl FnOnce(&TestEnv)) -> Self {
        let (tmp, plugins_dir) = stage(PROBE);
        let data = plugins_dir.join(PROBE);
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("probe.txt"), config).unwrap();
        let env = make_env_with(
            plugins_dir.clone(),
            EnvOptions::default()
                .grant(PROBE, "chat-intercept")
                .grant(PROBE, "plugin-messaging"),
        );
        before_load(&env);
        let loader = loader_from_toml("");
        loader.discover(&plugins_dir).await.unwrap();
        let plugin = load_enabled(&loader, &env.factory, PROBE).await;
        Self {
            _tmp: tmp,
            data,
            env,
            _loader: loader,
            _plugin: plugin,
        }
    }

    fn bus(&self) -> &EventBusImpl {
        &self.env.event_bus
    }

    fn dumped(&self, event: &str) -> Vec<String> {
        let prefix = format!("{event} ");
        read_log(&self.data)
            .into_iter()
            .filter(|line| line.starts_with(&prefix))
            .collect()
    }
}

fn steve() -> Arc<dyn Player> {
    support::session_player(
        1,
        GameProfile {
            uuid: uuid::Uuid::from_u128(7),
            username: "Steve".to_owned(),
            properties: vec![ProfileProperty {
                name: "textures".to_owned(),
                value: "dGV4dHVyZXM=".to_owned(),
                signature: Some("c2lnbmVk".to_owned()),
            }],
        },
        767,
        "203.0.113.7:40000".parse().unwrap(),
    )
}

fn expect_each(lines: &[String], needles: &[&str]) {
    assert_eq!(
        lines.len(),
        needles.len(),
        "one dump per fired event: {lines:#?}"
    );
    for (line, needle) in lines.iter().zip(needles) {
        assert!(line.contains(needle), "expected {needle:?} in {line}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn rare_variants_of_every_family_reach_the_guest_intact() {
    let dump = Dump::start().await;
    let bus = dump.bus();

    for cause in [
        DisconnectCause::Kicked {
            reason: Some(Component::text("kick-text")),
        },
        DisconnectCause::BackendClosed { reason: None },
        DisconnectCause::Shutdown,
        DisconnectCause::Error,
    ] {
        bus.fire(DisconnectEvent::new(steve(), None, cause)).await;
    }
    expect_each(
        &dump.dumped("DisconnectEvent"),
        &[
            "kick-text",
            "BackendClosed(None)",
            "cause: Shutdown",
            "cause: Error",
        ],
    );

    bus.fire(PostLoginEvent::new(steve())).await;
    expect_each(
        &dump.dumped("PostLoginEvent"),
        &["signature: Some(\"c2lnbmVk\")"],
    );

    for (cause, result) in [
        (
            KickCause::Unreachable {
                error: "refused-by-host".to_owned(),
            },
            KickedFromServerResult::Notify {
                message: Component::text("notify-text"),
            },
        ),
        (
            KickCause::LoginRefused,
            KickedFromServerResult::SendToLimbo {
                limbo_handlers: vec!["gate".to_owned()],
            },
        ),
        (
            KickCause::ConfigDisconnect,
            KickedFromServerResult::DisconnectPlayer { reason: None },
        ),
        (
            KickCause::ConnectionLost,
            KickedFromServerResult::Redirect(ServerId::new("hub")),
        ),
    ] {
        bus.fire(KickedFromServerEvent::new(
            steve(),
            ServerId::new("survival"),
            None,
            cause,
            true,
            None,
            result,
        ))
        .await;
    }
    expect_each(
        &dump.dumped("KickedFromServerEvent"),
        &[
            "refused-by-host",
            "LoginRefused",
            "ConfigDisconnect",
            "ConnectionLost",
        ],
    );
    let kicked = dump.dumped("KickedFromServerEvent");
    for (line, result) in
        kicked
            .iter()
            .zip(["notify-text", "gate", "DisconnectPlayer(None)", "hub"])
    {
        assert!(line.contains(result), "expected {result:?} in {line}");
    }

    for cause in [
        ConnectCause::Initial,
        ConnectCause::LimboExit,
        ConnectCause::KickRedirect,
        ConnectCause::PluginMessage,
    ] {
        bus.fire(ServerPreConnectEvent::new(
            steve(),
            ServerId::new("lobby"),
            None,
            cause,
        ))
        .await;
    }
    expect_each(
        &dump.dumped("ServerPreConnectEvent"),
        &["Initial", "LimboExit", "KickRedirect", "PluginMessage"],
    );

    for intent in [HandshakeIntent::Status, HandshakeIntent::Transfer] {
        bus.fire(
            ConnectionHandshakeEvent::new(
                "203.0.113.7:40000".parse().unwrap(),
                intent,
                ProtocolVersion::new(767),
            )
            .with_host("\u{a7}legacy", None, 25565),
        )
        .await;
    }
    expect_each(
        &dump.dumped("ConnectionHandshakeEvent"),
        &["intent: Status", "intent: Transfer"],
    );

    for reason in [
        RejectReason::IpFilter,
        RejectReason::RateLimit,
        RejectReason::UnknownDomain,
        RejectReason::IpBanned,
        RejectReason::Banned,
        RejectReason::ServerUnavailable,
        RejectReason::Plugin { plugin_id: None },
    ] {
        bus.fire(ConnectionRejectedEvent::new(
            "203.0.113.7:40000".parse().unwrap(),
            None,
            reason,
        ))
        .await;
    }
    expect_each(
        &dump.dumped("ConnectionRejectedEvent"),
        &[
            "IpFilter",
            "RateLimit",
            "UnknownDomain",
            "IpBanned",
            "reason: Banned",
            "ServerUnavailable",
            "Plugin(None)",
        ],
    );

    for context in [
        LimboEntryContext::InitialConnection {
            target_server: ServerId::new("hub"),
        },
        LimboEntryContext::PluginRedirect { from_server: None },
        LimboEntryContext::PluginRedirect {
            from_server: Some(ServerId::new("origin")),
        },
    ] {
        bus.fire(LimboEnterEvent::new(
            steve(),
            vec!["gate".to_owned()],
            context,
        ))
        .await;
    }
    expect_each(
        &dump.dumped("LimboEnterEvent"),
        &["InitialConnection", "PluginRedirect(None)", "origin"],
    );

    for reason in [
        LimboExitReason::Released,
        LimboExitReason::Redirected,
        LimboExitReason::Kicked {
            reason: Component::text("limbo-kick"),
        },
        LimboExitReason::Disconnected,
        LimboExitReason::TimedOut,
        LimboExitReason::Shutdown,
    ] {
        bus.fire(LimboExitEvent::new(steve(), reason, None)).await;
    }
    expect_each(
        &dump.dumped("LimboExitEvent"),
        &[
            "Released",
            "Redirected",
            "limbo-kick",
            "Disconnected",
            "TimedOut",
            "reason: Shutdown",
        ],
    );

    let mut settings = ClientSettings::new("ja_jp");
    settings.view_distance = 2;
    settings.chat_mode = ChatMode::Hidden;
    settings.chat_colors = false;
    settings.skin_parts = SkinParts::new(SkinParts::CAPE | SkinParts::HAT);
    settings.main_hand = MainHand::Left;
    settings.text_filtering = true;
    settings.allow_listing = false;
    settings.particle_status = ParticleStatus::Minimal;
    bus.fire(PlayerSettingsChangedEvent::new(steve(), settings))
        .await;
    let settings_dump = dump.dumped("PlayerSettingsChangedEvent");
    for needle in [
        "ja_jp",
        "view_distance: 2",
        "chat_mode: Hidden",
        "chat_colors: false",
        "SkinParts(CAPE | HAT)",
        "main_hand: Left",
        "text_filtering: true",
        "allow_listing: false",
        "particle_status: Minimal",
    ] {
        assert!(
            settings_dump[0].contains(needle),
            "expected {needle:?} in {}",
            settings_dump[0]
        );
    }

    bus.fire(PlayerChannelRegisterEvent::new(
        steve(),
        vec!["a:b".to_owned()],
        PacketDirection::Clientbound,
    ))
    .await;
    expect_each(&dump.dumped("PlayerChannelRegisterEvent"), &["Clientbound"]);

    bus.fire(PluginMessageEvent::new(
        steve(),
        Endpoint::Client,
        ChannelId::legacy("OldChan").unwrap(),
        "OldChan".to_owned(),
        Bytes::from_static(&[0xff, 0x00, 0xfe]),
        MessagePhase::Configuration,
    ))
    .await;
    expect_each(&dump.dumped("PluginMessageEvent"), &["Configuration"]);
    let message = &dump.dumped("PluginMessageEvent")[0];
    for needle in ["Client", "OldChan", "[255, 0, 254]"] {
        assert!(message.contains(needle), "expected {needle:?} in {message}");
    }

    for (target, source) in [
        (
            BanTarget::Ip("198.51.100.9".parse().unwrap()),
            BanSource::System,
        ),
        (
            BanTarget::IpRange("2001:db8::/32".parse().unwrap()),
            BanSource::WebApi { actor: None },
        ),
        (
            BanTarget::Uuid(uuid::Uuid::from_u128(42)),
            BanSource::Plugin("moderation".to_owned()),
        ),
    ] {
        let entry =
            BanEntry::new("ban-x", target, source.clone()).lasting(Duration::from_secs(3600));
        bus.fire(BanIssuedEvent::new(entry.clone(), source.clone(), false))
            .await;
        bus.fire(BanRevokedEvent::new(entry, source, true)).await;
    }
    let issued = dump.dumped("BanIssuedEvent");
    expect_each(&issued, &["198.51.100.9", "2001:db8::/32", "moderation"]);
    for line in &issued {
        assert!(
            line.contains("expires_at: Some"),
            "expected an expiry in {line}"
        );
    }
    expect_each(
        &dump.dumped("BanRevokedEvent"),
        &["silent: true", "WebApi(None)", "Uuid("],
    );

    for status in [
        ResourcePackStatus::Discarded,
        ResourcePackStatus::FailedReload,
        ResourcePackStatus::InvalidUrl,
    ] {
        bus.fire(PlayerResourcePackStatusEvent::new(
            steve(),
            None,
            status,
            ResourcePackOrigin::Backend,
        ))
        .await;
    }
    expect_each(
        &dump.dumped("PlayerResourcePackStatusEvent"),
        &["Discarded", "FailedReload", "InvalidUrl"],
    );

    for (old, new) in [
        (ServerState::Sleeping, ServerState::Crashed),
        (ServerState::Stopping, ServerState::Offline),
    ] {
        bus.fire(ServerStateChangeEvent::new(ServerId::new("s"), old, new))
            .await;
    }
    expect_each(
        &dump.dumped("ServerStateChangeEvent"),
        &["Crashed", "Offline"],
    );

    for state in [
        BackendState::Healthy,
        BackendState::Probing,
        BackendState::Unhealthy,
    ] {
        bus.fire(BackendHealthEvent::new(
            ServerAddress {
                host: "::1".to_owned(),
                port: 1,
            },
            vec![],
            state,
        ))
        .await;
    }
    expect_each(
        &dump.dumped("BackendHealthEvent"),
        &["Healthy", "Probing", "Unhealthy"],
    );

    bus.fire(PreTransferEvent::new(
        steve(),
        "x.example".to_owned(),
        1,
        TransferOrigin::Plugin,
    ))
    .await;
    expect_each(&dump.dumped("PreTransferEvent"), &["origin: Plugin"]);

    let mut response = PingResponse::new(
        Component::text("motd"),
        20,
        3,
        ProtocolVersion::new(767),
        "Infrarust".to_owned(),
        Some("data:image/png;base64,AAAA".to_owned()),
    );
    response.player_sample = vec![("Alex".to_owned(), uuid::Uuid::from_u128(5))];
    bus.fire(ProxyPingEvent::new(
        "203.0.113.7:40000".parse().unwrap(),
        None,
        None,
        ProtocolVersion::new(47),
        true,
        response,
    ))
    .await;
    let ping = &dump.dumped("ProxyPingEvent")[0];
    for needle in ["legacy: true", "AAAA", "Alex", "protocol: 47"] {
        assert!(ping.contains(needle), "expected {needle:?} in {ping}");
    }

    bus.fire(GameProfileRequestEvent::new(
        steve().profile().clone(),
        true,
        "203.0.113.7:40000".parse().unwrap(),
        None,
        ProtocolVersion::new(767),
    ))
    .await;
    let profile = &dump.dumped("GameProfileRequestEvent")[0];
    assert_eq!(profile.matches("c2lnbmVk").count(), 2, "{profile}");

    bus.fire(NamedEvent::new(
        "bin",
        "",
        Bytes::from_static(&[0, 159, 146, 150]),
    ))
    .await;
    let named = &dump.dumped("NamedEvent")[0];
    assert!(named.contains("[0, 159, 146, 150]"), "{named}");
}

#[tokio::test(flavor = "multi_thread")]
async fn rare_results_set_by_a_wasm_listener_apply_like_a_native_listeners() {
    let dump = Dump::with("answer", |env| {
        let bus: &dyn EventBus = &*env.event_bus;
        bus.subscribe::<PermissionsSetupEvent, _>(EventPriority::EARLY, |event| {
            event.set_result(PermissionsSetupResult::Custom(Arc::new(
                PermissionMap::new().with("x", true),
            )));
        });
    })
    .await;
    let bus = dump.bus();

    let chat = bus
        .fire(ChatMessageEvent::new(steve(), "hi".to_owned(), false, None))
        .await;
    assert!(
        matches!(chat.result(), ChatMessageResult::Deny { reason: None }),
        "{:?}",
        chat.result()
    );

    let command = bus
        .fire(CommandExecuteEvent::new(
            steve(),
            "spawn".to_owned(),
            false,
            None,
        ))
        .await;
    assert!(
        matches!(
            command.result(),
            CommandExecuteResult::Deny { reason: None }
        ),
        "{:?}",
        command.result()
    );

    let handshake = bus
        .fire(ConnectionHandshakeEvent::new(
            "203.0.113.7:40000".parse().unwrap(),
            HandshakeIntent::Login,
            ProtocolVersion::new(767),
        ))
        .await;
    assert!(
        matches!(
            handshake.result(),
            ConnectionHandshakeResult::Deny { reason: None }
        ),
        "{:?}",
        handshake.result()
    );

    let kicked = bus
        .fire(KickedFromServerEvent::new(
            steve(),
            ServerId::new("survival"),
            None,
            KickCause::PlayDisconnect,
            false,
            None,
            KickedFromServerResult::Notify {
                message: Component::text("n"),
            },
        ))
        .await;
    assert!(
        matches!(
            kicked.result(),
            KickedFromServerResult::DisconnectPlayer { reason: None }
        ),
        "{:?}",
        kicked.result()
    );

    let ping = bus
        .fire(ProxyPingEvent::new(
            "203.0.113.7:40000".parse().unwrap(),
            None,
            None,
            ProtocolVersion::new(767),
            false,
            PingResponse::new(
                Component::text("motd"),
                20,
                3,
                ProtocolVersion::new(767),
                "Infrarust".to_owned(),
                Some("data:image/png;base64,AAAA".to_owned()),
            ),
        ))
        .await;
    let response = &ping.response;
    assert_eq!(
        (
            response.max_players,
            response.online_players,
            response.protocol_version.raw(),
            response.version_name.as_str(),
            response.favicon.as_deref(),
            response.player_sample.clone(),
            response.description.to_plain(),
        ),
        (
            -1,
            i32::MAX,
            5,
            "",
            None,
            vec![("Zed".to_owned(), uuid::Uuid::from_u128(9))],
            "motd".to_owned()
        )
    );

    let request = bus
        .fire(GameProfileRequestEvent::new(
            steve().profile().clone(),
            true,
            "203.0.113.7:40000".parse().unwrap(),
            None,
            ProtocolVersion::new(767),
        ))
        .await;
    assert_eq!(request.profile.uuid, uuid::Uuid::from_u128(99));
    assert!(request.profile.properties.is_empty());
    assert_eq!(request.profile.username, "Steve");
    assert_eq!(request.original().uuid, uuid::Uuid::from_u128(7));
    assert_eq!(request.original().properties.len(), 1);

    let setup = bus.fire(PermissionsSetupEvent::new(steve(), true)).await;
    assert!(
        matches!(setup.result(), PermissionsSetupResult::UseDefault),
        "use_default resets a native Custom checker"
    );
}
