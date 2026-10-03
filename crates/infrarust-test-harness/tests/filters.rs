#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use bytes::Bytes;
use infrarust_api::event::BoxFuture;
use infrarust_api::filter::{
    CodecFilterError, CodecFilterFactory, CodecFilterInstance, CodecSessionInit, CodecVerdict,
    ConnectionSide, FilterMetadata, FilterRegistryError, FilterVerdict, FrameOutput,
    TransportContext, TransportFilter,
};
use infrarust_api::types::RawPacket;
use infrarust_core::filter::codec_chain::{CODEC_FILTER_FAILED, CODEC_FILTER_UNAVAILABLE};

use infrarust_test_harness::chat::ChatPacket;
use infrarust_test_harness::{
    DEFAULT_TIMEOUT, FakeBackend, ProtocolVersion, ScriptedPlugin, ServerSpec, TestProxy,
};

const T: Duration = DEFAULT_TIMEOUT;
const VERSION: ProtocolVersion = ProtocolVersion::V1_21;
const OWNER: &str = "rewriter";
const INTRUDER: &str = "intruder";
const REWRITE: &str = "rewrite";
const GATE: &str = "gate";
const MARKER: &str = "marker";
const SECRET: &[u8; 6] = b"secret";
const PUBLIC: &[u8; 6] = b"public";
const STOLEN: &[u8; 6] = b"stolen";

struct Rewrite {
    to: &'static [u8; 6],
}

impl CodecFilterFactory for Rewrite {
    fn metadata(&self) -> FilterMetadata {
        FilterMetadata::new(REWRITE)
    }

    fn create(&self, init: &CodecSessionInit) -> Box<dyn CodecFilterInstance> {
        Box::new(Rewriting {
            to: self.to,
            active: init.side == ConnectionSide::ClientSide,
        })
    }
}

struct Rewriting {
    to: &'static [u8; 6],
    active: bool,
}

impl CodecFilterInstance for Rewriting {
    fn filter(&mut self, packet: &mut RawPacket, _output: &mut FrameOutput) -> CodecVerdict {
        let found = packet
            .data
            .windows(SECRET.len())
            .position(|window| window == SECRET);
        if let (true, Some(at)) = (self.active, found) {
            let mut data = packet.data.to_vec();
            data[at..at + SECRET.len()].copy_from_slice(self.to);
            packet.data = Bytes::from(data);
        }
        CodecVerdict::Pass
    }
}

struct Gate;

impl TransportFilter for Gate {
    fn metadata(&self) -> FilterMetadata {
        FilterMetadata::new(GATE)
    }

    fn on_accept<'a>(&'a self, _ctx: &'a mut TransportContext) -> BoxFuture<'a, FilterVerdict> {
        Box::pin(async { FilterVerdict::Reject })
    }
}

fn owned_by_owner(id: &str) -> FilterRegistryError {
    FilterRegistryError::OwnedBy {
        id: id.to_owned(),
        owner: OWNER.to_owned(),
    }
}

async fn start(owner: ScriptedPlugin) -> (TestProxy, FakeBackend) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(owner)
        .plugin(ScriptedPlugin::new(INTRUDER).after(OWNER))
        .start()
        .await
        .unwrap();
    (proxy, backend)
}

