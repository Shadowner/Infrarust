use std::sync::Arc;

use tokio::sync::{Notify, watch};

#[derive(Debug)]
pub struct Gate {
    entered: Notify,
    open: watch::Sender<bool>,
}

impl Gate {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: Notify::new(),
            open: watch::channel(false).0,
        })
    }

    pub async fn entered(&self) {
        self.entered.notified().await;
    }

    pub fn open(&self) {
        self.open.send_replace(true);
    }

    pub async fn pass(&self) {
        self.entered.notify_one();
        let mut open = self.open.subscribe();
        let _ = open.wait_for(|open| *open).await;
    }
}
