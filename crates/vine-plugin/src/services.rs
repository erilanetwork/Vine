use std::sync::Arc;

use vine_config::{Config, SharedConfig};
use vine_moderation::Moderation;

use crate::health::HealthRegistry;
use crate::queue::QueueManager;

/// The proxy wide state that commands, plugins and connections share. Everything here can
/// change while the proxy runs, so callers read it fresh instead of keeping copies.
pub struct ProxyServices {
    pub config: SharedConfig,
    pub health: HealthRegistry,
    pub moderation: Moderation,
    pub queue: QueueManager,
}

impl ProxyServices {
    pub fn new(config: SharedConfig, moderation: Moderation) -> Self {
        Self {
            config,
            health: HealthRegistry::default(),
            moderation,
            queue: QueueManager::default(),
        }
    }

    /// Wraps a config that doesn't come from a file, with bans and whitelist kept in memory.
    /// Mostly handy in tests.
    pub fn from_config(config: Config) -> Arc<Self> {
        Arc::new(Self::new(
            SharedConfig::new(config, "vine.toml"),
            Moderation::in_memory(),
        ))
    }

    pub fn config(&self) -> Arc<Config> {
        self.config.get()
    }
}
