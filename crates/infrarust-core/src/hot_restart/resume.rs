use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use infrarust_api::types::GameProfile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::state::ProfileRecord;

pub const RESUME_COOKIE: &str = "infrarust:resume";
pub const RESUME_TOKEN_LEN: usize = 32;
pub const RESUME_TTL: Duration = Duration::from_secs(60);
pub const MAX_RESUMES: usize = 100_000;

pub type ResumeToken = [u8; RESUME_TOKEN_LEN];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeRecord {
    pub ticket: [u8; 32],
    pub profile: ProfileRecord,
    pub online_mode: bool,
    pub client_ip: IpAddr,
    pub domain: String,
    pub server: String,
    pub protocol_version: i32,
    pub expires_at_ms: u64,
}

impl ResumeRecord {
    pub fn profile(&self) -> GameProfile {
        self.profile.clone().into()
    }
}

#[derive(Debug, Clone)]
pub struct ResumeHolder {
    pub profile: GameProfile,
    pub online_mode: bool,
    pub client_ip: IpAddr,
    pub domain: String,
    pub server: String,
    pub protocol_version: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct ResumePresenter<'a> {
    pub client_ip: IpAddr,
    pub domain: &'a str,
    pub username: &'a str,
    pub uuid: Option<Uuid>,
    pub protocol_version: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redeemed {
    Resumed(Box<ResumeRecord>),
    Refused(&'static str),
    Unknown,
}

#[derive(Debug, Default)]
struct Waiting {
    records: HashMap<[u8; 32], ResumeRecord>,
    slots: HashMap<IpAddr, u32>,
}

impl Waiting {
    fn purge(&mut self, now_ms: u64) {
        if self
            .records
            .values()
            .all(|record| record.expires_at_ms > now_ms)
        {
            return;
        }
        self.records
            .retain(|_, record| record.expires_at_ms > now_ms);
        self.slots.clear();
        for record in self.records.values() {
            *self.slots.entry(record.client_ip).or_default() += 1;
        }
    }
}

#[derive(Debug, Default)]
pub struct ResumeRegistry {
    armed: AtomicBool,
    issued: Mutex<Vec<ResumeRecord>>,
    expecting: AtomicBool,
    waiting: Mutex<Waiting>,
}

impl ResumeRegistry {
    pub fn arm(&self) {
        self.armed.store(true, Ordering::Release);
    }

    pub fn disarm(&self) {
        self.armed.store(false, Ordering::Release);
        lock(&self.issued).clear();
    }

    pub fn is_armed(&self) -> bool {
        self.armed.load(Ordering::Acquire)
    }

    pub fn issue(&self, holder: ResumeHolder) -> Option<ResumeToken> {
        self.issue_at(holder, now_ms())
    }

    fn issue_at(&self, holder: ResumeHolder, now_ms: u64) -> Option<ResumeToken> {
        if !self.is_armed() {
            return None;
        }
        let mut issued = lock(&self.issued);
        if issued.len() >= MAX_RESUMES {
            return None;
        }
        let token: ResumeToken = rand::random();
        issued.push(ResumeRecord {
            ticket: ticket_of(&token),
            profile: ProfileRecord::from(&holder.profile),
            online_mode: holder.online_mode,
            client_ip: holder.client_ip,
            domain: holder.domain,
            server: holder.server,
            protocol_version: holder.protocol_version,
            expires_at_ms: now_ms.saturating_add(ttl_ms()),
        });
        Some(token)
    }

    pub fn take_issued(&self) -> Vec<ResumeRecord> {
        self.armed.store(false, Ordering::Release);
        std::mem::take(&mut *lock(&self.issued))
    }

    pub fn admit(&self, records: Vec<ResumeRecord>) -> usize {
        self.admit_at(records, now_ms())
    }

    fn admit_at(&self, records: Vec<ResumeRecord>, now_ms: u64) -> usize {
        let mut waiting = lock(&self.waiting);
        let latest = now_ms.saturating_add(ttl_ms());
        for record in records.into_iter().take(MAX_RESUMES) {
            if record.expires_at_ms <= now_ms || record.expires_at_ms > latest {
                continue;
            }
            *waiting.slots.entry(record.client_ip).or_default() += 1;
            waiting.records.insert(record.ticket, record);
        }
        let admitted = waiting.records.len();
        self.expecting.store(admitted > 0, Ordering::Release);
        admitted
    }

    pub fn expects_any(&self) -> bool {
        self.expects_any_at(now_ms())
    }

    fn expects_any_at(&self, now_ms: u64) -> bool {
        if !self.expecting.load(Ordering::Acquire) {
            return false;
        }
        let mut waiting = lock(&self.waiting);
        waiting.purge(now_ms);
        let any = !waiting.records.is_empty();
        self.expecting.store(any, Ordering::Release);
        any
    }

    pub fn claim_slot(&self, client_ip: IpAddr) -> bool {
        self.claim_slot_at(client_ip, now_ms())
    }

    fn claim_slot_at(&self, client_ip: IpAddr, now_ms: u64) -> bool {
        if !self.expecting.load(Ordering::Acquire) {
            return false;
        }
        let mut waiting = lock(&self.waiting);
        waiting.purge(now_ms);
        match waiting.slots.get_mut(&client_ip) {
            Some(left) if *left > 0 => {
                *left -= 1;
                true
            }
            _ => false,
        }
    }

    pub fn redeem(&self, token: &[u8], presenter: &ResumePresenter<'_>) -> Redeemed {
        self.redeem_at(token, presenter, now_ms())
    }

    fn redeem_at(&self, token: &[u8], presenter: &ResumePresenter<'_>, now_ms: u64) -> Redeemed {
        if token.len() != RESUME_TOKEN_LEN || !self.expecting.load(Ordering::Acquire) {
            return Redeemed::Unknown;
        }
        let record = {
            let mut waiting = lock(&self.waiting);
            let record = waiting.records.remove(&ticket_of(token));
            if waiting.records.is_empty() {
                self.expecting.store(false, Ordering::Release);
            }
            record
        };
        let Some(record) = record else {
            return Redeemed::Unknown;
        };
        if record.expires_at_ms <= now_ms {
            return Redeemed::Refused("the ticket expired");
        }
        if record.client_ip != presenter.client_ip {
            return Redeemed::Refused("the ticket was issued to another address");
        }
        if record.profile.username != presenter.username {
            return Redeemed::Refused("the ticket was issued to another player");
        }
        if presenter
            .uuid
            .is_some_and(|uuid| uuid != record.profile.uuid)
        {
            return Redeemed::Refused("the ticket was issued to another profile");
        }
        if record.protocol_version != presenter.protocol_version {
            return Redeemed::Refused("the client changed its protocol version");
        }
        if record.domain != presenter.domain {
            return Redeemed::Refused("the ticket was issued for another domain");
        }
        Redeemed::Resumed(Box::new(record))
    }
}

fn ticket_of(token: &[u8]) -> [u8; 32] {
    Sha256::digest(token).into()
}

fn ttl_ms() -> u64 {
    u64::try_from(RESUME_TTL.as_millis()).unwrap_or(u64::MAX)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::net::Ipv4Addr;

    use super::*;

    const NOW: u64 = 1_000_000;
    const HOME: IpAddr = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7));
    const ELSEWHERE: IpAddr = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 8));

    fn holder(name: &str) -> ResumeHolder {
        ResumeHolder {
            profile: GameProfile {
                uuid: Uuid::from_u128(7),
                username: name.to_string(),
                properties: Vec::new(),
            },
            online_mode: true,
            client_ip: HOME,
            domain: "play.example.com".to_string(),
            server: "lobby".to_string(),
            protocol_version: 769,
        }
    }

    fn presenter(name: &str) -> ResumePresenter<'_> {
        ResumePresenter {
            client_ip: HOME,
            domain: "play.example.com",
            username: name,
            uuid: Some(Uuid::from_u128(7)),
            protocol_version: 769,
        }
    }

    fn handed(old: &ResumeRegistry) -> ResumeRegistry {
        let new = ResumeRegistry::default();
        new.admit_at(old.take_issued(), NOW);
        new
    }

    fn issued(name: &str) -> (ResumeRegistry, ResumeToken) {
        let old = ResumeRegistry::default();
        old.arm();
        let token = old.issue_at(holder(name), NOW).unwrap();
        (handed(&old), token)
    }

    #[test]
    fn nothing_is_issued_until_the_registry_is_armed() {
        let registry = ResumeRegistry::default();
        assert!(registry.issue_at(holder("Notch"), NOW).is_none());
        registry.arm();
        assert!(registry.issue_at(holder("Notch"), NOW).is_some());
        registry.disarm();
        assert!(registry.issue_at(holder("Notch"), NOW).is_none());
        assert!(registry.take_issued().is_empty());
    }

    #[test]
    fn taking_the_issued_records_disarms_the_registry() {
        let registry = ResumeRegistry::default();
        registry.arm();
        registry.issue_at(holder("Notch"), NOW).unwrap();
        assert_eq!(registry.take_issued().len(), 1);
        assert!(!registry.is_armed());
    }

    #[test]
    fn the_record_holds_a_digest_and_never_the_token() {
        let old = ResumeRegistry::default();
        old.arm();
        let token = old.issue_at(holder("Notch"), NOW).unwrap();
        let records = old.take_issued();
        assert_eq!(records[0].ticket, ticket_of(&token));
        let wire = serde_json::to_vec(&records).unwrap();
        assert!(!wire.windows(token.len()).any(|window| window == token));
    }

    #[test]
    fn a_ticket_resumes_its_player_once() {
        let (registry, token) = issued("Notch");
        assert!(registry.expects_any_at(NOW));
        let Redeemed::Resumed(record) = registry.redeem_at(&token, &presenter("Notch"), NOW + 1)
        else {
            panic!("the ticket was refused");
        };
        assert_eq!(record.server, "lobby");
        assert!(record.online_mode);
        assert_eq!(record.profile().username, "Notch");
        assert_eq!(
            registry.redeem_at(&token, &presenter("Notch"), NOW + 2),
            Redeemed::Unknown
        );
        assert!(!registry.expects_any_at(NOW + 2));
    }

    #[test]
    fn an_unknown_or_misshapen_token_is_unknown() {
        let (registry, token) = issued("Notch");
        assert_eq!(
            registry.redeem_at(&[0; RESUME_TOKEN_LEN], &presenter("Notch"), NOW),
            Redeemed::Unknown
        );
        assert_eq!(
            registry.redeem_at(&token[..31], &presenter("Notch"), NOW),
            Redeemed::Unknown
        );
        assert_eq!(
            registry.redeem_at(&[], &presenter("Notch"), NOW),
            Redeemed::Unknown
        );
        assert!(matches!(
            registry.redeem_at(&token, &presenter("Notch"), NOW),
            Redeemed::Resumed(_)
        ));
    }

    #[test]
    fn a_ticket_presented_by_someone_else_is_refused_and_burnt() {
        for wrong in [
            ResumePresenter {
                client_ip: ELSEWHERE,
                ..presenter("Notch")
            },
            presenter("Herobrine"),
            ResumePresenter {
                uuid: Some(Uuid::from_u128(8)),
                ..presenter("Notch")
            },
            ResumePresenter {
                protocol_version: 774,
                ..presenter("Notch")
            },
            ResumePresenter {
                domain: "other.example.com",
                ..presenter("Notch")
            },
        ] {
            let (registry, token) = issued("Notch");
            assert!(
                matches!(
                    registry.redeem_at(&token, &wrong, NOW),
                    Redeemed::Refused(_)
                ),
                "{wrong:?} was accepted"
            );
            assert_eq!(
                registry.redeem_at(&token, &presenter("Notch"), NOW),
                Redeemed::Unknown
            );
        }
    }

    #[test]
    fn a_login_start_without_a_uuid_is_matched_by_name() {
        let (registry, token) = issued("Notch");
        let nameless = ResumePresenter {
            uuid: None,
            ..presenter("Notch")
        };
        assert!(matches!(
            registry.redeem_at(&token, &nameless, NOW),
            Redeemed::Resumed(_)
        ));
    }

    #[test]
    fn a_ticket_expires() {
        let (registry, token) = issued("Notch");
        let late = NOW + ttl_ms();
        assert!(!registry.expects_any_at(late));
        assert_eq!(
            registry.redeem_at(&token, &presenter("Notch"), late),
            Redeemed::Unknown
        );
    }

    #[test]
    fn a_ticket_redeemed_at_its_deadline_is_refused() {
        let (registry, token) = issued("Notch");
        assert_eq!(
            registry.redeem_at(&token, &presenter("Notch"), NOW + ttl_ms()),
            Redeemed::Refused("the ticket expired")
        );
    }

    #[test]
    fn records_that_are_stale_or_dated_too_far_ahead_are_not_admitted() {
        let old = ResumeRegistry::default();
        old.arm();
        old.issue_at(holder("Notch"), NOW).unwrap();
        let mut records = old.take_issued();
        let mut forged = records[0].clone();
        forged.ticket = [1; 32];
        forged.expires_at_ms = NOW + ttl_ms() * 10;
        records.push(forged);

        let late = ResumeRegistry::default();
        assert_eq!(late.admit_at(records.clone(), NOW + ttl_ms()), 0);
        assert!(!late.expects_any_at(NOW + ttl_ms()));

        let fresh = ResumeRegistry::default();
        assert_eq!(fresh.admit_at(records, NOW), 1);
    }

    #[test]
    fn an_address_gets_one_slot_per_waiting_ticket() {
        let old = ResumeRegistry::default();
        old.arm();
        old.issue_at(holder("Notch"), NOW).unwrap();
        old.issue_at(holder("Jeb"), NOW).unwrap();
        let registry = handed(&old);

        assert!(!registry.claim_slot_at(ELSEWHERE, NOW));
        assert!(registry.claim_slot_at(HOME, NOW));
        assert!(registry.claim_slot_at(HOME, NOW));
        assert!(!registry.claim_slot_at(HOME, NOW));
    }

    #[test]
    fn slots_disappear_with_their_expired_tickets() {
        let (registry, _token) = issued("Notch");
        assert!(!registry.claim_slot_at(HOME, NOW + ttl_ms()));
    }

    #[test]
    fn an_empty_registry_expects_nobody() {
        let registry = ResumeRegistry::default();
        assert!(!registry.expects_any_at(NOW));
        assert!(!registry.claim_slot_at(HOME, NOW));
        assert_eq!(
            registry.redeem_at(&[0; RESUME_TOKEN_LEN], &presenter("Notch"), NOW),
            Redeemed::Unknown
        );
    }

    #[test]
    fn a_record_round_trips_through_json() {
        let old = ResumeRegistry::default();
        old.arm();
        old.issue_at(holder("Notch"), NOW).unwrap();
        let records = old.take_issued();
        let wire = serde_json::to_string(&records).unwrap();
        let back: Vec<ResumeRecord> = serde_json::from_str(&wire).unwrap();
        assert_eq!(back, records);
    }
}
