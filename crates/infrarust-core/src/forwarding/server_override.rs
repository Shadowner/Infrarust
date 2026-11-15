use infrarust_config::{ForwardingMode as ConfigForwardingMode, ServerConfig};

use super::{ForwardingError, ForwardingHandler, ForwardingMode, build_forwarding_handler};

pub(crate) fn resolve_forwarding_handler(
    global: &ForwardingMode,
    server_config: &ServerConfig,
) -> Result<ForwardingHandler, ForwardingError> {
    if let Some(ref server_override) = server_config.forwarding_mode {
        let mode = config_to_core_mode(server_override, global)?;
        Ok(build_forwarding_handler(&mode))
    } else {
        Ok(build_forwarding_handler(global))
    }
}

fn config_to_core_mode(
    config_mode: &ConfigForwardingMode,
    global: &ForwardingMode,
) -> Result<ForwardingMode, ForwardingError> {
    match (config_mode, global) {
        (ConfigForwardingMode::None, _) => Ok(ForwardingMode::None),
        (ConfigForwardingMode::BungeeCord, _) => {
            tracing::warn!(
                "A server overrides forwarding to BungeeCord legacy mode. \
                 This mode is fundamentally insecure — anyone reaching the backend \
                 directly can impersonate any player."
            );
            Ok(ForwardingMode::BungeeCord)
        }
        (ConfigForwardingMode::BungeeGuard, ForwardingMode::BungeeGuard { token }) => {
            Ok(ForwardingMode::BungeeGuard {
                token: token.clone(),
            })
        }
        (ConfigForwardingMode::Velocity, ForwardingMode::Velocity { secret }) => {
            Ok(ForwardingMode::Velocity {
                secret: secret.clone(),
            })
        }
        (
            ConfigForwardingMode::BungeeGuard,
            ForwardingMode::None | ForwardingMode::BungeeCord | ForwardingMode::Velocity { .. },
        ) => Err(ForwardingError::SecretMismatch {
            requested: "bungee_guard",
        }),
        (
            ConfigForwardingMode::Velocity,
            ForwardingMode::None | ForwardingMode::BungeeCord | ForwardingMode::BungeeGuard { .. },
        ) => Err(ForwardingError::SecretMismatch {
            requested: "velocity",
        }),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn velocity() -> ForwardingMode {
        ForwardingMode::Velocity {
            secret: b"velocity-secret".to_vec(),
        }
    }

    fn bungee_guard() -> ForwardingMode {
        ForwardingMode::BungeeGuard {
            token: "token".to_string(),
        }
    }

    #[test]
    fn a_velocity_secret_never_becomes_a_bungee_guard_token() {
        assert!(matches!(
            config_to_core_mode(&ConfigForwardingMode::BungeeGuard, &velocity()),
            Err(ForwardingError::SecretMismatch {
                requested: "bungee_guard"
            })
        ));
    }

    #[test]
    fn an_override_without_its_secret_is_refused_instead_of_dropping_forwarding() {
        for (requested, global) in [
            (ConfigForwardingMode::BungeeGuard, ForwardingMode::None),
            (
                ConfigForwardingMode::BungeeGuard,
                ForwardingMode::BungeeCord,
            ),
            (ConfigForwardingMode::Velocity, ForwardingMode::None),
            (ConfigForwardingMode::Velocity, ForwardingMode::BungeeCord),
            (ConfigForwardingMode::Velocity, bungee_guard()),
        ] {
            assert!(
                matches!(
                    config_to_core_mode(&requested, &global),
                    Err(ForwardingError::SecretMismatch { .. })
                ),
                "{requested:?}"
            );
        }
    }

    #[test]
    fn an_override_keeps_the_secret_of_its_own_mode() {
        assert_eq!(
            config_to_core_mode(&ConfigForwardingMode::Velocity, &velocity()).unwrap(),
            velocity()
        );
        assert_eq!(
            config_to_core_mode(&ConfigForwardingMode::BungeeGuard, &bungee_guard()).unwrap(),
            bungee_guard()
        );
    }

    #[test]
    fn overrides_without_a_secret_work_under_any_global_mode() {
        for global in [
            ForwardingMode::None,
            ForwardingMode::BungeeCord,
            bungee_guard(),
            velocity(),
        ] {
            assert_eq!(
                config_to_core_mode(&ConfigForwardingMode::None, &global).unwrap(),
                ForwardingMode::None
            );
            assert_eq!(
                config_to_core_mode(&ConfigForwardingMode::BungeeCord, &global).unwrap(),
                ForwardingMode::BungeeCord
            );
        }
    }
}
