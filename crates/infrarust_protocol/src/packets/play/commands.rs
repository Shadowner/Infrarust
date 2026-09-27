use std::io::Write;

use crate::codec::{McBufReadExt, McBufWriteExt, VarInt};
use crate::error::{ProtocolError, ProtocolResult};
use crate::version::{ConnectionState, Direction, ProtocolVersion};

use super::super::{Packet, PacketMapping};
use super::command_parsers::parser_names;

const NODE_TYPE_MASK: u8 = 0x03;
const NODE_TYPE_ROOT: u8 = 0x00;
const NODE_TYPE_LITERAL: u8 = 0x01;
const NODE_TYPE_ARGUMENT: u8 = 0x02;
const FLAG_EXECUTABLE: u8 = 0x04;
const FLAG_REDIRECT: u8 = 0x08;
const FLAG_SUGGESTIONS: u8 = 0x10;

#[derive(Debug, Clone)]
pub struct CCommands {
    pub nodes: Vec<CommandNode>,
    pub root_index: i32,
}

#[derive(Debug, Clone)]
pub struct CommandNode {
    pub flags: u8,
    pub children: Vec<i32>,
    pub redirect_node: Option<i32>,
    pub name: Option<String>,
    pub parser: Option<Parser>,
    pub suggestions_type: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Parser {
    Named {
        identifier: String,
        properties: Vec<u8>,
    },
    Indexed {
        id: i32,
        properties: Vec<u8>,
    },
}

impl CommandNode {
    pub fn node_type(&self) -> u8 {
        self.flags & NODE_TYPE_MASK
    }

    pub fn is_executable(&self) -> bool {
        self.flags & FLAG_EXECUTABLE != 0
    }

    pub fn literal(name: &str) -> Self {
        Self {
            flags: NODE_TYPE_LITERAL,
            children: vec![],
            redirect_node: None,
            name: Some(name.to_string()),
            parser: None,
            suggestions_type: None,
        }
    }

    pub fn literal_executable(name: &str) -> Self {
        Self {
            flags: NODE_TYPE_LITERAL | FLAG_EXECUTABLE,
            children: vec![],
            redirect_node: None,
            name: Some(name.to_string()),
            parser: None,
            suggestions_type: None,
        }
    }

    pub fn redirect(name: &str, target: i32) -> Self {
        Self {
            flags: NODE_TYPE_LITERAL | FLAG_REDIRECT,
            children: vec![],
            redirect_node: Some(target),
            name: Some(name.to_string()),
            parser: None,
            suggestions_type: None,
        }
    }

    pub fn argument(name: &str, parser: Parser, suggestions: Option<&str>) -> Self {
        let mut flags = NODE_TYPE_ARGUMENT | FLAG_EXECUTABLE;
        if suggestions.is_some() {
            flags |= FLAG_SUGGESTIONS;
        }
        Self {
            flags,
            children: vec![],
            redirect_node: None,
            name: Some(name.to_string()),
            parser: Some(parser),
            suggestions_type: suggestions.map(String::from),
        }
    }

