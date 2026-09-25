#![allow(clippy::unwrap_used, clippy::expect_used)]

use infrarust_api::types::{
    ClickEvent, Component, ComponentParseError, Content, Decoration, HoverEvent, LEGACY_SECTION,
    MAX_COMPONENT_DEPTH, NamedColor, NbtSource, ObjectContent, ProtocolVersion, Style, TextColor,
};
use proptest::prelude::*;
use serde_json::{Value, json};
use uuid::Uuid;

const BOUNDARIES: [i32; 20] = [
    5, 47, 316, 335, 393, 477, 573, 578, 735, 754, 755, 762, 764, 765, 766, 769, 770, 771, 773, 774,
];

fn pv(raw: i32) -> ProtocolVersion {
    ProtocolVersion::new(raw)
}

fn json_at(component: &Component, raw: i32) -> Value {
    component.to_json_value_for(pv(raw))
}

fn nbt_roundtrip(component: &Component, raw: i32) -> Component {
    Component::from_nbt_network(&component.to_nbt_for(pv(raw))).expect("valid nbt")
}

fn uuid_abcd() -> Uuid {
    Uuid::parse_str("0000000a-0000-000b-0000-000c0000000d").unwrap()
}

fn kitchen_sink() -> Component {
    Component::text("Hello ")
        .color(TextColor::Hex(0xAB_2211))
        .bold()
        .decoration(Decoration::Italic, Some(false))
        .font("minecraft:uniform")
        .shadow_color(0xFF00_0000)
        .insertion("ins")
        .click(ClickEvent::RunCommand("/say hi".into()))
        .hover(HoverEvent::show_text("tip"))
        .append(Component::keybind("key.jump"))
        .append(
            Component::translatable_with(
                "chat.type.text",
                [
                    Component::text("Steve"),
                    Component::text("hi").color(NamedColor::Gold),
                ],
            )
            .fallback("<%s> %s"),
        )
}

#[test]
fn golden_kitchen_sink_1_8() {
    let expected = json!({
        "text": "Hello ",
        "color": "dark_red",
        "bold": true,
        "italic": false,
        "insertion": "ins",
        "clickEvent": {"action": "run_command", "value": "/say hi"},
        "hoverEvent": {"action": "show_text", "value": {"text": "tip"}},
        "extra": [
            {"translate": "key.jump"},
            {"translate": "chat.type.text", "with": [{"text": "Steve"}, {"text": "hi", "color": "gold"}]}
        ]
    });
    assert_eq!(json_at(&kitchen_sink(), 47), expected);
}

#[test]
fn golden_kitchen_sink_1_15_2() {
    let expected = json!({
        "text": "Hello ",
        "color": "dark_red",
        "bold": true,
        "italic": false,
        "insertion": "ins",
        "clickEvent": {"action": "run_command", "value": "/say hi"},
        "hoverEvent": {"action": "show_text", "value": {"text": "tip"}},
        "extra": [
            {"keybind": "key.jump"},
            {"translate": "chat.type.text", "with": [{"text": "Steve"}, {"text": "hi", "color": "gold"}]}
        ]
    });
    assert_eq!(json_at(&kitchen_sink(), 578), expected);
}

#[test]
fn golden_kitchen_sink_1_16() {
    let expected = json!({
        "text": "Hello ",
        "color": "#AB2211",
        "bold": true,
        "italic": false,
        "font": "minecraft:uniform",
        "insertion": "ins",
        "clickEvent": {"action": "run_command", "value": "/say hi"},
        "hoverEvent": {"action": "show_text", "contents": {"text": "tip"}},
        "extra": [
            {"keybind": "key.jump"},
            {"translate": "chat.type.text", "with": [{"text": "Steve"}, {"text": "hi", "color": "gold"}]}
        ]
    });
    assert_eq!(json_at(&kitchen_sink(), 735), expected);
}

#[test]
fn golden_kitchen_sink_1_20_2() {
    let expected = json!({
        "text": "Hello ",
        "color": "#AB2211",
        "bold": true,
        "italic": false,
        "font": "minecraft:uniform",
        "insertion": "ins",
        "clickEvent": {"action": "run_command", "value": "/say hi"},
        "hoverEvent": {"action": "show_text", "contents": {"text": "tip"}},
        "extra": [
            {"keybind": "key.jump"},
            {
                "translate": "chat.type.text",
                "fallback": "<%s> %s",
                "with": [{"text": "Steve"}, {"text": "hi", "color": "gold"}]
            }
        ]
    });
    assert_eq!(json_at(&kitchen_sink(), 764), expected);
}

#[test]
fn golden_kitchen_sink_1_21_4() {
    let expected = json!({
        "text": "Hello ",
        "color": "#AB2211",
        "bold": true,
        "italic": false,
        "font": "minecraft:uniform",
        "shadow_color": -16_777_216,
        "insertion": "ins",
        "clickEvent": {"action": "run_command", "value": "/say hi"},
        "hoverEvent": {"action": "show_text", "contents": "tip"},
        "extra": [
            {"keybind": "key.jump"},
            {
                "translate": "chat.type.text",
                "fallback": "<%s> %s",
                "with": ["Steve", {"text": "hi", "color": "gold"}]
            }
        ]
    });
    assert_eq!(json_at(&kitchen_sink(), 769), expected);
}

fn modern_kitchen_sink() -> Value {
    json!({
        "text": "Hello ",
        "color": "#AB2211",
        "bold": true,
        "italic": false,
        "font": "minecraft:uniform",
        "shadow_color": -16_777_216,
        "insertion": "ins",
        "click_event": {"action": "run_command", "command": "/say hi"},
        "hover_event": {"action": "show_text", "value": "tip"},
        "extra": [
            {"keybind": "key.jump"},
            {
                "translate": "chat.type.text",
                "fallback": "<%s> %s",
                "with": ["Steve", {"text": "hi", "color": "gold"}]
            }
        ]
    })
}

#[test]
fn golden_kitchen_sink_1_21_5() {
    assert_eq!(json_at(&kitchen_sink(), 770), modern_kitchen_sink());
}

#[test]
fn golden_kitchen_sink_1_21_6() {
    assert_eq!(json_at(&kitchen_sink(), 771), modern_kitchen_sink());
}

#[test]
fn golden_kitchen_sink_current() {
    assert_eq!(json_at(&kitchen_sink(), 774), modern_kitchen_sink());
}

#[test]
fn legacy_to_json_matches_1_20_2() {
    let parsed: Value = serde_json::from_str(&kitchen_sink().to_json()).unwrap();
    assert_eq!(parsed, json_at(&kitchen_sink(), 764));
}

#[test]
fn insertion_starts_at_1_8() {
    let c = Component::text("x").insertion("i");
    assert_eq!(json_at(&c, 5), json!({"text": "x"}));
    assert_eq!(json_at(&c, 47), json!({"text": "x", "insertion": "i"}));
}

#[test]
fn score_and_selector_start_at_1_8() {
    let score = Component::score("@p", "kills");
    assert_eq!(json_at(&score, 5), json!({"text": ""}));
    assert_eq!(
        json_at(&score, 47),
        json!({"score": {"name": "@p", "objective": "kills"}})
    );
    let selector = Component::selector("@a");
    assert_eq!(json_at(&selector, 5), json!({"text": "@a"}));
    assert_eq!(json_at(&selector, 47), json!({"selector": "@a"}));
}

#[test]
fn keybind_falls_back_to_translate_before_1_12() {
    let c = Component::keybind("key.jump");
    assert_eq!(json_at(&c, 316), json!({"translate": "key.jump"}));
    assert_eq!(json_at(&c, 335), json!({"keybind": "key.jump"}));
}

