use std::collections::HashSet;

use infrarust_protocol::packets::play::commands::{CCommands, CommandNode, string_parser};
use infrarust_protocol::version::ProtocolVersion;

use crate::services::command_manager::ProxyTree;

const ASK_SERVER: Option<&str> = Some("minecraft:ask_server");
const SINGLE_WORD: i32 = 0;
const GREEDY_PHRASE: i32 = 2;

fn push_node(nodes: &mut Vec<CommandNode>, base: i32, node: CommandNode) -> i32 {
    let idx = base + nodes.len() as i32;
    nodes.push(node);
    idx
}

const LITERAL: u8 = 0x01;

fn drop_shadowed_roots(commands: &mut CCommands, shadowed: &HashSet<String>) {
    let CCommands { nodes, root_index } = commands;
    let Some(root) = usize::try_from(*root_index)
        .ok()
        .filter(|&i| i < nodes.len())
    else {
        return;
    };
    let children = std::mem::take(&mut nodes[root].children);
    nodes[root].children = children
        .into_iter()
        .filter(|&child| {
            let node = usize::try_from(child).ok().and_then(|i| nodes.get(i));
            !node.is_some_and(|node| {
                node.node_type() == LITERAL
                    && node
                        .name
                        .as_deref()
                        .is_some_and(|name| shadowed.contains(&name.to_lowercase()))
            })
        })
        .collect();
}

