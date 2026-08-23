#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::io::Write;

use infrarust_config::LocalManagerConfig;
use infrarust_server_manager::{LocalProvider, ProviderStatus, ServerProvider};

fn make_mock_script(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    {
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.sync_all().unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let f = std::fs::File::open(&path).unwrap();
        f.sync_all().unwrap();
    }
    path
}

fn make_config(script_path: &std::path::Path, working_dir: &std::path::Path) -> LocalManagerConfig {
    LocalManagerConfig {
        command: "bash".to_string(),
        working_dir: working_dir.to_path_buf(),
        args: vec![script_path.to_str().unwrap().to_string()],
        ready_pattern: r#"For help, type "help""#.to_string(),
        shutdown_timeout: std::time::Duration::from_secs(5),
        shutdown_after: None,
        start_timeout: std::time::Duration::from_secs(10),
    }
}

#[tokio::test]
async fn test_check_status_not_started() {
    let dir = tempfile::tempdir().unwrap();
    let script = make_mock_script(dir.path(), "server.sh", "#!/bin/bash\necho done\n");
    let config = make_config(&script, dir.path());
    let provider = LocalProvider::new(config);

    let status = provider.check_status().await.unwrap();
    assert_eq!(status, ProviderStatus::Stopped);
}

#[tokio::test]
async fn test_start_detects_ready_pattern() {
    let dir = tempfile::tempdir().unwrap();
    let script = make_mock_script(
        dir.path(),
        "server.sh",
        r#"#!/bin/bash
echo "Loading..."
sleep 0.2
echo 'For help, type "help"'
# Wait for stop on stdin
while read -r line; do
    if [ "$line" = "stop" ]; then
        exit 0
    fi
done
"#,
    );
    let config = make_config(&script, dir.path());
    let provider = LocalProvider::new(config);

    provider.start().await.unwrap();

    // Wait for the ready pattern to be detected
    let mut detected = false;
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let status = provider.check_status().await.unwrap();
        if status == ProviderStatus::Running {
            detected = true;
            break;
        }
    }
    assert!(detected, "ready pattern should have been detected");

    // Stop the process
    provider.stop().await.unwrap();

    let status = provider.check_status().await.unwrap();
    assert_eq!(status, ProviderStatus::Stopped);
}

#[cfg(target_os = "linux")]
fn process_is_alive(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat
            .rsplit(')')
            .next()
            .and_then(|rest| rest.split_whitespace().next())
            .is_some_and(|state| state != "Z"),
        Err(_) => false,
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn test_child_process_killed_when_provider_dropped() {
    use std::time::Duration;

    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("server.pid");
    let script_body = format!(
        "#!/bin/bash\necho $$ > {}\necho 'For help, type \"help\"'\nwhile true; do sleep 1; done\n",
        pid_file.display()
    );
    let script = make_mock_script(dir.path(), "server.sh", &script_body);
    let config = make_config(&script, dir.path());

    let provider = LocalProvider::new(config);
    provider.start().await.unwrap();

    let mut pid = None;
    for _ in 0..100 {
        if let Ok(s) = std::fs::read_to_string(&pid_file)
            && let Ok(p) = s.trim().parse::<u32>()
        {
            pid = Some(p);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let pid = pid.expect("script should record its PID");
    assert!(process_is_alive(pid), "process must be alive before drop");

    drop(provider);

    let mut killed = false;
    for _ in 0..100 {
        if !process_is_alive(pid) {
            killed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        killed,
        "child process {pid} must be killed when the provider is dropped without stop()"
    );
}

#[tokio::test]
async fn test_check_status_starting() {
    let dir = tempfile::tempdir().unwrap();
    // Script that never outputs the ready pattern
    let script = make_mock_script(
        dir.path(),
        "server.sh",
        r#"#!/bin/bash
echo "Loading..."
# Wait forever
while read -r line; do
    if [ "$line" = "stop" ]; then
        exit 0
    fi
done
"#,
    );
    let config = make_config(&script, dir.path());
    let provider = LocalProvider::new(config);

    provider.start().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let status = provider.check_status().await.unwrap();
    assert_eq!(status, ProviderStatus::Starting);

    // Cleanup
    provider.stop().await.unwrap();
}

#[tokio::test]
async fn test_unexpected_exit_code_reports_crashed() {
    let dir = tempfile::tempdir().unwrap();
    let script = make_mock_script(dir.path(), "server.sh", "#!/bin/bash\nexit 1\n");
    let config = make_config(&script, dir.path());
    let provider = LocalProvider::new(config);

    provider.start().await.unwrap();

    // Wait for process to exit
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let status = provider.check_status().await.unwrap();
    assert_eq!(status, ProviderStatus::Crashed);
}

#[tokio::test]
async fn test_clean_self_exit_reports_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let script = make_mock_script(dir.path(), "server.sh", "#!/bin/bash\nexit 0\n");
    let config = make_config(&script, dir.path());
    let provider = LocalProvider::new(config);

    provider.start().await.unwrap();

    // Wait for process to exit
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let status = provider.check_status().await.unwrap();
    assert_eq!(status, ProviderStatus::Stopped);
}

#[tokio::test]
async fn test_stop_timeout_kills() {
    let dir = tempfile::tempdir().unwrap();
    // Script that ignores stop command
    let script = make_mock_script(
        dir.path(),
        "server.sh",
        r#"#!/bin/bash
echo 'For help, type "help"'
# Ignore stdin, trap signals
trap '' TERM INT
sleep 60
"#,
    );
    let mut config = make_config(&script, dir.path());
    config.shutdown_timeout = std::time::Duration::from_secs(1);
    let provider = LocalProvider::new(config);

    provider.start().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // Stop should timeout and kill
    provider.stop().await.unwrap();

    let status = provider.check_status().await.unwrap();
    assert_eq!(status, ProviderStatus::Stopped);
}

#[tokio::test]
async fn test_double_start_fails() {
    let dir = tempfile::tempdir().unwrap();
    let script = make_mock_script(
        dir.path(),
        "server.sh",
        r#"#!/bin/bash
while read -r line; do
    if [ "$line" = "stop" ]; then exit 0; fi
done
"#,
    );
    let config = make_config(&script, dir.path());
    let provider = LocalProvider::new(config);

    provider.start().await.unwrap();
    let result = provider.start().await;
    assert!(result.is_err());

    // Cleanup
    provider.stop().await.unwrap();
}
