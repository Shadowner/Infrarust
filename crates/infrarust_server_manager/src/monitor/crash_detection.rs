use crate::api::ApiProvider;
use crate::error::ServerManagerError;
use crate::monitor::status::ServerStatus;
use std::time::{Duration, Instant};

const DEFAULT_CRASH_THRESHOLD: u32 = 3;
const DEFAULT_CRASH_WINDOW: Duration = Duration::from_secs(300); // 5 minutes

pub struct CrashDetector {
    crash_threshold: u32,
    crash_window: Duration,
    last_check: Instant,
}

impl Default for CrashDetector {
    fn default() -> Self {
        Self {
            crash_threshold: DEFAULT_CRASH_THRESHOLD,
            crash_window: DEFAULT_CRASH_WINDOW,
            last_check: Instant::now(),
        }
    }
}

impl CrashDetector {
    pub fn new(crash_threshold: u32, crash_window: Duration) -> Self {
        Self {
            crash_threshold,
            crash_window,
            last_check: Instant::now(),
        }
    }

    pub fn is_in_crash_loop(&self, status: &ServerStatus) -> bool {
        if status.crash_count >= self.crash_threshold {
            if let Some(last_crash) = status.last_crash_time {
                let elapsed_since_crash = last_crash.elapsed();
                return elapsed_since_crash <= self.crash_window;
            }
        }
        false
    }

    pub fn reset(&mut self) {
        self.last_check = Instant::now();
    }
}

pub async fn detect_crash_loop<T: ApiProvider>(
    api: &T,
    server_id: &str,
    detector: &CrashDetector,
) -> Result<bool, ServerManagerError> {
    let status = crate::monitor::status::check_status(api, server_id).await?;
    Ok(detector.is_in_crash_loop(&status))
}
