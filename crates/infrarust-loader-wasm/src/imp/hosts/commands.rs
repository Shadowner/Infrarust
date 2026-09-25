use infrarust_api::command::{CommandRegistration, CommandSpec};
use infrarust_api::permissions::Capability;

use crate::actor::CallKind;
use crate::bindings::infrarust::plugin::command_manager as wcm;
use crate::host_error::{HostResult, command_error};
use crate::proxies::WasmCommandHandler;
use crate::registrations::Bound;
use crate::store_state::PluginStoreState;

fn registration_to_wit(registration: &CommandRegistration) -> wcm::CommandRegistration {
    wcm::CommandRegistration {
        name: registration.name.clone(),
        namespaced: registration.namespaced.clone(),
        aliases: registration.aliases.clone(),
        rejected_aliases: registration.rejected_aliases.clone(),
    }
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
}

impl PluginStoreState {
    fn register_command(
        &mut self,
        spec: wcm::CommandSpec,
        handler: u64,
    ) -> HostResult<wcm::CommandRegistration> {
        self.check(Capability::Command, "command-manager.register")?;
        let ctx = self.services()?;
        let instance = self.instance_ref(CallKind::Callback).any_generation();
        let name = spec.name.clone();
        let binding = match self
            .registrations()
            .bind_command(&name, self.generation(), handler)
        {
            Bound::Fresh(binding) => binding,
            Bound::Rebound => {
                return Ok(self
                    .registrations()
                    .command_registration(&name)
                    .map_or_else(
                        || wcm::CommandRegistration {
                            name: name.clone(),
                            namespaced: format!("{}:{name}", self.plugin_id),
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
        self.check(Capability::Command, "command-manager.unregister")?;
        let Ok(ctx) = self.services() else {
            self.registrations().unbind_command(name);
            return Ok(());
        };
        match ctx.command_manager().unregister(name) {
            Ok(()) => {
                self.registrations().unbind_command(name);
                Ok(())
            }
            Err(error) => {
                self.report_command_refusal(name, &format!("unregister refused, {error}"));
                Err(command_error(&error))
            }
        }
    }
}
