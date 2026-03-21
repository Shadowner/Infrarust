use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let registry_dir = Path::new(&manifest_dir).join("../../data/registry");

    println!("cargo:rerun-if-changed={}", registry_dir.display());

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest = Path::new(&out_dir).join("registry_bins.rs");
    let mut out = fs::File::create(&dest).unwrap();

    let mut bins: Vec<String> = Vec::new();

    if let Ok(entries) = fs::read_dir(&registry_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("bin") {
                let abs = fs::canonicalize(&path).unwrap();
                bins.push(abs.display().to_string());
            }
        }
    }

    bins.sort();

    writeln!(out, "const REGISTRY_BINS: &[&[u8]] = &[").unwrap();
    for bin in &bins {
        writeln!(out, "    include_bytes!(\"{bin}\"),").unwrap();
    }
    writeln!(out, "];").unwrap();
}
