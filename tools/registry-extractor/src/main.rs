use std::path::PathBuf;

use clap::Parser;
use tracing_subscriber::EnvFilter;

mod extractor;

#[derive(Parser)]
#[command(name = "registry-extractor")]
#[command(
    about = "Captures Minecraft registry data from a vanilla server for use in Infrarust limbo"
)]
struct Cli {
    #[arg(short, long)]
    server: String,

    #[arg(short, long, default_value = "data/registry")]
    output: PathBuf,

    #[arg(short, long)]
    protocol_version: Option<i32>,

    #[arg(short, long, default_value = "RegExtractor")]
    username: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("registry_extractor=info".parse()?),
        )
        .init();

    let cli = Cli::parse();

    std::fs::create_dir_all(&cli.output)?;

    let result =
        extractor::extract_registry_data(&cli.server, &cli.username, cli.protocol_version).await?;

    let filename = format!("v{}.bin", result.protocol_version);
    let path = cli.output.join(&filename);
    let binary = result
        .to_binary()
        .map_err(|e| anyhow::anyhow!("Failed to serialize: {e}"))?;
    std::fs::write(&path, &binary)?;

    tracing::info!(
        path = %path.display(),
        protocol = result.protocol_version,
        version = %result.minecraft_version,
        registry_frames = result.registry_frames.len(),
        has_known_packs = result.known_packs_frame.is_some(),
        "Registry data extracted successfully"
    );

    let json_path = cli
        .output
        .join(format!("v{}.json", result.protocol_version));
    let json = serde_json::to_string_pretty(&result)?;
    std::fs::write(&json_path, &json)?;
    tracing::info!(path = %json_path.display(), "Debug JSON written");

    Ok(())
}
