#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Cursor;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_api::services::{InstallRequest, InstallState, ProxyUpdates, RestartRequirement};
use infrarust_config::UpdateConfig;
use infrarust_core::binary::{BuildInfo, ExeSnapshot, RunningBinary};
use infrarust_core::update::{NoopObserver, TrustedKeys, UpdateError, UpdateService};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const TARGET: &str = "x86_64-unknown-linux-musl";
const ARCHIVE: &str = "infrarust-linux-x86_64.tar.gz";
const RUNNING: &str = "2.0.0-beta.5";
const NEXT: &str = "2.0.0-beta.6";

const BUILD: BuildInfo = BuildInfo {
    version: RUNNING,
    target: TARGET,
    official: true,
    docker: false,
};

type Routes = Arc<Mutex<HashMap<String, Vec<u8>>>>;

struct Mirror {
    base: String,
    routes: Routes,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Mirror {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes: Routes = Arc::default();
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let (table, log) = (Arc::clone(&routes), Arc::clone(&seen));
        tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let (table, log) = (Arc::clone(&table), Arc::clone(&log));
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                        if stream.read_buf(&mut head).await.unwrap() == 0 {
                            return;
                        }
                    }
                    let head = String::from_utf8_lossy(&head).into_owned();
                    log.lock().unwrap().push(head.clone());
                    let target = head.split(' ').nth(1).unwrap_or("/");
                    let path = target.split('?').next().unwrap_or("/");
                    let body = table.lock().unwrap().get(path).cloned();
                    let (status, body) = match body {
                        Some(body) => ("200 OK", body),
                        None => ("404 Not Found", Vec::new()),
                    };
                    let mut response = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    response.extend_from_slice(&body);
                    let _ = stream.write_all(&response).await;
                });
            }
        });
        Self { base, routes, seen }
    }

    fn serve(&self, path: &str, body: impl Into<Vec<u8>>) -> String {
        self.routes
            .lock()
            .unwrap()
            .insert(path.to_string(), body.into());
        format!("{}{path}", self.base)
    }

    fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

fn fake_binary(version: &str) -> Vec<u8> {
    let probe = serde_json::json!({
        "probe": 1,
        "version": version,
        "target": TARGET,
        "official": true,
        "docker": false,
        "handover": null,
    });
    let config = serde_json::json!({
        "ok": true,
        "config": "proxy.toml",
        "warnings": [],
        "error": null,
    });
    format!(
        "#!/bin/sh\nfor arg in \"$@\"; do\n  case \"$arg\" in\n    --handover-probe) echo '{probe}'; exit 0 ;;\n    check-config) echo '{config}'; exit 0 ;;\n  esac\ndone\nexit 2\n"
    )
    .into_bytes()
}

fn tar_gz(name: &str, content: &[u8]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(gz);
    let mut header = tar::Header::new_gnu();
    header.set_path(name).unwrap();
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder.append(&header, content).unwrap();
    builder.into_inner().unwrap().finish().unwrap()
}

