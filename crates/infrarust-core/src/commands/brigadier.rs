use infrarust_protocol::packets::play::commands::{CCommands, CommandNode, string_parser};
use infrarust_protocol::version::ProtocolVersion;

const ASK_SERVER: Option<&str> = Some("minecraft:ask_server");
const SINGLE_WORD: i32 = 0;
const GREEDY_PHRASE: i32 = 2;

pub fn inject_proxy_commands(commands: &mut CCommands, version: ProtocolVersion) {
    let base = commands.nodes.len() as i32;
    let root = commands.root_index;

    let mut nodes: Vec<CommandNode> = Vec::new();
    let mut push = |node: CommandNode| -> i32 {
        let idx = base + nodes.len() as i32;
        nodes.push(node);
        idx
    };

    let infrarust_idx = push(CommandNode::literal("infrarust"));
    let help_idx = push(CommandNode::literal_executable("help"));
    let help_cmd_idx = push(CommandNode::argument(
        "command",
        string_parser(SINGLE_WORD, version),
        ASK_SERVER,
    ));
    let version_idx = push(CommandNode::literal_executable("version"));
    let list_idx = push(CommandNode::literal_executable("list"));
    let plugins_idx = push(CommandNode::literal_executable("plugins"));
    let reload_idx = push(CommandNode::literal_executable("reload"));
    let server_idx = push(CommandNode::literal_executable("server"));
    let server_name_idx = push(CommandNode::argument(
        "name",
        string_parser(SINGLE_WORD, version),
        ASK_SERVER,
    ));
    let find_idx = push(CommandNode::literal("find"));
    let find_player_idx = push(CommandNode::argument(
        "player",
        string_parser(SINGLE_WORD, version),
        ASK_SERVER,
    ));
    let send_idx = push(CommandNode::literal("send"));
    let send_player_idx = push(CommandNode::argument_non_executable(
        "player",
        string_parser(SINGLE_WORD, version),
        ASK_SERVER,
    ));
    let send_server_idx = push(CommandNode::argument(
        "server",
        string_parser(SINGLE_WORD, version),
        ASK_SERVER,
    ));
    let kick_idx = push(CommandNode::literal("kick"));
    let kick_player_idx = push(CommandNode::argument(
        "player",
        string_parser(SINGLE_WORD, version),
        ASK_SERVER,
    ));
    let kick_reason_idx = push(CommandNode::argument(
        "reason",
        string_parser(GREEDY_PHRASE, version),
        None,
    ));
    let broadcast_idx = push(CommandNode::literal("broadcast"));
    let broadcast_msg_idx = push(CommandNode::argument(
        "message",
        string_parser(GREEDY_PHRASE, version),
        None,
    ));
    let ir_idx = push(CommandNode::redirect("ir", infrarust_idx));

    // Wire children
    nodes[(help_idx - base) as usize].children.push(help_cmd_idx);
    nodes[(server_idx - base) as usize].children.push(server_name_idx);
    nodes[(find_idx - base) as usize].children.push(find_player_idx);
    nodes[(send_idx - base) as usize].children.push(send_player_idx);
    nodes[(send_player_idx - base) as usize].children.push(send_server_idx);
    nodes[(kick_idx - base) as usize].children.push(kick_player_idx);
    nodes[(kick_player_idx - base) as usize].children.push(kick_reason_idx);
    nodes[(broadcast_idx - base) as usize].children.push(broadcast_msg_idx);

    // infrarust children
    nodes[(infrarust_idx - base) as usize].children = vec![
        help_idx,
        version_idx,
        list_idx,
        plugins_idx,
        reload_idx,
        server_idx,
        find_idx,
        send_idx,
        kick_idx,
        broadcast_idx,
    ];

    commands.nodes.extend(nodes);

    // Add infrarust and ir as children of root
    commands.nodes[root as usize].children.push(infrarust_idx);
    commands.nodes[root as usize].children.push(ir_idx);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use infrarust_protocol::packets::play::commands::CommandNode;

    fn make_empty_tree() -> CCommands {
        CCommands {
            nodes: vec![CommandNode {
                flags: 0x00, // root
                children: vec![],
                redirect_node: None,
                name: None,
                parser: None,
                suggestions_type: None,
            }],
            root_index: 0,
        }
    }

    #[test]
    fn inject_adds_infrarust_and_ir_to_root() {
        let mut cmds = make_empty_tree();
        inject_proxy_commands(&mut cmds, ProtocolVersion::V1_21);
        let root = &cmds.nodes[0];
        let names: Vec<&str> = root
            .children
            .iter()
            .filter_map(|&i| cmds.nodes[i as usize].name.as_deref())
            .collect();
        assert!(names.contains(&"infrarust"));
        assert!(names.contains(&"ir"));
    }

    #[test]
    fn ir_redirects_to_infrarust() {
        let mut cmds = make_empty_tree();
        inject_proxy_commands(&mut cmds, ProtocolVersion::V1_21);
        let ir = cmds
            .nodes
            .iter()
            .find(|n| n.name.as_deref() == Some("ir"))
            .unwrap();
        let infrarust_idx = cmds
            .nodes
            .iter()
            .position(|n| n.name.as_deref() == Some("infrarust"))
            .unwrap();
        assert_eq!(ir.redirect_node, Some(infrarust_idx as i32));
    }

    #[test]
    fn infrarust_has_all_subcommands() {
        let mut cmds = make_empty_tree();
        inject_proxy_commands(&mut cmds, ProtocolVersion::V1_21);
        let infrarust = cmds
            .nodes
            .iter()
            .find(|n| n.name.as_deref() == Some("infrarust"))
            .unwrap();
        let child_names: Vec<&str> = infrarust
            .children
            .iter()
            .filter_map(|&i| cmds.nodes[i as usize].name.as_deref())
            .collect();
        for expected in [
            "help", "version", "list", "plugins", "reload", "server", "find", "send", "kick",
            "broadcast",
        ] {
            assert!(
                child_names.contains(&expected),
                "missing subcommand: {expected}"
            );
        }
    }

    #[test]
    fn server_arg_has_ask_server_suggestions() {
        let mut cmds = make_empty_tree();
        inject_proxy_commands(&mut cmds, ProtocolVersion::V1_21);
        let server_node = cmds
            .nodes
            .iter()
            .find(|n| n.name.as_deref() == Some("server") && n.is_executable())
            .unwrap();
        let name_arg_idx = server_node.children[0] as usize;
        let name_arg = &cmds.nodes[name_arg_idx];
        assert_eq!(
            name_arg.suggestions_type.as_deref(),
            Some("minecraft:ask_server")
        );
    }

    #[test]
    fn round_trip_after_injection() {
        use infrarust_protocol::packets::Packet;
        let mut cmds = make_empty_tree();
        inject_proxy_commands(&mut cmds, ProtocolVersion::V1_21);
        let mut buf = Vec::new();
        cmds.encode(&mut buf, ProtocolVersion::V1_21).unwrap();
        let decoded = CCommands::decode(&mut buf.as_slice(), ProtocolVersion::V1_21).unwrap();
        assert_eq!(decoded.nodes.len(), cmds.nodes.len());
        assert_eq!(decoded.root_index, cmds.root_index);
    }
}
