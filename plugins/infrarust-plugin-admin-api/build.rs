use std::error::Error;
use std::path::{Path, PathBuf};

const FRONTEND_DIST: &str = "frontend/.output/public";

const PLACEHOLDER_INDEX: &str = concat!(
    "<!DOCTYPE html><html><head><title>Infrarust Admin</title></head>",
    "<body><h1>Frontend not built</h1>",
    "<p>Run <code>cd frontend &amp;&amp; npm install &amp;&amp; npx nuxt generate</code></p>",
    "</body></html>"
);

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let dist = manifest_dir.join(FRONTEND_DIST);

    println!("cargo:rerun-if-changed={FRONTEND_DIST}");

    let embed_dir = if dist.join("index.html").is_file() {
        dist
    } else {
        write_placeholder(&PathBuf::from(std::env::var("OUT_DIR")?))?
    };

    println!(
        "cargo:rustc-env=INFRARUST_ADMIN_FRONTEND_DIR={}",
        embed_dir.display()
    );
    Ok(())
}

fn write_placeholder(out_dir: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let placeholder = out_dir.join("frontend-placeholder");
    std::fs::create_dir_all(&placeholder)?;
    std::fs::write(placeholder.join("index.html"), PLACEHOLDER_INDEX)?;
    Ok(placeholder)
}
