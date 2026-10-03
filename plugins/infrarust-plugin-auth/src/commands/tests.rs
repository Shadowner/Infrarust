#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::sync::Arc;

use infrarust_api::command::{CommandContext, CommandHandler, CommandSource};
use infrarust_api::limbo::handler::{LimboHandler, LimboOutcome};
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::PlayerId;

use crate::account::Username;
use crate::password;
use crate::test_support::{
    TestEnv, admin, fast_config, limbo_session, limbo_session_with, player, premium_profile,
};

fn ctx(env: &TestEnv, player_id: u64, args: &[&str]) -> CommandContext {
    let sender = env
        .registry
        .get_player_by_id(PlayerId::new(player_id))
        .expect("the sender is registered");
    CommandContext::new(CommandSource::Player(sender), "test", args.join(" "))
}

#[tokio::test]
async fn changepassword_updates_hash_with_correct_old_password() {
    let env = TestEnv::new().await;
    env.create_account("Steve", Some("old-password-1")).await;
    env.registry.add(player(1, "Steve"));
    env.log_in(1, "Steve", "old-password-1").await;

    let cmd = super::changepassword::ChangePasswordCommand {
        handler: Arc::clone(&env.handler),
    };
    cmd.execute(ctx(&env, 1, &["old-password-1", "new-password-2"]))
        .await;

    let account = env
        .storage
        .get_account(&Username::new("Steve"))
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
    let sender = player(1, "Steve");
    env.registry.add(Arc::clone(&sender));
    env.log_in(1, "Steve", "old-password-1").await;

    let cmd = super::changepassword::ChangePasswordCommand {
        handler: Arc::clone(&env.handler),
    };
    cmd.execute(ctx(&env, 1, &["not-the-old-one", "new-password-2"]))
        .await;

    let account = env
        .storage
        .get_account(&Username::new("Steve"))
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
    env.registry.add(player(1, "Steve"));
    env.log_in(1, "Steve", "hunter2hunter2").await;

    let cmd = super::unregister::UnregisterCommand {
        handler: Arc::clone(&env.handler),
    };
    cmd.execute(ctx(&env, 1, &["hunter2hunter2"])).await;

    assert!(!env.storage.has_account(&Username::new("Steve")));
}

#[tokio::test]
async fn forcelogin_force_completes_target_in_limbo() {
    let env = TestEnv::new().await;
    env.create_account("Steve", Some("hunter2hunter2")).await;
    env.registry.add(admin(1, "Admin"));
    env.registry.add(player(2, "Steve"));

    let session = limbo_session(2, "Steve");
    env.handler.on_player_enter(&*session).await;

    let cmd = super::forcelogin::ForceLoginCommand {
        handler: Arc::clone(&env.handler),
    };
    cmd.execute(ctx(&env, 1, &["Steve"])).await;

    env.handler.on_chat(&*session, "ok").await;
    assert!(matches!(session.completions()[..], [LimboOutcome::Accept]));
}

#[tokio::test]
async fn forcelogin_requires_admin() {
    let env = TestEnv::new().await;
    let sender = player(1, "Mallory");
    env.registry.add(Arc::clone(&sender));
    env.registry.add(player(2, "Steve"));

    let session = limbo_session(2, "Steve");
    env.handler.on_player_enter(&*session).await;

    let cmd = super::forcelogin::ForceLoginCommand {
        handler: Arc::clone(&env.handler),
    };
    cmd.execute(ctx(&env, 1, &["Steve"])).await;

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
    env.create_account("Console", Some("console-password"))
        .await;
    let sender = player(1, "Console");
    env.registry.add(Arc::clone(&sender));
    env.log_in(1, "Console", "console-password").await;

    let cmd = super::forceunregister::ForceUnregisterCommand {
        handler: Arc::clone(&env.handler),
    };
    cmd.execute(ctx(&env, 1, &["Steve"])).await;

    assert!(!env.storage.has_account(&Username::new("Steve")));
    assert!(sender.sent_text().contains("Account deleted"));
}

