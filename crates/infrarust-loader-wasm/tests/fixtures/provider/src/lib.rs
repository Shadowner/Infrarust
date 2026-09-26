use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::rc::Rc;

use infrarust_plugin_sdk::prelude::*;

const CONFIG: &str = "/provider.txt";
const LOG: &str = "/log.txt";

fn log(line: &str) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(LOG)
        .expect("opening the fixture log");
    writeln!(file, "{line}").expect("writing the fixture log");
}

fn outcome<T>(result: &Result<T, Error>) -> &'static str {
    match result {
        Ok(_) => "ok",
        Err(error) => error.kind().as_str(),
    }
}

fn misbehave(username: &str) {
    match username {
        "Trap" => panic!("the provider fixture traps on purpose"),
        "Spin" => loop {
            std::hint::spin_loop();
        },
        _ => {}
    }
}

#[derive(Default)]
struct State {
    bans: Vec<BanRecord>,
    next_id: u32,
    check_calls_service: bool,
    grants: BTreeMap<String, PermissionSnapshot>,
    admins: BTreeSet<String>,
    console_admin: bool,
}

impl State {
    fn snapshot(&self, username: &str) -> PermissionSnapshot {
        if self.admins.contains(username) {
            return PermissionSnapshot::admin();
        }
        self.grants.get(username).cloned().unwrap_or_default()
    }
}

#[derive(Clone, Default)]
struct Store(Rc<RefCell<State>>);

impl BanProvider for Store {
    fn check(&self, attempt: &LoginAttempt) -> Result<Option<BanVerdict>, PluginError> {
        let username = attempt.username.clone().unwrap_or_default();
        misbehave(&username);
        if self.0.borrow().check_calls_service {
            let asked = Bans::get(&BanTarget::Username(username.clone()));
            log(&format!("check service {}", outcome(&asked)));
        }
        let state = self.0.borrow();
        Ok(state
            .bans
            .iter()
            .find(|record| record.target.matches(attempt))
            .map(|record| {
                let reason = record.reason.clone().unwrap_or_default();
                BanVerdict::new(record.clone()).message(format!("provider: {reason}"))
            }))
    }

    fn ban(&self, request: BanRequest, source: BanSource) -> Result<BanRecord, PluginError> {
        let mut state = self.0.borrow_mut();
        state.next_id += 1;
        let mut record = BanRecord::new(format!("p{}", state.next_id), request.target, source);
        record.reason = request.reason;
        state.bans.push(record.clone());
        Ok(record)
    }

    fn unban(&self, request: UnbanRequest) -> Result<Option<BanRecord>, PluginError> {
        let mut state = self.0.borrow_mut();
        let at = state
            .bans
            .iter()
            .position(|record| record.target == request.target);
        Ok(at.map(|at| state.bans.remove(at)))
    }

    fn get(&self, target: &BanTarget) -> Result<Option<BanRecord>, PluginError> {
        Ok(self
            .0
            .borrow()
            .bans
            .iter()
            .find(|record| &record.target == target)
            .cloned())
    }

    fn list(&self, _query: &BanQuery) -> Result<BanRecordPage, PluginError> {
        Ok(BanRecordPage::new(self.0.borrow().bans.clone(), None))
    }

    fn features(&self) -> BanFeatures {
        BanFeatures::new().ip_ranges(true)
    }
}

impl PermissionProvider for Store {
    fn snapshot_for(&self, subject: &PermissionSubject) -> PermissionSnapshot {
        match subject.profile() {
            Some(profile) => {
                misbehave(&profile.username);
                self.0.borrow().snapshot(&profile.username)
            }
            None if self.0.borrow().console_admin => PermissionSnapshot::admin(),
            None => PermissionSnapshot::new(),
        }
    }
}

