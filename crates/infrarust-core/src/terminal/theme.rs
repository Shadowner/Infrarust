use std::fmt::Display;

use console::{Style, StyledObject};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Strong,
    Brand,
    Title,
    Gold,
    Heading,
    Rust,
    Entity,
    Muted,
    Ok,
    Warn,
    Err,
}

const BRAND: (u8, u8, u8, u8) = (227, 139, 28, 208);
const GOLD: (u8, u8, u8, u8) = (242, 181, 68, 214);
const RUST: (u8, u8, u8, u8) = (196, 97, 14, 166);

fn rgb(style: Style, (r, g, b, fallback): (u8, u8, u8, u8)) -> Style {
    if console::true_colors_enabled() {
        style.true_color(r, g, b)
    } else {
        style.color256(fallback)
    }
}

impl Tone {
    pub fn style(self) -> Style {
        let style = Style::new();
        match self {
            Self::Plain => style,
            Self::Strong => style.bold(),
            Self::Brand => rgb(style, BRAND),
            Self::Title => rgb(style, BRAND).bold(),
            Self::Gold => rgb(style, GOLD),
            Self::Heading => rgb(style, GOLD).bold(),
            Self::Rust => rgb(style, RUST),
            Self::Entity => style.cyan(),
            Self::Muted => style.dim(),
            Self::Ok => style.green(),
            Self::Warn => style.yellow(),
            Self::Err => style.red(),
        }
    }

    pub fn paint<D: Display>(self, text: D, colored: bool) -> StyledObject<D> {
        self.style().apply_to(text).force_styling(colored)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Up,
    Idle,
    Busy,
    Down,
}

impl Mark {
    pub const fn glyph(self) -> &'static str {
        match self {
            Self::Up => "●",
            Self::Idle => "○",
            Self::Busy => "◌",
            Self::Down => "✕",
        }
    }

    pub const fn tone(self) -> Tone {
        match self {
            Self::Up => Tone::Ok,
            Self::Idle => Tone::Muted,
            Self::Busy => Tone::Warn,
            Self::Down => Tone::Err,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Mark, Tone};

    #[test]
    fn painting_without_color_leaves_the_text_untouched() {
        assert_eq!(
            Tone::Title.paint("Infrarust", false).to_string(),
            "Infrarust"
        );
    }

    #[test]
    fn painting_with_color_wraps_the_text_in_escape_codes() {
        let painted = Tone::Err.paint("boom", true).to_string();
        assert!(painted.starts_with("\u{1b}["), "{painted:?}");
        assert!(painted.contains("boom"));
        assert_eq!(console::strip_ansi_codes(&painted), "boom");
    }

    #[test]
    fn every_mark_has_a_single_column_glyph() {
        for mark in [Mark::Up, Mark::Idle, Mark::Busy, Mark::Down] {
            assert_eq!(console::measure_text_width(mark.glyph()), 1, "{mark:?}");
        }
    }
}
