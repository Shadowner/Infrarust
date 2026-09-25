use std::io::Write;
use std::path::Path;

pub const SCRIPT_FILE: &str = "script.txt";
pub const LOG_FILE: &str = "log.txt";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventName {
    PreLogin,
    PostLogin,
    Disconnect,
    OnlineAuthFailed,
    PermissionsSetup,
    ServerPreConnect,
    ServerConnected,
    ServerSwitch,
    KickedFromServer,
    PlayerChooseInitialServer,
    ProxyPing,
    ProxyInitialize,
    ProxyShutdown,
    ConfigReload,
    ServerStateChange,
    ChatMessage,
}

impl EventName {
    pub const ALL: [Self; 16] = [
        Self::PreLogin,
        Self::PostLogin,
        Self::Disconnect,
        Self::OnlineAuthFailed,
        Self::PermissionsSetup,
        Self::ServerPreConnect,
        Self::ServerConnected,
        Self::ServerSwitch,
        Self::KickedFromServer,
        Self::PlayerChooseInitialServer,
        Self::ProxyPing,
        Self::ProxyInitialize,
        Self::ProxyShutdown,
        Self::ConfigReload,
        Self::ServerStateChange,
        Self::ChatMessage,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PreLogin => "pre-login",
            Self::PostLogin => "post-login",
            Self::Disconnect => "disconnect",
            Self::OnlineAuthFailed => "online-auth-failed",
            Self::PermissionsSetup => "permissions-setup",
            Self::ServerPreConnect => "server-pre-connect",
            Self::ServerConnected => "server-connected",
            Self::ServerSwitch => "server-switch",
            Self::KickedFromServer => "kicked-from-server",
            Self::PlayerChooseInitialServer => "player-choose-initial-server",
            Self::ProxyPing => "proxy-ping",
            Self::ProxyInitialize => "proxy-initialize",
            Self::ProxyShutdown => "proxy-shutdown",
            Self::ConfigReload => "config-reload",
            Self::ServerStateChange => "server-state-change",
            Self::ChatMessage => "chat-message",
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|event| event.as_str() == word)
    }

    pub fn accepts(self, action: &Action) -> bool {
        match action {
            Action::Record | Action::Panic | Action::Cancelled => true,
            Action::Allow => matches!(
                self,
                Self::PreLogin
                    | Self::ServerPreConnect
                    | Self::PlayerChooseInitialServer
                    | Self::ChatMessage
            ),
            Action::Deny(_) => matches!(
                self,
                Self::PreLogin | Self::ServerPreConnect | Self::ChatMessage
            ),
            Action::ForceOffline | Action::ForceOnline => self == Self::PreLogin,
            Action::ConnectTo(_) => self == Self::ServerPreConnect,
            Action::Redirect(_) => matches!(
                self,
                Self::PlayerChooseInitialServer | Self::KickedFromServer
            ),
            Action::Limbo(_) => matches!(
                self,
                Self::ServerPreConnect | Self::PlayerChooseInitialServer | Self::KickedFromServer
            ),
            Action::Notify(_) | Action::Disconnect(_) => self == Self::KickedFromServer,
            Action::Modify(_) => self == Self::ChatMessage,
            Action::Description(_) => self == Self::ProxyPing,
            Action::Custom(_) => self == Self::PermissionsSetup,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Record,
    Panic,
    Cancelled,
    Allow,
    ForceOffline,
    ForceOnline,
    Deny(String),
    ConnectTo(String),
    Redirect(String),
    Limbo(Vec<String>),
    Notify(String),
    Disconnect(String),
    Modify(String),
    Description(String),
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Directive {
    On {
        event: EventName,
        priority: u8,
        action: Action,
    },
    Cmd {
        name: String,
    },
}

pub fn parse(script: &str) -> Result<Vec<Directive>, String> {
    script
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| parse_line(line).map_err(|e| format!("{line:?}: {e}")))
        .collect()
}

fn parse_line(line: &str) -> Result<Directive, String> {
    let (keyword, rest) = word(line);
    match keyword {
        "on" => {
            let (event, rest) = word(rest);
            let event =
                EventName::parse(event).ok_or_else(|| format!("unknown event {event:?}"))?;
            let (priority, rest) = word(rest);
            let priority = parse_priority(priority)?;
            let action = parse_action(rest)?;
            if !event.accepts(&action) {
                return Err(format!("{} cannot {action:?}", event.as_str()));
            }
            Ok(Directive::On {
                event,
                priority,
                action,
            })
        }
        "cmd" => {
            let (name, rest) = word(rest);
            if name.is_empty() || rest.trim() != "record" {
                return Err("expected `cmd <name> record`".to_owned());
            }
            Ok(Directive::Cmd {
                name: name.to_owned(),
            })
        }
        other => Err(format!("unknown directive {other:?}")),
    }
}

fn word(text: &str) -> (&str, &str) {
    let text = text.trim_start();
    text.split_once(char::is_whitespace).unwrap_or((text, ""))
}

fn parse_priority(word: &str) -> Result<u8, String> {
    match word {
        "first" => Ok(0),
        "early" => Ok(64),
        "normal" => Ok(128),
        "late" => Ok(192),
        "last" => Ok(255),
        other => other
            .parse()
            .map_err(|_| format!("unknown priority {other:?}")),
    }
}

fn parse_action(text: &str) -> Result<Action, String> {
    let (name, rest) = word(text);
    let rest = rest.trim();
    let bare = match name {
        "record" => Some(Action::Record),
        "panic" => Some(Action::Panic),
        "cancelled" => Some(Action::Cancelled),
        "allow" => Some(Action::Allow),
        "force-offline" => Some(Action::ForceOffline),
        "force-online" => Some(Action::ForceOnline),
        _ => None,
    };
    if let Some(action) = bare {
        return if rest.is_empty() {
            Ok(action)
        } else {
            Err(format!("{name} takes no argument"))
        };
    }
    if rest.is_empty() {
        return Err(format!("{name:?} needs an argument"));
    }
    let arg = rest
        .strip_prefix('"')
        .and_then(|quoted| quoted.strip_suffix('"'))
        .unwrap_or(rest)
        .to_owned();
    Ok(match name {
        "deny" => Action::Deny(arg),
        "connect-to" => Action::ConnectTo(arg),
        "redirect" => Action::Redirect(arg),
        "limbo" => Action::Limbo(arg.split(',').map(str::to_owned).collect()),
        "notify" => Action::Notify(arg),
        "disconnect" => Action::Disconnect(arg),
        "modify" => Action::Modify(arg),
        "description" => Action::Description(arg),
        "custom" if matches!(arg.as_str(), "admin" | "player") => Action::Custom(arg),
        other => return Err(format!("unknown action {other:?} {arg:?}")),
    })
}

pub fn event_line(event: EventName, priority: u8, fields: &[&str]) -> String {
    let mut line = format!("{} @{priority}", event.as_str());
    for field in fields {
        line.push(' ');
        line.push_str(field);
    }
    line
}

pub fn command_line(name: &str, args: &[String], player: Option<u64>) -> String {
    let args = if args.is_empty() {
        "-".to_owned()
    } else {
        args.join(",")
    };
    let player = player.map_or_else(|| "-".to_owned(), |id| id.to_string());
    format!("cmd {name} {args} {player}")
}

pub fn observe(log: &Path, line: &str, action: &Action) {
    append(log, line);
    if *action == Action::Panic {
        panic!("scripted panic after `{line}`");
    }
}

pub fn append(log: &Path, line: &str) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .unwrap_or_else(|e| panic!("opening {}: {e}", log.display()));
    file.write_all(format!("{line}\n").as_bytes())
        .unwrap_or_else(|e| panic!("writing {}: {e}", log.display()));
}
