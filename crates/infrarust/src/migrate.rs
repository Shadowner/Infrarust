use std::path::Path;
use std::process::ExitCode;

use infrarust_config::migrate::MigrationSeverity;

pub fn run(input: &Path, output: &Path) -> ExitCode {
    println!("Migrating V1 configs from: {}", input.display());
    println!("Output directory: {}", output.display());
    println!();

    match infrarust_config::migrate::migrate_directory(input, output) {
        Ok(warnings) => {
            let mut has_errors = false;

            for w in &warnings {
                if w.file == "summary" {
                    continue;
                }
                let prefix = match w.severity {
                    MigrationSeverity::Error => {
                        has_errors = true;
                        "ERROR"
                    }
                    MigrationSeverity::Warning => "WARN ",
                    MigrationSeverity::Info => "INFO ",
                };
                println!("  {prefix} [{:>30}] {}", w.file, w.message);
            }

            if let Some(summary) = warnings.iter().find(|w| w.file == "summary") {
                println!("\n{}", summary.message);
                if summary.message.starts_with("0 file(s) converted") {
                    has_errors = true;
                }
            }

            if has_errors {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            eprintln!("Migration failed: {e}");
            ExitCode::FAILURE
        }
    }
}
