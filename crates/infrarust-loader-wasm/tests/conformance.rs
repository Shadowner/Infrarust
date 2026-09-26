#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use infrarust_api::types::Component;

use support::conformance::{Scenario, ping_fields, seen, seen_with};
use support::script::{self, Action, Directive, EventName as E};

const FIRST: u8 = 0;
const EARLY: u8 = 64;
const NORMAL: u8 = 128;
const LATE: u8 = 192;
const LAST: u8 = 255;
const CHAT_INTERCEPT: &str = "chat-intercept";
const PLUGIN_MESSAGING: &str = "plugin-messaging";
const RAW_PACKET: &str = "raw-packet";

macro_rules! conformance {
    ($native:ident, $wasm:ident, $scenario:expr $(,)?) => {
        const _: () = assert!(
            support::conformance::paired(stringify!($native), stringify!($wasm)),
            "conformance! test names must be <name>_native and <name>_wasm"
        );

        #[tokio::test(flavor = "multi_thread")]
        async fn $native() {
            support::conformance::check_native($scenario).await;
        }

        #[cfg(all(feature = "wasm", wasm_fixtures_available))]
        #[tokio::test(flavor = "multi_thread")]
        async fn $wasm() {
            support::conformance::check_wasm($scenario).await;
        }
    };
}

fn default_result(event: E) -> &'static str {
    match event {
        E::PreLogin | E::ServerPreConnect | E::PlayerChooseInitialServer => "allowed",
        E::PermissionsSetup => "use-default",
        E::KickedFromServer => "disconnect",
        E::ProxyPing => "ping:A Minecraft Proxy v2",
        E::ChatMessage => "allow",
        E::Login | E::PreTransfer => "allowed",
        E::GameProfileRequest => "profile:Steve",
        E::CommandExecute | E::ConnectionHandshake => "allow",
        E::PluginMessage => "forward",
        E::NamedEvent => "named:false:-",
        E::RawPacket => "pass",
        _ => "none",
    }
}

fn grants_for(scenario: Scenario, id: &'static str, event: E) -> Scenario {
    match event {
        E::ChatMessage | E::CommandExecute => scenario.grant(id, CHAT_INTERCEPT),
        E::PluginMessage => scenario.grant(id, PLUGIN_MESSAGING),
        E::RawPacket => scenario.grant(id, RAW_PACKET),
        _ => scenario,
    }
}

fn single(line: &str, expected: &str) -> Scenario {
    let directives = script::parse(line).expect("a valid script line");
    let Some(&Directive::On {
        event, priority, ..
    }) = directives.first()
    else {
        panic!("expected an `on` directive: {line}");
    };
    grants_for(
        Scenario::new().plugin("scripted", [line]),
        "scripted",
        event,
    )
    .fire(event, expected)
    .log("scripted", [seen(event, priority)])
}

fn record_only(event: E) -> Scenario {
    single(
        &format!("on {} normal record", event.as_str()),
        default_result(event),
    )
}

fn every_event_late_record() -> Scenario {
    let script = E::ALL.map(|event| format!("on {} late record", event.as_str()));
    let mut scenario = Scenario::new()
        .plugin("scripted", script)
        .grant("scripted", CHAT_INTERCEPT)
        .grant("scripted", PLUGIN_MESSAGING)
        .grant("scripted", RAW_PACKET);
    for event in E::ALL {
        scenario = scenario.fire(event, default_result(event));
    }
    scenario.log("scripted", E::ALL.map(|event| seen(event, LATE)))
}

conformance!(
    pre_login_deny_native,
    pre_login_deny_wasm,
    single("on pre-login early deny \"Banned\"", "denied:Banned"),
);

conformance!(
    pre_login_force_offline_native,
    pre_login_force_offline_wasm,
    single("on pre-login normal force-offline", "force-offline"),
);

conformance!(
    pre_login_force_online_native,
    pre_login_force_online_wasm,
    single("on pre-login normal force-online", "force-online"),
);

conformance!(
    server_pre_connect_connect_to_native,
    server_pre_connect_connect_to_wasm,
    single(
        "on server-pre-connect normal connect-to backend-2",
        "connect-to:backend-2"
    ),
);