    pub fn argument_non_executable(name: &str, parser: Parser, suggestions: Option<&str>) -> Self {
        let mut flags = NODE_TYPE_ARGUMENT;
        if suggestions.is_some() {
            flags |= FLAG_SUGGESTIONS;
        }
        Self {
            flags,
            children: vec![],
            redirect_node: None,
            name: Some(name.to_string()),
            parser: Some(parser),
            suggestions_type: suggestions.map(String::from),
        }
    }
}

fn decode_node(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<CommandNode> {
    let flags = r.read_u8()?;
    let child_count = r.read_count("child count")?;
    let mut children = Vec::with_capacity(child_count.min(1024));
    for _ in 0..child_count {
        children.push(r.read_var_int()?.0);
    }

    let redirect_node = if flags & FLAG_REDIRECT != 0 {
        Some(r.read_var_int()?.0)
    } else {
        None
    };

    let node_type = flags & NODE_TYPE_MASK;

    let (name, parser, suggestions_type) = match node_type {
        NODE_TYPE_ROOT => (None, None, None),
        NODE_TYPE_LITERAL => {
            let name = r.read_string()?;
            (Some(name), None, None)
        }
        NODE_TYPE_ARGUMENT => {
            let name = r.read_string()?;
            let parser = decode_parser(r, version)?;
            let suggestions = if flags & FLAG_SUGGESTIONS != 0 {
                Some(r.read_string()?)
            } else {
                None
            };
            (Some(name), Some(parser), suggestions)
        }
        _ => {
            return Err(ProtocolError::invalid(format!(
                "unknown command node type: {node_type}"
            )));
        }
    };

    Ok(CommandNode {
        flags,
        children,
        redirect_node,
        name,
        parser,
        suggestions_type,
    })
}

fn encode_node(
    node: &CommandNode,
    w: &mut (impl Write + ?Sized),
    version: ProtocolVersion,
) -> ProtocolResult<()> {
    w.write_u8(node.flags)?;
    w.write_var_int(&VarInt(node.children.len() as i32))?;
    for &child in &node.children {
        w.write_var_int(&VarInt(child))?;
    }
    if let Some(redirect) = node.redirect_node {
        w.write_var_int(&VarInt(redirect))?;
    }

    let node_type = node.flags & NODE_TYPE_MASK;
    match node_type {
        NODE_TYPE_ROOT => {}
        NODE_TYPE_LITERAL => {
            if let Some(ref name) = node.name {
                w.write_string(name)?;
            }
        }
        NODE_TYPE_ARGUMENT => {
            if let Some(ref name) = node.name {
                w.write_string(name)?;
            }
            if let Some(ref parser) = node.parser {
                encode_parser(parser, w, version)?;
            }
            if let Some(ref suggestions) = node.suggestions_type {
                w.write_string(suggestions)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn decode_parser(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Parser> {
    if version.no_less_than(ProtocolVersion::V1_19) {
        let id = r.read_var_int()?.0;
        let name = indexed_parser_to_name(id, version)?;
        let properties = read_parser_properties(r, name, version)?;
        Ok(Parser::Indexed { id, properties })
    } else {
        let identifier = r.read_string()?;
        let properties = read_parser_properties(r, &identifier, version)?;
        Ok(Parser::Named {
            identifier,
            properties,
        })
    }
}

fn encode_parser(
    parser: &Parser,
    w: &mut (impl Write + ?Sized),
    version: ProtocolVersion,
) -> ProtocolResult<()> {
    match parser {
        Parser::Indexed { id, properties } => {
            if version.less_than(ProtocolVersion::V1_19) {
                return Err(ProtocolError::invalid(format!(
                    "indexed command parser {id} cannot be encoded for {version}"
                )));
            }
            w.write_var_int(&VarInt(*id))?;
            w.write_all(properties)?;
        }
        Parser::Named {
            identifier,
            properties,
        } => {
            if version.no_less_than(ProtocolVersion::V1_19) {
                let id = named_parser_to_id(identifier, version)?;
                w.write_var_int(&VarInt(id))?;
            } else {
                w.write_string(identifier)?;
            }
            w.write_all(properties)?;
        }
    }
    Ok(())
}

fn read_parser_properties(
    r: &mut &[u8],
    name: &str,
    version: ProtocolVersion,
) -> ProtocolResult<Vec<u8>> {
    let mut buf = Vec::new();
    match name {
        "brigadier:float" | "brigadier:integer" => read_bounds(r, 4, &mut buf)?,
        "brigadier:double" | "brigadier:long" => read_bounds(r, 8, &mut buf)?,
        "brigadier:string" => r.read_var_int()?.encode(&mut buf)?,
        "minecraft:entity" | "minecraft:score_holder" => buf.push(r.read_u8()?),
        "minecraft:time" if version.no_less_than(ProtocolVersion::V1_19_4) => {
            buf.extend_from_slice(&r.read_byte_array_bounded(4)?);
        }
        "minecraft:resource_or_tag"
        | "minecraft:resource_or_tag_key"
        | "minecraft:resource"
        | "minecraft:resource_key"
        | "minecraft:resource_selector" => buf.write_string(&r.read_string()?)?,
        _ => {}
    }
    Ok(buf)
}

fn read_bounds(r: &mut &[u8], width: usize, buf: &mut Vec<u8>) -> ProtocolResult<()> {
    let flags = r.read_u8()?;
    buf.push(flags);
    for bound in [0x01, 0x02] {
        if flags & bound != 0 {
            buf.extend_from_slice(&r.read_byte_array_bounded(width)?);
        }
    }
    Ok(())
}

fn named_parser_to_id(name: &str, version: ProtocolVersion) -> ProtocolResult<i32> {
    let name = if name == "minecraft:nbt" {
        "minecraft:nbt_compound_tag"
    } else {
        name
    };
    parser_names(version)
        .iter()
        .position(|&parser| parser == name)
        .map(|index| index as i32)
        .ok_or_else(|| {
            ProtocolError::invalid(format!("command parser {name} does not exist in {version}"))
        })
}

fn indexed_parser_to_name(id: i32, version: ProtocolVersion) -> ProtocolResult<&'static str> {
    usize::try_from(id)
        .ok()
        .and_then(|index| parser_names(version).get(index).copied())
        .ok_or_else(|| {
            ProtocolError::invalid(format!("unknown command parser id {id} in {version}"))
        })
}

impl Packet for CCommands {
    const NAME: &'static str = "CCommands";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Clientbound;
    const IDS: &'static [PacketMapping] = ids![
        V1_13   => 0x11,
        V1_15   => 0x12,
        V1_16   => 0x11,
        V1_16_2 => 0x10,
        V1_17   => 0x12,
        V1_19   => 0x0F,
        V1_19_3 => 0x0E,
        V1_19_4 => 0x10,
        V1_20_2 => 0x11,
        V1_21_5 => 0x10,
    ];

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let count = r.read_count("node count")?;
        let mut nodes = Vec::with_capacity(count.min(1024));
        for _ in 0..count {
            nodes.push(decode_node(r, version)?);
        }
        let root_index = r.read_var_int()?.0;
        Ok(Self { nodes, root_index })
    }

    fn encode(
        &self,
        w: &mut (impl Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_var_int(&VarInt(self.nodes.len() as i32))?;
        for node in &self.nodes {
            encode_node(node, w, version)?;
        }
        w.write_var_int(&VarInt(self.root_index))?;
        Ok(())
    }
}

pub fn string_parser(mode: i32, version: ProtocolVersion) -> ProtocolResult<Parser> {
    let mut props = Vec::new();
    VarInt(mode).encode(&mut props)?;
    Ok(if version.no_less_than(ProtocolVersion::V1_19) {
        Parser::Indexed {
            id: 5,
            properties: props,
        }
    } else {
        Parser::Named {
            identifier: "brigadier:string".to_string(),
            properties: props,
        }
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn round_trip(pkt: &CCommands, version: ProtocolVersion) -> CCommands {
        let mut buf = Vec::new();
        pkt.encode(&mut buf, version).unwrap();
        CCommands::decode(&mut buf.as_slice(), version).unwrap()
    }

    #[test]
    fn parser_table_lookups_are_inverses() {
        for &version in ProtocolVersion::SUPPORTED
            .iter()
            .filter(|v| v.no_less_than(ProtocolVersion::V1_19))
        {
            for (index, name) in parser_names(version).iter().enumerate() {
                let id = index as i32;
                assert_eq!(
                    named_parser_to_id(name, version).unwrap(),
                    id,
                    "{name} at {version}"
                );
                assert_eq!(
                    indexed_parser_to_name(id, version).unwrap(),
                    *name,
                    "{id} at {version}"
                );
            }
            assert_eq!(
                named_parser_to_id("minecraft:nbt", version).unwrap(),
                named_parser_to_id("minecraft:nbt_compound_tag", version).unwrap()
            );
            assert!(named_parser_to_id("nope:not_a_parser", version).is_err());
            assert!(indexed_parser_to_name(-1, version).is_err());
            assert!(indexed_parser_to_name(parser_names(version).len() as i32, version).is_err());
        }
    }

    #[test]
    fn parser_ids_match_the_vanilla_registry() {
        let cases = [
            (ProtocolVersion::V1_19, "minecraft:score_holder", 29),
            (ProtocolVersion::V1_19, "minecraft:time", 42),
            (ProtocolVersion::V1_19, "minecraft:resource", 44),
            (ProtocolVersion::V1_19_3, "minecraft:time", 40),
            (ProtocolVersion::V1_19_4, "minecraft:heightmap", 47),
            (ProtocolVersion::V1_20_3, "minecraft:score_holder", 30),
            (ProtocolVersion::V1_20_3, "minecraft:time", 41),
            (ProtocolVersion::V1_20_5, "minecraft:resource_key", 46),
            (ProtocolVersion::V1_21, "minecraft:time", 42),
            (ProtocolVersion::V1_21_5, "minecraft:resource_selector", 47),
            (ProtocolVersion::V1_21_6, "minecraft:score_holder", 31),
            (ProtocolVersion::V1_21_6, "minecraft:dialog", 55),
            (ProtocolVersion::V26_2, "minecraft:team_color", 16),
            (
                ProtocolVersion::V26_3,
                "minecraft:context_float_provider",
                55,
            ),
            (ProtocolVersion::V26_3, "minecraft:dialog", 58),
            (ProtocolVersion::V26_3, "minecraft:uuid", 61),
        ];
        for (version, name, id) in cases {
            assert_eq!(
                named_parser_to_id(name, version).unwrap(),
                id,
                "{name} at {version}"
            );
        }
    }

    #[test]
    fn parser_properties_follow_the_client_version() {
        let cases: [(ProtocolVersion, &str, &[u8]); 6] = [
            (ProtocolVersion::V1_20_3, "minecraft:score_holder", &[0x01]),
            (ProtocolVersion::V1_21, "minecraft:time", &[0, 0, 0, 20]),
            (ProtocolVersion::V1_21, "minecraft:template_mirror", &[]),
            (
                ProtocolVersion::V1_21_5,
                "minecraft:resource_selector",
                &[5, b'i', b't', b'e', b'm', b's'],
            ),
            (ProtocolVersion::V26_3, "minecraft:dialog", &[]),
            (ProtocolVersion::V26_3, "minecraft:uuid", &[]),
        ];
        for (version, name, properties) in cases {
            let id = named_parser_to_id(name, version).unwrap();
            let pkt = CCommands {
                nodes: vec![
                    CommandNode {
                        flags: NODE_TYPE_ROOT,
                        children: vec![1],
                        redirect_node: None,
                        name: None,
                        parser: None,
                        suggestions_type: None,
                    },
                    CommandNode::argument(
                        "arg",
                        Parser::Indexed {
                            id,
                            properties: properties.to_vec(),
                        },
                        None,
                    ),
                ],
                root_index: 0,
            };
            let decoded = round_trip(&pkt, version);
            assert!(
                matches!(
                    decoded.nodes[1].parser,
                    Some(Parser::Indexed { id: got, properties: ref got_props })
                        if got == id && got_props.as_slice() == properties
                ),
                "{name} at {version}"
            );
            assert_eq!(decoded.root_index, 0, "{name} at {version}");
        }
    }

    #[test]
    fn indexed_parser_cannot_be_encoded_before_1_19() {
        let pkt = CCommands {
            nodes: vec![CommandNode::argument(
                "arg",
                Parser::Indexed {
                    id: 5,
                    properties: vec![0],
                },
                None,
            )],
            root_index: 0,
        };
        let mut buf = Vec::new();
        assert!(pkt.encode(&mut buf, ProtocolVersion::V1_16).is_err());
    }

    #[test]
    fn empty_tree_round_trip() {
        let root = CommandNode {
            flags: NODE_TYPE_ROOT,
            children: vec![],
            redirect_node: None,
            name: None,
            parser: None,
            suggestions_type: None,
        };
        let pkt = CCommands {
            nodes: vec![root],
            root_index: 0,
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_21);
        assert_eq!(decoded.nodes.len(), 1);
        assert_eq!(decoded.root_index, 0);
        assert_eq!(decoded.nodes[0].node_type(), NODE_TYPE_ROOT);
    }

    #[test]
    fn literal_node_round_trip() {
        let pkt = CCommands {
            nodes: vec![
                CommandNode {
                    flags: NODE_TYPE_ROOT,
                    children: vec![1],
                    redirect_node: None,
                    name: None,
                    parser: None,
                    suggestions_type: None,
                },
                CommandNode::literal_executable("test"),
            ],
            root_index: 0,
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_21);
        assert_eq!(decoded.nodes.len(), 2);
        assert_eq!(decoded.nodes[1].name.as_deref(), Some("test"));
        assert!(decoded.nodes[1].is_executable());
    }

    #[test]
    fn argument_node_with_string_parser_1_19_plus() {
        let parser = string_parser(0, ProtocolVersion::V1_21).unwrap();
        let pkt = CCommands {
            nodes: vec![
                CommandNode {
                    flags: NODE_TYPE_ROOT,
                    children: vec![1],
                    redirect_node: None,
                    name: None,
                    parser: None,
                    suggestions_type: None,
                },
                CommandNode::argument("name", parser, Some("minecraft:ask_server")),
            ],
            root_index: 0,
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_21);
        assert_eq!(decoded.nodes[1].name.as_deref(), Some("name"));
        assert!(matches!(
            decoded.nodes[1].parser,
            Some(Parser::Indexed { id: 5, .. })
        ));
        assert_eq!(
            decoded.nodes[1].suggestions_type.as_deref(),
            Some("minecraft:ask_server")
        );
    }

    #[test]
    fn argument_node_with_string_parser_pre_1_19() {
        let parser = string_parser(2, ProtocolVersion::V1_16).unwrap();
        let pkt = CCommands {
            nodes: vec![
                CommandNode {
                    flags: NODE_TYPE_ROOT,
                    children: vec![1],
                    redirect_node: None,
                    name: None,
                    parser: None,
                    suggestions_type: None,
                },
                CommandNode::argument("msg", parser, None),
            ],
            root_index: 0,
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_16);
        assert!(matches!(
            decoded.nodes[1].parser,
            Some(Parser::Named { ref identifier, .. }) if identifier == "brigadier:string"
        ));
    }

    #[test]
    fn unknown_node_type_errors() {
        let mut buf = Vec::new();
        buf.write_u8(0x03).unwrap();
        buf.write_var_int(&VarInt(0)).unwrap();
        let err = decode_node(&mut buf.as_slice(), ProtocolVersion::V1_21).unwrap_err();
        assert!(matches!(err, ProtocolError::Invalid { .. }));
    }

    #[test]
    fn negative_child_count_rejected() {
        let mut buf = Vec::new();
        buf.write_u8(NODE_TYPE_LITERAL).unwrap();
        buf.write_var_int(&VarInt(-1)).unwrap();
        let err = decode_node(&mut buf.as_slice(), ProtocolVersion::V1_21).unwrap_err();
        assert!(matches!(err, ProtocolError::Invalid { .. }));
    }

    #[test]
    fn hostile_child_count_errors_without_allocating() {
        let mut buf = Vec::new();
        buf.write_u8(NODE_TYPE_LITERAL).unwrap();
        buf.write_var_int(&VarInt(i32::MAX)).unwrap();
        assert!(decode_node(&mut buf.as_slice(), ProtocolVersion::V1_21).is_err());
    }

    #[test]
    fn negative_node_count_rejected() {
        let mut buf = Vec::new();
        buf.write_var_int(&VarInt(-5)).unwrap();
        let err = CCommands::decode(&mut buf.as_slice(), ProtocolVersion::V1_21).unwrap_err();
        assert!(matches!(err, ProtocolError::Invalid { .. }));
    }

    #[test]
    fn hostile_node_count_errors_without_allocating() {
        let mut buf = Vec::new();
        buf.write_var_int(&VarInt(i32::MAX)).unwrap();
        assert!(CCommands::decode(&mut buf.as_slice(), ProtocolVersion::V1_21).is_err());
    }

    #[test]
    fn truncated_parser_properties_errors() {
        let mut buf = Vec::new();
        buf.write_u8(NODE_TYPE_ARGUMENT).unwrap();
        buf.write_var_int(&VarInt(0)).unwrap();
        buf.write_string("arg").unwrap();
        buf.write_var_int(&VarInt(1)).unwrap();
        buf.write_u8(0x03).unwrap();
        assert!(decode_node(&mut buf.as_slice(), ProtocolVersion::V1_21).is_err());
    }

    #[test]
    fn truncated_node_name_errors() {
        let mut buf = Vec::new();
        buf.write_u8(NODE_TYPE_LITERAL).unwrap();
        buf.write_var_int(&VarInt(0)).unwrap();
        buf.write_var_int(&VarInt(10)).unwrap();
        buf.extend_from_slice(b"ab");
        assert!(decode_node(&mut buf.as_slice(), ProtocolVersion::V1_21).is_err());
    }

    #[test]
    fn redirect_node_round_trip() {
        let pkt = CCommands {
            nodes: vec![
                CommandNode {
                    flags: NODE_TYPE_ROOT,
                    children: vec![1, 2],
                    redirect_node: None,
                    name: None,
                    parser: None,
                    suggestions_type: None,
                },
                CommandNode::literal_executable("original"),
                CommandNode::redirect("alias", 1),
            ],
            root_index: 0,
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_21);
        assert_eq!(decoded.nodes[2].redirect_node, Some(1));
        assert_eq!(decoded.nodes[2].name.as_deref(), Some("alias"));
    }
}