#[test]
fn hex_color_and_font_start_at_1_16() {
    let c = Component::text("x")
        .color(TextColor::Hex(0xEC_41AA))
        .font("minecraft:alt");
    assert_eq!(
        json_at(&c, 578),
        json!({"text": "x", "color": "light_purple"})
    );
    assert_eq!(
        json_at(&c, 735),
        json!({"text": "x", "color": "#EC41AA", "font": "minecraft:alt"})
    );
}

#[test]
fn downsampling_matches_adventure_nearest_color() {
    assert_eq!(NamedColor::nearest(0xFF_0000), NamedColor::DarkRed);
    assert_eq!(NamedColor::nearest(0xAB_2211), NamedColor::DarkRed);
    assert_eq!(NamedColor::nearest(0xEC_41AA), NamedColor::LightPurple);
    assert_eq!(NamedColor::nearest(0x80_8080), NamedColor::Gray);
    for named in NamedColor::ALL {
        assert_eq!(NamedColor::nearest(named.rgb()), named);
    }
}

#[test]
fn invalid_font_is_dropped() {
    let c = Component::text("x").font("Not A Font");
    assert_eq!(json_at(&c, 774), json!("x"));
}

#[test]
fn show_text_hover_across_versions() {
    let c = Component::text("h").hover(HoverEvent::show_text(Component::text("t").bold()));
    assert_eq!(
        json_at(&c, 578)["hoverEvent"],
        json!({"action": "show_text", "value": {"text": "t", "bold": true}})
    );
    assert_eq!(
        json_at(&c, 735)["hoverEvent"],
        json!({"action": "show_text", "contents": {"text": "t", "bold": true}})
    );
    assert_eq!(
        json_at(&c, 770)["hover_event"],
        json!({"action": "show_text", "value": {"text": "t", "bold": true}})
    );
}

#[test]
fn copy_to_clipboard_starts_at_1_15() {
    let c = Component::text("c").click(ClickEvent::CopyToClipboard("v".into()));
    assert_eq!(json_at(&c, 572), json!({"text": "c"}));
    assert_eq!(
        json_at(&c, 573)["clickEvent"],
        json!({"action": "copy_to_clipboard", "value": "v"})
    );
    assert_eq!(
        json_at(&c, 770)["click_event"],
        json!({"action": "copy_to_clipboard", "value": "v"})
    );
}

#[test]
fn show_entity_across_versions() {
    let hover = HoverEvent::ShowEntity {
        entity_type: "minecraft:pig".into(),
        uuid: uuid_abcd(),
        name: Some(Box::new(Component::text("Bob"))),
    };
    let c = Component::text("e").hover(hover);
    assert_eq!(json_at(&c, 5), json!({"text": "e"}));
    assert_eq!(
        json_at(&c, 47)["hoverEvent"],
        json!({
            "action": "show_entity",
            "value": {"text": "{id:\"0000000a-0000-000b-0000-000c0000000d\",type:\"minecraft:pig\",name:\"Bob\"}"}
        })
    );
    assert_eq!(
        json_at(&c, 578)["hoverEvent"],
        json!({
            "action": "show_entity",
            "value": {"text": "{id:\"0000000a-0000-000b-0000-000c0000000d\",type:\"minecraft:pig\",name:\"{\\\"text\\\":\\\"Bob\\\"}\"}"}
        })
    );
    assert_eq!(
        json_at(&c, 764)["hoverEvent"],
        json!({
            "action": "show_entity",
            "contents": {"type": "minecraft:pig", "id": "0000000a-0000-000b-0000-000c0000000d", "name": {"text": "Bob"}}
        })
    );
    assert_eq!(
        json_at(&c, 765)["hoverEvent"],
        json!({
            "action": "show_entity",
            "contents": {"type": "minecraft:pig", "id": [10, 11, 12, 13], "name": "Bob"}
        })
    );
    assert_eq!(
        json_at(&c, 770)["hover_event"],
        json!({"action": "show_entity", "id": "minecraft:pig", "uuid": [10, 11, 12, 13], "name": "Bob"})
    );
}

#[test]
fn show_entity_matches_adventure_int_array_vector() {
    let uuid = Uuid::from_u128(0x0123_4567_89AB_CDEF_FEDC_BA98_7654_3210);
    let c = Component::text("").hover(HoverEvent::show_entity("minecraft:zombie", uuid));
    let most = 0x0123_4567_89AB_CDEF_u64;
    let least = 0xFEDC_BA98_7654_3210_u64;
    let ints: Vec<i32> = [
        most >> 32,
        most & 0xFFFF_FFFF,
        least >> 32,
        least & 0xFFFF_FFFF,
    ]
    .iter()
    .map(|v| i32::from_ne_bytes((*v as u32).to_ne_bytes()))
    .collect();
    assert_eq!(
        json_at(&c, 774)["hover_event"],
        json!({"action": "show_entity", "id": "minecraft:zombie", "uuid": ints})
    );
}

#[test]
fn show_item_across_versions() {
    let hover = HoverEvent::ShowItem {
        id: "minecraft:diamond_sword".into(),
        count: 2,
        components: Some(json!({"minecraft:damage": 5})),
    };
    let c = Component::text("i").hover(hover);
    assert_eq!(
        json_at(&c, 47)["hoverEvent"],
        json!({"action": "show_item", "value": {"text": "{id:\"minecraft:diamond_sword\",Count:2b}"}})
    );
    assert_eq!(
        json_at(&c, 735)["hoverEvent"],
        json!({"action": "show_item", "contents": {"id": "minecraft:diamond_sword", "count": 2}})
    );
    assert_eq!(
        json_at(&c, 766)["hoverEvent"],
        json!({
            "action": "show_item",
            "contents": {"id": "minecraft:diamond_sword", "count": 2, "components": {"minecraft:damage": 5}}
        })
    );
    assert_eq!(
        json_at(&c, 770)["hover_event"],
        json!({
            "action": "show_item",
            "id": "minecraft:diamond_sword",
            "count": 2,
            "components": {"minecraft:damage": 5}
        })
    );
}

#[test]
fn show_item_matches_adventure_null_tag_vector() {
    let c = Component::text("").hover(HoverEvent::show_item("minecraft:diamond", 2));
    assert_eq!(
        json_at(&c, 774),
        json!({"text": "", "hover_event": {"action": "show_item", "id": "minecraft:diamond", "count": 2}})
    );
}

#[test]
fn show_item_count_clamped_to_valid_range() {
    let c = Component::text("i").hover(HoverEvent::show_item("minecraft:stone", 500));
    assert_eq!(json_at(&c, 774)["hover_event"]["count"], json!(99));
    assert_eq!(
        json_at(&c, 47)["hoverEvent"]["value"]["text"],
        json!("{id:\"minecraft:stone\",Count:127b}")
    );
    let one = Component::text("i").hover(HoverEvent::show_item("minecraft:stone", 1));
    assert_eq!(
        json_at(&one, 735)["hoverEvent"]["contents"],
        json!({"id": "minecraft:stone"})
    );
}

#[test]
fn change_page_value_type_changes_in_1_21_5() {
    let c = Component::text("p").click(ClickEvent::ChangePage(3));
    assert_eq!(
        json_at(&c, 769)["clickEvent"],
        json!({"action": "change_page", "value": "3"})
    );
    assert_eq!(
        json_at(&c, 770)["click_event"],
        json!({"action": "change_page", "page": 3})
    );
    let zero = Component::text("p").click(ClickEvent::ChangePage(0));
    assert_eq!(json_at(&zero, 770), json!("p"));
}

