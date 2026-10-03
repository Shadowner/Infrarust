use std::collections::HashSet;

use infrarust_protocol::error::{ProtocolError, ProtocolResult};
use infrarust_protocol::packets::play::commands::{
    CCommands, CommandNode, NodeKind, string_parser,
};
use infrarust_protocol::version::ProtocolVersion;

use crate::services::command_manager::ProxyTree;

const ASK_SERVER: Option<&str> = Some("minecraft:ask_server");
const SINGLE_WORD: i32 = 0;
const GREEDY_PHRASE: i32 = 2;

struct SubTree {
    base: i32,
    nodes: Vec<CommandNode>,
}

impl SubTree {
    fn new(base: usize) -> Self {
        Self {
            base: base as i32,
            nodes: Vec::new(),
        }
    }

    fn push(&mut self, node: CommandNode) -> usize {
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    fn absolute(&self, local: usize) -> i32 {
        self.base + local as i32
    }

    fn link(&mut self, parent: usize, child: usize) {
        let child = self.absolute(child);
        self.nodes[parent].children.push(child);
    }
}

fn drop_shadowed_roots(nodes: &mut [CommandNode], root: usize, shadowed: &HashSet<String>) {
    let children = std::mem::take(&mut nodes[root].children);
    nodes[root].children = children
        .into_iter()
        .filter(|&child| {
            let node = usize::try_from(child).ok().and_then(|i| nodes.get(i));
            !node.is_some_and(|node| {
                matches!(
                    &node.kind,
                    NodeKind::Literal { name } if shadowed.contains(&name.to_lowercase())
                )
            })
        })
        .collect();
}

pub fn inject_proxy_commands(
    commands: &mut CCommands,
    version: ProtocolVersion,
    tree: &ProxyTree,
    visible_subcommands: Option<&HashSet<String>>,
) -> ProtocolResult<()> {
    let root = commands
        .root_position()
        .ok_or_else(|| ProtocolError::invalid("command tree root index out of range"))?;
    drop_shadowed_roots(&mut commands.nodes, root, &tree.shadowed);

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
        let mut sub = SubTree::new(commands.nodes.len());
        let infrarust_idx = sub.push(CommandNode::literal("infrarust"));
        let mut ir_children = Vec::new();

        if is_visible("help") {
            let help_idx = sub.push(CommandNode::literal_executable("help"));
            let help_cmd_idx = sub.push(CommandNode::argument(
                "command",
                string_parser(SINGLE_WORD, version)?,
                ASK_SERVER,
            ));
            sub.link(help_idx, help_cmd_idx);
            ir_children.push(help_idx);
        }

        if is_visible("version") {
            ir_children.push(sub.push(CommandNode::literal_executable("version")));
        }

        if is_visible("list") {
            ir_children.push(sub.push(CommandNode::literal_executable("list")));
        }

        if is_visible("plugins") {
            ir_children.push(sub.push(CommandNode::literal_executable("plugins")));
        }

        if is_visible("reload") {
            ir_children.push(sub.push(CommandNode::literal_executable("reload")));
        }

        if is_visible("server") {
            let server_idx = sub.push(CommandNode::literal_executable("server"));
            let server_name_idx = sub.push(CommandNode::argument(
                "name",
                string_parser(SINGLE_WORD, version)?,
                ASK_SERVER,
            ));
            sub.link(server_idx, server_name_idx);
            ir_children.push(server_idx);
        }

        if is_visible("find") {
            let find_idx = sub.push(CommandNode::literal("find"));
            let find_player_idx = sub.push(CommandNode::argument(
                "player",
                string_parser(SINGLE_WORD, version)?,
                ASK_SERVER,
            ));
            sub.link(find_idx, find_player_idx);
            ir_children.push(find_idx);
        }

        if is_visible("send") {
            let send_idx = sub.push(CommandNode::literal("send"));
            let send_player_idx = sub.push(CommandNode::argument_non_executable(
                "player",
                string_parser(SINGLE_WORD, version)?,
                ASK_SERVER,
            ));
            let send_server_idx = sub.push(CommandNode::argument(
                "server",
                string_parser(SINGLE_WORD, version)?,
                ASK_SERVER,
            ));
            sub.link(send_player_idx, send_server_idx);
            sub.link(send_idx, send_player_idx);
            ir_children.push(send_idx);
        }

        if is_visible("kick") {
            let kick_idx = sub.push(CommandNode::literal("kick"));
            let kick_player_idx = sub.push(CommandNode::argument(
                "player",
                string_parser(SINGLE_WORD, version)?,
                ASK_SERVER,
            ));
            let kick_reason_idx = sub.push(CommandNode::argument(
                "reason",
                string_parser(GREEDY_PHRASE, version)?,
                None,
            ));
            sub.link(kick_player_idx, kick_reason_idx);
            sub.link(kick_idx, kick_player_idx);
            ir_children.push(kick_idx);
        }

        if is_visible("broadcast") {
            let broadcast_idx = sub.push(CommandNode::literal("broadcast"));
            let broadcast_msg_idx = sub.push(CommandNode::argument(
                "message",
                string_parser(GREEDY_PHRASE, version)?,
                None,
            ));
            sub.link(broadcast_idx, broadcast_msg_idx);
            ir_children.push(broadcast_idx);
        }

        if is_visible("plugin") {
            let plugin_lit_idx = sub.push(CommandNode::literal("plugin"));
            let plugin_id_idx = sub.push(CommandNode::argument_non_executable(
                "plugin_id",
                string_parser(SINGLE_WORD, version)?,
                ASK_SERVER,
            ));
            let plugin_cmd_idx = sub.push(CommandNode::argument(
                "command",
                string_parser(GREEDY_PHRASE, version)?,
                ASK_SERVER,
            ));
            sub.link(plugin_id_idx, plugin_cmd_idx);
            sub.link(plugin_lit_idx, plugin_id_idx);
            ir_children.push(plugin_lit_idx);
        }

        let children: Vec<i32> = ir_children
            .iter()
            .map(|&child| sub.absolute(child))
            .collect();
        sub.nodes[infrarust_idx].children = children;

        let ir_idx = sub.push(CommandNode::redirect("ir", sub.absolute(infrarust_idx)));

        let (infrarust, ir) = (sub.absolute(infrarust_idx), sub.absolute(ir_idx));
        commands.nodes.extend(sub.nodes);
        commands.nodes[root].children.push(infrarust);
        commands.nodes[root].children.push(ir);
    }

