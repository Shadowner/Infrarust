use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")?;
    let dist = std::path::Path::new(&manifest_dir).join("frontend/.output/public");

    println!("cargo:rerun-if-changed=frontend/.output/public");

    if !dist.exists() {
        std::fs::create_dir_all(&dist)?;
        std::fs::write(
            dist.join("index.html"),
            concat!(
                "<!DOCTYPE html><html><head><title>Infrarust Admin</title></head>",
                "<body><h1>Frontend not built</h1>",
                "<p>Run <code>cd frontend &amp;&amp; npm install &amp;&amp; npx nuxt generate</code></p>",
                "</body></html>"
            ),
        )?;
    }
    Ok(())
}
