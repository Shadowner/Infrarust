use std::future::Future;

use tokio::time::Instant;

tokio::task_local! {
    static DEADLINE: Instant;
}

pub fn current() -> Option<Instant> {
    DEADLINE.try_with(|deadline| *deadline).ok()
}

pub async fn scope<F: Future>(deadline: Instant, running: F) -> F::Output {
    let deadline = current().map_or(deadline, |outer| outer.min(deadline));
    DEADLINE.scope(deadline, running).await
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn the_deadline_is_seen_inside_its_scope_only() {
        let at = Instant::now() + Duration::from_secs(5);
        assert_eq!(current(), None);
        assert_eq!(scope(at, async { current() }).await, Some(at));
        assert_eq!(current(), None);
    }

    #[tokio::test]
    async fn a_nested_scope_keeps_the_earlier_deadline() {
        let soon = Instant::now() + Duration::from_secs(1);
        let later = soon + Duration::from_secs(10);
        let (inner, looser) = scope(soon, async {
            (
                scope(soon - Duration::from_millis(500), async { current() }).await,
                scope(later, async { current() }).await,
            )
        })
        .await;
        assert_eq!(inner, Some(soon - Duration::from_millis(500)));
        assert_eq!(looser, Some(soon));
    }

    #[tokio::test]
    async fn a_spawned_task_does_not_inherit_the_deadline() {
        let spawned = scope(Instant::now() + Duration::from_secs(1), async {
            tokio::spawn(async { current() }).await.ok().flatten()
        })
        .await;
        assert_eq!(spawned, None);
    }
}
