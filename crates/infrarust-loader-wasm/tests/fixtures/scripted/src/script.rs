use std::io::Write;
use std::path::Path;

pub const SCRIPT_FILE: &str = "script.txt";
pub const LOG_FILE: &str = "log.txt";
pub const PACKET_ID: i32 = 0x05;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventName {
    PreLogin,
    PostLogin,
    Disconnect,
    OnlineAuthFailed,
    PermissionsSetup,
    ServerPreConnect,
    ServerConnected,
    ServerPostConnect,
    KickedFromServer,
    PlayerChooseInitialServer,
    ProxyPing,
    ProxyInitialize,
    ProxyShutdown,
    ConfigReload,
    ServerStateChange,
    ChatMessage,
    BackendHealth,
    Login,
    GameProfileRequest,
    CommandExecute,
    ConnectionHandshake,
    ConnectionRejected,
    LimboEnter,
    LimboExit,
    PlayerClientBrand,
    PlayerSettingsChanged,
    PlayerChannelRegister,
    PluginMessage,
    BanIssued,
    BanRevoked,
    PluginEnabled,
    PluginDisabled,
    PreTransfer,
    PlayerResourcePackStatus,
    NamedEvent,
    RawPacket,
}

impl EventName {
    pub const ALL: [Self; 36] = [
        Self::PreLogin,
        Self::PostLogin,
        Self::Disconnect,
        Self::OnlineAuthFailed,
        Self::PermissionsSetup,
        Self::ServerPreConnect,
        Self::ServerConnected,
        Self::ServerPostConnect,
        Self::KickedFromServer,
        Self::PlayerChooseInitialServer,
        Self::ProxyPing,
        Self::ProxyInitialize,
        Self::ProxyShutdown,
        Self::ConfigReload,
        Self::ServerStateChange,
        Self::ChatMessage,
        Self::BackendHealth,
        Self::Login,
        Self::GameProfileRequest,
        Self::CommandExecute,
        Self::ConnectionHandshake,
        Self::ConnectionRejected,
        Self::LimboEnter,
        Self::LimboExit,
        Self::PlayerClientBrand,
        Self::PlayerSettingsChanged,
        Self::PlayerChannelRegister,
        Self::PluginMessage,
        Self::BanIssued,
        Self::BanRevoked,
        Self::PluginEnabled,
        Self::PluginDisabled,
        Self::PreTransfer,
        Self::PlayerResourcePackStatus,
        Self::NamedEvent,
        Self::RawPacket,
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
            Self::ServerPostConnect => "server-post-connect",
            Self::KickedFromServer => "kicked-from-server",
            Self::PlayerChooseInitialServer => "player-choose-initial-server",
            Self::ProxyPing => "proxy-ping",
            Self::ProxyInitialize => "proxy-initialize",
            Self::ProxyShutdown => "proxy-shutdown",
            Self::ConfigReload => "config-reload",
            Self::ServerStateChange => "server-state-change",
            Self::ChatMessage => "chat-message",
            Self::BackendHealth => "backend-health",
            Self::Login => "login",
            Self::GameProfileRequest => "game-profile-request",
            Self::CommandExecute => "command-execute",
            Self::ConnectionHandshake => "connection-handshake",
            Self::ConnectionRejected => "connection-rejected",
            Self::LimboEnter => "limbo-enter",
            Self::LimboExit => "limbo-exit",
            Self::PlayerClientBrand => "player-client-brand",
            Self::PlayerSettingsChanged => "player-settings-changed",
            Self::PlayerChannelRegister => "player-channel-register",
            Self::PluginMessage => "plugin-message",
            Self::BanIssued => "ban-issued",
            Self::BanRevoked => "ban-revoked",
            Self::PluginEnabled => "plugin-enabled",
            Self::PluginDisabled => "plugin-disabled",
            Self::PreTransfer => "pre-transfer",
            Self::PlayerResourcePackStatus => "player-resource-pack-status",
            Self::NamedEvent => "named-event",
            Self::RawPacket => "raw-packet",
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
                    | Self::Login
                    | Self::CommandExecute
                    | Self::ConnectionHandshake
                    | Self::PreTransfer
            ),
            Action::Deny(_) => matches!(
                self,
                Self::PreLogin
                    | Self::ServerPreConnect
                    | Self::ChatMessage
                    | Self::Login
                    | Self::CommandExecute
                    | Self::ConnectionHandshake
                    | Self::PreTransfer
            ),
            Action::ForceOffline | Action::ForceOnline => self == Self::PreLogin,
            Action::ConnectTo(_) => self == Self::ServerPreConnect,
            Action::Redirect(_) => matches!(
                self,
                Self::PlayerChooseInitialServer | Self::KickedFromServer | Self::PreTransfer
            ),
            Action::Rename(_) => self == Self::GameProfileRequest,
            Action::ForwardToBackend => self == Self::CommandExecute,
            Action::Drop => matches!(self, Self::ConnectionHandshake | Self::RawPacket),
            Action::Forward | Action::Handled | Action::Replace(_) | Action::Reply(_) => {
                self == Self::PluginMessage
            }
            Action::Cancel | Action::Respond(_) => self == Self::NamedEvent,
            Action::Pass => self == Self::RawPacket,
            Action::Limbo(_) => matches!(
                self,
                Self::ServerPreConnect | Self::PlayerChooseInitialServer | Self::KickedFromServer
            ),
            Action::Notify(_) | Action::Disconnect(_) => self == Self::KickedFromServer,
            Action::Modify(_) => matches!(
                self,
                Self::ChatMessage | Self::CommandExecute | Self::RawPacket
            ),
            Action::Connect(_) => self == Self::ChatMessage,
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
    Rename(String),
    Connect(String),
    ForwardToBackend,
    Drop,
    Forward,
    Handled,
    Replace(String),
    Reply(String),
    Cancel,
    Respond(String),
    Pass,
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
    Fire {
        command: String,
        event: String,
        payload: String,
    },
    Named {
        name: String,
        priority: u8,
        action: Action,
    },
    Channel {
        id: String,
    },
    Config {
        key: String,
    },
    Plugin {
        id: String,
    },
    Connect {
        command: String,
        server: String,
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
            if name.is_empty() {
                return Err(CMD_FORMS.to_owned());
            }
            let (verb, rest) = word(rest);
            match verb {
                "record" if rest.trim().is_empty() => Ok(Directive::Cmd {
                    name: name.to_owned(),
                }),
                "fire" => {
                    let (event, rest) = word(rest);
                    if event.is_empty() {
                        return Err("`cmd <name> fire` needs an event name".to_owned());
                    }
                    Ok(Directive::Fire {
                        command: name.to_owned(),
                        event: event.to_owned(),
                        payload: unquote(rest.trim()),
                    })
                }
                "connect" => {
                    let (server, rest) = word(rest);
                    if server.is_empty() || !rest.trim().is_empty() {
                        return Err("expected `cmd <name> connect <server>`".to_owned());
                    }
                    Ok(Directive::Connect {
                        command: name.to_owned(),
                        server: server.to_owned(),
                    })
                }
                _ => Err(CMD_FORMS.to_owned()),
            }
        }
        "named" => {
            let (name, rest) = word(rest);
            if name.is_empty() {
                return Err("`named` needs an event name".to_owned());
            }
            let (priority, rest) = word(rest);
            let priority = parse_priority(priority)?;
            let action = parse_action(rest)?;
            if !(EventName::NamedEvent.accepts(&action)) {
                return Err(format!("named events cannot {action:?}"));
            }
            Ok(Directive::Named {
                name: name.to_owned(),
                priority,
                action,
            })
        }
        "channel" => {
            let (id, rest) = word(rest);
            if id.is_empty() || !rest.trim().is_empty() {
                return Err("expected `channel <namespace:name>`".to_owned());
            }
            Ok(Directive::Channel { id: id.to_owned() })
        }
        "config" => {
            let (key, rest) = word(rest);
            if key.is_empty() || !rest.trim().is_empty() {
                return Err("expected `config <key>`".to_owned());
            }
            Ok(Directive::Config {
                key: key.to_owned(),
            })
        }
        "plugin" => {
            let (id, rest) = word(rest);
            if id.is_empty() || !rest.trim().is_empty() {
                return Err("expected `plugin <id>`".to_owned());
            }
            Ok(Directive::Plugin { id: id.to_owned() })
        }
        other => Err(format!("unknown directive {other:?}")),
    }
}

