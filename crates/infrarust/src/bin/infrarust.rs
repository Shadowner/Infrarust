use infrarust::InfraRust;
use macro_rules_attribute::apply;
use smol_macros::main;
use infrarust::core::config::Config;

#[apply(main!)]
async fn main() {
    let mut app = InfraRust::new(Config {
        listen_addr: "0.0.0.0:8080".to_string(),
        backend_addr: "192.168.1.235:25566".to_string(),
        max_connections: 100,
        timeout: 60, // in seconds
    });

    app.run().await;
}
