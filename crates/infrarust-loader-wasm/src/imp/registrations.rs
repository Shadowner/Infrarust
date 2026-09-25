use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use infrarust_api::limbo::SessionHandle;
use infrarust_api::types::PlayerId;

use crate::limbo::deny_unavailable;

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
}

struct Hold {
    handle: SessionHandle,
    generation: u64,
}

#[derive(Default)]
pub(crate) struct Registrations {
    commands: Mutex<HashMap<String, Arc<Binding>>>,
    limbo: Mutex<HashMap<String, Arc<Binding>>>,
    holds: Mutex<HashMap<PlayerId, Hold>>,
}

impl Registrations {
    pub(crate) fn bind_command(&self, name: &str, generation: u64, callback: u64) -> Bound {
        bind(&self.commands, name, generation, callback)
    }

    pub(crate) fn unbind_command(&self, name: &str) {
        lock(&self.commands).remove(name);
    }

    pub(crate) fn bind_limbo(&self, name: &str, generation: u64, callback: u64) -> Bound {
        bind(&self.limbo, name, generation, callback)
    }

    pub(crate) fn sweep(&self, generation: u64) -> Vec<String> {
        let mut stale = Vec::new();
        lock(&self.commands).retain(|name, binding| {
            let current = binding.generation() == Some(generation);
            if !current {
                binding.clear();
                stale.push(name.clone());
            }
            current
        });
        for binding in lock(&self.limbo).values() {
            if binding.generation() != Some(generation) {
                binding.clear();
            }
        }
        stale
    }

    pub(crate) fn track_hold(&self, handle: SessionHandle, generation: u64) {
        lock(&self.holds).insert(handle.player_id(), Hold { handle, generation });
    }

    pub(crate) fn release_hold(&self, player: PlayerId) {
        lock(&self.holds).remove(&player);
    }

    pub(crate) fn fail_holds(&self, generation: Option<u64>) -> usize {
        let mut failed = Vec::new();
        lock(&self.holds).retain(|_, hold| {
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
) -> Bound {
    let mut map = lock(map);
    if let Some(binding) = map.get(name)
        && binding.generation().is_none_or(|bound| bound < generation)
    {
        binding.bind(generation, callback);
        return Bound::Rebound;
    }
    let binding = Binding::new(generation, callback);
    map.insert(name.to_owned(), Arc::clone(&binding));
    Bound::Fresh(binding)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use infrarust_api::limbo::test_util::RecordingLimboSession;
    use infrarust_api::limbo::{HandlerResult, LimboEntryContext, LimboSession};
    use infrarust_api::types::{GameProfile, ServerId};

    use super::*;

    fn fresh(bound: Bound) -> Arc<Binding> {
        match bound {
            Bound::Fresh(binding) => binding,
            Bound::Rebound => panic!("expected a fresh binding"),
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
        let binding = fresh(registrations.bind_command("greet", 1, 10));
        assert_eq!(binding.callback_for(1), Some(10));

        assert!(matches!(
            registrations.bind_command("greet", 2, 20),
            Bound::Rebound
        ));
        assert_eq!(binding.callback_for(2), Some(20));
        assert_eq!(binding.callback_for(1), None, "the old generation lost it");
    }

    #[test]
    fn the_same_generation_registering_a_name_again_replaces_it() {
        let registrations = Registrations::default();
        let first = fresh(registrations.bind_command("greet", 1, 10));
        let second = fresh(registrations.bind_command("greet", 1, 11));
        assert_eq!(first.callback_for(1), Some(10));
        assert_eq!(second.callback_for(1), Some(11));
    }

    #[test]
    fn a_sweep_drops_the_names_the_new_generation_did_not_register_again() {
        let registrations = Registrations::default();
        fresh(registrations.bind_command("kept", 1, 1));
        let dropped = fresh(registrations.bind_command("dropped", 1, 2));
        let gate = fresh(registrations.bind_limbo("gate", 1, 3));
        let gone = fresh(registrations.bind_limbo("gone", 1, 4));
        registrations.bind_command("kept", 2, 5);
        registrations.bind_limbo("gate", 2, 6);

        assert_eq!(registrations.sweep(2), ["dropped".to_string()]);
        assert_eq!(dropped.callback_for(1), None);
        assert_eq!(gate.callback_for(2), Some(6));
        assert_eq!(gone.callback_for(1), None);
        assert!(
            matches!(registrations.bind_limbo("gone", 3, 7), Bound::Rebound),
            "a cleared limbo name is rebound when it comes back"
        );
        assert_eq!(gone.callback_for(3), Some(7));
        assert!(matches!(
            registrations.bind_command("dropped", 3, 8),
            Bound::Fresh(_)
        ));
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
}
