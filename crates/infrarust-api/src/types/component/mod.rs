use std::fmt;

use serde_json::Value;
use uuid::Uuid;

mod encode;
mod legacy;
mod nbt;
mod parse;
mod validate;

pub const MAX_COMPONENT_DEPTH: usize = 64;
pub const MAX_NBT_DEPTH: usize = 512;
pub const LEGACY_SECTION: char = '\u{a7}';
pub const LEGACY_AMPERSAND: char = '&';

const MAX_RAW_DEPTH: usize = 128;

pub use infrarust_plugin_common::text::{Decoration, IntoTextColor, NamedColor, TextColor};

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

pub fn format_placeholders(template: &str, vars: &[(&str, &str)]) -> String {
    let mut result = template.to_string();
    for (key, value) in vars {
        result = result.replace(&format!("{{{key}}}"), value);
    }
    result
}
