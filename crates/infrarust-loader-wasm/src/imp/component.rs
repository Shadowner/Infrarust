use infrarust_api::types::{ClickEvent, Component, Content, HoverEvent, Style, TextColor};
use infrarust_plugin_wit::arena::{self, ArenaError, ArenaNode, MAX_DEPTH, MAX_NODES};

use crate::bindings::infrarust::plugin::types as wt;

pub(crate) const FALLBACK_TEXT: &str = "[invalid text component]";

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

pub(crate) fn to_wit(component: &Component) -> wt::Component {
    let mut nodes = Vec::new();
    push(component, 1, &mut nodes);
    wt::Component { nodes }
}

pub(crate) fn from_wit(component: &wt::Component) -> Result<Component, ArenaError> {
    arena::validate(&component.nodes)?;
    build(&component.nodes, 0)
}

pub(crate) fn from_wit_or_fallback(
    component: &wt::Component,
    plugin_id: &str,
    what: &str,
) -> Component {
    from_wit(component).unwrap_or_else(|error| {
        tracing::warn!(plugin = %plugin_id, what, %error,
            "wasm plugin returned an invalid text component; using a fallback text");
        fallback()
    })
}

pub(crate) fn fallback() -> Component {
    Component::text(FALLBACK_TEXT)
}

fn index_of(nodes: &[wt::ComponentNode]) -> u32 {
    u32::try_from(nodes.len()).unwrap_or(u32::MAX)
}

fn push(component: &Component, depth: usize, nodes: &mut Vec<wt::ComponentNode>) -> Option<u32> {
    if nodes.len() >= MAX_NODES {
        return None;
    }
    let index = index_of(nodes);
    nodes.push(wt::ComponentNode {
        content: wt::NodeContent::Text(String::new()),
        style: style_to_wit(&component.style),
        click: component.style.click.as_deref().and_then(click_to_wit),
        hover: None,
        children: Vec::new(),
    });
    let nested = depth < MAX_DEPTH;
    let content = match &component.content {
        Content::Text(text) => wt::NodeContent::Text(text.clone()),
        Content::Translatable {
            key,
            fallback,
            with,
        } => {
            let args = if nested {
                with.iter()
                    .filter_map(|arg| push(arg, depth + 1, nodes))
                    .collect()
            } else {
                Vec::new()
            };
            wt::NodeContent::Translatable((key.clone(), args, fallback.clone()))
        }
        Content::Keybind(keybind) => wt::NodeContent::Keybind(keybind.clone()),
        Content::Selector { pattern, .. } => wt::NodeContent::Text(pattern.clone()),
        _ => wt::NodeContent::Text(String::new()),
    };
    let hover = match component.style.hover.as_deref() {
        Some(HoverEvent::ShowText(tooltip)) if nested => {
            push(tooltip, depth + 1, nodes).map(wt::HoverEvent::ShowText)
        }
        _ => None,
    };
    let children = if nested {
        component
            .children
            .iter()
            .filter_map(|child| push(child, depth + 1, nodes))
            .collect()
    } else {
        Vec::new()
    };
    let node = &mut nodes[index as usize];
    node.content = content;
    node.hover = hover;
    node.children = children;
    Some(index)
}

fn style_to_wit(style: &Style) -> wt::Style {
    wt::Style {
        color: style.color.map(|color| color.to_string()),
        bold: style.bold,
        italic: style.italic,
        underlined: style.underlined,
        strikethrough: style.strikethrough,
        obfuscated: style.obfuscated,
        font: style.font.clone(),
        insertion: style.insertion.clone(),
        shadow_color: style.shadow_color,
    }
}

fn click_to_wit(click: &ClickEvent) -> Option<wt::ClickEvent> {
    Some(match click {
        ClickEvent::OpenUrl(url) => wt::ClickEvent::OpenUrl(url.clone()),
        ClickEvent::RunCommand(command) => wt::ClickEvent::RunCommand(command.clone()),
        ClickEvent::SuggestCommand(command) => wt::ClickEvent::SuggestCommand(command.clone()),
        ClickEvent::CopyToClipboard(value) => wt::ClickEvent::CopyToClipboard(value.clone()),
        ClickEvent::ChangePage(page) => wt::ClickEvent::ChangePage(*page),
        _ => return None,
    })
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
    let mut style = Style::default();
    if let Some(color) = &node.style.color {
        style.color = Some(
            TextColor::parse(color).ok_or_else(|| ArenaError::InvalidValue {
                node: index,
                reason: format!("unknown color {color:?}"),
            })?,
        );
    }
    style.bold = node.style.bold;
    style.italic = node.style.italic;
    style.underlined = node.style.underlined;
    style.strikethrough = node.style.strikethrough;
    style.obfuscated = node.style.obfuscated;
    style.font.clone_from(&node.style.font);
    style.insertion.clone_from(&node.style.insertion);
    style.shadow_color = node.style.shadow_color;
    style.click = node
        .click
        .as_ref()
        .map(|click| Box::new(click_from_wit(click)));
    if let Some(wt::HoverEvent::ShowText(tooltip)) = &node.hover {
        style.hover = Some(Box::new(HoverEvent::ShowText(Box::new(build(
            nodes, *tooltip,
        )?))));
    }
    let mut component = Component::new(content);
    component.style = style;
    component.children = node
        .children
        .iter()
        .map(|child| build(nodes, *child))
        .collect::<Result<_, _>>()?;
    Ok(component)
}

