//! Secret fields of a configuration document.
//!
//! Documents leave the proxy redacted and come back for a write. A redacted
//! field keeps its key and carries [`REDACTED`] instead of its value, so the
//! document stays a valid instance of its schema — `server_manager.api_key` is
//! a required field — and a client that saves an untouched document restores
//! the live secret through [`reinject`] instead of overwriting it.

use toml_edit::{DocumentMut, Item, Table, TableLike};

/// Stands in for a secret in a document that leaves the proxy. Never a
/// credential: [`WebConfig::resolve_api_key`](crate::WebConfig::resolve_api_key)
/// refuses it, and a write that cannot put the real value back is rejected.
pub const REDACTED: &str = "<redacted>";

/// Secret fields of the global proxy config (`infrarust.toml`).
pub const PROXY_SECRETS: &[&[&str]] = &[&["web", "api_key"]];

/// Secret fields of a server config document.
pub const SERVER_SECRETS: &[&[&str]] = &[&["server_manager", "api_key"]];

/// Replaces the value of every listed field of `doc` with [`REDACTED`].
pub fn redact(doc: &mut DocumentMut, paths: &[&[&str]]) {
    for path in paths {
        let Some((key, parent)) = path.split_last() else {
            continue;
        };
        if let Some(table) = table_at_mut(doc, parent)
            && table.contains_key(key)
        {
            table.insert(key, toml_edit::value(REDACTED));
        }
    }
}

/// Copies every listed field of `current` that `doc` left out or redacted.
///
/// A field whose parent table is missing from `doc` is left out: dropping the
/// whole `[web]` section is a deliberate removal, not an omitted secret.
pub fn reinject(doc: &mut DocumentMut, current: &DocumentMut, paths: &[&[&str]]) {
    for path in paths {
        let Some((key, parent)) = path.split_last() else {
            continue;
        };
        let Some(value) = table_at(current, parent)
            .and_then(|table| table.get(key))
            .cloned()
        else {
            continue;
        };
        if let Some(table) = table_at_mut(doc, parent) {
            let submitted = table
                .get(key)
                .is_some_and(|item| item.as_str() != Some(REDACTED));
            if !submitted {
                table.insert(key, value);
            }
        }
    }
}

/// The listed fields of `doc` still carrying [`REDACTED`], dotted — secrets no
/// stored document could put back. Persisting one would turn the placeholder
/// into the live credential.
pub fn still_redacted(doc: &DocumentMut, paths: &[&[&str]]) -> Vec<String> {
    paths
        .iter()
        .filter(|path| {
            path.split_last().is_some_and(|(key, parent)| {
                table_at(doc, parent)
                    .and_then(|table| table.get(key))
                    .and_then(Item::as_str)
                    == Some(REDACTED)
            })
        })
        .map(|path| path.join("."))
        .collect()
}

pub const PLUGINS: &str = "plugins";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PluginScopeError {
    #[error("the document is not valid TOML: {0}")]
    Invalid(String),
    #[error("the stored configuration could not be read: {0}")]
    Unreadable(String),
    #[error("a plugin cannot write another plugin's block: {}", dotted(.0))]
    OtherPlugins(Vec<String>),
}

pub fn names_another_plugin(key: &str, plugin_id: &str) -> bool {
    let mut segments = key.split('.');
    segments.next() == Some(PLUGINS) && segments.next().is_some_and(|id| id != plugin_id)
}

pub fn plugin_view(document: &str, plugin_id: &str) -> Result<String, PluginScopeError> {
    let mut doc = document
        .parse::<DocumentMut>()
        .map_err(|e| PluginScopeError::Unreadable(e.to_string()))?;
    hide_other_plugins(&mut doc, plugin_id);
    Ok(doc.to_string())
}