fn configure(store: &Store, ctx: &Context, line: &str) -> Result<(), PluginError> {
    let words: Vec<&str> = line.split_whitespace().collect();
    match words.as_slice() {
        ["bans"] => {
            let registered = ctx.provide_bans(store.clone());
            log(&format!("bans {}", outcome(&registered)));
        }
        ["permissions"] => {
            let registered = ctx.provide_permissions(store.clone());
            log(&format!("permissions {}", outcome(&registered)));
        }
        ["ban", name, reason @ ..] => {
            let mut state = store.0.borrow_mut();
            let record = BanRecord::new(
                format!("seed-{name}"),
                BanTarget::Username((*name).to_owned()),
                BanSource::Console,
            )
            .reason(reason.join(" "));
            state.bans.push(record);
        }
        ["check-service"] => store.0.borrow_mut().check_calls_service = true,
        ["grant", name, node, value] => {
            let value = *value == "true";
            store
                .0
                .borrow_mut()
                .grants
                .entry((*name).to_owned())
                .or_default()
                .set(node, value);
        }
        ["admin", name] => {
            store.0.borrow_mut().admins.insert((*name).to_owned());
        }
        ["console-admin"] => store.0.borrow_mut().console_admin = true,
        ["command", label, node] => {
            let label = (*label).to_owned();
            let name = label.clone();
            ctx.command(&label)
                .permission(*node)
                .handler(move |invocation| {
                    let who = invocation
                        .player()
                        .map_or_else(|| "console".to_owned(), |p| p.username.clone());
                    log(&format!("ran {name} {who}"));
                    let _ = invocation.reply(format!("ran {name}"));
                })
                .register()?;
        }
        other => return Err(format!("unknown provider directive {other:?}").into()),
    }
    Ok(())
}

fn player_named(name: &str) -> Result<PlayerId, Error> {
    Players::by_name(name)
        .map(|info| info.id())
        .ok_or_else(|| Error::new(ErrorKind::PlayerGone, format!("{name} is not online")))
}

fn register_tools(store: &Store, ctx: &Context) -> Result<(), PluginError> {
    let grants = store.clone();
    ctx.command("pset")
        .handler(move |invocation| {
            let [name, node, value] = invocation.args.as_slice() else {
                return;
            };
            let snapshot = {
                let mut state = grants.0.borrow_mut();
                state
                    .grants
                    .entry(name.clone())
                    .or_default()
                    .set(node, value == "true");
                state.snapshot(name)
            };
            let applied =
                player_named(name).and_then(|player| Permissions::set_snapshot(player, &snapshot));
            log(&format!("pset {name} {}", outcome(&applied)));
        })
        .register()?;
    ctx.command("prelease")
        .handler(move |invocation| {
            let [name] = invocation.args.as_slice() else {
                return;
            };
            let released = player_named(name).and_then(Permissions::release);
            log(&format!("prelease {name} {}", outcome(&released)));
        })
        .register()?;
    ctx.command("pban")
        .handler(move |invocation| {
            let [name] = invocation.args.as_slice() else {
                return;
            };
            let banned = Bans::ban(BanRequest::new(BanTarget::Username(name.clone())));
            log(&format!("pban {name} {}", outcome(&banned)));
        })
        .register()?;
    ctx.command("pping").handler(|_| log("pping")).register()?;
    ctx.command("ptrap")
        .handler(|_| panic!("the provider fixture traps on purpose"))
        .register()?;
    Ok(())
}

#[derive(Default)]
struct ProviderFixture;

#[plugin(id = "provider", name = "Provider Fixture")]
impl Plugin for ProviderFixture {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        let line = match ctx.enable_reason() {
            Some(EnableReason::Recovered(info)) => format!("enable recovered {}", info.attempt),
            _ => "enable".to_owned(),
        };
        log(&line);
        let config = std::fs::read_to_string(CONFIG).unwrap_or_default();
        let store = Store::default();
        for line in config
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            configure(&store, ctx, line)?;
        }
        register_tools(&store, ctx)
    }
}