    for labels in &tree.commands {
        let args_idx = commands.nodes.len() as i32;
        commands.nodes.push(CommandNode::argument(
            "args",
            string_parser(GREEDY_PHRASE, version)?,
            ASK_SERVER,
        ));
        for label in labels {
            let cmd_idx = commands.nodes.len() as i32;
            let mut node = CommandNode::literal_executable(label);
            node.children.push(args_idx);
            commands.nodes.push(node);
            commands.nodes[root].children.push(cmd_idx);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

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
            .filter_map(|&i| cmds.nodes[i as usize].name())
            .collect()
    }

    fn make_empty_tree() -> CCommands {
        CCommands {
            nodes: vec![CommandNode::root()],
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
        )
        .unwrap();
        let root = &cmds.nodes[0];
        let names: Vec<&str> = root
            .children
            .iter()
            .filter_map(|&i| cmds.nodes[i as usize].name())
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
        )
        .unwrap();
        let ir = cmds.nodes.iter().find(|n| n.name() == Some("ir")).unwrap();
        let infrarust_idx = cmds
            .nodes
            .iter()
            .position(|n| n.name() == Some("infrarust"))
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
        )
        .unwrap();
        let infrarust = cmds
            .nodes
            .iter()
            .find(|n| n.name() == Some("infrarust"))
            .unwrap();
        let child_names: Vec<&str> = infrarust
            .children
            .iter()
            .filter_map(|&i| cmds.nodes[i as usize].name())
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
        )
        .unwrap();
        let server_node = cmds
            .nodes
            .iter()
            .find(|n| n.name() == Some("server") && n.executable)
            .unwrap();
        let name_arg_idx = server_node.children[0] as usize;
        let name_arg = &cmds.nodes[name_arg_idx];
        assert!(matches!(
            name_arg.kind,
            NodeKind::Argument {
                suggestions_type: Some(ref suggestions),
                ..
            } if suggestions == "minecraft:ask_server"
        ));
    }

    #[test]
    fn plugin_commands_injected_at_root() {
        let mut cmds = make_empty_tree();
        let plugin_cmds = tree(&[&["hello", "hello-plugin:hello"], &["forcelogin"]]);
        inject_proxy_commands(&mut cmds, ProtocolVersion::V1_21, &plugin_cmds, None).unwrap();
        let root = &cmds.nodes[0];
        let names: Vec<&str> = root
            .children
            .iter()
            .filter_map(|&i| cmds.nodes[i as usize].name())
            .collect();
        assert!(names.contains(&"hello"), "missing plugin command: hello");
        assert!(
            names.contains(&"forcelogin"),
            "missing plugin command: forcelogin"
        );

        let hello_idx = root
            .children
            .iter()
            .find(|&&i| cmds.nodes[i as usize].name() == Some("hello"))
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
        )
        .unwrap();
        let infrarust = cmds
            .nodes
            .iter()
            .find(|n| n.name() == Some("infrarust"))
            .unwrap();
        let child_names: Vec<&str> = infrarust
            .children
            .iter()
            .filter_map(|&i| cmds.nodes[i as usize].name())
            .collect();
        assert!(
            child_names.contains(&"plugin"),
            "infrarust should have 'plugin' subcommand"
        );

        let plugin_idx = infrarust
            .children
            .iter()
            .find(|&&i| cmds.nodes[i as usize].name() == Some("plugin"))
            .unwrap();
        let plugin_node = &cmds.nodes[*plugin_idx as usize];
        assert_eq!(plugin_node.children.len(), 1);
        let plugin_id_node = &cmds.nodes[plugin_node.children[0] as usize];
        assert_eq!(plugin_id_node.name(), Some("plugin_id"));
        assert_eq!(plugin_id_node.children.len(), 1);
        let cmd_node = &cmds.nodes[plugin_id_node.children[0] as usize];
        assert_eq!(cmd_node.name(), Some("command"));
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
        )
        .unwrap();
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
        )
        .unwrap();

        let infrarust = cmds
            .nodes
            .iter()
            .find(|n| n.name() == Some("infrarust"))
            .unwrap();
        let child_names: Vec<&str> = infrarust
            .children
            .iter()
            .filter_map(|&i| cmds.nodes[i as usize].name())
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
        )
        .unwrap();

        let root = &cmds.nodes[0];
        let names: Vec<&str> = root
            .children
            .iter()
            .filter_map(|&i| cmds.nodes[i as usize].name())
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
        )
        .unwrap();

        let root = &cmds.nodes[0];
        let names: Vec<&str> = root
            .children
            .iter()
            .filter_map(|&i| cmds.nodes[i as usize].name())
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
        )
        .unwrap();

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
        )
        .unwrap();
        let children: HashSet<Vec<i32>> = ["hello", "hi", "p:hello"]
            .iter()
            .map(|label| {
                cmds.nodes
                    .iter()
                    .find(|n| n.name() == Some(*label))
                    .unwrap()
                    .children
                    .clone()
            })
            .collect();
        assert_eq!(children.len(), 1);
    }

    #[test]
    fn an_out_of_range_root_is_an_error_not_a_panic() {
        for root_index in [-1, 1, i32::MAX] {
            let mut cmds = make_empty_tree();
            cmds.root_index = root_index;
            let tree = tree(&[&["hello"]]);
            assert!(inject_proxy_commands(&mut cmds, ProtocolVersion::V1_21, &tree, None).is_err());
        }
    }

    #[test]
    fn shadowed_but_hidden_labels_leave_no_root() {
        let mut cmds = make_empty_tree();
        cmds.nodes.push(CommandNode::literal_executable("secret"));
        cmds.nodes[0].children.push(1);
        let mut hidden = tree(&[]);
        hidden.shadowed.insert("secret".into());

        inject_proxy_commands(&mut cmds, ProtocolVersion::V1_21, &hidden, None).unwrap();

        assert!(!root_names(&cmds).contains(&"secret"));
    }
}
