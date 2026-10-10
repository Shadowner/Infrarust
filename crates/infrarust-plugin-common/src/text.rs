use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NamedColor {
    Black,
    DarkBlue,
    DarkGreen,
    DarkAqua,
    DarkRed,
    DarkPurple,
    Gold,
    Gray,
    DarkGray,
    Blue,
    Green,
    Aqua,
    Red,
    LightPurple,
    Yellow,
    White,
}

impl NamedColor {
    pub const ALL: [Self; 16] = [
        Self::Black,
        Self::DarkBlue,
        Self::DarkGreen,
        Self::DarkAqua,
        Self::DarkRed,
        Self::DarkPurple,
        Self::Gold,
        Self::Gray,
        Self::DarkGray,
        Self::Blue,
        Self::Green,
        Self::Aqua,
        Self::Red,
        Self::LightPurple,
        Self::Yellow,
        Self::White,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Black => "black",
            Self::DarkBlue => "dark_blue",
            Self::DarkGreen => "dark_green",
            Self::DarkAqua => "dark_aqua",
            Self::DarkRed => "dark_red",
            Self::DarkPurple => "dark_purple",
            Self::Gold => "gold",
            Self::Gray => "gray",
            Self::DarkGray => "dark_gray",
            Self::Blue => "blue",
            Self::Green => "green",
            Self::Aqua => "aqua",
            Self::Red => "red",
            Self::LightPurple => "light_purple",
            Self::Yellow => "yellow",
            Self::White => "white",
        }
    }

    pub const fn rgb(self) -> u32 {
        match self {
            Self::Black => 0x00_0000,
            Self::DarkBlue => 0x00_00AA,
            Self::DarkGreen => 0x00_AA00,
            Self::DarkAqua => 0x00_AAAA,
            Self::DarkRed => 0xAA_0000,
            Self::DarkPurple => 0xAA_00AA,
            Self::Gold => 0xFF_AA00,
            Self::Gray => 0xAA_AAAA,
            Self::DarkGray => 0x55_5555,
            Self::Blue => 0x55_55FF,
            Self::Green => 0x55_FF55,
            Self::Aqua => 0x55_FFFF,
            Self::Red => 0xFF_5555,
            Self::LightPurple => 0xFF_55FF,
            Self::Yellow => 0xFF_FF55,
            Self::White => 0xFF_FFFF,
        }
    }

    pub const fn legacy_code(self) -> char {
        match self {
            Self::Black => '0',
            Self::DarkBlue => '1',
            Self::DarkGreen => '2',
            Self::DarkAqua => '3',
            Self::DarkRed => '4',
            Self::DarkPurple => '5',
            Self::Gold => '6',
            Self::Gray => '7',
            Self::DarkGray => '8',
            Self::Blue => '9',
            Self::Green => 'a',
            Self::Aqua => 'b',
            Self::Red => 'c',
            Self::LightPurple => 'd',
            Self::Yellow => 'e',
            Self::White => 'f',
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|color| color.name().eq_ignore_ascii_case(name))
    }

    pub fn from_legacy_code(code: char) -> Option<Self> {
        let code = code.to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|color| color.legacy_code() == code)
    }

    pub fn nearest(rgb: u32) -> Self {
        let target = hsv(rgb);
        let mut best = Self::Black;
        let mut best_distance = f32::MAX;
        for candidate in Self::ALL {
            let distance = hsv_distance(target, hsv(candidate.rgb()));
            if distance < best_distance {
                best = candidate;
                best_distance = distance;
            }
            if distance == 0.0 {
                break;
            }
        }
        best
    }
}

fn hsv(rgb: u32) -> (f32, f32, f32) {
    let channel = |shift: u32| ((rgb >> shift) & 0xFF) as f32 / 255.0;
    let (r, g, b) = (channel(16), channel(8), channel(0));
    let min = r.min(g.min(b));
    let max = r.max(g.max(b));
    let delta = max - min;
    let saturation = if max != 0.0 { delta / max } else { 0.0 };
    if saturation == 0.0 {
        return (0.0, saturation, max);
    }
    let mut hue = if r == max {
        (g - b) / delta
    } else if g == max {
        2.0 + (b - r) / delta
    } else {
        4.0 + (r - g) / delta
    };
    hue *= 60.0;
    if hue < 0.0 {
        hue += 360.0;
    }
    (hue / 360.0, saturation, max)
}

fn hsv_distance(a: (f32, f32, f32), b: (f32, f32, f32)) -> f32 {
    let hue_gap = (a.0 - b.0).abs();
    let hue = 3.0 * hue_gap.min(1.0 - hue_gap);
    let saturation = a.1 - b.1;
    let value = a.2 - b.2;
    hue * hue + saturation * saturation + value * value
}

impl fmt::Display for NamedColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextColor {
    Named(NamedColor),
    Hex(u32),
}

impl TextColor {
    pub fn parse(input: &str) -> Option<Self> {
        if let Some(hex) = input.strip_prefix('#') {
            let digits = hex.strip_prefix('+').unwrap_or(hex);
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let significant = digits.trim_start_matches('0');
            if significant.len() > 6 {
                return None;
            }
            if significant.is_empty() {
                return Some(Self::Hex(0));
            }
            return u32::from_str_radix(significant, 16).ok().map(Self::Hex);
        }
        NamedColor::from_name(input).map(Self::Named)
    }

