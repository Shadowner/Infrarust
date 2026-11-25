use std::time::Duration;

use infrarust_api::services::PlayerRegistry;
use infrarust_api::services::proxy_control::{RestartOptions, RestartRequestError};
use infrarust_api::types::Component;
use infrarust_config::{MAX_RESTART_DELAY, MAX_RESTART_MESSAGE, RestartConfig};

use crate::player::registry::PlayerRegistryImpl;

pub const DEFAULT_MESSAGE: &str = "&eThe proxy restarts in {delay}";
pub const IMMINENT_MESSAGE: &str = "&eThe proxy restarts now";
pub const CANCELLED_MESSAGE: &str = "&eThe restart is cancelled";
const REMINDERS: [u64; 6] = [300, 60, 30, 10, 5, 1];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Announcement {
    pub delay: Duration,
    pub message: String,
}

impl Announcement {
    pub fn resolve(options: &RestartOptions, config: &RestartConfig) -> Self {
        let delay = options.delay.unwrap_or(config.delay);
        let chosen = options
            .message
            .as_deref()
            .map_or(config.message.as_str(), str::trim);
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

    pub fn marks(&self) -> Vec<Duration> {
        let mut marks = vec![self.delay];
        marks.extend(
            REMINDERS
                .into_iter()
                .map(Duration::from_secs)
                .filter(|mark| *mark < self.delay),
        );
        marks
    }
}

pub fn validate(options: &RestartOptions) -> Result<(), RestartRequestError> {
    if let Some(delay) = options.delay
        && delay > MAX_RESTART_DELAY
    {
        return Err(RestartRequestError::Invalid(format!(
            "the delay is {}, the longest delay is {}",
            compact(delay),
            compact(MAX_RESTART_DELAY)
        )));
    }
    if let Some(message) = &options.message {
        let length = message.chars().count();
        if length > MAX_RESTART_MESSAGE {
            return Err(RestartRequestError::Invalid(format!(
                "the message is {length} characters long, the limit is {MAX_RESTART_MESSAGE}"
            )));
        }
    }
    Ok(())
}

pub fn render(template: &str, remaining: Duration) -> String {
    let seconds = remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0);
    template
        .replace("{delay}", &compact(Duration::from_secs(seconds)))
        .replace("{seconds}", &seconds.to_string())
}

pub fn compact(duration: Duration) -> String {
    let total = duration.as_secs();
    let (hours, minutes, seconds) = (total / 3600, total % 3600 / 60, total % 60);
    let parts: Vec<String> = [(hours, "h"), (minutes, "m"), (seconds, "s")]
        .into_iter()
        .filter(|(amount, _)| *amount > 0)
        .map(|(amount, unit)| format!("{amount}{unit}"))
        .collect();
    if parts.is_empty() {
        "0s".to_string()
    } else {
        parts.join(" ")
    }
}

pub fn tell(players: &PlayerRegistryImpl, text: &str) -> usize {
    if text.trim().is_empty() {
        return 0;
    }
    let component = Component::from_legacy(text);
    players
        .get_all_players()
        .iter()
        .filter(|player| player.send_message(component.clone()).is_ok())
        .count()
}

#[cfg(test)]
mod tests {
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
    fn a_message_without_a_delay_is_said_once() {
        let announcement = Announcement::resolve(&options(None, Some("Bye")), &config(0, ""));
        assert!(!announcement.is_silent());
        assert_eq!(announcement.marks(), [Duration::ZERO]);
    }

    #[test]
    fn the_message_is_repeated_at_the_marks_below_the_delay() {
        let marks = |delay| {
            Announcement {
                delay: secs(delay),
                message: "x".into(),
            }
            .marks()
            .into_iter()
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
    fn durations_are_written_without_words() {
        assert_eq!(compact(secs(3600)), "1h");
        assert_eq!(compact(secs(3725)), "1h 2m 5s");
        assert_eq!(compact(secs(120)), "2m");
        assert_eq!(compact(secs(59)), "59s");
    }

    #[test]
    fn a_delay_or_a_message_over_the_limit_is_refused() {
        assert_eq!(validate(&options(Some(3600), None)), Ok(()));
        assert!(matches!(
            validate(&options(Some(3601), None)),
            Err(RestartRequestError::Invalid(reason)) if reason.contains("1h")
        ));
        let long = "é".repeat(MAX_RESTART_MESSAGE);
        assert_eq!(validate(&options(None, Some(&long))), Ok(()));
        let longer = "é".repeat(MAX_RESTART_MESSAGE + 1);
        assert!(matches!(
            validate(&options(None, Some(&longer))),
            Err(RestartRequestError::Invalid(reason)) if reason.contains("257")
        ));
    }
}
