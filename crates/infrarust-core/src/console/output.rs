use std::io::IsTerminal;

use comfy_table::Table;
use comfy_table::{Attribute, CellAlignment, ContentArrangement, presets};

pub enum CommandOutput {
    Table {
        table: Table,
        footer: Option<String>,
    },
    Lines(Vec<OutputLine>),
    Success(String),
    Error(String),
    None,
}

pub enum OutputLine {
    Info(String),
    Success(String),
    Warning(String),
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandCategory {
    Players,
    Bans,
    Servers,
    Config,
    Plugins,
    System,
}

impl CommandCategory {
    pub const ALL: [CommandCategory; 6] = [
        Self::Players,
        Self::Bans,
        Self::Servers,
        Self::Config,
        Self::Plugins,
        Self::System,
    ];

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Players => "Players",
            Self::Bans => "Bans",
            Self::Servers => "Servers",
            Self::Config => "Configuration",
            Self::Plugins => "Plugins",
            Self::System => "System",
        }
    }
}

pub struct OutputRenderer {
    is_tty: bool,
}

impl OutputRenderer {
    pub fn new() -> Self {
        Self {
            is_tty: std::io::stdout().is_terminal(),
        }
    }

    #[cfg(test)]
    pub fn with_tty(is_tty: bool) -> Self {
        Self { is_tty }
    }

    pub fn is_tty(&self) -> bool {
        self.is_tty
    }

    pub async fn render(&self, output: CommandOutput) {
        match output {
            CommandOutput::Table { table, footer } => {
                let content = table.to_string();
                self.print_with_paging(&content, footer.as_deref()).await;
            }
            CommandOutput::Lines(lines) => {
                for line in &lines {
                    let formatted = self.format_line(line);
                    println!("{formatted}");
                }
            }
            CommandOutput::Success(msg) => {
                if self.is_tty {
                    let style = console::Style::new().green();
                    println!("  {} {}", style.apply_to("\u{2713}"), msg);
                } else {
                    println!("  OK: {msg}");
                }
            }
            CommandOutput::Error(msg) => {
                if self.is_tty {
                    let style = console::Style::new().red();
                    println!("  {} {}", style.apply_to("\u{2717}"), msg);
                } else {
                    println!("  ERROR: {msg}");
                }
            }
            CommandOutput::None => {}
        }
    }

    pub fn create_table(&self) -> Table {
        let mut table = Table::new();
        if self.is_tty {
            table
                .load_preset(presets::UTF8_FULL_CONDENSED)
                .set_content_arrangement(ContentArrangement::Dynamic);
        } else {
            table.load_preset(presets::ASCII_MARKDOWN);
            table.force_no_tty();
        }
        table
    }

    pub fn bold_header(&self, headers: Vec<&str>) -> Vec<comfy_table::Cell> {
        headers
            .into_iter()
            .map(|h| {
                let mut cell = comfy_table::Cell::new(h);
                if self.is_tty {
                    cell = cell
                        .add_attribute(Attribute::Bold)
                        .set_alignment(CellAlignment::Left);
                }
                cell
            })
            .collect()
    }

    pub fn header(&self, text: &str) -> String {
        if self.is_tty {
            format!(
                "{}",
                console::style(format!("=== {text} ===")).green().bold()
            )
        } else {
            format!("=== {text} ===")
        }
    }

    pub fn entity(&self, text: &str) -> String {
        if self.is_tty {
            format!("{}", console::style(text).cyan())
        } else {
            text.to_string()
        }
    }

    pub fn secondary(&self, text: &str) -> String {
        if self.is_tty {
            format!("{}", console::style(text).dim())
        } else {
            text.to_string()
        }
    }

    pub fn label(&self, text: &str) -> String {
        if self.is_tty {
            format!("{}", console::style(text).bold())
        } else {
            text.to_string()
        }
    }

    fn format_line(&self, line: &OutputLine) -> String {
        match line {
            OutputLine::Info(msg) => format!("  {msg}"),
            OutputLine::Success(msg) => {
                if self.is_tty {
                    let style = console::Style::new().green();
                    format!("  {} {msg}", style.apply_to("\u{2713}"))
                } else {
                    format!("  OK: {msg}")
                }
            }
            OutputLine::Warning(msg) => {
                if self.is_tty {
                    let style = console::Style::new().yellow();
                    format!("  {} {msg}", style.apply_to("!"))
                } else {
                    format!("  WARN: {msg}")
                }
            }
            OutputLine::Error(msg) => {
                if self.is_tty {
                    let style = console::Style::new().red();
                    format!("  {} {msg}", style.apply_to("\u{2717}"))
                } else {
                    format!("  ERROR: {msg}")
                }
            }
        }
    }

    async fn print_with_paging(&self, content: &str, footer: Option<&str>) {
        let line_count = content.lines().count();
        let terminal_height = terminal_height();

        if self.is_tty && line_count > terminal_height {
            let full = match footer {
                Some(f) => format!("{content}\n {f}"),
                None => content.to_string(),
            };
            let result = tokio::task::spawn_blocking(move || {
                let pager = minus::Pager::new();
                pager.set_text(full).ok();
                minus::page_all(pager)
            })
            .await;

            if let Ok(Err(e)) = result {
                tracing::debug!(error = %e, "pager failed, falling back to direct output");
                println!("{content}");
                if let Some(f) = footer {
                    println!(" {f}");
                }
            }
        } else {
            println!("{content}");
            if let Some(f) = footer {
                println!(" {f}");
            }
        }
    }
}

fn terminal_height() -> usize {
    console::Term::stdout()
        .size_checked()
        .map(|(h, _)| h as usize)
        .unwrap_or(24)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn test_create_table_non_tty_uses_ascii() {
        let renderer = OutputRenderer::with_tty(false);
        let table = renderer.create_table();
        let output = table.to_string();
        assert!(!output.contains('\u{2502}')); // │
    }

    #[test]
    fn test_command_category_display_names() {
        assert_eq!(CommandCategory::Players.display_name(), "Players");
        assert_eq!(CommandCategory::Bans.display_name(), "Bans");
        assert_eq!(CommandCategory::Servers.display_name(), "Servers");
        assert_eq!(CommandCategory::Config.display_name(), "Configuration");
        assert_eq!(CommandCategory::Plugins.display_name(), "Plugins");
        assert_eq!(CommandCategory::System.display_name(), "System");
    }

    #[test]
    fn test_format_line_non_tty_success() {
        let renderer = OutputRenderer::with_tty(false);
        let line = OutputLine::Success("done".to_string());
        let output = renderer.format_line(&line);
        assert_eq!(output, "  OK: done");
    }

    #[test]
    fn test_format_line_non_tty_error() {
        let renderer = OutputRenderer::with_tty(false);
        let line = OutputLine::Error("failed".to_string());
        let output = renderer.format_line(&line);
        assert_eq!(output, "  ERROR: failed");
    }

    #[test]
    fn test_format_line_non_tty_warning() {
        let renderer = OutputRenderer::with_tty(false);
        let line = OutputLine::Warning("careful".to_string());
        let output = renderer.format_line(&line);
        assert_eq!(output, "  WARN: careful");
    }

    #[test]
    fn test_format_line_non_tty_info() {
        let renderer = OutputRenderer::with_tty(false);
        let line = OutputLine::Info("some info".to_string());
        let output = renderer.format_line(&line);
        assert_eq!(output, "  some info");
    }
}
