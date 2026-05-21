use serde::{Deserialize, Serialize};

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub schema_version: u32,
    pub base_currency: String,
    #[serde(default)]
    pub cache: CacheConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheConfig {
    #[serde(default = "default_scan_interval")]
    pub scan_interval: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            scan_interval: default_scan_interval(),
        }
    }
}

fn default_scan_interval() -> u64 {
    60
}