#[test]
fn open_url_validated_from_1_21_5() {
    let bad = Component::text("u").click(ClickEvent::OpenUrl("ftp://example.com".into()));
    assert_eq!(
        json_at(&bad, 769)["clickEvent"],
        json!({"action": "open_url", "value": "ftp://example.com"})
    );
    assert_eq!(json_at(&bad, 770), json!("u"));
    let good = Component::text("u").click(ClickEvent::OpenUrl(
        "HTTPS://Example.com/a%20b?q=1&r=2#frag".into(),
    ));
    assert_eq!(
        json_at(&good, 770)["click_event"],
        json!({"action": "open_url", "url": "HTTPS://Example.com/a%20b?q=1&r=2#frag"})
    );
    for invalid in [
        "https://exa mple.com",
        "https://",
        "http:",
        "https://a/%zz",
        "https://a/#b#c",
        "https://a/<script>",
        "javascript:alert(1)",
        "example.com",
    ] {
        let c = Component::text("u").click(ClickEvent::OpenUrl(invalid.into()));
        assert_eq!(json_at(&c, 774), json!("u"), "{invalid}");
    }
}

#[test]
fn commands_validated_from_1_21_5() {
    let bad = Component::text("c").click(ClickEvent::RunCommand("/say a\nb".into()));
    assert!(json_at(&bad, 769).get("clickEvent").is_some());
    assert_eq!(json_at(&bad, 770), json!("c"));
    let section = Component::text("c").click(ClickEvent::SuggestCommand("/msg \u{a7}c".into()));
    assert_eq!(json_at(&section, 774), json!("c"));
    let fine = Component::text("c").click(ClickEvent::SuggestCommand("msg Steve ".into()));
    assert_eq!(
        json_at(&fine, 774)["click_event"],
        json!({"action": "suggest_command", "command": "msg Steve "})
    );
}

#[test]
fn custom_click_starts_at_1_21_6() {
    let c = Component::text("x").click(ClickEvent::Custom {
        id: "infrarust:ping".into(),
        payload: Some("p".into()),
    });
    assert_eq!(json_at(&c, 764), json!({"text": "x"}));
    assert_eq!(json_at(&c, 769), json!("x"));
    assert_eq!(json_at(&c, 770), json!("x"));
    assert_eq!(
        json_at(&c, 771)["click_event"],
        json!({"action": "custom", "id": "infrarust:ping", "payload": "p"})
    );
    let invalid = Component::text("x").click(ClickEvent::Custom {
        id: "Bad Id".into(),
        payload: None,
    });
    assert_eq!(json_at(&invalid, 774), json!("x"));
}

#[test]
fn shadow_color_starts_at_1_21_4() {
    let c = Component::text("s").shadow_color(0x80FF_0000);
    assert_eq!(json_at(&c, 768), json!("s"));
    assert_eq!(
        json_at(&c, 769),
        json!({"text": "s", "shadow_color": i32::from_ne_bytes(0x80FF_0000_u32.to_ne_bytes())})
    );
}

#[test]
fn translate_fallback_starts_at_1_19_4() {
    let c = Component::translatable("a.b").fallback("F");
    assert_eq!(json_at(&c, 761), json!({"translate": "a.b"}));
    assert_eq!(
        json_at(&c, 762),
        json!({"translate": "a.b", "fallback": "F"})
    );
}

#[test]
fn separator_starts_at_1_17() {
    let c = Component::selector("@a").separator(Component::text("|").color(NamedColor::Gray));
    assert_eq!(json_at(&c, 754), json!({"selector": "@a"}));
    assert_eq!(
        json_at(&c, 755),
        json!({"selector": "@a", "separator": {"text": "|", "color": "gray"}})
    );
    assert_eq!(
        json_at(&c, 774),
        json!({"selector": "@a", "separator": {"text": "|", "color": "gray"}})
    );
}

#[test]
fn nbt_content_boundaries() {
    let block = Component::new(Content::Nbt {
        path: "Items[0]".into(),
        interpret: Some(true),
        separator: None,
        source: NbtSource::Block("~ ~ ~".into()),
    });
    assert_eq!(json_at(&block, 404), json!({"text": ""}));
    assert_eq!(
        json_at(&block, 477),
        json!({"nbt": "Items[0]", "interpret": true, "block": "~ ~ ~"})
    );
    let storage = Component::new(Content::Nbt {
        path: "x".into(),
        interpret: None,
        separator: None,
        source: NbtSource::Storage("infrarust:data".into()),
    });
    assert_eq!(json_at(&storage, 498), json!({"text": ""}));
    assert_eq!(
        json_at(&storage, 573),
        json!({"nbt": "x", "storage": "infrarust:data"})
    );
}

#[test]
fn object_content_starts_at_1_21_9() {
    let atlas = Component::new(Content::Object(ObjectContent::Atlas {
        atlas: None,
        sprite: "item/porkchop".into(),
    }));
    assert_eq!(json_at(&atlas, 772), json!(""));
    assert_eq!(json_at(&atlas, 773), json!({"sprite": "item/porkchop"}));
    let player = Component::new(Content::Object(ObjectContent::Player {
        player: json!({"name": "Notch"}),
        hat: Some(false),
    }));
    assert_eq!(
        json_at(&player, 774),
        json!({"player": {"name": "Notch"}, "hat": false})
    );
}

#[test]
fn plain_text_collapses_from_1_20_3() {
    let c = Component::text("hi");
    assert_eq!(json_at(&c, 764), json!({"text": "hi"}));
    assert_eq!(json_at(&c, 765), json!("hi"));
    let with_child = Component::text("a").append(Component::text("b"));
    assert_eq!(
        json_at(&with_child, 765),
        json!({"text": "a", "extra": ["b"]})
    );
    assert_eq!(
        json_at(&with_child, 764),
        json!({"text": "a", "extra": [{"text": "b"}]})
    );
}

#[test]
fn nbt_plain_text_is_bare_string_tag() {
    let bytes = Component::text("hi").to_nbt_for(pv(774));
    assert_eq!(bytes, vec![0x08, 0x00, 0x02, b'h', b'i']);
    assert_eq!(
        Component::from_nbt_network(&bytes).unwrap(),
        Component::text("hi")
    );
}

fn nbt_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&u16::try_from(s.len()).unwrap().to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn nbt_field(out: &mut Vec<u8>, tag: u8, name: &str) {
    out.push(tag);
    nbt_str(out, name);
}

#[test]
fn nbt_hand_built_fixture_with_click_and_hover() {
    let mut fixture = vec![0x0A];
    nbt_field(&mut fixture, 0x08, "text");
    nbt_str(&mut fixture, "Hi");
    nbt_field(&mut fixture, 0x01, "bold");
    fixture.push(1);
    nbt_field(&mut fixture, 0x0A, "click_event");
    nbt_field(&mut fixture, 0x08, "action");
    nbt_str(&mut fixture, "open_url");
    nbt_field(&mut fixture, 0x08, "url");
    nbt_str(&mut fixture, "https://a.b");
    fixture.push(0);
    nbt_field(&mut fixture, 0x0A, "hover_event");
    nbt_field(&mut fixture, 0x08, "action");
    nbt_str(&mut fixture, "show_text");
    nbt_field(&mut fixture, 0x08, "value");
    nbt_str(&mut fixture, "tip");
    fixture.push(0);
    fixture.push(0);

    let expected = Component::text("Hi")
        .bold()
        .click(ClickEvent::OpenUrl("https://a.b".into()))
        .hover(HoverEvent::show_text("tip"));
    assert_eq!(Component::from_nbt_network(&fixture).unwrap(), expected);
    assert_eq!(expected.to_nbt_for(pv(774)), fixture);
}

