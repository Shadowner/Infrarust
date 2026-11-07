#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Write;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use infrarust_transport::IpLimiter;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn ip(text: &str) -> IpAddr {
    text.parse().unwrap()
}

#[test]
fn only_the_first_refusal_warns() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(move || writer.clone())
        .finish();

    tracing::subscriber::with_default(subscriber, || {
        let limiter = IpLimiter::new(1).unwrap();
        let _a = limiter.try_acquire(ip("10.0.0.1")).unwrap();
        let _b = limiter.try_acquire(ip("10.0.0.2")).unwrap();
        assert!(limiter.try_acquire(ip("10.0.0.1")).is_none());
        assert!(limiter.try_acquire(ip("10.0.0.2")).is_none());
    });

    let logs = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert_eq!(logs.matches("WARN").count(), 1, "{logs}");
    assert!(logs.contains("max_connections_per_ip"), "{logs}");
}
