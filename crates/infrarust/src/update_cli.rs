use std::ffi::OsString;
use std::fmt::{self, Display};
use std::io::{self, IsTerminal};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use clap::{Subcommand, ValueEnum};
use dialoguer::Confirm;
use infrarust_api::services::{AvailableUpdate, InstallRequest, RestartRequirement, UpdateStatus};
use infrarust_config::{UpdateChannel, UpdateConfig};
use infrarust_core::binary::{ExeSnapshot, RunningBinary};
use infrarust_core::terminal::formatter::InfrarustFormatter;
use infrarust_core::update::{InstallObserver, InstallStep, UpdateError, UpdateService, gate};
use serde::Serialize;
use tokio::runtime::Runtime;
use tracing_subscriber::EnvFilter;

use crate::{BUILD, Cli};

const UPDATE_AVAILABLE: u8 = 100;
const SUCCESS: u8 = 0;
const FAILED: u8 = 1;
const PROGRESS_STEP: u64 = 25;
const LOG_FILTER: &str = "warn,infrarust_core::update::service=error";
const RESTART_HINT: &str = "restart Infrarust to run it; a proxy already running from this binary notices the change and says so in its logs";

const CHECK_EXIT_CODES: &str = "\
Exit codes:
  0    Infrarust is up to date
  100  an update is available
  1    the check failed";

const CHANGE_EXIT_CODES: &str = "\
Exit codes:
  0    done, or nothing to do
  1    refused, cancelled or failed";

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum UpdateAction {
    #[command(
        about = "Look for a newer signed release",
        after_help = CHECK_EXIT_CODES
    )]
    Check {
        #[arg(
            long,
            value_enum,
            help = "Channel to look at instead of the [update] channel of the configuration"
        )]
        channel: Option<ChannelArg>,
        #[arg(long, help = "Print the result as one JSON line")]
        json: bool,
    },
    #[command(
        about = "Download, verify and install a release over this binary",
        after_help = CHANGE_EXIT_CODES
    )]
    Install {
        #[arg(
            long,
            value_name = "VERSION",
            help = "Install this version instead of the newest one on the channel"
        )]
        version: Option<String>,
        #[arg(long, short, help = "Install without asking for confirmation")]
        yes: bool,
    },
    #[command(
        about = "Put back the binary that the last install replaced",
        after_help = CHANGE_EXIT_CODES
    )]
    Rollback {
        #[arg(long, short, help = "Roll back without asking for confirmation")]
        yes: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ChannelArg {
    #[value(help = "Betas for a beta build, stable releases for a stable build")]
    Auto,
    #[value(help = "Stable releases only")]
    Stable,
    #[value(help = "Betas and stable releases")]
    Prerelease,
}

impl From<ChannelArg> for UpdateChannel {
    fn from(channel: ChannelArg) -> Self {
        match channel {
            ChannelArg::Auto => Self::Auto,
            ChannelArg::Stable => Self::Stable,
            ChannelArg::Prerelease => Self::Prerelease,
        }
    }
}

pub fn run(cli: &Cli, action: &UpdateAction, exe: io::Result<ExeSnapshot>) -> ExitCode {
    init_logging();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => return ExitCode::from(fail(format!("failed to build tokio runtime: {e}"))),
    };
    let code = match action {
        UpdateAction::Check { channel, json } => {
            check(&runtime, cli, exe, channel.map(UpdateChannel::from), *json)
        }
        UpdateAction::Install { version, yes } => {
            install(&runtime, cli, exe, version.as_deref(), *yes)
        }
        UpdateAction::Rollback { yes } => rollback(&runtime, cli, exe, *yes),
    };
    ExitCode::from(code)
}

pub fn global_args(cli: &Cli) -> Vec<OsString> {
    let mut args = vec![OsString::from("-c"), cli.config.clone().into_os_string()];
    if let Some(bind) = cli.bind {
        args.extend([OsString::from("--bind"), OsString::from(bind.to_string())]);
    }
    if let Some(dir) = &cli.plugins_dir {
        args.extend([
            OsString::from("--plugins-dir"),
            dir.clone().into_os_string(),
        ]);
    }
    if let Some(dir) = &cli.servers_dir {
        args.extend([
            OsString::from("--servers-dir"),
            dir.clone().into_os_string(),
        ]);
    }
    args
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(LOG_FILTER));
    let _ = tracing_subscriber::fmt()
        .event_format(InfrarustFormatter::new())
        .with_writer(io::stderr)
        .with_env_filter(filter)
        .try_init();
}

