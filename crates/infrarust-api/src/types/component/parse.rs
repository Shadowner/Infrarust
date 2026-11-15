use serde_json::{Map, Value};
use uuid::Uuid;

use super::encode::{raw_depth_within, uuid_from_ints};
use super::nbt::NbtReader;
use super::{
    ClickEvent, Component, ComponentParseError, Content, Decoration, HoverEvent,
    MAX_COMPONENT_DEPTH, MAX_RAW_DEPTH, NbtSource, ObjectContent, Style, TextColor,
};

impl Component {
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
