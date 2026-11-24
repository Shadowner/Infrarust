use infrarust_api::BoxFuture;

use super::platform::RestartPlan;
use crate::binary::{self, HANDOVER_PROTOCOL, HandoverProbe, ProbeError};

pub trait Preflight: Send + Sync {
    fn check<'a>(
        &'a self,
        plan: &'a RestartPlan,
    ) -> BoxFuture<'a, Result<HandoverProbe, PreflightError>>;
}

#[derive(Debug, thiserror::Error)]
pub enum PreflightError {
    #[error("the new binary cannot be probed: {0}")]
    Probe(ProbeError),
    #[error(
        "{path} ({version}) cannot take over live sessions; restart the proxy by hand to run it"
    )]
    NoHandover { path: String, version: String },
    #[error(
        "{path} ({version}) accepts handover protocols {accepts:?}, but this proxy hands over protocol {written}; restart the proxy by hand to run it"
    )]
    Protocol {
        path: String,
        version: String,
        written: u32,
        accepts: Vec<u32>,
    },
    #[error(
        "{path} is built for {found}, but this proxy runs as {running}; install the {running} build"
    )]
    Target {
        path: String,
        found: String,
        running: String,
    },
    #[error("the new binary cannot check the configuration: {0}")]
    ConfigCheck(ProbeError),
    #[error("{path} rejects the configuration {config}: {reason}; fix it, then restart again")]
    Config {
        path: String,
        config: String,
        reason: String,
    },
    #[error(
        "{} run as child processes of the proxy and would be orphaned by a hot restart; stop them first",
        .0.join(", ")
    )]
    ChildServers(Vec<String>),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BinaryPreflight;

impl Preflight for BinaryPreflight {
    fn check<'a>(
        &'a self,
        plan: &'a RestartPlan,
    ) -> BoxFuture<'a, Result<HandoverProbe, PreflightError>> {
        Box::pin(check_binary(plan))
    }
}

