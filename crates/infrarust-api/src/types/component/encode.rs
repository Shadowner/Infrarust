use std::borrow::Cow;

use serde_json::{Map, Number, Value};
use uuid::Uuid;

use super::legacy::plain_into;
use super::nbt::write_payload;
use super::validate::{is_chat_safe, is_valid_identifier, is_valid_untrusted_url};
use super::{
    ClickEvent, Component, Content, Decoration, HoverEvent, MAX_COMPONENT_DEPTH, MAX_RAW_DEPTH,
    NbtSource, ObjectContent, Style, TextColor,
};
use crate::types::ProtocolVersion;

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

pub(super) enum Ir {
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
    pub(super) const fn tag_id(&self) -> u8 {
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

pub(super) fn uuid_from_ints(ints: [i32; 4]) -> Uuid {
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

pub(super) fn raw_depth_within(value: &Value, remaining: usize) -> bool {
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
