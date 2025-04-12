use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AccessListConfig<T> {
    pub enabled: bool,
    pub whitelist: Vec<T>,
    pub blacklist: Vec<T>,
}
