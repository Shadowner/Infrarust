use std::time::{Duration, Instant};

use crate::bindings::infrarust::plugin::scheduler;

use super::{is_limit_exceeded, quota_state, quotas};

#[tokio::test]
async fn the_task_quota_counts_live_tasks_and_a_fired_delay_frees_its_room() {
    let mut state = quota_state(quotas(|q| q.scheduled_tasks = 2), &[]);
    let interval = scheduler::Host::interval(&mut state, 60_000, None, 1)
        .await
        .unwrap()
        .unwrap();
    scheduler::Host::delay(&mut state, 60_000, 2)
        .await
        .unwrap()
        .unwrap();
    let over = scheduler::Host::delay(&mut state, 60_000, 3).await.unwrap();
    assert!(is_limit_exceeded(&over, "scheduled_tasks"), "{over:?}");

    assert_eq!(
        scheduler::Host::cancel(&mut state, interval).await.unwrap(),
        Ok(())
    );
    scheduler::Host::delay(&mut state, 0, 4)
        .await
        .unwrap()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let again = scheduler::Host::delay(&mut state, 60_000, 5).await.unwrap();
        if again.is_ok() {
            break;
        }
        assert!(is_limit_exceeded(&again, "scheduled_tasks"), "{again:?}");
        assert!(
            Instant::now() < deadline,
            "the fired delay still took room in the quota"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
