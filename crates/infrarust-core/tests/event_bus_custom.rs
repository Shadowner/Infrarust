#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::any::TypeId;
use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use infrarust_api::event::bus::{EventBus, EventBusExt, FireError};
use infrarust_api::event::{Event, EventPriority};
use infrarust_api::events::chat::ChatMessageEvent;
use infrarust_api::events::connection::{
    KickedFromServerEvent, PlayerChooseInitialServerEvent, ServerConnectedEvent,
    ServerPreConnectEvent, ServerSwitchEvent,
};
use infrarust_api::events::lifecycle::{
    DisconnectEvent, GameProfileRequestEvent, LoginEvent, OnlineAuthFailed, PermissionsSetupEvent,
    PostLoginEvent, PreLoginEvent,
};
use infrarust_api::events::named::{NamedEvent, NamedEventResponse};
use infrarust_api::events::packet::RawPacketEvent;
use infrarust_api::events::proxy::{
    BackendHealthEvent, ConfigReloadEvent, ProxyInitializeEvent, ProxyPingEvent,
    ProxyShutdownEvent, ServerStateChangeEvent,
};
use infrarust_api::types::{GameProfile, ProtocolVersion};
use infrarust_core::event_bus::{BUILTIN_EVENTS, DiagnosticKind, EventBusImpl, is_builtin_event};
use infrarust_core::plugin::tracking::TrackingEventBus;

struct Question {
    text: String,
    answer: Option<String>,
}
impl Event for Question {}

fn plugin(bus: &Arc<EventBusImpl>, id: &str) -> TrackingEventBus {
    TrackingEventBus::new(Arc::clone(bus), id)
}

fn count<E: Event>(bus: &dyn EventBus) -> Arc<AtomicUsize> {
    let count = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&count);
    bus.subscribe::<E, _>(EventPriority::FIRST, move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
    });
    count
}

fn pre_login() -> PreLoginEvent {
    PreLoginEvent::new(
        GameProfile {
            uuid: uuid::Uuid::nil(),
            username: "Forged".to_string(),
            properties: vec![],
        },
        SocketAddr::from(([127, 0, 0, 1], 25565)),
        ProtocolVersion::MINECRAFT_1_21,
        "lobby.test".to_string(),
    )
}

#[tokio::test]
async fn a_plugin_answers_another_plugins_custom_event() {
    let bus = Arc::new(EventBusImpl::new());
    let asker = plugin(&bus, "asker");
    let answerer = plugin(&bus, "answerer");
    let answerer_ref: &dyn EventBus = &answerer;
    answerer_ref.subscribe::<Question, _>(EventPriority::NORMAL, |question| {
        question.answer = Some(format!("you asked: {}", question.text));
    });
    let asker_ref: &dyn EventBus = &asker;

    let answered = asker_ref
        .fire(Question {
            text: "ping?".to_string(),
            answer: None,
        })
        .await
        .unwrap();

    assert_eq!(answered.answer.as_deref(), Some("you asked: ping?"));
}

#[tokio::test]
async fn a_custom_event_nobody_listens_to_comes_back_untouched() {
    let bus = Arc::new(EventBusImpl::new());
    let asker = plugin(&bus, "asker");
    let asker_ref: &dyn EventBus = &asker;

    let answered = asker_ref
        .fire(Question {
            text: "anyone?".to_string(),
            answer: None,
        })
        .await
        .unwrap();

    assert_eq!(answered.text, "anyone?");
    assert_eq!(answered.answer, None);
}

