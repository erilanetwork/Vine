use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use crate::backend::Router;
use crate::backend::forwarding::ForwardingHelper;
use crate::client::ClientHandler;
use crate::console::Console;
use crate::network;
use crate::status::{StatusHandler, StatusIconManager};
use vine_auth::{Authenticator, KeyStore};
use vine_config::{Config, ForwardingMode};
use vine_i18n::translate;
use vine_plugin::command::{CommandDispatcher, CommandSender, ProxyCommandSource};
use vine_plugin::session::{PlayerAction, SessionManager, send_reliably};
use vine_security::ConnectionRateLimiter;

pub struct ProxyServer {
    config: Arc<Config>,
    keystore: Arc<KeyStore>,
    authenticator: Arc<Authenticator>,
    rate_limiter: Arc<ConnectionRateLimiter>,
    status_handler: Arc<StatusHandler>,
    router: Arc<Router>,
    online_players: Arc<AtomicUsize>,
    session_manager: Arc<SessionManager>,
    command_dispatcher: Arc<CommandDispatcher>,
    plugin_manager: Arc<vine_plugin::plugin::PluginManager>,
}

impl ProxyServer {
    pub fn new(config: Config) -> Result<Self, Box<dyn std::error::Error>> {
        let config = Arc::new(config);
        let keystore = Arc::new(KeyStore::new()?);
        let authenticator = Arc::new(Authenticator::new());

        let rate_limiter = Arc::new(ConnectionRateLimiter::new(
            config.security.rate_limit_enabled,
            config.security.max_connections_per_sec,
            config.security.connection_burst,
        ));

        let online_players = Arc::new(AtomicUsize::new(0));
        let session_manager = Arc::new(SessionManager::new());
        let command_dispatcher = Arc::new(CommandDispatcher::new());

        let server_icons: HashMap<String, String> = config
            .servers
            .iter()
            .filter_map(|(name, s)| s.icon_path.as_ref().map(|p| (name.clone(), p.clone())))
            .collect();

        let icon_manager = Arc::new(StatusIconManager::new(
            config.motd.icon_path.clone(),
            server_icons,
            config.routing.forced_hosts.clone(),
            config.routing.default_server.clone(),
        ));

        let status_handler = Arc::new(StatusHandler::new(
            config.motd.clone(),
            icon_manager,
            config.servers.clone(),
            config.routing.forced_hosts.clone(),
        ));

        let router = Arc::new(Router::new(config.routing.clone(), config.servers.clone()));

        let plugins_dir = std::path::PathBuf::from(&config.plugins.plugin_dir);
        let plugin_manager =
            vine_plugin::plugin::PluginManager::new(plugins_dir, (&config.plugins).into())?;
        plugin_manager.init_proxy_context(
            config.clone(),
            session_manager.clone(),
            command_dispatcher.clone(),
        );

        Ok(Self {
            config,
            keystore,
            authenticator,
            rate_limiter,
            status_handler,
            router,
            online_players,
            session_manager,
            command_dispatcher,
            plugin_manager,
        })
    }