#[test]
fn nbt_camel_case_fixture_for_1_20_3() {
    let mut fixture = vec![0x0A];
    nbt_field(&mut fixture, 0x08, "text");
    nbt_str(&mut fixture, "e");
    nbt_field(&mut fixture, 0x0A, "hoverEvent");
    nbt_field(&mut fixture, 0x08, "action");
    nbt_str(&mut fixture, "show_entity");
    nbt_field(&mut fixture, 0x0A, "contents");
    nbt_field(&mut fixture, 0x08, "type");
    nbt_str(&mut fixture, "minecraft:pig");
    nbt_field(&mut fixture, 0x0B, "id");
    fixture.extend_from_slice(&4i32.to_be_bytes());
    for part in [10i32, 11, 12, 13] {
        fixture.extend_from_slice(&part.to_be_bytes());
    }
    fixture.push(0);
    fixture.push(0);
    fixture.push(0);

    let expected =
        Component::text("e").hover(HoverEvent::show_entity("minecraft:pig", uuid_abcd()));
    assert_eq!(Component::from_nbt_network(&fixture).unwrap(), expected);
    assert_eq!(expected.to_nbt_for(pv(765)), fixture);
}

#[test]
fn nbt_heterogeneous_list_wrappers_are_unwrapped() {
    let mut fixture = vec![0x0A];
    nbt_field(&mut fixture, 0x08, "text");
    nbt_str(&mut fixture, "");
    nbt_field(&mut fixture, 0x09, "extra");
    fixture.push(0x0A);
    fixture.extend_from_slice(&2i32.to_be_bytes());
    nbt_field(&mut fixture, 0x08, "");
    nbt_str(&mut fixture, "a");
    fixture.push(0);
    nbt_field(&mut fixture, 0x08, "text");
    nbt_str(&mut fixture, "b");
    nbt_field(&mut fixture, 0x01, "italic");
    fixture.push(0);
    fixture.push(0);
    fixture.push(0);

    let parsed = Component::from_nbt_network(&fixture).unwrap();
    let expected = Component::text("")
        .append(Component::text("a"))
        .append(Component::text("b").decoration(Decoration::Italic, Some(false)));
    assert_eq!(parsed, expected);
}

#[test]
fn nbt_raw_components_heterogeneous_list_round_trips() {
    let components = json!({"minecraft:lore": ["plain", {"text": "styled", "bold": true}]});
    let c = Component::text("i").hover(HoverEvent::ShowItem {
        id: "minecraft:stick".into(),
        count: 1,
        components: Some(components.clone()),
    });
    let back = nbt_roundtrip(&c, 774);
    let Some(HoverEvent::ShowItem {
        components: Some(parsed),
        ..
    }) = back.style.hover.as_deref()
    else {
        panic!("show_item lost: {back:?}");
    };
    assert_eq!(parsed["minecraft:lore"][0], json!("plain"));
    assert_eq!(parsed["minecraft:lore"][1]["text"], json!("styled"));
    assert_eq!(parsed["minecraft:lore"][1]["bold"], json!(1));
}

#[test]
fn nbt_strings_use_modified_utf8() {
    let bytes = Component::text("\0\u{1F600}").to_nbt_for(pv(774));
    assert_eq!(
        bytes,
        vec![
            0x08, 0x00, 0x08, 0xC0, 0x80, 0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80
        ]
    );
    assert_eq!(
        Component::from_nbt_network(&bytes).unwrap(),
        Component::text("\0\u{1F600}")
    );
}

#[test]
fn nbt_oversized_string_truncated_at_char_boundary() {
    let mut s = "a".repeat(65_534);
    s.push('\u{20AC}');
    let bytes = Component::text(s).to_nbt_for(pv(774));
    assert_eq!(bytes[0], 0x08);
    assert_eq!(u16::from_be_bytes([bytes[1], bytes[2]]), 65_534);
    assert_eq!(bytes.len(), 3 + 65_534);
}

#[test]
fn nbt_round_trips_kitchen_sink_at_every_nbt_version() {
    let c = kitchen_sink();
    assert_eq!(nbt_roundtrip(&c, 774), c);
    assert_eq!(nbt_roundtrip(&c, 770), c);
    assert_eq!(nbt_roundtrip(&c, 769), c);
    let mut without_shadow = c.clone();
    without_shadow.style.shadow_color = None;
    assert_eq!(nbt_roundtrip(&c, 765), without_shadow);
}

#[test]
fn nbt_legacy_method_keeps_camel_case() {
    let c = Component::text("x").click(ClickEvent::RunCommand("/a".into()));
    let bytes = c.to_nbt_network();
    assert_eq!(bytes, c.to_nbt_for(ProtocolVersion::MINECRAFT_1_21_4));
    assert!(bytes.windows(10).any(|w| w == b"clickEvent"));
}

#[test]
fn nbt_rejects_malformed_input() {
    assert!(Component::from_nbt_network(&[]).is_err());
    assert!(Component::from_nbt_network(&[0x00]).is_err());
    assert!(Component::from_nbt_network(&[0x03, 0, 0, 0, 1]).is_err());
    assert!(Component::from_nbt_network(&[0x0A, 0x08, 0x00, 0x04, b't']).is_err());
    assert!(Component::from_nbt_network(&[0x09, 0x00, 0x7F, 0xFF, 0xFF, 0xFF]).is_err());
    assert!(Component::from_nbt_network(&[0x0A, 0x0D, 0x00, 0x00, 0x00]).is_err());
    assert!(matches!(
        Component::from_nbt_network(&[0x08, 0x00, 0x01, b'a', 0xFF]),
        Err(ComponentParseError::InvalidNbt(_))
    ));
    let (c, used) = Component::from_nbt_network_prefix(&[0x08, 0x00, 0x01, b'a', 0xFF]).unwrap();
    assert_eq!(c, Component::text("a"));
    assert_eq!(used, 4);
}

#[test]
fn nbt_depth_is_bounded() {
    let levels = 600;
    let mut bytes = vec![0x09];
    for _ in 0..levels {
        bytes.push(0x09);
        bytes.extend_from_slice(&1i32.to_be_bytes());
    }
    bytes.push(0x00);
    bytes.extend_from_slice(&0i32.to_be_bytes());
    assert!(Component::from_nbt_network(&bytes).is_err());
}

#[test]
fn parser_accepts_both_casings() {
    let camel = Component::from_json(
        r#"{"text":"x","clickEvent":{"action":"open_url","value":"https://a.b"},"hoverEvent":{"action":"show_text","contents":"t"}}"#,
    )
    .unwrap();
    let snake = Component::from_json(
        r#"{"text":"x","click_event":{"action":"open_url","url":"https://a.b"},"hover_event":{"action":"show_text","value":"t"}}"#,
    )
    .unwrap();
    let legacy_value = Component::from_json(
        r#"{"text":"x","clickEvent":{"action":"open_url","value":"https://a.b"},"hoverEvent":{"action":"show_text","value":{"text":"t"}}}"#,
    )
    .unwrap();
    let expected = Component::text("x")
        .click(ClickEvent::OpenUrl("https://a.b".into()))
        .hover(HoverEvent::show_text("t"));
    assert_eq!(camel, expected);
    assert_eq!(snake, expected);
    assert_eq!(legacy_value, expected);
}

