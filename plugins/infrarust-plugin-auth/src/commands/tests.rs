use std::sync::Arc;

use infrarust_api::command::{CommandContext, CommandHandler};
use infrarust_api::limbo::handler::{HandlerResult, LimboHandler};
use infrarust_api::types::PlayerId;

use crate::account::Username;
use crate::password;
use crate::test_support::{MockPlayer, TestEnv, fast_config, limbo_session};

fn ctx(player_id: u64, args: &[&str]) -> CommandContext {
    CommandContext {
        player_id: Some(PlayerId::new(player_id)),
        args: args.iter().map(ToString::to_string).collect(),
        raw: String::new(),
    }
}

#[tokio::test]
async fn changepassword_updates_hash_with_correct_old_password() {
    let env = TestEnv::new().await;
    env.create_account("Steve", Some("old-password-1")).await;
    env.registry.add(MockPlayer::new(1, "Steve"));

    let cmd = super::changepassword::ChangePasswordCommand {
        handler: Arc::clone(&env.handler),
    };
    cmd.execute(
        ctx(1, &["old-password-1", "new-password-2"]),
        &*env.registry,
    )
    .await;

    let account = env
        .storage
        .get_account_blocking(&Username::new("Steve"))
        .unwrap()
        .unwrap();
    let hash = account.password_hash.unwrap();
    assert!(
        password::verify_password("new-password-2", &hash)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn changepassword_rejects_wrong_old_password() {
    let env = TestEnv::new().await;
    env.create_account("Steve", Some("old-password-1")).await;
    let sender = MockPlayer::new(1, "Steve");
    env.registry.add(Arc::clone(&sender));

    let cmd = super::changepassword::ChangePasswordCommand {
        handler: Arc::clone(&env.handler),
    };
    cmd.execute(
        ctx(1, &["not-the-old-one", "new-password-2"]),
        &*env.registry,
    )
    .await;

    let account = env
        .storage
        .get_account_blocking(&Username::new("Steve"))
        .unwrap()
        .unwrap();
    let hash = account.password_hash.unwrap();
    assert!(
        password::verify_password("old-password-1", &hash)
            .await
            .unwrap()
    );
    assert!(sender.sent_text().contains("incorrect"));
}

#[tokio::test]
async fn unregister_deletes_account_with_correct_password() {
    let env = TestEnv::new().await;
    env.create_account("Steve", Some("hunter2hunter2")).await;
    env.registry.add(MockPlayer::new(1, "Steve"));

    let cmd = super::unregister::UnregisterCommand {
        handler: Arc::clone(&env.handler),
    };
    cmd.execute(ctx(1, &["hunter2hunter2"]), &*env.registry)
        .await;

    assert!(!env.storage.has_account_blocking(&Username::new("Steve")));
}

#[tokio::test]
async fn forcelogin_force_completes_target_in_limbo() {
    let env = TestEnv::new().await;
    env.create_account("Steve", Some("hunter2hunter2")).await;
    env.registry.add(MockPlayer::admin(1, "Admin"));
    env.registry.add(MockPlayer::new(2, "Steve"));

    let session = limbo_session(2, "Steve");
    env.handler.on_player_enter(&*session).await;

    let cmd = super::forcelogin::ForceLoginCommand {
        handler: Arc::clone(&env.handler),
    };
    cmd.execute(ctx(1, &["Steve"]), &*env.registry).await;

    env.handler.on_chat(&*session, "ok").await;
    assert!(matches!(session.completions()[..], [HandlerResult::Accept]));
}

#[tokio::test]
async fn forcelogin_requires_admin() {
    let env = TestEnv::new().await;
    let sender = MockPlayer::new(1, "Mallory");
    env.registry.add(Arc::clone(&sender));
    env.registry.add(MockPlayer::new(2, "Steve"));

    let session = limbo_session(2, "Steve");
    env.handler.on_player_enter(&*session).await;

    let cmd = super::forcelogin::ForceLoginCommand {
        handler: Arc::clone(&env.handler),
    };
    cmd.execute(ctx(1, &["Steve"]), &*env.registry).await;

    assert!(sender.sent_text().contains("permission"));
    env.handler.on_chat(&*session, "ok").await;
    assert!(session.completions().is_empty());
}

#[tokio::test]
async fn forceunregister_allows_config_listed_admin() {
    let mut config = fast_config();
    config.admin.admin_usernames = vec!["Console".to_string()];
    let env = TestEnv::with_config(config).await;
    env.create_account("Steve", Some("hunter2hunter2")).await;
    let sender = MockPlayer::new(1, "Console");
    env.registry.add(Arc::clone(&sender));

    let cmd = super::forceunregister::ForceUnregisterCommand {
        handler: Arc::clone(&env.handler),
    };
    cmd.execute(ctx(1, &["Steve"]), &*env.registry).await;

    assert!(!env.storage.has_account_blocking(&Username::new("Steve")));
    assert!(sender.sent_text().contains("Account deleted"));
}

#[tokio::test]
async fn forcechangepassword_sets_new_password() {
    let env = TestEnv::new().await;
    env.create_account("Steve", Some("old-password-1")).await;
    env.registry.add(MockPlayer::admin(1, "Admin"));

    let cmd = super::forcechangepassword::ForceChangePasswordCommand {
        handler: Arc::clone(&env.handler),
    };
    cmd.execute(ctx(1, &["Steve", "new-password-2"]), &*env.registry)
        .await;

    let account = env
        .storage
        .get_account_blocking(&Username::new("Steve"))
        .unwrap()
        .unwrap();
    assert!(
        password::verify_password("new-password-2", &account.password_hash.unwrap())
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn cracked_sets_force_cracked() {
    let env = TestEnv::new().await;
    env.create_account("Notch", None).await;
    env.set_premium_info("Notch", false).await;
    env.registry.add(MockPlayer::new(1, "Notch"));

    let cmd = super::cracked::CrackedCommand {
        handler: Arc::clone(&env.handler),
    };
    cmd.execute(ctx(1, &[]), &*env.registry).await;

    let account = env
        .storage
        .get_account_blocking(&Username::new("Notch"))
        .unwrap()
        .unwrap();
    assert!(account.premium_info.unwrap().force_cracked);
}

#[tokio::test]
async fn premium_unsets_force_cracked() {
    let env = TestEnv::new().await;
    env.create_account("Notch", None).await;
    env.set_premium_info("Notch", true).await;
    env.registry.add(MockPlayer::new(1, "Notch"));

    let cmd = super::premium::PremiumCommand {
        handler: Arc::clone(&env.handler),
    };
    cmd.execute(ctx(1, &[]), &*env.registry).await;

    let account = env
        .storage
        .get_account_blocking(&Username::new("Notch"))
        .unwrap()
        .unwrap();
    assert!(!account.premium_info.unwrap().force_cracked);
}