#[tokio::test]
async fn plugins_cannot_fire_builtin_events() {
    let bus = Arc::new(EventBusImpl::new());
    let forger = plugin(&bus, "forger");
    let forger_ref: &dyn EventBus = &forger;
    let pre_logins = count::<PreLoginEvent>(bus.as_ref());
    let reloads = count::<ConfigReloadEvent>(bus.as_ref());
    let shutdowns = count::<ProxyShutdownEvent>(bus.as_ref());

    let pre_login = forger_ref.fire(pre_login()).await;
    let reload = forger_ref.fire(ConfigReloadEvent).await;
    let shutdown = forger_ref.fire(ProxyShutdownEvent).await;

    assert_eq!(pre_login.err(), Some(FireError::Reserved));
    assert_eq!(reload.err(), Some(FireError::Reserved));
    assert_eq!(shutdown.err(), Some(FireError::Reserved));
    assert_eq!(pre_logins.load(Ordering::SeqCst), 0);
    assert_eq!(reloads.load(Ordering::SeqCst), 0);
    assert_eq!(shutdowns.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn the_generic_fire_path_rejects_builtin_events_on_the_core_bus_too() {
    let bus = Arc::new(EventBusImpl::new());
    let core_ref: &dyn EventBus = bus.as_ref();
    let pre_logins = count::<PreLoginEvent>(core_ref);

    let result = core_ref.fire(pre_login()).await;

    assert_eq!(result.err(), Some(FireError::Reserved));
    assert_eq!(pre_logins.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn named_events_round_trip_with_the_firing_plugin_stamped() {
    let bus = Arc::new(EventBusImpl::new());
    let asker = plugin(&bus, "asker");
    let answerer = plugin(&bus, "answerer");
    let answerer_ref: &dyn EventBus = &answerer;
    let seen_source = Arc::new(std::sync::Mutex::new(String::new()));
    let source_sink = Arc::clone(&seen_source);
    answerer_ref.subscribe::<NamedEvent, _>(EventPriority::NORMAL, move |event| {
        if event.name != "echo" {
            return;
        }
        source_sink.lock().unwrap().clone_from(&event.source_plugin);
        let mut reversed = event.payload.to_vec();
        reversed.reverse();
        event.respond("text/plain", reversed);
    });
    let asker_ref: &dyn EventBus = &asker;
    let mut outgoing = NamedEvent::new("echo", "text/plain", "hello");
    outgoing.source_plugin = "mallory".to_string();

    let answered = asker_ref.fire(outgoing).await.unwrap();

    assert_eq!(&*seen_source.lock().unwrap(), "asker");
    assert_eq!(answered.source_plugin, "asker");
    assert!(!answered.cancelled);
    assert_eq!(
        answered.response,
        Some(NamedEventResponse::new("text/plain", "olleh"))
    );
}

#[tokio::test]
async fn a_cancelled_named_event_reports_it_to_the_sender() {
    let bus = Arc::new(EventBusImpl::new());
    let gate = plugin(&bus, "gate");
    let gate_ref: &dyn EventBus = &gate;
    gate_ref.subscribe::<NamedEvent, _>(EventPriority::FIRST, NamedEvent::cancel);
    let sender = plugin(&bus, "sender");
    let sender_ref: &dyn EventBus = &sender;

    let answered = sender_ref
        .fire(NamedEvent::new("teleport", "application/json", "{}"))
        .await
        .unwrap();

    assert!(answered.cancelled);
    assert_eq!(answered.response, None);
}

#[tokio::test]
async fn a_failing_handler_names_the_plugin_that_fired_the_event() {
    let bus = Arc::new(EventBusImpl::new());
    let mut diagnostics = bus.diagnostics();
    let asker = plugin(&bus, "asker");
    let answerer = plugin(&bus, "answerer");
    let answerer_ref: &dyn EventBus = &answerer;
    answerer_ref.subscribe::<Question, _>(EventPriority::NORMAL, |_| panic!("no answer"));
    let asker_ref: &dyn EventBus = &asker;

    asker_ref
        .fire(Question {
            text: "why?".to_string(),
            answer: None,
        })
        .await
        .unwrap();

    let diagnostic = diagnostics.try_recv().unwrap();
    assert_eq!(&*diagnostic.owner, "answerer");
    assert_eq!(&*diagnostic.fired_by, "asker");
    assert_eq!(diagnostic.event, "Question");
    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Panicked {
            message: "no answer".to_string()
        }
    );
}

type Probe = (&'static str, fn() -> TypeId);

const BUILTIN_NAMES: [Probe; 19] = [
    ("PreLoginEvent", TypeId::of::<PreLoginEvent>),
    (
        "GameProfileRequestEvent",
        TypeId::of::<GameProfileRequestEvent>,
    ),
    ("LoginEvent", TypeId::of::<LoginEvent>),
    ("PostLoginEvent", TypeId::of::<PostLoginEvent>),
    ("PermissionsSetupEvent", TypeId::of::<PermissionsSetupEvent>),
    ("OnlineAuthFailed", TypeId::of::<OnlineAuthFailed>),
    ("DisconnectEvent", TypeId::of::<DisconnectEvent>),
    (
        "PlayerChooseInitialServerEvent",
        TypeId::of::<PlayerChooseInitialServerEvent>,
    ),
    ("ServerPreConnectEvent", TypeId::of::<ServerPreConnectEvent>),
    ("ServerConnectedEvent", TypeId::of::<ServerConnectedEvent>),
    ("ServerSwitchEvent", TypeId::of::<ServerSwitchEvent>),
    ("KickedFromServerEvent", TypeId::of::<KickedFromServerEvent>),
    ("ChatMessageEvent", TypeId::of::<ChatMessageEvent>),
    ("ProxyPingEvent", TypeId::of::<ProxyPingEvent>),
    ("ProxyInitializeEvent", TypeId::of::<ProxyInitializeEvent>),
    ("ProxyShutdownEvent", TypeId::of::<ProxyShutdownEvent>),
    ("ConfigReloadEvent", TypeId::of::<ConfigReloadEvent>),
    ("BackendHealthEvent", TypeId::of::<BackendHealthEvent>),
    (
        "ServerStateChangeEvent",
        TypeId::of::<ServerStateChangeEvent>,
    ),
];

const OPEN_API_EVENTS: [&str; 1] = ["NamedEvent"];

fn api_event_impls() -> BTreeSet<String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../infrarust-api/src/events");
    let mut names = BTreeSet::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }
        for line in std::fs::read_to_string(&path).unwrap().lines() {
            if let Some(rest) = line.trim().strip_prefix("impl Event for ") {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                names.insert(name);
            }
        }
    }
    names
}

#[test]
fn every_builtin_event_is_reserved() {
    for (name, type_id) in BUILTIN_NAMES {
        assert!(is_builtin_event(type_id()), "{name} is not reserved");
    }
    assert!(is_builtin_event(TypeId::of::<RawPacketEvent>()));
    assert_eq!(BUILTIN_EVENTS.len(), BUILTIN_NAMES.len() + 1);
}

#[test]
fn open_events_are_not_reserved() {
    assert!(!is_builtin_event(TypeId::of::<NamedEvent>()));
    assert!(!is_builtin_event(TypeId::of::<Question>()));
}

#[test]
fn the_reserved_list_names_every_event_the_api_defines() {
    let defined = api_event_impls();
    let listed: BTreeSet<String> = BUILTIN_NAMES
        .iter()
        .map(|(name, _)| (*name).to_string())
        .chain(OPEN_API_EVENTS.iter().map(|name| (*name).to_string()))
        .collect();

    assert_eq!(defined, listed);
}
