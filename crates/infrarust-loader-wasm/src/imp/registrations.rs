use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use infrarust_api::command::CommandRegistration;
use infrarust_api::limbo::{LimboHandlerRegistration, SessionHandle};
use infrarust_api::types::PlayerId;

use crate::limbo::deny_unavailable;
use crate::providers::{WasmBanProvider, WasmPermissionProvider};
use crate::snapshots::PermissionSnapshots;
use crate::sync::lock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Target {
    generation: u64,
    callback: u64,
}

#[derive(Debug)]
pub(crate) struct Binding {
    target: Mutex<Option<Target>>,
}

impl Binding {
    fn new(generation: u64, callback: u64) -> Arc<Self> {
        Arc::new(Self {
            target: Mutex::new(Some(Target {
                generation,
                callback,
            })),
        })
    }

    pub(crate) fn callback_for(&self, generation: u64) -> Option<u64> {
        lock(&self.target)
            .filter(|target| target.generation == generation)
            .map(|target| target.callback)
    }

    fn generation(&self) -> Option<u64> {
        lock(&self.target).map(|target| target.generation)
    }

    fn bind(&self, generation: u64, callback: u64) {
        *lock(&self.target) = Some(Target {
            generation,
            callback,
        });
    }

    fn clear(&self) {
        *lock(&self.target) = None;
    }
}

pub(crate) enum Bound {
    Fresh(Arc<Binding>),
    Rebound,
    Full,
}

struct Hold {
    handle: SessionHandle,
    generation: u64,
}

impl Hold {
    fn ended(&self) -> bool {
        self.handle.cancellation_token().is_cancelled()
    }
}

#[derive(Default)]
pub(crate) struct Stale {
    pub(crate) commands: Vec<String>,
    pub(crate) limbo: Vec<LimboHandlerRegistration>,
}

#[derive(Default)]
pub(crate) struct Registrations {
    commands: Mutex<HashMap<String, Arc<Binding>>>,
    command_registrations: Mutex<HashMap<String, CommandRegistration>>,
    limbo: Mutex<HashMap<String, Arc<Binding>>>,
    limbo_registrations: Mutex<HashMap<String, LimboHandlerRegistration>>,
    holds: Mutex<HashMap<PlayerId, Hold>>,
    ban_provider: Mutex<Option<Arc<WasmBanProvider>>>,
    permission_provider: Mutex<Option<Arc<WasmPermissionProvider>>>,
    snapshots: Arc<PermissionSnapshots>,
    codec_filters: Mutex<BTreeSet<String>>,
}

impl Registrations {
    pub(crate) fn record_codec_filter(&self, id: &str) {
        lock(&self.codec_filters).insert(id.to_owned());
    }

    pub(crate) fn forget_codec_filter(&self, id: &str) {
        lock(&self.codec_filters).remove(id);
    }

    pub(crate) fn holds_codec_filter(&self, id: &str) -> bool {
        lock(&self.codec_filters).contains(id)
    }

    pub(crate) fn codec_filter_count(&self) -> usize {
        lock(&self.codec_filters).len()
    }

    pub(crate) fn take_codec_filters(&self) -> Vec<String> {
        std::mem::take(&mut *lock(&self.codec_filters))
            .into_iter()
            .collect()
    }

    pub(crate) fn snapshots(&self) -> &Arc<PermissionSnapshots> {
        &self.snapshots
    }

    pub(crate) fn registered_ban_provider(&self) -> Option<Arc<WasmBanProvider>> {
        lock(&self.ban_provider).clone()
    }

    pub(crate) fn keep_ban_provider(&self, provider: Arc<WasmBanProvider>) {
        *lock(&self.ban_provider) = Some(provider);
    }

    pub(crate) fn registered_permission_provider(&self) -> Option<Arc<WasmPermissionProvider>> {
        lock(&self.permission_provider).clone()
    }

    pub(crate) fn keep_permission_provider(&self, provider: Arc<WasmPermissionProvider>) {
        *lock(&self.permission_provider) = Some(provider);
    }

