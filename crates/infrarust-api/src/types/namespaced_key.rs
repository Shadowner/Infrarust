pub fn is_namespace_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.')
}

pub fn is_valid(namespace: &str, path: &str) -> bool {
    !namespace.is_empty()
        && !path.is_empty()
        && namespace.chars().all(is_namespace_char)
        && path.chars().all(|c| is_namespace_char(c) || c == '/')
}

pub fn parse(key: &str) -> Option<(&str, &str)> {
    key.split_once(':')
        .filter(|(namespace, path)| is_valid(namespace, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_a_lowercase_namespace_and_path() {
        assert_eq!(parse("minecraft:brand"), Some(("minecraft", "brand")));
        assert_eq!(
            parse("infrarust:auth/session"),
            Some(("infrarust", "auth/session"))
        );
        assert_eq!(parse("a.b-c_1:d.e-f_2"), Some(("a.b-c_1", "d.e-f_2")));
        for bad in ["brand", "Upper:case", ":x", "x:", "a/b:c", "a:b:c", "a:b c"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
        assert!(!is_valid("", "x"));
        assert!(!is_valid("x", ""));
    }
}
