use infrarust_api::types::Component;

/// Parse Minecraft `&` color/format codes into a [`Component`] tree.
///
/// Delegates to [`Component::from_legacy()`].
pub fn parse_colored(text: &str) -> Component {
    Component::from_legacy(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_no_codes() {
        let c = parse_colored("Hello world");
        assert_eq!(c.text, "Hello world");
    }

    #[test]
    fn single_color() {
        let c = parse_colored("&aGreen text");
        assert_eq!(c.text, "Green text");
        assert_eq!(c.color.as_deref(), Some("green"));
    }

    #[test]
    fn multiple_segments() {
        let c = parse_colored("&aGreen &cRed");
        assert_eq!(c.text, "Green ");
        assert_eq!(c.color.as_deref(), Some("green"));
        assert_eq!(c.extra.len(), 1);
        assert_eq!(c.extra[0].text, "Red");
        assert_eq!(c.extra[0].color.as_deref(), Some("red"));
    }

    #[test]
    fn bold_formatting() {
        let c = parse_colored("&lBold text");
        assert_eq!(c.text, "Bold text");
        assert_eq!(c.bold, Some(true));
    }

    #[test]
    fn reset_clears_formatting() {
        let c = parse_colored("&c&lBold Red &rPlain");
        assert_eq!(c.text, "Bold Red ");
        assert_eq!(c.color.as_deref(), Some("red"));
        assert_eq!(c.bold, Some(true));
        assert_eq!(c.extra.len(), 1);
        assert_eq!(c.extra[0].text, "Plain");
        assert_eq!(c.extra[0].color, None);
        assert_eq!(c.extra[0].bold, None);
    }

    #[test]
    fn unknown_code_preserved_literally() {
        let c = parse_colored("&zUnknown");
        assert_eq!(c.text, "&zUnknown");
    }

    #[test]
    fn trailing_ampersand() {
        let c = parse_colored("end&");
        assert_eq!(c.text, "end&");
    }

    #[test]
    fn empty_string() {
        let c = parse_colored("");
        assert_eq!(c.text, "");
    }
}