conformance!(
    server_pre_connect_deny_native,
    server_pre_connect_deny_wasm,
    single("on server-pre-connect normal deny \"no\"", "denied:no"),
);

conformance!(
    server_pre_connect_limbo_native,
    server_pre_connect_limbo_wasm,
    single(
        "on server-pre-connect normal limbo gate,queue",
        "limbo:gate,queue"
    ),
);

conformance!(
    player_choose_initial_server_redirect_native,
    player_choose_initial_server_redirect_wasm,
    single(
        "on player-choose-initial-server normal redirect lobby",
        "redirect:lobby"
    ),
);

conformance!(
    player_choose_initial_server_limbo_native,
    player_choose_initial_server_limbo_wasm,
    single(
        "on player-choose-initial-server normal limbo gate",
        "limbo:gate"
    ),
);

conformance!(
    kicked_from_server_redirect_native,
    kicked_from_server_redirect_wasm,
    single(
        "on kicked-from-server normal redirect fallback",
        "redirect:fallback"
    ),
);

conformance!(
    kicked_from_server_notify_native,
    kicked_from_server_notify_wasm,
    single("on kicked-from-server normal notify \"msg\"", "notify:msg"),
);

conformance!(
    kicked_from_server_disconnect_native,
    kicked_from_server_disconnect_wasm,
    single(
        "on kicked-from-server normal disconnect \"bye\"",
        "disconnect:bye"
    ),
);

conformance!(
    kicked_from_server_limbo_native,
    kicked_from_server_limbo_wasm,
    single("on kicked-from-server normal limbo gate", "limbo:gate"),
);

conformance!(
    chat_message_deny_native,
    chat_message_deny_wasm,
    single("on chat-message normal deny \"muted\"", "deny:muted"),
);

conformance!(
    chat_message_modify_native,
    chat_message_modify_wasm,
    single("on chat-message normal modify \"[x] hi\"", "modify:[x] hi"),
);

conformance!(
    proxy_ping_description_native,
    proxy_ping_description_wasm,
    single("on proxy-ping normal description \"hello\"", "ping:hello"),
);

conformance!(
    permissions_setup_custom_native,
    permissions_setup_custom_wasm,
    single("on permissions-setup normal custom admin", "custom:admin"),
);

conformance!(
    permissions_setup_custom_player_native,
    permissions_setup_custom_player_wasm,
    single("on permissions-setup normal custom player", "custom:player"),
);

conformance!(
    permissions_setup_custom_survives_a_later_listener_native,
    permissions_setup_custom_survives_a_later_listener_wasm,
    Scenario::new()
        .plugin(
            "scripted",
            [
                "on permissions-setup early custom admin",
                "on permissions-setup late record",
            ]
        )
        .fire(E::PermissionsSetup, "custom:admin")
        .log(
            "scripted",
            [
                seen(E::PermissionsSetup, EARLY),
                seen(E::PermissionsSetup, LATE)
            ]
        ),
);

conformance!(
    pre_login_record_native,
    pre_login_record_wasm,
    record_only(E::PreLogin)
);
conformance!(
    post_login_record_native,
    post_login_record_wasm,
    record_only(E::PostLogin)
);
conformance!(
    disconnect_record_native,
    disconnect_record_wasm,
    record_only(E::Disconnect)
);
conformance!(
    online_auth_failed_record_native,
    online_auth_failed_record_wasm,
    record_only(E::OnlineAuthFailed)
);
conformance!(
    permissions_setup_record_native,
    permissions_setup_record_wasm,
    record_only(E::PermissionsSetup)
);
conformance!(
    server_pre_connect_record_native,
    server_pre_connect_record_wasm,
    record_only(E::ServerPreConnect)
);
conformance!(
    server_connected_record_native,
    server_connected_record_wasm,
    record_only(E::ServerConnected)
);
conformance!(
    server_post_connect_record_native,
    server_post_connect_record_wasm,
    record_only(E::ServerPostConnect)
);
conformance!(
    kicked_from_server_record_native,
    kicked_from_server_record_wasm,
    record_only(E::KickedFromServer)
);
conformance!(
    player_choose_initial_server_record_native,
    player_choose_initial_server_record_wasm,
    record_only(E::PlayerChooseInitialServer)
);
conformance!(
    proxy_ping_record_native,
    proxy_ping_record_wasm,
    record_only(E::ProxyPing)
);
conformance!(
    proxy_initialize_record_native,
    proxy_initialize_record_wasm,
    record_only(E::ProxyInitialize)
);
conformance!(
    proxy_shutdown_record_native,
    proxy_shutdown_record_wasm,
    record_only(E::ProxyShutdown)
);
conformance!(
    config_reload_record_native,
    config_reload_record_wasm,
    record_only(E::ConfigReload)
);
conformance!(
    server_state_change_record_native,
    server_state_change_record_wasm,
    record_only(E::ServerStateChange)
);
conformance!(
    chat_message_record_native,
    chat_message_record_wasm,
    record_only(E::ChatMessage)
);
conformance!(
    backend_health_record_native,
    backend_health_record_wasm,
    record_only(E::BackendHealth)
);

