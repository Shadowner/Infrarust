#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::net::IpAddr;
use std::time::{Duration, SystemTime};

use infrarust_core::ban::types::{BanAction, BanAuditLogEntry, BanEntry, BanSource, BanTarget};
use uuid::Uuid;

fn entry(target: BanTarget) -> BanEntry {
    BanEntry::new("1", target, BanSource::Console)
}

#[test]
fn test_ban_entry_permanent() {
    let entry = entry(BanTarget::Ip("1.2.3.4".parse().unwrap())).reason("grief");
    assert!(entry.is_permanent());
    assert!(!entry.is_expired());
}

#[test]
fn test_ban_entry_not_expired() {
    let entry = entry(BanTarget::Username("Steve".into())).lasting(Duration::from_secs(3600));
    assert!(!entry.is_expired());
    assert!(!entry.is_permanent());
}

#[test]
fn test_ban_entry_expired() {
    let entry = entry(BanTarget::Ip("10.0.0.1".parse().unwrap()))
        .created_at(SystemTime::now() - Duration::from_secs(3600))
        .expires_at(SystemTime::now() - Duration::from_secs(60));
    assert!(entry.is_expired());
}

#[test]
fn test_ban_entry_remaining() {
    let entry = entry(BanTarget::Username("Player".into())).lasting(Duration::from_secs(7200));
    let remaining = entry.remaining().expect("should have remaining time");
    assert!(remaining.as_secs() > 7100);
    assert!(remaining.as_secs() <= 7200);
}

#[test]
fn test_ban_entry_remaining_permanent() {
    let entry = entry(BanTarget::Ip("1.1.1.1".parse().unwrap()));
    assert!(entry.remaining().is_none());
}

#[test]
fn test_ban_entry_kick_message() {
    let entry = entry(BanTarget::Username("Griefer".into()))
        .reason("Griefing")
        .lasting(Duration::from_secs(2 * 24 * 3600));
    let msg = entry.default_kick_message().to_plain();
    assert!(msg.contains("Griefing"));
    assert!(msg.contains("day(s)"));
}

#[test]
fn test_ban_entry_kick_message_perm() {
    let entry = entry(BanTarget::Username("Cheater".into())).reason("Hacking");
    let msg = entry.default_kick_message().to_plain();
    assert!(msg.contains("Hacking"));
    assert!(msg.contains("permanent"));
}

#[test]
fn test_ban_target_display() {
    let ip: IpAddr = "1.2.3.4".parse().unwrap();
    assert_eq!(BanTarget::Ip(ip).to_string(), "IP:1.2.3.4");
    assert_eq!(
        BanTarget::Username("Steve".into()).to_string(),
        "username:Steve"
    );
    let uuid = Uuid::nil();
    assert_eq!(BanTarget::Uuid(uuid).to_string(), format!("UUID:{uuid}"));
}

#[test]
fn test_ban_target_serde_roundtrip() {
    let targets = vec![
        BanTarget::Ip("192.168.1.1".parse().unwrap()),
        BanTarget::IpRange("10.0.0.0/8".parse().unwrap()),
        BanTarget::Username("Steve".into()),
        BanTarget::Uuid(Uuid::new_v4()),
    ];

    for target in targets {
        let json = serde_json::to_string(&target).unwrap();
        let deserialized: BanTarget = serde_json::from_str(&json).unwrap();
        assert_eq!(target, deserialized);
    }
}

#[test]
fn test_ban_source_serde_roundtrip() {
    let sources = vec![
        BanSource::Console,
        BanSource::System,
        BanSource::Plugin("libertybans".into()),
        BanSource::WebApi {
            actor: Some("ops".into()),
        },
        BanSource::Player {
            uuid: Uuid::new_v4(),
            name: "Moderator".into(),
        },
    ];

    for source in sources {
        let json = serde_json::to_string(&source).unwrap();
        let deserialized: BanSource = serde_json::from_str(&json).unwrap();
        assert_eq!(source, deserialized);
    }
}

#[test]
fn test_ban_audit_log_serde() {
    let entry = BanAuditLogEntry {
        action: BanAction::Ban,
        target: BanTarget::Username("Test".into()),
        reason: Some("testing".into()),
        source: "console".into(),
        timestamp: SystemTime::now(),
    };

    let json = serde_json::to_string(&entry).unwrap();
    let deserialized: BanAuditLogEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.action, BanAction::Ban);
    assert_eq!(deserialized.source, "console");
}
