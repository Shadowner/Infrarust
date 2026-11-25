#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const BIN: &str = env!("CARGO_BIN_EXE_infrarust");
const VERSION: &str = env!("CARGO_PKG_VERSION");

async fn release_server(status: &'static str, body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/releases", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut head = Vec::new();
                while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                    if stream.read_buf(&mut head).await.unwrap() == 0 {
                        return;
                    }
                }
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    url
}

struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("update-cli-")
            .tempdir_in(
                Path::new(env!("CARGO_TARGET_TMPDIR"))
                    .canonicalize()
                    .unwrap(),
            )
            .unwrap();
        std::fs::create_dir(dir.path().join("servers")).unwrap();
        Self { dir }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn config(&self, update_url: &str) -> PathBuf {
        let path = self.path("infrarust.toml");
        let servers = self.path("servers");
        std::fs::write(
            &path,
            format!(
                "servers_dir = {:?}\n\n[update]\nurl = {update_url:?}\n",
                servers.display().to_string()
            ),
        )
        .unwrap();
        path
    }

    fn broken_config(&self) -> PathBuf {
        let path = self.path("infrarust.toml");
        std::fs::write(&path, "bind = [this is not toml\n").unwrap();
        path
    }

    fn linked_binary(&self) -> PathBuf {
        let exe = self.path("infrarust");
        if std::fs::hard_link(BIN, &exe).is_err() {
            std::fs::copy(BIN, &exe).unwrap();
        }
        exe
    }

    fn previous_binary(&self, version: &str) -> PathBuf {
        let old = self.path("infrarust.old");
        let args = self.path("check-config-args");
        let probe = format!(
            r#"{{"probe":1,"version":"{version}","target":"x86_64-unknown-linux-gnu","official":true,"docker":false,"handover":null}}"#
        );
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = \"--handover-probe\" ]; then\n  printf '%s\\n' '{probe}'\n  exit 0\nfi\nprintf '%s\\n' \"$@\" > '{}'\nprintf '%s\\n' '{{\"ok\":true,\"config\":\"infrarust.toml\",\"warnings\":[],\"error\":null}}'\n",
            args.display()
        );
        std::fs::write(&old, script).unwrap();
        std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o755)).unwrap();
        old
    }

    fn recorded_args(&self) -> Vec<String> {
        std::fs::read_to_string(self.path("check-config-args"))
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

async fn run(exe: &Path, args: &[&str]) -> Output {
    let exe = exe.to_path_buf();
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    tokio::task::spawn_blocking(move || {
        Command::new(exe)
            .args(args)
            .env_remove("RUST_LOG")
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    })
    .await
    .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn json_line(output: &Output) -> Value {
    let text = stdout(output);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1, "one JSON line expected, got {text:?}");
    serde_json::from_str(lines[0]).unwrap()
}

fn is_official_build() -> bool {
    let output = Command::new(BIN).arg("--handover-probe").output().unwrap();
    json_line(&output)["official"] == Value::Bool(true)
}

const NO_TRUSTED_KEY: &str = "no trusted release key is built into this binary";

fn stopped_without_a_release_key(output: &Output) -> bool {
    let report = json_line(output);
    if report["last_error"] != NO_TRUSTED_KEY {
        return false;
    }
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(report["current_version"], VERSION);
    assert_eq!(report["available"], Value::Null);
    true
}

#[tokio::test]
async fn check_json_reports_an_empty_release_list_as_up_to_date() {
    let sandbox = Sandbox::new();
    let config = sandbox.config(&release_server("200 OK", "[]").await);
    let config = config.to_str().unwrap();

    let output = run(Path::new(BIN), &["-c", config, "update", "check", "--json"]).await;
    if stopped_without_a_release_key(&output) {
        return;
    }
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let report = json_line(&output);
    assert_eq!(report["current_version"], VERSION);
    assert_eq!(report["available"], Value::Null);
    assert_eq!(report["last_error"], Value::Null);
    assert!(report["last_check"].is_string(), "{report}");
    assert!(report.get("install_refusal").is_some(), "{report}");

    let output = run(Path::new(BIN), &["-c", config, "update", "check"]).await;
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!("current version: {VERSION}\nup to date\n")
    );
}