pub fn plugins_value_view(value: &str, plugin_id: &str) -> Result<String, PluginScopeError> {
    let mut wrapper: toml::Table = toml::from_str(&format!("{PLUGINS} = {value}"))
        .map_err(|e| PluginScopeError::Unreadable(e.to_string()))?;
    let Some(toml::Value::Table(mut plugins)) = wrapper.remove(PLUGINS) else {
        return Err(PluginScopeError::Unreadable(format!(
            "`{PLUGINS}` is not a table"
        )));
    };
    plugins.retain(|id, _| id == plugin_id);
    Ok(toml::Value::Table(plugins).to_string())
}

pub fn plugin_write(
    submitted: &str,
    current: &str,
    plugin_id: &str,
) -> Result<String, PluginScopeError> {
    let mut doc = submitted
        .parse::<DocumentMut>()
        .map_err(|e| PluginScopeError::Invalid(e.to_string()))?;
    let others = other_plugins(&doc, plugin_id);
    if !others.is_empty() {
        return Err(PluginScopeError::OtherPlugins(others));
    }
    let current = current
        .parse::<DocumentMut>()
        .map_err(|e| PluginScopeError::Unreadable(e.to_string()))?;
    restore_other_plugins(&mut doc, &current, plugin_id);
    Ok(doc.to_string())
}

fn hide_other_plugins(doc: &mut DocumentMut, plugin_id: &str) {
    if doc
        .get(PLUGINS)
        .is_some_and(|plugins| plugins.as_table_like().is_none())
    {
        doc.remove(PLUGINS);
    }
    if let Some(plugins) = table_at_mut(doc, &[PLUGINS]) {
        for id in other_plugin_ids(&*plugins, plugin_id) {
            plugins.remove(&id);
        }
    }
}

fn other_plugins(doc: &DocumentMut, plugin_id: &str) -> Vec<String> {
    table_at(doc, &[PLUGINS])
        .map(|plugins| other_plugin_ids(plugins, plugin_id))
        .unwrap_or_default()
}

fn restore_other_plugins(doc: &mut DocumentMut, current: &DocumentMut, plugin_id: &str) {
    let blocks: Vec<(String, Item)> = table_at(current, &[PLUGINS])
        .map(|stored| {
            stored
                .iter()
                .filter(|(id, _)| *id != plugin_id)
                .map(|(id, block)| (id.to_owned(), block.clone()))
                .collect()
        })
        .unwrap_or_default();
    if blocks.is_empty() {
        return;
    }
    if !doc.contains_key(PLUGINS) {
        let mut plugins = Table::new();
        plugins.set_implicit(true);
        doc.insert(PLUGINS, Item::Table(plugins));
    }
    if let Some(plugins) = table_at_mut(doc, &[PLUGINS]) {
        for (id, block) in blocks {
            plugins.insert(&id, block);
        }
    }
}

fn other_plugin_ids(plugins: &dyn TableLike, plugin_id: &str) -> Vec<String> {
    plugins
        .iter()
        .map(|(id, _)| id)
        .filter(|id| *id != plugin_id)
        .map(str::to_owned)
        .collect()
}