conformance!(
    login_record_native,
    login_record_wasm,
    record_only(E::Login)
);
conformance!(
    game_profile_request_record_native,
    game_profile_request_record_wasm,
    record_only(E::GameProfileRequest)
);
conformance!(
    command_execute_record_native,
    command_execute_record_wasm,
    record_only(E::CommandExecute)
);
conformance!(
    connection_handshake_record_native,
    connection_handshake_record_wasm,
    record_only(E::ConnectionHandshake)
);
conformance!(
    connection_rejected_record_native,
    connection_rejected_record_wasm,
    record_only(E::ConnectionRejected)
);
conformance!(
    limbo_enter_record_native,
    limbo_enter_record_wasm,
    record_only(E::LimboEnter)
);
conformance!(
    limbo_exit_record_native,
    limbo_exit_record_wasm,
    record_only(E::LimboExit)
);
conformance!(
    player_client_brand_record_native,
    player_client_brand_record_wasm,
    record_only(E::PlayerClientBrand)
);
conformance!(
    player_settings_changed_record_native,
    player_settings_changed_record_wasm,
    record_only(E::PlayerSettingsChanged)
);
conformance!(
    player_channel_register_record_native,
    player_channel_register_record_wasm,
    record_only(E::PlayerChannelRegister)
);
conformance!(
    plugin_message_record_native,
    plugin_message_record_wasm,
    record_only(E::PluginMessage)
);
conformance!(
    ban_issued_record_native,
    ban_issued_record_wasm,
    record_only(E::BanIssued)
);
conformance!(
    ban_revoked_record_native,
    ban_revoked_record_wasm,
    record_only(E::BanRevoked)
);
conformance!(
    plugin_enabled_record_native,
    plugin_enabled_record_wasm,
    record_only(E::PluginEnabled)
);
conformance!(
    plugin_disabled_record_native,
    plugin_disabled_record_wasm,
    record_only(E::PluginDisabled)
);
conformance!(
    pre_transfer_record_native,
    pre_transfer_record_wasm,
    record_only(E::PreTransfer)
);
conformance!(
    player_resource_pack_status_record_native,
    player_resource_pack_status_record_wasm,
    record_only(E::PlayerResourcePackStatus)
);
conformance!(
    named_event_record_native,
    named_event_record_wasm,
    record_only(E::NamedEvent)
);
conformance!(
    raw_packet_record_native,
    raw_packet_record_wasm,
    record_only(E::RawPacket)
);

conformance!(
    login_deny_native,
    login_deny_wasm,
    single("on login early deny \"Closed\"", "denied:Closed"),
);

conformance!(
    game_profile_request_rename_native,
    game_profile_request_rename_wasm,
    single("on game-profile-request normal rename Alex", "profile:Alex"),
);

conformance!(
    command_execute_deny_native,
    command_execute_deny_wasm,
    single("on command-execute normal deny \"no\"", "deny:no"),
);

conformance!(
    command_execute_modify_native,
    command_execute_modify_wasm,
    single("on command-execute normal modify \"hub\"", "modify:hub"),
);

conformance!(
    command_execute_forward_native,
    command_execute_forward_wasm,
    single(
        "on command-execute normal forward-to-backend",
        "forward-to-backend"
    ),
);

