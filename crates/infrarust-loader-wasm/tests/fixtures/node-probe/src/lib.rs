use std::fs::OpenOptions;
use std::io::Write;

use infrarust_plugin_sdk::prelude::*;

const LOG: &str = "nodes.log";

const REFUSED: [&str; 4] = [
    "fly",
    "native-owner.kick",
    "infrarust.fly",
    "node-probe.extra",
];

const LOOKED_UP: [&str; 4] = [
    "node-probe.fly",
    "native-owner.kick",
    "infrarust.admin",
    "nobody.node",
];

#[derive(Default)]
struct NodeProbe;

#[plugin(id = "node-probe", name = "Permission Node Probe Fixture")]
impl Plugin for NodeProbe {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        ctx.register_permission_node(
            PermissionNode::new("node-probe.use", PermissionDefault::True)
                .description("Use the probe"),
        )?;
        ctx.register_permission_node(PermissionNode::new(
            "node-probe.fly",
            PermissionDefault::True,
        ))?;
        ctx.register_permission_node(
            PermissionNode::new(" Node-Probe.FLY ", PermissionDefault::Admin)
                .description("Fly anywhere"),
        )?;
        let phase = match ctx.enable_reason() {
            Some(EnableReason::Recovered(_)) => "recovered",
            _ => "initial",
        };
        let mut lines = vec![format!("enable {phase}")];
        for name in REFUSED {
            let answer =
                ctx.register_permission_node(PermissionNode::new(name, PermissionDefault::True));
            lines.push(format!("register {name} {}", answer_text(answer)));
        }
        for name in LOOKED_UP {
            let found = ctx
                .permission_node(name)
                .map_or_else(|| "-".to_owned(), |info| describe(&info));
            lines.push(format!("get {name} {found}"));
        }
        let listed: Vec<String> = ctx
            .permission_nodes()
            .iter()
            .map(|info| {
                format!(
                    "{}={}",
                    info.node.name,
                    info.plugin_id.as_deref().unwrap_or("-")
                )
            })
            .collect();
        lines.push(format!("list {}", listed.join(",")));
        ctx.command("nodetrap")
            .description("Traps on purpose so the host recovers the plugin")
            .handler(|_| panic!("node-probe traps on purpose"))
            .register()?;
        append(&lines)?;
        Ok(())
    }
}

fn answer_text(answer: Result<(), Error>) -> String {
    match answer {
        Ok(()) => "ok".to_owned(),
        Err(error) => error.kind().to_string(),
    }
}

fn describe(info: &PermissionNodeInfo) -> String {
    format!(
        "{:?} {} {}",
        info.node.default,
        info.plugin_id.as_deref().unwrap_or("-"),
        info.node.description
    )
}

fn append(lines: &[String]) -> std::io::Result<()> {
    let mut log = OpenOptions::new().create(true).append(true).open(LOG)?;
    for line in lines {
        writeln!(log, "{line}")?;
    }
    Ok(())
}
