//! Minecraft rich text component types.
//!
//! The [`Component`] type represents Minecraft's JSON text format used for
//! chat messages, titles, action bar text, and kick reasons.

use std::fmt;

/// A Minecraft rich text component.
///
/// Supports builder-style construction for readable message creation.
///
/// # Example
/// ```
/// use infrarust_api::types::Component;
///
/// let msg = Component::text("Hello ")
///     .color("gold")
///     .bold()
///     .append(Component::text("World!").color("white"));
///
/// assert_eq!(msg.text, "Hello ");
/// assert_eq!(msg.extra.len(), 1);
/// ```
#[derive(Debug, Clone, Default)]
pub struct Component {
    /// The literal text content.
    pub text: String,
    /// Text color (e.g. `"gold"`, `"red"`, `"#ff5555"`).
    pub color: Option<String>,
    /// Bold formatting.
    pub bold: Option<bool>,
    /// Italic formatting.
    pub italic: Option<bool>,
    /// Underlined formatting.
    pub underlined: Option<bool>,
    /// Strikethrough formatting.
    pub strikethrough: Option<bool>,
    /// Obfuscated (magic) formatting.
    pub obfuscated: Option<bool>,
    /// Child components appended after this component's text.
    pub extra: Vec<Self>,
    /// Click event triggered when this component is clicked.
    pub click_event: Option<ClickEvent>,
    /// Hover event triggered when this component is hovered.
    pub hover_event: Option<HoverEvent>,
}

/// An action triggered when a text component is clicked.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ClickEvent {
    /// Opens a URL in the player's browser.
    OpenUrl(String),
    /// Suggests a command in the player's chat input.
    SuggestCommand(String),
    /// Runs a command as the player.
    RunCommand(String),
    /// Copies text to the player's clipboard.
    CopyToClipboard(String),
}

/// An action triggered when a text component is hovered.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum HoverEvent {
    /// Shows a text tooltip.
    ShowText(Box<Component>),
}

impl Component {
    /// Creates a new text component with the given content.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Default::default()
        }
    }

    /// Creates an error-styled component (red text).
    pub fn error(text: impl Into<String>) -> Self {
        Self::text(text).color("red")
    }

    /// Sets the text color.
    #[must_use]
    pub fn color(mut self, color: impl Into<String>) -> Self {
        self.color = Some(color.into());
        self
    }

    /// Enables bold formatting.
    #[must_use]
    pub const fn bold(mut self) -> Self {
        self.bold = Some(true);
        self
    }

    /// Enables italic formatting.
    #[must_use]
    pub const fn italic(mut self) -> Self {
        self.italic = Some(true);
        self
    }

    /// Enables underline formatting.
    #[must_use]
    pub const fn underlined(mut self) -> Self {
        self.underlined = Some(true);
        self
    }

    /// Enables strikethrough formatting.
    #[must_use]
    pub const fn strikethrough(mut self) -> Self {
        self.strikethrough = Some(true);
        self
    }

    /// Enables obfuscated (magic) formatting.
    #[must_use]
    pub const fn obfuscated(mut self) -> Self {
        self.obfuscated = Some(true);
        self
    }

    /// Appends a child component after this component's text.
    #[must_use]
    pub fn append(mut self, child: Self) -> Self {
        self.extra.push(child);
        self
    }

    /// Sets a click event on this component.
    #[must_use]
    pub fn click(mut self, event: ClickEvent) -> Self {
        self.click_event = Some(event);
        self
    }

    /// Sets a hover event on this component.
    #[must_use]
    pub fn hover(mut self, event: HoverEvent) -> Self {
        self.hover_event = Some(event);
        self
    }

    /// Joins multiple components with a separator between them.
    ///
    /// # Example
    /// ```
    /// use infrarust_api::types::Component;
    ///
    /// let parts = vec![
    ///     Component::text("one"),
    ///     Component::text("two"),
    ///     Component::text("three"),
    /// ];
    /// let joined = Component::join(parts, &Component::text(", "));
    /// assert_eq!(joined.text, "one");
    /// // "one" + ", " + "two" + ", " + "three"
    /// assert_eq!(joined.extra.len(), 4);
    /// ```
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
}

impl Component {
    /// Serializes this component to Minecraft's JSON text component format.
    ///
    /// Produces a JSON string suitable for pre-1.20.3 packet encoding.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut out = String::from("{");
        // text field is always present
        out.push_str(&format!("\"text\":\"{}\"", Self::escape_json(&self.text)));