    pub(crate) fn bind_command(
        &self,
        name: &str,
        generation: u64,
        callback: u64,
        limit: usize,
    ) -> Bound {
        bind(&self.commands, name, generation, callback, false, limit)
    }

    pub(crate) fn unbind_command(&self, name: &str) {
        lock(&self.commands).remove(name);
        lock(&self.command_registrations).remove(name);
    }

    pub(crate) fn record_command_registration(
        &self,
        name: &str,
        registration: CommandRegistration,
    ) {
        lock(&self.command_registrations).insert(name.to_owned(), registration);
    }

    pub(crate) fn command_registration(&self, name: &str) -> Option<CommandRegistration> {
        lock(&self.command_registrations).get(name).cloned()
    }

    pub(crate) fn bind_limbo(
        &self,
        name: &str,
        generation: u64,
        callback: u64,
        limit: usize,
    ) -> Bound {
        bind(&self.limbo, name, generation, callback, true, limit)
    }

    pub(crate) fn unbind_limbo(&self, name: &str) {
        lock(&self.limbo).remove(name);
    }

    pub(crate) fn record_limbo_registration(
        &self,
        name: &str,
        registration: LimboHandlerRegistration,
    ) {
        lock(&self.limbo_registrations).insert(name.to_owned(), registration);
    }

    pub(crate) fn sweep(&self, generation: u64) -> Stale {
        let mut stale = Stale::default();
        lock(&self.commands).retain(|name, binding| {
            let current = binding.generation() == Some(generation);
            if !current {
                binding.clear();
                stale.commands.push(name.clone());
            }
            current
        });
        let mut registrations = lock(&self.command_registrations);
        for name in &stale.commands {
            registrations.remove(name);
        }
        drop(registrations);
        let mut gone = Vec::new();
        lock(&self.limbo).retain(|name, binding| {
            let current = binding.generation() == Some(generation);
            if !current {
                binding.clear();
                gone.push(name.clone());
            }
            current
        });
        let mut registrations = lock(&self.limbo_registrations);
        stale.limbo = gone
            .iter()
            .filter_map(|name| registrations.remove(name))
            .collect();
        stale
    }

    pub(crate) fn track_hold(&self, handle: SessionHandle, generation: u64) {
        let mut holds = lock(&self.holds);
        holds.retain(|_, hold| !hold.ended());
        holds.insert(handle.player_id(), Hold { handle, generation });
    }

    pub(crate) fn release_hold(&self, player: PlayerId) {
        lock(&self.holds).remove(&player);
    }

    pub(crate) fn fail_holds(&self, generation: Option<u64>) -> usize {
        let mut failed = Vec::new();
        lock(&self.holds).retain(|_, hold| {
            if hold.ended() {
                return false;
            }
            let owned = generation.is_none_or(|generation| hold.generation == generation);
            if owned {
                failed.push(hold.handle.clone());
            }
            !owned
        });
        for handle in &failed {
            handle.complete(deny_unavailable());
        }
        failed.len()
    }
}

