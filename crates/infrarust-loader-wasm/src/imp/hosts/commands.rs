use std::sync::Arc;

use infrarust_api::command::{CommandInfo, CommandManager, CommandRegistration, CommandSpec};

use super::Gate;
use crate::actor::CallKind;
use crate::bindings::infrarust::plugin::command_manager as wcm;
use crate::host_error::{HostResult, command_error};
use crate::proxies::WasmCommandHandler;
use crate::registrations::Bound;
use crate::store_state::{PluginStoreState, Quota};

fn registration_to_wit(registration: &CommandRegistration) -> wcm::CommandRegistration {
    wcm::CommandRegistration {
        name: registration.name.clone(),
        namespaced: registration.namespaced.clone(),
        aliases: registration.aliases.clone(),
        rejected_aliases: registration.rejected_aliases.clone(),
    }
}

fn spec_to_wit(spec: CommandSpec) -> wcm::CommandSpec {
    wcm::CommandSpec {
        name: spec.name,
        aliases: spec.aliases,
        description: spec.description,
        usage: spec.usage,
        permission: spec.permission,
        hidden: spec.hidden,
    }
}

fn info_to_wit(info: CommandInfo) -> wcm::CommandInfo {
    wcm::CommandInfo {
        spec: spec_to_wit(info.spec),
        plugin_id: info.plugin_id,
    }
}

fn infos_to_wit(infos: Vec<CommandInfo>) -> Vec<wcm::CommandInfo> {
    infos.into_iter().map(info_to_wit).collect()
}

fn command_key(name: &str) -> String {
    name.trim().to_lowercase()
}

fn spec_from_wit(spec: wcm::CommandSpec) -> CommandSpec {
    let mut native = CommandSpec::new(spec.name)
        .aliases(spec.aliases)
        .description(spec.description)
        .hidden(spec.hidden);
    if let Some(usage) = spec.usage {
        native = native.usage(usage);
    }
    if let Some(permission) = spec.permission {
        native = native.permission(permission);
    }
    native
}

impl wcm::Host for PluginStoreState {
    async fn register(
        &mut self,
        spec: wcm::CommandSpec,
        handler: u64,
    ) -> wasmtime::Result<HostResult<wcm::CommandRegistration>> {
        Ok(self.register_command(spec, handler))
    }

    async fn unregister(&mut self, name: String) -> wasmtime::Result<HostResult<()>> {
        Ok(self.unregister_command(&name))
    }

    async fn get(
        &mut self,
        label: String,
    ) -> wasmtime::Result<HostResult<Option<wcm::CommandInfo>>> {
        Ok(self
            .commands(gate!("command-manager", "get"))
            .map(|commands| commands.get(&label).map(info_to_wit)))
    }

    async fn get_by_name(
        &mut self,
        name: String,
    ) -> wasmtime::Result<HostResult<Option<wcm::CommandInfo>>> {
        Ok(self
            .commands(gate!("command-manager", "get-by-name"))
            .map(|commands| commands.get_by_name(&name).map(info_to_wit)))
    }

    async fn get_by_alias(
        &mut self,
        alias: String,
    ) -> wasmtime::Result<HostResult<Option<wcm::CommandInfo>>> {
        Ok(self
            .commands(gate!("command-manager", "get-by-alias"))
            .map(|commands| commands.get_by_alias(&alias).map(info_to_wit)))
    }

    async fn contains(&mut self, label: String) -> wasmtime::Result<HostResult<bool>> {
        Ok(self
            .commands(gate!("command-manager", "contains"))
            .map(|commands| commands.contains(&label)))
    }

    async fn list(&mut self) -> wasmtime::Result<HostResult<Vec<wcm::CommandInfo>>> {
        Ok(self
            .commands(gate!("command-manager", "list"))
            .map(|commands| infos_to_wit(commands.list())))
    }

    async fn list_owned(&mut self) -> wasmtime::Result<HostResult<Vec<wcm::CommandInfo>>> {
        Ok(self
            .commands(gate!("command-manager", "list-owned"))
            .map(|commands| infos_to_wit(commands.list_owned())))
    }
}

impl PluginStoreState {
    fn commands(&mut self, call: Gate) -> HostResult<Arc<dyn CommandManager>> {
        self.check(call)?;
        Ok(self.services()?.command_manager())
    }

    fn register_command(
        &mut self,
        spec: wcm::CommandSpec,
        handler: u64,
    ) -> HostResult<wcm::CommandRegistration> {
        self.check(gate!("command-manager", "register"))?;
        let ctx = self.services()?;
        let instance = self.instance_ref(CallKind::Callback)?.any_generation();
        let name = command_key(&spec.name);
        let limit = self.quota(Quota::Commands);
        let binding =
            match self
                .registrations()
                .bind_command(&name, self.generation(), handler, limit)
            {
                Bound::Fresh(binding) => binding,
                Bound::Full => return Err(self.quota_exceeded(Quota::Commands)),
                Bound::Rebound => {
                    return Ok(self
                        .registrations()
                        .command_registration(&name)
                        .map_or_else(
                            || wcm::CommandRegistration {
                                name: name.clone(),
                                namespaced: format!("{}:{name}", self.plugin_id()),
                                aliases: Vec::new(),
                                rejected_aliases: Vec::new(),
                            },
                            |registration| registration_to_wit(&registration),
                        ));
                }
            };
        let handler = Box::new(WasmCommandHandler::new(binding, instance));
        match ctx.command_manager().register(spec_from_wit(spec), handler) {
            Ok(registration) => {
                if !registration.rejected_aliases.is_empty() {
                    let reason = format!(
                        "aliases {:?} are taken or invalid and were skipped",
                        registration.rejected_aliases
                    );
                    self.report_command_refusal(&name, &reason);
                }
                let answer = registration_to_wit(&registration);
                self.registrations()
                    .record_command_registration(&name, registration);
                Ok(answer)
            }
            Err(error) => {
                self.registrations().unbind_command(&name);
                self.report_command_refusal(&name, &format!("refused, {error}"));
                Err(command_error(&error))
            }
        }
    }

    fn unregister_command(&mut self, name: &str) -> HostResult<()> {
        self.check(gate!("command-manager", "unregister"))?;
        let key = command_key(name);
        let Ok(ctx) = self.services() else {
            self.registrations().unbind_command(&key);
            return Ok(());
        };
        match ctx.command_manager().unregister(name) {
            Ok(()) => {
                self.registrations().unbind_command(&key);
                Ok(())
            }
            Err(error) => {
                self.report_command_refusal(name, &format!("unregister refused, {error}"));
                Err(command_error(&error))
            }
        }
    }
}
