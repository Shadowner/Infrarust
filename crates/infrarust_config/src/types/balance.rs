//! Load-balancing configuration: strategy, weighted addresses, slow-start.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::ServerAddress;

/// Address selection strategy among a server's backend addresses.
///
/// Every strategy is weight-aware: with all weights equal (the default),
/// each degenerates into its unweighted form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BalanceStrategy {
    /// V1 parity: addresses tried in config order. Migration-safe default.
    #[default]
    FirstAvailable,
    /// Smooth weighted round-robin (NGINX-style SWRR).
    RoundRobin,
    /// Weighted least-connections — recommended for multi-address servers.
    LeastConn,
}

impl BalanceStrategy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FirstAvailable => "first_available",
            Self::RoundRobin => "round_robin",
            Self::LeastConn => "least_conn",
        }
    }
}

impl fmt::Display for BalanceStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

pub(crate) fn default_aggression() -> f64 {
    1.0
}

/// Normalized load-balancing settings of a server.
///
/// Built from the flat `balance` / `slow_start` / `slow_start_aggression`
/// keys of a server config (see [`crate::ServerConfig::balance_config`])
/// and consumed by the core load balancer factory.
#[derive(Debug, Clone, PartialEq)]
pub struct BalanceConfig {
    pub strategy: BalanceStrategy,
    /// Slow-start ramp window. `None` = disabled.
    pub slow_start: Option<Duration>,
    /// Slow-start curve: 1.0 = linear, > 1.0 = softer start.
    pub slow_start_aggression: f64,
}

impl Default for BalanceConfig {
    fn default() -> Self {
        Self {
            strategy: BalanceStrategy::default(),
            slow_start: None,
            slow_start_aggression: default_aggression(),
        }
    }
}

/// One backend address entry with its static weight.
///
/// Deserializes from either a plain string (`"10.0.0.1:25565"`, weight 1)
/// or a table (`{ address = "10.0.0.1:25565", weight = 3 }`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WeightedAddress {
    pub address: ServerAddress,
    pub weight: u32,
}

pub(crate) const DEFAULT_WEIGHT: u32 = 1;

impl From<ServerAddress> for WeightedAddress {
    fn from(address: ServerAddress) -> Self {
        Self {
            address,
            weight: DEFAULT_WEIGHT,
        }
    }
}

impl std::str::FromStr for WeightedAddress {
    type Err = <ServerAddress as std::str::FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<ServerAddress>().map(Into::into)
    }
}

impl fmt::Display for WeightedAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.weight == DEFAULT_WEIGHT {
            write!(f, "{}", self.address)
        } else {
            write!(f, "{} (weight {})", self.address, self.weight)
        }
    }
}

const fn default_weight() -> u32 {
    DEFAULT_WEIGHT
}

/// Serde-facing form of an address entry: bare string or `{ address, weight }`.
#[derive(Deserialize)]
#[serde(untagged)]
enum AddressEntry {
    Simple(ServerAddress),
    Weighted(WeightedEntry),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WeightedEntry {
    address: ServerAddress,
    #[serde(default = "default_weight")]
    weight: u32,
}

impl<'de> Deserialize<'de> for WeightedAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        match AddressEntry::deserialize(deserializer)? {
            AddressEntry::Simple(address) => Ok(address.into()),
            AddressEntry::Weighted(WeightedEntry { address, weight }) => {
                Ok(Self { address, weight })
            }
        }
    }
}

impl Serialize for WeightedAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if self.weight == DEFAULT_WEIGHT {
            self.address.serialize(serializer)
        } else {
            use serde::ser::SerializeStruct;
            let mut st = serializer.serialize_struct("WeightedAddress", 2)?;
            st.serialize_field("address", &self.address)?;
            st.serialize_field("weight", &self.weight)?;
            st.end()
        }
    }
}
