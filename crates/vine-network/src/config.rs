use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

fn default_tcp_keepalive() -> u64 {
    60
}

fn default_handshake_timeout() -> u64 {
    10
}

fn default_status_timeout() -> u64 {
    5
}

fn default_login_timeout() -> u64 {
    15
}

fn default_backend_connect_timeout() -> u64 {
    5
}

fn default_backend_login_timeout() -> u64 {
    15
}

fn default_backend_retries() -> u32 {
    2
}

fn default_listen_backlog() -> u32 {
    1024
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Enable TCP_NODELAY (disable Nagle's algorithm) on client and backend sockets.
    /// Crucial for eliminating latency spikes in Minecraft packet transmission.
    #[serde(default = "default_true")]
    pub tcp_nodelay: bool,

    /// TCP keepalive interval in seconds on client and backend sockets (0 to disable).
    #[serde(default = "default_tcp_keepalive")]
    pub tcp_keepalive_secs: u64,

    /// Timeout in seconds to wait for initial client Handshake packet
    #[serde(default = "default_handshake_timeout")]
    pub client_handshake_timeout_secs: u64,

    /// Timeout in seconds to wait for client Status/Ping packets
    #[serde(default = "default_status_timeout")]
    pub client_status_timeout_secs: u64,

    /// Timeout in seconds to wait for client LoginStart and Encryption packets
    #[serde(default = "default_login_timeout")]
    pub client_login_timeout_secs: u64,

    /// Timeout in seconds when connecting to a backend server
    #[serde(default = "default_backend_connect_timeout")]
    pub backend_connect_timeout_secs: u64,

    /// Timeout in seconds for backend socket reads during the login handshake phase
    #[serde(default = "default_backend_login_timeout")]
    pub backend_login_timeout_secs: u64,

    /// Number of connection retry attempts when connecting to a backend server before failing
    #[serde(default = "default_backend_retries")]
    pub backend_connect_retry_attempts: u32,

    /// Optional TCP receive buffer size (SO_RCVBUF) in bytes
    #[serde(default)]
    pub recv_buffer_size: Option<usize>,

    /// Optional TCP send buffer size (SO_SNDBUF) in bytes
    #[serde(default)]
    pub send_buffer_size: Option<usize>,

    /// TCP listen backlog queue size
    #[serde(default = "default_listen_backlog")]
    pub listen_backlog: u32,

    /// Support HAProxy PROXY Protocol (v1 and v2) from upstream reverse proxies (e.g. Cloudflare Spectrum, HAProxy)
    #[serde(default)]
    pub proxy_protocol: bool,

    /// IPs or CIDR ranges allowed to send PROXY headers; required when proxy_protocol is on
    #[serde(default)]
    pub proxy_protocol_trusted_proxies: Vec<String>,

    /// Log incoming server list status pings in the console
    #[serde(default)]
    pub log_pings: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            tcp_nodelay: default_true(),
            tcp_keepalive_secs: default_tcp_keepalive(),
            client_handshake_timeout_secs: default_handshake_timeout(),
            client_status_timeout_secs: default_status_timeout(),
            client_login_timeout_secs: default_login_timeout(),
            backend_connect_timeout_secs: default_backend_connect_timeout(),
            backend_login_timeout_secs: default_backend_login_timeout(),
            backend_connect_retry_attempts: default_backend_retries(),
            recv_buffer_size: None,
            send_buffer_size: None,
            listen_backlog: default_listen_backlog(),
            proxy_protocol: false,
            proxy_protocol_trusted_proxies: Vec::new(),
            log_pings: false,
        }
    }
}
