use crate::cli::ShutdownController;
use crate::cli::format as fmt;
use atty::Stream;
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::debug;

pub trait Command: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn execute(&self, args: Vec<String>) -> CommandFuture;
}

pub type CommandFuture = std::pin::Pin<Box<dyn std::future::Future<Output = String> + Send>>;
pub type CommandResult = String;

pub enum CommandMessage {
    Execute(String),
    Shutdown,
}

pub struct CommandProcessor {
    commands: HashMap<String, Arc<dyn Command>>,
    tx: mpsc::Sender<CommandMessage>,
    shutdown_controller: Option<Arc<ShutdownController>>,
}

impl CommandProcessor {
    pub fn new(
        commands: Vec<Arc<dyn Command>>,
        shutdown_controller: Option<Arc<ShutdownController>>,
    ) -> (Self, mpsc::Receiver<CommandMessage>) {
        let (tx, rx) = mpsc::channel(32);
        let mut command_map = HashMap::new();

        for cmd in commands {
            command_map.insert(cmd.name().to_string(), cmd);
        }

        (
            Self {
                commands: command_map,
                tx,
                shutdown_controller,
            },
            rx,
        )
    }

    pub fn register_command(&mut self, command: Arc<dyn Command>) {
        self.commands.insert(command.name().to_string(), command);
    }

    pub async fn process_command(&self, input: &str) -> CommandResult {
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.is_empty() {
            return "No command entered".to_string();
        }

        let command_name = parts[0].to_lowercase();
        if command_name == "help" {
            return self.get_help();
        }

        match self.commands.get(&command_name) {
            Some(cmd) => {
                let args: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();
                cmd.execute(args).await
            }
            None => format!("Unknown command: {}", command_name),
        }
    }

    fn get_help(&self) -> String {
        let mut help = format!("{}\n\n", fmt::header("Available Commands"));

        for (name, cmd) in &self.commands {
            help.push_str(&format!(
                "  {} - {}\n",
                fmt::entity(name),
                fmt::secondary(cmd.description())
            ));
        }

        help.push_str(&format!(
            "  {} - {}\n",
            fmt::entity("help"),
            fmt::secondary("Show this help message")
        ));

        help.push_str(&format!(
            "  {} - {}",
            fmt::entity("exit/quit"),
            fmt::secondary("Exit the program")
        ));

        help
    }

    pub fn sender(&self) -> mpsc::Sender<CommandMessage> {
        self.tx.clone()
    }

    pub async fn start_input_loop(&self) {
        let is_tty = atty::is(Stream::Stdin);

        if !is_tty {
            debug!("stdin is not a TTY, using simplified input handling");
        }
        let tx = self.tx.clone();

        // If we have a shutdown controller, get a receiver
        let shutdown_rx = if let Some(controller) = &self.shutdown_controller {
            Some(controller.subscribe().await)
        } else {
            None
        };

        // Signal the blocking thread to terminate
        let (terminate_tx, terminate_rx) = tokio::sync::watch::channel(false);

        if let Some(mut rx) = shutdown_rx {
            let tx = terminate_tx.clone();
            tokio::spawn(async move {
                if rx.recv().await.is_ok() {
                    // Signal termination to the blocking thread
                    let _ = tx.send(true);
                }
            });
        }

        tokio::task::spawn_blocking(move || {
            let stdin = io::stdin();

            let mut reader = std::io::BufReader::new(stdin);
            let mut buffer = String::new();
            let terminate_watcher = terminate_rx;

            loop {
                if *terminate_watcher.borrow() {
                    debug!("CLI input loop received shutdown signal, terminating");
                    break;
                }

                if is_tty {
                    print!("> ");
                    io::stdout().flush().unwrap();
                }

                buffer.clear();
                if reader.read_line(&mut buffer).is_err() {
                    if !is_tty {
                        // HACK: In non-TTY mode, don't spin at 100% CPU on errors
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                    continue;
                }

                let input = buffer.trim();
                if input.is_empty() {
                    continue;
                }

                if input == "exit" || input == "quit" {
                    let _ = futures::executor::block_on(tx.send(CommandMessage::Shutdown));
                    break;
                }

                let _ = futures::executor::block_on(
                    tx.send(CommandMessage::Execute(input.to_string())),
                );
            }
        });
    }
}
