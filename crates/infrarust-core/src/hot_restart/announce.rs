use std::time::Duration;

use infrarust_api::services::PlayerRegistry;
use infrarust_api::services::proxy_control::{RestartOptions, RestartRequestError};
use infrarust_config::{RestartConfig, RestartLimit};

use crate::commands::actions::deliver;
use crate::console::parser::format_duration_short;
use crate::util::whole_seconds;

pub const DEFAULT_MESSAGE: &str = "&eThe proxy restarts in {delay}";
pub const CANCELLED_MESSAGE: &str = "&eThe restart is cancelled";
const REMINDERS: [Duration; 6] = [
    Duration::from_secs(300),
    Duration::from_secs(60),
    Duration::from_secs(30),
    Duration::from_secs(10),
    Duration::from_secs(5),
    Duration::from_secs(1),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Announcement {
    pub delay: Duration,
    pub message: String,
}

impl Announcement {
    pub fn resolve(options: &RestartOptions, config: &RestartConfig) -> Self {
        let delay = options.delay.unwrap_or(config.delay);
        let chosen = options.message.as_deref().unwrap_or(&config.message).trim();
        let message = if !chosen.is_empty() {
            chosen.to_string()
        } else if delay.is_zero() {
            String::new()
        } else {
            DEFAULT_MESSAGE.to_string()
        };
        Self { delay, message }
    }

    pub const fn is_silent(&self) -> bool {
        self.delay.is_zero() && self.message.is_empty()
    }

    pub fn text(&self, remaining: Duration) -> String {
        render(&self.message, remaining)
    }

    pub fn marks(&self) -> impl Iterator<Item = Duration> + '_ {
        std::iter::once(self.delay).chain(REMINDERS.into_iter().filter(|mark| *mark < self.delay))
    }
}

pub(crate) fn validate(options: &RestartOptions) -> Result<(), RestartRequestError> {
    RestartLimit::check(options.delay, options.message.as_deref())
        .map_err(|limit| RestartRequestError::Invalid(limit.to_string()))
}

fn render(template: &str, remaining: Duration) -> String {
    let remaining = whole_seconds(remaining);
    template
        .replace("{delay}", &format_duration_short(remaining))
        .replace("{seconds}", &remaining.as_secs().to_string())
}

pub(crate) fn tell(players: &dyn PlayerRegistry, text: &str) -> usize {
    if text.trim().is_empty() {
        return 0;
    }
    deliver(&players.get_all_players(), text)
}

#[cfg(test)]
mod tests {
    use infrarust_config::{MAX_RESTART_DELAY, MAX_RESTART_MESSAGE};

    use super::*;

