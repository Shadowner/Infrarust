use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use infrarust_api::permissions::{PermissionSnapshot, SnapshotPermissionChecker};
use infrarust_api::types::PlayerId;

use crate::bindings::infrarust::plugin::permissions as wp;

pub(crate) const MAX_PERMISSION_RULES: usize = 65_536;

#[derive(Debug, Default)]
pub(crate) struct PermissionSnapshots {
    players: Mutex<HashMap<PlayerId, Weak<SnapshotPermissionChecker>>>,
}

impl PermissionSnapshots {
    pub(crate) fn install(
        &self,
        player: PlayerId,
        snapshot: PermissionSnapshot,
    ) -> Arc<SnapshotPermissionChecker> {
        let mut players = self.players();
        if let Some(live) = players.get(&player).and_then(Weak::upgrade) {
            live.replace(snapshot);
            return live;
        }
        players.retain(|_, checker| checker.strong_count() > 0);
        let checker = Arc::new(SnapshotPermissionChecker::new(snapshot));
        players.insert(player, Arc::downgrade(&checker));
        checker
    }

    pub(crate) fn update(&self, player: PlayerId, snapshot: PermissionSnapshot) -> bool {
        match self.live(player) {
            Some(checker) => {
                checker.replace(snapshot);
                true
            }
            None => false,
        }
    }

    pub(crate) fn live(&self, player: PlayerId) -> Option<Arc<SnapshotPermissionChecker>> {
        self.players().get(&player).and_then(Weak::upgrade)
    }

    pub(crate) fn release(&self, player: PlayerId) -> Option<Arc<SnapshotPermissionChecker>> {
        self.players()
            .remove(&player)
            .and_then(|checker| checker.upgrade())
    }

    fn players(&self) -> MutexGuard<'_, HashMap<PlayerId, Weak<SnapshotPermissionChecker>>> {
        self.players.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[cfg(test)]
    fn tracked(&self) -> usize {
        self.players().len()
    }
}

pub(crate) fn snapshot_from_wit(
    snapshot: &wp::PermissionSnapshot,
) -> Result<PermissionSnapshot, String> {
    if snapshot.rules.len() > MAX_PERMISSION_RULES {
        return Err(format!(
            "a permission snapshot holds at most {MAX_PERMISSION_RULES} rules, this one has {}",
            snapshot.rules.len()
        ));
    }
    let mut native = PermissionSnapshot::new().with_admin(snapshot.admin);
    for rule in &snapshot.rules {
        native.set(&rule.node, rule.value);
    }
    Ok(native)
}

pub(crate) fn snapshot_to_wit(snapshot: &PermissionSnapshot) -> wp::PermissionSnapshot {
    let mut rules: Vec<wp::PermissionRule> = snapshot
        .rules()
        .iter()
        .map(|(node, value)| wp::PermissionRule {
            node: node.to_owned(),
            value,
        })
        .collect();
    rules.sort_by(|a, b| a.node.cmp(&b.node));
    wp::PermissionSnapshot {
        rules,
        admin: snapshot.is_admin(),
    }
}

#[cfg(test)]
mod tests {
    use infrarust_api::permissions::{PermissionChecker, Tristate};

    use super::*;

    fn rule(node: &str, value: bool) -> wp::PermissionRule {
        wp::PermissionRule {
            node: node.to_owned(),
            value,
        }
    }

    #[test]
    fn installing_again_updates_the_checker_the_player_already_holds() {
        let snapshots = PermissionSnapshots::default();
        let player = PlayerId::new(7);
        let held = snapshots.install(player, PermissionSnapshot::new());
        let again = snapshots.install(player, PermissionSnapshot::new().with("demo.use", true));
        assert!(Arc::ptr_eq(&held, &again));
        assert!(held.has_permission("demo.use"));

        assert!(snapshots.update(player, PermissionSnapshot::new().with("demo.use", false)));
        assert_eq!(held.value("demo.use"), Tristate::False);
    }

    #[test]
    fn a_checker_nobody_holds_any_more_is_forgotten() {
        let snapshots = PermissionSnapshots::default();
        drop(snapshots.install(PlayerId::new(1), PermissionSnapshot::new()));
        assert!(!snapshots.update(PlayerId::new(1), PermissionSnapshot::new()));
        assert!(snapshots.live(PlayerId::new(1)).is_none());

        let kept = snapshots.install(PlayerId::new(2), PermissionSnapshot::new());
        assert_eq!(snapshots.tracked(), 1, "installing prunes the dead entries");
        assert!(snapshots.live(PlayerId::new(2)).is_some());
        drop(kept);
    }

    #[test]
    fn releasing_hands_back_the_live_checker_once() {
        let snapshots = PermissionSnapshots::default();
        let held = snapshots.install(PlayerId::new(3), PermissionSnapshot::new().with_admin(true));
        let released = snapshots.release(PlayerId::new(3)).expect("a live checker");
        assert!(Arc::ptr_eq(&held, &released));
        assert!(snapshots.release(PlayerId::new(3)).is_none());
        assert!(!snapshots.update(PlayerId::new(3), PermissionSnapshot::new()));
    }

    #[test]
    fn a_wit_snapshot_round_trips_with_normalized_nodes() {
        let wit = wp::PermissionSnapshot {
            rules: vec![rule("Demo.Use", true), rule("demo.*", false)],
            admin: false,
        };
        let native = snapshot_from_wit(&wit).unwrap();
        assert_eq!(native.value("demo.use"), Tristate::True);
        assert_eq!(native.value("demo.kick"), Tristate::False);
        assert_eq!(
            snapshot_to_wit(&native),
            wp::PermissionSnapshot {
                rules: vec![rule("demo.*", false), rule("demo.use", true)],
                admin: false,
            }
        );
    }

    #[test]
    fn an_oversized_snapshot_is_refused() {
        let wit = wp::PermissionSnapshot {
            rules: vec![rule("x", true); MAX_PERMISSION_RULES + 1],
            admin: false,
        };
        let error = snapshot_from_wit(&wit).unwrap_err();
        assert!(error.contains("at most"), "{error}");
    }
}
