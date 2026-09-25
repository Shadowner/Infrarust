#![allow(clippy::unwrap_used, clippy::expect_used, clippy::unused_async)]
use std::collections::BTreeSet;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use infrarust_api::services::ban_service::{BanQuery, LoginAttempt};
use infrarust_core::ban::file_storage::FileBanStorage;
use infrarust_core::ban::storage::BanStorage;
use infrarust_core::ban::types::{BanEntry, BanSource, BanTarget};
use uuid::Uuid;

async fn temp_storage() -> (FileBanStorage, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test_bans.json");
    let storage = FileBanStorage::new(path);
    (storage, dir)
}

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

fn range(s: &str) -> BanTarget {
    BanTarget::IpRange(s.parse().unwrap())
}

fn permanent_ban(target: BanTarget) -> BanEntry {
    BanEntry::new(String::new(), target, BanSource::Console).reason("test")
}

fn temp_ban(target: BanTarget) -> BanEntry {
    permanent_ban(target).lasting(Duration::from_secs(3600))
}

fn expired_ban(target: BanTarget) -> BanEntry {
    BanEntry::new(String::new(), target, BanSource::Console)
        .reason("test")
        .created_at(SystemTime::now() - Duration::from_secs(7200))
        .expires_at(SystemTime::now() - Duration::from_secs(3600))
}

fn pre_auth(address: &str, username: &str) -> LoginAttempt {
    LoginAttempt::pre_auth(ip(address), username)
}

#[tokio::test]
async fn test_add_and_check_ip_ban() {
    let (storage, _dir) = temp_storage().await;

    storage
        .add_ban(permanent_ban(BanTarget::Ip(ip("192.168.1.100"))))
        .await
        .unwrap();

    let result = storage
        .check(&pre_auth("192.168.1.100", "someone"))
        .await
        .unwrap();
    assert!(result.is_some());
}

#[tokio::test]
async fn test_add_and_check_username_ban() {
    let (storage, _dir) = temp_storage().await;

    storage
        .add_ban(permanent_ban(BanTarget::Username("Griefer".into())))
        .await
        .unwrap();

    let result = storage
        .check(&pre_auth("10.0.0.1", "Griefer"))
        .await
        .unwrap();
    assert!(result.is_some());
}

#[tokio::test]
async fn test_add_and_check_uuid_ban() {
    let (storage, _dir) = temp_storage().await;
    let uuid = Uuid::new_v4();

    storage
        .add_ban(permanent_ban(BanTarget::Uuid(uuid)))
        .await
        .unwrap();

    let result = storage.get_ban(&BanTarget::Uuid(uuid)).await.unwrap();
    assert!(result.is_some());
}

#[tokio::test]
async fn test_username_case_insensitive() {
    let (storage, _dir) = temp_storage().await;

    storage
        .add_ban(permanent_ban(BanTarget::Username("Steve".into())))
        .await
        .unwrap();

    for name in ["steve", "STEVE", "sTeVe"] {
        let result = storage.check(&pre_auth("10.0.0.1", name)).await.unwrap();
        assert!(result.is_some(), "{name}");
    }
}

#[tokio::test]
async fn test_remove_ban_returns_the_removed_entry() {
    let (storage, _dir) = temp_storage().await;
    let target = BanTarget::Username("Temp".into());

    let added = storage
        .add_ban(permanent_ban(target.clone()))
        .await
        .unwrap();
    assert!(storage.get_ban(&target).await.unwrap().is_some());

    let removed = storage
        .remove_ban(&target, &BanSource::Console)
        .await
        .unwrap()
        .expect("the ban existed");
    assert_eq!(removed.id, added.id);
    assert!(storage.get_ban(&target).await.unwrap().is_none());
}

#[tokio::test]
async fn test_remove_nonexistent() {
    let (storage, _dir) = temp_storage().await;
    let removed = storage
        .remove_ban(&BanTarget::Username("nobody".into()), &BanSource::Console)
        .await
        .unwrap();
    assert!(removed.is_none());
}

