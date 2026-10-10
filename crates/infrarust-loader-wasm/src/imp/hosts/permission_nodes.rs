use infrarust_api::permissions::{
    PermissionDefault, PermissionNode, PermissionNodeInfo, normalize_node,
};
use infrarust_api::plugin::PluginContext;

use crate::bindings::infrarust::plugin::permission_nodes as wpn;
use crate::bindings::infrarust::plugin::types::ErrorKind;
use crate::convert::wit_enum_map;
use crate::host_error::{HostResult, host_error, permission_node_error};
use crate::store_state::{PluginStoreState, Quota};

wit_enum_map!(default_from_wit: wpn::PermissionDefault => PermissionDefault {
    False, True, Admin
});

wit_enum_map!(default_to_wit: PermissionDefault => wpn::PermissionDefault {
    False, True, Admin
} else False);

fn node_from_wit(node: wpn::PermissionNode) -> PermissionNode {
    PermissionNode::new(node.name, default_from_wit(node.default)).description(node.description)
}

fn info_to_wit(info: PermissionNodeInfo) -> wpn::PermissionNodeInfo {
    let node = info.node;
    wpn::PermissionNodeInfo {
        node: wpn::PermissionNode {
            default: default_to_wit(node.default),
            name: node.name,
            description: node.description,
        },
        plugin_id: info.plugin_id,
    }
}

fn namespace_of(plugin_id: &str) -> String {
    format!("{}.", plugin_id.to_lowercase())
}

fn owned_by(info: &PermissionNodeInfo, plugin_id: &str) -> bool {
    info.plugin_id.as_deref() == Some(plugin_id)
}

impl wpn::Host for PluginStoreState {
    async fn register(&mut self, node: wpn::PermissionNode) -> wasmtime::Result<HostResult<()>> {
        Ok(self.register_permission_node(node))
    }

    async fn get(&mut self, name: String) -> wasmtime::Result<Option<wpn::PermissionNodeInfo>> {
        Ok(self
            .ctx()
            .and_then(|ctx| ctx.permission_node(&name))
            .map(info_to_wit))
    }

    async fn list(&mut self) -> wasmtime::Result<Vec<wpn::PermissionNodeInfo>> {
        Ok(self
            .ctx()
            .map(|ctx| {
                ctx.permission_nodes()
                    .into_iter()
                    .map(info_to_wit)
                    .collect()
            })
            .unwrap_or_default())
    }
}

impl PluginStoreState {
    fn register_permission_node(&mut self, node: wpn::PermissionNode) -> HostResult<()> {
        let ctx = self.services()?;
        let name = normalize_node(&node.name).into_owned();
        let namespace = namespace_of(self.plugin_id());
        if !name.starts_with(&namespace) {
            let reason = format!(
                "'{name}' is outside the plugin's namespace: a WASM plugin may only register nodes that start with '{namespace}'"
            );
            self.report_permission_node_refusal(&name, &format!("refused, {reason}"));
            return Err(host_error(ErrorKind::InvalidArgument, reason));
        }
        if !self.owns_permission_node(ctx.as_ref(), &name) {
            let held = self.held_permission_nodes(ctx.as_ref());
            self.admit(Quota::PermissionNodes, held, 1)?;
        }
        match ctx.register_permission_node(node_from_wit(node)) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.report_permission_node_refusal(&name, &format!("refused, {error}"));
                Err(permission_node_error(&error))
            }
        }
    }

    fn owns_permission_node(&self, ctx: &dyn PluginContext, name: &str) -> bool {
        ctx.permission_node(name)
            .is_some_and(|info| owned_by(&info, self.plugin_id()))
    }

    fn held_permission_nodes(&self, ctx: &dyn PluginContext) -> usize {
        ctx.permission_nodes()
            .iter()
            .filter(|info| owned_by(info, self.plugin_id()))
            .count()
    }
}
