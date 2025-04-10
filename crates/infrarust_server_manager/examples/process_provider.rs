use async_trait::async_trait;
use infrarust_server_manager::{
    LocalProvider, ProcessProvider, ServerManager, ServerManagerError, local::LocalServerConfig,
};
use tokio::sync::mpsc;
use tokio::time::{Duration, sleep};

/// Example of a custom process provider that doesn't rely on the default ProcessManager
#[derive(Clone)]
struct CustomProcessProvider {
    // In reality, this might be a client for a remote system
    local_provider: LocalProvider,
}

impl CustomProcessProvider {
    fn new(provider: LocalProvider) -> Self {
        Self {
            local_provider: provider,
        }
    }

    // Register a server to manage
    fn register_server(&self, server_id: &str, config: LocalServerConfig) {
        self.local_provider.register_server(server_id, config);
    }
}

/// Implement ProcessProvider for our custom provider
#[async_trait]
impl ProcessProvider for CustomProcessProvider {
    // We delegate to the local provider, but this could use any custom implementation
    // like Pterodactyl's websocket API for real-time interaction
    async fn write_stdin(&self, server_id: &str, input: &str) -> Result<(), ServerManagerError> {
        println!("CustomProvider: writing to stdin: {}", input);
        self.local_provider.write_stdin(server_id, input).await
    }

    fn get_stdout_stream(
        &self,
        server_id: &str,
    ) -> Result<mpsc::Receiver<String>, ServerManagerError> {
        println!("CustomProvider: getting stdout stream");
        self.local_provider.get_stdout_stream(server_id)
    }

    fn is_process_running(&self, server_id: &str) -> Result<bool, ServerManagerError> {
        self.local_provider.is_process_running(server_id)
    }

    async fn stop_process(&self, server_id: &str) -> Result<(), ServerManagerError> {
        println!("CustomProvider: stopping process");
        self.local_provider.stop_process(server_id).await
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    {
        println!("=== Example 1: Using LocalProvider as both API and Process provider ===");

        // Create a local provider that handles both API and process interaction
        let local_provider = LocalProvider::new();

        // Register a simple echo server
        let server_config = LocalServerConfig {
            #[cfg(target_os = "windows")]
            executable: "cmd".to_string(),
            #[cfg(target_os = "windows")]
            args: vec!["/K", "echo Server starting..."]
                .iter()
                .map(|s| s.to_string())
                .collect(),

            #[cfg(not(target_os = "windows"))]
            executable: "bash".to_string(),
            #[cfg(not(target_os = "windows"))]
            args: [
                "-c",
                "echo 'Server starting...' && while read line; do echo \"Server: $line\"; done",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),

            working_dir: None,
        };

        // Register the server with an ID
        local_provider.register_server("echo-server", server_config);

        // Create a server manager with the local provider as both API and process provider
        let server_manager =
            ServerManager::new(local_provider.clone()).with_process_provider(local_provider);

        // Start the server
        server_manager.start_server("echo-server").await?;
        println!("Server started");

        // Get a stream of stdout
        let mut stdout_stream = server_manager.get_stdout_stream("echo-server")?;

        // Spawn a task to print server output
        let _output_task = tokio::spawn(async move {
            while let Some(line) = stdout_stream.recv().await {
                println!("Output: {}", line);
            }
        });

        sleep(Duration::from_secs(1)).await;

        // Send some commands to the server
        server_manager
            .write_stdin("echo-server", "Hello from direct provider!")
            .await?;
        sleep(Duration::from_secs(1)).await;

        // Cleanup
        server_manager.stop_server("echo-server").await?;
        sleep(Duration::from_secs(2)).await;
    }
    {
        println!("\n=== Example 2: Using a custom process provider ===");

        // Create a custom process provider

        let server_config = LocalServerConfig {
        #[cfg(target_os = "windows")]
        executable: "cmd".to_string(),
        #[cfg(target_os = "windows")]
        args: vec!["/K", "echo Custom server starting..."]
            .iter()
            .map(|s| s.to_string())
            .collect(),

        #[cfg(not(target_os = "windows"))]
        executable: "bash".to_string(),
        #[cfg(not(target_os = "windows"))]
        args: ["-c",
            "echo 'Custom server starting...' && while read line; do echo \"Custom: $line\"; done"]
        .iter()
        .map(|s| s.to_string())
        .collect(),

        working_dir: None,
    };

        let local_provider = LocalProvider::new();
        let custom_provider = CustomProcessProvider::new(local_provider.clone());

        // Register the server in our custom provider
        custom_provider.register_server("custom-server", server_config);

        // Create a server manager using LocalProvider for API and CustomProcessProvider for process interaction
        let server_manager =
            ServerManager::new(local_provider).with_process_provider(custom_provider.clone());

        // Start the server through the API provider
        println!("Starting server: custom-server");
        server_manager.start_server("custom-server").await?;
        println!("Custom server started");

        // Get a stream of stdout through our custom provider - now it should work!
        let mut stdout_stream = server_manager.get_stdout_stream("custom-server")?;

        // Spawn a task to print server output
        let _output_task = tokio::spawn(async move {
            while let Some(line) = stdout_stream.recv().await {
                println!("Custom output: {}", line);
            }
        });

        sleep(Duration::from_secs(1)).await;

        // Send some commands to the server through our custom provider
        server_manager
            .write_stdin("custom-server", "Hello from custom provider!")
            .await?;
        sleep(Duration::from_secs(1)).await;

        // Cleanup
        server_manager.stop_server("custom-server").await?;

        sleep(Duration::from_secs(2)).await;

        println!("Example completed successfully!");

        Ok(())
    }
}
