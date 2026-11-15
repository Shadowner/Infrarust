use super::{
    Component, Content, Decoration, LEGACY_AMPERSAND, MAX_COMPONENT_DEPTH, NamedColor, Style,
    TextColor, format_placeholders,
};

impl Component {
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

pub(super) fn plain_into(component: &Component, depth: usize, out: &mut String) {
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

fn color_state(color: TextColor) -> LegacyState {
    LegacyState {
        color: Some(color),
        decorations: [false; 5],
    }
}