    const fn secs(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    fn config(delay: u64, message: &str) -> RestartConfig {
        RestartConfig {
            delay: secs(delay),
            message: message.to_string(),
            ..RestartConfig::default()
        }
    }

    fn options(delay: Option<u64>, message: Option<&str>) -> RestartOptions {
        RestartOptions {
            delay: delay.map(secs),
            message: message.map(str::to_string),
        }
    }

    #[test]
    fn a_plain_request_follows_the_configuration() {
        let silent = Announcement::resolve(&options(None, None), &config(0, ""));
        assert!(silent.is_silent());
        let configured = Announcement::resolve(&options(None, None), &config(30, "&cSoon"));
        assert_eq!(
            configured,
            Announcement {
                delay: secs(30),
                message: "&cSoon".into()
            }
        );
    }

    #[test]
    fn the_request_overrides_the_configuration_field_by_field() {
        let configured = config(30, "&cSoon");
        let now = Announcement::resolve(&options(Some(0), None), &configured);
        assert_eq!(now.delay, Duration::ZERO);
        assert_eq!(now.message, "&cSoon");
        let other = Announcement::resolve(&options(None, Some("  Back in a minute ")), &configured);
        assert_eq!(other.delay, secs(30));
        assert_eq!(other.message, "Back in a minute");
    }

    #[test]
    fn a_delay_without_a_message_gets_the_default_notice() {
        let announcement = Announcement::resolve(&options(Some(10), None), &config(0, ""));
        assert_eq!(announcement.message, DEFAULT_MESSAGE);
        let blank = Announcement::resolve(&options(Some(10), Some("   ")), &config(0, "&cSoon"));
        assert_eq!(blank.message, DEFAULT_MESSAGE);
    }

    #[test]
    fn a_blank_configured_message_counts_as_none() {
        let silent = Announcement::resolve(&options(None, None), &config(0, "   "));
        assert!(silent.is_silent());
        let delayed = Announcement::resolve(&options(None, None), &config(10, " \t "));
        assert_eq!(delayed.message, DEFAULT_MESSAGE);
        let padded = Announcement::resolve(&options(None, None), &config(0, "  &cSoon "));
        assert_eq!(padded.message, "&cSoon");
    }

    #[test]
    fn a_message_without_a_delay_is_said_once() {
        let announcement = Announcement::resolve(&options(None, Some("Bye")), &config(0, ""));
        assert!(!announcement.is_silent());
        assert_eq!(announcement.marks().collect::<Vec<_>>(), [Duration::ZERO]);
    }

    #[test]
    fn the_message_is_repeated_at_the_marks_below_the_delay() {
        let marks = |delay| {
            Announcement {
                delay: secs(delay),
                message: "x".into(),
            }
            .marks()
            .map(|mark| mark.as_secs())
            .collect::<Vec<_>>()
        };
        assert_eq!(marks(600), [600, 300, 60, 30, 10, 5, 1]);
        assert_eq!(marks(60), [60, 30, 10, 5, 1]);
        assert_eq!(marks(45), [45, 30, 10, 5, 1]);
        assert_eq!(marks(5), [5, 1]);
        assert_eq!(marks(1), [1]);
    }

    #[test]
    fn placeholders_show_the_time_left() {
        assert_eq!(render("in {delay}", secs(90)), "in 1m 30s");
        assert_eq!(render("in {seconds} s", secs(90)), "in 90 s");
        assert_eq!(render("{delay} / {delay}", secs(5)), "5s / 5s");
        assert_eq!(render("no placeholder", secs(5)), "no placeholder");
        assert_eq!(render("in {delay}", Duration::from_millis(4200)), "in 5s");
        assert_eq!(render("in {delay}", Duration::ZERO), "in 0s");
    }

    #[test]
    fn every_delay_within_the_limit_is_written_in_full() {
        assert_eq!(render("{delay}", MAX_RESTART_DELAY), "1h");
        assert_eq!(render("{delay}", secs(3599)), "59m 59s");
        assert_eq!(render("{delay}", secs(120)), "2m");
        assert_eq!(render("{delay}", secs(59)), "59s");
        for seconds in 0..=MAX_RESTART_DELAY.as_secs() {
            assert_eq!(
                render("{delay}", secs(seconds)),
                humantime::format_duration(secs(seconds)).to_string()
            );
        }
    }

    #[test]
    fn a_delay_or_a_message_over_the_limit_is_refused() {
        assert_eq!(validate(&options(Some(3600), None)), Ok(()));
        assert!(matches!(
            validate(&options(Some(3601), None)),
            Err(RestartRequestError::Invalid(reason))
                if reason == "the delay is 1h 1s, the longest delay is 1h"
        ));
        let long = "é".repeat(MAX_RESTART_MESSAGE);
        assert_eq!(validate(&options(None, Some(&long))), Ok(()));
        let longer = "é".repeat(MAX_RESTART_MESSAGE + 1);
        assert!(matches!(
            validate(&options(None, Some(&longer))),
            Err(RestartRequestError::Invalid(reason))
                if reason == "the message is 257 characters long, the limit is 256"
        ));
    }
}
