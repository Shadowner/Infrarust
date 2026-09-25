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
        _ => "none",
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
    let scenario = Scenario::new().plugin("scripted", [line]);
    let scenario = if event == E::ChatMessage {
        scenario.grant("scripted", CHAT_INTERCEPT)
    } else {
        scenario
    };
    scenario
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
        .grant("scripted", CHAT_INTERCEPT);
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
    Scenario::new()
        .plugin("scripted", ["on permissions-setup normal custom admin"])
        .fire_diverging(E::PermissionsSetup, "custom:admin", "use-default")
        .log("scripted", [seen(E::PermissionsSetup, NORMAL)])
        .expect_divergence(
            "the SDK exposes permissions-setup as observe-only and the loader drops any \
             permissions-setup outcome, so a WASM guest cannot install a custom checker"
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
    server_switch_record_native,
    server_switch_record_wasm,
    record_only(E::ServerSwitch)
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
        .fire_diverging(E::PreLogin, "allowed", "denied:Banned")
        .fire_diverging(E::ServerPreConnect, "allowed", "connect-to:backend-2")
        .fire_diverging(E::PlayerChooseInitialServer, "allowed", "redirect:lobby")
        .fire_diverging(E::ChatMessage, "allow", "deny:muted")
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
        )
        .expect_divergence(
            "the SDK's allow() clears the guest's own result to EventOutcome::None, which the \
             host reads as no change, so a later WASM handler cannot reset an earlier result"
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
                "enable".to_owned(),
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
             instance: a fresh one runs on_enable again before it handles the later events, so \
             the WASM log shows a second enable"
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
        "on pre-login early deny \"Banned\"\n\ncmd greet record\non post-login 32 cancelled",
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
    ] {
        assert!(script::parse(bad).is_err(), "`{bad}` must be rejected");
    }
}