async fn chat_seen_by_backend(
    proxy: &TestProxy,
    backend: &FakeBackend,
    username: &str,
    message: &str,
) -> Vec<String> {
    let session = proxy
        .client(VERSION)
        .login(username)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();
    session.chat(message).await.unwrap();
    session.chat(MARKER).await.unwrap();
    let seen = conn.chat_until(MARKER, T).await.unwrap();
    session.quit().await;
    seen.into_iter()
        .filter_map(|chat| match chat.packet {
            ChatPacket::Message(message) => Some(message.message),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_codec_filter_leaves_with_its_plugin_and_no_other_plugin_can_take_it() {
    let owner = ScriptedPlugin::new(OWNER).on_enable(|ctx| {
        ctx.codec_filters()
            .expect("trusted plugins hold the codec-filter capability")
            .register(Box::new(Rewrite { to: PUBLIC }))
            .expect("the filter id is free");
    });
    let (proxy, backend) = start(owner).await;

    assert_eq!(
        chat_seen_by_backend(&proxy, &backend, "Alex", "secret plan").await,
        ["public plan"],
        "the plugin's filter rewrites what the client sends"
    );

    let intruder = proxy.plugin_context(INTRUDER).await.unwrap();
    let registry = intruder.codec_filters().unwrap();
    assert_eq!(
        registry.register(Box::new(Rewrite { to: STOLEN })),
        Err(owned_by_owner(REWRITE))
    );
    assert_eq!(registry.unregister(REWRITE), Err(owned_by_owner(REWRITE)));
    assert_eq!(
        chat_seen_by_backend(&proxy, &backend, "Steve", "secret plan").await,
        ["public plan"],
        "the owner's filter is neither replaced nor removed"
    );

    proxy.disable_plugin(OWNER).await.unwrap();

    assert_eq!(
        chat_seen_by_backend(&proxy, &backend, "Notch", "secret plan").await,
        ["secret plan"],
        "a connection opened after the disable runs no filter code of the plugin"
    );
    assert!(proxy.services().codec_filter_registry.is_empty());
    assert_eq!(
        registry.unregister(REWRITE),
        Err(FilterRegistryError::NotFound(REWRITE.to_owned()))
    );

    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_transport_filter_leaves_with_its_plugin_and_no_other_plugin_can_remove_it() {
    let owner = ScriptedPlugin::new(OWNER).on_enable(|ctx| {
        ctx.transport_filters()
            .expect("trusted plugins hold the transport-filter capability")
            .register(Box::new(Gate))
            .expect("the filter id is free");
    });
    let (proxy, _backend) = start(owner).await;

    assert!(
        proxy.client(VERSION).status().await.is_err(),
        "the plugin's filter rejects every connection"
    );

    let intruder = proxy.plugin_context(INTRUDER).await.unwrap();
    let registry = intruder.transport_filters().unwrap();
    assert_eq!(registry.unregister(GATE), Err(owned_by_owner(GATE)));
    assert_eq!(registry.register(Box::new(Gate)), Err(owned_by_owner(GATE)));
    assert!(proxy.client(VERSION).status().await.is_err());

    proxy.disable_plugin(OWNER).await.unwrap();

    proxy
        .client(VERSION)
        .status()
        .await
        .expect("a connection accepted after the disable passes no filter of the plugin");
    assert!(
        proxy
            .services()
            .transport_filter_registry
            .chain()
            .is_empty()
    );

    proxy.shutdown().await.unwrap();
}

const REQUIRED: &str = "required";
const TRIGGER: &str = "close me";

struct Required {
    unavailable: bool,
}

impl CodecFilterFactory for Required {
    fn metadata(&self) -> FilterMetadata {
        FilterMetadata::new(REQUIRED)
    }

    fn create(&self, _init: &CodecSessionInit) -> Box<dyn CodecFilterInstance> {
        Box::new(RequiredInstance {
            closing: self
                .unavailable
                .then(|| "required filter could not start".to_owned()),
        })
    }
}

struct RequiredInstance {
    closing: Option<String>,
}

impl CodecFilterInstance for RequiredInstance {
    fn filter(&mut self, packet: &mut RawPacket, _output: &mut FrameOutput) -> CodecVerdict {
        let asked = packet
            .data
            .windows(TRIGGER.len())
            .any(|window| window == TRIGGER.as_bytes());
        if asked {
            self.closing = Some("required filter failed".to_owned());
        }
        match &self.closing {
            Some(reason) => CodecVerdict::Error(CodecFilterError::Internal(reason.clone())),
            None => CodecVerdict::Pass,
        }
    }

    fn close_reason(&self) -> Option<&str> {
        self.closing.as_deref()
    }
}

fn with_required_filter(unavailable: bool) -> ScriptedPlugin {
    ScriptedPlugin::new(OWNER).on_enable(move |ctx| {
        ctx.codec_filters()
            .expect("trusted plugins hold the codec-filter capability")
            .register(Box::new(Required { unavailable }))
            .expect("the filter id is free");
    })
}

#[tokio::test]
async fn a_filter_that_cannot_start_for_a_connection_refuses_it_before_the_backend() {
    let (proxy, backend) = start(with_required_filter(true)).await;

    let refused = proxy
        .client(VERSION)
        .login("Alex")
        .await
        .unwrap()
        .disconnected()
        .expect("the connection is refused");
    assert_eq!(refused.text, CODEC_FILTER_UNAVAILABLE);
    assert!(
        backend
            .next_connection(Duration::from_millis(300))
            .await
            .is_err(),
        "the refused player never reaches the backend"
    );

    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_filter_that_fails_mid_session_closes_that_connection_only() {
    let (proxy, backend) = start(with_required_filter(false)).await;

    let mut doomed = proxy
        .client(VERSION)
        .login("Alex")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _doomed_backend = backend.next_connection(T).await.unwrap();
    assert_eq!(
        chat_seen_by_backend(&proxy, &backend, "Steve", "hello").await,
        ["hello"]
    );

    doomed.chat(TRIGGER).await.unwrap();
    let kicked = doomed.expect_disconnect(T).await.unwrap();
    assert_eq!(kicked.text, CODEC_FILTER_FAILED);
    assert_eq!(
        chat_seen_by_backend(&proxy, &backend, "Notch", "still here").await,
        ["still here"],
        "other connections keep their filter and their session"
    );

    proxy.shutdown().await.unwrap();
}

const KICK_TEXT: &str = "Hidden reason";

struct DropKick;

impl CodecFilterFactory for DropKick {
    fn metadata(&self) -> FilterMetadata {
        FilterMetadata::new("drop_kick")
    }

    fn create(&self, _init: &CodecSessionInit) -> Box<dyn CodecFilterInstance> {
        Box::new(DroppingKick)
    }
}

struct DroppingKick;

impl CodecFilterInstance for DroppingKick {
    fn filter(&mut self, packet: &mut RawPacket, _output: &mut FrameOutput) -> CodecVerdict {
        let kick = packet
            .data
            .windows(KICK_TEXT.len())
            .any(|window| window == KICK_TEXT.as_bytes());
        if kick {
            CodecVerdict::Drop
        } else {
            CodecVerdict::Pass
        }
    }
}

#[tokio::test]
async fn a_backend_kick_reaches_the_player_past_a_filter_that_drops_it() {
    let owner = ScriptedPlugin::new(OWNER).on_enable(|ctx| {
        ctx.codec_filters()
            .expect("trusted plugins hold the codec-filter capability")
            .register(Box::new(DropKick))
            .expect("the filter id is free");
    });
    let (proxy, backend) = start(owner).await;

    let mut session = proxy
        .client(VERSION)
        .login("Alex")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();
    conn.kick_json(&format!(r#"{{"text":"{KICK_TEXT}"}}"#))
        .await
        .unwrap();

    let info = session.expect_disconnect(T).await.unwrap();
    assert_eq!(info.text, KICK_TEXT, "{info:?}");

    proxy.shutdown().await.unwrap();
}