        if let Some(ref color) = self.color {
            out.push_str(&format!(",\"color\":\"{}\"", Self::escape_json(color)));
        }
        if self.bold == Some(true) {
            out.push_str(",\"bold\":true");
        }
        if self.italic == Some(true) {
            out.push_str(",\"italic\":true");
        }
        if self.underlined == Some(true) {
            out.push_str(",\"underlined\":true");
        }
        if self.strikethrough == Some(true) {
            out.push_str(",\"strikethrough\":true");
        }
        if self.obfuscated == Some(true) {
            out.push_str(",\"obfuscated\":true");
        }

        if let Some(ref click) = self.click_event {
            let (action, value) = match click {
                ClickEvent::OpenUrl(url) => ("open_url", url.as_str()),
                ClickEvent::SuggestCommand(cmd) => ("suggest_command", cmd.as_str()),
                ClickEvent::RunCommand(cmd) => ("run_command", cmd.as_str()),
                ClickEvent::CopyToClipboard(text) => ("copy_to_clipboard", text.as_str()),
            };
            out.push_str(&format!(
                ",\"clickEvent\":{{\"action\":\"{action}\",\"value\":\"{}\"}}",
                Self::escape_json(value)
            ));
        }

        if let Some(ref hover) = self.hover_event {
            match hover {
                HoverEvent::ShowText(component) => {
                    out.push_str(&format!(
                        ",\"hoverEvent\":{{\"action\":\"show_text\",\"contents\":{}}}",
                        component.to_json()
                    ));
                }
            }
        }

        if !self.extra.is_empty() {
            out.push_str(",\"extra\":[");
            for (i, child) in self.extra.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&child.to_json());
            }
            out.push(']');
        }

        out.push('}');
        out
    }

    /// Serializes this component to Minecraft Network NBT format (1.20.3+).
    ///
    /// Network NBT differs from standard NBT: the root compound has only a
    /// type byte (`0x0A`) with no name length or name bytes.
    #[must_use]
    pub fn to_nbt_network(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(64);
        buf.push(0x0A); // TAG_Compound (root, no name for Network NBT)
        self.write_nbt_compound_content(&mut buf);
        buf
    }

    /// Writes the compound fields and TAG_End for this component.
    /// Used for both the root compound and nested compounds (extra, hover).
    fn write_nbt_compound_content(&self, buf: &mut Vec<u8>) {
        // text (always present)
        Self::write_nbt_string_field(buf, "text", &self.text);

        if let Some(ref color) = self.color {
            Self::write_nbt_string_field(buf, "color", color);
        }
        if self.bold == Some(true) {
            Self::write_nbt_byte_field(buf, "bold", 1);
        }
        if self.italic == Some(true) {
            Self::write_nbt_byte_field(buf, "italic", 1);
        }
        if self.underlined == Some(true) {
            Self::write_nbt_byte_field(buf, "underlined", 1);
        }
        if self.strikethrough == Some(true) {
            Self::write_nbt_byte_field(buf, "strikethrough", 1);
        }
        if self.obfuscated == Some(true) {
            Self::write_nbt_byte_field(buf, "obfuscated", 1);
        }

        if let Some(ref click) = self.click_event {
            let (action, value) = match click {
                ClickEvent::OpenUrl(url) => ("open_url", url.as_str()),
                ClickEvent::SuggestCommand(cmd) => ("suggest_command", cmd.as_str()),
                ClickEvent::RunCommand(cmd) => ("run_command", cmd.as_str()),
                ClickEvent::CopyToClipboard(text) => ("copy_to_clipboard", text.as_str()),
            };
            buf.push(0x0A); // TAG_Compound
            Self::write_nbt_name(buf, "clickEvent");
            Self::write_nbt_string_field(buf, "action", action);
            Self::write_nbt_string_field(buf, "value", value);
            buf.push(0x00); // TAG_End
        }

        if let Some(ref hover) = self.hover_event {
            match hover {
                HoverEvent::ShowText(component) => {
                    buf.push(0x0A); // TAG_Compound
                    Self::write_nbt_name(buf, "hoverEvent");
                    Self::write_nbt_string_field(buf, "action", "show_text");
                    buf.push(0x0A); // TAG_Compound (contents)
                    Self::write_nbt_name(buf, "contents");
                    component.write_nbt_compound_content(buf);
                    buf.push(0x00); // TAG_End (hoverEvent)
                }
            }
        }

        if !self.extra.is_empty() {
            buf.push(0x09); // TAG_List
            Self::write_nbt_name(buf, "extra");
            buf.push(0x0A); // element type = TAG_Compound
            buf.extend_from_slice(&(self.extra.len() as i32).to_be_bytes());
            for child in &self.extra {
                child.write_nbt_compound_content(buf);
            }
        }

        buf.push(0x00); // TAG_End
    }

    /// Writes an NBT TAG_String field (type byte + name + value).
    fn write_nbt_string_field(buf: &mut Vec<u8>, name: &str, value: &str) {
        buf.push(0x08); // TAG_String
        Self::write_nbt_name(buf, name);
        let bytes = value.as_bytes();
        buf.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
        buf.extend_from_slice(bytes);
    }

    /// Writes an NBT TAG_Byte field (type byte + name + value).
    fn write_nbt_byte_field(buf: &mut Vec<u8>, name: &str, value: u8) {
        buf.push(0x01); // TAG_Byte
        Self::write_nbt_name(buf, name);
        buf.push(value);
    }

    /// Writes an NBT field name (u16 length + UTF-8 bytes).
    fn write_nbt_name(buf: &mut Vec<u8>, name: &str) {
        let bytes = name.as_bytes();
        buf.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
        buf.extend_from_slice(bytes);
    }

    /// Escapes a string for JSON embedding.
    fn escape_json(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if c < '\x20' => {
                    out.push_str(&format!("\\u{:04x}", c as u32));
                }
                c => out.push(c),
            }
        }
        out
    }
}