fn dotted(ids: &[String]) -> String {
    ids.iter()
        .map(|id| format!("{PLUGINS}.{id}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn table_at<'a>(doc: &'a DocumentMut, path: &[&str]) -> Option<&'a dyn TableLike> {
    let mut current: &dyn TableLike = doc.as_table();
    for key in path {
        current = current.get(key)?.as_table_like()?;
    }
    Some(current)
}

fn table_at_mut<'a>(doc: &'a mut DocumentMut, path: &[&str]) -> Option<&'a mut dyn TableLike> {
    let mut current: &mut dyn TableLike = doc.as_table_mut();
    for key in path {
        current = current.get_mut(key)?.as_table_like_mut()?;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn doc(text: &str) -> DocumentMut {
        text.parse().unwrap()
    }

    #[test]
    fn redact_replaces_only_the_secret() {
        let mut d = doc("[web]\n# keep me\nbind = \"127.0.0.1:8080\"\napi_key = \"topsecret\"\n");
        redact(&mut d, PROXY_SECRETS);
        let text = d.to_string();
        assert!(!text.contains("topsecret"));
        assert!(text.contains(REDACTED));
        assert!(text.contains("# keep me"));
        assert!(text.contains("bind"));
    }

    #[test]
    fn redact_does_not_add_a_field_the_document_lacks() {
        let mut d = doc("[web]\nbind = \"127.0.0.1:8080\"\n");
        redact(&mut d, PROXY_SECRETS);
        assert!(!d.to_string().contains("api_key"));
    }

    #[test]
    fn reinject_restores_a_redacted_secret() {
        let current = doc("[web]\napi_key = \"topsecret\"\n");
        let mut submitted = current.clone();
        redact(&mut submitted, PROXY_SECRETS);

        reinject(&mut submitted, &current, PROXY_SECRETS);

        assert!(submitted.to_string().contains("topsecret"));
        assert!(still_redacted(&submitted, PROXY_SECRETS).is_empty());
    }

    #[test]
    fn a_redaction_nothing_can_restore_is_reported() {
        let mut submitted = doc("[web]\napi_key = \"topsecret\"\n");
        redact(&mut submitted, PROXY_SECRETS);

        reinject(
            &mut submitted,
            &doc("bind = \"0.0.0.0:25565\"\n"),
            PROXY_SECRETS,
        );

        assert_eq!(still_redacted(&submitted, PROXY_SECRETS), ["web.api_key"]);
    }

    #[test]
    fn a_required_secret_survives_a_redacted_round_trip() {
        let current = doc(
            "[server_manager]\ntype = \"pterodactyl\"\napi_url = \"https://panel\"\napi_key = \"ptlc_x\"\nserver_id = \"abc\"\n",
        );
        let mut redacted = current.clone();
        redact(&mut redacted, SERVER_SECRETS);

        #[derive(serde::Deserialize)]
        struct Document {
            server_manager: crate::ServerManagerConfig,
        }
        let document: Document = toml::from_str(&redacted.to_string())
            .expect("a redacted document still matches the schema");
        assert!(matches!(
            document.server_manager,
            crate::ServerManagerConfig::Pterodactyl(_)
        ));

        reinject(&mut redacted, &current, SERVER_SECRETS);
        assert!(redacted.to_string().contains("ptlc_x"));
    }

    #[test]
    fn reinject_restores_an_omitted_secret() {
        let current = doc("[web]\napi_key = \"topsecret\"\n");
        let mut submitted = doc("[web]\nbind = \"0.0.0.0:9000\"\n");
        reinject(&mut submitted, &current, PROXY_SECRETS);
        assert!(submitted.to_string().contains("topsecret"));
    }

    #[test]
    fn reinject_keeps_a_submitted_secret() {
        let current = doc("[web]\napi_key = \"old\"\n");
        let mut submitted = doc("[web]\napi_key = \"new\"\n");
        reinject(&mut submitted, &current, PROXY_SECRETS);
        let text = submitted.to_string();
        assert!(text.contains("new"));
        assert!(!text.contains("old"));
    }

    #[test]
    fn reinject_does_not_resurrect_a_removed_section() {
        let current = doc("[web]\napi_key = \"topsecret\"\n");
        let mut submitted = doc("bind = \"0.0.0.0:25565\"\n");
        reinject(&mut submitted, &current, PROXY_SECRETS);
        assert!(!submitted.to_string().contains("web"));
    }

    #[test]
    fn server_manager_secret_round_trips() {
        let current = doc(
            "[server_manager]\ntype = \"pterodactyl\"\napi_url = \"https://panel\"\napi_key = \"ptlc_x\"\nserver_id = \"abc\"\n",
        );
        let mut redacted = current.clone();
        redact(&mut redacted, SERVER_SECRETS);
        assert!(!redacted.to_string().contains("ptlc_x"));

        reinject(&mut redacted, &current, SERVER_SECRETS);
        assert!(redacted.to_string().contains("ptlc_x"));
    }

    const PLUGIN_BLOCKS: &str = "\
bind = \"0.0.0.0:25565\"

# the other plugin's block
[plugins.other]
path = \"/srv/other.wasm\"
permissions = [\"ban\"]

[plugins.other.wasm]
max_memory = \"64MiB\"

[plugins.mine]
permissions = [\"limbo\"]

[web]
bind = \"127.0.0.1:8080\"
";

    fn plugins_of(text: &str) -> toml::Table {
        let document: toml::Table = toml::from_str(text).unwrap();
        document
            .get(PLUGINS)
            .and_then(toml::Value::as_table)
            .cloned()
            .unwrap_or_default()
    }

    #[test]
    fn a_plugin_view_keeps_its_own_block_and_the_rest_of_the_document() {
        let view = plugin_view(PLUGIN_BLOCKS, "mine").unwrap();

        assert!(!view.contains("other"), "{view}");
        assert!(!view.contains("/srv/other.wasm"), "{view}");
        assert!(!view.contains("64MiB"), "{view}");
        assert!(view.contains("[plugins.mine]"), "{view}");
        assert!(view.contains("127.0.0.1:8080"), "{view}");
        assert!(view.contains("0.0.0.0:25565"), "{view}");
        assert_eq!(plugins_of(&view).keys().collect::<Vec<_>>(), ["mine"]);
    }

    #[test]
    fn a_plugin_view_hides_dotted_and_inline_blocks() {
        for text in [
            "plugins.other.path = \"/srv/other.wasm\"\nplugins.mine.enabled = true\n",
            "plugins = { other = { path = \"/srv/other.wasm\" }, mine = { enabled = true } }\n",
            "[plugins]\nother = { path = \"/srv/other.wasm\" }\nmine = { enabled = true }\n",
        ] {
            let view = plugin_view(text, "mine").unwrap();
            assert!(!view.contains("/srv/other.wasm"), "{view}");
            assert_eq!(
                plugins_of(&view).keys().collect::<Vec<_>>(),
                ["mine"],
                "{view}"
            );
        }
    }

    #[test]
    fn a_plugin_with_no_block_sees_no_plugin_block() {
        let view = plugin_view(PLUGIN_BLOCKS, "stranger").unwrap();
        assert!(plugins_of(&view).is_empty(), "{view}");
        assert!(view.contains("127.0.0.1:8080"), "{view}");
    }

    #[test]
    fn a_plugins_entry_that_is_not_a_table_is_hidden_whole() {
        let view = plugin_view(
            "bind = \"0.0.0.0:25565\"\n[[plugins]]\nid = \"other\"\ntoken = \"SECRET\"\n",
            "mine",
        )
        .unwrap();
        assert!(!view.contains("SECRET"), "{view}");
        assert!(view.contains("0.0.0.0:25565"), "{view}");
    }

    #[test]
    fn a_document_that_is_not_toml_has_no_plugin_view() {
        assert!(matches!(
            plugin_view("[plugins.other] token=SECRET", "mine"),
            Err(PluginScopeError::Unreadable(_))
        ));
    }

    #[test]
    fn a_key_under_another_plugin_is_named_by_its_second_segment() {
        for key in [
            "plugins.other",
            "plugins.other.path",
            "plugins.nobody.x",
            "plugins.mine-too.path",
            "plugins.",
        ] {
            assert!(names_another_plugin(key, "mine"), "{key}");
        }
        for key in [
            "plugins",
            "plugins.mine",
            "plugins.mine.permissions",
            "bind",
            "web.bind",
            "wasm.plugins.other",
        ] {
            assert!(!names_another_plugin(key, "mine"), "{key}");
        }
    }

    #[test]
    fn the_plugins_value_keeps_only_the_callers_entry() {
        let value = "{ mine = { enabled = true }, other = { path = \"/srv/other.wasm\" } }";
        assert_eq!(
            plugins_value_view(value, "mine").unwrap(),
            "{ mine = { enabled = true } }"
        );
        assert_eq!(plugins_value_view(value, "stranger").unwrap(), "{}");
        assert!(plugins_value_view("\"text\"", "mine").is_err());
        assert!(plugins_value_view("{ unclosed", "mine").is_err());
    }

    #[test]
    fn a_plugin_write_puts_the_other_blocks_back() {
        let view = plugin_view(PLUGIN_BLOCKS, "mine").unwrap();
        let edited = view.replace("0.0.0.0:25565", "0.0.0.0:25566");

        let written = plugin_write(&edited, PLUGIN_BLOCKS, "mine").unwrap();

        assert!(written.contains("0.0.0.0:25566"), "{written}");
        assert!(written.contains("# the other plugin's block"), "{written}");
        assert_eq!(plugins_of(&written), plugins_of(PLUGIN_BLOCKS), "{written}");
    }

    #[test]
    fn a_plugin_write_that_drops_its_own_block_keeps_the_others() {
        let written = plugin_write(
            "bind = \"0.0.0.0:25565\"\n[web]\nbind = \"127.0.0.1:8080\"\n",
            PLUGIN_BLOCKS,
            "mine",
        )
        .unwrap();

        let mut expected = plugins_of(PLUGIN_BLOCKS);
        expected.remove("mine");
        assert_eq!(plugins_of(&written), expected, "{written}");
    }

    #[test]
    fn a_plugin_write_keeps_blocks_whatever_their_layout() {
        let stored = [
            "plugins.other.path = \"/srv/other.wasm\"\nplugins.mine.enabled = true\n",
            "plugins = { other = { path = \"/srv/other.wasm\" }, mine = { enabled = true } }\n",
            PLUGIN_BLOCKS,
        ];
        let submitted = [
            "bind = \"0.0.0.0:1\"\n",
            "plugins.mine.enabled = false\n",
            "plugins = { mine = { enabled = false } }\n",
            "[plugins]\nmine = { enabled = false }\n",
            "[plugins.mine]\nenabled = false\n",
        ];
        for current in stored {
            for text in submitted {
                let written = plugin_write(text, current, "mine").unwrap();
                let plugins = plugins_of(&written);
                assert_eq!(
                    plugins.get("other"),
                    plugins_of(current).get("other"),
                    "{current:?} + {text:?} -> {written}"
                );
                assert_eq!(
                    plugins.get("mine"),
                    plugins_of(text).get("mine"),
                    "{current:?} + {text:?} -> {written}"
                );
            }
        }
    }

    #[test]
    fn a_plugin_write_that_names_another_plugin_is_refused() {
        let submitted = "[plugins.other]\npermissions = [\"server-manage\"]\n[plugins.mine]\n";

        assert_eq!(
            plugin_write(submitted, PLUGIN_BLOCKS, "mine"),
            Err(PluginScopeError::OtherPlugins(vec!["other".to_owned()]))
        );
        assert_eq!(
            PluginScopeError::OtherPlugins(vec!["other".to_owned()]).to_string(),
            "a plugin cannot write another plugin's block: plugins.other"
        );
    }

    #[test]
    fn a_plugin_write_that_is_not_toml_is_invalid() {
        assert!(matches!(
            plugin_write("bind = [unclosed", PLUGIN_BLOCKS, "mine"),
            Err(PluginScopeError::Invalid(_))
        ));
    }

    #[test]
    fn still_redacted_ignores_a_real_value() {
        let d = doc("[web]\napi_key = \"a-real-and-long-key\"\n");
        assert!(still_redacted(&d, PROXY_SECRETS).is_empty());
    }
}
