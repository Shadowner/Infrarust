//! Minimal Minecraft text-component builder.
//!
//! The host carries `component` as the canonical text-component JSON string;
//! this builds that JSON for the common text/color/format/extra subset the host
//! parses. Convert with [`Component::into_json`] (or `String::from`) before
//! passing to a host method.

/// A text component. Build with [`text`](Self::text) and chain styling.
#[derive(Clone, Default)]
pub struct Component {
    text: String,
    color: Option<String>,
    bold: bool,
    italic: bool,
    extra: Vec<Component>,
}

impl Component {
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn color(mut self, color: impl Into<String>) -> Self {
        self.color = Some(color.into());
        self
    }

    #[must_use]
    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    #[must_use]
    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    #[must_use]
    pub fn append(mut self, child: Component) -> Self {
        self.extra.push(child);
        self
    }

    #[must_use]
    pub fn into_json(self) -> String {
        let mut out = String::from("{\"text\":");
        push_json_string(&mut out, &self.text);
        if let Some(color) = &self.color {
            out.push_str(",\"color\":");
            push_json_string(&mut out, color);
        }
        if self.bold {
            out.push_str(",\"bold\":true");
        }
        if self.italic {
            out.push_str(",\"italic\":true");
        }
        if !self.extra.is_empty() {
            out.push_str(",\"extra\":[");
            for (i, child) in self.extra.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&child.into_json());
            }
            out.push(']');
        }
        out.push('}');
        out
    }
}

impl From<Component> for String {
    fn from(component: Component) -> Self {
        component.into_json()
    }
}

fn push_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}
