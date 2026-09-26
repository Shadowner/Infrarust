//! Limbo handler trait.

use std::sync::Arc;
use std::time::Duration;

use crate::event::BoxFuture;
use crate::types::{Component, PlayerId, ServerId};

use super::session::LimboSession;

/// The result of a limbo handler action.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum HandlerResult {
    /// Accept — continue to the next handler in the chain or the real server.
    Accept,
    /// Deny — kick the player with a reason message.
    Deny(Component),
    /// Hold — the handler will signal completion later via
    /// [`LimboSession::complete`].
    Hold,
    /// Redirect — send the player to a specific server.
    Redirect(ServerId),
    SendToLimbo(Vec<String>),
    /// Hold, but auto-complete with `on_timeout` if [`LimboSession::complete`] is
    /// not called within `after`. The engine owns the timer, so the deadline holds
    /// even if the handler's own tasks die. `on_timeout` must be terminal
    /// (`Accept`/`Deny`/`Redirect`/`SendToLimbo`); a nested hold is treated as
    /// `Accept` to avoid re-arming forever.
    HoldWithTimeout {
        /// How long to wait before auto-completing.
        after: Duration,
        /// The result to apply when the deadline elapses.
        on_timeout: Box<HandlerResult>,
    },
}

pub const HANDLER_UNAVAILABLE: &str = "Limbo handler unavailable";

impl HandlerResult {
    #[must_use]
    pub fn unavailable() -> Self {
        Self::Deny(Component::text(HANDLER_UNAVAILABLE))
    }
}

/// Why a player's limbo session ended. Passed to [`LimboHandler::on_session_end`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SessionEndReason {
    /// The client disconnected while in limbo.
    Disconnected,
    /// A handler completed the chain, the player is released to the backend.
    Released,
    /// A handler denied the player (kick).
    Kicked,
    /// A handler redirected the player to another server.
    Redirected,
    /// The keepalive liveness check timed out.
    TimedOut,
    /// The proxy is shutting down.
    Shutdown,
}

/// A handler for a limbo stage (Tier 2).
///
/// Limbo handlers are chained in order as configured per-server. The proxy
/// handles the Minecraft protocol (`JoinGame`, `KeepAlive`, chunks) — the
/// handler only provides the game logic.
///
/// Methods use [`BoxFuture`] to allow dyn-dispatch (`Box<dyn LimboHandler>`).
/// Implement by returning `Box::pin(async move { ... })`.
///
/// # Example
/// ```ignore
/// use infrarust_api::prelude::*;
///
/// struct AuthHandler;
///
/// impl LimboHandler for AuthHandler {
///     fn name(&self) -> &str { "auth" }
///
///     fn on_player_enter<'a>(&'a self, session: &'a dyn LimboSession) -> BoxFuture<'a, HandlerResult> {
///         Box::pin(async move {
///             session.send_title(TitleData::new(
///                 Component::text("Please login").color("gold"),
///                 Component::text("/login <password>").color("gray"),
///             )).ok();
///             HandlerResult::Hold
///         })
///     }
/// }
/// ```
pub trait LimboHandler: Send + Sync {
    /// Returns the name of this handler (must match the config reference).
    fn name(&self) -> &str;

    /// Called when a player enters this limbo stage.
    ///
    /// Return [`HandlerResult::Hold`] to keep the player in limbo until
    /// [`LimboSession::complete`] is called.
    fn on_player_enter<'a>(&'a self, session: &'a dyn LimboSession)
    -> BoxFuture<'a, HandlerResult>;

    /// Called when the player sends a `/command args` while in this limbo stage.
    ///
    /// The session can be captured in the returned future for async work.
    /// The default implementation does nothing.
    fn on_command<'a>(
        &'a self,
        _session: &'a dyn LimboSession,
        _command: &'a str,
        _args: &'a [&'a str],
    ) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    /// Called when the player sends a chat message (not a command).
    ///
    /// The default implementation does nothing.
    fn on_chat<'a>(
        &'a self,
        _session: &'a dyn LimboSession,
        _message: &'a str,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    /// Called when the player disconnects while in this limbo stage.
    ///
    /// The default implementation does nothing.
    fn on_disconnect(&self, _player_id: PlayerId) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    /// Called when the player's limbo session ends, for ANY reason.
    ///
    /// Unlike [`on_disconnect`](Self::on_disconnect) (client drop only), this fires
    /// on every terminal outcome : released, kicked, redirected, timed out, or
    /// shutdown so handlers can tear down retained state (registry entries,
    /// spawned tasks) uniformly. The per-session cancellation token
    /// ([`LimboSession::cancellation_token`]) is cancelled around the same time.
    ///
    /// The default implementation does nothing.
    fn on_session_end(&self, _player_id: PlayerId, _reason: SessionEndReason) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

