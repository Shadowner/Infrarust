use std::borrow::Cow;
use std::fmt;

use serde_json::{Map, Number, Value};
use uuid::Uuid;

use super::ProtocolVersion;

pub const MAX_COMPONENT_DEPTH: usize = 64;
pub const MAX_NBT_DEPTH: usize = 512;
pub const LEGACY_SECTION: char = '\u{a7}';
pub const LEGACY_AMPERSAND: char = '&';

const MAX_RAW_DEPTH: usize = 128;
const MAX_NBT_NODES: usize = 262_144;
const MAX_NBT_STRING_BYTES: usize = 65_535;
const MAX_LEGACY_ITEM_COUNT: i32 = 127;
const MAX_MODERN_ITEM_COUNT: i32 = 99;

const INSERTION: ProtocolVersion = ProtocolVersion::MINECRAFT_1_8;
const SCORE_AND_SELECTOR: ProtocolVersion = ProtocolVersion::MINECRAFT_1_8;
const SHOW_ENTITY: ProtocolVersion = ProtocolVersion::MINECRAFT_1_8;
const KEYBIND: ProtocolVersion = ProtocolVersion::MINECRAFT_1_12;
const ENTITY_NAME_AS_JSON: ProtocolVersion = ProtocolVersion::MINECRAFT_1_13;
const NBT_CONTENT: ProtocolVersion = ProtocolVersion::MINECRAFT_1_14;
const NBT_STORAGE: ProtocolVersion = ProtocolVersion::MINECRAFT_1_15;
const COPY_TO_CLIPBOARD: ProtocolVersion = ProtocolVersion::MINECRAFT_1_15;
const RGB_FONT_AND_CONTENTS: ProtocolVersion = ProtocolVersion::MINECRAFT_1_16;
const SEPARATOR: ProtocolVersion = ProtocolVersion::MINECRAFT_1_17;
const TRANSLATE_FALLBACK: ProtocolVersion = ProtocolVersion::MINECRAFT_1_19_4;
const NBT_TEXT_ERA: ProtocolVersion = ProtocolVersion::MINECRAFT_1_20_4;
const ITEM_COMPONENTS: ProtocolVersion = ProtocolVersion::MINECRAFT_1_20_6;
const SHADOW_COLOR: ProtocolVersion = ProtocolVersion::MINECRAFT_1_21_4;
const SNAKE_CASE_EVENTS: ProtocolVersion = ProtocolVersion::MINECRAFT_1_21_5;
const CUSTOM_CLICK: ProtocolVersion = ProtocolVersion::MINECRAFT_1_21_6;
const OBJECT_CONTENT: ProtocolVersion = ProtocolVersion::MINECRAFT_1_21_9;
const LEGACY_JSON_TARGET: ProtocolVersion = ProtocolVersion::MINECRAFT_1_20_2;
const LEGACY_NBT_TARGET: ProtocolVersion = ProtocolVersion::MINECRAFT_1_21_4;

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

    fn from_legacy_code(code: char) -> Option<Self> {
        let code = code.to_ascii_lowercase();
        Self::ALL.into_iter().find(|d| d.legacy_code() == code)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ClickEvent {
    OpenUrl(String),
    RunCommand(String),
    SuggestCommand(String),
    ChangePage(i32),
    CopyToClipboard(String),
    Custom { id: String, payload: Option<String> },
}

impl ClickEvent {
    pub const fn action(&self) -> &'static str {
        match self {
            Self::OpenUrl(_) => "open_url",
            Self::RunCommand(_) => "run_command",
            Self::SuggestCommand(_) => "suggest_command",
            Self::ChangePage(_) => "change_page",
            Self::CopyToClipboard(_) => "copy_to_clipboard",
            Self::Custom { .. } => "custom",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum HoverEvent {
    ShowText(Box<Component>),
    ShowItem {
        id: String,
        count: i32,
        components: Option<Value>,
    },
    ShowEntity {
        entity_type: String,
        uuid: Uuid,
        name: Option<Box<Component>>,
    },
}

impl HoverEvent {
    pub fn show_text(text: impl Into<Component>) -> Self {
        Self::ShowText(Box::new(text.into()))
    }

    pub fn show_item(id: impl Into<String>, count: i32) -> Self {
        Self::ShowItem {
            id: id.into(),
            count,
            components: None,
        }
    }

    pub fn show_entity(entity_type: impl Into<String>, uuid: Uuid) -> Self {
        Self::ShowEntity {
            entity_type: entity_type.into(),
            uuid,
            name: None,
        }
    }

    pub const fn action(&self) -> &'static str {
        match self {
            Self::ShowText(_) => "show_text",
            Self::ShowItem { .. } => "show_item",
            Self::ShowEntity { .. } => "show_entity",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NbtSource {
    Block(String),
    Entity(String),
    Storage(String),
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ObjectContent {
    Atlas {
        atlas: Option<String>,
        sprite: String,
    },
    Player {
        player: Value,
        hat: Option<bool>,
    },
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Content {
    Text(String),
    Translatable {
        key: String,
        fallback: Option<String>,
        with: Vec<Component>,
    },
    Keybind(String),
    Score {
        name: String,
        objective: String,
    },
    Selector {
        pattern: String,
        separator: Option<Box<Component>>,
    },
    Nbt {
        path: String,
        interpret: Option<bool>,
        separator: Option<Box<Component>>,
        source: NbtSource,
    },
    Object(ObjectContent),
}

impl Default for Content {
    fn default() -> Self {
        Self::Text(String::new())
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct Style {
    pub color: Option<TextColor>,
    pub font: Option<String>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underlined: Option<bool>,
    pub strikethrough: Option<bool>,
    pub obfuscated: Option<bool>,
    pub shadow_color: Option<u32>,
    pub insertion: Option<String>,
    pub click: Option<Box<ClickEvent>>,
    pub hover: Option<Box<HoverEvent>>,
}

impl Style {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub const fn decoration(&self, decoration: Decoration) -> Option<bool> {
        match decoration {
            Decoration::Obfuscated => self.obfuscated,
            Decoration::Bold => self.bold,
            Decoration::Strikethrough => self.strikethrough,
            Decoration::Underlined => self.underlined,
            Decoration::Italic => self.italic,
        }
    }

    pub const fn set_decoration(&mut self, decoration: Decoration, value: Option<bool>) {
        match decoration {
            Decoration::Obfuscated => self.obfuscated = value,
            Decoration::Bold => self.bold = value,
            Decoration::Strikethrough => self.strikethrough = value,
            Decoration::Underlined => self.underlined = value,
            Decoration::Italic => self.italic = value,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct Component {
    pub content: Content,
    pub style: Style,
    pub children: Vec<Component>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ComponentParseError {
    #[error("invalid text-component JSON: {0}")]
    InvalidJson(String),
    #[error("invalid text-component NBT: {0}")]
    InvalidNbt(String),
}

impl From<&str> for Component {
    fn from(text: &str) -> Self {
        Self::text(text)
    }
}

impl From<String> for Component {
    fn from(text: String) -> Self {
        Self::text(text)
    }
}

impl Component {
    pub fn new(content: Content) -> Self {
        Self {
            content,
            style: Style::default(),
            children: Vec::new(),
        }
    }

    pub fn empty() -> Self {
        Self::default()
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self::new(Content::Text(text.into()))
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self::text(text).color(NamedColor::Red)
    }

    pub fn translatable(key: impl Into<String>) -> Self {
        Self::translatable_with(key, Vec::new())
    }

    pub fn translatable_with(
        key: impl Into<String>,
        args: impl IntoIterator<Item = Component>,
    ) -> Self {
        Self::new(Content::Translatable {
            key: key.into(),
            fallback: None,
            with: args.into_iter().collect(),
        })
    }

    pub fn keybind(keybind: impl Into<String>) -> Self {
        Self::new(Content::Keybind(keybind.into()))
    }

    pub fn score(name: impl Into<String>, objective: impl Into<String>) -> Self {
        Self::new(Content::Score {
            name: name.into(),
            objective: objective.into(),
        })
    }

    pub fn selector(pattern: impl Into<String>) -> Self {
        Self::new(Content::Selector {
            pattern: pattern.into(),
            separator: None,
        })
    }

    #[must_use]
    pub fn fallback(mut self, text: impl Into<String>) -> Self {
        if let Content::Translatable { fallback, .. } = &mut self.content {
            *fallback = Some(text.into());
        }
        self
    }

    #[must_use]
    pub fn separator(mut self, separator: Self) -> Self {
        match &mut self.content {
            Content::Selector {
                separator: slot, ..
            }
            | Content::Nbt {
                separator: slot, ..
            } => {
                *slot = Some(Box::new(separator));
            }
            _ => {}
        }
        self
    }

    #[must_use]
    pub fn with_style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    #[must_use]
    pub fn color(mut self, color: impl IntoTextColor) -> Self {
        if let Some(color) = color.into_text_color() {
            self.style.color = Some(color);
        }
        self
    }

    #[must_use]
    pub const fn decoration(mut self, decoration: Decoration, value: Option<bool>) -> Self {
        self.style.set_decoration(decoration, value);
        self
    }

    #[must_use]
    pub const fn bold(self) -> Self {
        self.decoration(Decoration::Bold, Some(true))
    }

    #[must_use]
    pub const fn italic(self) -> Self {
        self.decoration(Decoration::Italic, Some(true))
    }

    #[must_use]
    pub const fn underlined(self) -> Self {
        self.decoration(Decoration::Underlined, Some(true))
    }

    #[must_use]
    pub const fn strikethrough(self) -> Self {
        self.decoration(Decoration::Strikethrough, Some(true))
    }

    #[must_use]
    pub const fn obfuscated(self) -> Self {
        self.decoration(Decoration::Obfuscated, Some(true))
    }

    #[must_use]
    pub fn font(mut self, font: impl Into<String>) -> Self {
        self.style.font = Some(font.into());
        self
    }

    #[must_use]
    pub const fn shadow_color(mut self, argb: u32) -> Self {
        self.style.shadow_color = Some(argb);
        self
    }

    #[must_use]
    pub fn insertion(mut self, insertion: impl Into<String>) -> Self {
        self.style.insertion = Some(insertion.into());
        self
    }

    #[must_use]
    pub fn click(mut self, event: ClickEvent) -> Self {
        self.style.click = Some(Box::new(event));
        self
    }

    #[must_use]
    pub fn hover(mut self, event: HoverEvent) -> Self {
        self.style.hover = Some(Box::new(event));
        self
    }

    #[must_use]
    pub fn append(mut self, child: impl Into<Self>) -> Self {
        self.children.push(child.into());
        self
    }

    #[must_use]
    pub fn join(components: Vec<Self>, separator: &Self) -> Self {
        let mut iter = components.into_iter();
        let Some(first) = iter.next() else {
            return Self::text("");
        };
        let mut result = first;
        for component in iter {
            result = result.append(separator.clone()).append(component);
        }
        result
    }

    pub fn as_text(&self) -> Option<&str> {
        match &self.content {
            Content::Text(text) => Some(text),
            _ => None,
        }
    }

    pub fn is_plain_text(&self) -> bool {
        matches!(self.content, Content::Text(_))
            && self.style.is_empty()
            && self.children.is_empty()
    }

    pub fn depth(&self) -> usize {
        let mut deepest = 0;
        let mut stack = vec![(self, 1usize)];
        while let Some((component, level)) = stack.pop() {
            deepest = deepest.max(level);
            for nested in component.nested() {
                stack.push((nested, level + 1));
            }
        }
        deepest
    }

    fn nested(&self) -> impl Iterator<Item = &Self> {
        let content: Vec<&Self> = match &self.content {
            Content::Translatable { with, .. } => with.iter().collect(),
            Content::Selector {
                separator: Some(separator),
                ..
            }
            | Content::Nbt {
                separator: Some(separator),
                ..
            } => vec![separator.as_ref()],
            _ => Vec::new(),
        };
        let hover: Option<&Self> = match self.style.hover.as_deref() {
            Some(HoverEvent::ShowText(text)) => Some(text.as_ref()),
            Some(HoverEvent::ShowEntity {
                name: Some(name), ..
            }) => Some(name.as_ref()),
            _ => None,
        };
        content.into_iter().chain(hover).chain(self.children.iter())
    }
}

impl fmt::Display for Component {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_plain())
    }
}

impl Component {
    pub fn to_json_for(&self, version: ProtocolVersion) -> String {
        self.to_json_value_for(version).to_string()
    }

    pub fn to_json_value_for(&self, version: ProtocolVersion) -> Value {
        let target = Target {
            version,
            format: Format::Json,
        };
        ir_to_json(component_ir(self, target, 1, true))
    }

    pub fn to_nbt_for(&self, version: ProtocolVersion) -> Vec<u8> {
        let target = Target {
            version,
            format: Format::Nbt,
        };
        let ir = component_ir(self, target, 1, true);
        let mut out = Vec::with_capacity(64);
        out.push(ir.tag_id());
        write_payload(&ir, &mut out);
        out
    }

    #[must_use]
    pub fn to_json(&self) -> String {
        self.to_json_for(LEGACY_JSON_TARGET)
    }

    #[must_use]
    pub fn to_nbt_network(&self) -> Vec<u8> {
        self.to_nbt_for(LEGACY_NBT_TARGET)
    }

    pub fn to_plain(&self) -> String {
        let mut out = String::new();
        plain_into(self, 1, &mut out);
        out
    }

    pub fn to_legacy(&self, code: char) -> String {
        let mut segments = Vec::new();
        flatten_legacy(self, LegacyState::default(), 1, &mut segments);
        let mut out = String::new();
        let mut current = LegacyState::default();
        for (text, state) in segments {
            if state != current {
                emit_legacy_transition(&current, &state, code, &mut out);
                current = state;
            }
            out.push_str(&text);
        }
        out
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Json,
    Nbt,
}

#[derive(Clone, Copy)]
struct Target {
    version: ProtocolVersion,
    format: Format,
}

impl Target {
    const fn at(self, minimum: ProtocolVersion) -> bool {
        self.version.at_least(minimum)
    }

    fn collapse_list_elements(self) -> bool {
        self.format == Format::Json
    }
}

enum Ir {
    Bool(bool),
    Int(i32),
    Long(i64),
    Double(f64),
    Str(String),
    IntArray(Vec<i32>),
    List(Vec<Ir>),
    Compound(Vec<(Cow<'static, str>, Ir)>),
}

impl Ir {
    const fn tag_id(&self) -> u8 {
        match self {
            Self::Bool(_) => 1,
            Self::Int(_) => 3,
            Self::Long(_) => 4,
            Self::Double(_) => 6,
            Self::Str(_) => 8,
            Self::List(_) => 9,
            Self::Compound(_) => 10,
            Self::IntArray(_) => 11,
        }
    }

    fn str(value: &str) -> Self {
        Self::Str(value.to_owned())
    }
}

struct Fields(Vec<(Cow<'static, str>, Ir)>);

impl Fields {
    const fn new() -> Self {
        Self(Vec::new())
    }

    fn put(&mut self, key: &'static str, value: Ir) {
        self.0.push((Cow::Borrowed(key), value));
    }

    fn into_ir(self) -> Ir {
        Ir::Compound(self.0)
    }
}

fn component_ir(component: &Component, target: Target, depth: usize, collapse: bool) -> Ir {
    let mut fields = Fields::new();
    content_fields(&component.content, target, depth, &mut fields);
    style_fields(&component.style, target, depth, &mut fields);
    if depth < MAX_COMPONENT_DEPTH && !component.children.is_empty() {
        let children = component
            .children
            .iter()
            .map(|child| component_ir(child, target, depth + 1, target.collapse_list_elements()))
            .collect();
        fields.put("extra", Ir::List(children));
    }
    if collapse
        && target.at(NBT_TEXT_ERA)
        && fields.0.len() == 1
        && fields.0[0].0 == "text"
        && let Some((_, Ir::Str(text))) = fields.0.pop()
    {
        return Ir::Str(text);
    }
    fields.into_ir()
}

fn nested_ir(component: &Component, target: Target, depth: usize) -> Option<Ir> {
    (depth < MAX_COMPONENT_DEPTH).then(|| component_ir(component, target, depth + 1, true))
}

fn content_fields(content: &Content, target: Target, depth: usize, fields: &mut Fields) {
    match content {
        Content::Text(text) => fields.put("text", Ir::str(text)),
        Content::Translatable {
            key,
            fallback,
            with,
        } => {
            fields.put("translate", Ir::str(key));
            if let Some(fallback) = fallback
                && target.at(TRANSLATE_FALLBACK)
            {
                fields.put("fallback", Ir::str(fallback));
            }
            if depth < MAX_COMPONENT_DEPTH && !with.is_empty() {
                let args = with
                    .iter()
                    .map(|arg| {
                        component_ir(arg, target, depth + 1, target.collapse_list_elements())
                    })
                    .collect();
                fields.put("with", Ir::List(args));
            }
        }
        Content::Keybind(keybind) => {
            let key = if target.at(KEYBIND) {
                "keybind"
            } else {
                "translate"
            };
            fields.put(key, Ir::str(keybind));
        }
        Content::Score { name, objective } => {
            if target.at(SCORE_AND_SELECTOR) {
                let mut score = Fields::new();
                score.put("name", Ir::str(name));
                score.put("objective", Ir::str(objective));
                fields.put("score", score.into_ir());
            } else {
                fields.put("text", Ir::str(""));
            }
        }
        Content::Selector { pattern, separator } => {
            if !target.at(SCORE_AND_SELECTOR) {
                fields.put("text", Ir::str(pattern));
                return;
            }
            fields.put("selector", Ir::str(pattern));
            separator_field(separator.as_deref(), target, depth, fields);
        }
        Content::Nbt {
            path,
            interpret,
            separator,
            source,
        } => {
            let supported = target.at(NBT_CONTENT)
                && match source {
                    NbtSource::Storage(id) => target.at(NBT_STORAGE) && is_valid_identifier(id),
                    NbtSource::Block(_) | NbtSource::Entity(_) => true,
                };
            if !supported {
                fields.put("text", Ir::str(""));
                return;
            }
            fields.put("nbt", Ir::str(path));
            if let Some(interpret) = interpret {
                fields.put("interpret", Ir::Bool(*interpret));
            }
            separator_field(separator.as_deref(), target, depth, fields);
            match source {
                NbtSource::Block(pos) => fields.put("block", Ir::str(pos)),
                NbtSource::Entity(selector) => fields.put("entity", Ir::str(selector)),
                NbtSource::Storage(id) => fields.put("storage", Ir::str(id)),
            }
        }
        Content::Object(object) => {
            if !target.at(OBJECT_CONTENT) || !object_fields(object, fields) {
                fields.0.retain(|(key, _)| {
                    !matches!(key.as_ref(), "atlas" | "sprite" | "player" | "hat")
                });
                fields.put("text", Ir::str(""));
            }
        }
    }
}

fn object_fields(object: &ObjectContent, fields: &mut Fields) -> bool {
    match object {
        ObjectContent::Atlas { atlas, sprite } => {
            if !is_valid_identifier(sprite)
                || atlas.as_deref().is_some_and(|a| !is_valid_identifier(a))
            {
                return false;
            }
            if let Some(atlas) = atlas {
                fields.put("atlas", Ir::str(atlas));
            }
            fields.put("sprite", Ir::str(sprite));
            true
        }
        ObjectContent::Player { player, hat } => {
            let Some(player) = raw_ir(player) else {
                return false;
            };
            fields.put("player", player);
            if let Some(hat) = hat {
                fields.put("hat", Ir::Bool(*hat));
            }
            true
        }
    }
}

fn separator_field(
    separator: Option<&Component>,
    target: Target,
    depth: usize,
    fields: &mut Fields,
) {
    if let Some(separator) = separator
        && target.at(SEPARATOR)
        && let Some(ir) = nested_ir(separator, target, depth)
    {
        fields.put("separator", ir);
    }
}

fn style_fields(style: &Style, target: Target, depth: usize, fields: &mut Fields) {
    if let Some(color) = style.color {
        let color = if target.at(RGB_FONT_AND_CONTENTS) {
            color
        } else {
            TextColor::Named(color.downsample())
        };
        fields.put("color", Ir::Str(color.to_string()));
    }
    if let Some(shadow) = style.shadow_color
        && target.at(SHADOW_COLOR)
    {
        fields.put(
            "shadow_color",
            Ir::Int(i32::from_ne_bytes(shadow.to_ne_bytes())),
        );
    }
    for decoration in [
        Decoration::Bold,
        Decoration::Italic,
        Decoration::Underlined,
        Decoration::Strikethrough,
        Decoration::Obfuscated,
    ] {
        if let Some(value) = style.decoration(decoration) {
            fields.put(decoration.name(), Ir::Bool(value));
        }
    }
    if let Some((key, click)) = style.click.as_deref().and_then(|c| click_ir(c, target)) {
        fields.put(key, click);
    }
    if let Some((key, hover)) = style
        .hover
        .as_deref()
        .and_then(|h| hover_ir(h, target, depth))
    {
        fields.put(key, hover);
    }
    if let Some(insertion) = &style.insertion
        && target.at(INSERTION)
    {
        fields.put("insertion", Ir::str(insertion));
    }
    if let Some(font) = &style.font
        && target.at(RGB_FONT_AND_CONTENTS)
        && is_valid_identifier(font)
    {
        fields.put("font", Ir::str(font));
    }
}

fn click_ir(click: &ClickEvent, target: Target) -> Option<(&'static str, Ir)> {
    let mut fields = Fields::new();
    fields.put("action", Ir::str(click.action()));
    if target.at(SNAKE_CASE_EVENTS) {
        match click {
            ClickEvent::OpenUrl(url) => {
                if !is_valid_untrusted_url(url) {
                    return None;
                }
                fields.put("url", Ir::str(url));
            }
            ClickEvent::RunCommand(command) | ClickEvent::SuggestCommand(command) => {
                if !is_chat_safe(command) {
                    return None;
                }
                fields.put("command", Ir::str(command));
            }
            ClickEvent::ChangePage(page) => {
                if *page < 1 {
                    return None;
                }
                fields.put("page", Ir::Int(*page));
            }
            ClickEvent::CopyToClipboard(value) => fields.put("value", Ir::str(value)),
            ClickEvent::Custom { id, payload } => {
                if !target.at(CUSTOM_CLICK) || !is_valid_identifier(id) {
                    return None;
                }
                fields.put("id", Ir::str(id));
                if let Some(payload) = payload {
                    fields.put("payload", Ir::str(payload));
                }
            }
        }
        return Some(("click_event", fields.into_ir()));
    }
    let value = match click {
        ClickEvent::OpenUrl(value)
        | ClickEvent::RunCommand(value)
        | ClickEvent::SuggestCommand(value) => value.clone(),
        ClickEvent::ChangePage(page) => page.to_string(),
        ClickEvent::CopyToClipboard(value) => {
            if !target.at(COPY_TO_CLIPBOARD) {
                return None;
            }
            value.clone()
        }
        ClickEvent::Custom { .. } => return None,
    };
    fields.put("value", Ir::Str(value));
    Some(("clickEvent", fields.into_ir()))
}

fn hover_ir(hover: &HoverEvent, target: Target, depth: usize) -> Option<(&'static str, Ir)> {
    let mut fields = Fields::new();
    fields.put("action", Ir::str(hover.action()));
    if target.at(SNAKE_CASE_EVENTS) {
        match hover {
            HoverEvent::ShowText(text) => fields.put("value", nested_ir(text, target, depth)?),
            HoverEvent::ShowItem {
                id,
                count,
                components,
            } => {
                if !is_valid_identifier(id) {
                    return None;
                }
                fields.put("id", Ir::str(id));
                fields.put("count", Ir::Int((*count).clamp(1, MAX_MODERN_ITEM_COUNT)));
                if let Some(components) = components
                    .as_ref()
                    .filter(|c| c.is_object())
                    .and_then(raw_ir)
                {
                    fields.put("components", components);
                }
            }
            HoverEvent::ShowEntity {
                entity_type,
                uuid,
                name,
            } => {
                if !is_valid_identifier(entity_type) {
                    return None;
                }
                fields.put("id", Ir::str(entity_type));
                fields.put("uuid", Ir::IntArray(uuid_to_ints(*uuid).to_vec()));
                if let Some(name) = name.as_deref().and_then(|n| nested_ir(n, target, depth)) {
                    fields.put("name", name);
                }
            }
        }
        return Some(("hover_event", fields.into_ir()));
    }
    if target.at(RGB_FONT_AND_CONTENTS) {
        let contents = match hover {
            HoverEvent::ShowText(text) => nested_ir(text, target, depth)?,
            HoverEvent::ShowItem {
                id,
                count,
                components,
            } => {
                if !is_valid_identifier(id) {
                    return None;
                }
                let mut item = Fields::new();
                item.put("id", Ir::str(id));
                if target.at(ITEM_COMPONENTS) {
                    item.put("count", Ir::Int((*count).clamp(1, MAX_MODERN_ITEM_COUNT)));
                    if let Some(components) = components
                        .as_ref()
                        .filter(|c| c.is_object())
                        .and_then(raw_ir)
                    {
                        item.put("components", components);
                    }
                } else if *count > 1 {
                    item.put("count", Ir::Int(*count));
                }
                item.into_ir()
            }
            HoverEvent::ShowEntity {
                entity_type,
                uuid,
                name,
            } => {
                if !is_valid_identifier(entity_type) {
                    return None;
                }
                let mut entity = Fields::new();
                entity.put("type", Ir::str(entity_type));
                let id = if target.at(NBT_TEXT_ERA) {
                    Ir::IntArray(uuid_to_ints(*uuid).to_vec())
                } else {
                    Ir::Str(uuid.hyphenated().to_string())
                };
                entity.put("id", id);
                if let Some(name) = name.as_deref().and_then(|n| nested_ir(n, target, depth)) {
                    entity.put("name", name);
                }
                entity.into_ir()
            }
        };
        fields.put("contents", contents);
        return Some(("hoverEvent", fields.into_ir()));
    }
    let value = match hover {
        HoverEvent::ShowText(text) => nested_ir(text, target, depth)?,
        HoverEvent::ShowItem { id, count, .. } => {
            if !is_valid_identifier(id) {
                return None;
            }
            legacy_text_ir(format!(
                "{{id:{},Count:{}b}}",
                snbt_quote(id),
                (*count).clamp(1, MAX_LEGACY_ITEM_COUNT)
            ))
        }
        HoverEvent::ShowEntity {
            entity_type,
            uuid,
            name,
        } => {
            if !target.at(SHOW_ENTITY) || !is_valid_identifier(entity_type) {
                return None;
            }
            let mut snbt = format!(
                "{{id:{},type:{}",
                snbt_quote(&uuid.hyphenated().to_string()),
                snbt_quote(entity_type)
            );
            if let Some(name) = name.as_deref()
                && depth < MAX_COMPONENT_DEPTH
            {
                let rendered = if target.at(ENTITY_NAME_AS_JSON) {
                    let json_target = Target {
                        version: target.version,
                        format: Format::Json,
                    };
                    ir_to_json(component_ir(name, json_target, depth + 1, true)).to_string()
                } else {
                    let mut plain = String::new();
                    plain_into(name, depth + 1, &mut plain);
                    plain
                };
                snbt.push_str(",name:");
                snbt.push_str(&snbt_quote(&rendered));
            }
            snbt.push('}');
            legacy_text_ir(snbt)
        }
    };
    fields.put("value", value);
    Some(("hoverEvent", fields.into_ir()))
}

fn legacy_text_ir(text: String) -> Ir {
    let mut fields = Fields::new();
    fields.put("text", Ir::Str(text));
    fields.into_ir()
}

fn snbt_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        if ch == '"' || ch == '\\' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

fn uuid_to_ints(uuid: Uuid) -> [i32; 4] {
    let value = uuid.as_u128();
    [96u32, 64, 32, 0].map(|shift| i32::from_ne_bytes(((value >> shift) as u32).to_ne_bytes()))
}

fn uuid_from_ints(ints: [i32; 4]) -> Uuid {
    let value = ints.iter().fold(0u128, |acc, part| {
        (acc << 32) | u128::from(u32::from_ne_bytes(part.to_ne_bytes()))
    });
    Uuid::from_u128(value)
}

fn raw_ir(value: &Value) -> Option<Ir> {
    if !raw_depth_within(value, MAX_RAW_DEPTH) {
        return None;
    }
    raw_to_ir(value)
}

fn raw_depth_within(value: &Value, remaining: usize) -> bool {
    match value {
        Value::Array(items) => {
            remaining > 0 && items.iter().all(|v| raw_depth_within(v, remaining - 1))
        }
        Value::Object(map) => {
            remaining > 0 && map.values().all(|v| raw_depth_within(v, remaining - 1))
        }
        _ => true,
    }
}

fn raw_to_ir(value: &Value) -> Option<Ir> {
    match value {
        Value::Null => None,
        Value::Bool(b) => Some(Ir::Bool(*b)),
        Value::Number(number) => Some(number_ir(number)),
        Value::String(s) => Some(Ir::str(s)),
        Value::Array(items) => {
            let ints: Option<Vec<i32>> = items
                .iter()
                .map(|item| item.as_i64().and_then(|n| i32::try_from(n).ok()))
                .collect();
            match ints {
                Some(ints) if !ints.is_empty() => Some(Ir::IntArray(ints)),
                _ => Some(Ir::List(items.iter().filter_map(raw_to_ir).collect())),
            }
        }
        Value::Object(map) => Some(Ir::Compound(
            map.iter()
                .filter_map(|(key, v)| raw_to_ir(v).map(|ir| (Cow::Owned(key.clone()), ir)))
                .collect(),
        )),
    }
}

fn number_ir(number: &Number) -> Ir {
    if let Some(int) = number.as_i64() {
        return i32::try_from(int).map_or(Ir::Long(int), Ir::Int);
    }
    Ir::Double(number.as_f64().unwrap_or(0.0))
}

fn ir_to_json(ir: Ir) -> Value {
    match ir {
        Ir::Bool(b) => Value::Bool(b),
        Ir::Int(i) => Value::from(i),
        Ir::Long(l) => Value::from(l),
        Ir::Double(d) => Number::from_f64(d).map_or(Value::Null, Value::Number),
        Ir::Str(s) => Value::String(s),
        Ir::IntArray(ints) => Value::Array(ints.into_iter().map(Value::from).collect()),
        Ir::List(items) => Value::Array(items.into_iter().map(ir_to_json).collect()),
        Ir::Compound(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key.into_owned(), ir_to_json(value)))
                .collect::<Map<String, Value>>(),
        ),
    }
}

fn write_payload(ir: &Ir, out: &mut Vec<u8>) {
    match ir {
        Ir::Bool(b) => out.push(u8::from(*b)),
        Ir::Int(i) => out.extend_from_slice(&i.to_be_bytes()),
        Ir::Long(l) => out.extend_from_slice(&l.to_be_bytes()),
        Ir::Double(d) => out.extend_from_slice(&d.to_be_bytes()),
        Ir::Str(s) => write_mutf8(s, out),
        Ir::IntArray(ints) => {
            write_len(ints.len(), out);
            for i in ints {
                out.extend_from_slice(&i.to_be_bytes());
            }
        }
        Ir::List(items) => write_list(items, out),
        Ir::Compound(fields) => write_compound(fields, out),
    }
}

fn write_len(len: usize, out: &mut Vec<u8>) {
    out.extend_from_slice(&i32::try_from(len).unwrap_or(i32::MAX).to_be_bytes());
}

fn write_compound(fields: &[(Cow<'static, str>, Ir)], out: &mut Vec<u8>) {
    for (key, value) in fields {
        out.push(value.tag_id());
        write_mutf8(key, out);
        write_payload(value, out);
    }
    out.push(0);
}

fn is_wrapper_compound(ir: &Ir) -> bool {
    matches!(ir, Ir::Compound(fields) if fields.len() == 1 && fields[0].0.is_empty())
}

fn write_list(items: &[Ir], out: &mut Vec<u8>) {
    let Some(first) = items.first() else {
        out.push(0);
        write_len(0, out);
        return;
    };
    let element = first.tag_id();
    let homogeneous = items.iter().all(|item| item.tag_id() == element);
    if homogeneous && !(element == 10 && items.iter().any(is_wrapper_compound)) {
        out.push(element);
        write_len(items.len(), out);
        for item in items {
            write_payload(item, out);
        }
        return;
    }
    out.push(10);
    write_len(items.len(), out);
    for item in items {
        match item {
            Ir::Compound(fields) if !is_wrapper_compound(item) => write_compound(fields, out),
            other => {
                out.push(other.tag_id());
                write_mutf8("", out);
                write_payload(other, out);
                out.push(0);
            }
        }
    }
}

fn mutf8_unit_len(unit: u16) -> usize {
    match unit {
        0x0001..=0x007F => 1,
        0x0000 | 0x0080..=0x07FF => 2,
        _ => 3,
    }
}

fn write_mutf8(value: &str, out: &mut Vec<u8>) {
    let start = out.len();
    out.extend_from_slice(&[0, 0]);
    let mut written = 0usize;
    let mut units = [0u16; 2];
    for ch in value.chars() {
        let encoded = ch.encode_utf16(&mut units);
        let needed: usize = encoded.iter().map(|u| mutf8_unit_len(*u)).sum();
        if written + needed > MAX_NBT_STRING_BYTES {
            break;
        }
        for unit in encoded.iter().copied() {
            match mutf8_unit_len(unit) {
                1 => out.push(unit.to_be_bytes()[1]),
                2 => {
                    out.push(0xC0 | ((unit >> 6) & 0x1F).to_be_bytes()[1]);
                    out.push(0x80 | (unit & 0x3F).to_be_bytes()[1]);
                }
                _ => {
                    out.push(0xE0 | ((unit >> 12) & 0x0F).to_be_bytes()[1]);
                    out.push(0x80 | ((unit >> 6) & 0x3F).to_be_bytes()[1]);
                    out.push(0x80 | (unit & 0x3F).to_be_bytes()[1]);
                }
            }
        }
        written += needed;
    }
    let len = u16::try_from(written).unwrap_or(u16::MAX).to_be_bytes();
    out[start] = len[0];
    out[start + 1] = len[1];
}

fn decode_mutf8(bytes: &[u8]) -> String {
    let continuation = |index: usize| bytes.get(index).is_some_and(|b| b & 0xC0 == 0x80);
    let low = |index: usize| u16::from(bytes.get(index).copied().unwrap_or(0) & 0x3F);
    let mut units: Vec<u16> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&lead) = bytes.get(i) {
        if lead < 0x80 {
            units.push(u16::from(lead));
            i += 1;
        } else if lead & 0xE0 == 0xC0 && continuation(i + 1) {
            units.push((u16::from(lead & 0x1F) << 6) | low(i + 1));
            i += 2;
        } else if lead & 0xF0 == 0xE0 && continuation(i + 1) && continuation(i + 2) {
            units.push((u16::from(lead & 0x0F) << 12) | (low(i + 1) << 6) | low(i + 2));
            i += 3;
        } else if lead & 0xF8 == 0xF0
            && continuation(i + 1)
            && continuation(i + 2)
            && continuation(i + 3)
        {
            let code = (u32::from(lead & 0x07) << 18)
                | (u32::from(low(i + 1)) << 12)
                | (u32::from(low(i + 2)) << 6)
                | u32::from(low(i + 3));
            match char::from_u32(code) {
                Some(ch) => {
                    let mut buf = [0u16; 2];
                    units.extend_from_slice(ch.encode_utf16(&mut buf));
                }
                None => units.push(0xFFFD),
            }
            i += 4;
        } else {
            units.push(0xFFFD);
            i += 1;
        }
    }
    String::from_utf16_lossy(&units)
}

fn is_valid_identifier(value: &str) -> bool {
    let (namespace, path) = value.split_once(':').unwrap_or(("", value));
    !path.is_empty()
        && namespace
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.'))
        && path
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/'))
}

fn is_chat_safe(value: &str) -> bool {
    value
        .chars()
        .all(|c| c >= ' ' && c != '\u{7f}' && c != LEGACY_SECTION)
}

fn is_valid_untrusted_url(url: &str) -> bool {
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

fn content_plain(content: &Content) -> &str {
    match content {
        Content::Text(text) => text,
        Content::Translatable { key, fallback, .. } => fallback.as_deref().unwrap_or(key),
        Content::Keybind(keybind) => keybind,
        Content::Selector { pattern, .. } => pattern,
        Content::Score { .. } | Content::Nbt { .. } | Content::Object(_) => "",
    }
}

fn plain_into(component: &Component, depth: usize, out: &mut String) {
    out.push_str(content_plain(&component.content));
    if depth < MAX_COMPONENT_DEPTH {
        for child in &component.children {
            plain_into(child, depth + 1, out);
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LegacyState {
    color: Option<TextColor>,
    decorations: [bool; 5],
}

impl LegacyState {
    fn inherit(&self, style: &Style) -> Self {
        let mut next = self.clone();
        if let Some(color) = style.color {
            next.color = Some(color);
        }
        for (slot, decoration) in next.decorations.iter_mut().zip(Decoration::ALL) {
            if let Some(value) = style.decoration(decoration) {
                *slot = value;
            }
        }
        next
    }

    fn to_style(&self) -> Style {
        let mut style = Style {
            color: self.color,
            ..Style::default()
        };
        for (enabled, decoration) in self.decorations.iter().zip(Decoration::ALL) {
            if *enabled {
                style.set_decoration(decoration, Some(true));
            }
        }
        style
    }
}

fn flatten_legacy(
    component: &Component,
    inherited: LegacyState,
    depth: usize,
    out: &mut Vec<(String, LegacyState)>,
) {
    let state = inherited.inherit(&component.style);
    let text = content_plain(&component.content);
    if !text.is_empty() {
        out.push((text.to_owned(), state.clone()));
    }
    if depth < MAX_COMPONENT_DEPTH {
        for child in &component.children {
            flatten_legacy(child, state.clone(), depth + 1, out);
        }
    }
}

fn push_legacy_color(color: TextColor, code: char, out: &mut String) {
    match color {
        TextColor::Named(named) => {
            out.push(code);
            out.push(named.legacy_code());
        }
        TextColor::Hex(value) => {
            out.push(code);
            out.push('x');
            for digit in format!("{:06x}", value & 0xFF_FFFF).chars() {
                out.push(code);
                out.push(digit);
            }
        }
    }
}

fn emit_legacy_transition(current: &LegacyState, next: &LegacyState, code: char, out: &mut String) {
    let removed = current
        .decorations
        .iter()
        .zip(next.decorations.iter())
        .any(|(was, is)| *was && !*is)
        || (current.color.is_some() && next.color.is_none());
    let emit_all = |out: &mut String| {
        for (enabled, decoration) in next.decorations.iter().zip(Decoration::ALL) {
            if *enabled {
                out.push(code);
                out.push(decoration.legacy_code());
            }
        }
    };
    if let Some(color) = next.color.filter(|c| removed || Some(*c) != current.color) {
        push_legacy_color(color, code, out);
        emit_all(out);
    } else if removed {
        out.push(code);
        out.push('r');
        emit_all(out);
    } else {
        for ((was, is), decoration) in current
            .decorations
            .iter()
            .zip(next.decorations.iter())
            .zip(Decoration::ALL)
        {
            if *is && !*was {
                out.push(code);
                out.push(decoration.legacy_code());
            }
        }
    }
}

fn legacy_hex_at(chars: &[char], start: usize, code: char) -> Option<u32> {
    let mut value = 0u32;
    for pair in 0..6 {
        let marker = chars.get(start + pair * 2).copied()?;
        let digit = chars.get(start + pair * 2 + 1).copied()?;
        if marker != code {
            return None;
        }
        value = (value << 4) | digit.to_digit(16)?;
    }
    Some(value)
}

fn hash_hex_at(chars: &[char], start: usize) -> Option<u32> {
    let mut value = 0u32;
    for offset in 0..6 {
        value = (value << 4) | chars.get(start + offset)?.to_digit(16)?;
    }
    Some(value)
}

impl Component {
    #[must_use]
    pub fn from_legacy(text: &str) -> Self {
        Self::from_legacy_with(text, LEGACY_AMPERSAND)
    }

    #[must_use]
    pub fn from_legacy_with(text: &str, code: char) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let mut segments: Vec<(String, LegacyState)> = Vec::new();
        let mut segment = String::new();
        let mut state = LegacyState::default();
        let mut i = 0;
        while let Some(&ch) = chars.get(i) {
            let format = chars.get(i + 1).copied().filter(|_| ch == code);
            let Some(format) = format else {
                segment.push(ch);
                i += 1;
                continue;
            };
            let lowered = format.to_ascii_lowercase();
            let (next, consumed) = if lowered == 'x' {
                match legacy_hex_at(&chars, i + 2, code) {
                    Some(rgb) => (Some(color_state(TextColor::Hex(rgb))), 14),
                    None => (None, 0),
                }
            } else if lowered == '#' {
                match hash_hex_at(&chars, i + 2) {
                    Some(rgb) => (Some(color_state(TextColor::Hex(rgb))), 8),
                    None => (None, 0),
                }
            } else if let Some(named) = NamedColor::from_legacy_code(lowered) {
                (Some(color_state(TextColor::Named(named))), 2)
            } else if let Some(decoration) = Decoration::from_legacy_code(lowered) {
                let mut decorated = state.clone();
                if let Some(slot) = Decoration::ALL
                    .iter()
                    .position(|d| *d == decoration)
                    .and_then(|index| decorated.decorations.get_mut(index))
                {
                    *slot = true;
                }
                (Some(decorated), 2)
            } else if lowered == 'r' {
                (Some(LegacyState::default()), 2)
            } else {
                (None, 0)
            };
            match next {
                Some(next) => {
                    if !segment.is_empty() {
                        segments.push((std::mem::take(&mut segment), state));
                    }
                    state = next;
                    i += consumed;
                }
                None => {
                    segment.push(ch);
                    i += 1;
                }
            }
        }
        if !segment.is_empty() {
            segments.push((segment, state));
        }
        let mut parts = segments
            .into_iter()
            .map(|(text, state)| Self::text(text).with_style(state.to_style()));
        let Some(first) = parts.next() else {
            return Self::text("");
        };
        let rest: Vec<Self> = parts.collect();
        if rest.is_empty() {
            return first;
        }
        if first.style.is_empty() {
            let mut root = first;
            root.children.extend(rest);
            return root;
        }
        let mut root = Self::text("");
        root.children.push(first);
        root.children.extend(rest);
        root
    }

    #[must_use]
    pub fn from_legacy_format(template: &str, vars: &[(&str, &str)]) -> Self {
        Self::from_legacy(&format_placeholders(template, vars))
    }

    pub fn from_json(input: &str) -> Result<Self, ComponentParseError> {
        let value: Value = serde_json::from_str(input)
            .map_err(|e| ComponentParseError::InvalidJson(e.to_string()))?;
        Ok(Self::from_json_value(&value))
    }

    pub fn from_json_value(value: &Value) -> Self {
        parse_value(value, 1)
    }

    pub fn from_nbt_network(bytes: &[u8]) -> Result<Self, ComponentParseError> {
        let (component, consumed) = Self::from_nbt_network_prefix(bytes)?;
        if consumed != bytes.len() {
            return Err(ComponentParseError::InvalidNbt(format!(
                "{} trailing bytes after root tag",
                bytes.len() - consumed
            )));
        }
        Ok(component)
    }

    pub fn from_nbt_network_prefix(bytes: &[u8]) -> Result<(Self, usize), ComponentParseError> {
        let mut reader = NbtReader {
            buf: bytes,
            pos: 0,
            nodes: 0,
        };
        let value = reader.root().map_err(ComponentParseError::InvalidNbt)?;
        Ok((Self::from_json_value(&value), reader.pos))
    }
}

fn color_state(color: TextColor) -> LegacyState {
    LegacyState {
        color: Some(color),
        decorations: [false; 5],
    }
}

fn parse_value(value: &Value, depth: usize) -> Component {
    match value {
        Value::String(text) => Component::text(text.clone()),
        Value::Bool(b) => Component::text(b.to_string()),
        Value::Number(n) => Component::text(n.to_string()),
        Value::Null => Component::default(),
        Value::Array(items) => {
            let mut iter = items.iter();
            let Some(mut root) = iter.next().and_then(|first| parse_nested(first, depth)) else {
                return Component::default();
            };
            root.children
                .extend(iter.filter_map(|item| parse_nested(item, depth)));
            root
        }
        Value::Object(map) => {
            let content = parse_content(map, depth);
            let style = parse_style(map, depth);
            let children = match map.get("extra") {
                Some(Value::Array(extra)) => extra
                    .iter()
                    .filter_map(|item| parse_nested(item, depth))
                    .collect(),
                _ => Vec::new(),
            };
            Component {
                content,
                style,
                children,
            }
        }
    }
}

fn parse_nested(value: &Value, depth: usize) -> Option<Component> {
    (depth < MAX_COMPONENT_DEPTH).then(|| parse_value(value, depth + 1))
}

fn get_str<'a>(map: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    map.get(key).and_then(Value::as_str)
}

fn parse_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_f64().map(|f| f != 0.0),
        Value::String(s) if s.eq_ignore_ascii_case("true") => Some(true),
        Value::String(s) if s.eq_ignore_ascii_case("false") => Some(false),
        _ => None,
    }
}

fn parse_i32(value: &Value) -> Option<i32> {
    match value {
        Value::Number(n) => n.as_i64().and_then(|i| i32::try_from(i).ok()).or_else(|| {
            n.as_f64()
                .filter(|f| f.is_finite() && f.abs() < 2_147_483_648.0)
                .map(|f| f.trunc() as i32)
        }),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn parse_shadow_color(value: &Value) -> Option<u32> {
    match value {
        Value::Number(n) => {
            let int = n.as_i64()?;
            i32::try_from(int)
                .map(|i| u32::from_ne_bytes(i.to_ne_bytes()))
                .ok()
                .or_else(|| u32::try_from(int).ok())
        }
        Value::Array(items) if items.len() == 4 => {
            let mut channels = [0u32; 4];
            for (slot, item) in channels.iter_mut().zip(items) {
                let f = item.as_f64()?;
                *slot = (f * 255.0).floor().clamp(0.0, 255.0) as u32;
            }
            let [r, g, b, a] = channels;
            Some((a << 24) | (r << 16) | (g << 8) | b)
        }
        _ => None,
    }
}

fn parse_uuid(value: &Value) -> Option<Uuid> {
    match value {
        Value::String(s) => Uuid::parse_str(s).ok(),
        Value::Array(items) if items.len() == 4 => {
            let mut ints = [0i32; 4];
            for (slot, item) in ints.iter_mut().zip(items) {
                *slot = match item {
                    Value::Number(n) => n.as_i64().map(|i| i as i32).or_else(|| {
                        n.as_f64()
                            .filter(|f| f.is_finite())
                            .map(|f| f as i64 as i32)
                    })?,
                    Value::Bool(b) => i32::from(*b),
                    _ => return None,
                };
            }
            Some(uuid_from_ints(ints))
        }
        _ => None,
    }
}

fn parse_raw(value: &Value) -> Option<Value> {
    raw_depth_within(value, MAX_RAW_DEPTH).then(|| value.clone())
}

const CONTENT_TYPES: [&str; 7] = [
    "text",
    "translatable",
    "keybind",
    "score",
    "selector",
    "nbt",
    "object",
];

fn parse_content(map: &Map<String, Value>, depth: usize) -> Content {
    if let Some(declared) = get_str(map, "type")
        && let Some(content) = parse_content_of(declared, map, depth)
    {
        return content;
    }
    CONTENT_TYPES
        .iter()
        .find_map(|kind| parse_content_of(kind, map, depth))
        .unwrap_or_default()
}

fn parse_content_of(kind: &str, map: &Map<String, Value>, depth: usize) -> Option<Content> {
    match kind {
        "text" => match map.get("text")? {
            Value::String(text) => Some(Content::Text(text.clone())),
            Value::Number(n) => Some(Content::Text(n.to_string())),
            Value::Bool(b) => Some(Content::Text(b.to_string())),
            _ => None,
        },
        "translatable" => {
            let key = get_str(map, "translate")?.to_owned();
            let fallback = get_str(map, "fallback").map(str::to_owned);
            let with = match map.get("with") {
                Some(Value::Array(args)) => args
                    .iter()
                    .filter_map(|arg| parse_nested(arg, depth))
                    .collect(),
                _ => Vec::new(),
            };
            Some(Content::Translatable {
                key,
                fallback,
                with,
            })
        }
        "keybind" => Some(Content::Keybind(get_str(map, "keybind")?.to_owned())),
        "score" => {
            let score = map.get("score")?.as_object()?;
            Some(Content::Score {
                name: get_str(score, "name")?.to_owned(),
                objective: get_str(score, "objective")?.to_owned(),
            })
        }
        "selector" => Some(Content::Selector {
            pattern: get_str(map, "selector")?.to_owned(),
            separator: map
                .get("separator")
                .and_then(|s| parse_nested(s, depth))
                .map(Box::new),
        }),
        "nbt" => {
            let path = get_str(map, "nbt")?.to_owned();
            let source_of = |key: &str| {
                get_str(map, key).map(|v| match key {
                    "block" => NbtSource::Block(v.to_owned()),
                    "storage" => NbtSource::Storage(v.to_owned()),
                    _ => NbtSource::Entity(v.to_owned()),
                })
            };
            let source = match get_str(map, "source") {
                Some(declared @ ("entity" | "block" | "storage")) => source_of(declared),
                _ => None,
            }
            .or_else(|| {
                ["entity", "block", "storage"]
                    .into_iter()
                    .find_map(source_of)
            })?;
            Some(Content::Nbt {
                path,
                interpret: map.get("interpret").and_then(parse_bool),
                separator: map
                    .get("separator")
                    .and_then(|s| parse_nested(s, depth))
                    .map(Box::new),
                source,
            })
        }
        "object" => {
            let atlas = || {
                Some(ObjectContent::Atlas {
                    atlas: get_str(map, "atlas").map(str::to_owned),
                    sprite: get_str(map, "sprite")?.to_owned(),
                })
            };
            let player = || {
                Some(ObjectContent::Player {
                    player: parse_raw(map.get("player")?)?,
                    hat: map.get("hat").and_then(parse_bool),
                })
            };
            let object = match get_str(map, "object") {
                Some("atlas") => atlas(),
                Some("player") => player(),
                _ => atlas().or_else(player),
            }?;
            Some(Content::Object(object))
        }
        _ => None,
    }
}

fn parse_style(map: &Map<String, Value>, depth: usize) -> Style {
    let mut style = Style {
        color: get_str(map, "color").and_then(TextColor::parse),
        font: get_str(map, "font").map(str::to_owned),
        shadow_color: map.get("shadow_color").and_then(parse_shadow_color),
        insertion: get_str(map, "insertion").map(str::to_owned),
        click: ["click_event", "clickEvent"]
            .into_iter()
            .filter_map(|key| map.get(key).and_then(Value::as_object))
            .find_map(parse_click)
            .map(Box::new),
        hover: ["hover_event", "hoverEvent"]
            .into_iter()
            .filter_map(|key| map.get(key).and_then(Value::as_object))
            .find_map(|hover| parse_hover(hover, depth))
            .map(Box::new),
        ..Style::default()
    };
    for decoration in Decoration::ALL {
        style.set_decoration(decoration, map.get(decoration.name()).and_then(parse_bool));
    }
    style
}

fn parse_click(map: &Map<String, Value>) -> Option<ClickEvent> {
    let text = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| get_str(map, key))
            .map(str::to_owned)
    };
    match get_str(map, "action")? {
        "open_url" => text(&["url", "value"]).map(ClickEvent::OpenUrl),
        "run_command" => text(&["command", "value"]).map(ClickEvent::RunCommand),
        "suggest_command" => text(&["command", "value"]).map(ClickEvent::SuggestCommand),
        "change_page" => ["page", "value"]
            .into_iter()
            .find_map(|key| map.get(key).and_then(parse_i32))
            .map(ClickEvent::ChangePage),
        "copy_to_clipboard" => text(&["value"]).map(ClickEvent::CopyToClipboard),
        "custom" => Some(ClickEvent::Custom {
            id: get_str(map, "id")?.to_owned(),
            payload: get_str(map, "payload").map(str::to_owned),
        }),
        _ => None,
    }
}

fn parse_item(map: &Map<String, Value>) -> Option<HoverEvent> {
    Some(HoverEvent::ShowItem {
        id: get_str(map, "id")?.to_owned(),
        count: map.get("count").and_then(parse_i32).unwrap_or(1),
        components: map
            .get("components")
            .filter(|c| c.is_object())
            .and_then(parse_raw),
    })
}

fn parse_hover(map: &Map<String, Value>, depth: usize) -> Option<HoverEvent> {
    match get_str(map, "action")? {
        "show_text" => {
            let value = ["contents", "value", "text"]
                .into_iter()
                .find_map(|key| map.get(key))?;
            parse_nested(value, depth).map(|text| HoverEvent::ShowText(Box::new(text)))
        }
        "show_item" => match map.get("contents") {
            Some(Value::String(id)) => Some(HoverEvent::ShowItem {
                id: id.clone(),
                count: 1,
                components: None,
            }),
            Some(Value::Object(contents)) => parse_item(contents),
            Some(_) => None,
            None => parse_item(map),
        },
        "show_entity" => {
            let (entity, type_key, uuid_key) = match map.get("contents") {
                Some(Value::Object(contents)) => (contents, "type", "id"),
                Some(_) => return None,
                None => (map, "id", "uuid"),
            };
            Some(HoverEvent::ShowEntity {
                entity_type: get_str(entity, type_key)?.to_owned(),
                uuid: parse_uuid(entity.get(uuid_key)?)?,
                name: entity
                    .get("name")
                    .and_then(|name| parse_nested(name, depth))
                    .map(Box::new),
            })
        }
        _ => None,
    }
}

struct NbtReader<'a> {
    buf: &'a [u8],
    pos: usize,
    nodes: usize,
}

impl NbtReader<'_> {
    fn take(&mut self, len: usize) -> Result<&[u8], String> {
        let end = self
            .pos
            .checked_add(len)
            .filter(|end| *end <= self.buf.len())
            .ok_or_else(|| "unexpected end of input".to_owned())?;
        let slice = self
            .buf
            .get(self.pos..end)
            .ok_or_else(|| "unexpected end of input".to_owned())?;
        self.pos = end;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let slice = self.take(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(slice);
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.array::<1>()?[0])
    }

    fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_be_bytes(self.array()?))
    }

    fn len(&mut self) -> Result<usize, String> {
        let len = self.i32()?;
        usize::try_from(len).map_err(|_| format!("negative length {len}"))
    }

    fn string(&mut self) -> Result<String, String> {
        let len = usize::from(u16::from_be_bytes(self.array()?));
        let bytes = self.take(len)?;
        Ok(decode_mutf8(bytes))
    }

    fn account(&mut self, nodes: usize) -> Result<(), String> {
        self.nodes = self.nodes.saturating_add(nodes);
        if self.nodes > MAX_NBT_NODES {
            return Err("NBT has too many elements".to_owned());
        }
        Ok(())
    }

    fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    fn root(&mut self) -> Result<Value, String> {
        let tag = self.u8()?;
        match tag {
            8..=10 => self.payload(tag, 0),
            other => Err(format!("unsupported root tag {other}")),
        }
    }

    fn payload(&mut self, tag: u8, depth: usize) -> Result<Value, String> {
        if depth > MAX_NBT_DEPTH {
            return Err("NBT nested too deeply".to_owned());
        }
        self.account(1)?;
        Ok(match tag {
            1 => Value::from(i8::from_be_bytes(self.array()?)),
            2 => Value::from(i16::from_be_bytes(self.array()?)),
            3 => Value::from(self.i32()?),
            4 => Value::from(i64::from_be_bytes(self.array()?)),
            5 => float_value(f64::from(f32::from_be_bytes(self.array()?))),
            6 => float_value(f64::from_be_bytes(self.array()?)),
            7 => {
                let len = self.len()?;
                self.account(len)?;
                let bytes = self.take(len)?;
                Value::Array(
                    bytes
                        .iter()
                        .map(|b| Value::from(i8::from_ne_bytes([*b])))
                        .collect(),
                )
            }
            8 => Value::String(self.string()?),
            9 => {
                let element = self.u8()?;
                let len = self.len()?;
                if element == 0 && len > 0 {
                    return Err("non-empty list of TAG_End".to_owned());
                }
                let mut items = Vec::with_capacity(len.min(self.remaining()));
                for _ in 0..len {
                    let item = self.payload(element, depth + 1)?;
                    items.push(if element == 10 {
                        unwrap_list_wrapper(item)
                    } else {
                        item
                    });
                }
                Value::Array(items)
            }
            10 => {
                let mut map = Map::new();
                loop {
                    let field = self.u8()?;
                    if field == 0 {
                        break;
                    }
                    let name = self.string()?;
                    let value = self.payload(field, depth + 1)?;
                    map.insert(name, value);
                }
                Value::Object(map)
            }
            11 => {
                let len = self.len()?;
                self.account(len)?;
                let bytes = self.take(len.checked_mul(4).ok_or("array too long")?)?;
                Value::Array(
                    bytes
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|c| Value::from(i32::from_be_bytes(*c)))
                        .collect(),
                )
            }
            12 => {
                let len = self.len()?;
                self.account(len)?;
                let bytes = self.take(len.checked_mul(8).ok_or("array too long")?)?;
                Value::Array(
                    bytes
                        .as_chunks::<8>()
                        .0
                        .iter()
                        .map(|c| Value::from(i64::from_be_bytes(*c)))
                        .collect(),
                )
            }
            other => return Err(format!("unknown tag type {other}")),
        })
    }
}

fn float_value(value: f64) -> Value {
    Number::from_f64(value).map_or(Value::Null, Value::Number)
}

fn unwrap_list_wrapper(value: Value) -> Value {
    match value {
        Value::Object(mut map) if map.len() == 1 && map.contains_key("") => {
            map.remove("").unwrap_or(Value::Null)
        }
        other => other,
    }
}

pub fn format_placeholders(template: &str, vars: &[(&str, &str)]) -> String {
    let mut result = template.to_string();
    for (key, value) in vars {
        result = result.replace(&format!("{{{key}}}"), value);
    }
    result
}

#[derive(Debug, Clone, PartialEq)]
pub struct TitleData {
    pub title: Component,
    pub subtitle: Component,
    pub fade_in_ticks: i32,
    pub stay_ticks: i32,
    pub fade_out_ticks: i32,
}

impl TitleData {
    #[must_use]
    pub const fn new(title: Component, subtitle: Component) -> Self {
        Self {
            title,
            subtitle,
            fade_in_ticks: 10,
            stay_ticks: 70,
            fade_out_ticks: 20,
        }
    }

    #[must_use]
    pub const fn fade_in(mut self, ticks: i32) -> Self {
        self.fade_in_ticks = ticks;
        self
    }

    #[must_use]
    pub const fn stay(mut self, ticks: i32) -> Self {
        self.stay_ticks = ticks;
        self
    }

    #[must_use]
    pub const fn fade_out(mut self, ticks: i32) -> Self {
        self.fade_out_ticks = ticks;
        self
    }
}