fn sha256(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

struct Release {
    key: minisign::KeyPair,
    new_binary: Vec<u8>,
}

fn publish(mirror: &Mirror) -> Release {
    let key = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let new_binary = fake_binary(NEXT);
    let archive = tar_gz("./infrarust", &new_binary);
    let manifest = serde_json::json!({
        "schema": 1,
        "version": NEXT,
        "tag": format!("v{NEXT}"),
        "published_at": "2026-11-24T10:00:00Z",
        "notes_url": format!("https://github.com/Shadowner/Infrarust/releases/tag/v{NEXT}"),
        "security": true,
        "handover": null,
        "artifacts": [{
            "targets": [TARGET, "x86_64-unknown-linux-gnu"],
            "file": ARCHIVE,
            "binary": "infrarust",
            "sha256": sha256(&archive),
            "size": archive.len(),
        }],
    })
    .to_string();
    let signature = minisign::sign(
        Some(&key.pk),
        &key.sk,
        Cursor::new(manifest.as_bytes()),
        None,
        None,
    )
    .unwrap()
    .into_string();
    let tag = format!("v{NEXT}");
    let asset = |name: &str, body: Vec<u8>| {
        let url = mirror.serve(&format!("/download/{tag}/{name}"), body.clone());
        serde_json::json!({"name": name, "browser_download_url": url, "size": body.len()})
    };
    let releases = serde_json::json!([
        {
            "tag_name": tag,
            "draft": false,
            "prerelease": false,
            "published_at": "2026-11-24T10:00:00Z",
            "html_url": format!("https://github.com/Shadowner/Infrarust/releases/tag/{tag}"),
            "assets": [
                asset("manifest.json", manifest.into_bytes()),
                asset("manifest.json.minisig", signature.into_bytes()),
                asset("SHA256SUMS", format!("{}  {ARCHIVE}\n", sha256(&archive)).into_bytes()),
                asset(ARCHIVE, archive),
            ],
        },
        {
            "tag_name": "v2.0.0-beta.4",
            "assets": [],
        },
    ]);
    mirror.serve("/repos/Shadowner/Infrarust/releases", releases.to_string());
    Release { key, new_binary }
}

struct Install {
    _dir: tempfile::TempDir,
    exe: std::path::PathBuf,
    old: std::path::PathBuf,
    current: Vec<u8>,
    service: Arc<UpdateService>,
}

fn install_dir(mirror: &Mirror, release: &Release) -> Install {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("infrarust");
    let current = fake_binary(RUNNING);
    std::fs::write(&exe, &current).unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    let binary = RunningBinary::new(
        BUILD,
        ExeSnapshot::at(exe.clone()),
        vec![OsString::from("-c"), OsString::from("proxy.toml")],
    );
    let config = UpdateConfig {
        check: false,
        url: format!("{}/repos/Shadowner/Infrarust/releases", mirror.base),
        ..UpdateConfig::default()
    };
    let keys = TrustedKeys::from_strs([release.key.pk.to_box().unwrap().into_string()]).unwrap();
    let service = UpdateService::with_keys(config, binary, None, keys);
    Install {
        old: dir.path().join("infrarust.old"),
        _dir: dir,
        exe,
        current,
        service,
    }
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

#[tokio::test]
async fn install_then_roll_back_against_a_local_mirror() {
    let mirror = Mirror::start().await;
    let release = publish(&mirror);
    let site = install_dir(&mirror, &release);
    let original_inode = std::fs::metadata(&site.exe).unwrap().ino();

    let available = site.service.check(None).await.unwrap().unwrap();
    assert_eq!(available.version, NEXT);
    assert!(available.security);
    assert_eq!(available.restart, RestartRequirement::Full);

    let installed = site
        .service
        .install(InstallRequest::default(), &NoopObserver)
        .await
        .unwrap();
    assert_eq!(installed.version.to_string(), NEXT);
    assert!(installed.security);
    assert_eq!(read(&site.exe), release.new_binary);
    assert_eq!(read(&site.old), site.current);
    assert_eq!(std::fs::metadata(&site.old).unwrap().ino(), original_inode);
    assert_eq!(
        std::fs::metadata(&site.exe).unwrap().permissions().mode() & 0o777,
        0o755
    );
    let status = site.service.status();
    assert_eq!(
        status.install,
        InstallState::Installed {
            version: NEXT.to_string()
        }
    );
    assert!(status.binary_changed);

    let back = site.service.rollback().await.unwrap();
    assert_eq!(back.version, RUNNING);
    assert_eq!(read(&site.exe), site.current);
    assert_eq!(read(&site.old), release.new_binary);
    assert!(!site.service.status().binary_changed);

    let forward = site.service.rollback().await.unwrap();
    assert_eq!(forward.version, NEXT);
    assert_eq!(read(&site.exe), release.new_binary);

    let seen = mirror.seen();
    let list = seen
        .iter()
        .find(|head| head.starts_with("GET /repos/Shadowner/Infrarust/releases?"))
        .expect("the release list was requested");
    assert!(list.contains("per_page=20"), "{list}");
    assert!(
        list.to_ascii_lowercase()
            .contains(&format!("user-agent: infrarust/{RUNNING}")),
        "{list}"
    );
    assert!(seen.iter().all(|head| !head.contains("SHA256SUMS")));
}

#[tokio::test]
async fn the_plugin_service_installs_in_the_background() {
    let mirror = Mirror::start().await;
    let release = publish(&mirror);
    let site = install_dir(&mirror, &release);
    let updates: Arc<dyn ProxyUpdates> = site.service.clone();

    assert!(updates.status().available.is_none());
    let status = updates.check_now().await;
    assert_eq!(status.available.unwrap().version, NEXT);
    updates.start_install(InstallRequest::default()).unwrap();

    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let status = loop {
        let status = updates.status();
        if matches!(
            status.install,
            InstallState::Installed { .. } | InstallState::Failed { .. }
        ) {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the install never ended"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(
        status.install,
        InstallState::Installed {
            version: NEXT.to_string()
        }
    );
    assert_eq!(read(&site.exe), release.new_binary);
}

#[tokio::test]
async fn a_manifest_signed_by_another_key_is_never_installed() {
    let mirror = Mirror::start().await;
    let release = publish(&mirror);
    let impostor = Release {
        key: minisign::KeyPair::generate_unencrypted_keypair().unwrap(),
        new_binary: release.new_binary.clone(),
    };
    let site = install_dir(&mirror, &impostor);

    assert!(site.service.check(None).await.unwrap().is_none());
    let pinned = site
        .service
        .install(
            InstallRequest {
                version: Some(NEXT.to_string()),
                then_restart: false,
            },
            &NoopObserver,
        )
        .await;
    assert!(
        matches!(pinned, Err(UpdateError::Signature { .. })),
        "{pinned:?}"
    );
    assert_eq!(read(&site.exe), site.current);
    assert!(!site.old.exists());
}