conformance!(
    connection_handshake_deny_native,
    connection_handshake_deny_wasm,
    single("on connection-handshake normal deny \"bots\"", "deny:bots"),
);

conformance!(
    connection_handshake_drop_native,
    connection_handshake_drop_wasm,
    single("on connection-handshake normal drop", "drop"),
);

conformance!(
    plugin_message_handled_native,
    plugin_message_handled_wasm,
    single("on plugin-message normal handled", "handled"),
);

conformance!(
    plugin_message_replace_native,
    plugin_message_replace_wasm,
    single(
        "on plugin-message normal replace \"swapped\"",
        "replace:swapped"
    ),
);

conformance!(
    pre_transfer_deny_native,
    pre_transfer_deny_wasm,
    single("on pre-transfer normal deny \"stay\"", "denied:stay"),
);

conformance!(
    pre_transfer_redirect_native,
    pre_transfer_redirect_wasm,
    single(
        "on pre-transfer normal redirect new.example.com:25566",
        "redirect:new.example.com:25566"
    ),
);

conformance!(
    named_event_cancel_native,
    named_event_cancel_wasm,
    single("on named-event normal cancel", "named:true:-"),
);

conformance!(
    named_event_respond_native,
    named_event_respond_wasm,
    single(
        "on named-event normal respond \"pong\"",
        "named:false:text/plain=pong"
    ),
);

conformance!(
    raw_packet_drop_native,
    raw_packet_drop_wasm,
    single("on raw-packet normal drop", "drop"),
);

conformance!(
    raw_packet_modify_native,
    raw_packet_modify_wasm,
    single("on raw-packet normal modify \"xyz\"", "modify:5:xyz"),
);

conformance!(
    named_subscription_filters_by_name_native,
    named_subscription_filters_by_name_wasm,
    Scenario::new()
        .plugin(
            "scripted",
            [
                "named echo late respond \"pong\"",
                "named other first cancel",
            ],
        )
        .fire(E::NamedEvent, "named:false:text/plain=pong")
        .log("scripted", ["named echo @192 - text/plain ping false -"],),
);

conformance!(
    named_event_round_trip_across_plugins_native,
    named_event_round_trip_across_plugins_wasm,
    Scenario::new()
        .plugin("scripted-peer", ["named echo late respond \"pong\""])
        .plugin("scripted", ["cmd ask fire echo \"ping\""])
        .command("ask")
        .log("scripted", ["cmd ask fired echo false pong"])
        .log(
            "scripted-peer",
            ["named echo @192 scripted text/plain ping false -"],
        ),
);

conformance!(
    named_event_fired_to_its_own_plugin_native,
    named_event_fired_to_its_own_plugin_wasm,
    Scenario::new()
        .plugin(
            "scripted",
            [
                "named echo late respond \"pong\"",
                "cmd ask fire echo \"ping\""
            ],
        )
        .command("ask")
        .disable()
        .log(
            "scripted",
            [
                "named echo @192 scripted text/plain ping false -",
                "cmd ask fired echo false pong",
                "disable",
            ],
        )
        .wasm_log(
            "scripted",
            [
                "cmd ask fired echo false -",
                "named echo @192 scripted text/plain ping false -",
                "disable",
            ],
        )
        .expect_divergence(
            "a WASM plugin that fires a named event it listens to itself is still running its \
             command when the event reaches its own listener; the host never re-enters a busy \
             instance, so the listener gets the event right after the command returns and its \
             answer is not part of the result the command saw"
        ),
);