#[tokio::test]
async fn forcechangepassword_sets_new_password() {
    let env = TestEnv::new().await;
    env.create_account("Steve", Some("old-password-1")).await;
    env.registry.add(admin(1, "Admin"));

    let cmd = super::forcechangepassword::ForceChangePasswordCommand {
        handler: Arc::clone(&env.handler),
    };
    cmd.execute(ctx(&env, 1, &["Steve", "new-password-2"]))
        .await;

    let account = env
        .storage
        .get_account(&Username::new("Steve"))
        .unwrap()
        .unwrap();
    assert!(
        password::verify_password("new-password-2", &account.password_hash.unwrap())
            .await
            .unwrap()
    );
}

fn admin_named(name: &str) -> crate::config::AuthConfig {
    let mut config = fast_config();
    config.admin.admin_usernames = vec![name.to_string()];
    config
}

async fn password_is(env: &TestEnv, username: &str, candidate: &str) -> bool {
    let account = env
        .storage
        .get_account(&Username::new(username))
        .unwrap()
        .unwrap();
    password::verify_password(candidate, &account.password_hash.unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn an_admin_name_held_in_auth_limbo_cannot_force_login_itself() {
    let env = TestEnv::with_config(admin_named("Admin")).await;
    env.create_account("Admin", Some("the-real-password")).await;
    let impostor = player(1, "Admin");
    env.registry.add(Arc::clone(&impostor));
    let session = limbo_session(1, "Admin");
    env.handler.on_player_enter(&*session).await;

    let cmd = super::forcelogin::ForceLoginCommand {
        handler: Arc::clone(&env.handler),
    };
    cmd.execute(ctx(&env, 1, &["Admin"])).await;

    assert!(impostor.sent_text().contains("permission"));
    env.handler.on_chat(&*session, "ok").await;
    assert!(session.completions().is_empty());
}

#[tokio::test]
async fn a_held_player_with_the_admin_permission_cannot_force_change_a_password() {
    let env = TestEnv::new().await;
    env.create_account("Steve", Some("old-password-1")).await;
    env.registry.add(admin(1, "Mallory"));
    let session = limbo_session(1, "Mallory");
    env.handler.on_player_enter(&*session).await;

    let cmd = super::forcechangepassword::ForceChangePasswordCommand {
        handler: Arc::clone(&env.handler),
    };
    cmd.execute(ctx(&env, 1, &["Steve", "new-password-2"]))
        .await;

    assert!(password_is(&env, "Steve", "old-password-1").await);
}

#[tokio::test]
async fn an_admin_name_is_trusted_only_after_logging_in() {
    let env = TestEnv::with_config(admin_named("Admin")).await;
    env.create_account("Steve", Some("hunter2hunter2")).await;
    env.create_account("Admin", Some("the-real-password")).await;
    env.registry.add(player(1, "Admin"));
    let cmd = super::forceunregister::ForceUnregisterCommand {
        handler: Arc::clone(&env.handler),
    };

    cmd.execute(ctx(&env, 1, &["Steve"])).await;
    assert!(env.storage.has_account(&Username::new("Steve")));

    env.log_in(1, "Admin", "the-real-password").await;
    cmd.execute(ctx(&env, 1, &["Steve"])).await;
    assert!(!env.storage.has_account(&Username::new("Steve")));
}

#[tokio::test]
async fn changepassword_and_unregister_are_refused_in_auth_limbo() {
    let env = TestEnv::new().await;
    env.create_account("Steve", Some("old-password-1")).await;
    let sender = player(1, "Steve");
    env.registry.add(Arc::clone(&sender));
    let session = limbo_session(1, "Steve");
    env.handler.on_player_enter(&*session).await;

    super::changepassword::ChangePasswordCommand {
        handler: Arc::clone(&env.handler),
    }
    .execute(ctx(&env, 1, &["old-password-1", "new-password-2"]))
    .await;
    super::unregister::UnregisterCommand {
        handler: Arc::clone(&env.handler),
    }
    .execute(ctx(&env, 1, &["old-password-1"]))
    .await;

    assert!(password_is(&env, "Steve", "old-password-1").await);
    assert!(sender.sent_text().contains(super::NOT_LOGGED_IN));
}

#[tokio::test]
async fn wrong_current_passwords_spend_the_login_attempt_budget() {
    let mut config = fast_config();
    config.security.max_login_attempts = 3;
    let env = TestEnv::with_config(config).await;
    env.create_account("Steve", Some("old-password-1")).await;
    let sender = player(1, "Steve");
    env.registry.add(Arc::clone(&sender));
    env.log_in(1, "Steve", "old-password-1").await;

    let change = super::changepassword::ChangePasswordCommand {
        handler: Arc::clone(&env.handler),
    };
    let unregister = super::unregister::UnregisterCommand {
        handler: Arc::clone(&env.handler),
    };
    change
        .execute(ctx(&env, 1, &["guess-1", "new-password-2"]))
        .await;
    unregister.execute(ctx(&env, 1, &["guess-2"])).await;
    assert!(sender.kicks().is_empty());
    change
        .execute(ctx(&env, 1, &["guess-3", "new-password-2"]))
        .await;

    assert_eq!(sender.kicks().len(), 1);
    assert!(password_is(&env, "Steve", "old-password-1").await);
}

#[tokio::test]
async fn a_wrong_current_password_does_not_reveal_the_account_type() {
    let mut config = fast_config();
    config.premium.enabled = true;
    let env = TestEnv::with_config(config).await;
    env.create_account("Notch", None).await;
    env.set_premium_info("Notch", false).await;
    env.create_account("Steve", Some("old-password-1")).await;
    let notch = player(1, "Notch");
    let steve = player(2, "Steve");
    env.registry.add(Arc::clone(&notch));
    env.registry.add(Arc::clone(&steve));
    let premium = limbo_session_with(1, premium_profile(1, "Notch"));
    env.handler.on_player_enter(&*premium).await;
    env.log_in(2, "Steve", "old-password-1").await;

    let cmd = super::changepassword::ChangePasswordCommand {
        handler: Arc::clone(&env.handler),
    };
    cmd.execute(ctx(&env, 1, &["guess", "new-password-2"]))
        .await;
    cmd.execute(ctx(&env, 2, &["guess", "new-password-2"]))
        .await;

    assert_eq!(notch.sent_text(), steve.sent_text());
}

#[tokio::test]
async fn cracked_sets_force_cracked() {
    let env = TestEnv::new().await;
    env.create_account("Notch", None).await;
    env.set_premium_info("Notch", false).await;
    env.registry.add(player(1, "Notch"));

    let cmd = super::cracked_mode::CrackedCommand {
        handler: Arc::clone(&env.handler),
    };
    cmd.execute(ctx(&env, 1, &[])).await;

    let account = env
        .storage
        .get_account(&Username::new("Notch"))
        .unwrap()
        .unwrap();
    assert!(account.premium_info.unwrap().force_cracked);
}

#[tokio::test]
async fn premium_unsets_force_cracked() {
    let env = TestEnv::new().await;
    env.create_account("Notch", None).await;
    env.set_premium_info("Notch", true).await;
    env.registry.add(player(1, "Notch"));

    let cmd = super::cracked_mode::PremiumCommand {
        handler: Arc::clone(&env.handler),
    };
    cmd.execute(ctx(&env, 1, &[])).await;

    let account = env
        .storage
        .get_account(&Username::new("Notch"))
        .unwrap()
        .unwrap();
    assert!(!account.premium_info.unwrap().force_cracked);
}

#[tokio::test]
async fn cracked_tells_a_cracked_player_they_already_are() {
    let env = TestEnv::new().await;
    env.create_account("Notch", None).await;
    env.set_premium_info("Notch", true).await;
    let notch = player(1, "Notch");
    env.registry.add(Arc::clone(&notch));

    let cmd = super::cracked_mode::CrackedCommand {
        handler: Arc::clone(&env.handler),
    };
    cmd.execute(ctx(&env, 1, &[])).await;

    assert!(notch.sent_text().contains("already in cracked mode"));
}

#[tokio::test]
async fn premium_tells_a_premium_player_they_already_are() {
    let env = TestEnv::new().await;
    env.create_account("Notch", None).await;
    env.set_premium_info("Notch", false).await;
    let notch = player(1, "Notch");
    env.registry.add(Arc::clone(&notch));

    let cmd = super::cracked_mode::PremiumCommand {
        handler: Arc::clone(&env.handler),
    };
    cmd.execute(ctx(&env, 1, &[])).await;

    assert!(notch.sent_text().contains("already in premium mode"));
}