impl<T: LimboHandler + ?Sized> LimboHandler for Arc<T> {
    fn name(&self) -> &str {
        (**self).name()
    }

    fn on_player_enter<'a>(
        &'a self,
        session: &'a dyn LimboSession,
    ) -> BoxFuture<'a, HandlerResult> {
        (**self).on_player_enter(session)
    }

    fn on_command<'a>(
        &'a self,
        session: &'a dyn LimboSession,
        command: &'a str,
        args: &'a [&'a str],
    ) -> BoxFuture<'a, ()> {
        (**self).on_command(session, command, args)
    }

    fn on_chat<'a>(&'a self, session: &'a dyn LimboSession, message: &'a str) -> BoxFuture<'a, ()> {
        (**self).on_chat(session, message)
    }

    fn on_disconnect(&self, player_id: PlayerId) -> BoxFuture<'_, ()> {
        (**self).on_disconnect(player_id)
    }

    fn on_session_end(&self, player_id: PlayerId, reason: SessionEndReason) -> BoxFuture<'_, ()> {
        (**self).on_session_end(player_id, reason)
    }
}

#[cfg(all(test, feature = "test-util"))]
mod tests {
    use std::sync::Mutex;

    use super::super::context::LimboEntryContext;
    use super::super::test_util::RecordingLimboSession;
    use super::*;
    use crate::types::GameProfile;

    #[derive(Default)]
    struct Recording {
        calls: Mutex<Vec<String>>,
    }

    impl Recording {
        fn record(&self, call: impl Into<String>) {
            self.calls.lock().unwrap().push(call.into());
        }
    }

    impl LimboHandler for Recording {
        fn name(&self) -> &str {
            "recording"
        }

        fn on_player_enter<'a>(
            &'a self,
            session: &'a dyn LimboSession,
        ) -> BoxFuture<'a, HandlerResult> {
            self.record(format!("enter {}", session.player_id().as_u64()));
            Box::pin(async { HandlerResult::Accept })
        }

        fn on_command<'a>(
            &'a self,
            _session: &'a dyn LimboSession,
            command: &'a str,
            args: &'a [&'a str],
        ) -> BoxFuture<'a, ()> {
            self.record(format!("command {command} {}", args.join(",")));
            Box::pin(async {})
        }

        fn on_chat<'a>(
            &'a self,
            _session: &'a dyn LimboSession,
            message: &'a str,
        ) -> BoxFuture<'a, ()> {
            self.record(format!("chat {message}"));
            Box::pin(async {})
        }

        fn on_disconnect(&self, player_id: PlayerId) -> BoxFuture<'_, ()> {
            self.record(format!("disconnect {}", player_id.as_u64()));
            Box::pin(async {})
        }

        fn on_session_end(
            &self,
            player_id: PlayerId,
            reason: SessionEndReason,
        ) -> BoxFuture<'_, ()> {
            self.record(format!("end {} {reason:?}", player_id.as_u64()));
            Box::pin(async {})
        }
    }

    #[tokio::test]
    async fn a_shared_handler_forwards_every_callback_to_the_handler_it_wraps() {
        let inner = Arc::new(Recording::default());
        let shared: Box<dyn LimboHandler> = Box::new(Arc::clone(&inner));
        let session = RecordingLimboSession::new(
            PlayerId::new(7),
            GameProfile {
                uuid: uuid::Uuid::nil(),
                username: "Steve".to_string(),
                properties: Vec::new(),
            },
            LimboEntryContext::InitialConnection {
                target_server: ServerId::new("hub"),
            },
        );

        assert_eq!(shared.name(), "recording");
        assert!(matches!(
            shared.on_player_enter(&*session).await,
            HandlerResult::Accept
        ));
        shared.on_command(&*session, "login", &["pw"]).await;
        shared.on_chat(&*session, "hello").await;
        shared.on_disconnect(PlayerId::new(7)).await;
        shared
            .on_session_end(PlayerId::new(7), SessionEndReason::Disconnected)
            .await;

        assert_eq!(
            *inner.calls.lock().unwrap(),
            [
                "enter 7",
                "command login pw",
                "chat hello",
                "disconnect 7",
                "end 7 Disconnected"
            ]
        );
    }
}
