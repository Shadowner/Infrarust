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
    use infrarust_api::types::NamedColor;

    #[test]
    fn plain_text_no_codes() {
        let c = parse_colored("Hello world");
        assert_eq!(c.as_text(), Some("Hello world"));
    }

    #[test]
    fn single_color() {
        let c = parse_colored("&aGreen text");
        assert_eq!(c.as_text(), Some("Green text"));
        assert_eq!(c.style.color, Some(NamedColor::Green.into()));
    }

    #[test]
    fn multiple_segments() {
        let c = parse_colored("&aGreen &cRed");
        assert_eq!(c.as_text(), Some(""));
        assert!(c.style.is_empty());
        assert_eq!(c.children.len(), 2);
        assert_eq!(c.children[0].as_text(), Some("Green "));
        assert_eq!(c.children[0].style.color, Some(NamedColor::Green.into()));
        assert_eq!(c.children[1].as_text(), Some("Red"));
        assert_eq!(c.children[1].style.color, Some(NamedColor::Red.into()));
    }

    #[test]
    fn bold_formatting() {
        let c = parse_colored("&lBold text");
        assert_eq!(c.as_text(), Some("Bold text"));
        assert_eq!(c.style.bold, Some(true));
    }

    #[test]
    fn reset_clears_formatting() {
        let c = parse_colored("&c&lBold Red &rPlain");
        assert_eq!(c.children.len(), 2);
        assert_eq!(c.children[0].as_text(), Some("Bold Red "));
        assert_eq!(c.children[0].style.color, Some(NamedColor::Red.into()));
        assert_eq!(c.children[0].style.bold, Some(true));
        assert_eq!(c.children[1].as_text(), Some("Plain"));
        assert!(c.children[1].style.is_empty());
    }

    #[test]
    fn unknown_code_preserved_literally() {
        let c = parse_colored("&zUnknown");
        assert_eq!(c.as_text(), Some("&zUnknown"));
    }

    #[test]
    fn trailing_ampersand() {
        let c = parse_colored("end&");
        assert_eq!(c.as_text(), Some("end&"));
    }

    #[test]
    fn empty_string() {
        let c = parse_colored("");
        assert_eq!(c.as_text(), Some(""));
    }
}
