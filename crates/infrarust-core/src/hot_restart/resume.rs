use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fmt;
use std::net::IpAddr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use infrarust_api::types::GameProfile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::now_ms;
use super::state::ProfileRecord;
use crate::util::sync::lock;

pub const RESUME_COOKIE: &str = "infrarust:resume";
pub const RESUME_TOKEN_LEN: usize = 32;
const RESUME_TTL_MS: u64 = 60_000;
const MAX_RESUMES: usize = 100_000;

pub type ResumeToken = [u8; RESUME_TOKEN_LEN];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Ticket([u8; 32]);

impl Ticket {
    fn of(token: &[u8]) -> Self {
        Self(Sha256::digest(token).into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeRecord {
    ticket: Ticket,
    pub profile: ProfileRecord,
    pub online_mode: bool,
    client_ip: IpAddr,
    domain: String,
    #[serde(default)]
    pub server: Option<String>,
    protocol_version: i32,
    expires_at_ms: u64,
}

#[derive(Debug, Clone)]
pub struct ResumeHolder {
    pub profile: GameProfile,
    pub online_mode: bool,
    pub client_ip: IpAddr,
    pub domain: String,
    pub server: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Expired,
    OtherAddress,
    OtherPlayer,
    OtherProfile,
    OtherVersion,
    OtherDomain,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Expired => "the ticket expired",
            Self::OtherAddress => "the ticket was issued to another address",
            Self::OtherPlayer => "the ticket was issued to another player",
            Self::OtherProfile => "the ticket was issued to another profile",
            Self::OtherVersion => "the client changed its protocol version",
            Self::OtherDomain => "the ticket was issued for another domain",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redeemed {
    Resumed(Box<ResumeRecord>),
    Refused(Refusal),
    Unknown,
}

#[derive(Debug, Default)]
struct Waiting {
    records: HashMap<Ticket, ResumeRecord>,
    slots: HashMap<IpAddr, u32>,
    earliest_expiry_ms: Option<u64>,
}

impl Waiting {
    fn insert(&mut self, record: ResumeRecord) {
        let client_ip = record.client_ip;
        let expires_at_ms = record.expires_at_ms;
        if self.records.insert(record.ticket, record).is_none() {
            *self.slots.entry(client_ip).or_default() += 1;
        }
        self.earliest_expiry_ms = Some(
            self.earliest_expiry_ms
                .map_or(expires_at_ms, |earliest| earliest.min(expires_at_ms)),
        );
    }

    fn purge(&mut self, now_ms: u64) {
        if self
            .earliest_expiry_ms
            .is_none_or(|earliest| now_ms < earliest)
        {
            return;
        }
        let slots = &mut self.slots;
        let mut earliest: Option<u64> = None;
        self.records.retain(|_, record| {
            if record.expires_at_ms > now_ms {
                earliest = Some(earliest.map_or(record.expires_at_ms, |earliest| {
                    earliest.min(record.expires_at_ms)
                }));
                return true;
            }
            if let Entry::Occupied(mut slot) = slots.entry(record.client_ip) {
                let left = slot.get().saturating_sub(1);
                if left == 0 {
                    slot.remove();
                } else {
                    slot.insert(left);
                }
            }
            false
        });
        self.earliest_expiry_ms = earliest;
    }
}

#[derive(Debug, Default)]
pub struct ResumeRegistry {
    issued: Mutex<Option<Vec<ResumeRecord>>>,
    expecting: AtomicBool,
    waiting: Mutex<Waiting>,
}

impl ResumeRegistry {
    pub fn arm(&self) {
        lock(&self.issued).get_or_insert_with(Vec::new);
    }

    pub fn disarm(&self) {
        *lock(&self.issued) = None;
    }

    pub fn is_armed(&self) -> bool {
        lock(&self.issued).is_some()
    }

    pub fn issue(&self, holder: ResumeHolder) -> Option<ResumeToken> {
        self.issue_at(holder, now_ms())
    }

    fn issue_at(&self, holder: ResumeHolder, now_ms: u64) -> Option<ResumeToken> {
        let mut issued = lock(&self.issued);
        let issued = issued
            .as_mut()
            .filter(|records| records.len() < MAX_RESUMES)?;
        let token: ResumeToken = rand::random();
        issued.push(ResumeRecord {
            ticket: Ticket::of(&token),
            profile: ProfileRecord::from(&holder.profile),
            online_mode: holder.online_mode,
            client_ip: holder.client_ip,
            domain: holder.domain,
            server: holder.server,
            protocol_version: holder.protocol_version,
            expires_at_ms: now_ms.saturating_add(RESUME_TTL_MS),
        });
        Some(token)
    }

    pub fn take_issued(&self) -> Vec<ResumeRecord> {
        lock(&self.issued).take().unwrap_or_default()
    }

    pub fn admit(&self, records: Vec<ResumeRecord>) -> usize {
        self.admit_at(records, now_ms())
    }

    fn admit_at(&self, records: Vec<ResumeRecord>, now_ms: u64) -> usize {
        let mut waiting = lock(&self.waiting);
        let latest = now_ms.saturating_add(RESUME_TTL_MS);
        for record in records.into_iter().take(MAX_RESUMES) {
            if record.expires_at_ms <= now_ms || record.expires_at_ms > latest {
                continue;
            }
            waiting.insert(record);
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
            let record = waiting.records.remove(&Ticket::of(token));
            if waiting.records.is_empty() {
                self.expecting.store(false, Ordering::Release);
            }
            record
        };
        let Some(record) = record else {
            return Redeemed::Unknown;
        };
        if record.expires_at_ms <= now_ms {
            return Redeemed::Refused(Refusal::Expired);
        }
        if record.client_ip != presenter.client_ip {
            return Redeemed::Refused(Refusal::OtherAddress);
        }
        if record.profile.username != presenter.username {
            return Redeemed::Refused(Refusal::OtherPlayer);
        }
        if presenter
            .uuid
            .is_some_and(|uuid| uuid != record.profile.uuid)
        {
            return Redeemed::Refused(Refusal::OtherProfile);
        }
        if record.protocol_version != presenter.protocol_version {
            return Redeemed::Refused(Refusal::OtherVersion);
        }
        if record.domain != presenter.domain {
            return Redeemed::Refused(Refusal::OtherDomain);
        }
        Redeemed::Resumed(Box::new(record))
    }
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
            server: Some("lobby".to_string()),
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
        assert_eq!(records[0].ticket, Ticket::of(&token));
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
        assert_eq!(record.server.as_deref(), Some("lobby"));
        assert!(record.online_mode);
        assert_eq!(record.profile.username, "Notch");
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
            registry.redeem_at(&token[..RESUME_TOKEN_LEN - 1], &presenter("Notch"), NOW),
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
        for (wrong, reason) in [
            (
                ResumePresenter {
                    client_ip: ELSEWHERE,
                    ..presenter("Notch")
                },
                Refusal::OtherAddress,
            ),
            (presenter("Herobrine"), Refusal::OtherPlayer),
            (
                ResumePresenter {
                    uuid: Some(Uuid::from_u128(8)),
                    ..presenter("Notch")
                },
                Refusal::OtherProfile,
            ),
            (
                ResumePresenter {
                    protocol_version: 774,
                    ..presenter("Notch")
                },
                Refusal::OtherVersion,
            ),
            (
                ResumePresenter {
                    domain: "other.example.com",
                    ..presenter("Notch")
                },
                Refusal::OtherDomain,
            ),
        ] {
            let (registry, token) = issued("Notch");
            assert_eq!(
                registry.redeem_at(&token, &wrong, NOW),
                Redeemed::Refused(reason),
                "{wrong:?}"
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
        let without_uuid = ResumePresenter {
            uuid: None,
            ..presenter("Notch")
        };
        assert!(matches!(
            registry.redeem_at(&token, &without_uuid, NOW),
            Redeemed::Resumed(_)
        ));
    }

    #[test]
    fn an_expired_ticket_is_refused_until_a_purge_forgets_it() {
        let deadline = NOW + RESUME_TTL_MS;

        let (still_held, token) = issued("Notch");
        assert_eq!(
            still_held.redeem_at(&token, &presenter("Notch"), deadline),
            Redeemed::Refused(Refusal::Expired)
        );

        let (purged, token) = issued("Notch");
        assert!(!purged.expects_any_at(deadline));
        assert_eq!(
            purged.redeem_at(&token, &presenter("Notch"), deadline),
            Redeemed::Unknown
        );
    }

    #[test]
    fn a_ticket_is_good_until_the_millisecond_before_its_deadline() {
        let (registry, token) = issued("Notch");
        let last = NOW + RESUME_TTL_MS - 1;
        assert!(registry.expects_any_at(last));
        assert!(matches!(
            registry.redeem_at(&token, &presenter("Notch"), last),
            Redeemed::Resumed(_)
        ));
    }

    #[test]
    fn records_that_are_stale_or_dated_too_far_ahead_are_not_admitted() {
        let old = ResumeRegistry::default();
        old.arm();
        old.issue_at(holder("Notch"), NOW).unwrap();
        let mut records = old.take_issued();
        let mut forged = records[0].clone();
        forged.ticket = Ticket([1; 32]);
        forged.expires_at_ms = NOW + RESUME_TTL_MS * 10;
        records.push(forged);

        let late = ResumeRegistry::default();
        assert_eq!(late.admit_at(records.clone(), NOW + RESUME_TTL_MS), 0);
        assert!(!late.expects_any_at(NOW + RESUME_TTL_MS));

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
        assert!(!registry.claim_slot_at(HOME, NOW + RESUME_TTL_MS));
    }

    #[test]
    fn a_ticket_admitted_twice_holds_one_slot() {
        let old = ResumeRegistry::default();
        old.arm();
        old.issue_at(holder("Notch"), NOW).unwrap();
        let mut records = old.take_issued();
        records.push(records[0].clone());

        let registry = ResumeRegistry::default();
        assert_eq!(registry.admit_at(records, NOW), 1);
        assert!(registry.claim_slot_at(HOME, NOW));
        assert!(!registry.claim_slot_at(HOME, NOW));
    }

    #[test]
    fn an_expired_ticket_frees_only_its_own_slot() {
        let old = ResumeRegistry::default();
        old.arm();
        old.issue_at(holder("Notch"), NOW).unwrap();
        old.issue_at(holder("Jeb"), NOW + 10).unwrap();
        let registry = ResumeRegistry::default();
        assert_eq!(registry.admit_at(old.take_issued(), NOW + 10), 2);

        let between = NOW + RESUME_TTL_MS;
        assert!(registry.expects_any_at(between));
        assert!(registry.claim_slot_at(HOME, between));
        assert!(!registry.claim_slot_at(HOME, between));
        assert!(!registry.expects_any_at(between + 10));
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
