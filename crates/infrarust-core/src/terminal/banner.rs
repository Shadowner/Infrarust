use std::fmt::Write as _;

use infrarust_protocol::version::ProtocolVersion;

use super::{Tone, color};

const CUBE: [&str; 5] = [
    "    .......",
    "  ---.....+++",
    "  -----.+++++",
    "  ----- +++++",
    "    --- +++",
];

const TEXT_COLUMN: usize = 16;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Banner {
    rows: Vec<(String, String)>,
}

impl Banner {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn row(mut self, label: impl Into<String>, value: impl ToString) -> Self {
        self.rows.push((label.into(), value.to_string()));
        self
    }

    pub fn render(&self, colored: bool) -> String {
        let mut right = vec![title(colored), tagline(colored), String::new()];
        let width = self
            .rows
            .iter()
            .map(|(label, _)| label.chars().count())
            .max()
            .unwrap_or(0)
            + 2;
        right.extend(self.rows.iter().map(|(label, value)| {
            format!(
                "{}{value}",
                Tone::Muted.paint(format!("{label:<width$}"), colored)
            )
        }));

        let lines = CUBE.len().max(right.len());
        let mut out = String::new();
        for index in 0..lines {
            let cube = CUBE.get(index).copied().unwrap_or("");
            let text = right.get(index).map(String::as_str).unwrap_or("");
            out.push_str(&paint_cube(cube, colored));
            if !text.is_empty() {
                let pad = TEXT_COLUMN.saturating_sub(cube.chars().count());
                let _ = write!(out, "{:pad$}{text}", "");
            }
            out.push('\n');
        }
        out
    }

    #[allow(clippy::print_stdout)]
    pub fn print(&self) {
        println!("{}", self.render(color::enabled()));
    }
}

fn title(colored: bool) -> String {
    format!(
        "{} {}",
        Tone::Title.paint("Infrarust", colored),
        Tone::Muted.paint(env!("CARGO_PKG_VERSION"), colored)
    )
}

fn tagline(colored: bool) -> String {
    let (dot, arrow) = if colored { ("·", "→") } else { ("-", "->") };
    let mut names = ProtocolVersion::SUPPORTED
        .iter()
        .map(|version| version.name())
        .filter(|name| !matches!(*name, "legacy" | "unknown"));
    let oldest = names.next().unwrap_or("?");
    let newest = names.next_back().unwrap_or(oldest);
    Tone::Muted
        .paint(
            format!("Minecraft proxy {dot} {oldest} {arrow} {newest}"),
            colored,
        )
        .to_string()
}

fn cube_tone(glyph: char) -> Tone {
    match glyph {
        '.' => Tone::Gold,
        '-' => Tone::Brand,
        '+' => Tone::Rust,
        _ => Tone::Plain,
    }
}

fn paint_cube(row: &str, colored: bool) -> String {
    let mut out = String::new();
    let mut rest = row;
    while let Some(first) = rest.chars().next() {
        let run = rest.find(|c| c != first).unwrap_or(rest.len());
        let (chunk, tail) = rest.split_at(run);
        if first == ' ' {
            out.push_str(chunk);
        } else {
            let _ = write!(out, "{}", cube_tone(first).paint(chunk, colored));
        }
        rest = tail;
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use infrarust_protocol::version::ProtocolVersion;

    use super::{Banner, TEXT_COLUMN};

    const VERSION: &str = env!("CARGO_PKG_VERSION");

    fn newest() -> &'static str {
        ProtocolVersion::HIGHEST_KNOWN.name()
    }

    fn sample() -> Banner {
        Banner::new()
            .row("listen", "0.0.0.0:25565")
            .row("servers", "./servers")
            .row("plugins", "./plugins")
            .row("workers", "auto")
    }

    #[test]
    fn an_empty_banner_is_the_cube_and_the_title() {
        let newest = newest();
        let expected = format!(
            "    .......     Infrarust {VERSION}\n  ---.....+++   Minecraft proxy - 1.7.2 -> {newest}\n  -----.+++++\n  ----- +++++\n    --- +++\n"
        );
        assert_eq!(Banner::new().render(false), expected);
    }

    #[test]
    fn rows_past_the_cube_continue_in_the_text_column() {
        let newest = newest();
        let expected = format!(
            "    .......     Infrarust {VERSION}\n  ---.....+++   Minecraft proxy - 1.7.2 -> {newest}\n  -----.+++++\n  ----- +++++   listen   0.0.0.0:25565\n    --- +++     servers  ./servers\n                plugins  ./plugins\n                workers  auto\n"
        );
        assert_eq!(sample().render(false), expected);
    }

    #[test]
    fn every_text_starts_at_the_same_column() {
        for line in sample().render(false).lines() {
            if line.chars().count() > TEXT_COLUMN {
                let (cube, text) = line.split_at(TEXT_COLUMN);
                assert!(cube.ends_with(' '), "{line:?}");
                assert!(!text.starts_with(' '), "{line:?}");
            }
        }
    }

    #[test]
    fn the_uncolored_banner_is_plain_ascii() {
        let rendered = sample().render(false);
        assert!(rendered.is_ascii(), "{rendered:?}");
        assert!(!rendered.contains('\u{1b}'));
    }

    #[test]
    fn the_colored_banner_uses_the_unicode_tagline() {
        let rendered = sample().render(true);
        assert!(rendered.contains('\u{1b}'));
        let stripped = console::strip_ansi_codes(&rendered);
        let expected = sample()
            .render(false)
            .replace("proxy - ", "proxy · ")
            .replace(" -> ", " → ");
        assert_eq!(stripped, expected);
    }
}
