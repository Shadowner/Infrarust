//! Type-erased extension map for storing arbitrary typed data.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;

/// A type-erased extension map for storing middleware/filter data.
///
/// Allows inserting and retrieving typed values without coupling
/// components. Uses `TypeId` keys so each concrete type can appear
/// at most once.
///
/// # Example
/// ```
/// use infrarust_api::types::Extensions;
///
/// let mut ext = Extensions::new();
/// ext.insert(42u32);
/// assert_eq!(ext.get::<u32>(), Some(&42));
/// ```
#[derive(Default)]
pub struct Extensions {
    map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Extensions {
    /// Creates an empty extension map.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a value, returning the previous value of the same type if any.
    pub fn insert<T: Send + Sync + 'static>(&mut self, val: T) -> Option<T> {
        self.map
            .insert(TypeId::of::<T>(), Box::new(val))
            .and_then(|prev| prev.downcast().ok().map(|b| *b))
    }

    /// Returns a reference to the stored value of type `T`, if present.
    #[must_use]
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|b| b.downcast_ref())
    }

    /// Returns a mutable reference to the stored value of type `T`, if present.
    pub fn get_mut<T: Send + Sync + 'static>(&mut self) -> Option<&mut T> {
        self.map
            .get_mut(&TypeId::of::<T>())
            .and_then(|b| b.downcast_mut())
    }

    /// Returns `true` if a value of type `T` is stored.
    #[must_use]
    pub fn contains<T: Send + Sync + 'static>(&self) -> bool {
        self.map.contains_key(&TypeId::of::<T>())
    }

    /// Removes and returns the value of type `T`, if present.
    pub fn remove<T: Send + Sync + 'static>(&mut self) -> Option<T> {
        self.map
            .remove(&TypeId::of::<T>())
            .and_then(|b| b.downcast().ok().map(|b| *b))
    }
}

impl fmt::Debug for Extensions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Extensions")
            .field("count", &self.map.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn insert_and_get() {
        let mut ext = Extensions::new();
        ext.insert(42u32);
        ext.insert("hello".to_string());
        assert_eq!(ext.get::<u32>(), Some(&42));
        assert_eq!(ext.get::<String>(), Some(&"hello".to_string()));
    }

    #[test]
    fn insert_replaces() {
        let mut ext = Extensions::new();
        assert!(ext.insert(1u32).is_none());
        assert_eq!(ext.insert(2u32), Some(1));
        assert_eq!(ext.get::<u32>(), Some(&2));
    }

    #[test]
    fn get_mut() {
        let mut ext = Extensions::new();
        ext.insert(10u32);
        *ext.get_mut::<u32>().unwrap() = 20;
        assert_eq!(ext.get::<u32>(), Some(&20));
    }

    #[test]
    fn contains_and_remove() {
        let mut ext = Extensions::new();
        ext.insert(42u32);
        assert!(ext.contains::<u32>());
        assert_eq!(ext.remove::<u32>(), Some(42));
        assert!(!ext.contains::<u32>());
    }
}
