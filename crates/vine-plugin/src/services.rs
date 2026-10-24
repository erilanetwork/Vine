use std::sync::Arc;

use vine_config::{Config, SharedConfig};

/// The proxy wide state that commands, plugins and connections share. Everything here can
/// change while the proxy runs, so callers read it fresh instead of keeping copies.
pub struct ProxyServices {
    pub config: SharedConfig,
}

impl ProxyServices {
    pub fn new(config: SharedConfig) -> Self {
        Self { config }
    }

    /// Wraps a config that doesn't come from a file, mostly handy in tests
    pub fn from_config(config: Config) -> Arc<Self> {
        Arc::new(Self::new(SharedConfig::new(config, "vine.toml")))
    }

    pub fn config(&self) -> Arc<Config> {
        self.config.get()
    }
}