fn update_config(cli: &Cli) -> anyhow::Result<UpdateConfig> {
    if matches!(cli.config.try_exists(), Ok(false)) {
        return Ok(UpdateConfig::default());
    }
    crate::load_config(cli).map(|(config, _)| config.update)
}

fn service(
    cli: &Cli,
    exe: io::Result<ExeSnapshot>,
    config: UpdateConfig,
) -> Result<Arc<UpdateService>, String> {
    let exe = exe.map_err(|e| format!("cannot locate the running executable: {e}"))?;
    let binary = RunningBinary::new(BUILD, exe, global_args(cli));
    Ok(UpdateService::new(config, binary, None))
}

fn configured_service(
    cli: &Cli,
    exe: io::Result<ExeSnapshot>,
) -> Result<Arc<UpdateService>, String> {
    let config = update_config(cli).map_err(|e| format!("{e:#}"))?;
    service(cli, exe, config)
}

fn check(
    runtime: &Runtime,
    cli: &Cli,
    exe: io::Result<ExeSnapshot>,
    channel: Option<UpdateChannel>,
    json: bool,
) -> u8 {
    let (report, code) = match configured_service(cli, exe) {
        Ok(service) => runtime.block_on(checked(&service, channel)),
        Err(error) => (CheckReport::failed(error), FAILED),
    };
    if json {
        match serde_json::to_string(&report) {
            Ok(line) => println!("{line}"),
            Err(e) => return fail(e),
        }
    } else {
        print!("{}", report.summary());
        if let Some(error) = &report.last_error {
            eprintln!("error: {error}");
        }
    }
    code
}

async fn checked(service: &UpdateService, channel: Option<UpdateChannel>) -> (CheckReport, u8) {
    let outcome = service.check(channel).await;
    (
        CheckReport::from_status(service.status()),
        check_code(&outcome),
    )
}

fn check_code<E>(outcome: &Result<Option<AvailableUpdate>, E>) -> u8 {
    match outcome {
        Ok(Some(_)) => UPDATE_AVAILABLE,
        Ok(None) => SUCCESS,
        Err(_) => FAILED,
    }
}

fn install(
    runtime: &Runtime,
    cli: &Cli,
    exe: io::Result<ExeSnapshot>,
    version: Option<&str>,
    yes: bool,
) -> u8 {
    let version = version.map(|version| version.strip_prefix('v').unwrap_or(version));
    if let Some(refusal) = gate::build_refusal(&BUILD) {
        return fail(refusal.instructions(version));
    }
    if matches!(cli.config.try_exists(), Ok(false)) {
        return fail(format_args!(
            "{} does not exist; the new binary checks the proxy configuration before it replaces this one, pass its path with -c",
            cli.config.display()
        ));
    }
    let service = match configured_service(cli, exe) {
        Ok(service) => service,
        Err(e) => return fail(e),
    };
    let target = match version {
        Some(version) => version.to_string(),
        None => match runtime.block_on(service.check(None)) {
            Ok(Some(update)) => update.version,
            Ok(None) => {
                println!(
                    "Infrarust {} is up to date, nothing to install",
                    BUILD.version
                );
                return SUCCESS;
            }
            Err(e) => return fail(e),
        },
    };
    if let Some(refusal) = service.status().install_refusal {
        return fail(refusal);
    }
    let exe_path = service.binary().exe.path().to_path_buf();
    match confirmed(yes, || {
        Ok(format!(
            "Install Infrarust {target} over {}?",
            exe_path.display()
        ))
    }) {
        Ok(true) => {}
        Ok(false) => return cancelled(),
        Err(e) => return fail(e),
    }
    let request = InstallRequest {
        version: Some(target),
        then_restart: false,
    };
    match runtime.block_on(service.install(request, &Progress::default())) {
        Ok(installed) => {
            println!(
                "installed Infrarust {} at {}, the previous binary is kept at {}",
                installed.version,
                installed.exe.display(),
                installed.previous.display()
            );
            println!("{RESTART_HINT}");
            SUCCESS
        }
        Err(e) => fail(e),
    }
}

