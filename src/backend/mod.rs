pub mod bridge;
mod client_queue;
mod connection;
pub mod forwarding;
mod login;
mod readers;
mod suggestions;
mod switch;
mod task;
mod tunnel;

pub use bridge::{BackendBridge, BridgeError};

use std::sync::Arc;
use vine_config::{BackendConfig, normalize_host};
use vine_plugin::services::ProxyServices;

pub struct Router {
    services: Arc<ProxyServices>,
}

impl Router {
    pub fn new(services: Arc<ProxyServices>) -> Self {
        Self { services }
    }

    /// Resolves which backend server to connect to based on virtual host or default
    pub fn resolve_server(&self, virtual_host: &str) -> Option<(String, BackendConfig)> {
        let config = self.services.config();
        let (routing, servers) = (&config.routing, &config.servers);
        if let Some(target_name) = routing.forced_hosts.get(&normalize_host(virtual_host))
            && let Some(cfg) = servers.get(target_name)
        {
            return Some((target_name.clone(), cfg.clone()));
        }

        if let Some(cfg) = servers.get(&routing.default_server) {
            return Some((routing.default_server.clone(), cfg.clone()));
        }

        if let Some(fallback_name) = &routing.fallback_server
            && let Some(cfg) = servers.get(fallback_name)
        {
            return Some((fallback_name.clone(), cfg.clone()));
        }

        servers
            .iter()
            .next()
            .map(|(name, cfg)| (name.clone(), cfg.clone()))
    }
}
