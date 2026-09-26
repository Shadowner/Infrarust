use std::future::Future;

use crate::types::PlayerId;

tokio::task_local! {
    static SESSION: Option<PlayerId>;
}

pub fn current() -> Option<PlayerId> {
    SESSION.try_with(|player| *player).ok().flatten()
}

pub async fn scope<F: Future>(player: Option<PlayerId>, running: F) -> F::Output {
    SESSION.scope(player, running).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_marker_is_seen_inside_its_scope_only() {
        let steve = PlayerId::new(7);
        assert_eq!(current(), None);
        let (inside, masked) = scope(Some(steve), async {
            (current(), scope(None, async { current() }).await)
        })
        .await;
        assert_eq!(inside, Some(steve));
        assert_eq!(masked, None);
        assert_eq!(current(), None);
    }

    #[tokio::test]
    async fn a_spawned_task_does_not_inherit_the_marker() {
        let spawned = scope(Some(PlayerId::new(7)), async {
            tokio::spawn(async { current() }).await.ok().flatten()
        })
        .await;
        assert_eq!(spawned, None);
    }
}