conformance!(
    later_result_resets_new_events_native,
    later_result_resets_new_events_wasm,
    Scenario::new()
        .plugin(
            "scripted",
            [
                "on login early deny \"Closed\"",
                "on command-execute early deny \"no\"",
                "on connection-handshake early drop",
                "on plugin-message early handled",
                "on pre-transfer early deny \"stay\"",
                "on raw-packet early drop",
            ],
        )
        .plugin(
            "scripted-peer",
            [
                "on login late allow",
                "on command-execute late allow",
                "on connection-handshake late allow",
                "on plugin-message late forward",
                "on pre-transfer late allow",
                "on raw-packet late pass",
            ],
        )
        .grant("scripted", CHAT_INTERCEPT)
        .grant("scripted-peer", CHAT_INTERCEPT)
        .grant("scripted", PLUGIN_MESSAGING)
        .grant("scripted-peer", PLUGIN_MESSAGING)
        .grant("scripted", RAW_PACKET)
        .grant("scripted-peer", RAW_PACKET)
        .fire(E::Login, "allowed")
        .fire(E::CommandExecute, "allow")
        .fire(E::ConnectionHandshake, "allow")
        .fire(E::PluginMessage, "forward")
        .fire(E::PreTransfer, "allowed")
        .fire(E::RawPacket, "pass")
        .log(
            "scripted",
            [
                seen(E::Login, EARLY),
                seen(E::CommandExecute, EARLY),
                seen(E::ConnectionHandshake, EARLY),
                seen(E::PluginMessage, EARLY),
                seen(E::PreTransfer, EARLY),
                seen(E::RawPacket, EARLY),
            ],
        )
        .log(
            "scripted-peer",
            [
                seen(E::Login, LATE),
                seen(E::CommandExecute, LATE),
                seen(E::ConnectionHandshake, LATE),
                seen(E::PluginMessage, LATE),
                seen(E::PreTransfer, LATE),
                seen(E::RawPacket, LATE),
            ],
        ),
);

conformance!(
    every_event_late_record_native,
    every_event_late_record_wasm,
    every_event_late_record()
);

conformance!(
    priority_order_within_plugin_native,
    priority_order_within_plugin_wasm,
    Scenario::new()
        .plugin(
            "scripted",
            [
                "on post-login last record",
                "on post-login normal cancelled",
                "on post-login normal record",
                "on post-login 32 record",
                "on post-login first record",
            ],
        )
        .fire(E::PostLogin, "none")
        .fire(E::PostLogin, "none")
        .log(
            "scripted",
            [FIRST, 32, NORMAL, LAST, FIRST, 32, NORMAL, LAST].map(|p| seen(E::PostLogin, p)),
        ),
);

conformance!(
    priority_order_across_plugins_native,
    priority_order_across_plugins_wasm,
    Scenario::new()
        .plugin(
            "scripted-peer",
            [
                "on chat-message late modify \"late\"",
                "on proxy-ping late record",
            ],
        )
        .plugin(
            "scripted",
            [
                "on chat-message early modify \"early\"",
                "on proxy-ping early description \"from-early\"",
            ],
        )
        .grant("scripted", CHAT_INTERCEPT)
        .grant("scripted-peer", CHAT_INTERCEPT)
        .fire(E::ChatMessage, "modify:late")
        .fire(E::ProxyPing, "ping:from-early")
        .log(
            "scripted",
            [seen(E::ChatMessage, EARLY), seen(E::ProxyPing, EARLY)],
        )
        .log(
            "scripted-peer",
            [
                seen(E::ChatMessage, LATE),
                seen_with(
                    E::ProxyPing,
                    LATE,
                    &ping_fields(&Component::text("from-early").to_json()),
                ),
            ],
        ),
);

conformance!(
    later_allow_resets_result_native,
    later_allow_resets_result_wasm,
    Scenario::new()
        .plugin(
            "scripted",
            [
                "on pre-login early deny \"Banned\"",
                "on server-pre-connect early connect-to backend-2",
                "on player-choose-initial-server early redirect lobby",
                "on chat-message early deny \"muted\"",
            ],
        )
        .plugin(
            "scripted-peer",
            [
                "on pre-login late allow",
                "on server-pre-connect late allow",
                "on player-choose-initial-server late allow",
                "on chat-message late allow",
            ],
        )
        .grant("scripted", CHAT_INTERCEPT)
        .grant("scripted-peer", CHAT_INTERCEPT)
        .fire(E::PreLogin, "allowed")
        .fire(E::ServerPreConnect, "allowed")
        .fire(E::PlayerChooseInitialServer, "allowed")
        .fire(E::ChatMessage, "allow")
        .log(
            "scripted",
            [
                seen(E::PreLogin, EARLY),
                seen(E::ServerPreConnect, EARLY),
                seen(E::PlayerChooseInitialServer, EARLY),
                seen(E::ChatMessage, EARLY),
            ],
        )
        .log(
            "scripted-peer",
            [
                seen(E::PreLogin, LATE),
                seen(E::ServerPreConnect, LATE),
                seen(E::PlayerChooseInitialServer, LATE),
                seen(E::ChatMessage, LATE),
            ],
        ),
);