fn rollback(runtime: &Runtime, cli: &Cli, exe: io::Result<ExeSnapshot>, yes: bool) -> u8 {
    let config = update_config(cli).unwrap_or_else(|e| {
        eprintln!("warning: {e:#}");
        eprintln!(
            "warning: rolling back anyway, the previous binary checks the configuration before it is restored"
        );
        UpdateConfig::default()
    });
    let service = match service(cli, exe, config) {
        Ok(service) => service,
        Err(e) => return fail(e),
    };
    let paths = match gate::paths_for(service.binary().exe.path()) {
        Ok(paths) => paths,
        Err(refusal) => return fail(refusal),
    };
    if std::fs::symlink_metadata(&paths.old).is_err() {
        return fail(UpdateError::NothingToRollBack);
    }
    match confirmed(yes, || {
        let previous = runtime
            .block_on(infrarust_core::binary::probe(&paths.old))
            .map_err(|e| e.to_string())?;
        Ok(format!(
            "Restore Infrarust {} over {}?",
            previous.version,
            paths.exe.display()
        ))
    }) {
        Ok(true) => {}
        Ok(false) => return cancelled(),
        Err(e) => return fail(e),
    }
    match runtime.block_on(service.rollback()) {
        Ok(rolled_back) => {
            println!(
                "restored Infrarust {} at {}, the replaced binary is kept at {}",
                rolled_back.version,
                rolled_back.exe.display(),
                rolled_back.previous.display()
            );
            println!("{RESTART_HINT}");
            println!("run `infrarust update rollback` again to undo");
            SUCCESS
        }
        Err(e) => fail(e),
    }
}

fn fail(error: impl Display) -> u8 {
    eprintln!("error: {error}");
    FAILED
}

fn cancelled() -> u8 {
    eprintln!("cancelled, nothing was changed");
    FAILED
}

fn confirmed(yes: bool, prompt: impl FnOnce() -> Result<String, String>) -> Result<bool, String> {
    if !needs_prompt(yes, io::stdin().is_terminal())? {
        return Ok(true);
    }
    Confirm::new()
        .with_prompt(prompt()?)
        .default(false)
        .interact()
        .map_err(|e| format!("cannot ask for confirmation: {e}"))
}

fn needs_prompt(yes: bool, interactive: bool) -> Result<bool, String> {
    match (yes, interactive) {
        (true, _) => Ok(false),
        (false, true) => Ok(true),
        (false, false) => Err(
            "stdin is not a terminal, so nobody can confirm; pass --yes to go ahead".to_string(),
        ),
    }
}

#[derive(Debug, Default)]
struct Progress {
    shown: AtomicU64,
}

impl Progress {
    fn advance(&self, received: u64, total: u64) -> Option<u64> {
        let mark = progress_mark(received, total)?;
        (self.shown.fetch_max(mark, Ordering::Relaxed) < mark).then_some(mark)
    }
}

impl InstallObserver for Progress {
    fn on_step(&self, step: &InstallStep) {
        if !matches!(step, InstallStep::Installed { .. }) {
            eprintln!("{step}");
        }
    }

    fn on_download(&self, received: u64, total: u64) {
        if let Some(mark) = self.advance(received, total) {
            eprintln!("  {mark}%");
        }
    }
}

