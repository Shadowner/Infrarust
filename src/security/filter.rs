use std::{io, sync::Arc, time::Duration};

use async_trait::async_trait;
use serde::Deserialize;
use tokio::net::TcpStream;

#[async_trait]
pub trait Filter: Send + Sync {
    async fn filter(&self, stream: &TcpStream) -> io::Result<()>;
}

#[derive(Default, Clone)]
pub struct FilterChain {
    filters: Vec<Arc<dyn Filter>>,
}

impl FilterChain {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_filter<F: Filter + 'static>(&mut self, filter: F) {
        self.filters.push(Arc::new(filter));
    }

    pub async fn filter(&self, stream: &TcpStream) -> io::Result<()> {
        for filter in &self.filters {
            filter.filter(stream).await?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct FilterConfig {
    pub rate_limiter: Option<RateLimiterConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RateLimiterConfig {
    pub request_limit: u32,
    pub window_length: Duration,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestFilter {
        should_fail: bool,
    }

    #[async_trait]
    impl Filter for TestFilter {
        async fn filter(&self, _: &TcpStream) -> io::Result<()> {
            if self.should_fail {
                Err(io::Error::new(io::ErrorKind::Other, "filter failed"))
            } else {
                Ok(())
            }
        }
    }

    #[tokio::test]
    async fn test_filter_chain() {
        let mut chain = FilterChain::new();
        chain.add_filter(TestFilter { should_fail: false });
        chain.add_filter(TestFilter { should_fail: false });

        // Set up a listener first
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // Spawn a task to accept one connection
        let accept_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            socket
        });

        // Connect to the listener
        let stream = TcpStream::connect(addr).await.unwrap();

        // Get the server side of the connection (don't need it, but must accept it)
        let _server_stream = accept_task.await.unwrap();

        // Test the filter chain
        assert!(chain.filter(&stream).await.is_ok());

        chain.add_filter(TestFilter { should_fail: true });
        assert!(chain.filter(&stream).await.is_err());
    }
}
