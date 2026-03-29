use std::path::Path;
use std::process::ExitCode;

use infrarust_config::migrate::{MigrationSeverity, MigrationWarning};

pub fn run(input: &Path, output: &Path, config: Option<&Path>) -> ExitCode {
    let mut has_errors = false;

    if let Some(config_path) = config {
        println!("Migrating V1 global config: {}", config_path.display());
        let out_file = output.join("infrarust.toml");
        match infrarust_config::migrate::migrate_proxy_config(config_path, &out_file) {
            Ok(warnings) => {
                print_warnings(&warnings, &mut has_errors);
                println!("  -> {}\n", out_file.display());
            }
            Err(e) => {
                eprintln!("Global config migration failed: {e}");
                has_errors = true;
            }
        }
    }

    println!("Migrating V1 server configs from: {}", input.display());
    println!("Output directory: {}", output.display());
    println!();

    match infrarust_config::migrate::migrate_directory(input, output) {
        Ok(warnings) => {
            print_warnings(&warnings, &mut has_errors);

            if let Some(summary) = warnings.iter().find(|w| w.file == "summary") {
                println!("\n{}", summary.message);
                if summary.message.starts_with("0 file(s) converted") {
                    has_errors = true;
                }
            }
        }
        Err(e) => {
            eprintln!("Server config migration failed: {e}");
            has_errors = true;
        }
    }

    if has_errors {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn print_warnings(warnings: &[MigrationWarning], has_errors: &mut bool) {
    for w in warnings {
        if w.file == "summary" {
            continue;
        }
        let prefix = match w.severity {
            MigrationSeverity::Error => {
                *has_errors = true;
                "ERROR"
            }
            MigrationSeverity::Warning => "WARN ",
            MigrationSeverity::Info => "INFO ",
        };
        println!("  {prefix} [{:>30}] {}", w.file, w.message);
    }
}
