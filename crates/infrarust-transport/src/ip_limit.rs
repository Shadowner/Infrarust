use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use dashmap::DashMap;
use dashmap::mapref::entry::Entry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IpKey {
    V4(Ipv4Addr),
    V6Prefix(Ipv6Addr),
}

impl From<IpAddr> for IpKey {
    fn from(ip: IpAddr) -> Self {
        match ip {
            IpAddr::V4(v4) => Self::V4(v4),
            IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                Some(v4) => Self::V4(v4),
                None => Self::V6Prefix(Ipv6Addr::from(u128::from(v6) & (u128::MAX << 64))),
            },
        }
    }
}

impl fmt::Display for IpKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::V4(v4) => v4.fmt(f),
            Self::V6Prefix(prefix) => write!(f, "{prefix}/64"),
        }
    }
}

#[derive(Debug)]
pub struct IpLimiter {
    limit: u32,
    counts: DashMap<IpKey, u32>,
    warned: AtomicBool,
}

impl IpLimiter {
    pub fn new(limit: u32) -> Option<Arc<Self>> {
        (limit > 0).then(|| {
            Arc::new(Self {
                limit,
                counts: DashMap::new(),
                warned: AtomicBool::new(false),
            })
        })
    }

    pub fn try_acquire(self: &Arc<Self>, ip: IpAddr) -> Option<IpGuard> {
        let key = IpKey::from(ip);
        {
            let mut count = self.counts.entry(key).or_insert(0);
            if *count < self.limit {
                *count += 1;
                return Some(IpGuard {
                    limiter: Arc::clone(self),
                    key,
                });
            }
        }
        if !self.warned.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                limit = self.limit,
                key = %key,
                "refusing connections over max_connections_per_ip; clients behind playit, frp, \
                 ngrok, Docker userland proxy or NAT share one address: send the real address \
                 with receive_proxy_protocol and trusted_proxies, or raise \
                 max_connections_per_ip (0 disables it). This warning is logged once"
            );
        }
        None
    }

    pub(crate) fn force_acquire(self: &Arc<Self>, ip: IpAddr) -> IpGuard {
        let key = IpKey::from(ip);
        *self.counts.entry(key).or_insert(0) += 1;
        IpGuard {
            limiter: Arc::clone(self),
            key,
        }
    }

    pub fn tracked_keys(&self) -> usize {
        self.counts.len()
    }
}

#[derive(Debug)]
pub struct IpGuard {
    limiter: Arc<IpLimiter>,
    key: IpKey,
}

impl Drop for IpGuard {
    fn drop(&mut self) {
        if let Entry::Occupied(mut entry) = self.limiter.counts.entry(self.key) {
            if *entry.get() > 1 {
                *entry.get_mut() -= 1;
            } else {
                entry.remove();
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn ipv6_addresses_in_one_64_share_a_key() {
        assert_eq!(
            IpKey::from(ip("2001:db8:1:2:aaaa::1")),
            IpKey::from(ip("2001:db8:1:2:bbbb::2"))
        );
        assert_ne!(
            IpKey::from(ip("2001:db8:1:2::1")),
            IpKey::from(ip("2001:db8:1:3::1"))
        );
    }

    #[test]
    fn ipv4_addresses_have_their_own_key() {
        assert_ne!(IpKey::from(ip("10.0.0.1")), IpKey::from(ip("10.0.0.2")));
    }

    #[test]
    fn an_ipv4_mapped_address_is_its_ipv4_key() {
        assert_eq!(
            IpKey::from(ip("::ffff:10.0.0.1")),
            IpKey::from(ip("10.0.0.1"))
        );
    }

    #[test]
    fn a_limit_of_zero_disables_the_limiter() {
        assert!(IpLimiter::new(0).is_none());
    }

    #[test]
    fn a_dropped_guard_frees_its_slot_and_its_entry() {
        let limiter = IpLimiter::new(2).unwrap();
        let first = limiter.try_acquire(ip("10.0.0.1")).unwrap();
        let second = limiter.try_acquire(ip("10.0.0.1")).unwrap();
        assert!(limiter.try_acquire(ip("10.0.0.1")).is_none());

        drop(first);
        let third = limiter.try_acquire(ip("10.0.0.1")).unwrap();
        assert_eq!(limiter.tracked_keys(), 1);

        drop(second);
        drop(third);
        assert_eq!(limiter.tracked_keys(), 0);
    }

    #[test]
    fn a_forced_guard_counts_beyond_the_limit() {
        let limiter = IpLimiter::new(1).unwrap();
        let first = limiter.force_acquire(ip("10.0.0.1"));
        let second = limiter.force_acquire(ip("::ffff:10.0.0.1"));
        assert_eq!(
            limiter.counts.get(&IpKey::from(ip("10.0.0.1"))).map(|c| *c),
            Some(2)
        );
        assert!(limiter.try_acquire(ip("10.0.0.1")).is_none());
        assert!(limiter.try_acquire(ip("10.0.0.2")).is_some());

        drop(first);
        assert!(limiter.try_acquire(ip("10.0.0.1")).is_none());

        drop(second);
        let after = limiter.try_acquire(ip("10.0.0.1")).unwrap();
        assert!(limiter.try_acquire(ip("10.0.0.1")).is_none());
        drop(after);
        assert_eq!(limiter.tracked_keys(), 0);
    }

    #[test]
    fn a_forced_guard_under_the_limit_counts_like_a_normal_one() {
        let limiter = IpLimiter::new(2).unwrap();
        let forced = limiter.force_acquire(ip("10.0.0.1"));
        let normal = limiter.try_acquire(ip("10.0.0.1")).unwrap();
        assert!(limiter.try_acquire(ip("10.0.0.1")).is_none());

        drop(forced);
        drop(normal);
        assert_eq!(limiter.tracked_keys(), 0);
    }
}
