use std::env;
use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::Path;

use flate2::Compression;
use flate2::write::GzEncoder;

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR")?;
    let registry_dir = Path::new(&manifest_dir).join("registry");

    println!("cargo:rerun-if-changed={}", registry_dir.display());

    let out_dir = env::var("OUT_DIR")?;
    let dest = Path::new(&out_dir).join("registry_bins.rs");
    let mut out = fs::File::create(&dest)?;

    let mut gz_paths: Vec<String> = Vec::new();

    for entry in fs::read_dir(&registry_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("bin") {
            continue;
        }
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("registry file name is not UTF-8: {}", path.display()))?;
        let raw = fs::read(&path)?;

        let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
        encoder.write_all(&raw)?;
        let compressed = encoder.finish()?;

        let gz_path = Path::new(&out_dir).join(format!("{filename}.gz"));
        fs::write(&gz_path, &compressed)?;

        gz_paths.push(gz_path.display().to_string());
    }

    if gz_paths.is_empty() {
        return Err(format!("no registry .bin files found in {}", registry_dir.display()).into());
    }
    gz_paths.sort();

    writeln!(out, "const REGISTRY_BINS: &[&[u8]] = &[")?;
    for gz in &gz_paths {
        writeln!(out, "    include_bytes!({gz:?}),")?;
    }
    writeln!(out, "];")?;
    Ok(())
}
