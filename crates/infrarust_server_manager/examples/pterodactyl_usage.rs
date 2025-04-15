use infrarust_server_manager::{PterodactylClient, ServerManager};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a Pterodactyl API client
    let api_key = std::env::var("PTERODACTYL_API_KEY")
        .expect("PTERODACTYL_API_KEY environment variable must be set");
    let base_url = std::env::var("PTERODACTYL_URL")
        .unwrap_or_else(|_| "https://ptero.example.com".to_string());
    let server_id = std::env::var("SERVER_ID").unwrap_or_else(|_| "3dafa09d".to_string());

    // Create the API client
    let api_client = PterodactylClient::new(api_key, base_url);

    // Initialize the server manager with 15 second check interval
    let server_manager =
        ServerManager::new(api_client).with_check_interval(Duration::from_secs(15));

    println!("Starting example application");

    // Start the server if it's not running
    match server_manager.start_server(&server_id).await {
        Ok(_) => println!("Server start command sent successfully"),
        Err(e) => println!("Failed to start server: {}", e),
    }

    // Execute a system command (example: list files)
    #[cfg(target_os = "windows")]
    let command = "dir";
    #[cfg(target_os = "linux")]
    let command = "ls -la";

    match server_manager.execute_system_command(command) {
        Ok(output) => println!("Command output: \n{}", output),
        Err(e) => println!("Command execution error: {}", e),
    }

    server_manager.stop_server(&server_id).await?;
    tokio::time::sleep(Duration::from_secs(5)).await;
    server_manager.start_server(&server_id).await?;

    // Monitor the server for 2 minutes
    println!("Monitoring server for 120 seconds...");
    let _monitor_handle =
        tokio::spawn(async move { server_manager.monitor_server(&server_id).await });

    tokio::time::sleep(Duration::from_secs(120)).await;

    // We'll never reach here in the real application as monitor_server() runs indefinitely
    // This is just for the example
    println!("Example completed");

    Ok(())
}