    /// Run the proxy listener loop
    pub async fn run(
        &self,
        shutdown_tx: broadcast::Sender<()>,
        console: Console,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut shutdown_rx = shutdown_tx.subscribe();
        let bind_addr = &self.config.server.bind_address;
        let listener = network::create_tcp_listener(bind_addr, &self.config.network)?;

        for (name, server) in &self.config.servers {
            let forwarding = match &server.forwarding {
                ForwardingMode::None => "no".to_string(),
                mode => format!("{mode:?}").to_ascii_lowercase(),
            };
            info!(
                "Server '{}' at {} ({} forwarding)",
                name, server.address, forwarding
            );
            if server.forwarding == ForwardingMode::Vine
                && let Some(key) = server
                    .secret
                    .as_deref()
                    .and_then(|secret| ForwardingHelper::get_vine_public_key_hex(secret).ok())
            {
                info!(
                    "Vine public key for '{name}': {key} (networking.proxy.vine.public_key on Pumpkin)"
                );
            }
        }
        let net = &self.config.network;
        debug!(
            "Network: nodelay={} keepalive={}s proxy_protocol={} timeouts handshake={}s login={}s backend={}s, compression level {} above {} bytes",
            net.tcp_nodelay,
            net.tcp_keepalive_secs,
            net.proxy_protocol,
            net.client_handshake_timeout_secs,
            net.client_login_timeout_secs,
            net.backend_connect_timeout_secs,
            self.config.server.compression_level,
            self.config.server.compression_threshold
        );

        let trusted_proxies: Arc<Vec<network::IpRange>> = Arc::new(
            self.config
                .network
                .proxy_protocol_trusted_proxies
                .iter()
                .filter_map(|entry| network::IpRange::parse(entry))
                .collect(),
        );

        console.spawn(
            self.command_dispatcher.clone(),
            ProxyCommandSource {
                sender: CommandSender::Console,
                config: self.config.clone(),
                session_manager: self.session_manager.clone(),
            },
            shutdown_tx,
        );

        if self.config.plugins.enabled {
            debug!(
                "Loading plugins from '{}'...",
                self.config.plugins.plugin_dir
            );
            if let Err(e) = self.plugin_manager.load_plugins().await {
                error!("Failed to load plugins: {}", e);
            }
        }

        info!(
            "Listening on {} (online mode {}, Minecraft {}, default server '{}')",
            bind_addr,
            if self.config.server.online_mode {
                "on"
            } else {
                "off"
            },
            vine_protocol::ProtocolVersion::supported_range(),
            self.config.routing.default_server
        );

        loop {
            tokio::select! {
                accept_res = listener.accept() => {
                    match accept_res {
                        Ok((stream, peer_addr)) => {
                            let config = self.config.clone();
                            let keystore = self.keystore.clone();
                            let authenticator = self.authenticator.clone();
                            let status_handler = self.status_handler.clone();
                            let router = self.router.clone();
                            let online_players = self.online_players.clone();
                            let rate_limiter = self.rate_limiter.clone();
                            let session_manager = self.session_manager.clone();
                            let command_dispatcher = self.command_dispatcher.clone();
                            let plugin_manager = self.plugin_manager.clone();
                            let trusted_proxies = trusted_proxies.clone();

                            tokio::spawn(async move {
                                if let Err(e) = network::configure_client_socket(&stream, &config.network) {
                                    debug!("[{}] Failed to configure client socket: {}", peer_addr, e);
                                }

                                let mut stream = stream;
                                let mut effective_addr = peer_addr;
                                let mut leftover_bytes = Vec::new();

                                if config.network.proxy_protocol {
                                    if !trusted_proxies.iter().any(|range| range.contains(&peer_addr.ip()))
                                    {
                                        debug!("[{}] Dropped connection: peer is not a trusted PROXY protocol source", peer_addr);
                                        return;
                                    }

                                    let header_timeout = Duration::from_secs(config.network.client_handshake_timeout_secs);
                                    match tokio::time::timeout(header_timeout, network::parse_proxy_protocol(&mut stream, peer_addr)).await {
                                        Ok(Ok(res)) => {
                                             effective_addr = res.client_addr;
                                             leftover_bytes = res.leftover_bytes;
                                             debug!("[{}] PROXY protocol resolved client IP: {}", peer_addr, effective_addr);
                                        }
                                        Ok(Err(err)) => {
                                            warn!("[{}] Dropped connection: PROXY protocol error: {}", peer_addr, err);
                                            return;
                                        }
                                        Err(_) => {
                                            debug!("[{}] Dropped connection: PROXY protocol header timed out", peer_addr);
                                            return;
                                        }
                                    }
                                }

                                if !rate_limiter.check_connection(&effective_addr.ip()) {
                                    warn!("[{}] Dropped connection: Rate limit exceeded", effective_addr);
                                    return;
                                }

                                let handler = ClientHandler::new(
                                    effective_addr,
                                    config,
                                    keystore,
                                    authenticator,
                                    status_handler,
                                    router,
                                    online_players,
                                    session_manager,
                                    command_dispatcher,
                                    plugin_manager,
                                );

                                if let Err(err) = handler.handle(stream, leftover_bytes).await {
                                    debug!("[{}] Connection closed: {}", effective_addr, err);
                                }
                            });
                        }
                        Err(e) => {
                            error!("TCP listener accept error: {}", e);
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    }
                }
                _ = shutdown_rx.recv() => break,
            }
        }

        self.disconnect_players().await;

        if self.config.plugins.enabled {
            self.plugin_manager.unload_plugins().await;
        }

        Ok(())
    }

    async fn disconnect_players(&self) {
        for session in self.session_manager.list_sessions() {
            let reason = translate(session.effective_locale(), "vine.shutdown", &[]);
            send_reliably(&session.action_tx, PlayerAction::Disconnect(reason));
        }
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(self.config.server.shutdown_timeout_secs.max(1));
        while self.session_manager.count() > 0 && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}