fn bind(
    map: &Mutex<HashMap<String, Arc<Binding>>>,
    name: &str,
    generation: u64,
    callback: u64,
    same_generation_rebinds: bool,
    limit: usize,
) -> Bound {
    let mut map = lock(map);
    let current = |binding: &Arc<Binding>| binding.generation() == Some(generation);
    if !map.get(name).is_some_and(current) && map.values().filter(|b| current(b)).count() >= limit {
        return Bound::Full;
    }
    if let Some(binding) = map.get(name)
        && binding.generation().is_none_or(|bound| {
            bound < generation || (same_generation_rebinds && bound == generation)
        })
    {
        binding.bind(generation, callback);
        return Bound::Rebound;
    }
    let binding = Binding::new(generation, callback);
    map.insert(name.to_owned(), Arc::clone(&binding));
    Bound::Fresh(binding)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use infrarust_api::limbo::{HandlerResult, LimboEntryContext, LimboSession};
    use infrarust_api::test_util::RecordingLimboSession;
    use infrarust_api::types::{GameProfile, ServerId};

    use super::*;

    const ROOM: usize = 64;

    fn fresh(bound: Bound) -> Arc<Binding> {
        match bound {
            Bound::Fresh(binding) => binding,
            Bound::Rebound | Bound::Full => panic!("expected a fresh binding"),
        }
    }

    fn session(player: u64) -> Arc<RecordingLimboSession> {
        RecordingLimboSession::new(
            PlayerId::new(player),
            GameProfile {
                uuid: uuid::Uuid::nil(),
                username: "tester".to_string(),
                properties: vec![],
            },
            LimboEntryContext::InitialConnection {
                target_server: ServerId::from("hub"),
            },
        )
    }

    #[test]
    fn a_later_generation_rebinds_a_name_instead_of_registering_it_again() {
        let registrations = Registrations::default();
        let binding = fresh(registrations.bind_command("greet", 1, 10, ROOM));
        assert_eq!(binding.callback_for(1), Some(10));

        assert!(matches!(
            registrations.bind_command("greet", 2, 20, ROOM),
            Bound::Rebound
        ));
        assert_eq!(binding.callback_for(2), Some(20));
        assert_eq!(binding.callback_for(1), None, "the old generation lost it");
    }

    #[test]
    fn the_same_generation_registering_a_name_again_replaces_it() {
        let registrations = Registrations::default();
        let first = fresh(registrations.bind_command("greet", 1, 10, ROOM));
        let second = fresh(registrations.bind_command("greet", 1, 11, ROOM));
        assert_eq!(first.callback_for(1), Some(10));
        assert_eq!(second.callback_for(1), Some(11));
    }

    #[test]
    fn the_same_generation_registering_a_limbo_name_again_rebinds_it() {
        let registrations = Registrations::default();
        let gate = fresh(registrations.bind_limbo("gate", 1, 10, ROOM));
        assert!(matches!(
            registrations.bind_limbo("gate", 1, 11, ROOM),
            Bound::Rebound
        ));
        assert_eq!(gate.callback_for(1), Some(11));
    }

    #[test]
    fn a_sweep_drops_the_names_the_new_generation_did_not_register_again() {
        let registrations = Registrations::default();
        fresh(registrations.bind_command("kept", 1, 1, ROOM));
        let dropped = fresh(registrations.bind_command("dropped", 1, 2, ROOM));
        let gate = fresh(registrations.bind_limbo("gate", 1, 3, ROOM));
        let gone = fresh(registrations.bind_limbo("gone", 1, 4, ROOM));
        registrations.bind_command("kept", 2, 5, ROOM);
        registrations.bind_limbo("gate", 2, 6, ROOM);
        let revoked = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&revoked);
        registrations.record_limbo_registration(
            "gone",
            LimboHandlerRegistration::new("gone", move || !flag.swap(true, Ordering::SeqCst)),
        );
        registrations.record_limbo_registration(
            "gate",
            LimboHandlerRegistration::new("gate", || panic!("the kept handler is not revoked")),
        );

        let stale = registrations.sweep(2);
        assert_eq!(stale.commands, ["dropped".to_string()]);
        let stale_limbo: Vec<&str> = stale.limbo.iter().map(|r| r.name()).collect();
        assert_eq!(stale_limbo, ["gone"]);
        assert!(stale.limbo[0].unregister());
        assert!(revoked.load(Ordering::SeqCst));
        assert_eq!(dropped.callback_for(1), None);
        assert_eq!(gate.callback_for(2), Some(6));
        assert_eq!(gone.callback_for(1), None);
        assert!(
            matches!(
                registrations.bind_limbo("gone", 3, 7, ROOM),
                Bound::Fresh(_)
            ),
            "a swept limbo name is registered afresh when it comes back"
        );
        assert_eq!(gone.callback_for(3), None);
        assert!(matches!(
            registrations.bind_command("dropped", 3, 8, ROOM),
            Bound::Fresh(_)
        ));
    }

    #[test]
    fn a_full_generation_refuses_a_new_name_but_replaces_one_it_holds() {
        let registrations = Registrations::default();
        fresh(registrations.bind_command("a", 1, 1, 2));
        fresh(registrations.bind_command("b", 1, 2, 2));
        assert!(matches!(
            registrations.bind_command("c", 1, 3, 2),
            Bound::Full
        ));
        let replaced = fresh(registrations.bind_command("b", 1, 4, 2));
        assert_eq!(replaced.callback_for(1), Some(4));
        registrations.unbind_command("a");
        fresh(registrations.bind_command("c", 1, 5, 2));

        fresh(registrations.bind_limbo("gate", 1, 6, 1));
        assert!(matches!(
            registrations.bind_limbo("gate", 1, 7, 1),
            Bound::Rebound
        ));
        assert!(matches!(
            registrations.bind_limbo("other", 1, 8, 1),
            Bound::Full
        ));
    }

    #[test]
    fn a_new_generation_counts_only_the_names_it_registered_again() {
        let registrations = Registrations::default();
        for (callback, name) in [(1, "a"), (2, "b"), (3, "c")] {
            fresh(registrations.bind_command(name, 1, callback, 3));
        }
        assert!(matches!(
            registrations.bind_command("a", 2, 10, 2),
            Bound::Rebound
        ));
        fresh(registrations.bind_command("d", 2, 11, 2));
        assert!(
            matches!(registrations.bind_command("b", 2, 12, 2), Bound::Full),
            "rebinding a name of the old generation takes room in the new one"
        );
        let stale = registrations.sweep(2);
        let mut stale = stale.commands;
        stale.sort();
        assert_eq!(stale, ["b".to_string(), "c".to_string()]);
    }

    #[test]
    fn codec_filters_are_tracked_across_generations_until_taken() {
        let registrations = Registrations::default();
        registrations.record_codec_filter("tally");
        registrations.record_codec_filter("ops");
        registrations.record_codec_filter("tally");
        registrations.record_codec_filter("dropped");
        registrations.forget_codec_filter("dropped");
        registrations.sweep(2);
        assert_eq!(registrations.codec_filter_count(), 2);
        assert!(registrations.holds_codec_filter("ops"));
        assert!(!registrations.holds_codec_filter("dropped"));

        assert_eq!(registrations.take_codec_filters(), ["ops", "tally"]);
        assert!(registrations.take_codec_filters().is_empty());
    }

    #[test]
    fn failing_the_holds_of_a_generation_denies_only_its_players() {
        let registrations = Registrations::default();
        let (old, new, released) = (session(1), session(2), session(3));
        registrations.track_hold(old.handle(), 1);
        registrations.track_hold(new.handle(), 2);
        registrations.track_hold(released.handle(), 1);
        registrations.release_hold(PlayerId::new(3));

        assert_eq!(registrations.fail_holds(Some(1)), 1);
        assert!(
            matches!(old.completions().as_slice(), [HandlerResult::Deny(reason)] if reason.to_plain() == "Limbo handler unavailable")
        );
        assert!(new.completions().is_empty());
        assert!(released.completions().is_empty());

        assert_eq!(registrations.fail_holds(None), 1);
        assert_eq!(new.completions().len(), 1);
    }

    #[test]
    fn a_hold_whose_session_ended_is_dropped_without_being_completed() {
        let registrations = Registrations::default();
        let (moved, held) = (session(1), session(2));
        registrations.track_hold(moved.handle(), 1);
        moved.cancellation_token().cancel();
        assert_eq!(lock(&registrations.holds).len(), 1);
        registrations.track_hold(held.handle(), 1);
        assert_eq!(
            lock(&registrations.holds).len(),
            1,
            "tracking a new hold forgets the ones whose session ended"
        );

        let gone = session(3);
        registrations.track_hold(gone.handle(), 1);
        gone.cancellation_token().cancel();
        assert_eq!(registrations.fail_holds(Some(1)), 1);
        assert!(moved.completions().is_empty());
        assert!(gone.completions().is_empty());
        assert_eq!(held.completions().len(), 1);
        assert!(lock(&registrations.holds).is_empty());
    }
}