impl fmt::Display for Component {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)?;
        for child in &self.extra {
            write!(f, "{child}")?;
        }
        Ok(())
    }
}

/// Data for a title display (title + subtitle + timing).
///
/// # Example
/// ```
/// use infrarust_api::types::{Component, TitleData};
///
/// let title = TitleData::new(
///     Component::text("Welcome!").color("gold"),
///     Component::text("Enjoy your stay").color("gray"),
/// );
/// assert_eq!(title.fade_in_ticks, 10);
/// ```
#[derive(Debug, Clone)]
pub struct TitleData {
    /// The main title text.
    pub title: Component,
    pub subtitle: Component,
    /// Fade-in duration in ticks (default: 10).
    pub fade_in_ticks: i32,
    /// Stay duration in ticks (default: 70).
    pub stay_ticks: i32,
    /// Fade-out duration in ticks (default: 20).
    pub fade_out_ticks: i32,
}

impl TitleData {
    /// Creates a new title with default timings (10 fade-in, 70 stay, 20 fade-out).
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

    /// Sets the fade-in duration in ticks.
    #[must_use]
    pub const fn fade_in(mut self, ticks: i32) -> Self {
        self.fade_in_ticks = ticks;
        self
    }

    /// Sets the stay duration in ticks.
    #[must_use]
    pub const fn stay(mut self, ticks: i32) -> Self {
        self.stay_ticks = ticks;
        self
    }

