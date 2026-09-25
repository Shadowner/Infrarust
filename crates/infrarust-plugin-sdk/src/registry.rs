use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

pub(crate) struct Registry<T: ?Sized> {
    entries: RefCell<BTreeMap<u64, Rc<T>>>,
}

impl<T: ?Sized> Registry<T> {
    pub(crate) const fn new() -> Self {
        Self {
            entries: RefCell::new(BTreeMap::new()),
        }
    }

    pub(crate) fn insert(&self, id: u64, entry: Rc<T>) -> Option<Rc<T>> {
        self.entries.borrow_mut().insert(id, entry)
    }

    pub(crate) fn get(&self, id: u64) -> Option<Rc<T>> {
        self.entries.borrow().get(&id).cloned()
    }

    pub(crate) fn remove(&self, id: u64) -> Option<Rc<T>> {
        self.entries.borrow_mut().remove(&id)
    }

    pub(crate) fn find(&self, mut matches: impl FnMut(&T) -> bool) -> Option<u64> {
        self.entries
            .borrow()
            .iter()
            .find(|(_, entry)| matches(entry))
            .map(|(id, _)| *id)
    }

    #[cfg(test)]
    pub(crate) fn contains(&self, id: u64) -> bool {
        self.entries.borrow().contains_key(&id)
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.borrow().len()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::Registry;

    struct Guard {
        on_drop: Option<Box<dyn FnOnce()>>,
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            if let Some(on_drop) = self.on_drop.take() {
                on_drop();
            }
        }
    }

    #[test]
    fn get_hands_out_a_shared_entry() {
        let registry: Registry<Cell<u32>> = Registry::new();
        assert!(registry.insert(7, Rc::new(Cell::new(1))).is_none());
        let entry = registry.get(7).expect("entry present");
        entry.set(5);
        assert_eq!(registry.get(7).map(|e| e.get()), Some(5));
        assert!(registry.get(8).is_none());
    }

    #[test]
    fn insert_returns_the_replaced_entry() {
        let registry: Registry<u32> = Registry::new();
        assert!(registry.insert(1, Rc::new(10)).is_none());
        assert_eq!(registry.insert(1, Rc::new(20)).as_deref(), Some(&10));
        assert_eq!(registry.get(1).as_deref(), Some(&20));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn removed_entry_outlives_the_map_while_held() {
        let registry: Registry<String> = Registry::new();
        registry.insert(3, Rc::new("held".to_owned()));
        let in_flight = registry.get(3).expect("entry present");
        let removed = registry.remove(3).expect("entry removed");
        assert!(!registry.contains(3));
        assert_eq!(Rc::strong_count(&in_flight), 2);
        drop(removed);
        assert_eq!(in_flight.as_str(), "held");
        assert_eq!(Rc::strong_count(&in_flight), 1);
    }

    #[test]
    fn find_matches_by_entry_contents() {
        let registry: Registry<str> = Registry::new();
        registry.insert(4, Rc::from("alpha"));
        registry.insert(5, Rc::from("beta"));
        assert_eq!(registry.find(|e| e == "beta"), Some(5));
        assert_eq!(registry.find(|e| e == "gamma"), None);
    }

    #[test]
    fn dropping_a_removed_entry_may_reenter_the_registry() {
        let registry: Rc<Registry<Guard>> = Rc::new(Registry::new());
        let dropped = Rc::new(Cell::new(false));
        let on_drop = {
            let registry = Rc::clone(&registry);
            let dropped = Rc::clone(&dropped);
            move || {
                assert!(registry.get(1).is_none());
                registry.insert(2, Rc::new(Guard { on_drop: None }));
                dropped.set(true);
            }
        };
        registry.insert(
            1,
            Rc::new(Guard {
                on_drop: Some(Box::new(on_drop)),
            }),
        );
        drop(registry.remove(1));
        assert!(dropped.get());
        assert!(registry.contains(2));
    }
}
