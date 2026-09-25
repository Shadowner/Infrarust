use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use infrarust_api::error::PlayerError;
use infrarust_api::events::resource_pack::ResourcePackOrigin;
use infrarust_api::player::{BossBar, BossBarControl, BossBarUpdate, ResourcePackStatus};
use infrarust_api::types::Component;

use super::{BossBarCommand, PlayerCommand};

const MAX_TRACKED_COOKIE_KEYS: usize = 256;
const MAX_TRACKED_REQUESTS_PER_KEY: usize = 16;
const MAX_TRACKED_LEGACY_PACKS: usize = 16;

type CookieReply = oneshot::Sender<Option<Bytes>>;

enum CookieRequester {
    Proxy(CookieReply),
    Backend,
}

#[derive(Default)]
struct Screen {
    header_footer: Option<(Component, Component)>,
    boss_bars: Vec<(Uuid, BossBar)>,
    backend_boss_bars: HashSet<Uuid>,
}

#[derive(Default)]
struct Packs {
    proxy: HashSet<Uuid>,
    legacy: VecDeque<Option<Uuid>>,
}

#[derive(Default)]
pub(crate) struct Presentation {
    ended: AtomicBool,
    screen: Mutex<Screen>,
    cookies: Mutex<HashMap<String, VecDeque<CookieRequester>>>,
    packs: Mutex<Packs>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Presentation {
    pub(crate) fn is_ended(&self) -> bool {
        self.ended.load(Ordering::Acquire)
    }

    pub(crate) fn end(&self) {
        self.ended.store(true, Ordering::Release);
        *lock(&self.screen) = Screen::default();
        lock(&self.cookies).clear();
        *lock(&self.packs) = Packs::default();
    }

    pub(crate) fn set_header_footer(&self, header: Component, footer: Component) {
        lock(&self.screen).header_footer = Some((header, footer));
    }

    pub(crate) fn header_footer(&self) -> Option<(Component, Component)> {
        lock(&self.screen).header_footer.clone()
    }

    pub(crate) fn show_bar(&self, id: Uuid, bar: BossBar) {
        let mut screen = lock(&self.screen);
        match screen.boss_bars.iter_mut().find(|(shown, _)| *shown == id) {
            Some((_, existing)) => *existing = bar,
            None => screen.boss_bars.push((id, bar)),
        }
    }

    pub(crate) fn update_bar(&self, id: Uuid, update: &BossBarUpdate) -> bool {
        let mut screen = lock(&self.screen);
        match screen.boss_bars.iter_mut().find(|(shown, _)| *shown == id) {
            Some((_, bar)) => {
                bar.apply(update);
                true
            }
            None => false,
        }
    }

    pub(crate) fn hide_bar(&self, id: Uuid) -> bool {
        let mut screen = lock(&self.screen);
        let before = screen.boss_bars.len();
        screen.boss_bars.retain(|(shown, _)| *shown != id);
        screen.boss_bars.len() != before
    }

    pub(crate) fn bars(&self) -> Vec<(Uuid, BossBar)> {
        lock(&self.screen).boss_bars.clone()
    }

    pub(crate) fn backend_bar(&self, id: Uuid, shown: bool) {
        let mut screen = lock(&self.screen);
        if shown {
            screen.backend_boss_bars.insert(id);
        } else {
            screen.backend_boss_bars.remove(&id);
        }
    }

    pub(crate) fn take_backend_bars(&self) -> Vec<Uuid> {
        lock(&self.screen).backend_boss_bars.drain().collect()
    }

    pub(crate) fn cookie_requested(&self, key: &str, reply: Option<CookieReply>) {
        if self.is_ended() {
            return;
        }
        let mut cookies = lock(&self.cookies);
        if !cookies.contains_key(key) && cookies.len() >= MAX_TRACKED_COOKIE_KEYS {
            tracing::debug!(
                key,
                "too many cookie keys in flight; not tracking this request"
            );
            return;
        }
        let queue = cookies.entry(key.to_string()).or_default();
        match reply {
            Some(reply) => queue.push_back(CookieRequester::Proxy(reply)),
            None if queue.len() < MAX_TRACKED_REQUESTS_PER_KEY => {
                queue.push_back(CookieRequester::Backend);
            }
            None => tracing::debug!(key, "too many backend cookie requests in flight"),
        }
    }

    pub(crate) fn cookie_answered(&self, key: &str, payload: Option<Vec<u8>>) -> bool {
        let requester = {
            let mut cookies = lock(&self.cookies);
            let Some(queue) = cookies.get_mut(key) else {
                return false;
            };
            let requester = queue.pop_front();
            if queue.is_empty() {
                cookies.remove(key);
            }
            requester
        };
        match requester {
            Some(CookieRequester::Proxy(reply)) => {
                let _ = reply.send(payload.map(Bytes::from));
                true
            }
            Some(CookieRequester::Backend) | None => false,
        }
    }

    pub(crate) fn pack_pushed(&self, id: Option<Uuid>, by_proxy: bool, legacy: bool) {
        let mut packs = lock(&self.packs);
        if legacy {
            if packs.legacy.len() == MAX_TRACKED_LEGACY_PACKS {
                packs.legacy.pop_front();
            }
            packs.legacy.push_back(id.filter(|_| by_proxy));
        } else if by_proxy && let Some(id) = id {
            packs.proxy.insert(id);
        }
    }

    pub(crate) fn pack_answered(
        &self,
        id: Option<Uuid>,
        status: ResourcePackStatus,
        legacy: bool,
    ) -> (ResourcePackOrigin, Option<Uuid>) {
        let mut packs = lock(&self.packs);
        if legacy {
            let front = packs.legacy.front().copied().flatten();
            if status.is_final() {
                packs.legacy.pop_front();
            }
            return match front {
                Some(proxy_pack) => (ResourcePackOrigin::Proxy, Some(proxy_pack)),
                None => (ResourcePackOrigin::Backend, None),
            };
        }
        match id {
            Some(id) if packs.proxy.contains(&id) => {
                if status.is_final() {
                    packs.proxy.remove(&id);
                }
                (ResourcePackOrigin::Proxy, Some(id))
            }
            other => (ResourcePackOrigin::Backend, other),
        }
    }
}

pub(crate) struct BarControl {
    presentation: Arc<Presentation>,
    commands: mpsc::Sender<PlayerCommand>,
}

impl BarControl {
    pub(crate) const fn new(
        presentation: Arc<Presentation>,
        commands: mpsc::Sender<PlayerCommand>,
    ) -> Self {
        Self {
            presentation,
            commands,
        }
    }

