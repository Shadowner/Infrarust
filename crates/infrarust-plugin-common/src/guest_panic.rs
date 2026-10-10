pub const GUEST_PANIC_PREFIX: &str = "panicked at ";

pub const MAX_GUEST_PANIC_LEN: usize = 1024;

pub fn guest_panic_line(location: Option<(&str, u32, u32)>, message: &str) -> String {
    match location {
        Some((file, line, column)) => {
            format!("{GUEST_PANIC_PREFIX}{file}:{line}:{column}: {message}")
        }
        None => format!("{GUEST_PANIC_PREFIX}an unknown location: {message}"),
    }
}

pub fn is_guest_panic_line(line: &str) -> bool {
    line.starts_with(GUEST_PANIC_PREFIX)
}

pub fn bounded_guest_panic(mut line: String) -> String {
    if line.len() > MAX_GUEST_PANIC_LEN {
        let mut end = MAX_GUEST_PANIC_LEN;
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        line.truncate(end);
        line.push_str("...");
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_line_names_the_location_and_the_message_and_is_recognised() {
        let line = guest_panic_line(Some(("src/lib.rs", 4, 9)), "boom");
        assert_eq!(line, "panicked at src/lib.rs:4:9: boom");
        assert!(is_guest_panic_line(&line));
        assert!(is_guest_panic_line(&guest_panic_line(None, "boom")));
        assert!(!is_guest_panic_line("the plugin panicked at noon"));
    }

    #[test]
    fn a_long_panic_line_is_cut_on_a_char_boundary() {
        let line = guest_panic_line(None, &"é".repeat(MAX_GUEST_PANIC_LEN));
        let bounded = bounded_guest_panic(line);
        assert!(
            bounded.len() <= MAX_GUEST_PANIC_LEN + 3,
            "{}",
            bounded.len()
        );
        assert!(bounded.ends_with("..."));
        assert_eq!(bounded_guest_panic("short".to_owned()), "short");
    }
}