const CMD_FORMS: &str = "expected `cmd <name> record`, `cmd <name> fire <event> \"<payload>\"` or `cmd <name> connect <server>`";

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
        "forward-to-backend" => Some(Action::ForwardToBackend),
        "drop" => Some(Action::Drop),
        "forward" => Some(Action::Forward),
        "handled" => Some(Action::Handled),
        "cancel" => Some(Action::Cancel),
        "pass" => Some(Action::Pass),
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
    let arg = unquote(rest);
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
        "rename" => Action::Rename(arg),
        "connect" => Action::Connect(arg),
        "replace" => Action::Replace(arg),
        "reply" => Action::Reply(arg),
        "respond" => Action::Respond(arg),
        other => return Err(format!("unknown action {other:?} {arg:?}")),
    })
}

fn unquote(text: &str) -> String {
    text.strip_prefix('"')
        .and_then(|quoted| quoted.strip_suffix('"'))
        .unwrap_or(text)
        .to_owned()
}

pub fn named_line(name: &str, priority: u8, fields: &[&str]) -> String {
    let mut line = format!("named {name} @{priority}");
    for field in fields {
        line.push(' ');
        line.push_str(field);
    }
    line
}

pub fn config_line(key: &str, value: Option<&str>) -> String {
    format!("config {key} {}", or_dash(value))
}

pub fn plugin_line(id: &str, state: Option<&str>) -> String {
    format!("plugin {id} {}", or_dash(state))
}

pub fn fired_line(command: &str, event: &str, cancelled: bool, response: Option<&str>) -> String {
    format!(
        "cmd {command} fired {event} {cancelled} {}",
        or_dash(response)
    )
}

pub fn text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    if text.is_empty() {
        "-".to_owned()
    } else {
        text.into_owned()
    }
}

pub fn event_line(event: EventName, priority: u8, fields: &[&str]) -> String {
    let mut line = format!("{} @{priority}", event.as_str());
    for field in fields {
        line.push(' ');
        line.push_str(field);
    }
    line
}

pub fn or_dash(value: Option<&str>) -> &str {
    value.unwrap_or("-")
}

pub fn joined<I, S>(items: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let items: Vec<String> = items
        .into_iter()
        .map(|item| item.as_ref().to_owned())
        .collect();
    if items.is_empty() {
        "-".to_owned()
    } else {
        items.join(",")
    }
}

pub fn connect_line(origin: &str, server: &str, outcome: &str) -> String {
    format!("{origin} connect {server} {outcome}")
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
