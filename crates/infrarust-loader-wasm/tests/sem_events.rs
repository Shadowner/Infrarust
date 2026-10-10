#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;

use infrarust_api::event::bus::{EventBus, EventBusExt};
use infrarust_api::event::{EventPriority, ResultedEvent};
use infrarust_api::events::chat::{ChatMessageEvent, ChatMessageResult};
use infrarust_api::events::lifecycle::{PreLoginEvent, PreLoginResult};
use infrarust_api::loader::PluginLoader;
use infrarust_api::types::{Component, ProtocolVersion, ServerId};
use tracing::Level;
use tracing::instrument::WithSubscriber;

use support::log_capture::LogCapture;
use support::{EnvOptions, fresh_loader, load_enabled, make_env_with, stage};

const WRONG: &str = "wrong-outcome";

fn steve() -> Arc<dyn infrarust_api::player::Player> {
    support::session_player(
        1,
        support::nil_profile("Steve"),
        767,
        "203.0.113.7:40000".parse().unwrap(),
    )
}

fn pre_login() -> PreLoginEvent {
    PreLoginEvent::new(
        support::nil_profile("Steve"),
        "203.0.113.7:40000".parse().unwrap(),
        ProtocolVersion::new(767),
        "play.example.com".to_owned(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn an_outcome_for_another_event_is_ignored_with_a_warning_and_a_matching_one_applies() {
    let logs = LogCapture::at(Level::WARN);
    let (login, chat) = async {
        let (_tmp, plugins_dir) = stage(WRONG);
        let env = make_env_with(
            plugins_dir.clone(),
            EnvOptions::default().grant(WRONG, "chat-intercept"),
        );
        let bus: &dyn EventBus = &*env.event_bus;
        bus.subscribe::<PreLoginEvent, _>(EventPriority::EARLY, |event| {
            event.set_result(PreLoginResult::Denied {
                reason: Component::text("Banned"),
            });
        });
        let loader = fresh_loader();
        loader.discover(&plugins_dir).await.unwrap();
        let _plugin = load_enabled(&loader, &env.factory, WRONG).await;
        let login = env.event_bus.fire(pre_login()).await;
        let chat = env
            .event_bus
            .fire(ChatMessageEvent::new(
                steve(),
                "hello".to_owned(),
                false,
                Some(ServerId::new("lobby")),
            ))
            .await;
        (login, chat)
    }
    .with_subscriber(logs.clone())
    .await;

    assert!(
        matches!(login.result(), PreLoginResult::Denied { reason } if reason.to_plain() == "Banned"),
        "{:?}",
        login.result()
    );
    assert!(
        matches!(chat.result(), ChatMessageResult::Deny { reason: None }),
        "{:?}",
        chat.result()
    );
    let warned = logs.matching("outcome of another event");
    assert_eq!(warned.len(), 1, "{:?}", logs.lines());
    assert!(warned[0].contains("pre-login") && warned[0].contains("chat-message"));
}

#[tokio::test(flavor = "multi_thread")]
async fn every_limbo_entry_outcome_reaches_the_engine_as_the_guest_chose_it() {
    use infrarust_api::limbo::{HandlerResult, LimboEntryContext, LimboHandler, LimboOutcome};
    use infrarust_api::test_util::RecordingLimboSession;
    use infrarust_api::types::PlayerId;

    let (_tmp, plugins_dir) = stage("sem-probe");
    let data = plugins_dir.join("sem-probe");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("probe.txt"), "keeper").unwrap();
    let env = make_env_with(
        plugins_dir.clone(),
        EnvOptions::default().grant("sem-probe", "limbo"),
    );
    let loader = fresh_loader();
    loader.discover(&plugins_dir).await.unwrap();
    let _plugin = load_enabled(&loader, &env.factory, "sem-probe").await;
    let handlers = env.factory.context("sem-probe").limbo_handlers();
    let handler = |name: &str| -> Arc<dyn LimboHandler> {
        handlers
            .iter()
            .find(|handler| handler.name() == name)
            .cloned()
            .unwrap_or_else(|| panic!("{name} is registered"))
    };
    let session = RecordingLimboSession::new(
        PlayerId::new(4),
        support::nil_profile("Steve"),
        LimboEntryContext::InitialConnection {
            target_server: ServerId::new("hub"),
        },
    );
    let denied = handler("denier").on_player_enter(session.as_ref()).await;
    assert!(
        matches!(&denied, HandlerResult::Deny(reason) if reason.to_plain() == "no entry"),
        "{denied:?}"
    );
    let redirected = handler("redirector")
        .on_player_enter(session.as_ref())
        .await;
    assert!(
        matches!(&redirected, HandlerResult::Redirect(server) if server.as_str() == "hub"),
        "{redirected:?}"
    );
    let chained = handler("chainer").on_player_enter(session.as_ref()).await;
    assert!(
        matches!(&chained, HandlerResult::SendToLimbo(names) if names == &["keeper".to_owned()]),
        "{chained:?}"
    );
    let timed = handler("timed").on_player_enter(session.as_ref()).await;
    let HandlerResult::HoldWithTimeout { after, on_timeout } = timed else {
        panic!("a timed hold stays a timed hold: {timed:?}");
    };
    assert_eq!(after, std::time::Duration::from_millis(1500));
    assert!(
        matches!(on_timeout, LimboOutcome::Redirect(ref server) if server.as_str() == "fallback")
    );
}