fn progress_mark(received: u64, total: u64) -> Option<u64> {
    if total == 0 {
        return None;
    }
    let percent = (received.saturating_mul(100) / total).min(100);
    let mark = percent / PROGRESS_STEP * PROGRESS_STEP;
    (mark > 0).then_some(mark)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CheckReport {
    current_version: String,
    last_check: Option<String>,
    last_error: Option<String>,
    available: Option<AvailableReport>,
    install_refusal: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct AvailableReport {
    version: String,
    published_at: Option<String>,
    notes_url: Option<String>,
    security: bool,
    restart: Restart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Restart {
    Hot,
    Full,
    Manual,
}

impl From<RestartRequirement> for Restart {
    fn from(restart: RestartRequirement) -> Self {
        match restart {
            RestartRequirement::Hot => Self::Hot,
            RestartRequirement::Full => Self::Full,
            RestartRequirement::Manual => Self::Manual,
        }
    }
}

impl Restart {
    const fn text(self) -> &'static str {
        match self {
            Self::Hot => "hot, players stay connected",
            Self::Full => "full, players are disconnected",
            Self::Manual => "manual, stop and start the proxy by hand",
        }
    }
}

impl From<AvailableUpdate> for AvailableReport {
    fn from(update: AvailableUpdate) -> Self {
        Self {
            version: update.version,
            published_at: update.published_at,
            notes_url: update.notes_url,
            security: update.security,
            restart: update.restart.into(),
        }
    }
}

impl CheckReport {
    fn from_status(status: UpdateStatus) -> Self {
        Self {
            current_version: status.current_version,
            last_check: status
                .last_check
                .map(|at| humantime::format_rfc3339_seconds(at).to_string()),
            last_error: status.last_error,
            available: status.available.map(AvailableReport::from),
            install_refusal: status.install_refusal,
        }
    }

    fn failed(error: String) -> Self {
        Self {
            current_version: BUILD.version.to_string(),
            last_check: None,
            last_error: Some(error),
            available: None,
            install_refusal: gate::build_refusal(&BUILD).map(|refusal| refusal.instructions(None)),
        }
    }

    fn summary(&self) -> Summary<'_> {
        Summary(self)
    }
}

struct Summary<'a>(&'a CheckReport);

