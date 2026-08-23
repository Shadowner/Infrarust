//! Secret fields of a configuration document.
//!
//! Documents leave the proxy redacted and come back for a write. A redacted
//! field keeps its key and carries [`REDACTED`] instead of its value, so the
//! document stays a valid instance of its schema — `server_manager.api_key` is
//! a required field — and a client that saves an untouched document restores
//! the live secret through [`reinject`] instead of overwriting it.

use toml_edit::{DocumentMut, TableLike};

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
                    .and_then(toml_edit::Item::as_str)
                    == Some(REDACTED)
            })
        })
        .map(|path| path.join("."))
        .collect()
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
    #![allow(clippy::unwrap_used)]
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

    #[test]
    fn still_redacted_ignores_a_real_value() {
        let d = doc("[web]\napi_key = \"a-real-and-long-key\"\n");
        assert!(still_redacted(&d, PROXY_SECRETS).is_empty());
    }
}