    /// Sets the fade-out duration in ticks.
    #[must_use]
    pub const fn fade_out(mut self, ticks: i32) -> Self {
        self.fade_out_ticks = ticks;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_constructor() {
        let c = Component::text("Hello");
        assert_eq!(c.text, "Hello");
        assert!(c.color.is_none());
        assert!(c.bold.is_none());
        assert!(c.extra.is_empty());
    }

    #[test]
    fn error_constructor() {
        let c = Component::error("Bad!");
        assert_eq!(c.text, "Bad!");
        assert_eq!(c.color.as_deref(), Some("red"));
    }

    #[test]
    fn builder_chain() {
        let c = Component::text("Hello")
            .color("gold")
            .bold()
            .italic()
            .append(Component::text(" World").color("white"));

        assert_eq!(c.text, "Hello");
        assert_eq!(c.color.as_deref(), Some("gold"));
        assert_eq!(c.bold, Some(true));
        assert_eq!(c.italic, Some(true));
        assert_eq!(c.extra.len(), 1);
        assert_eq!(c.extra[0].text, " World");
    }

    #[test]
    fn display_flattens_text() {
        let c = Component::text("A")
            .append(Component::text("B"))
            .append(Component::text("C"));
        assert_eq!(c.to_string(), "ABC");
    }

    #[test]
    fn join_components() {
        let parts = vec![
            Component::text("a"),
            Component::text("b"),
            Component::text("c"),
        ];
        let joined = Component::join(parts, &Component::text(", "));
        assert_eq!(joined.to_string(), "a, b, c");
    }

    #[test]
    fn join_empty() {
        let joined = Component::join(vec![], &Component::text(", "));
        assert_eq!(joined.to_string(), "");
    }

    #[test]
    fn join_single() {
        let joined = Component::join(vec![Component::text("only")], &Component::text(", "));
        assert_eq!(joined.to_string(), "only");
    }

    #[test]
    fn title_data_defaults() {
        let t = TitleData::new(Component::text("Hi"), Component::text("Sub"));
        assert_eq!(t.fade_in_ticks, 10);
        assert_eq!(t.stay_ticks, 70);
        assert_eq!(t.fade_out_ticks, 20);
    }

    #[test]
    fn title_data_builder() {
        let t = TitleData::new(Component::text("Hi"), Component::text("Sub"))
            .fade_in(5)
            .stay(100)
            .fade_out(10);
        assert_eq!(t.fade_in_ticks, 5);
        assert_eq!(t.stay_ticks, 100);
        assert_eq!(t.fade_out_ticks, 10);
    }

    #[test]
    fn nbt_network_plain_text() {
        let c = Component::text("Hello");
        let nbt = c.to_nbt_network();
        // Root TAG_Compound (0x0A), no name
        assert_eq!(nbt[0], 0x0A);
        // TAG_String (0x08) for "text" field
        assert_eq!(nbt[1], 0x08);
        // name length = 4 ("text")
        assert_eq!(&nbt[2..4], &[0x00, 0x04]);
        assert_eq!(&nbt[4..8], b"text");
        // value length = 5 ("Hello")
        assert_eq!(&nbt[8..10], &[0x00, 0x05]);
        assert_eq!(&nbt[10..15], b"Hello");
        // TAG_End
        assert_eq!(nbt[15], 0x00);
        assert_eq!(nbt.len(), 16);
    }

    #[test]
    fn nbt_network_with_color_and_bold() {
        let c = Component::text("Hi").color("red").bold();
        let nbt = c.to_nbt_network();
        assert_eq!(nbt[0], 0x0A); // root compound
        // Should contain "text", "color", "bold" fields + TAG_End
        // Find "color" TAG_String (0x08)
        let nbt_str = String::from_utf8_lossy(&nbt);
        assert!(nbt_str.contains("text"));
        assert!(nbt_str.contains("color"));
        assert!(nbt_str.contains("red"));
        assert!(nbt_str.contains("bold"));
        // Last byte is TAG_End
        assert_eq!(*nbt.last().unwrap(), 0x00);
    }

    #[test]
    fn nbt_network_with_extra() {
        let c = Component::text("Hello ")
            .append(Component::text("World").color("gold"));
        let nbt = c.to_nbt_network();
        assert_eq!(nbt[0], 0x0A); // root compound
        // Should contain TAG_List (0x09) for "extra"
        assert!(nbt.contains(&0x09));
        let nbt_str = String::from_utf8_lossy(&nbt);
        assert!(nbt_str.contains("extra"));
        assert!(nbt_str.contains("World"));
        assert!(nbt_str.contains("gold"));
    }

    #[test]
    fn nbt_network_with_click_event() {
        let c = Component::text("Click me")
            .click(ClickEvent::OpenUrl("https://example.com".into()));
        let nbt = c.to_nbt_network();
        assert_eq!(nbt[0], 0x0A);
        let nbt_str = String::from_utf8_lossy(&nbt);
        assert!(nbt_str.contains("clickEvent"));
        assert!(nbt_str.contains("open_url"));
        assert!(nbt_str.contains("https://example.com"));
    }

    #[test]
    fn nbt_network_with_hover_event() {
        let c = Component::text("Hover me")
            .hover(HoverEvent::ShowText(Box::new(Component::text("Tooltip"))));
        let nbt = c.to_nbt_network();
        assert_eq!(nbt[0], 0x0A);
        let nbt_str = String::from_utf8_lossy(&nbt);
        assert!(nbt_str.contains("hoverEvent"));
        assert!(nbt_str.contains("show_text"));
        assert!(nbt_str.contains("Tooltip"));
    }

    #[test]
    fn click_event_non_exhaustive() {
        let event = ClickEvent::OpenUrl("https://example.com".into());
        #[allow(unreachable_patterns)]
        match event {
            ClickEvent::OpenUrl(_)
            | ClickEvent::SuggestCommand(_)
            | ClickEvent::RunCommand(_)
            | ClickEvent::CopyToClipboard(_)
            | _ => {}
        }
    }
}