#[test]
fn parser_accepts_both_entity_forms() {
    let camel = Component::from_json(
        r#"{"text":"e","hoverEvent":{"action":"show_entity","contents":{"type":"minecraft:pig","id":"0000000a-0000-000b-0000-000c0000000d","name":"Bob"}}}"#,
    )
    .unwrap();
    let int_array = Component::from_json(
        r#"{"text":"e","hoverEvent":{"action":"show_entity","contents":{"type":"minecraft:pig","id":[10,11,12,13],"name":"Bob"}}}"#,
    )
    .unwrap();
    let snake = Component::from_json(
        r#"{"text":"e","hover_event":{"action":"show_entity","id":"minecraft:pig","uuid":[10,11,12,13],"name":{"text":"Bob"}}}"#,
    )
    .unwrap();
    let expected = Component::text("e").hover(HoverEvent::ShowEntity {
        entity_type: "minecraft:pig".into(),
        uuid: uuid_abcd(),
        name: Some(Box::new(Component::text("Bob"))),
    });
    assert_eq!(camel, expected);
    assert_eq!(int_array, expected);
    assert_eq!(snake, expected);
}

#[test]
fn parser_accepts_all_item_forms() {
    let id_only = Component::from_json(
        r#"{"text":"i","hoverEvent":{"action":"show_item","contents":"minecraft:stick"}}"#,
    )
    .unwrap();
    assert_eq!(
        id_only,
        Component::text("i").hover(HoverEvent::show_item("minecraft:stick", 1))
    );
    let contents = Component::from_json(
        r#"{"text":"i","hoverEvent":{"action":"show_item","contents":{"id":"minecraft:stick","count":2,"components":{"minecraft:damage":1}}}}"#,
    )
    .unwrap();
    let inline = Component::from_json(
        r#"{"text":"i","hover_event":{"action":"show_item","id":"minecraft:stick","count":2,"components":{"minecraft:damage":1}}}"#,
    )
    .unwrap();
    let expected = Component::text("i").hover(HoverEvent::ShowItem {
        id: "minecraft:stick".into(),
        count: 2,
        components: Some(json!({"minecraft:damage": 1})),
    });
    assert_eq!(contents, expected);
    assert_eq!(inline, expected);
}

