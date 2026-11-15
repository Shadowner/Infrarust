use std::collections::HashMap;
use std::hash::Hash;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};

use infrarust_api::event::ListenerHandle;

use crate::event_bus::handler::HandlerEntry;
use crate::util::sync::{read, write};

pub(super) type Handlers = Arc<Vec<HandlerEntry>>;

type HandlerMap<K> = HashMap<K, Handlers>;

fn snapshot<K: Eq + Hash>(map: &HandlerMap<K>, key: &K) -> Option<Handlers> {
    map.get(key).cloned()
}

fn has<K: Eq + Hash>(map: &HandlerMap<K>, key: &K) -> bool {
    map.get(key).is_some_and(|entries| !entries.is_empty())
}

fn insert<K: Eq + Hash>(map: &mut HandlerMap<K>, key: K, entry: HandlerEntry) {
    let entries = Arc::make_mut(map.entry(key).or_default());
    let pos = entries.partition_point(|h| h.priority.value() <= entry.priority.value());
    entries.insert(pos, entry);
}

fn remove<K: Copy + Eq + Hash>(
    map: &mut HandlerMap<K>,
    owner: &str,
    handle: ListenerHandle,
) -> bool {
    let Some((key, pos)) = map.iter().find_map(|(key, entries)| {
        entries
            .iter()
            .position(|h| h.handle == handle && *h.owner == *owner)
            .map(|pos| (*key, pos))
    }) else {
        return false;
    };
    let Some(entries) = map.get_mut(&key) else {
        return false;
    };
    entries[pos].alive.store(false, Ordering::Release);
    if entries.len() == 1 {
        map.remove(&key);
    } else {
        Arc::make_mut(entries).remove(pos);
    }
    true
}

pub(super) struct LockedRegistry<K> {
    map: RwLock<HandlerMap<K>>,
}

impl<K: Copy + Eq + Hash> LockedRegistry<K> {
    pub(super) fn new() -> Self {
        Self {
            map: RwLock::new(HashMap::new()),
        }
    }

    pub(super) fn snapshot(&self, key: &K) -> Option<Handlers> {
        snapshot(&read(&self.map), key)
    }

    pub(super) fn has(&self, key: &K) -> bool {
        has(&read(&self.map), key)
    }

    pub(super) fn insert(&self, key: K, entry: HandlerEntry) {
        insert(&mut write(&self.map), key, entry);
    }

    pub(super) fn remove(&self, owner: &str, handle: ListenerHandle) -> bool {
        remove(&mut write(&self.map), owner, handle)
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        read(&self.map).len()
    }
}
