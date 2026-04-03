//! Premium auto-login: detect premium players and bypass password auth.

pub mod cache;
pub mod config;
pub mod detector;
pub mod lookup;

pub use cache::{PremiumCache, PremiumStatus};
pub use config::PremiumConfig;
pub use detector::PremiumDetector;
pub use lookup::MojangApiLookup;