#[tokio::test]
async fn test_replace_existing_ban() {
    let (storage, _dir) = temp_storage().await;
    let target = BanTarget::Username("Player".into());

    let first = storage
        .add_ban(permanent_ban(target.clone()).reason("reason1"))
        .await
        .unwrap();
    let second = storage
        .add_ban(permanent_ban(target.clone()).reason("reason2"))
        .await
        .unwrap();

    let entry = storage.get_ban(&target).await.unwrap().unwrap();
    assert_eq!(entry.reason.as_deref(), Some("reason2"));
    assert_eq!(entry.id, second.id);
    assert_ne!(first.id, second.id);
    assert_eq!(storage.get_all_active().await.unwrap().len(), 1);
}

#[tokio::test]
async fn test_expired_ban_not_returned() {
    let (storage, _dir) = temp_storage().await;

    storage
        .add_ban(expired_ban(BanTarget::Ip(ip("10.0.0.1"))))
        .await
        .unwrap();

    let result = storage
        .check(&pre_auth("10.0.0.1", "someone"))
        .await
        .unwrap();
    assert!(result.is_none());
    assert!(
        storage
            .get_ban(&BanTarget::Ip(ip("10.0.0.1")))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn test_check_ip_ban_wins_over_name_ban() {
    let (storage, _dir) = temp_storage().await;

    storage
        .add_ban(permanent_ban(BanTarget::Ip(ip("192.168.1.1"))).reason("ip ban"))
        .await
        .unwrap();
    storage
        .add_ban(permanent_ban(BanTarget::Username("Player".into())).reason("name ban"))
        .await
        .unwrap();

    let result = storage
        .check(&pre_auth("192.168.1.1", "Player"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.reason.as_deref(), Some("ip ban"));
}

#[tokio::test]
async fn a_uuid_ban_is_not_matched_before_authentication() {
    let (storage, _dir) = temp_storage().await;
    let uuid = Uuid::new_v4();

    storage
        .add_ban(permanent_ban(BanTarget::Uuid(uuid)))
        .await
        .unwrap();

    let claimed = pre_auth("10.0.0.1", "someone").claimed_uuid(Some(uuid));
    assert!(storage.check(&claimed).await.unwrap().is_none());
}

#[tokio::test]
async fn a_uuid_ban_is_matched_once_the_profile_is_final() {
    let (storage, _dir) = temp_storage().await;
    let uuid = Uuid::new_v4();

    storage
        .add_ban(permanent_ban(BanTarget::Uuid(uuid)))
        .await
        .unwrap();

    let attempt = LoginAttempt::post_auth(ip("203.0.113.55"), "BrandNewName", uuid, false);
    let result = storage.check(&attempt).await.unwrap();
    assert!(
        matches!(result.map(|e| e.target), Some(BanTarget::Uuid(_))),
        "UUID ban must match once the UUID is supplied"
    );
}

#[tokio::test]
async fn an_ipv4_ban_matches_the_same_address_seen_through_a_dual_stack_socket() {
    let (storage, _dir) = temp_storage().await;

    storage
        .add_ban(permanent_ban(BanTarget::Ip(ip("203.0.113.7"))))
        .await
        .unwrap();

    let result = storage
        .check(&pre_auth("::ffff:203.0.113.7", "someone"))
        .await
        .unwrap();
    assert!(
        result.is_some(),
        "a v4-mapped client address must hit the v4 ban"
    );
}

#[tokio::test]
async fn an_ipv4_range_bans_every_address_inside_it() {
    let (storage, _dir) = temp_storage().await;

    let added = storage
        .add_ban(permanent_ban(range("198.51.100.0/24")))
        .await
        .unwrap();

    for inside in ["198.51.100.1", "198.51.100.254", "::ffff:198.51.100.9"] {
        let hit = storage.check(&pre_auth(inside, "someone")).await.unwrap();
        assert_eq!(hit.map(|e| e.id), Some(added.id.clone()), "{inside}");
    }
    for outside in ["198.51.101.1", "10.0.0.1", "2001:db8::1"] {
        let hit = storage.check(&pre_auth(outside, "someone")).await.unwrap();
        assert!(hit.is_none(), "{outside}");
    }
}

#[tokio::test]
async fn an_ipv6_range_bans_every_address_inside_it() {
    let (storage, _dir) = temp_storage().await;

    storage
        .add_ban(permanent_ban(range("2001:db8:1::/48")))
        .await
        .unwrap();

    assert!(
        storage
            .check(&pre_auth("2001:db8:1:ffff::2", "a"))
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        storage
            .check(&pre_auth("2001:db8:2::2", "a"))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_v4_mapped_range_is_stored_as_its_ipv4_range() {
    let (storage, _dir) = temp_storage().await;

    let added = storage
        .add_ban(permanent_ban(range("::ffff:192.0.2.0/120")))
        .await
        .unwrap();

    assert_eq!(added.target, range("192.0.2.0/24"));
    assert!(
        storage
            .get_ban(&range("192.0.2.0/24"))
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        storage
            .check(&pre_auth("192.0.2.77", "a"))
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn a_status_request_is_checked_by_address_only() {
    let (storage, _dir) = temp_storage().await;

    storage
        .add_ban(permanent_ban(BanTarget::Username("Pinger".into())))
        .await
        .unwrap();
    storage
        .add_ban(permanent_ban(range("192.0.2.0/24")))
        .await
        .unwrap();

    assert!(
        storage
            .check(&LoginAttempt::status(ip("10.0.0.1")))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        storage
            .check(&LoginAttempt::status(ip("192.0.2.1")))
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn test_purge_expired() {
    let (storage, _dir) = temp_storage().await;

    storage
        .add_ban(expired_ban(BanTarget::Ip(ip("1.1.1.1"))))
        .await
        .unwrap();
    storage
        .add_ban(expired_ban(range("10.0.0.0/8")))
        .await
        .unwrap();
    storage
        .add_ban(permanent_ban(BanTarget::Username("active".into())))
        .await
        .unwrap();

    let purged = storage.purge_expired().await.unwrap();
    assert_eq!(purged, 2);

    let active = storage.get_all_active().await.unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(storage.purge_expired().await.unwrap(), 0);
}

#[tokio::test]
async fn test_get_all_active() {
    let (storage, _dir) = temp_storage().await;

    storage
        .add_ban(permanent_ban(BanTarget::Ip(ip("1.1.1.1"))))
        .await
        .unwrap();
    storage
        .add_ban(temp_ban(BanTarget::Username("Player".into())))
        .await
        .unwrap();
    storage
        .add_ban(expired_ban(BanTarget::Username("Old".into())))
        .await
        .unwrap();

    let active = storage.get_all_active().await.unwrap();
    assert_eq!(active.len(), 2);
}

#[tokio::test]
async fn test_persistence_keeps_ids_sources_and_ranges() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("persist_bans.json");

    let (ip_ban, range_ban) = {
        let storage = FileBanStorage::new(path.clone());
        let ip_ban = storage
            .add_ban(permanent_ban(BanTarget::Ip(ip("5.5.5.5"))))
            .await
            .unwrap();
        let range_ban = storage
            .add_ban(
                BanEntry::new(
                    String::new(),
                    range("10.9.0.0/16"),
                    BanSource::Plugin("guard".into()),
                )
                .reason("range"),
            )
            .await
            .unwrap();
        storage.save().await.unwrap();
        (ip_ban, range_ban)
    };

    let storage = FileBanStorage::new(path);
    storage.load().await.unwrap();
    assert_eq!(storage.get_all_active().await.unwrap().len(), 2);

    let loaded_ip = storage.get_ban(&ip_ban.target).await.unwrap().unwrap();
    assert_eq!(loaded_ip.id, ip_ban.id);
    assert_eq!(loaded_ip.source, BanSource::Console);

    let loaded_range = storage
        .check(&pre_auth("10.9.8.7", "x"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded_range.id, range_ban.id);
    assert_eq!(loaded_range.source, BanSource::Plugin("guard".into()));

    let next = storage
        .add_ban(permanent_ban(BanTarget::Username("Later".into())))
        .await
        .unwrap();
    assert!(
        next.id.parse::<u64>().unwrap() > range_ban.id.parse::<u64>().unwrap(),
        "ids keep growing across restarts"
    );
}

#[tokio::test]
async fn a_ban_file_from_before_ids_and_typed_sources_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy_bans.json");
    std::fs::write(
        &path,
        r#"{
  "bans": [
    { "target": { "type": "username", "value": "Griefer" }, "reason": "grief",
      "expires_at": null, "created_at": 1710720000, "source": "console" },
    { "target": { "type": "ip", "value": "192.0.2.4" }, "reason": null,
      "expires_at": null, "created_at": 1710720001, "source": "plugin" }
  ],
  "audit_log": [
    { "action": "ban", "target": { "type": "username", "value": "Griefer" },
      "reason": "grief", "source": "console", "timestamp": 1710720000 }
  ]
}"#,
    )
    .unwrap();

    let storage = FileBanStorage::new(path);
    storage.load().await.unwrap();

    let griefer = storage
        .get_ban(&BanTarget::Username("griefer".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(griefer.source, BanSource::Console);
    let address = storage
        .get_ban(&BanTarget::Ip(ip("192.0.2.4")))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(address.source, BanSource::Plugin("plugin".into()));
    let ids: BTreeSet<String> = [griefer.id, address.id].into_iter().collect();
    assert_eq!(ids.len(), 2, "every legacy entry gets its own id");
}

#[tokio::test]
async fn test_empty_file_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nonexistent_bans.json");

    let storage = FileBanStorage::new(path);
    storage.load().await.unwrap();
    let all = storage.get_all_active().await.unwrap();
    assert!(all.is_empty());
}

#[tokio::test]
async fn test_concurrent_access() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("concurrent_bans.json");
    let storage = Arc::new(FileBanStorage::new(path));

    let mut handles = Vec::new();

    for i in 0..100u32 {
        let s = Arc::clone(&storage);
        handles.push(tokio::spawn(async move {
            let target = BanTarget::Ip(ip(&format!("10.0.{}.{}", i / 256, i % 256)));
            s.add_ban(permanent_ban(target.clone())).await.unwrap();
            s.get_ban(&target).await.unwrap();
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    let all = storage.get_all_active().await.unwrap();
    assert_eq!(all.len(), 100);
    let ids: BTreeSet<String> = all.into_iter().map(|e| e.id).collect();
    assert_eq!(ids.len(), 100, "concurrent bans never share an id");
}

async fn page_ids(storage: &FileBanStorage, query: BanQuery) -> (Vec<String>, Option<String>) {
    let page = storage.list(query).await.unwrap();
    (
        page.entries.into_iter().map(|e| e.id).collect(),
        page.next_cursor,
    )
}

#[tokio::test]
async fn a_cursor_keeps_its_place_while_bans_come_and_go() {
    let (storage, _dir) = temp_storage().await;
    let mut ids = Vec::new();
    for n in 0..5 {
        let entry = storage
            .add_ban(permanent_ban(BanTarget::Username(format!("p{n}"))))
            .await
            .unwrap();
        ids.push(entry.id);
    }

    let (first, cursor) = page_ids(&storage, BanQuery::new().limit(2)).await;
    assert_eq!(first, ids[0..2]);
    let cursor = cursor.expect("more bans follow");

    storage
        .remove_ban(&BanTarget::Username("p0".into()), &BanSource::Console)
        .await
        .unwrap();
    storage
        .remove_ban(&BanTarget::Username("p2".into()), &BanSource::Console)
        .await
        .unwrap();
    let late = storage
        .add_ban(permanent_ban(BanTarget::Username("late".into())))
        .await
        .unwrap();
    storage
        .add_ban(expired_ban(BanTarget::Username("stale".into())))
        .await
        .unwrap();

    let (second, cursor) = page_ids(&storage, BanQuery::new().after(cursor).limit(2)).await;
    assert_eq!(second, [ids[3].clone(), ids[4].clone()]);
    let (third, cursor) = page_ids(&storage, BanQuery::new().after(cursor.unwrap()).limit(2)).await;
    assert_eq!(third, [late.id]);
    assert_eq!(cursor, None);
}

#[tokio::test]
async fn the_last_full_page_has_no_cursor() {
    let (storage, _dir) = temp_storage().await;
    for n in 0..2 {
        storage
            .add_ban(permanent_ban(BanTarget::Username(format!("p{n}"))))
            .await
            .unwrap();
    }

    let (entries, cursor) = page_ids(&storage, BanQuery::new().limit(2)).await;
    assert_eq!(entries.len(), 2);
    assert_eq!(cursor, None);
}

#[tokio::test]
async fn a_cursor_that_is_not_one_of_ours_is_refused() {
    let (storage, _dir) = temp_storage().await;
    assert!(storage.list(BanQuery::new().after("abc")).await.is_err());
}