pub fn inject_proxy_commands(
    commands: &mut CCommands,
    version: ProtocolVersion,
    tree: &ProxyTree,
    visible_subcommands: Option<&HashSet<String>>,
) {
    drop_shadowed_roots(commands, &tree.shadowed);
    let base = commands.nodes.len() as i32;
    let root = commands.root_index;

    let is_visible = |name: &str| -> bool {
        match &visible_subcommands {
            None => true,
            Some(set) => set.contains(name),
        }
    };

    let has_any_visible = match &visible_subcommands {
        None => true,
        Some(set) => !set.is_empty(),
    };

    if has_any_visible {
        let mut nodes: Vec<CommandNode> = Vec::new();
        let infrarust_idx = push_node(&mut nodes, base, CommandNode::literal("infrarust"));
        let mut ir_children = Vec::new();

        if is_visible("help") {
            let help_idx = push_node(&mut nodes, base, CommandNode::literal_executable("help"));
            let help_cmd_idx = push_node(
                &mut nodes,
                base,
                CommandNode::argument("command", string_parser(SINGLE_WORD, version), ASK_SERVER),
            );
            nodes[(help_idx - base) as usize]
                .children
                .push(help_cmd_idx);
            ir_children.push(help_idx);
        }

        if is_visible("version") {
            ir_children.push(push_node(
                &mut nodes,
                base,
                CommandNode::literal_executable("version"),
            ));
        }

        if is_visible("list") {
            ir_children.push(push_node(
                &mut nodes,
                base,
                CommandNode::literal_executable("list"),
            ));
        }

        if is_visible("plugins") {
            ir_children.push(push_node(
                &mut nodes,
                base,
                CommandNode::literal_executable("plugins"),
            ));
        }

        if is_visible("reload") {
            ir_children.push(push_node(
                &mut nodes,
                base,
                CommandNode::literal_executable("reload"),
            ));
        }

        if is_visible("server") {
            let server_idx = push_node(&mut nodes, base, CommandNode::literal_executable("server"));
            let server_name_idx = push_node(
                &mut nodes,
                base,
                CommandNode::argument("name", string_parser(SINGLE_WORD, version), ASK_SERVER),
            );
            nodes[(server_idx - base) as usize]
                .children
                .push(server_name_idx);
            ir_children.push(server_idx);
        }

        if is_visible("find") {
            let find_idx = push_node(&mut nodes, base, CommandNode::literal("find"));
            let find_player_idx = push_node(
                &mut nodes,
                base,
                CommandNode::argument("player", string_parser(SINGLE_WORD, version), ASK_SERVER),
            );
            nodes[(find_idx - base) as usize]
                .children
                .push(find_player_idx);
            ir_children.push(find_idx);
        }

        if is_visible("send") {
            let send_idx = push_node(&mut nodes, base, CommandNode::literal("send"));
            let send_player_idx = push_node(
                &mut nodes,
                base,
                CommandNode::argument_non_executable(
                    "player",
                    string_parser(SINGLE_WORD, version),
                    ASK_SERVER,
                ),
            );
            let send_server_idx = push_node(
                &mut nodes,
                base,
                CommandNode::argument("server", string_parser(SINGLE_WORD, version), ASK_SERVER),
            );
            nodes[(send_player_idx - base) as usize]
                .children
                .push(send_server_idx);
            nodes[(send_idx - base) as usize]
                .children
                .push(send_player_idx);
            ir_children.push(send_idx);
        }

        if is_visible("kick") {
            let kick_idx = push_node(&mut nodes, base, CommandNode::literal("kick"));
            let kick_player_idx = push_node(
                &mut nodes,
                base,
                CommandNode::argument("player", string_parser(SINGLE_WORD, version), ASK_SERVER),
            );
            let kick_reason_idx = push_node(
                &mut nodes,
                base,
                CommandNode::argument("reason", string_parser(GREEDY_PHRASE, version), None),
            );
            nodes[(kick_player_idx - base) as usize]
                .children
                .push(kick_reason_idx);
            nodes[(kick_idx - base) as usize]
                .children
                .push(kick_player_idx);
            ir_children.push(kick_idx);
        }

        if is_visible("broadcast") {
            let broadcast_idx = push_node(&mut nodes, base, CommandNode::literal("broadcast"));
            let broadcast_msg_idx = push_node(
                &mut nodes,
                base,
                CommandNode::argument("message", string_parser(GREEDY_PHRASE, version), None),
            );
            nodes[(broadcast_idx - base) as usize]
                .children
                .push(broadcast_msg_idx);
            ir_children.push(broadcast_idx);
        }

        if is_visible("plugin") {
            let plugin_lit_idx = push_node(&mut nodes, base, CommandNode::literal("plugin"));
            let plugin_id_idx = push_node(
                &mut nodes,
                base,
                CommandNode::argument_non_executable(
                    "plugin_id",
                    string_parser(SINGLE_WORD, version),
                    ASK_SERVER,
                ),
            );
            let plugin_cmd_idx = push_node(
                &mut nodes,
                base,
                CommandNode::argument("command", string_parser(GREEDY_PHRASE, version), ASK_SERVER),
            );
            nodes[(plugin_id_idx - base) as usize]
                .children
                .push(plugin_cmd_idx);
            nodes[(plugin_lit_idx - base) as usize]
                .children
                .push(plugin_id_idx);
            ir_children.push(plugin_lit_idx);
        }

        nodes[(infrarust_idx - base) as usize].children = ir_children;

        let ir_idx = push_node(&mut nodes, base, CommandNode::redirect("ir", infrarust_idx));

        commands.nodes.extend(nodes);
        commands.nodes[root as usize].children.push(infrarust_idx);
        commands.nodes[root as usize].children.push(ir_idx);
    }

    for labels in &tree.commands {
        let args_idx = commands.nodes.len() as i32;
        commands.nodes.push(CommandNode::argument(
            "args",
            string_parser(GREEDY_PHRASE, version),
            ASK_SERVER,
        ));
        for label in labels {
            let cmd_idx = commands.nodes.len() as i32;
            let mut node = CommandNode::literal_executable(label);
            node.children.push(args_idx);
            commands.nodes.push(node);
            commands.nodes[root as usize].children.push(cmd_idx);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use infrarust_protocol::packets::play::commands::CommandNode;

    fn tree(commands: &[&[&str]]) -> ProxyTree {
        let commands: Vec<Vec<String>> = commands
            .iter()
            .map(|labels| labels.iter().map(ToString::to_string).collect())
            .collect();
        let mut shadowed: HashSet<String> = commands.iter().flatten().cloned().collect();
        shadowed.extend(["infrarust".to_string(), "ir".to_string()]);
        ProxyTree { commands, shadowed }
    }

    fn root_names(cmds: &CCommands) -> Vec<&str> {
        cmds.nodes[cmds.root_index as usize]
            .children
            .iter()
            .filter_map(|&i| cmds.nodes[i as usize].name.as_deref())
            .collect()
    }

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
        inject_proxy_commands(
            &mut cmds,
            ProtocolVersion::V1_21,
            &ProxyTree::default(),
            None,
        );
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
        inject_proxy_commands(
            &mut cmds,
            ProtocolVersion::V1_21,
            &ProxyTree::default(),
            None,
        );
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
        inject_proxy_commands(
            &mut cmds,
            ProtocolVersion::V1_21,
            &ProxyTree::default(),
            None,
        );
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
            "help",
            "version",
            "list",
            "plugins",
            "reload",
            "server",
            "find",
            "send",
            "kick",
            "broadcast",
            "plugin",
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
        inject_proxy_commands(
            &mut cmds,
            ProtocolVersion::V1_21,
            &ProxyTree::default(),
            None,
        );
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
    fn plugin_commands_injected_at_root() {
        let mut cmds = make_empty_tree();
        let plugin_cmds = tree(&[&["hello", "hello-plugin:hello"], &["forcelogin"]]);
        inject_proxy_commands(&mut cmds, ProtocolVersion::V1_21, &plugin_cmds, None);
        let root = &cmds.nodes[0];
        let names: Vec<&str> = root
            .children
            .iter()
            .filter_map(|&i| cmds.nodes[i as usize].name.as_deref())
            .collect();
        assert!(names.contains(&"hello"), "missing plugin command: hello");
        assert!(
            names.contains(&"forcelogin"),
            "missing plugin command: forcelogin"
        );

        let hello_idx = root
            .children
            .iter()
            .find(|&&i| cmds.nodes[i as usize].name.as_deref() == Some("hello"))
            .unwrap();
        let hello_node = &cmds.nodes[*hello_idx as usize];
        assert!(
            !hello_node.children.is_empty(),
            "hello should have args child"
        );
    }

    #[test]
    fn plugin_node_subtree_exists() {
        let mut cmds = make_empty_tree();
        inject_proxy_commands(
            &mut cmds,
            ProtocolVersion::V1_21,
            &ProxyTree::default(),
            None,
        );
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
        assert!(
            child_names.contains(&"plugin"),
            "infrarust should have 'plugin' subcommand"
        );

        let plugin_idx = infrarust
            .children
            .iter()
            .find(|&&i| cmds.nodes[i as usize].name.as_deref() == Some("plugin"))
            .unwrap();
        let plugin_node = &cmds.nodes[*plugin_idx as usize];
        assert_eq!(plugin_node.children.len(), 1);
        let plugin_id_node = &cmds.nodes[plugin_node.children[0] as usize];
        assert_eq!(plugin_id_node.name.as_deref(), Some("plugin_id"));
        assert_eq!(plugin_id_node.children.len(), 1);
        let cmd_node = &cmds.nodes[plugin_id_node.children[0] as usize];
        assert_eq!(cmd_node.name.as_deref(), Some("command"));
    }

    #[test]
    fn round_trip_after_injection() {
        use infrarust_protocol::packets::Packet;
        let mut cmds = make_empty_tree();
        inject_proxy_commands(
            &mut cmds,
            ProtocolVersion::V1_21,
            &ProxyTree::default(),
            None,
        );
        let mut buf = Vec::new();
        cmds.encode(&mut buf, ProtocolVersion::V1_21).unwrap();
        let decoded = CCommands::decode(&mut buf.as_slice(), ProtocolVersion::V1_21).unwrap();
        assert_eq!(decoded.nodes.len(), cmds.nodes.len());
        assert_eq!(decoded.root_index, cmds.root_index);
    }

    #[test]
    fn filtered_subcommands_only_shows_visible() {
        let mut cmds = make_empty_tree();
        let mut visible = HashSet::new();
        visible.insert("help".to_string());
        visible.insert("list".to_string());

        inject_proxy_commands(
            &mut cmds,
            ProtocolVersion::V1_21,
            &ProxyTree::default(),
            Some(&visible),
        );

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

        assert!(child_names.contains(&"help"));
        assert!(child_names.contains(&"list"));
        assert!(!child_names.contains(&"kick"));
        assert!(!child_names.contains(&"reload"));
        assert!(!child_names.contains(&"send"));
    }

    #[test]
    fn empty_visible_set_hides_ir_entirely() {
        let mut cmds = make_empty_tree();
        let visible = HashSet::new();

        inject_proxy_commands(
            &mut cmds,
            ProtocolVersion::V1_21,
            &ProxyTree::default(),
            Some(&visible),
        );

        let root = &cmds.nodes[0];
        let names: Vec<&str> = root
            .children
            .iter()
            .filter_map(|&i| cmds.nodes[i as usize].name.as_deref())
            .collect();
        assert!(!names.contains(&"infrarust"));
        assert!(!names.contains(&"ir"));
    }

    #[test]
    fn plugin_commands_visible_even_with_empty_subcommands() {
        let mut cmds = make_empty_tree();
        let visible = HashSet::new();
        let plugin_cmds = tree(&[&["hello"]]);

        inject_proxy_commands(
            &mut cmds,
            ProtocolVersion::V1_21,
            &plugin_cmds,
            Some(&visible),
        );

        let root = &cmds.nodes[0];
        let names: Vec<&str> = root
            .children
            .iter()
            .filter_map(|&i| cmds.nodes[i as usize].name.as_deref())
            .collect();
        assert!(!names.contains(&"infrarust"));
        assert!(names.contains(&"hello"));
    }

    #[test]
    fn a_backend_root_named_like_a_proxy_command_is_replaced() {
        let mut cmds = make_empty_tree();
        cmds.nodes.push(CommandNode::literal_executable("Hello"));
        cmds.nodes.push(CommandNode::literal_executable("gamemode"));
        cmds.nodes.push(CommandNode::literal_executable("ir"));
        cmds.nodes[0].children.extend([1, 2, 3]);

        inject_proxy_commands(
            &mut cmds,
            ProtocolVersion::V1_21,
            &tree(&[&["hello", "hi", "p:hello"]]),
            None,
        );

        let mut names = root_names(&cmds);
        names.sort_unstable();
        assert_eq!(
            names,
            ["gamemode", "hello", "hi", "infrarust", "ir", "p:hello"]
        );
    }

    #[test]
    fn every_label_of_a_command_shares_one_args_node() {
        let mut cmds = make_empty_tree();
        inject_proxy_commands(
            &mut cmds,
            ProtocolVersion::V1_21,
            &tree(&[&["hello", "hi", "p:hello"]]),
            None,
        );
        let children: HashSet<Vec<i32>> = ["hello", "hi", "p:hello"]
            .iter()
            .map(|label| {
                cmds.nodes
                    .iter()
                    .find(|n| n.name.as_deref() == Some(*label))
                    .unwrap()
                    .children
                    .clone()
            })
            .collect();
        assert_eq!(children.len(), 1);
    }

    #[test]
    fn shadowed_but_hidden_labels_leave_no_root() {
        let mut cmds = make_empty_tree();
        cmds.nodes.push(CommandNode::literal_executable("secret"));
        cmds.nodes[0].children.push(1);
        let mut hidden = tree(&[]);
        hidden.shadowed.insert("secret".into());

        inject_proxy_commands(&mut cmds, ProtocolVersion::V1_21, &hidden, None);

        assert!(!root_names(&cmds).contains(&"secret"));
    }
}