    fn send(&self, id: Uuid, command: BossBarCommand) -> Result<(), PlayerError> {
        self.commands
            .try_send(PlayerCommand::BossBar(id, command))
            .map_err(|e| PlayerError::SendFailed(e.to_string()))
    }

    fn ensure_connected(&self) -> Result<(), PlayerError> {
        if self.presentation.is_ended() || self.commands.is_closed() {
            return Err(PlayerError::Disconnected);
        }
        Ok(())
    }
}

fn not_shown() -> PlayerError {
    PlayerError::InvalidArgument("the boss bar is not shown".to_string())
}

impl BossBarControl for BarControl {
    fn update(&self, id: Uuid, update: BossBarUpdate) -> Result<(), PlayerError> {
        self.ensure_connected()?;
        if !self.presentation.update_bar(id, &update) {
            return Err(not_shown());
        }
        self.send(id, BossBarCommand::Update(update))
    }

    fn hide(&self, id: Uuid) -> Result<(), PlayerError> {
        self.ensure_connected()?;
        if !self.presentation.hide_bar(id) {
            return Err(not_shown());
        }
        self.send(id, BossBarCommand::Hide)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use infrarust_api::player::BossBarColor;

    use super::*;

    #[test]
    fn cookie_answers_go_to_requesters_in_order() {
        let presentation = Presentation::default();
        let (reply, mut answer) = oneshot::channel();
        presentation.cookie_requested("a:b", None);
        presentation.cookie_requested("a:b", Some(reply));

        assert!(!presentation.cookie_answered("a:b", Some(vec![1])));
        assert!(answer.try_recv().is_err());
        assert!(presentation.cookie_answered("a:b", Some(vec![2])));
        assert_eq!(answer.try_recv().unwrap(), Some(Bytes::from_static(&[2])));
        assert!(!presentation.cookie_answered("a:b", None));
    }

    #[test]
    fn ending_drops_every_cookie_waiter() {
        let presentation = Presentation::default();
        let (reply, mut answer) = oneshot::channel();
        presentation.cookie_requested("a:b", Some(reply));
        presentation.end();
        assert!(matches!(
            answer.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        ));
        let (late, mut late_answer) = oneshot::channel();
        presentation.cookie_requested("a:b", Some(late));
        assert!(matches!(
            late_answer.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        ));
    }

    #[test]
    fn modern_packs_are_matched_by_id() {
        let presentation = Presentation::default();
        let ours = Uuid::from_u128(1);
        let theirs = Uuid::from_u128(2);
        presentation.pack_pushed(Some(ours), true, false);
        presentation.pack_pushed(Some(theirs), false, false);

        let accepted = presentation.pack_answered(Some(ours), ResourcePackStatus::Accepted, false);
        assert_eq!(accepted, (ResourcePackOrigin::Proxy, Some(ours)));
        let backend = presentation.pack_answered(Some(theirs), ResourcePackStatus::Declined, false);
        assert_eq!(backend, (ResourcePackOrigin::Backend, Some(theirs)));
        presentation.pack_answered(Some(ours), ResourcePackStatus::SuccessfullyLoaded, false);
        let after = presentation.pack_answered(Some(ours), ResourcePackStatus::Discarded, false);
        assert_eq!(after.0, ResourcePackOrigin::Backend);
    }

    #[test]
    fn legacy_packs_are_matched_in_push_order() {
        let presentation = Presentation::default();
        let ours = Uuid::from_u128(1);
        presentation.pack_pushed(None, false, true);
        presentation.pack_pushed(Some(ours), true, true);

        let first = presentation.pack_answered(None, ResourcePackStatus::Accepted, true);
        assert_eq!(first, (ResourcePackOrigin::Backend, None));
        presentation.pack_answered(None, ResourcePackStatus::SuccessfullyLoaded, true);
        let second = presentation.pack_answered(None, ResourcePackStatus::Declined, true);
        assert_eq!(second, (ResourcePackOrigin::Proxy, Some(ours)));
        let unknown = presentation.pack_answered(None, ResourcePackStatus::Declined, true);
        assert_eq!(unknown, (ResourcePackOrigin::Backend, None));
    }

    #[test]
    fn boss_bars_keep_their_latest_state() {
        let presentation = Presentation::default();
        let id = Uuid::from_u128(5);
        presentation.show_bar(id, BossBar::new(Component::text("a")));
        assert!(presentation.update_bar(
            id,
            &BossBarUpdate::Style {
                color: BossBarColor::Red,
                overlay: infrarust_api::player::BossBarOverlay::Notched6,
            }
        ));
        assert_eq!(presentation.bars()[0].1.color, BossBarColor::Red);
        assert!(presentation.hide_bar(id));
        assert!(!presentation.hide_bar(id));
        assert!(!presentation.update_bar(id, &BossBarUpdate::Progress(0.1)));
    }
}