async fn check_binary(plan: &RestartPlan) -> Result<HandoverProbe, PreflightError> {
    let target = plan.target.as_path();
    let path = target.display().to_string();
    let probe = binary::probe(target).await.map_err(PreflightError::Probe)?;
    let Some(support) = &probe.handover else {
        return Err(PreflightError::NoHandover {
            path,
            version: probe.version,
        });
    };
    if !support.accepts(HANDOVER_PROTOCOL) {
        return Err(PreflightError::Protocol {
            path,
            version: probe.version.clone(),
            written: HANDOVER_PROTOCOL,
            accepts: support.accepts.clone(),
        });
    }
    let running = plan.binary.build.target;
    if probe.target != running {
        return Err(PreflightError::Target {
            path,
            found: probe.target,
            running: running.to_string(),
        });
    }
    let check = binary::check_config(target, &plan.binary.args)
        .await
        .map_err(PreflightError::ConfigCheck)?;
    if !check.ok {
        return Err(PreflightError::Config {
            path,
            config: check.config,
            reason: check.error.unwrap_or_else(|| "no reason given".to_string()),
        });
    }
    for warning in &check.warnings {
        tracing::warn!(target_binary = %path, %warning, "the new binary warns about the configuration");
    }
    Ok(probe)
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::ffi::OsString;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use super::*;
    use crate::binary::{BuildInfo, ExeSnapshot, RunningBinary};

    const RUNNING_TARGET: &str = "x86_64-unknown-linux-musl";

    struct Fake {
        target: &'static str,
        handover: &'static str,
        config: &'static str,
    }

    impl Fake {
        const fn ready() -> Self {
            Self {
                target: RUNNING_TARGET,
                handover: r#"{"protocol":1,"accepts":[1]}"#,
                config: r#"echo '{"ok":true,"config":"proxy.toml","warnings":[],"error":null}'"#,
            }
        }

        fn install(&self, dir: &Path) -> PathBuf {
            let probe = format!(
                r#"{{"probe":1,"version":"2.0.0-beta.7","target":"{}","official":true,"docker":false,"handover":{}}}"#,
                self.target, self.handover
            );
            let script = format!(
                "#!/bin/sh\nif [ \"$1\" = \"--handover-probe\" ]; then\n  echo '{probe}'\n  exit 0\nfi\n[ \"$1\" = \"-c\" ] && [ \"$2\" = \"proxy.toml\" ] && [ \"$3\" = \"check-config\" ] && [ \"$4\" = \"--json\" ] || exit 9\n{}\n",
                self.config
            );
            let path = dir.join("infrarust-next");
            std::fs::write(&path, script).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        }
    }

    fn plan(target: PathBuf) -> RestartPlan {
        let build = BuildInfo {
            version: "2.0.0-beta.6",
            target: RUNNING_TARGET,
            official: true,
            docker: false,
        };
        let args = vec![OsString::from("-c"), OsString::from("proxy.toml")];
        RestartPlan {
            binary: RunningBinary::new(build, ExeSnapshot::at(target.clone()), args),
            target,
        }
    }

    async fn check(fake: &Fake) -> Result<HandoverProbe, PreflightError> {
        let dir = tempfile::tempdir().unwrap();
        let plan = plan(fake.install(dir.path()));
        let preflight: Arc<dyn Preflight> = Arc::new(BinaryPreflight);
        preflight.check(&plan).await
    }

    #[tokio::test]
    async fn a_ready_binary_passes_and_reports_its_probe() {
        let probe = check(&Fake::ready()).await.unwrap();
        assert_eq!(probe.version, "2.0.0-beta.7");
        assert_eq!(probe.target, RUNNING_TARGET);
        assert!(probe.handover.unwrap().accepts(HANDOVER_PROTOCOL));
    }

    #[tokio::test]
    async fn a_binary_without_handover_support_is_refused() {
        let fake = Fake {
            handover: "null",
            ..Fake::ready()
        };
        let error = check(&fake).await.unwrap_err();
        assert!(
            matches!(&error, PreflightError::NoHandover { version, .. } if version == "2.0.0-beta.7"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_binary_that_does_not_accept_our_protocol_is_refused() {
        let fake = Fake {
            handover: r#"{"protocol":2,"accepts":[2]}"#,
            ..Fake::ready()
        };
        let error = check(&fake).await.unwrap_err();
        let PreflightError::Protocol {
            written, accepts, ..
        } = &error
        else {
            panic!("expected a protocol refusal, got {error}");
        };
        assert_eq!(*written, HANDOVER_PROTOCOL);
        assert_eq!(accepts, &[2]);
    }

    #[tokio::test]
    async fn a_binary_that_accepts_our_protocol_among_others_passes() {
        let fake = Fake {
            handover: r#"{"protocol":2,"accepts":[1,2]}"#,
            ..Fake::ready()
        };
        check(&fake).await.unwrap();
    }

    #[tokio::test]
    async fn a_binary_built_for_another_target_is_refused() {
        let fake = Fake {
            target: "aarch64-unknown-linux-musl",
            ..Fake::ready()
        };
        let error = check(&fake).await.unwrap_err();
        assert!(
            matches!(&error, PreflightError::Target { found, running, .. }
                if found == "aarch64-unknown-linux-musl" && running == RUNNING_TARGET),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_binary_rejecting_the_configuration_is_refused_with_its_reason() {
        let fake = Fake {
            config: r#"echo '{"ok":false,"config":"proxy.toml","warnings":[],"error":"unknown field `update`"}'
exit 1"#,
            ..Fake::ready()
        };
        let error = check(&fake).await.unwrap_err();
        assert!(
            matches!(&error, PreflightError::Config { config, reason, .. }
                if config == "proxy.toml" && reason == "unknown field `update`"),
            "{error}"
        );
        assert!(
            error.to_string().contains("unknown field `update`"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_binary_whose_config_check_crashes_is_refused() {
        let fake = Fake {
            config: "exit 3",
            ..Fake::ready()
        };
        let error = check(&fake).await.unwrap_err();
        assert!(
            matches!(error, PreflightError::ConfigCheck(ProbeError::Exit { .. })),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_missing_binary_cannot_be_probed() {
        let plan = plan(PathBuf::from("/nonexistent/infrarust"));
        let error = BinaryPreflight.check(&plan).await.unwrap_err();
        assert!(
            matches!(error, PreflightError::Probe(ProbeError::Spawn { .. })),
            "{error}"
        );
    }

    #[test]
    fn child_servers_are_named_in_the_refusal() {
        let error = PreflightError::ChildServers(vec!["lobby".into(), "survival".into()]);
        assert!(
            error
                .to_string()
                .starts_with("lobby, survival run as child processes")
        );
    }
}
