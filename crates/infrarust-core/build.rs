use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;

use flate2::Compression;
use flate2::write::GzEncoder;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let registry_dir = Path::new(&manifest_dir).join("../../data/registry");

    println!("cargo:rerun-if-changed={}", registry_dir.display());

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest = Path::new(&out_dir).join("registry_bins.rs");
    let mut out = fs::File::create(&dest).unwrap();

    let mut gz_paths: Vec<String> = Vec::new();

    if let Ok(entries) = fs::read_dir(&registry_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("bin") {
                let raw = fs::read(&path).unwrap();
                let filename = path.file_name().unwrap().to_str().unwrap();

                let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
                encoder.write_all(&raw).unwrap();
                let compressed = encoder.finish().unwrap();

                let gz_name = format!("{filename}.gz");
                let gz_path = Path::new(&out_dir).join(&gz_name);
                fs::write(&gz_path, &compressed).unwrap();

                gz_paths.push(gz_path.display().to_string());
            }
        }
    }

    gz_paths.sort();

    writeln!(out, "const REGISTRY_BINS: &[&[u8]] = &[").unwrap();
    for gz in &gz_paths {
        writeln!(out, "    include_bytes!(\"{gz}\"),").unwrap();
    }
    writeln!(out, "];").unwrap();
}
