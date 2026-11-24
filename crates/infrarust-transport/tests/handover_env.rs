#![allow(clippy::unwrap_used, clippy::expect_used)]

#[cfg(target_os = "linux")]
fn main() {
    use std::process::Command;

    if std::env::var_os(CHILD).is_some() {
        child();
        return;
    }
    let status = Command::new(std::env::current_exe().unwrap())
        .env(CHILD, "1")
        .env("INFRARUST_ENV_TEST_GONE", "gone")
        .env("INFRARUST_ENV_TEST_ALSO_GONE", "also gone")
        .env("INFRARUST_ENV_TEST_KEPT", "kept")
        .status()
        .unwrap();
    assert!(
        status.success(),
        "the single-threaded child failed: {status}"
    );
}

#[cfg(target_os = "linux")]
const CHILD: &str = "INFRARUST_ENV_TEST_CHILD";

#[cfg(target_os = "linux")]
fn child() {
    use std::env::var_os;
    use std::sync::mpsc;

    use infrarust_transport::handover::remove_env_vars;

    remove_env_vars(&[
        "INFRARUST_ENV_TEST_GONE",
        "INFRARUST_ENV_TEST_ALSO_GONE",
        "INFRARUST_ENV_TEST_NEVER_SET",
    ])
    .unwrap();
    assert!(var_os("INFRARUST_ENV_TEST_GONE").is_none());
    assert!(var_os("INFRARUST_ENV_TEST_ALSO_GONE").is_none());
    assert_eq!(var_os("INFRARUST_ENV_TEST_KEPT").unwrap(), "kept");

    let (release, wait) = mpsc::channel::<()>();
    let worker = std::thread::spawn(move || wait.recv().ok());
    let err = remove_env_vars(&["INFRARUST_ENV_TEST_KEPT"]).unwrap_err();
    assert!(err.to_string().contains("one thread"), "{err}");
    assert_eq!(var_os("INFRARUST_ENV_TEST_KEPT").unwrap(), "kept");
    drop(release);
    worker.join().unwrap();
}

#[cfg(not(target_os = "linux"))]
fn main() {}
