use infrarust_config::{MotdConfig, MotdEntry, ProxyConfig, ServerConfig};
use infrarust_server_manager::ServerState;

pub const DEFAULT_PROXY_MOTD: &str = "An Infrarust Proxy";

pub fn state_motd(motd: &MotdConfig, state: ServerState) -> (Option<&MotdEntry>, &'static str) {
    match state {
        ServerState::Sleeping => (
            motd.sleeping.as_ref(),
            "\u{00a7}7Server sleeping \u{2014} \u{00a7}aConnect to wake up!",
        ),
        ServerState::Starting => (motd.starting.as_ref(), "\u{00a7}eServer is starting..."),
        ServerState::Crashed => (motd.crashed.as_ref(), "\u{00a7}cServer unavailable"),
        ServerState::Stopping => (motd.stopping.as_ref(), "\u{00a7}6Server is stopping..."),
        _ => (None, "A Minecraft Server"),
    }
}

pub fn state_max_players(entry: Option<&MotdEntry>, config: &ServerConfig) -> i32 {
    entry
        .and_then(|entry| entry.max_players)
        .unwrap_or(config.max_players)
        .cast_signed()
}

pub fn default_entry(config: &ProxyConfig) -> Option<&MotdEntry> {
    config
        .default_motd
        .as_ref()
        .and_then(|motd| motd.online.as_ref())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn entry(text: &str, max_players: Option<u32>) -> MotdEntry {
        MotdEntry {
            text: text.into(),
            favicon: None,
            version_name: None,
            max_players,
        }
    }

    #[test]
    fn every_managed_state_has_its_own_entry_and_fallback_text() {
        let motd = MotdConfig {
            online: Some(entry("on", None)),
            sleeping: Some(entry("sleep", None)),
            starting: Some(entry("start", None)),
            crashed: Some(entry("crash", None)),
            stopping: Some(entry("stop", None)),
            unreachable: Some(entry("gone", None)),
        };
        let texts: Vec<&str> = [
            ServerState::Sleeping,
            ServerState::Starting,
            ServerState::Crashed,
            ServerState::Stopping,
        ]
        .into_iter()
        .map(|state| state_motd(&motd, state).0.unwrap().text.as_str())
        .collect();
        assert_eq!(texts, ["sleep", "start", "crash", "stop"]);
        assert!(state_motd(&motd, ServerState::Online).0.is_none());
        assert_eq!(
            state_motd(&MotdConfig::default(), ServerState::Sleeping),
            (
                None,
                "\u{00a7}7Server sleeping \u{2014} \u{00a7}aConnect to wake up!"
            )
        );
    }

    #[test]
    fn max_players_falls_back_to_the_server_limit() {
        let mut config: ServerConfig = toml::from_str(
            "name = \"lobby\"\ndomains = [\"lobby.test\"]\naddresses = [\"127.0.0.1:25566\"]\nmax_players = 42\n",
        )
        .unwrap();
        assert_eq!(state_max_players(None, &config), 42);
        assert_eq!(state_max_players(Some(&entry("x", Some(7))), &config), 7);
        assert_eq!(state_max_players(Some(&entry("x", None)), &config), 42);
        config.max_players = 0;
        assert_eq!(state_max_players(None, &config), 0);
    }
}
