use bytes::{BytesMut, buf};
use smol::{
    future::or,
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

use crate::core::sans_io::{Decision, decision};

pub struct ConnectionManager {
    client: TcpStream,
    backend_stream: Option<TcpStream>,
}

impl ConnectionManager {
    pub fn new(client: TcpStream) -> Self {
        Self {
            client,
            backend_stream: None,
        }
    }

    pub fn set_backend_stream(&mut self, stream: TcpStream) {
        self.backend_stream = Some(stream);
    }

    pub fn get_client(&self) -> &TcpStream {
        &self.client
    }

    pub fn get_backend_stream(&self) -> Option<&TcpStream> {
        self.backend_stream.as_ref()
    }

    pub async fn handle_connection(&mut self) {
        println!("Handling connection for client: {:?}", self.client);
        let mut buf = vec![0; 2097152];

        while let Ok(size) = self.client.read(&mut buf).await {
            if size != 0 {
                println!("Read {} bytes from client", size);
                let data = BytesMut::from(&buf[..size]);
                let decision = decision(&data);
                self.handle_decision(&decision).await;
            } else {
                println!("Client disconnected");
                break;
            }
        }
    }

    async fn handle_decision(&mut self, decision: &Decision) {
        match decision {
            Decision::ConnectToBackend(ip, port, data) => {
                println!("Connecting to backend at {}:{}", ip, port);
                let tcp_stream = TcpStream::connect((*ip, *port)).await;
                match tcp_stream {
                    Ok(stream) => {
                        self.set_backend_stream(stream);
                        println!("Connected to backend successfully");
                        // Here you would implement the logic to send data to the backend.
                        if let Some(backend_stream) = self.backend_stream.as_mut() {
                            backend_stream.write_all(data).await.unwrap();
                            println!("Data sent to backend");
                        }
                        self.handle_proxy().await;
                    }
                    Err(e) => {
                        println!("Failed to connect to backend: {}", e);
                    }
                }
            }
            Decision::PassThrough(data) => {
                println!("Passing through data without action");
                let backend_stream = self.backend_stream.take();
                if backend_stream.is_none() {
                    println!("No backend stream available for pass-through");
                    return;
                }
                let mut backend_stream = backend_stream.unwrap();

                backend_stream.write_all(&data).await.unwrap();
            }
        }
    }

    async fn handle_proxy(&mut self) {
        let backend_stream = self.backend_stream.as_mut();
        if backend_stream.is_none() {
            println!("No backend server available for proxying");
            return;
        }
        let mut buf1 = vec![0; 2097152];
        let mut buf2 = vec![0; 2097152];
        let backend_stream = backend_stream.unwrap();

        while !buf1.is_empty() || !buf2.is_empty() {
            println!("Waiting for data from client or backend");
            let packet = or(
                backend_stream.read(&mut buf1),
                self.client.read(&mut buf2),
            )
            .await;
            println!("Data read from streams");
            if buf1.is_empty() && buf2.is_empty() || packet.is_err() || packet.unwrap() == 0 {
                println!("Both streams closed");
                return;
            }

            if !buf1.is_empty() {
                println!("Read {} bytes from backend", buf1.len());
                let data = BytesMut::from(&buf1[..]);
                let decision = decision(&data);
                if let Decision::PassThrough(data) = decision {
                    self.client.write_all(&data).await.unwrap();
                }
            }

            if !buf2.is_empty() {
                println!("Read {} bytes from client", buf2.len());
                let data = BytesMut::from(&buf2[..]);
                let decision = decision(&data);
                if let Decision::PassThrough(data) = decision {
                    backend_stream.write_all(&data).await.unwrap();
                    println!("Data sent to backend from client");
                }
            }
        }
        print!("Connection closed, cleaning up resources\n");
    }
}