fn click_from_wit(click: &wt::ClickEvent) -> ClickEvent {
    match click {
        wt::ClickEvent::OpenUrl(url) => ClickEvent::OpenUrl(url.clone()),
        wt::ClickEvent::RunCommand(command) => ClickEvent::RunCommand(command.clone()),
        wt::ClickEvent::SuggestCommand(command) => ClickEvent::SuggestCommand(command.clone()),
        wt::ClickEvent::CopyToClipboard(value) => ClickEvent::CopyToClipboard(value.clone()),
        wt::ClickEvent::ChangePage(page) => ClickEvent::ChangePage(*page),
    }
}

#[cfg(test)]
mod tests {
    use infrarust_api::types::NamedColor;

    use super::*;

    fn plain(text: &str) -> wt::ComponentNode {
        wt::ComponentNode {
            content: wt::NodeContent::Text(text.to_owned()),
            style: style_to_wit(&Style::default()),
            click: None,
            hover: None,
            children: Vec::new(),
        }
    }

    fn with_children(text: &str, children: &[u32]) -> wt::ComponentNode {
        wt::ComponentNode {
            children: children.to_vec(),
            ..plain(text)
        }
    }

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
                Component::translatable_with("chat.type.text", [Component::keybind("key.jump")])
                    .fallback("<%s>")
                    .obfuscated()
                    .click(ClickEvent::ChangePage(3)),
            )
    }

    #[test]
    fn a_rich_component_survives_a_round_trip() {
        let original = rich();
        let arena = to_wit(&original);
        assert_eq!(arena.nodes.len(), 5);
        assert_eq!(from_wit(&arena).unwrap(), original);
        assert_eq!(from_wit(&arena).unwrap().to_json(), original.to_json());
    }

    #[test]
    fn the_arena_the_host_builds_is_always_valid() {
        let arena = to_wit(&rich());
        assert_eq!(arena::validate(&arena.nodes), Ok(()));
    }

    #[test]
    fn a_cycle_is_refused() {
        let arena = wt::Component {
            nodes: vec![with_children("a", &[1]), with_children("b", &[0])],
        };
        assert!(matches!(
            from_wit(&arena),
            Err(ArenaError::BackwardReference { .. })
        ));
    }

    #[test]
    fn a_shared_node_is_refused() {
        let mut root = with_children("root", &[1, 2]);
        root.hover = Some(wt::HoverEvent::ShowText(2));
        let arena = wt::Component {
            nodes: vec![root, plain("a"), plain("b")],
        };
        assert_eq!(from_wit(&arena), Err(ArenaError::SharedNode { node: 2 }));
    }

    #[test]
    fn a_tree_deeper_than_the_limit_is_refused() {
        let depth = MAX_DEPTH + 1;
        let nodes = (0..depth)
            .map(|i| {
                if i + 1 < depth {
                    with_children("x", &[u32::try_from(i + 1).unwrap()])
                } else {
                    plain("x")
                }
            })
            .collect();
        assert!(matches!(
            from_wit(&wt::Component { nodes }),
            Err(ArenaError::TooDeep { .. })
        ));
    }

    #[test]
    fn too_much_text_is_refused() {
        let arena = wt::Component {
            nodes: vec![plain(&"x".repeat(arena::MAX_TEXT_BYTES + 1))],
        };
        assert!(matches!(
            from_wit(&arena),
            Err(ArenaError::TooMuchText { .. })
        ));
    }

    #[test]
    fn an_unknown_color_is_refused() {
        let mut node = plain("x");
        node.style.color = Some("not-a-color".into());
        let arena = wt::Component { nodes: vec![node] };
        assert!(matches!(
            from_wit(&arena),
            Err(ArenaError::InvalidValue { node: 0, .. })
        ));
    }

    #[test]
    fn an_invalid_outcome_text_becomes_the_fallback() {
        let arena = wt::Component { nodes: Vec::new() };
        assert_eq!(from_wit_or_fallback(&arena, "p", "deny"), fallback());
    }

    #[test]
    fn contents_the_contract_does_not_carry_degrade_to_text() {
        let component = Component::selector("@a")
            .append(Component::score("Steve", "kills"))
            .hover(HoverEvent::show_item("minecraft:stone", 1))
            .click(ClickEvent::Custom {
                id: "x:y".into(),
                payload: None,
            });
        let back = from_wit(&to_wit(&component)).unwrap();
        assert_eq!(back.to_plain(), "@a");
        assert!(back.style.hover.is_none());
        assert!(back.style.click.is_none());
        assert_eq!(back.children.len(), 1);
    }

    #[test]
    fn the_host_truncates_what_it_sends_to_the_limits() {
        let mut deep = Component::text("leaf");
        for _ in 0..(MAX_DEPTH * 2) {
            deep = Component::text("x").append(deep);
        }
        let arena = to_wit(&deep);
        assert_eq!(arena.nodes.len(), MAX_DEPTH);
        assert_eq!(arena::validate(&arena.nodes), Ok(()));

        let wide = (0..MAX_NODES + 10).fold(Component::text("root"), |root, i| {
            root.append(Component::text(i.to_string()))
        });
        let arena = to_wit(&wide);
        assert_eq!(arena.nodes.len(), MAX_NODES);
        assert_eq!(arena::validate(&arena.nodes), Ok(()));
    }
}
