use super::LEGACY_SECTION;

pub(super) fn is_valid_identifier(value: &str) -> bool {
    let (namespace, path) = value.split_once(':').unwrap_or(("", value));
    !path.is_empty()
        && namespace
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.'))
        && path
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/'))
}

pub(super) fn is_chat_safe(value: &str) -> bool {
    value
        .chars()
        .all(|c| c >= ' ' && c != '\u{7f}' && c != LEGACY_SECTION)
}

pub(super) fn is_valid_untrusted_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once(':') else {
        return false;
    };
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")) {
        return false;
    }
    if rest.is_empty() || rest == "//" {
        return false;
    }
    let chars: Vec<char> = rest.chars().collect();
    let mut fragment = false;
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        match c {
            '%' => {
                let hex =
                    |offset: usize| chars.get(i + offset).is_some_and(char::is_ascii_hexdigit);
                if !(hex(1) && hex(2)) {
                    return false;
                }
                i += 3;
                continue;
            }
            '#' => {
                if fragment {
                    return false;
                }
                fragment = true;
            }
            c if c.is_ascii_alphanumeric() => {}
            '-' | '_' | '.' | '!' | '~' | '*' | '\'' | '(' | ')' | ';' | '/' | '?' | ':' | '@'
            | '&' | '=' | '+' | '$' | ',' => {}
            c if c.is_ascii() => return false,
            c if c.is_control() || c.is_whitespace() => return false,
            _ => {}
        }
        i += 1;
    }
    true
}
