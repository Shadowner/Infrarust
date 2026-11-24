use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use super::{CHECK_CONFIG_COMMAND, HandoverProbe, PROBE_FLAG};

const RUN_TIMEOUT: Duration = Duration::from_secs(10);
const OUTPUT_LIMIT: u64 = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("cannot run {path}: {source}")]
    Spawn {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} did not answer within {}s", RUN_TIMEOUT.as_secs())]
    Timeout { path: String },
    #[error("{path} exited with {status}")]
    Exit { path: String, status: ExitStatus },
    #[error("{path} gave an answer that cannot be read: {reason}")]
    Malformed { path: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigCheck {
    pub ok: bool,
    pub config: String,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

pub async fn probe(exe: &Path) -> Result<HandoverProbe, ProbeError> {
    let (status, stdout) = run(exe, [OsStr::new(PROBE_FLAG)]).await?;
    if !status.success() {
        return Err(ProbeError::Exit {
            path: exe.display().to_string(),
            status,
        });
    }
    parse_line(exe, &stdout)
}

pub async fn check_config(exe: &Path, args: &[OsString]) -> Result<ConfigCheck, ProbeError> {
    let tail = [OsStr::new(CHECK_CONFIG_COMMAND), OsStr::new("--json")];
    let argv = args.iter().map(OsString::as_os_str).chain(tail);
    let (status, stdout) = run(exe, argv).await?;
    match status.code() {
        Some(0 | 1) => parse_line(exe, &stdout),
        _ => Err(ProbeError::Exit {
            path: exe.display().to_string(),
            status,
        }),
    }
}

async fn run<'a>(
    exe: &Path,
    args: impl IntoIterator<Item = &'a OsStr>,
) -> Result<(ExitStatus, Vec<u8>), ProbeError> {
    let path = exe.display().to_string();
    let mut command = Command::new(exe);
    command
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = spawn(&mut command)
        .await
        .map_err(|source| ProbeError::Spawn {
            path: path.clone(),
            source,
        })?;
    let stdout = child.stdout.take();
    let answer = async {
        let mut output = Vec::new();
        if let Some(stdout) = stdout {
            stdout.take(OUTPUT_LIMIT).read_to_end(&mut output).await?;
        }
        let status = child.wait().await?;
        Ok::<_, std::io::Error>((status, output))
    };
    match tokio::time::timeout(RUN_TIMEOUT, answer).await {
        Ok(Ok(answer)) => Ok(answer),
        Ok(Err(source)) => Err(ProbeError::Spawn { path, source }),
        Err(_) => Err(ProbeError::Timeout { path }),
    }
}

async fn spawn(command: &mut Command) -> std::io::Result<tokio::process::Child> {
    const BUSY_RETRIES: u32 = 5;
    let mut attempt = 0;
    loop {
        match command.spawn() {
            Err(e)
                if e.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < BUSY_RETRIES =>
            {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            other => return other,
        }
    }
}

fn parse_line<T: serde::de::DeserializeOwned>(exe: &Path, stdout: &[u8]) -> Result<T, ProbeError> {
    let malformed = |reason: String| ProbeError::Malformed {
        path: exe.display().to_string(),
        reason,
    };
    let line = stdout
        .split(|byte| *byte == b'\n')
        .find(|line| !line.iter().all(u8::is_ascii_whitespace))
        .ok_or_else(|| malformed("empty output".to_string()))?;
    serde_json::from_slice(line).map_err(|e| malformed(e.to_string()))
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    use super::*;

    fn script(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("fake-infrarust");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[tokio::test]
    async fn probe_reads_the_first_json_line() {
        let dir = tempfile::tempdir().unwrap();
        let exe = script(
            dir.path(),
            r#"[ "$1" = "--handover-probe" ] || exit 9
echo '{"probe":1,"version":"2.0.0-beta.6","target":"x86_64-unknown-linux-musl","official":true,"docker":false,"handover":null}'"#,
        );
        let probe = probe(&exe).await.unwrap();
        assert_eq!(probe.version, "2.0.0-beta.6");
        assert!(probe.handover.is_none());
    }

    #[tokio::test]
    async fn probe_rejects_a_failing_binary() {
        let dir = tempfile::tempdir().unwrap();
        let exe = script(dir.path(), "exit 3");
        assert!(matches!(probe(&exe).await, Err(ProbeError::Exit { .. })));
    }

    #[tokio::test]
    async fn probe_rejects_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let exe = script(dir.path(), "echo not-json");
        assert!(matches!(
            probe(&exe).await,
            Err(ProbeError::Malformed { .. })
        ));
    }

    #[tokio::test]
    async fn probe_reports_a_missing_binary() {
        let missing = PathBuf::from("/nonexistent/infrarust");
        assert!(matches!(
            probe(&missing).await,
            Err(ProbeError::Spawn { .. })
        ));
    }

    #[tokio::test]
    async fn check_config_passes_the_original_arguments_first() {
        let dir = tempfile::tempdir().unwrap();
        let exe = script(
            dir.path(),
            r#"[ "$1" = "-c" ] && [ "$2" = "proxy.toml" ] && [ "$3" = "check-config" ] && [ "$4" = "--json" ] || exit 9
echo '{"ok":false,"config":"proxy.toml","warnings":[],"error":"bad bind"}'
exit 1"#,
        );
        let args = [OsString::from("-c"), OsString::from("proxy.toml")];
        let check = check_config(&exe, &args).await.unwrap();
        assert!(!check.ok);
        assert_eq!(check.error.as_deref(), Some("bad bind"));
    }

    #[tokio::test]
    async fn check_config_rejects_an_unexpected_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        let exe = script(dir.path(), "exit 2");
        assert!(matches!(
            check_config(&exe, &[]).await,
            Err(ProbeError::Exit { .. })
        ));
    }
}
