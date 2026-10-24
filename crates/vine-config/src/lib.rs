pub mod permissions;
mod shared;

use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io::Write, path::Path};

pub use vine_network::NetworkConfig;

// Covers the handshake, status and every login packet a client sends, profile keys included
pub const LOGIN_PACKET_LIMIT: usize = 8 * 1024;

fn default_true() -> bool {
    true
}

fn default_compression_level() -> i32 {
    6
}

fn default_shutdown_timeout() -> u64 {
    10
}

const BIND_ADDRESS_ENV: &str = "VINE_BIND_ADDRESS";

fn default_vine_key_file() -> String {
    "vine-forwarding.key".to_string()
}

fn default_locale() -> String {
    "en_us".to_string()
}

fn generate_forwarding_secret() -> String {
    hex::encode(rand::random::<[u8; 32]>())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    #[serde(default)]
    pub network: NetworkConfig,
    pub motd: MotdConfig,
    pub security: SecurityConfig,
    pub routing: RoutingConfig,
    pub servers: HashMap<String, BackendConfig>,
    #[serde(default)]
    pub permissions: PermissionsConfig,
    #[serde(default)]
    pub plugins: PluginConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_plugins_dir")]
    pub plugin_dir: String,
    /// Linear memory a single plugin may allocate
    #[serde(default = "default_plugin_memory_mb")]
    pub max_memory_mb: u64,
    /// A plugin call running longer than this disables the plugin
    #[serde(default = "default_plugin_call_timeout_ms")]
    pub call_timeout_ms: u64,
}

fn default_plugin_memory_mb() -> u64 {
    256
}

fn default_plugin_call_timeout_ms() -> u64 {
    5000
}