#[test]
fn parser_accepts_change_page_string_and_int() {
    let old =
        Component::from_json(r#"{"text":"p","clickEvent":{"action":"change_page","value":"4"}}"#)
            .unwrap();
    let new =
        Component::from_json(r#"{"text":"p","click_event":{"action":"change_page","page":4}}"#)
            .unwrap();
    let expected = Component::text("p").click(ClickEvent::ChangePage(4));
    assert_eq!(old, expected);
    assert_eq!(new, expected);
}

#[test]
fn parser_accepts_custom_click_and_drops_unknown_actions() {
    let custom = Component::from_json(
        r#"{"text":"c","click_event":{"action":"custom","id":"infrarust:ping","payload":"x"}}"#,
    )
    .unwrap();
    assert_eq!(
        custom,
        Component::text("c").click(ClickEvent::Custom {
            id: "infrarust:ping".into(),
            payload: Some("x".into())
        })
    );
    let file =
        Component::from_json(r#"{"text":"c","clickEvent":{"action":"open_file","value":"/etc"}}"#)
            .unwrap();
    assert_eq!(file, Component::text("c"));
}

#[test]
fn parser_handles_every_content_kind() {
    let translate = Component::from_json(
        r#"{"translate":"chat.type.text","fallback":"F","with":["a",{"text":"b"},3,true]}"#,
    )
    .unwrap();
    assert_eq!(
        translate,
        Component::translatable_with(
            "chat.type.text",
            [
                Component::text("a"),
                Component::text("b"),
                Component::text("3"),
                Component::text("true")
            ]
        )
        .fallback("F")
    );
    assert_eq!(
        Component::from_json(r#"{"keybind":"key.jump"}"#).unwrap(),
        Component::keybind("key.jump")
    );
    assert_eq!(
        Component::from_json(r#"{"score":{"name":"*","objective":"o"}}"#).unwrap(),
        Component::score("*", "o")
    );
    assert_eq!(
        Component::from_json(r#"{"selector":"@e","separator":", "}"#).unwrap(),
        Component::selector("@e").separator(Component::text(", "))
    );
    assert_eq!(
        Component::from_json(r#"{"nbt":"p","storage":"a:b","interpret":1}"#).unwrap(),
        Component::new(Content::Nbt {
            path: "p".into(),
            interpret: Some(true),
            separator: None,
            source: NbtSource::Storage("a:b".into()),
        })
    );
    assert_eq!(
        Component::from_json(r#"{"nbt":"p","source":"block","block":"0 0 0","entity":"@s"}"#)
            .unwrap()
            .content,
        Content::Nbt {
            path: "p".into(),
            interpret: None,
            separator: None,
            source: NbtSource::Block("0 0 0".into()),
        }
    );
    assert_eq!(
        Component::from_json(r#"{"nbt":"p","block":"0 0 0","entity":"@s"}"#)
            .unwrap()
            .content,
        Content::Nbt {
            path: "p".into(),
            interpret: None,
            separator: None,
            source: NbtSource::Entity("@s".into()),
        }
    );
    assert_eq!(
        Component::from_json(
            r#"{"object":"atlas","atlas":"minecraft:blocks","sprite":"block/stone"}"#
        )
        .unwrap()
        .content,
        Content::Object(ObjectContent::Atlas {
            atlas: Some("minecraft:blocks".into()),
            sprite: "block/stone".into()
        })
    );
    assert_eq!(
        Component::from_json(r#"{"player":"Notch","hat":false}"#)
            .unwrap()
            .content,
        Content::Object(ObjectContent::Player {
            player: json!("Notch"),
            hat: Some(false)
        })
    );
    assert_eq!(
        Component::from_json(r#"{"type":"keybind","text":"x","keybind":"k"}"#).unwrap(),
        Component::keybind("k")
    );
    assert_eq!(
        Component::from_json(r#"{"type":"keybind","text":"x"}"#).unwrap(),
        Component::text("x")
    );
}

#[test]
fn parser_handles_roots() {
    assert_eq!(
        Component::from_json(r#""hi""#).unwrap(),
        Component::text("hi")
    );
    assert_eq!(
        Component::from_json(r#"["a",{"text":"b","color":"gold"}]"#).unwrap(),
        Component::text("a").append(Component::text("b").color(NamedColor::Gold))
    );
    assert_eq!(Component::from_json("7").unwrap(), Component::text("7"));
    assert_eq!(Component::from_json("null").unwrap(), Component::text(""));
    assert_eq!(Component::from_json("[]").unwrap(), Component::text(""));
    assert!(Component::from_json("{not json").is_err());
    assert!(matches!(
        Component::from_json(""),
        Err(ComponentParseError::InvalidJson(_))
    ));
}

#[test]
fn parser_reads_style_fields() {
    let c = Component::from_json(
        r##"{"text":"s","color":"#ff5555","font":"minecraft:alt","bold":true,"italic":false,"underlined":1,"strikethrough":"true","obfuscated":false,"shadow_color":[1.0,0.0,0.0,0.5],"insertion":"i"}"##,
    )
    .unwrap();
    let mut style = Style::new();
    style.color = Some(TextColor::Hex(0xFF_5555));
    style.font = Some("minecraft:alt".into());
    style.bold = Some(true);
    style.italic = Some(false);
    style.underlined = Some(true);
    style.strikethrough = Some(true);
    style.obfuscated = Some(false);
    style.shadow_color = Some(0x7FFF_0000);
    style.insertion = Some("i".into());
    assert_eq!(c.style, style);
    assert_eq!(
        Component::from_json(r#"{"text":"s","shadow_color":-16777216}"#)
            .unwrap()
            .style
            .shadow_color,
        Some(0xFF00_0000)
    );
    assert_eq!(
        Component::from_json(r#"{"text":"s","color":"not_a_color"}"#)
            .unwrap()
            .style
            .color,
        None
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
    assert_eq!(TextColor::Hex(0xab_cdef).to_string(), "#ABCDEF");
}

#[test]
fn builder_keeps_existing_ergonomics() {
    let c = Component::text("Hello")
        .color("gold")
        .bold()
        .italic()
        .underlined()
        .strikethrough()
        .obfuscated()
        .append(Component::text(" World").color("white"));
    assert_eq!(c.as_text(), Some("Hello"));
    assert_eq!(c.style.color, Some(NamedColor::Gold.into()));
    assert_eq!(c.style.bold, Some(true));
    assert_eq!(c.style.obfuscated, Some(true));
    assert_eq!(c.children.len(), 1);
    assert_eq!(c.to_string(), "Hello World");
    assert_eq!(
        Component::error("bad").style.color,
        Some(NamedColor::Red.into())
    );
    assert_eq!(Component::text("x").color("nope").style.color, None);
    let joined = Component::join(
        vec![
            Component::text("a"),
            Component::text("b"),
            Component::text("c"),
        ],
        &Component::text(", "),
    );
    assert_eq!(joined.to_plain(), "a, b, c");
    assert_eq!(
        Component::join(vec![], &Component::text(", ")).to_plain(),
        ""
    );
}

#[test]
fn legacy_parse_matches_adventure_vectors() {
    assert_eq!(
        Component::from_legacy("&a&lfoo&9bar"),
        Component::text("")
            .append(Component::text("foo").color(NamedColor::Green).bold())
            .append(Component::text("bar").color(NamedColor::Blue))
    );
    assert_eq!(
        Component::from_legacy("&a&9foo"),
        Component::text("foo").color(NamedColor::Blue)
    );
    assert_eq!(Component::from_legacy("&q"), Component::text("&q"));
    assert_eq!(Component::from_legacy("&#no"), Component::text("&#no"));
    assert_eq!(
        Component::from_legacy_with(
            "\u{a7}x\u{a7}f\u{a7}f\u{a7}e\u{a7}f\u{a7}d\u{a7}5Kittens!",
            LEGACY_SECTION
        ),
        Component::text("Kittens!").color(TextColor::Hex(0xFF_EFD5))
    );
    assert_eq!(
        Component::from_legacy_with(
            "\u{a7}cHugs and \u{a7}x\u{a7}f\u{a7}f\u{a7}e\u{a7}f\u{a7}d\u{a7}5Kittens!",
            LEGACY_SECTION
        ),
        Component::text("")
            .append(Component::text("Hugs and ").color(NamedColor::Red))
            .append(Component::text("Kittens!").color(TextColor::Hex(0xFF_EFD5)))
    );
    assert_eq!(
        Component::from_legacy_with(
            "\u{a7}cHugs and \u{a7}f\u{a7}f\u{a7}e\u{a7}f\u{a7}d\u{a7}5Kittens!",
            LEGACY_SECTION
        ),
        Component::text("")
            .append(Component::text("Hugs and ").color(NamedColor::Red))
            .append(Component::text("Kittens!").color(NamedColor::DarkPurple))
    );
    assert_eq!(
        Component::from_legacy_with(
            "Happy with \u{a7}x\u{a7}6\u{a7}b\u{a7}4\u{a7}6\u{a7}6\u{a7}8Lavender and \u{a7}x\u{a7}f\u{a7}f\u{a7}e\u{a7}f\u{a7}d\u{a7}5Cyan!",
            LEGACY_SECTION
        ),
        Component::text("Happy with ")
            .append(Component::text("Lavender and ").color(TextColor::Hex(0x6B_4668)))
            .append(Component::text("Cyan!").color(TextColor::Hex(0xFF_EFD5)))
    );
    assert_eq!(
        Component::from_legacy_with("\u{a7}x\u{a7}eKittens!", LEGACY_SECTION),
        Component::text("\u{a7}x").append(Component::text("Kittens!").color(NamedColor::Yellow))
    );
    assert_eq!(
        Component::from_legacy("&Epop4959"),
        Component::text("pop4959").color(NamedColor::Yellow)
    );
    assert_eq!(
        Component::from_legacy("&#ffb6c1pretty&#ff69b4&lin&#ffc0cbpink"),
        Component::text("")
            .append(Component::text("pretty").color(TextColor::Hex(0xFF_B6C1)))
            .append(
                Component::text("in")
                    .color(TextColor::Hex(0xFF_69B4))
                    .bold()
            )
            .append(Component::text("pink").color(TextColor::Hex(0xFF_C0CB)))
    );
}

#[test]
fn legacy_color_resets_formatting_like_vanilla() {
    assert_eq!(
        Component::from_legacy("&nX&cY"),
        Component::text("")
            .append(Component::text("X").underlined())
            .append(Component::text("Y").color(NamedColor::Red))
    );
    assert_eq!(
        Component::from_legacy("&lA&rB"),
        Component::text("")
            .append(Component::text("A").bold())
            .append(Component::text("B"))
    );
    assert_eq!(Component::from_legacy("end&"), Component::text("end&"));
    assert_eq!(Component::from_legacy(""), Component::text(""));
    assert_eq!(
        Component::from_legacy_format("&e{server} up", &[("server", "lobby")]),
        Component::text("lobby up").color(NamedColor::Yellow)
    );
}

#[test]
fn legacy_serialize_matches_adventure_vectors() {
    let c1 = Component::text("hi")
        .bold()
        .append(
            Component::text("foo")
                .color(NamedColor::Green)
                .decoration(Decoration::Bold, Some(false)),
        )
        .append(Component::text("bar").color(NamedColor::Blue))
        .append(Component::text("baz"));
    assert_eq!(
        c1.to_legacy(LEGACY_SECTION),
        "\u{a7}lhi\u{a7}afoo\u{a7}9\u{a7}lbar\u{a7}r\u{a7}lbaz"
    );
    let c2 = Component::text("").color(NamedColor::Yellow).append(
        Component::text("Hello ")
            .append(Component::text("world").color(NamedColor::Green))
            .append(Component::text("!")),
    );
    assert_eq!(
        c2.to_legacy(LEGACY_SECTION),
        "\u{a7}eHello \u{a7}aworld\u{a7}e!"
    );
    let c3 = Component::text("").bold().append(
        Component::text("").color(NamedColor::Yellow).append(
            Component::text("Hello ")
                .append(Component::text("world").color(NamedColor::Green))
                .append(Component::text("!")),
        ),
    );
    assert_eq!(
        c3.to_legacy(LEGACY_SECTION),
        "\u{a7}e\u{a7}lHello \u{a7}a\u{a7}lworld\u{a7}e\u{a7}l!"
    );
    let hex = Component::text("Kittens!").color(TextColor::Hex(0xFF_EFD5));
    assert_eq!(
        hex.to_legacy(LEGACY_SECTION),
        "\u{a7}x\u{a7}f\u{a7}f\u{a7}e\u{a7}f\u{a7}d\u{a7}5Kittens!"
    );
    let compound = Component::text("hi there ")
        .append(Component::text("this bit is green ").color(NamedColor::Green))
        .append(Component::text("this isn't "))
        .append(Component::text("and woa, this is again").color(NamedColor::Green));
    assert_eq!(
        compound.to_legacy('&'),
        "hi there &athis bit is green &rthis isn't &aand woa, this is again"
    );
}

#[test]
fn legacy_round_trips() {
    for input in [
        "plain",
        "\u{a7}cHello \u{a7}lWorld\u{a7}r!",
        "\u{a7}x\u{a7}a\u{a7}b\u{a7}c\u{a7}d\u{a7}e\u{a7}fHex \u{a7}rplain \u{a7}k\u{a7}mx",
        "a\u{a7}nb\u{a7}oc\u{a7}6d",
    ] {
        let parsed = Component::from_legacy_with(input, LEGACY_SECTION);
        assert_eq!(parsed.to_legacy(LEGACY_SECTION), input);
    }
    let amp = "&4Red &lBold&r &x&1&2&3&4&5&6hex";
    assert_eq!(Component::from_legacy(amp).to_legacy('&'), amp);
}

#[test]
fn plain_rendering_of_non_text_content() {
    let c = Component::text("a")
        .append(Component::translatable("k.v"))
        .append(Component::translatable("k.w").fallback("W"))
        .append(Component::keybind("key.jump"))
        .append(Component::score("p", "o"))
        .append(Component::selector("@p"));
    assert_eq!(c.to_plain(), "ak.vWkey.jump@p");
}

fn nested_json(levels: usize) -> Value {
    let mut value = json!("leaf");
    for _ in 0..levels {
        value = json!({"text": "a", "extra": [value], "hoverEvent": {"action": "show_text", "contents": "t"}});
    }
    value
}

fn with_big_stack(test: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(test)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn parser_depth_is_bounded_for_deep_values() {
    with_big_stack(parser_depth_is_bounded_for_deep_values_inner);
}

fn parser_depth_is_bounded_for_deep_values_inner() {
    let parsed = Component::from_json_value(&nested_json(1_000));
    assert!(parsed.depth() <= MAX_COMPONENT_DEPTH);
    assert_eq!(parsed.depth(), MAX_COMPONENT_DEPTH);
    let text = format!("{}\"x\"{}", "[\"a\",".repeat(100), "]".repeat(100));
    let parsed = Component::from_json(&text).unwrap();
    assert!(parsed.depth() <= MAX_COMPONENT_DEPTH);
    let too_deep_for_serde = format!("{}\"x\"{}", "[".repeat(1_000), "]".repeat(1_000));
    assert!(Component::from_json(&too_deep_for_serde).is_err());
}

#[test]
fn serializer_depth_is_bounded_for_deep_trees() {
    with_big_stack(serializer_depth_is_bounded_for_deep_trees_inner);
}

fn serializer_depth_is_bounded_for_deep_trees_inner() {
    let mut component = Component::text("leaf");
    for _ in 0..1_000 {
        component = Component::text("n")
            .hover(HoverEvent::show_text(Component::text("t")))
            .append(component);
    }
    assert_eq!(component.depth(), 1_001);
    for raw in [47, 735, 765, 774] {
        let json = component.to_json_value_for(pv(raw));
        assert!(Component::from_json_value(&json).depth() <= MAX_COMPONENT_DEPTH);
        let nbt = component.to_nbt_for(pv(raw));
        assert!(Component::from_nbt_network(&nbt).unwrap().depth() <= MAX_COMPONENT_DEPTH);
    }
    assert_eq!(component.to_plain().len(), MAX_COMPONENT_DEPTH);
}

const JSON_KEYS: &[&str] = &[
    "text",
    "translate",
    "fallback",
    "with",
    "keybind",
    "score",
    "name",
    "objective",
    "selector",
    "separator",
    "nbt",
    "interpret",
    "block",
    "entity",
    "storage",
    "source",
    "object",
    "atlas",
    "sprite",
    "player",
    "hat",
    "type",
    "color",
    "font",
    "bold",
    "italic",
    "underlined",
    "strikethrough",
    "obfuscated",
    "shadow_color",
    "insertion",
    "clickEvent",
    "click_event",
    "hoverEvent",
    "hover_event",
    "action",
    "value",
    "contents",
    "url",
    "command",
    "page",
    "id",
    "payload",
    "count",
    "components",
    "uuid",
    "extra",
    "",
];

const JSON_STRINGS: &[&str] = &[
    "show_text",
    "show_item",
    "show_entity",
    "open_url",
    "run_command",
    "suggest_command",
    "change_page",
    "copy_to_clipboard",
    "custom",
    "open_file",
    "text",
    "translatable",
    "keybind",
    "score",
    "selector",
    "nbt",
    "object",
    "atlas",
    "player",
    "entity",
    "block",
    "storage",
    "minecraft:pig",
    "https://a.b",
    "#ff00ff",
    "gold",
    "0000000a-0000-000b-0000-000c0000000d",
    "true",
    "12",
];

fn arb_json() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<f64>()
            .prop_map(|f| serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number)),
        prop::sample::select(JSON_STRINGS).prop_map(Value::from),
        "\\PC{0,8}".prop_map(Value::from),
    ];
    leaf.prop_recursive(10, 256, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            prop::collection::vec(
                (
                    prop::sample::select(JSON_KEYS).prop_map(String::from),
                    inner
                ),
                0..8
            )
            .prop_map(|entries| Value::Object(entries.into_iter().collect())),
        ]
    })
}

fn arb_ident() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "minecraft:stone",
        "minecraft:pig",
        "infrarust:ping",
        "stick",
        "custom/path.x-y_z",
    ])
    .prop_map(String::from)
}

fn arb_text() -> impl Strategy<Value = String> {
    "(?s).{0,10}"
}

fn arb_color() -> impl Strategy<Value = Option<TextColor>> {
    prop_oneof![
        Just(None),
        prop::sample::select(NamedColor::ALL.to_vec()).prop_map(|c| Some(TextColor::Named(c))),
        (0u32..=0xFF_FFFF).prop_map(|v| Some(TextColor::Hex(v))),
    ]
}

fn arb_click() -> impl Strategy<Value = ClickEvent> {
    prop_oneof![
        prop::sample::select(vec![
            "https://example.com",
            "http://a.b/c?d=e#f",
            "https://x.y/%41"
        ])
        .prop_map(|u| ClickEvent::OpenUrl(u.into())),
        "/[a-z ]{0,8}".prop_map(ClickEvent::RunCommand),
        "[a-z ]{0,8}".prop_map(ClickEvent::SuggestCommand),
        (1i32..500).prop_map(ClickEvent::ChangePage),
        arb_text().prop_map(ClickEvent::CopyToClipboard),
        (arb_ident(), proptest::option::of(arb_text()))
            .prop_map(|(id, payload)| ClickEvent::Custom { id, payload }),
    ]
}

fn arb_hover(inner: BoxedStrategy<Component>) -> impl Strategy<Value = HoverEvent> {
    prop_oneof![
        inner
            .clone()
            .prop_map(|c| HoverEvent::ShowText(Box::new(c))),
        (
            arb_ident(),
            1i32..=99,
            proptest::option::of((0i32..1000).prop_map(|d| json!({"minecraft:damage": d})))
        )
            .prop_map(|(id, count, components)| HoverEvent::ShowItem {
                id,
                count,
                components
            }),
        (arb_ident(), any::<u128>(), proptest::option::of(inner)).prop_map(
            |(entity_type, uuid, name)| HoverEvent::ShowEntity {
                entity_type,
                uuid: Uuid::from_u128(uuid),
                name: name.map(Box::new),
            }
        ),
    ]
}

fn arb_style(inner: BoxedStrategy<Component>) -> impl Strategy<Value = Style> {
    (
        arb_color(),
        proptest::option::of(arb_ident()),
        prop::array::uniform5(proptest::option::of(any::<bool>())),
        proptest::option::of(any::<u32>()),
        proptest::option::of(arb_text()),
        proptest::option::of(arb_click()),
        proptest::option::of(arb_hover(inner)),
    )
        .prop_map(
            |(color, font, decorations, shadow_color, insertion, click, hover)| {
                let mut style = Style::new();
                style.color = color;
                style.font = font;
                for (decoration, value) in Decoration::ALL.into_iter().zip(decorations) {
                    style.set_decoration(decoration, value);
                }
                style.shadow_color = shadow_color;
                style.insertion = insertion;
                style.click = click.map(Box::new);
                style.hover = hover.map(Box::new);
                style
            },
        )
}

fn arb_content(inner: BoxedStrategy<Component>) -> impl Strategy<Value = Content> {
    prop_oneof![
        arb_text().prop_map(Content::Text),
        (
            "[a-z.]{1,10}",
            proptest::option::of(arb_text()),
            prop::collection::vec(inner.clone(), 0..3)
        )
            .prop_map(|(key, fallback, with)| Content::Translatable {
                key,
                fallback,
                with
            }),
        "key\\.[a-z]{1,6}".prop_map(Content::Keybind),
        ("[a-zA-Z@*]{1,6}", "[a-z]{1,6}")
            .prop_map(|(name, objective)| Content::Score { name, objective }),
        (
            prop::sample::select(vec!["@a", "@p", "@e[type=pig]"]),
            proptest::option::of(inner.clone())
        )
            .prop_map(|(pattern, separator)| Content::Selector {
                pattern: pattern.into(),
                separator: separator.map(Box::new)
            }),
        (
            "[A-Za-z.\\[\\]0-9]{1,8}",
            proptest::option::of(any::<bool>()),
            proptest::option::of(inner),
            prop_oneof![
                Just(NbtSource::Block("~ ~ ~".into())),
                Just(NbtSource::Entity("@s".into())),
                arb_ident().prop_map(NbtSource::Storage),
            ]
        )
            .prop_map(|(path, interpret, separator, source)| Content::Nbt {
                path,
                interpret,
                separator: separator.map(Box::new),
                source
            }),
        (proptest::option::of(arb_ident()), arb_ident())
            .prop_map(|(atlas, sprite)| Content::Object(ObjectContent::Atlas { atlas, sprite })),
        (
            prop_oneof![
                Just(json!("Notch")),
                Just(json!({"name": "Notch", "id": [1, 2, 3, 4]}))
            ],
            proptest::option::of(any::<bool>())
        )
            .prop_map(|(player, hat)| Content::Object(ObjectContent::Player { player, hat })),
    ]
}

fn arb_component() -> impl Strategy<Value = Component> {
    let leaf = arb_text().prop_map(Component::text);
    leaf.prop_recursive(3, 24, 3, |inner| {
        let inner = inner.boxed();
        (
            arb_content(inner.clone()),
            arb_style(inner.clone()),
            prop::collection::vec(inner, 0..3),
        )
            .prop_map(|(content, style, children)| {
                let mut component = Component::new(content).with_style(style);
                component.children = children;
                component
            })
    })
}

fn walk_keys(value: &Value, raw: i32, inside_raw: bool) -> Result<(), String> {
    match value {
        Value::Array(items) => items.iter().try_for_each(|v| walk_keys(v, raw, inside_raw)),
        Value::Object(map) => {
            for (key, v) in map {
                if !inside_raw {
                    let forbidden = match key.as_str() {
                        "insertion" | "score" | "selector" => raw < 47,
                        "keybind" => raw < 335,
                        "nbt" => raw < 477,
                        "storage" => raw < 573,
                        "font" | "contents" => raw < 735 || (key == "contents" && raw >= 770),
                        "separator" => raw < 755,
                        "fallback" => raw < 762,
                        "shadow_color" => raw < 769,
                        "clickEvent" | "hoverEvent" => raw >= 770,
                        "action" => raw < 47 && v == "show_entity",
                        "click_event" | "hover_event" => raw < 770,
                        "sprite" | "player" | "atlas" | "hat" => raw < 773,
                        "extra" | "with" => v.as_array().is_some_and(Vec::is_empty),
                        "color" => raw < 735 && v.as_str().is_some_and(|c| c.starts_with('#')),
                        _ => false,
                    };
                    if forbidden {
                        return Err(format!("key {key} not allowed at {raw}: {value}"));
                    }
                    if key == "action" && v == "custom" && raw < 771 {
                        return Err(format!("custom click at {raw}"));
                    }
                    if key == "action" && v == "copy_to_clipboard" && raw < 573 {
                        return Err(format!("copy_to_clipboard at {raw}"));
                    }
                }
                let raw_child = inside_raw || key == "components" || key == "player";
                walk_keys(v, raw, raw_child)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn json_parser_never_panics_and_respects_depth(value in arb_json()) {
        let parsed = Component::from_json_value(&value);
        prop_assert!(parsed.depth() <= MAX_COMPONENT_DEPTH);
        let reparsed = Component::from_json(&value.to_string()).unwrap();
        prop_assert!(reparsed.depth() <= MAX_COMPONENT_DEPTH);
        for raw in BOUNDARIES {
            let emitted = parsed.to_json_value_for(pv(raw));
            prop_assert!(Component::from_json_value(&emitted).depth() <= MAX_COMPONENT_DEPTH);
            let nbt = parsed.to_nbt_for(pv(raw));
            prop_assert!(Component::from_nbt_network(&nbt).is_ok());
        }
    }

    #[test]
    fn nbt_parser_never_panics_on_arbitrary_bytes(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        if let Ok(parsed) = Component::from_nbt_network(&bytes) {
            prop_assert!(parsed.depth() <= MAX_COMPONENT_DEPTH);
        }
        let mut compound = vec![0x0A];
        compound.extend_from_slice(&bytes);
        if let Ok(parsed) = Component::from_nbt_network(&compound) {
            prop_assert!(parsed.depth() <= MAX_COMPONENT_DEPTH);
        }
        let _ = Component::from_nbt_network_prefix(&compound);
    }

    #[test]
    fn nbt_parser_never_panics_on_mutated_valid_nbt(
        component in arb_component(),
        flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..6),
    ) {
        let mut bytes = component.to_nbt_for(pv(774));
        for (index, byte) in flips {
            let at = index.index(bytes.len());
            bytes[at] = byte;
        }
        if let Ok(parsed) = Component::from_nbt_network(&bytes) {
            prop_assert!(parsed.depth() <= MAX_COMPONENT_DEPTH);
        }
    }

    #[test]
    fn current_version_round_trips_json_and_nbt(component in arb_component()) {
        let json = component.to_json_for(pv(774));
        prop_assert_eq!(&Component::from_json(&json).unwrap(), &component);
        let nbt = component.to_nbt_for(pv(774));
        prop_assert_eq!(&Component::from_nbt_network(&nbt).unwrap(), &component);
    }

    #[test]
    fn emitted_json_only_uses_fields_known_to_the_target(component in arb_component()) {
        for raw in BOUNDARIES {
            let json = component.to_json_value_for(pv(raw));
            prop_assert!(walk_keys(&json, raw, false).is_ok(), "{:?}", walk_keys(&json, raw, false));
        }
    }

    #[test]
    fn legacy_serialization_is_stable(component in arb_component()) {
        let once = component.to_legacy(LEGACY_SECTION);
        let twice = Component::from_legacy_with(&once, LEGACY_SECTION).to_legacy(LEGACY_SECTION);
        prop_assume!(!component.to_plain().contains(LEGACY_SECTION));
        prop_assert_eq!(once, twice);
    }
}
