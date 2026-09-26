use std::sync::Mutex;

use crate::error::ServiceError;
use crate::event::BoxFuture;
use crate::services::ban_service::{
    BanEntry, BanFeatures, BanPage, BanProvider, BanQuery, BanRequest, BanSource, BanTarget,
    BanVerdict, LoginAttempt, UnbanRequest,
};
use crate::types::Component;

use super::lock;

type KickMessage = Box<dyn Fn(&BanEntry) -> Component + Send + Sync>;

#[derive(Default)]
pub struct MemoryBanProvider {
    entries: Mutex<Vec<BanEntry>>,
    attempts: Mutex<Vec<LoginAttempt>>,
    message: Option<KickMessage>,
    ranges: bool,
    stuck: bool,
}

impl std::fmt::Debug for MemoryBanProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryBanProvider")
            .field("entries", &lock(&self.entries))
            .field("attempts", &lock(&self.attempts))
            .field("ranges", &self.ranges)
            .field("stuck", &self.stuck)
            .finish_non_exhaustive()
    }
}

impl MemoryBanProvider {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with(entries: impl IntoIterator<Item = BanEntry>) -> Self {
        Self {
            entries: Mutex::new(entries.into_iter().collect()),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn kick_message(self, message: Component) -> Self {
        self.kick_message_with(move |_| message.clone())
    }

    #[must_use]
    pub fn kick_message_with(
        mut self,
        message: impl Fn(&BanEntry) -> Component + Send + Sync + 'static,
    ) -> Self {
        self.message = Some(Box::new(message));
        self
    }

    #[must_use]
    pub const fn ranges(mut self, supported: bool) -> Self {
        self.ranges = supported;
        self
    }

    #[must_use]
    pub const fn stuck(mut self) -> Self {
        self.stuck = true;
        self
    }

    #[must_use]
    pub fn entries(&self) -> Vec<BanEntry> {
        lock(&self.entries).clone()
    }

    #[must_use]
    pub fn attempts(&self) -> Vec<LoginAttempt> {
        lock(&self.attempts).clone()
    }

    fn verdict(&self, entry: BanEntry) -> BanVerdict {
        match &self.message {
            Some(message) => {
                let message = message(&entry);
                BanVerdict::new(entry).message(message)
            }
            None => BanVerdict::new(entry),
        }
    }
}

impl BanProvider for MemoryBanProvider {
    fn check<'a>(
        &'a self,
        attempt: &'a LoginAttempt,
    ) -> BoxFuture<'a, Result<Option<BanVerdict>, ServiceError>> {
        lock(&self.attempts).push(attempt.clone());
        if self.stuck {
            return Box::pin(std::future::pending());
        }
        let verdict = lock(&self.entries)
            .iter()
            .find(|entry| entry.target.matches(attempt))
            .cloned()
            .map(|entry| self.verdict(entry));
        Box::pin(async move { Ok(verdict) })
    }

    fn ban(&self, request: BanRequest) -> BoxFuture<'_, Result<BanEntry, ServiceError>> {
        let mut entries = lock(&self.entries);
        let mut entry = BanEntry::new(
            format!("mem{}", entries.len() + 1),
            request.target,
            request.source.unwrap_or(BanSource::System),
        );
        entry.reason = request.reason;
        entries.push(entry.clone());
        Box::pin(async move { Ok(entry) })
    }

    fn unban(
        &self,
        request: UnbanRequest,
    ) -> BoxFuture<'_, Result<Option<BanEntry>, ServiceError>> {
        let mut entries = lock(&self.entries);
        let removed = entries
            .iter()
            .position(|entry| entry.target == request.target)
            .map(|index| entries.remove(index));
        Box::pin(async move { Ok(removed) })
    }

    fn get<'a>(
        &'a self,
        target: &'a BanTarget,
    ) -> BoxFuture<'a, Result<Option<BanEntry>, ServiceError>> {
        let found = lock(&self.entries)
            .iter()
            .find(|entry| &entry.target == target)
            .cloned();
        Box::pin(async move { Ok(found) })
    }

    fn list(&self, _query: BanQuery) -> BoxFuture<'_, Result<BanPage, ServiceError>> {
        let entries = lock(&self.entries).clone();
        Box::pin(async move { Ok(BanPage::new(entries, None)) })
    }

    fn features(&self) -> BanFeatures {
        BanFeatures::new().ip_ranges(self.ranges)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::net::IpAddr;

    use super::*;
    use crate::test_util::block_on;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn seeded_entries_match_and_carry_the_kick_message() {
        let provider = MemoryBanProvider::with([BanEntry::new(
            "seed",
            BanTarget::Username("Griefer".into()),
            BanSource::Console,
        )
        .reason("grief")])
        .kick_message_with(|entry| {
            Component::text(format!("kicked: {}", entry.reason.as_deref().unwrap_or("")))
        })
        .ranges(true);
        let attempt = LoginAttempt::pre_auth(ip("1.2.3.4"), "griefer");
        let verdict = block_on(provider.check(&attempt)).unwrap().unwrap();
        assert_eq!(verdict.kick_message.to_plain(), "kicked: grief");
        assert_eq!(provider.attempts().len(), 1);
        assert!(provider.features().ip_ranges);

        let banned = block_on(provider.ban(BanRequest::new(BanTarget::Ip(ip("9.9.9.9"))))).unwrap();
        assert_eq!(banned.id, "mem2");
        assert!(
            block_on(provider.unban(UnbanRequest::new(BanTarget::Ip(ip("9.9.9.9")))))
                .unwrap()
                .is_some()
        );
        assert_eq!(provider.entries().len(), 1);
    }

    #[test]
    fn a_stuck_provider_records_the_attempt_and_never_answers() {
        let provider = MemoryBanProvider::new().stuck();
        let attempt = LoginAttempt::status(ip("1.1.1.1"));
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let mut future = provider.check(&attempt);
        assert!(future.as_mut().poll(&mut cx).is_pending());
        assert_eq!(provider.attempts().len(), 1);
    }
}
