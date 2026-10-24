pub mod bridge;
mod client_queue;
mod connection;
pub mod forwarding;
mod health;
mod login;
mod readers;
mod suggestions;
mod switch;
mod task;
mod tunnel;

pub use bridge::{BackendBridge, BridgeError};
pub use health::spawn_health_checks;

use std::sync::Arc;
use vine_config::{BackendConfig, normalize_host};
use vine_plugin::services::ProxyServices;
use vine_plugin::session::SessionManager;

pub struct Router {
    services: Arc<ProxyServices>,
    session_manager: Arc<SessionManager>,
}

impl Router {
    pub fn new(services: Arc<ProxyServices>, session_manager: Arc<SessionManager>) -> Self {
        Self {
            services,
            session_manager,
        }
    }

    /// Picks the backend for a new player: the forced host's server, then the default, then the
    /// fallback, skipping any that failed their health checks or are full. With none of them
    /// usable it still returns the first choice, so the player gets the usual connect error
    /// instead of nothing.
    pub fn resolve_server(&self, virtual_host: &str) -> Option<(String, BackendConfig)> {
        let config = self.services.config();
        let (routing, servers) = (&config.routing, &config.servers);
        let preferred: Vec<&String> = routing
            .forced_hosts
            .get(&normalize_host(virtual_host))
            .into_iter()
            .chain(std::iter::once(&routing.default_server))
            .chain(routing.fallback_server.as_ref())
            .filter(|name| servers.contains_key(*name))
            .collect();

        let health = &self.services.health;
        let sessions = self.session_manager.list_sessions();
        let usable = |name: &&String| {
            !health.is_down(name) && !self.services.queue.is_full(&config, name, &sessions)
        };
        let pick = preferred
            .iter()
            .copied()
            .find(usable)
            .or_else(|| servers.keys().find(usable))
            .or_else(|| preferred.first().copied())
            .or_else(|| servers.keys().next())?;
        servers.get(pick).map(|cfg| (pick.clone(), cfg.clone()))
    }
}
