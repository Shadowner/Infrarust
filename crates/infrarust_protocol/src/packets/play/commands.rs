use std::io::Write;

use crate::codec::{McBufReadExt, McBufWriteExt, VarInt};
use crate::error::ProtocolResult;
use crate::version::{ConnectionState, Direction, ProtocolVersion};

use super::super::Packet;

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
    let child_count = r.read_var_int()?.0;
    if child_count < 0 {
        return Err(crate::error::ProtocolError::invalid("negative child count"));
    }
    let mut children = Vec::with_capacity((child_count as usize).min(1024));
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
            return Err(crate::error::ProtocolError::invalid(format!(
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
    mut w: &mut (impl Write + ?Sized),
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
        let properties = read_parser_properties(r, id)?;
        Ok(Parser::Indexed { id, properties })
    } else {
        let identifier = r.read_string()?;
        let properties = read_parser_properties_by_name(r, &identifier)?;
        Ok(Parser::Named {
            identifier,
            properties,
        })
    }
}

fn encode_parser(
    parser: &Parser,
    mut w: &mut (impl Write + ?Sized),
    version: ProtocolVersion,
) -> ProtocolResult<()> {
    match parser {
        Parser::Indexed { id, properties } => {
            if version.no_less_than(ProtocolVersion::V1_19) {
                w.write_var_int(&VarInt(*id))?;
            } else {
                let name = indexed_parser_to_name(*id);
                w.write_string(name)?;
            }
            w.write_all(properties)?;
        }
        Parser::Named {
            identifier,
            properties,
        } => {
            if version.no_less_than(ProtocolVersion::V1_19) {
                let id = named_parser_to_id(identifier);
                w.write_var_int(&VarInt(id))?;
            } else {
                w.write_string(identifier)?;
            }
            w.write_all(properties)?;
        }
    }
    Ok(())
}

/// Read parser-specific properties by VarInt ID (1.19+).
/// We read the exact number of bytes each parser type needs.
fn read_parser_properties(r: &mut &[u8], id: i32) -> ProtocolResult<Vec<u8>> {
    let mut buf = Vec::new();
    match id {
        0 => {} // bool — no properties
        1 | 2 => {
            // float / double — flags byte, then optional min/max
            let flags = r.read_u8()?;
            buf.push(flags);
            if flags & 0x01 != 0 {
                let bytes = r.read_byte_array_bounded(if id == 1 { 4 } else { 8 })?;
                buf.extend_from_slice(&bytes);
            }
            if flags & 0x02 != 0 {
                let bytes = r.read_byte_array_bounded(if id == 1 { 4 } else { 8 })?;
                buf.extend_from_slice(&bytes);
            }
        }
        3 | 4 => {
            // integer / long — flags byte, then optional min/max
            let flags = r.read_u8()?;
            buf.push(flags);
            if flags & 0x01 != 0 {
                let bytes = r.read_byte_array_bounded(if id == 3 { 4 } else { 8 })?;
                buf.extend_from_slice(&bytes);
            }
            if flags & 0x02 != 0 {
                let bytes = r.read_byte_array_bounded(if id == 3 { 4 } else { 8 })?;
                buf.extend_from_slice(&bytes);
            }
        }
        5 => {
            // string — VarInt mode
            let mode = r.read_var_int()?;
            mode.encode(&mut buf)?;
        }
        6 => {
            // entity — flags byte
            buf.push(r.read_u8()?);
        }
        31 => {
            // score_holder — flags byte
            buf.push(r.read_u8()?);
        }
        43 => {
            // time — i32 min
            let bytes = r.read_byte_array_bounded(4)?;
            buf.extend_from_slice(&bytes);
        }
        44..=47 => {
            // resource_or_tag, resource_or_tag_key, resource, resource_key — string identifier
            let s = r.read_string()?;
            let mut tmp = Vec::new();
            tmp.write_string(&s)?;
            buf.extend_from_slice(&tmp);
        }
        _ => {
            // All other parsers have no properties (7-30, 32-42, 48+)
        }
    }
    Ok(buf)
}

/// Read parser-specific properties by string identifier (pre-1.19).
fn read_parser_properties_by_name(r: &mut &[u8], identifier: &str) -> ProtocolResult<Vec<u8>> {
    let id = named_parser_to_id(identifier);
    read_parser_properties(r, id)
}

fn named_parser_to_id(name: &str) -> i32 {
    match name {
        "brigadier:bool" => 0,
        "brigadier:float" => 1,
        "brigadier:double" => 2,
        "brigadier:integer" => 3,
        "brigadier:long" => 4,
        "brigadier:string" => 5,
        "minecraft:entity" => 6,
        "minecraft:game_profile" => 7,
        "minecraft:block_pos" => 8,
        "minecraft:column_pos" => 9,
        "minecraft:vec3" => 10,
        "minecraft:vec2" => 11,
        "minecraft:block_state" => 12,
        "minecraft:block_predicate" => 13,
        "minecraft:item_stack" => 14,
        "minecraft:item_predicate" => 15,
        "minecraft:color" => 16,
        "minecraft:component" => 17,
        "minecraft:message" => 18,
        "minecraft:nbt_compound_tag" | "minecraft:nbt" => 19,
        "minecraft:nbt_tag" => 20,
        "minecraft:nbt_path" => 21,
        "minecraft:objective" => 22,
        "minecraft:objective_criteria" => 23,
        "minecraft:operation" => 24,
        "minecraft:particle" => 25,
        "minecraft:angle" => 26,
        "minecraft:rotation" => 27,
        "minecraft:scoreboard_slot" => 28,
        "minecraft:score_holder" => 29,
        "minecraft:swizzle" => 30,
        "minecraft:team" => 31,
        "minecraft:item_slot" => 32,
        "minecraft:resource_location" => 33,
        "minecraft:function" => 34,
        "minecraft:entity_anchor" => 35,
        "minecraft:int_range" => 36,
        "minecraft:float_range" => 37,
        "minecraft:dimension" => 38,
        "minecraft:gamemode" => 39,
        "minecraft:time" => 40,
        "minecraft:resource_or_tag" => 41,
        "minecraft:resource_or_tag_key" => 42,
        "minecraft:resource" => 43,
        "minecraft:resource_key" => 44,
        "minecraft:template_mirror" => 45,
        "minecraft:template_rotation" => 46,
        "minecraft:heightmap" => 47,
        _ => -1,
    }
}

fn indexed_parser_to_name(id: i32) -> &'static str {
    match id {
        0 => "brigadier:bool",
        1 => "brigadier:float",
        2 => "brigadier:double",
        3 => "brigadier:integer",
        4 => "brigadier:long",
        5 => "brigadier:string",
        6 => "minecraft:entity",
        7 => "minecraft:game_profile",
        8 => "minecraft:block_pos",
        9 => "minecraft:column_pos",
        10 => "minecraft:vec3",
        11 => "minecraft:vec2",
        12 => "minecraft:block_state",
        13 => "minecraft:block_predicate",
        14 => "minecraft:item_stack",
        15 => "minecraft:item_predicate",
        16 => "minecraft:color",
        17 => "minecraft:component",
        18 => "minecraft:message",
        19 => "minecraft:nbt_compound_tag",
        20 => "minecraft:nbt_tag",
        21 => "minecraft:nbt_path",
        22 => "minecraft:objective",
        23 => "minecraft:objective_criteria",
        24 => "minecraft:operation",
        25 => "minecraft:particle",
        26 => "minecraft:angle",
        27 => "minecraft:rotation",
        28 => "minecraft:scoreboard_slot",
        29 => "minecraft:score_holder",
        30 => "minecraft:swizzle",
        31 => "minecraft:team",
        32 => "minecraft:item_slot",
        33 => "minecraft:resource_location",
        34 => "minecraft:function",
        35 => "minecraft:entity_anchor",
        36 => "minecraft:int_range",
        37 => "minecraft:float_range",
        38 => "minecraft:dimension",
        39 => "minecraft:gamemode",
        40 => "minecraft:time",
        41 => "minecraft:resource_or_tag",
        42 => "minecraft:resource_or_tag_key",
        43 => "minecraft:resource",
        44 => "minecraft:resource_key",
        45 => "minecraft:template_mirror",
        46 => "minecraft:template_rotation",
        47 => "minecraft:heightmap",
        _ => "brigadier:string",
    }
}