    pub const fn rgb(self) -> u32 {
        match self {
            Self::Named(named) => named.rgb(),
            Self::Hex(value) => value & 0xFF_FFFF,
        }
    }

    pub fn downsample(self) -> NamedColor {
        match self {
            Self::Named(named) => named,
            Self::Hex(value) => NamedColor::nearest(value & 0xFF_FFFF),
        }
    }
}

impl fmt::Display for TextColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Named(named) => f.write_str(named.name()),
            Self::Hex(value) => write!(f, "#{:06X}", value & 0xFF_FFFF),
        }
    }
}

impl From<NamedColor> for TextColor {
    fn from(color: NamedColor) -> Self {
        Self::Named(color)
    }
}

pub trait IntoTextColor {
    fn into_text_color(self) -> Option<TextColor>;
}

impl IntoTextColor for TextColor {
    fn into_text_color(self) -> Option<TextColor> {
        Some(self)
    }
}

impl IntoTextColor for NamedColor {
    fn into_text_color(self) -> Option<TextColor> {
        Some(TextColor::Named(self))
    }
}

impl IntoTextColor for &str {
    fn into_text_color(self) -> Option<TextColor> {
        TextColor::parse(self)
    }
}

impl IntoTextColor for String {
    fn into_text_color(self) -> Option<TextColor> {
        TextColor::parse(&self)
    }
}

impl IntoTextColor for &String {
    fn into_text_color(self) -> Option<TextColor> {
        TextColor::parse(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Decoration {
    Obfuscated,
    Bold,
    Strikethrough,
    Underlined,
    Italic,
}

impl Decoration {
    pub const ALL: [Self; 5] = [
        Self::Obfuscated,
        Self::Bold,
        Self::Strikethrough,
        Self::Underlined,
        Self::Italic,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Obfuscated => "obfuscated",
            Self::Bold => "bold",
            Self::Strikethrough => "strikethrough",
            Self::Underlined => "underlined",
            Self::Italic => "italic",
        }
    }

    pub const fn legacy_code(self) -> char {
        match self {
            Self::Obfuscated => 'k',
            Self::Bold => 'l',
            Self::Strikethrough => 'm',
            Self::Underlined => 'n',
            Self::Italic => 'o',
        }
    }

    pub fn from_legacy_code(code: char) -> Option<Self> {
        let code = code.to_ascii_lowercase();
        Self::ALL.into_iter().find(|d| d.legacy_code() == code)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn downsampling_matches_adventure_nearest_color() {
        assert_eq!(NamedColor::nearest(0xFF_0000), NamedColor::DarkRed);
        assert_eq!(NamedColor::nearest(0xAB_2211), NamedColor::DarkRed);
        assert_eq!(NamedColor::nearest(0xEC_41AA), NamedColor::LightPurple);
        assert_eq!(NamedColor::nearest(0x80_8080), NamedColor::Gray);
        for named in NamedColor::ALL {
            assert_eq!(NamedColor::nearest(named.rgb()), named);
        }
        assert_eq!(TextColor::Hex(0xAB_2211).downsample(), NamedColor::DarkRed);
        assert_eq!(
            TextColor::Named(NamedColor::Aqua).downsample(),
            NamedColor::Aqua
        );
    }

    #[test]
    fn text_color_parsing_follows_vanilla() {
        assert_eq!(TextColor::parse("#FF5555"), Some(TextColor::Hex(0xFF_5555)));
        assert_eq!(TextColor::parse("#f"), Some(TextColor::Hex(0xF)));
        assert_eq!(
            TextColor::parse("#0000FFFFFF"),
            Some(TextColor::Hex(0xFF_FFFF))
        );
        assert_eq!(TextColor::parse("#1000000"), None);
        assert_eq!(TextColor::parse("#"), None);
        assert_eq!(TextColor::parse("#xyz"), None);
        assert_eq!(TextColor::parse("gold"), Some(NamedColor::Gold.into()));
        assert_eq!(TextColor::parse("GOLD"), Some(NamedColor::Gold.into()));
        assert_eq!(TextColor::parse("mauve"), None);
        assert_eq!(TextColor::Hex(0xab_cdef).to_string(), "#ABCDEF");
        assert_eq!(TextColor::Hex(0xAB).to_string(), "#0000AB");
        assert_eq!(NamedColor::Gold.to_string(), "gold");
        assert_eq!(
            "dark_aqua".into_text_color(),
            Some(NamedColor::DarkAqua.into())
        );
        assert_eq!(
            String::from("#010203").into_text_color(),
            Some(TextColor::Hex(0x01_0203))
        );
        assert_eq!(TextColor::Hex(0x0102_0304).rgb(), 0x02_0304);
    }

    #[test]
    fn legacy_codes_round_trip_for_colors_and_decorations() {
        for color in NamedColor::ALL {
            assert_eq!(
                NamedColor::from_legacy_code(color.legacy_code()),
                Some(color)
            );
            assert_eq!(
                NamedColor::from_legacy_code(color.legacy_code().to_ascii_uppercase()),
                Some(color)
            );
            assert_eq!(NamedColor::from_name(color.name()), Some(color));
        }
        for decoration in Decoration::ALL {
            assert_eq!(
                Decoration::from_legacy_code(decoration.legacy_code()),
                Some(decoration)
            );
        }
        assert_eq!(NamedColor::from_legacy_code('k'), None);
        assert_eq!(Decoration::from_legacy_code('0'), None);
        assert_eq!(Decoration::Bold.name(), "bold");
    }
}