fn default_plugins_dir() -> String {
    "./plugins".to_string()
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            plugin_dir: default_plugins_dir(),
            max_memory_mb: default_plugin_memory_mb(),
            call_timeout_ms: default_plugin_call_timeout_ms(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Address and port for the proxy to listen on
    pub bind_address: String,
    /// Whether to verify player accounts with Mojang session servers
    pub online_mode: bool,
    /// Send player IP to Mojang during session authentication to prevent proxy hijacking
    pub prevent_proxy_connections: bool,
    /// Packets of at least this many bytes are compressed (-1 = off, 0 = every packet, e.g. 256)
    pub compression_threshold: i32,
    /// zlib compression level (-1 = zlib's default, 0 = none, 1 = fastest, 9 = smallest)
    #[serde(default = "default_compression_level")]
    pub compression_level: i32,
    /// Graceful shutdown timeout in seconds to wait for active connections to close
    #[serde(default = "default_shutdown_timeout")]
    pub shutdown_timeout_secs: u64,
    /// Signing key for vine backends without their own `secret`
    #[serde(default = "default_vine_key_file")]
    pub vine_key_file: String,
    /// Require 1.19-1.19.2 clients to present a Mojang-signed chat key in online mode
    #[serde(default = "default_true")]
    pub force_key_authentication: bool,
    /// Language used when a player's own language has no translation, e.g. `en_us`
    #[serde(default = "default_locale")]
    pub default_locale: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MotdConfig {
    /// Server list description; use \n for a second line
    pub text: String,
    /// Maximum players shown in the server list ping
    pub max_players: u32,
    /// Custom version display string (e.g. "Vine 1.7.x - 26.x")
    pub version_name: String,
    /// Protocol version reported to clients (776 is 26.2)
    pub protocol_version: i32,
    /// Optional sample player list shown on hover in server list
    pub sample_players: Vec<String>,
    /// Optional path to a 64x64 PNG file for the server list icon (favicons)
    #[serde(default)]
    pub icon_path: Option<String>,
    /// Whether to show the player sample list on hover in the multiplayer menu
    #[serde(default = "default_true")]
    pub show_sample_players: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// Enable connection rate limiting per IP
    pub rate_limit_enabled: bool,
    /// Maximum new TCP connections per second per IP
    pub max_connections_per_sec: u32,
    /// Burst allowance for new connections
    pub connection_burst: u32,
    /// Maximum packets per second from a single client before being kicked for flooding
    pub max_packets_per_sec: u32,
    /// Maximum packet size a logged-in client may send, in bytes, both on the wire and once
    /// inflated (default 2MB; the wire size is capped at the protocol maximum of 2097151)
    pub max_packet_size: usize,
    /// Enforce strict Minecraft username validation (^[a-zA-Z0-9_]{2,16}$)
    pub validate_usernames: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutingConfig {
    /// Default backend server name to connect to
    pub default_server: String,
    /// Fallback server if default server is offline
    pub fallback_server: Option<String>,
    /// Virtual host routing: maps domain/hostname to backend server name
    pub forced_hosts: HashMap<String, String>,
    /// Automatically try fallback server if connection to target server fails
    #[serde(default = "default_true")]
    pub try_fallback_on_connect_failure: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ForwardingMode {
    /// Plain connection without forwarding info (for Vanilla servers in offline-mode)
    None,
    /// Vine modern forwarding with Ed25519 asymmetric signatures, timestamp, and challenge nonce (for Pumpkin)
    Vine,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackendConfig {
    /// Address of backend server (e.g. "127.0.0.1:25566")
    pub address: String,
    /// Forwarding mode to use when connecting to this backend
    pub forwarding: ForwardingMode,
    /// Signing key for Vine forwarding; falls back to `server.vine_key_file` when unset
    pub secret: Option<String>,
    /// Optional custom 64x64 PNG status icon path for this backend / virtual host
    #[serde(default)]
    pub icon_path: Option<String>,
    /// Optional server list description for this backend / virtual host
    #[serde(default)]
    pub motd: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        let mut servers = HashMap::new();
        servers.insert(
            "lobby".to_string(),
            BackendConfig {
                address: "127.0.0.1:25566".to_string(),
                forwarding: ForwardingMode::Vine,
                secret: None,
                icon_path: None,
                motd: None,
            },
        );
        servers.insert(
            "survival".to_string(),
            BackendConfig {
                address: "127.0.0.1:25567".to_string(),
                forwarding: ForwardingMode::Vine,
                secret: None,
                icon_path: None,
                motd: None,
            },
        );
        servers.insert(
            "vanilla".to_string(),
            BackendConfig {
                address: "127.0.0.1:25568".to_string(),
                forwarding: ForwardingMode::None,
                secret: None,
                icon_path: None,
                motd: None,
            },
        );

        let mut forced_hosts = HashMap::new();
        forced_hosts.insert("survival.example.com".to_string(), "survival".to_string());
        forced_hosts.insert("vanilla.example.com".to_string(), "vanilla".to_string());

        Self {
            server: ServerConfig {
                bind_address: "0.0.0.0:25565".to_string(),
                online_mode: true,
                prevent_proxy_connections: false,
                compression_threshold: 256,
                compression_level: 6,
                shutdown_timeout_secs: 10,
                vine_key_file: default_vine_key_file(),
                force_key_authentication: true,
                default_locale: default_locale(),
            },
            network: NetworkConfig::default(),
            motd: MotdConfig {
                text: "A Vine Server".to_string(),
                max_players: 100,
                version_name: "Vine 1.7.x - 26.x".to_string(),
                protocol_version: 776,
                sample_players: Vec::new(),
                icon_path: None,
                show_sample_players: true,
            },
            security: SecurityConfig {
                rate_limit_enabled: true,
                max_connections_per_sec: 5,
                connection_burst: 10,
                max_packets_per_sec: 500,
                max_packet_size: 2 * 1024 * 1024,
                validate_usernames: true,
            },
            routing: RoutingConfig {
                default_server: "lobby".to_string(),
                fallback_server: Some("survival".to_string()),
                forced_hosts,
                try_fallback_on_connect_failure: true,
            },
            servers,
            permissions: PermissionsConfig::default(),
            plugins: PluginConfig::default(),
        }
    }
}

pub use permissions::PermissionsConfig;
pub use shared::SharedConfig;

pub fn normalize_host(host: &str) -> String {
    host.trim().trim_end_matches('.').to_ascii_lowercase()
}

fn needs_secret(server: &BackendConfig) -> bool {
    server.forwarding == ForwardingMode::Vine
        && server.secret.as_deref().is_none_or(|s| s.trim().is_empty())
}

fn load_or_create_secret_file(
    path: &Path,
    usage: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    match std::fs::read_to_string(path) {
        Ok(content) if content.trim().is_empty() => {
            Err(format!("forwarding secret file '{}' is empty", path.display()).into())
        }
        Ok(content) => Ok(content.trim().to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let secret = generate_forwarding_secret();
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            options.open(path)?.write_all(secret.as_bytes())?;
            tracing::info!("Generated '{}'; {usage}", path.display());
            Ok(secret)
        }
        Err(e) => Err(format!(
            "failed to read forwarding secret file '{}': {e}",
            path.display()
        )
        .into()),
    }
}

impl Config {
    pub fn load_or_create<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let path = path.as_ref();
        if !path.exists() {
            std::fs::write(path, toml::to_string_pretty(&Config::default())?)?;
        }
        Self::load(path)
    }

    /// Reads and validates an existing config file. Unlike `load_or_create` this never writes a
    /// default one, so a reload can't replace a file that went missing.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let path = path.as_ref();
        let mut config: Config = toml::from_str(&std::fs::read_to_string(path)?)?;
        config.routing.forced_hosts = std::mem::take(&mut config.routing.forced_hosts)
            .into_iter()
            .map(|(host, server)| (normalize_host(&host), server))
            .collect();
        // Hosting panels assign the port at start, so it wins over the file
        if let Ok(bind) = std::env::var(BIND_ADDRESS_ENV)
            && !bind.trim().is_empty()
        {
            config.server.bind_address = bind.trim().to_string();
        }
        config.resolve_forwarding_secret(path.parent().unwrap_or(Path::new("")))?;
        config.validate()?;
        Ok(config)
    }

    fn resolve_forwarding_secret(
        &mut self,
        config_dir: &Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if !self.servers.values().any(needs_secret) {
            return Ok(());
        }
        let key = load_or_create_secret_file(
            &config_dir.join(&self.server.vine_key_file),
            "give each Pumpkin backend the Vine public key logged below",
        )?;
        for server in self.servers.values_mut().filter(|s| needs_secret(s)) {
            server.secret = Some(key.clone());
        }
        Ok(())
    }

    /// Names the settings that differ from `other` but only take effect on a restart
    pub fn restart_required(&self, other: &Config) -> Vec<&'static str> {
        // hmm, the sections don't implement PartialEq, comparing their TOML is good enough here
        fn differs<T: Serialize>(a: &T, b: &T) -> bool {
            toml::to_string(a).ok() != toml::to_string(b).ok()
        }
        let mut changed = Vec::new();
        if self.server.bind_address != other.server.bind_address {
            changed.push("server.bind_address");
        }
        if differs(&self.network, &other.network) {
            changed.push("network");
        }
        let (a, b) = (&self.security, &other.security);
        if (
            a.rate_limit_enabled,
            a.max_connections_per_sec,
            a.connection_burst,
        ) != (
            b.rate_limit_enabled,
            b.max_connections_per_sec,
            b.connection_burst,
        ) {
            changed.push("security rate limits");
        }
        if differs(&self.plugins, &other.plugins) {
            changed.push("plugins");
        }
        changed
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.server.compression_threshold < -1 {
            return Err("server.compression_threshold must be -1 (off) or more".to_string());
        }
        if !(-1..=9).contains(&self.server.compression_level) {
            return Err("server.compression_level must be between -1 and 9".to_string());
        }
        let security = &self.security;
        if security.max_packets_per_sec == 0 {
            return Err("security.max_packets_per_sec must be at least 1".to_string());
        }
        if security.max_packet_size < LOGIN_PACKET_LIMIT {
            return Err(format!(
                "security.max_packet_size must be at least {}",
                LOGIN_PACKET_LIMIT
            ));
        }
        if security.rate_limit_enabled
            && (security.max_connections_per_sec == 0 || security.connection_burst == 0)
        {
            return Err(
                "security.max_connections_per_sec and security.connection_burst must be at least 1 \
                 while rate limiting is on"
                    .to_string(),
            );
        }
        if self.network.proxy_protocol && self.network.proxy_protocol_trusted_proxies.is_empty() {
            return Err(
                "network.proxy_protocol is on, but network.proxy_protocol_trusted_proxies is empty; \
                 list the proxies allowed to send PROXY headers (\"0.0.0.0/0\" and \"::/0\" trust every address)"
                    .to_string(),
            );
        }
        for entry in &self.network.proxy_protocol_trusted_proxies {
            if vine_network::IpRange::parse(entry).is_none() {
                return Err(format!(
                    "invalid entry '{entry}' in network.proxy_protocol_trusted_proxies"
                ));
            }
        }
        let routes = std::iter::once(("routing.default_server", &self.routing.default_server))
            .chain(
                self.routing
                    .fallback_server
                    .iter()
                    .map(|name| ("routing.fallback_server", name)),
            )
            .chain(
                self.routing
                    .forced_hosts
                    .values()
                    .map(|name| ("routing.forced_hosts", name)),
            );
        for (field, name) in routes {
            if !self.servers.contains_key(name) {
                return Err(format!("{field} points to unknown server '{name}'"));
            }
        }
        for (name, server) in &self.servers {
            if needs_secret(server) {
                return Err(format!(
                    "server '{name}' uses vine forwarding but has no secret configured"
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vine_key_file_is_generated_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = Config::default();
        a.resolve_forwarding_secret(dir.path()).unwrap();
        assert!(a.validate().is_ok());

        let path = dir.path().join("vine-forwarding.key");
        let key = std::fs::read_to_string(&path).unwrap();
        assert_eq!(key.len(), 64);
        assert_eq!(a.servers["lobby"].secret.as_deref(), Some(key.as_str()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        let mut b = Config::default();
        b.resolve_forwarding_secret(dir.path()).unwrap();
        assert_eq!(b.servers["lobby"].secret, a.servers["lobby"].secret);
    }

    #[test]
    fn test_validate_rejects_missing_secrets() {
        let mut config = Config::default();
        config.servers.get_mut("lobby").unwrap().secret = Some("  ".to_string());
        assert!(config.validate().is_err());

        config.servers.get_mut("lobby").unwrap().secret = None;
        assert!(config.validate().is_err());

        config.servers.get_mut("lobby").unwrap().secret = Some("k".to_string());
        config.servers.get_mut("survival").unwrap().forwarding = ForwardingMode::None;
        assert!(config.validate().is_ok());
    }
}
