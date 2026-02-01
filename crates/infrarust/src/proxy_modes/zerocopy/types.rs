use super::super::ProxyMessage;

#[derive(Debug)]
pub enum ZeroCopyMessage {}

impl ProxyMessage for ZeroCopyMessage {}
