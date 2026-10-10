use std::env;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

impl ColorMode {
    pub const ENV: &'static str = "INFRARUST_COLOR";

    pub fn from_env() -> Self {
        Self::resolve(|key| env::var(key).ok())
    }

    pub fn resolve(var: impl Fn(&str) -> Option<String>) -> Self {
        if let Some(mode) = var(Self::ENV).as_deref().and_then(Self::parse) {
            return mode;
        }
        if var("NO_COLOR").is_some_and(|value| !value.is_empty()) {
            return Self::Never;
        }
        let forced = |key: &str| var(key).is_some_and(|value| !value.is_empty() && value != "0");
        if forced("FORCE_COLOR") || forced("CLICOLOR_FORCE") {
            return Self::Always;
        }
        Self::Auto
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "always" | "on" | "true" | "1" => Some(Self::Always),
            "never" | "off" | "false" | "0" => Some(Self::Never),
            _ => None,
        }
    }

    pub fn apply(self) {
        match self {
            Self::Auto => {}
            Self::Always => {
                console::set_colors_enabled(true);
                console::set_true_colors_enabled(
                    env::var("COLORTERM")
                        .is_ok_and(|value| matches!(value.as_str(), "truecolor" | "24bit")),
                );
            }
            Self::Never => {
                console::set_colors_enabled(false);
                console::set_true_colors_enabled(false);
            }
        }
    }
}

pub fn init() -> ColorMode {
    let mode = ColorMode::from_env();
    mode.apply();
    mode
}

pub fn enabled() -> bool {
    console::colors_enabled()
}

#[cfg(test)]
mod tests {
    use super::ColorMode;

    fn resolve(pairs: &[(&str, &str)]) -> ColorMode {
        ColorMode::resolve(|key| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_string())
        })
    }

    #[test]
    fn nothing_set_means_auto() {
        assert_eq!(resolve(&[]), ColorMode::Auto);
    }

    #[test]
    fn the_infrarust_variable_wins_over_the_conventions() {
        assert_eq!(
            resolve(&[("INFRARUST_COLOR", "always"), ("NO_COLOR", "1")]),
            ColorMode::Always
        );
        assert_eq!(
            resolve(&[("INFRARUST_COLOR", "never"), ("FORCE_COLOR", "1")]),
            ColorMode::Never
        );
    }

    #[test]
    fn an_unreadable_infrarust_value_falls_back_to_the_conventions() {
        assert_eq!(
            resolve(&[("INFRARUST_COLOR", "sometimes"), ("NO_COLOR", "1")]),
            ColorMode::Never
        );
    }

    #[test]
    fn no_color_turns_colors_off_unless_empty() {
        assert_eq!(resolve(&[("NO_COLOR", "1")]), ColorMode::Never);
        assert_eq!(resolve(&[("NO_COLOR", "")]), ColorMode::Auto);
    }

    #[test]
    fn force_color_and_clicolor_force_turn_colors_on() {
        assert_eq!(resolve(&[("FORCE_COLOR", "1")]), ColorMode::Always);
        assert_eq!(resolve(&[("CLICOLOR_FORCE", "1")]), ColorMode::Always);
        assert_eq!(resolve(&[("FORCE_COLOR", "0")]), ColorMode::Auto);
    }

    #[test]
    fn parse_accepts_the_usual_spellings() {
        assert_eq!(ColorMode::parse(" Always "), Some(ColorMode::Always));
        assert_eq!(ColorMode::parse("off"), Some(ColorMode::Never));
        assert_eq!(ColorMode::parse("auto"), Some(ColorMode::Auto));
        assert_eq!(ColorMode::parse("maybe"), None);
    }
}
