use infrarust_api::types::Component;

fn color_name(c: char) -> Option<&'static str> {
    match c {
        '0' => Some("black"),
        '1' => Some("dark_blue"),
        '2' => Some("dark_green"),
        '3' => Some("dark_aqua"),
        '4' => Some("dark_red"),
        '5' => Some("dark_purple"),
        '6' => Some("gold"),
        '7' => Some("gray"),
        '8' => Some("dark_gray"),
        '9' => Some("blue"),
        'a' => Some("green"),
        'b' => Some("aqua"),
        'c' => Some("red"),
        'd' => Some("light_purple"),
        'e' => Some("yellow"),
        'f' => Some("white"),
        _ => None,
    }
}

#[derive(Clone)]
struct Fmt {
    color: Option<&'static str>,
    bold: bool,
    italic: bool,
    underlined: bool,
}

impl Fmt {
    fn new() -> Self {
        Self { color: None, bold: false, italic: false, underlined: false }
    }

    fn apply(&self, text: &str) -> Component {
        let mut c = Component::text(text);
        if let Some(color) = self.color { c = c.color(color); }
        if self.bold { c = c.bold(); }
        if self.italic { c = c.italic(); }
        if self.underlined { c = c.underlined(); }
        c
    }
}

/// Parse Minecraft `&` color/format codes into a [`Component`] tree.
pub fn parse_colored(text: &str) -> Component {
    let mut parts: Vec<Component> = Vec::new();
    let mut segment = String::new();
    let mut fmt = Fmt::new();
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '&' {
            if let Some(&code) = chars.peek() {
                let is_format_code = matches!(code, 'l' | 'o' | 'n' | 'r')
                    || color_name(code).is_some();

                if is_format_code {
                    chars.next();

                    if !segment.is_empty() {
                        parts.push(fmt.apply(&segment));
                        segment.clear();
                    }

                    match code {
                        'l' => fmt.bold = true,
                        'o' => fmt.italic = true,
                        'n' => fmt.underlined = true,
                        'r' => fmt = Fmt::new(),
                        c => fmt.color = color_name(c),
                    }
                    continue;
                }
            }
            segment.push('&');
        } else {
            segment.push(ch);
        }
    }

    if !segment.is_empty() {
        parts.push(fmt.apply(&segment));
    }

    match parts.len() {
        0 => Component::text(""),
        1 => parts.remove(0),
        _ => {
            let mut root = parts.remove(0);
            for part in parts {
                root = root.append(part);
            }
            root
        }
    }
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