conformance!(
    panicking_handler_native,
    panicking_handler_wasm,
    Scenario::new()
        .plugin(
            "scripted",
            [
                "on pre-login normal panic",
                "on chat-message normal modify \"alive\"",
            ],
        )
        .plugin("scripted-peer", ["on chat-message late record"])
        .grant("scripted", CHAT_INTERCEPT)
        .grant("scripted-peer", CHAT_INTERCEPT)
        .fire(E::PreLogin, "allowed")
        .fire(E::ChatMessage, "modify:alive")
        .disable()
        .log(
            "scripted",
            [
                seen(E::PreLogin, NORMAL),
                seen(E::ChatMessage, NORMAL),
                "disable".to_owned(),
            ],
        )
        .wasm_log(
            "scripted",
            [
                seen(E::PreLogin, NORMAL),
                "enable recovered 1".to_owned(),
                seen(E::ChatMessage, NORMAL),
                "disable".to_owned(),
            ],
        )
        .log(
            "scripted-peer",
            [seen(E::ChatMessage, LATE), "disable".to_owned()],
        )
        .expect_divergence(
            "a native panic is contained to the one handler call, but a WASM trap discards the \
             instance: a fresh one runs on_enable again, told it is a recovery, before it \
             handles the later events, so the WASM log shows a second enable"
        ),
);

conformance!(
    command_record_native,
    command_record_wasm,
    Scenario::new()
        .plugin("scripted", ["cmd greet record"])
        .command("greet world peace")
        .log("scripted", ["cmd greet world,peace -"]),
);

conformance!(
    lifecycle_disable_native,
    lifecycle_disable_wasm,
    Scenario::new()
        .plugin("scripted", ["on post-login normal record"])
        .fire(E::PostLogin, "none")
        .disable()
        .log(
            "scripted",
            [seen(E::PostLogin, NORMAL), "disable".to_owned()],
        ),
);

#[test]
fn script_parser_accepts_the_grammar_and_rejects_mistakes() {
    let parsed = script::parse(
        "on pre-login early deny \"Banned\"\n\ncmd greet record\non post-login 32 cancelled\n\
         named echo late respond \"pong\"\ncmd ask fire echo \"ping\"\nchannel test:echo\n\
         config keepalive.retries\nplugin auth",
    )
    .expect("valid script");
    assert_eq!(
        parsed,
        [
            Directive::On {
                event: E::PreLogin,
                priority: EARLY,
                action: Action::Deny("Banned".to_owned()),
            },
            Directive::Cmd {
                name: "greet".to_owned(),
            },
            Directive::On {
                event: E::PostLogin,
                priority: 32,
                action: Action::Cancelled,
            },
            Directive::Named {
                name: "echo".to_owned(),
                priority: LATE,
                action: Action::Respond("pong".to_owned()),
            },
            Directive::Fire {
                command: "ask".to_owned(),
                event: "echo".to_owned(),
                payload: "ping".to_owned(),
            },
            Directive::Channel {
                id: "test:echo".to_owned(),
            },
            Directive::Config {
                key: "keepalive.retries".to_owned(),
            },
            Directive::Plugin {
                id: "auth".to_owned(),
            },
        ]
    );
    for bad in [
        "on nope normal record",
        "on pre-login sometimes record",
        "on post-login normal deny \"x\"",
        "on chat-message normal modify",
        "on pre-login normal record extra",
        "on permissions-setup normal custom root",
        "cmd greet panic",
        "off pre-login normal record",
        "on login normal drop",
        "on raw-packet normal respond \"x\"",
        "named echo normal drop",
        "cmd ask fire",
        "channel",
        "config",
        "config web.bind extra",
        "plugin",
        "plugin auth extra",
    ] {
        assert!(script::parse(bad).is_err(), "`{bad}` must be rejected");
    }
}
