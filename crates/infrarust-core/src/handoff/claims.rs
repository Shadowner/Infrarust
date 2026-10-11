use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use bytes::Bytes;
use infrarust_api::handoff::{
    ClaimError, ConnectionClaimRegistry, MAX_CLAIM_PREFIXES, validate_claim_prefix,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimMatch {
    Match(Bytes),
    NeedMoreBytes,
    NoMatchPossible,
}

#[derive(Debug)]
struct Claim {
    owner: Arc<str>,
    prefix: Bytes,
}

#[derive(Debug, Default)]
pub struct ConnectionClaims {
    claims: RwLock<Vec<Claim>>,
    count: AtomicUsize,
}

impl ConnectionClaims {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.count.load(Ordering::Acquire) == 0
    }

    pub fn len(&self) -> usize {
        self.count.load(Ordering::Acquire)
    }

    pub fn register(&self, owner: &str, prefix: &[u8]) -> Result<(), ClaimError> {
        validate_claim_prefix(prefix)?;
        let mut claims = self.write();
        let held = claims.iter().filter(|claim| &*claim.owner == owner);
        if held.clone().any(|claim| claim.prefix == prefix) {
            return Ok(());
        }
        if held.count() >= MAX_CLAIM_PREFIXES {
            return Err(ClaimError::Limit(MAX_CLAIM_PREFIXES));
        }
        claims.push(Claim {
            owner: Arc::from(owner),
            prefix: Bytes::copy_from_slice(prefix),
        });
        self.count.store(claims.len(), Ordering::Release);
        drop(claims);
        tracing::debug!(plugin = owner, prefix = ?Bytes::copy_from_slice(prefix), "connection claim prefix registered");
        Ok(())
    }

    pub fn unregister(&self, owner: &str, prefix: &[u8]) -> Result<(), ClaimError> {
        let mut claims = self.write();
        let at = claims
            .iter()
            .position(|claim| &*claim.owner == owner && claim.prefix == prefix)
            .ok_or(ClaimError::NotFound)?;
        claims.swap_remove(at);
        self.count.store(claims.len(), Ordering::Release);
        Ok(())
    }

    pub fn unregister_owner(&self, owner: &str) -> usize {
        let mut claims = self.write();
        let before = claims.len();
        claims.retain(|claim| &*claim.owner != owner);
        self.count.store(claims.len(), Ordering::Release);
        before - claims.len()
    }

    pub fn prefixes(&self) -> Vec<Bytes> {
        let mut prefixes: Vec<Bytes> = self
            .read()
            .iter()
            .map(|claim| claim.prefix.clone())
            .collect();
        prefixes.sort_unstable();
        prefixes.dedup();
        prefixes
    }

    pub fn owned_by(&self, owner: &str) -> Vec<Bytes> {
        self.read()
            .iter()
            .filter(|claim| &*claim.owner == owner)
            .map(|claim| claim.prefix.clone())
            .collect()
    }

    pub fn matches(&self, data: &[u8]) -> ClaimMatch {
        if self.is_empty() {
            return ClaimMatch::NoMatchPossible;
        }
        let claims = self.read();
        let mut matched: Option<&Bytes> = None;
        let mut pending = false;
        for claim in claims.iter() {
            if data.starts_with(&claim.prefix) {
                if matched.is_none_or(|best| claim.prefix.len() > best.len()) {
                    matched = Some(&claim.prefix);
                }
            } else if claim.prefix.starts_with(data) {
                pending = true;
            }
        }
        match matched {
            Some(prefix) => ClaimMatch::Match(prefix.clone()),
            None if pending => ClaimMatch::NeedMoreBytes,
            None => ClaimMatch::NoMatchPossible,
        }
    }

    pub fn for_plugin(self: &Arc<Self>, plugin_id: &str) -> PluginConnectionClaims {
        PluginConnectionClaims {
            registry: Arc::clone(self),
            plugin_id: plugin_id.to_owned(),
        }
    }

    fn read(&self) -> RwLockReadGuard<'_, Vec<Claim>> {
        self.claims
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, Vec<Claim>> {
        self.claims
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[derive(Debug)]
pub struct PluginConnectionClaims {
    registry: Arc<ConnectionClaims>,
    plugin_id: String,
}

impl PluginConnectionClaims {
    pub fn unregister_all(&self) -> usize {
        self.registry.unregister_owner(&self.plugin_id)
    }

    pub fn registry(&self) -> &Arc<ConnectionClaims> {
        &self.registry
    }
}

impl infrarust_api::handoff::private::Sealed for PluginConnectionClaims {}

impl ConnectionClaimRegistry for PluginConnectionClaims {
    fn register_prefix(&self, prefix: &[u8]) -> Result<(), ClaimError> {
        self.registry.register(&self.plugin_id, prefix)
    }

    fn unregister_prefix(&self, prefix: &[u8]) -> Result<(), ClaimError> {
        self.registry.unregister(&self.plugin_id, prefix)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn an_empty_registry_never_matches() {
        let claims = ConnectionClaims::new();
        assert!(claims.is_empty());
        assert_eq!(claims.matches(b""), ClaimMatch::NoMatchPossible);
        assert_eq!(claims.matches(b"XFER"), ClaimMatch::NoMatchPossible);
    }

    #[test]
    fn bytes_match_wait_or_give_up_against_the_registered_prefixes() {
        let claims = ConnectionClaims::new();
        claims.register("transfer", b"XFER").unwrap();
        assert!(!claims.is_empty());

        assert_eq!(claims.matches(b""), ClaimMatch::NeedMoreBytes);
        assert_eq!(claims.matches(b"XF"), ClaimMatch::NeedMoreBytes);
        assert_eq!(
            claims.matches(b"XFER"),
            ClaimMatch::Match(Bytes::from_static(b"XFER"))
        );
        assert_eq!(
            claims.matches(b"XFER\x00\x01rest"),
            ClaimMatch::Match(Bytes::from_static(b"XFER"))
        );
        assert_eq!(claims.matches(b"XFX"), ClaimMatch::NoMatchPossible);
        assert_eq!(claims.matches(&[0x10, 0x00]), ClaimMatch::NoMatchPossible);
    }

    #[test]
    fn the_longest_full_match_wins_and_a_longer_partial_does_not_hold_it_back() {
        let claims = ConnectionClaims::new();
        claims.register("a", b"AB").unwrap();
        claims.register("b", b"ABCD").unwrap();
        assert_eq!(
            claims.matches(b"ABC"),
            ClaimMatch::Match(Bytes::from_static(b"AB"))
        );
        assert_eq!(
            claims.matches(b"ABCDE"),
            ClaimMatch::Match(Bytes::from_static(b"ABCD"))
        );
        assert_eq!(claims.matches(b"A"), ClaimMatch::NeedMoreBytes);
    }

    #[test]
    fn two_plugins_may_claim_the_same_prefix() {
        let claims = ConnectionClaims::new();
        claims.register("one", b"XFER").unwrap();
        claims.register("two", b"XFER").unwrap();
        assert_eq!(claims.len(), 2);
        assert_eq!(claims.prefixes(), [Bytes::from_static(b"XFER")]);
        claims.unregister("one", b"XFER").unwrap();
        assert_eq!(
            claims.matches(b"XFER"),
            ClaimMatch::Match(Bytes::from_static(b"XFER"))
        );
        claims.unregister("two", b"XFER").unwrap();
        assert!(claims.is_empty());
    }

    #[test]
    fn registering_twice_is_idempotent_and_unregistering_twice_is_not_found() {
        let claims = ConnectionClaims::new();
        claims.register("one", b"XFER").unwrap();
        claims.register("one", b"XFER").unwrap();
        assert_eq!(claims.len(), 1);
        assert_eq!(claims.unregister("one", b"XFER"), Ok(()));
        assert_eq!(claims.unregister("one", b"XFER"), Err(ClaimError::NotFound));
        assert_eq!(claims.unregister("two", b"XFOK"), Err(ClaimError::NotFound));
    }

    #[test]
    fn invalid_prefixes_are_refused_before_anything_is_stored() {
        let claims = ConnectionClaims::new();
        for prefix in [&b"A"[..], &[0x10, 0x00], &[0xFE, 0x01], b"PROXY "] {
            assert!(matches!(
                claims.register("one", prefix),
                Err(ClaimError::InvalidPrefix(_))
            ));
        }
        assert!(claims.is_empty());
    }

    #[test]
    fn a_plugin_holds_at_most_sixteen_prefixes() {
        let claims = ConnectionClaims::new();
        for at in 0..MAX_CLAIM_PREFIXES {
            claims
                .register("one", &[b'A', b'A' + u8::try_from(at).unwrap()])
                .unwrap();
        }
        assert_eq!(
            claims.register("one", b"ZZ"),
            Err(ClaimError::Limit(MAX_CLAIM_PREFIXES))
        );
        claims.register("one", b"AA").unwrap();
        claims.register("two", b"ZZ").unwrap();
        assert_eq!(claims.owned_by("one").len(), MAX_CLAIM_PREFIXES);
    }

    #[test]
    fn a_plugin_handle_registers_under_its_own_name_and_drops_everything_it_owns() {
        let claims = Arc::new(ConnectionClaims::new());
        let one = claims.for_plugin("one");
        let two = claims.for_plugin("two");
        one.register_prefix(b"XFER").unwrap();
        one.register_prefix(b"XFOK").unwrap();
        two.register_prefix(b"XFRQ").unwrap();
        assert_eq!(two.unregister_prefix(b"XFER"), Err(ClaimError::NotFound));
        assert_eq!(one.unregister_all(), 2);
        assert_eq!(claims.prefixes(), [Bytes::from_static(b"XFRQ")]);
        assert_eq!(one.unregister_all(), 0);
        assert!(Arc::ptr_eq(two.registry(), &claims));
    }
}
