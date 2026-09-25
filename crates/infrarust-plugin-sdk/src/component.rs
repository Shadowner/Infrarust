use std::fmt;

use infrarust_plugin_wit::arena::{self, ArenaError, ArenaNode};

use crate::bindings::types as wt;
use crate::error::{Error, ErrorKind};

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

    #[must_use]
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

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|color| color.name().eq_ignore_ascii_case(name))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextColor {
    Named(NamedColor),
    Hex(u32),
}

impl TextColor {
    #[must_use]
    pub fn parse(input: &str) -> Option<Self> {
        let Some(hex) = input.strip_prefix('#') else {
            return NamedColor::from_name(input).map(Self::Named);
        };
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
        u32::from_str_radix(significant, 16).ok().map(Self::Hex)
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ClickEvent {
    OpenUrl(String),
    RunCommand(String),
    SuggestCommand(String),
    CopyToClipboard(String),
    ChangePage(i32),
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum HoverEvent {
    ShowText(Box<Component>),
}

impl HoverEvent {
    #[must_use]
    pub fn show_text(text: impl Into<Component>) -> Self {
        Self::ShowText(Box::new(text.into()))
    }
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
    pub hover: Option<HoverEvent>,
}

impl Style {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    #[must_use]
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

impl From<&String> for Component {
    fn from(text: &String) -> Self {
        Self::text(text.clone())
    }
}

impl From<&Component> for Component {
    fn from(component: &Component) -> Self {
        component.clone()
    }
}

impl Component {
    #[must_use]
    pub fn new(content: Content) -> Self {
        Self {
            content,
            style: Style::default(),
            children: Vec::new(),
        }
    }

    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::new(Content::Text(text.into()))
    }

    #[must_use]
    pub fn error(text: impl Into<String>) -> Self {
        Self::text(text).color(NamedColor::Red)
    }

    #[must_use]
    pub fn translatable(key: impl Into<String>) -> Self {
        Self::translatable_with(key, Vec::new())
    }

    #[must_use]
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

    #[must_use]
    pub fn keybind(keybind: impl Into<String>) -> Self {
        Self::new(Content::Keybind(keybind.into()))
    }

    #[must_use]
    pub fn fallback(mut self, text: impl Into<String>) -> Self {
        if let Content::Translatable { fallback, .. } = &mut self.content {
            *fallback = Some(text.into());
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
        self.style.hover = Some(event);
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
        iter.fold(first, |joined, component| {
            joined.append(separator.clone()).append(component)
        })
    }

    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match &self.content {
            Content::Text(text) => Some(text),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_plain_text(&self) -> bool {
        matches!(self.content, Content::Text(_))
            && self.style.is_empty()
            && self.children.is_empty()
    }

    #[must_use]
    pub fn to_plain(&self) -> String {
        let mut out = String::new();
        let mut pending = vec![self];
        while let Some(component) = pending.pop() {
            out.push_str(match &component.content {
                Content::Text(text) | Content::Keybind(text) => text,
                Content::Translatable { key, fallback, .. } => fallback.as_deref().unwrap_or(key),
            });
            pending.extend(component.children.iter().rev());
        }
        out
    }

    pub fn from_json(json: &str) -> Result<Self, Error> {
        let arena = crate::bindings::text::parse_json(json)?;
        Self::from_arena(arena).map_err(invalid)
    }

    #[must_use]
    pub fn from_legacy(legacy: &str) -> Self {
        Self::from_arena(crate::bindings::text::parse_legacy(legacy)).unwrap_or_default()
    }

    pub fn to_json(&self) -> Result<String, Error> {
        Ok(crate::bindings::text::to_json(&self.to_arena())?)
    }

    #[must_use]
    pub fn to_arena(&self) -> wt::Component {
        let mut nodes: Vec<wt::ComponentNode> = Vec::new();
        let mut pending: Vec<(&Self, Option<(usize, Slot)>)> = vec![(self, None)];
        while let Some((component, parent)) = pending.pop() {
            let index = nodes.len();
            let reference = u32::try_from(index).unwrap_or(u32::MAX);
            nodes.push(node_of(component));
            if let Some((parent, slot)) = parent {
                let parent = &mut nodes[parent];
                match slot {
                    Slot::Arg => {
                        if let wt::NodeContent::Translatable((_, args, _)) = &mut parent.content {
                            args.push(reference);
                        }
                    }
                    Slot::Hover => parent.hover = Some(wt::HoverEvent::ShowText(reference)),
                    Slot::Child => parent.children.push(reference),
                }
            }
            for child in component.children.iter().rev() {
                pending.push((child, Some((index, Slot::Child))));
            }
            if let Some(HoverEvent::ShowText(tooltip)) = &component.style.hover {
                pending.push((tooltip, Some((index, Slot::Hover))));
            }
            if let Content::Translatable { with, .. } = &component.content {
                for arg in with.iter().rev() {
                    pending.push((arg, Some((index, Slot::Arg))));
                }
            }
        }
        wt::Component { nodes }
    }

    pub fn from_arena(arena: wt::Component) -> Result<Self, ArenaError> {
        arena::validate(&arena.nodes)?;
        build(&arena.nodes, 0)
    }
}

impl fmt::Display for Component {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_plain())
    }
}

#[derive(Debug, Clone, Copy)]
enum Slot {
    Arg,
    Hover,
    Child,
}

fn invalid(error: ArenaError) -> Error {
    Error::new(ErrorKind::InvalidArgument, error.to_string())
}

fn node_of(component: &Component) -> wt::ComponentNode {
    let style = &component.style;
    wt::ComponentNode {
        content: match &component.content {
            Content::Text(text) => wt::NodeContent::Text(text.clone()),
            Content::Translatable { key, fallback, .. } => {
                wt::NodeContent::Translatable((key.clone(), Vec::new(), fallback.clone()))
            }
            Content::Keybind(keybind) => wt::NodeContent::Keybind(keybind.clone()),
        },
        style: wt::Style {
            color: style.color.map(|color| color.to_string()),
            bold: style.bold,
            italic: style.italic,
            underlined: style.underlined,
            strikethrough: style.strikethrough,
            obfuscated: style.obfuscated,
            font: style.font.clone(),
            insertion: style.insertion.clone(),
            shadow_color: style.shadow_color,
        },
        click: style.click.as_deref().map(|click| match click {
            ClickEvent::OpenUrl(url) => wt::ClickEvent::OpenUrl(url.clone()),
            ClickEvent::RunCommand(command) => wt::ClickEvent::RunCommand(command.clone()),
            ClickEvent::SuggestCommand(command) => wt::ClickEvent::SuggestCommand(command.clone()),
            ClickEvent::CopyToClipboard(value) => wt::ClickEvent::CopyToClipboard(value.clone()),
            ClickEvent::ChangePage(page) => wt::ClickEvent::ChangePage(*page),
        }),
        hover: None,
        children: Vec::new(),
    }
}

impl ArenaNode for wt::ComponentNode {
    fn references(&self, visit: &mut dyn FnMut(u32)) {
        if let wt::NodeContent::Translatable((_, args, _)) = &self.content {
            for arg in args {
                visit(*arg);
            }
        }
        if let Some(wt::HoverEvent::ShowText(tooltip)) = &self.hover {
            visit(*tooltip);
        }
        for child in &self.children {
            visit(*child);
        }
    }

    fn text_bytes(&self) -> usize {
        let content = match &self.content {
            wt::NodeContent::Text(text) | wt::NodeContent::Keybind(text) => text.len(),
            wt::NodeContent::Translatable((key, _, fallback)) => {
                key.len() + fallback.as_ref().map_or(0, String::len)
            }
        };
        let style = [&self.style.color, &self.style.font, &self.style.insertion]
            .into_iter()
            .map(|value| value.as_ref().map_or(0, String::len))
            .sum::<usize>();
        let click = match &self.click {
            Some(
                wt::ClickEvent::OpenUrl(value)
                | wt::ClickEvent::RunCommand(value)
                | wt::ClickEvent::SuggestCommand(value)
                | wt::ClickEvent::CopyToClipboard(value),
            ) => value.len(),
            Some(wt::ClickEvent::ChangePage(_)) | None => 0,
        };
        content + style + click
    }
}

fn build(nodes: &[wt::ComponentNode], index: u32) -> Result<Component, ArenaError> {
    let node = &nodes[index as usize];
    let content = match &node.content {
        wt::NodeContent::Text(text) => Content::Text(text.clone()),
        wt::NodeContent::Translatable((key, args, fallback)) => Content::Translatable {
            key: key.clone(),
            fallback: fallback.clone(),
            with: args
                .iter()
                .map(|arg| build(nodes, *arg))
                .collect::<Result<_, _>>()?,
        },
        wt::NodeContent::Keybind(keybind) => Content::Keybind(keybind.clone()),
    };
    let color = match &node.style.color {
        Some(color) => Some(
            TextColor::parse(color).ok_or_else(|| ArenaError::InvalidValue {
                node: index,
                reason: format!("unknown color {color:?}"),
            })?,
        ),
        None => None,
    };
    let hover = match &node.hover {
        Some(wt::HoverEvent::ShowText(tooltip)) => {
            Some(HoverEvent::ShowText(Box::new(build(nodes, *tooltip)?)))
        }
        None => None,
    };
    Ok(Component {
        content,
        style: Style {
            color,
            font: node.style.font.clone(),
            bold: node.style.bold,
            italic: node.style.italic,
            underlined: node.style.underlined,
            strikethrough: node.style.strikethrough,
            obfuscated: node.style.obfuscated,
            shadow_color: node.style.shadow_color,
            insertion: node.style.insertion.clone(),
            click: node.click.as_ref().map(|click| {
                Box::new(match click {
                    wt::ClickEvent::OpenUrl(url) => ClickEvent::OpenUrl(url.clone()),
                    wt::ClickEvent::RunCommand(command) => ClickEvent::RunCommand(command.clone()),
                    wt::ClickEvent::SuggestCommand(command) => {
                        ClickEvent::SuggestCommand(command.clone())
                    }
                    wt::ClickEvent::CopyToClipboard(value) => {
                        ClickEvent::CopyToClipboard(value.clone())
                    }
                    wt::ClickEvent::ChangePage(page) => ClickEvent::ChangePage(*page),
                })
            }),
            hover,
        },
        children: node
            .children
            .iter()
            .map(|child| build(nodes, *child))
            .collect::<Result<_, _>>()?,
    })
}

pub(crate) fn from_host(arena: wt::Component) -> Component {
    Component::from_arena(arena).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use infrarust_plugin_wit::arena::{MAX_DEPTH, MAX_NODES};

    use super::*;

    fn rich() -> Component {
        Component::text("Welcome ")
            .color(NamedColor::Gold)
            .bold()
            .underlined()
            .font("minecraft:uniform")
            .insertion("inserted")
            .shadow_color(0xFF00_00FF)
            .click(ClickEvent::RunCommand("/spawn".into()))
            .hover(HoverEvent::show_text(Component::text("tip").italic()))
            .append(Component::text("Steve").color("#55ff55").strikethrough())
            .append(
                Component::translatable_with(
                    "chat.type.text",
                    [Component::keybind("key.jump"), Component::text("b")],
                )
                .fallback("<%s>")
                .obfuscated()
                .click(ClickEvent::ChangePage(3)),
            )
    }

    #[test]
    fn a_rich_component_round_trips_through_the_arena() {
        let original = rich();
        let arena = original.to_arena();
        assert_eq!(arena.nodes.len(), 6);
        assert_eq!(arena::validate(&arena.nodes), Ok(()));
        assert_eq!(Component::from_arena(arena).unwrap(), original);
    }

    #[test]
    fn children_and_arguments_keep_their_order() {
        let component = Component::text("a")
            .append("b")
            .append(Component::translatable_with("k", ["x".into(), "y".into()]))
            .append("c");
        let back = Component::from_arena(component.to_arena()).unwrap();
        assert_eq!(back.to_plain(), "abkc");
        assert_eq!(back, component);
    }

    #[test]
    fn strings_are_plain_text_only() {
        let from_str = Component::from("{\"text\":\"x\",\"bold\":true}");
        assert!(from_str.is_plain_text());
        assert_eq!(from_str.as_text(), Some("{\"text\":\"x\",\"bold\":true}"));
        assert_eq!(Component::from(String::from("hi")), Component::text("hi"));
    }

    #[test]
    fn colors_parse_names_and_hex() {
        assert_eq!(
            TextColor::parse("GOLD"),
            Some(TextColor::Named(NamedColor::Gold))
        );
        assert_eq!(TextColor::parse("#55ff55"), Some(TextColor::Hex(0x55_FF55)));
        assert_eq!(TextColor::Hex(0xAB).to_string(), "#0000AB");
        assert_eq!(TextColor::parse("#1234567"), None);
        assert_eq!(TextColor::parse("mauve"), None);
        assert_eq!(Component::text("x").color("mauve").style.color, None);
    }

    #[test]
    fn plain_text_follows_the_tree_order() {
        assert_eq!(rich().to_plain(), "Welcome Steve<%s>");
    }

    #[test]
    fn a_deep_tree_is_encoded_without_recursion() {
        let mut deep = Component::text("leaf");
        for _ in 0..3_000 {
            deep = Component::text("x").append(deep);
        }
        let arena = deep.to_arena();
        assert_eq!(arena.nodes.len(), 3_001);
        assert!(matches!(
            arena::validate(&arena.nodes),
            Err(ArenaError::TooDeep { .. })
        ));
    }

    #[test]
    fn the_limits_are_the_contract_limits() {
        let mut deep = Component::text("leaf");
        for _ in 1..MAX_DEPTH {
            deep = Component::text("x").append(deep);
        }
        assert_eq!(arena::validate(&deep.to_arena().nodes), Ok(()));
        let deeper = Component::text("x").append(deep);
        assert!(matches!(
            arena::validate(&deeper.to_arena().nodes),
            Err(ArenaError::TooDeep { .. })
        ));

        let wide = (1..MAX_NODES).fold(Component::text("root"), |root, i| {
            root.append(Component::text(i.to_string()))
        });
        assert_eq!(arena::validate(&wide.to_arena().nodes), Ok(()));
    }

    #[test]
    fn an_invalid_arena_is_refused_when_decoded() {
        let shared = wt::Component {
            nodes: vec![
                wt::ComponentNode {
                    children: vec![1, 1],
                    ..Component::text("a").to_arena().nodes.remove(0)
                },
                Component::text("b").to_arena().nodes.remove(0),
            ],
        };
        assert_eq!(
            Component::from_arena(shared),
            Err(ArenaError::SharedNode { node: 1 })
        );
        assert_eq!(
            from_host(wt::Component { nodes: vec![] }),
            Component::empty()
        );
    }
}
