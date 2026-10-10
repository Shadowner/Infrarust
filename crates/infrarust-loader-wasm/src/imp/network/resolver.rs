use std::future::Future;
use std::io;
use std::net::IpAddr;
use std::pin::Pin;

pub(crate) type ResolveFuture = Pin<Box<dyn Future<Output = io::Result<Vec<IpAddr>>> + Send>>;

pub(crate) trait Resolver: Send + Sync {
    fn resolve(&self, name: &str) -> ResolveFuture;
}

pub(crate) struct SystemResolver;

impl Resolver for SystemResolver {
    fn resolve(&self, name: &str) -> ResolveFuture {
        let name = name.to_owned();
        Box::pin(async move {
            let addrs = tokio::net::lookup_host((name.as_str(), 0)).await?;
            Ok(addrs.map(|addr| addr.ip()).collect())
        })
    }
}