impl Display for Summary<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let report = self.0;
        writeln!(f, "current version: {}", report.current_version)?;
        if report.last_error.is_some() {
            return Ok(());
        }
        let Some(update) = &report.available else {
            return writeln!(f, "up to date");
        };
        writeln!(f, "update available: {}", update.version)?;
        let security = if update.security {
            "yes, this release fixes a security issue"
        } else {
            "no"
        };
        writeln!(f, "  security fix: {security}")?;
        if let Some(published) = &update.published_at {
            writeln!(f, "  published:    {published}")?;
        }
        if let Some(notes) = &update.notes_url {
            writeln!(f, "  notes:        {notes}")?;
        }
        writeln!(f, "  restart:      {}", update.restart.text())?;
        match &report.install_refusal {
            Some(refusal) => writeln!(f, "this binary cannot install it: {refusal}"),
            None => writeln!(f, "install it with `infrarust update install`"),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;
    use std::io::Cursor;
    use std::time::{Duration, SystemTime};

    use infrarust_api::services::InstallState;
    use infrarust_core::binary::BuildInfo;
    use infrarust_core::update::TrustedKeys;
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;
    use crate::tests::parse;

    const UNOFFICIAL: BuildInfo = BuildInfo {
        official: false,
        docker: false,
        ..BUILD
    };

    type Routes = HashMap<String, (u16, String)>;

    async fn serve(routes: impl FnOnce(&str) -> Routes) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes = Arc::new(routes(&base));
        tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let routes = Arc::clone(&routes);
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                        if stream.read_buf(&mut head).await.unwrap() == 0 {
                            return;
                        }
                    }
                    let head = String::from_utf8_lossy(&head).into_owned();
                    let target = head.split(' ').nth(1).unwrap_or("/");
                    let path = target.split('?').next().unwrap_or("/");
                    let (status, body) = routes.get(path).cloned().unwrap_or((404, String::new()));
                    let response = format!(
                        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        base
    }

    struct Signer {
        public: minisign::PublicKey,
        secret: minisign::SecretKey,
    }

    impl Signer {
        fn new() -> Self {
            let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
            Self {
                public: pair.pk,
                secret: pair.sk,
            }
        }

        fn keys(&self) -> TrustedKeys {
            TrustedKeys::from_strs([self.public.to_box().unwrap().into_string()]).unwrap()
        }

        fn sign(&self, data: &str) -> String {
            minisign::sign(
                Some(&self.public),
                &self.secret,
                Cursor::new(data.as_bytes()),
                Some("file:manifest.json"),
                None,
            )
            .unwrap()
            .into_string()
        }
    }

    fn signed_release(signer: &Signer, base: &str, version: &str, security: bool) -> Routes {
        let manifest = json!({
            "schema": 1,
            "version": version,
            "tag": format!("v{version}"),
            "published_at": "2026-11-24T10:00:00Z",
            "notes_url": format!("https://example.invalid/notes/{version}"),
            "security": security,
            "handover": null,
            "artifacts": [{
                "targets": [BUILD.target],
                "file": "infrarust-linux-x86_64.tar.gz",
                "binary": "infrarust",
                "sha256": "ab".repeat(32),
                "size": 1024,
            }],
        })
        .to_string();
        let releases = json!([{
            "tag_name": format!("v{version}"),
            "html_url": format!("https://example.invalid/releases/v{version}"),
            "assets": [
                {"name": "manifest.json", "browser_download_url": format!("{base}/manifest.json")},
                {"name": "manifest.json.minisig", "browser_download_url": format!("{base}/manifest.json.minisig")},
            ],
        }])
        .to_string();
        HashMap::from([
            ("/releases".to_string(), (200, releases)),
            (
                "/manifest.json.minisig".to_string(),
                (200, signer.sign(&manifest)),
            ),
            ("/manifest.json".to_string(), (200, manifest)),
        ])
    }

    fn local_service(base: &str, keys: TrustedKeys) -> Arc<UpdateService> {
        let config = UpdateConfig {
            url: format!("{base}/releases"),
            ..UpdateConfig::default()
        };
        let exe = ExeSnapshot::at(std::env::current_exe().unwrap());
        let binary = RunningBinary::new(UNOFFICIAL, exe, global_args(&parse(&[])));
        UpdateService::with_keys(config, binary, None, keys)
    }

    #[tokio::test]
    async fn a_signed_newer_release_is_reported_with_exit_code_100() {
        let signer = Signer::new();
        let base = serve(|base| signed_release(&signer, base, "99.0.0", true)).await;
        let (report, code) = checked(&local_service(&base, signer.keys()), None).await;
        assert_eq!(code, 100);
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["current_version"], json!(BUILD.version));
        assert_eq!(value["last_error"], json!(null));
        assert_eq!(
            value["available"],
            json!({
                "version": "99.0.0",
                "published_at": "2026-11-24T10:00:00Z",
                "notes_url": "https://example.invalid/notes/99.0.0",
                "security": true,
                "restart": "full",
            })
        );
        let refusal = value["install_refusal"].as_str().unwrap();
        assert!(
            refusal.contains("`cargo install infrarust --version 99.0.0`"),
            "{refusal}"
        );
        let text = report.summary().to_string();
        assert!(text.contains("update available: 99.0.0\n"), "{text}");
        assert!(text.contains("this binary cannot install it: "), "{text}");
    }

    #[tokio::test]
    async fn an_empty_release_list_is_up_to_date_with_exit_code_0() {
        let base =
            serve(|_| HashMap::from([("/releases".to_string(), (200, "[]".to_string()))])).await;
        let (report, code) = checked(&local_service(&base, Signer::new().keys()), None).await;
        assert_eq!(code, 0);
        assert_eq!(report.available, None);
        assert_eq!(report.last_error, None);
        assert!(report.last_check.is_some());
    }

    #[tokio::test]
    async fn a_newer_release_without_a_signature_is_not_offered() {
        let signer = Signer::new();
        let base = serve(|base| {
            let mut routes = signed_release(&signer, base, "99.0.0", false);
            routes.remove("/manifest.json.minisig");
            routes
        })
        .await;
        let stable = Some(UpdateChannel::Stable);
        let (report, code) = checked(&local_service(&base, signer.keys()), stable).await;
        assert_eq!(code, 0, "{report:?}");
        assert_eq!(report.available, None);
        assert_eq!(report.last_error, None);
    }

    #[tokio::test]
    async fn a_failing_release_list_gives_exit_code_1_and_the_error() {
        let base =
            serve(|_| HashMap::from([("/releases".to_string(), (500, String::new()))])).await;
        let (report, code) = checked(&local_service(&base, Signer::new().keys()), None).await;
        assert_eq!(code, 1);
        let error = report.last_error.unwrap();
        assert!(error.contains("HTTP 500"), "{error}");
        assert_eq!(report.available, None);
    }

    fn update(security: bool, restart: RestartRequirement) -> AvailableUpdate {
        AvailableUpdate {
            version: "2.0.0-beta.6".to_string(),
            published_at: Some("2026-11-20T10:00:00Z".to_string()),
            notes_url: Some(
                "https://github.com/Shadowner/Infrarust/releases/tag/v2.0.0-beta.6".to_string(),
            ),
            security,
            restart,
        }
    }

    fn status(available: Option<AvailableUpdate>) -> UpdateStatus {
        UpdateStatus {
            current_version: "2.0.0-beta.5".to_string(),
            checks_enabled: true,
            last_check: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_795_000_000)),
            last_error: None,
            available,
            install: InstallState::Idle,
            install_refusal: None,
            binary_changed: false,
        }
    }

    #[test]
    fn the_global_args_carry_the_config_path_even_when_it_is_the_default() {
        assert_eq!(
            global_args(&parse(&["update", "check"])),
            [OsString::from("-c"), OsString::from("infrarust.toml")]
        );
    }

    #[test]
    fn the_global_args_keep_the_overrides_and_drop_the_rest() {
        let cli = parse(&[
            "-l",
            "debug",
            "--servers-dir",
            "/srv/mc",
            "-c",
            "/etc/infrarust/infrarust.toml",
            "--bind",
            "0.0.0.0:25577",
            "--plugins-dir",
            "/srv/plugins",
            "update",
            "install",
            "--yes",
        ]);
        let args: Vec<String> = global_args(&cli)
            .into_iter()
            .map(|arg| arg.into_string().unwrap())
            .collect();
        assert_eq!(
            args,
            [
                "-c",
                "/etc/infrarust/infrarust.toml",
                "--bind",
                "0.0.0.0:25577",
                "--plugins-dir",
                "/srv/plugins",
                "--servers-dir",
                "/srv/mc",
            ]
        );
    }

    #[test]
    fn the_global_args_are_a_valid_prefix_for_check_config() {
        let cli = parse(&[
            "-c",
            "custom.toml",
            "--bind",
            "127.0.0.1:25599",
            "--servers-dir",
            "/srv/mc",
            "update",
            "rollback",
        ]);
        let mut argv = vec![OsString::from("infrarust")];
        argv.extend(global_args(&cli));
        argv.extend([OsString::from("check-config"), OsString::from("--json")]);
        let replayed = <Cli as clap::Parser>::parse_from(argv);
        assert!(matches!(
            replayed.command,
            Some(crate::Command::CheckConfig { json: true })
        ));
        assert_eq!(replayed.config, cli.config);
        assert_eq!(replayed.bind, cli.bind);
        assert_eq!(replayed.plugins_dir, None);
        assert_eq!(replayed.servers_dir, cli.servers_dir);
    }

    #[test]
    fn the_check_exit_code_tells_an_update_from_up_to_date_and_errors() {
        let available: Result<_, UpdateError> = Ok(Some(update(false, RestartRequirement::Full)));
        assert_eq!(check_code(&available), 100);
        assert_eq!(check_code::<UpdateError>(&Ok(None)), 0);
        assert_eq!(check_code(&Err(UpdateError::NoTrustedKey)), 1);
        assert_eq!(UPDATE_AVAILABLE, 100);
    }

    #[test]
    fn the_json_report_mirrors_the_update_status() {
        let mut status = status(Some(update(true, RestartRequirement::Full)));
        status.install_refusal = Some("install with `cargo install infrarust`".to_string());
        let value = serde_json::to_value(CheckReport::from_status(status)).unwrap();
        assert_eq!(
            value,
            json!({
                "current_version": "2.0.0-beta.5",
                "last_check": "2026-11-18T11:06:40Z",
                "last_error": null,
                "available": {
                    "version": "2.0.0-beta.6",
                    "published_at": "2026-11-20T10:00:00Z",
                    "notes_url": "https://github.com/Shadowner/Infrarust/releases/tag/v2.0.0-beta.6",
                    "security": true,
                    "restart": "full",
                },
                "install_refusal": "install with `cargo install infrarust`",
            })
        );
    }

    #[test]
    fn the_json_report_of_an_up_to_date_check_has_no_update() {
        let value = serde_json::to_value(CheckReport::from_status(status(None))).unwrap();
        assert_eq!(value["available"], json!(null));
        assert_eq!(value["last_error"], json!(null));
        assert_eq!(value["current_version"], json!("2.0.0-beta.5"));
    }

    #[test]
    fn a_check_that_never_ran_still_reports_the_version_and_the_error() {
        let report = CheckReport::failed("invalid TOML in infrarust.toml".to_string());
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["current_version"], json!(BUILD.version));
        assert_eq!(value["last_error"], json!("invalid TOML in infrarust.toml"));
        assert_eq!(value["available"], json!(null));
        assert_eq!(value["last_check"], json!(null));
        assert_eq!(
            report.summary().to_string(),
            format!("current version: {}\n", BUILD.version)
        );
    }

    #[test]
    fn the_summary_lists_the_update_details() {
        let text = CheckReport::from_status(status(Some(update(true, RestartRequirement::Hot))))
            .summary()
            .to_string();
        assert_eq!(
            text,
            "current version: 2.0.0-beta.5\n\
             update available: 2.0.0-beta.6\n  \
             security fix: yes, this release fixes a security issue\n  \
             published:    2026-11-20T10:00:00Z\n  \
             notes:        https://github.com/Shadowner/Infrarust/releases/tag/v2.0.0-beta.6\n  \
             restart:      hot, players stay connected\n\
             install it with `infrarust update install`\n"
        );
    }

    #[test]
    fn the_summary_names_the_refusal_instead_of_the_install_command() {
        let mut status = status(Some(update(false, RestartRequirement::Manual)));
        status.install_refusal = Some("download it by hand".to_string());
        let text = CheckReport::from_status(status).summary().to_string();
        assert!(text.contains("  security fix: no\n"), "{text}");
        assert!(
            text.contains("manual, stop and start the proxy by hand"),
            "{text}"
        );
        assert!(
            text.ends_with("this binary cannot install it: download it by hand\n"),
            "{text}"
        );
    }

    #[test]
    fn the_summary_of_an_up_to_date_check() {
        let text = CheckReport::from_status(status(None)).summary().to_string();
        assert_eq!(text, "current version: 2.0.0-beta.5\nup to date\n");
    }

    #[test]
    fn the_download_progress_is_shown_every_quarter() {
        let progress = Progress::default();
        let shown: Vec<u64> = [0, 10, 24, 25, 30, 60, 74, 99, 100, 100]
            .into_iter()
            .filter_map(|received| progress.advance(received, 100))
            .collect();
        assert_eq!(shown, [25, 50, 75, 100]);
        assert_eq!(progress_mark(5, 0), None);
        assert_eq!(progress_mark(u64::MAX, 10), Some(100));
    }

    #[test]
    fn a_prompt_is_needed_unless_yes_and_refused_without_a_terminal() {
        assert_eq!(needs_prompt(true, false), Ok(false));
        assert_eq!(needs_prompt(true, true), Ok(false));
        assert_eq!(needs_prompt(false, true), Ok(true));
        let refused = needs_prompt(false, false).unwrap_err();
        assert!(refused.contains("--yes"), "{refused}");
    }

    #[test]
    fn the_channel_values_map_to_the_config_channels() {
        assert_eq!(UpdateChannel::from(ChannelArg::Auto), UpdateChannel::Auto);
        assert_eq!(
            UpdateChannel::from(ChannelArg::Stable),
            UpdateChannel::Stable
        );
        assert_eq!(
            UpdateChannel::from(ChannelArg::Prerelease),
            UpdateChannel::Prerelease
        );
    }
}
