use std::fmt;

pub const MAX_PLUGIN_ID_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidPluginId {
    shown: String,
    problem: Problem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Problem {
    Empty,
    TooLong,
    BadStart,
    BadChar(char),
}

impl InvalidPluginId {
    fn new(id: &str, problem: Problem) -> Self {
        Self {
            shown: shown(id),
            problem,
        }
    }
}

fn shown(id: &str) -> String {
    let mut out = String::new();
    for (at, ch) in id.chars().enumerate() {
        if at == MAX_PLUGIN_ID_LEN {
            out.push('…');
            break;
        }
        out.extend(ch.escape_debug());
    }
    out
}

impl fmt::Display for InvalidPluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let id = &self.shown;
        match self.problem {
            Problem::Empty => f.write_str("a plugin id cannot be empty"),
            Problem::TooLong => write!(
                f,
                "plugin id `{id}` is longer than {MAX_PLUGIN_ID_LEN} characters"
            ),
            Problem::BadStart => write!(
                f,
                "plugin id `{id}` must start with a lowercase letter or a digit"
            ),
            Problem::BadChar(bad) => write!(
                f,
                "plugin id `{id}` contains `{}`; use lowercase letters, digits, `-` and `_`",
                bad.escape_debug()
            ),
        }
    }
}

impl std::error::Error for InvalidPluginId {}

pub fn validate_plugin_id(id: &str) -> Result<(), InvalidPluginId> {
    let Some(first) = id.chars().next() else {
        return Err(InvalidPluginId::new(id, Problem::Empty));
    };
    if id.len() > MAX_PLUGIN_ID_LEN {
        return Err(InvalidPluginId::new(id, Problem::TooLong));
    }
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return Err(InvalidPluginId::new(id, Problem::BadStart));
    }
    if let Some(bad) = id
        .chars()
        .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-' || *c == '_'))
    {
        return Err(InvalidPluginId::new(id, Problem::BadChar(bad)));
    }
    Ok(())
}

#[must_use]
pub fn is_valid_plugin_id(id: &str) -> bool {
    validate_plugin_id(id).is_ok()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn ids_inside_the_rule_are_accepted() {
        let longest = "a".repeat(MAX_PLUGIN_ID_LEN);
        for good in ["stats", "admin-api", "a1_b2", "0day", "x", longest.as_str()] {
            assert_eq!(validate_plugin_id(good), Ok(()), "{good}");
            assert!(is_valid_plugin_id(good), "{good}");
        }
    }

    #[test]
    fn ids_outside_the_rule_are_refused() {
        let too_long = "x".repeat(MAX_PLUGIN_ID_LEN + 1);
        for bad in [
            "",
            "-lead",
            "_lead",
            "Upper",
            "sp ace",
            "dot.ted",
            ".cache",
            "..",
            "../escape",
            "/etc",
            "a/b",
            "a\\b",
            "café-plugin",
            "nul\0byte",
            too_long.as_str(),
        ] {
            assert!(validate_plugin_id(bad).is_err(), "{bad:?}");
            assert!(!is_valid_plugin_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn each_refusal_says_what_is_wrong() {
        let message = |id: &str| validate_plugin_id(id).unwrap_err().to_string();
        assert_eq!(message(""), "a plugin id cannot be empty");
        assert_eq!(
            message("Upper"),
            "plugin id `Upper` must start with a lowercase letter or a digit"
        );
        assert_eq!(
            message("../escape"),
            "plugin id `../escape` must start with a lowercase letter or a digit"
        );
        assert_eq!(
            message("up/../escape"),
            "plugin id `up/../escape` contains `/`; use lowercase letters, digits, `-` and `_`"
        );
        assert_eq!(
            message("café"),
            "plugin id `café` contains `é`; use lowercase letters, digits, `-` and `_`"
        );
        assert!(message(&"y".repeat(65)).contains("is longer than 64 characters"));
    }

    #[test]
    fn a_hostile_id_is_escaped_and_cut_in_the_message() {
        let message = validate_plugin_id("evil\nline").unwrap_err().to_string();
        assert!(message.contains("evil\\nline"), "{message}");
        assert!(!message.contains('\n'), "{message}");

        let huge = format!("a{}", "\u{7}".repeat(100_000));
        let message = validate_plugin_id(&huge).unwrap_err().to_string();
        assert!(message.len() < 1024, "{} bytes", message.len());
        assert!(message.contains('…'), "{message}");
    }
}
