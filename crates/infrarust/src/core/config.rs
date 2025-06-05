pub struct Config {
    pub listen_addr: String,
    pub backend_addr: String,
    pub max_connections: usize,
    pub timeout: u64, // in seconds
}