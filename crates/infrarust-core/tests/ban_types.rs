use std::net::IpAddr;
use std::time::{Duration, SystemTime};

use infrarust_core::ban::types::{BanAction, BanAuditLogEntry, BanEntry, BanTarget};
use uuid::Uuid;

#[test]
fn test_ban_entry_permanent() {
    let entry = BanEntry::new(
        BanTarget::Ip("1.2.3.4".parse().unwrap()),
        Some("grief".into()),
        None,
        "console".into(),
    );
    assert!(entry.is_permanent());
    assert!(!entry.is_expired());
}

#[test]
fn test_ban_entry_not_expired() {
    let entry = BanEntry::new(
        BanTarget::Username("Steve".into()),
        None,
        Some(Duration::from_secs(3600)),
        "admin".into(),
    );
    assert!(!entry.is_expired());
    assert!(!entry.is_permanent());
}

#[test]
fn test_ban_entry_expired() {
    let entry = BanEntry {
        target: BanTarget::Ip("10.0.0.1".parse().unwrap()),
        reason: Some("spam".into()),
        expires_at: Some(SystemTime::now() - Duration::from_secs(60)),
        created_at: SystemTime::now() - Duration::from_secs(3600),
        source: "test".into(),
    };
    assert!(entry.is_expired());
}

#[test]
fn test_ban_entry_remaining() {
    let entry = BanEntry::new(
        BanTarget::Username("Player".into()),
        None,
        Some(Duration::from_secs(7200)),
        "test".into(),
    );
    let remaining = entry.remaining().expect("should have remaining time");
    // Should be close to 7200 seconds (allow some margin)
    assert!(remaining.as_secs() > 7100);
    assert!(remaining.as_secs() <= 7200);
}

#[test]
fn test_ban_entry_remaining_permanent() {
    let entry = BanEntry::new(
        BanTarget::Ip("1.1.1.1".parse().unwrap()),
        None,
        None,
        "test".into(),
    );
    assert!(entry.remaining().is_none());
}

#[test]
fn test_ban_entry_kick_message() {
    let entry = BanEntry::new(
        BanTarget::Username("Griefer".into()),
        Some("Griefing".into()),
        Some(Duration::from_secs(2 * 24 * 3600)),
        "admin".into(),
    );
    let msg = entry.kick_message();
    assert!(msg.contains("Griefing"));
    assert!(msg.contains("day(s)"));
}

#[test]
fn test_ban_entry_kick_message_perm() {
    let entry = BanEntry::new(
        BanTarget::Username("Cheater".into()),
        Some("Hacking".into()),
        None,
        "console".into(),
    );
    let msg = entry.kick_message();
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