#[tokio::test]
async fn check_exits_with_1_and_the_error_when_the_release_list_fails() {
    let sandbox = Sandbox::new();
    let config = sandbox.config(&release_server("500 Internal Server Error", "").await);
    let config = config.to_str().unwrap();

    let output = run(
        Path::new(BIN),
        &[
            "-c",
            config,
            "update",
            "check",
            "--json",
            "--channel",
            "stable",
        ],
    )
    .await;
    if stopped_without_a_release_key(&output) {
        return;
    }
    assert_eq!(output.status.code(), Some(1));
    let report = json_line(&output);
    let error = report["last_error"].as_str().unwrap();
    assert!(error.contains("HTTP 500"), "{error}");
    assert_eq!(report["available"], Value::Null);

    let output = run(Path::new(BIN), &["-c", config, "update", "check"]).await;
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("error: "), "{}", stderr(&output));
    assert!(stderr(&output).contains("HTTP 500"), "{}", stderr(&output));
}

#[tokio::test]
async fn check_refuses_a_broken_config() {
    let sandbox = Sandbox::new();
    let config = sandbox.broken_config();

    let output = run(
        Path::new(BIN),
        &["-c", config.to_str().unwrap(), "update", "check", "--json"],
    )
    .await;
    assert_eq!(output.status.code(), Some(1));
    let error = json_line(&output)["last_error"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(error.contains("invalid TOML"), "{error}");
}

#[tokio::test]
async fn install_on_a_build_outside_the_release_pipeline_gives_the_cargo_command() {
    if is_official_build() {
        return;
    }
    let output = run(
        Path::new(BIN),
        &[
            "-c",
            "missing.toml",
            "update",
            "install",
            "--version",
            "2.0.0-beta.9",
            "--yes",
        ],
    )
    .await;
    assert_eq!(output.status.code(), Some(1));
    let text = stderr(&output);
    assert!(text.contains("not made by the release pipeline"), "{text}");
    assert!(
        text.contains("`cargo install infrarust --version 2.0.0-beta.9`"),
        "{text}"
    );

    let output = run(Path::new(BIN), &["update", "install"]).await;
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("`cargo install infrarust`"),
        "{}",
        stderr(&output)
    );
    assert!(stdout(&output).is_empty());
}

#[tokio::test]
async fn rollback_without_a_previous_binary_changes_nothing() {
    let sandbox = Sandbox::new();
    let exe = sandbox.linked_binary();

    let output = run(&exe, &["update", "rollback", "--yes"]).await;
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("there is no previous binary to roll back to"),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        std::fs::metadata(&exe).unwrap().ino(),
        std::fs::metadata(BIN).unwrap().ino()
    );
}

#[tokio::test]
async fn rollback_without_yes_and_without_a_terminal_is_refused() {
    let sandbox = Sandbox::new();
    let exe = sandbox.linked_binary();
    sandbox.previous_binary("9.9.9");

    let output = run(&exe, &["update", "rollback"]).await;
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("--yes"), "{}", stderr(&output));
    assert!(!sandbox.path("check-config-args").exists());
}

#[tokio::test]
async fn rollback_goes_on_with_a_broken_config_and_passes_the_global_options() {
    let sandbox = Sandbox::new();
    let exe = sandbox.linked_binary();
    let old = sandbox.previous_binary("9.9.9");
    let script = std::fs::read(&old).unwrap();
    let config = sandbox.broken_config();
    let config = config.to_str().unwrap();

    let output = run(
        &exe,
        &[
            "-c",
            config,
            "--bind",
            "127.0.0.1:25599",
            "--servers-dir",
            "/srv/mc",
            "-l",
            "debug",
            "update",
            "rollback",
            "-y",
        ],
    )
    .await;
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(stderr(&output).contains("warning: "), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.starts_with(&format!(
            "restored Infrarust 9.9.9 at {}, the replaced binary is kept at {}\n",
            exe.display(),
            old.display()
        )),
        "{text}"
    );
    assert!(text.contains("restart Infrarust to run it"), "{text}");

    assert_eq!(
        sandbox.recorded_args(),
        [
            "-c",
            config,
            "--bind",
            "127.0.0.1:25599",
            "--servers-dir",
            "/srv/mc",
            "check-config",
            "--json",
        ]
    );
    assert_eq!(std::fs::read(&exe).unwrap(), script);
    assert_eq!(
        std::fs::metadata(&old).unwrap().ino(),
        std::fs::metadata(BIN).unwrap().ino()
    );
}
