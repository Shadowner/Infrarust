use smol::{io::AsyncWriteExt, net::TcpListener, spawn, stream::StreamExt};

use crate::core::{config::Config, connection_manager::ConnectionManager};

pub mod core;

pub struct InfraRust {
    config: Config,
    connection_manager: Vec<ConnectionManager>,
}

impl InfraRust {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            connection_manager: vec![],
        }
    }

    pub async fn run(&mut self) -> smol::io::Result<()> {
        let server = TcpListener::bind(&self.config.listen_addr).await?;
        println!("Server listening on {}", self.config.listen_addr);
        let mut incoming = server.incoming();
        while let Some(stream) = incoming.next().await {
            let stream = stream?;
            let mut conn_manager = ConnectionManager::new(stream);

            spawn(async move {
                println!("New connection established");
                conn_manager.handle_connection().await;
            })
            .detach();
        }

        Ok(())
    }
}
