use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct CommandPlugin;

#[plugin(id = "command-plugin", name = "Command Plugin Fixture")]
impl Plugin for CommandPlugin {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        ctx.command("greet")
            .description("Greets the caller")
            .handler(|invocation| {
                let _ = std::fs::write("command.marker", invocation.args.join(","));
            })
            .completer(|completion| {
                let partial = completion.partial();
                ["world", "everyone", "friend"]
                    .into_iter()
                    .filter(|candidate| candidate.starts_with(partial))
                    .collect::<Vec<_>>()
            })
            .register()?;
        ctx.command("nest")
            .description("Registers `nested` from inside its own completer")
            .completer(|_| {
                let _ = Context::new()
                    .command("nested")
                    .handler(|_| {
                        let _ = std::fs::write("nested.marker", "ran");
                    })
                    .completer(|_| vec!["inner"])
                    .register();
                vec!["registered"]
            })
            .register()?;
        ctx.command("unnest")
            .description("Unregisters `nested` through the SDK")
            .handler(|_| {
                let removed = Context::new().unregister_command("nested").unwrap_or(false);
                let _ = std::fs::write("unnest.marker", removed.to_string());
            })
            .register()?;
        ctx.command("lookup")
            .alias("find")
            .description("Writes what the host answers for each label")
            .handler(|invocation| {
                let _ = std::fs::write("lookup.marker", lookups(&invocation.args).join("\n"));
            })
            .register()?;
        Ok(())
    }
}

fn label_of(info: CommandInfo) -> String {
    info.namespaced().unwrap_or(info.name)
}

fn found(answer: Result<Option<CommandInfo>, Error>) -> String {
    match answer {
        Ok(info) => info.map_or_else(|| "-".to_owned(), label_of),
        Err(error) => format!("err {error}"),
    }
}

fn listed(answer: Result<Vec<CommandInfo>, Error>) -> String {
    match answer {
        Ok(infos) => infos
            .into_iter()
            .map(label_of)
            .collect::<Vec<_>>()
            .join(","),
        Err(error) => format!("err {error}"),
    }
}

fn lookups(labels: &[String]) -> Vec<String> {
    let mut lines: Vec<String> = labels
        .iter()
        .map(|label| {
            let contains = Commands::contains(label)
                .map_or_else(|error| format!("err {error}"), |answer| answer.to_string());
            format!(
                "{label} get={} name={} alias={} contains={contains}",
                found(Commands::get(label)),
                found(Commands::get_by_name(label)),
                found(Commands::get_by_alias(label)),
            )
        })
        .collect();
    lines.push(format!("list {}", listed(Commands::list())));
    lines.push(format!("owned {}", listed(Commands::list_owned())));
    lines
}