impl Packet for CCommands {
    const NAME: &'static str = "CCommands";

    fn state() -> ConnectionState {
        ConnectionState::Play
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let count = r.read_var_int()?.0;
        if count < 0 {
            return Err(crate::error::ProtocolError::invalid("negative node count"));
        }
        let mut nodes = Vec::with_capacity((count as usize).min(1024));
        for _ in 0..count {
            nodes.push(decode_node(r, version)?);
        }
        let root_index = r.read_var_int()?.0;
        Ok(Self { nodes, root_index })
    }

    fn encode(
        &self,
        mut w: &mut (impl Write + ?Sized),
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

/// Creates a `brigadier:string` parser for the given mode.
/// Mode: 0 = SINGLE_WORD, 1 = QUOTABLE_PHRASE, 2 = GREEDY_PHRASE.
pub fn string_parser(mode: i32, version: ProtocolVersion) -> Parser {
    let mut props = Vec::new();
    VarInt(mode).encode(&mut props).expect("VarInt encode");
    if version.no_less_than(ProtocolVersion::V1_19) {
        Parser::Indexed {
            id: 5,
            properties: props,
        }
    } else {
        Parser::Named {
            identifier: "brigadier:string".to_string(),
            properties: props,
        }
    }
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
        let parser = string_parser(0, ProtocolVersion::V1_21);
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
        let parser = string_parser(2, ProtocolVersion::V1_16);
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
