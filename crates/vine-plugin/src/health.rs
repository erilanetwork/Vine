use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

/// What the last health checks said about one backend
#[derive(Clone, Debug)]
pub struct ServerHealth {
    pub online: bool,
    pub latency: Option<Duration>,
    /// Online and max players as the backend reports them
    pub players: Option<(u32, u32)>,
    pub checked_at: Instant,
    failures: u32,
}

/// Health of every backend, filled in by the proxy's health checker. A server nobody checked
/// yet counts as up, so turning health checks off changes nothing.
#[derive(Default)]
pub struct HealthRegistry {
    servers: RwLock<HashMap<String, ServerHealth>>,
}

impl HealthRegistry {
    pub fn get(&self, server: &str) -> Option<ServerHealth> {
        self.servers.read().ok()?.get(server).cloned()
    }

    pub fn is_down(&self, server: &str) -> bool {
        self.get(server).is_some_and(|health| !health.online)
    }

    pub fn record_up(&self, server: &str, latency: Duration, players: Option<(u32, u32)>) {
        if let Ok(mut servers) = self.servers.write() {
            servers.insert(
                server.to_string(),
                ServerHealth {
                    online: true,
                    latency: Some(latency),
                    players,
                    checked_at: Instant::now(),
                    failures: 0,
                },
            );
        }
    }

    /// Counts a failed check. The server only goes down after `down_after` failures in a row,
    /// so one dropped ping doesn't send players elsewhere. Returns true when it just went down.
    pub fn record_failure(&self, server: &str, down_after: u32) -> bool {
        let Ok(mut servers) = self.servers.write() else {
            return false;
        };
        let health = servers
            .entry(server.to_string())
            .or_insert_with(|| ServerHealth {
                online: true,
                latency: None,
                players: None,
                checked_at: Instant::now(),
                failures: 0,
            });
        health.failures = health.failures.saturating_add(1);
        health.checked_at = Instant::now();
        let was_online = health.online;
        if health.failures >= down_after.max(1) {
            health.online = false;
            health.latency = None;
            health.players = None;
        }
        was_online && !health.online
    }

    /// Drops servers that are no longer in the config
    pub fn retain(&self, keep: impl Fn(&str) -> bool) {
        if let Ok(mut servers) = self.servers.write() {
            servers.retain(|name, _| keep(name));
        }
    }

    pub fn snapshot(&self) -> HashMap<String, ServerHealth> {
        self.servers
            .read()
            .map(|servers| servers.clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_goes_down_after_repeated_failures() {
        let health = HealthRegistry::default();
        assert!(!health.is_down("lobby"));

        assert!(!health.record_failure("lobby", 2));
        assert!(!health.is_down("lobby"));
        assert!(health.record_failure("lobby", 2));
        assert!(health.is_down("lobby"));
        assert!(!health.record_failure("lobby", 2));

        health.record_up("lobby", Duration::from_millis(5), Some((3, 20)));
        assert!(!health.is_down("lobby"));
        assert_eq!(health.get("lobby").unwrap().players, Some((3, 20)));
    }
}
