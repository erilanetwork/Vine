use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use pumpkin_command::argument_builder::{ArgumentBuilder, command};
use pumpkin_command::context::command_context::CommandContext;
use pumpkin_command::dispatcher::CommandDispatcher as RawCommandDispatcher;
use pumpkin_command::node::{CommandExecutor, CommandExecutorResult};
use pumpkin_command::{CommandSource, Requirement};
use vine_protocol::ProtocolVersion;
use vine_protocol::text::parse_message;

use super::{ProxyCommandSource, command_error};

const PERMISSION: &str = "vine.command.status";

/// Shows what the proxy is running, how long it has been up and where players are
pub struct StatusCommandExecutor {
    // hmm, the dispatcher is built when the proxy starts, so this doubles as the start time
    started: Instant,
}

impl CommandExecutor<ProxyCommandSource> for StatusCommandExecutor {
    fn execute(&self, context: &CommandContext<ProxyCommandSource>) -> CommandExecutorResult {
        let source = &context.source;
        if !source.has_permission(PERMISSION) {
            return Err(command_error(
                source.tr("vine.command.no_permission", &[&PERMISSION]),
            ));
        }

        let sessions = source.session_manager.list_sessions();
        let config = source.config();
        let mut per_server: BTreeMap<&str, usize> = config
            .servers
            .keys()
            .map(|name| (name.as_str(), 0))
            .collect();
        for session in &sessions {
            if let Some(count) = per_server.get_mut(session.current_server.as_str()) {
                *count += 1;
            }
        }
        let servers: Vec<String> = per_server
            .into_iter()
            .map(|(name, count)| match source.services.health.get(name) {
                Some(health) if !health.online => {
                    source.tr("vine.command.status.server_offline", &[&name])
                }
                Some(health) if health.latency.is_some() => source.tr(
                    "vine.command.status.server_ping",
                    &[
                        &name,
                        &count,
                        &health.latency.unwrap_or_default().as_millis(),
                    ],
                ),
                _ => source.tr("vine.command.status.server", &[&name, &count]),
            })
            .collect();

        let online_mode = if config.server.online_mode {
            "on"
        } else {
            "off"
        };
        let lines = [
            source.tr(
                "vine.command.status.version",
                &[
                    &env!("CARGO_PKG_VERSION"),
                    &ProtocolVersion::supported_range(),
                ],
            ),
            source.tr(
                "vine.command.status.uptime",
                &[&format_uptime(self.started.elapsed())],
            ),
            source.tr(
                "vine.command.status.players",
                &[&sessions.len(), &config.motd.max_players],
            ),
            source.tr("vine.command.status.online_mode", &[&online_mode]),
            source.tr(
                "vine.command.status.servers",
                &[&servers.len(), &servers.join(", ")],
            ),
        ];
        source.send_message(parse_message(&lines.join("\n")));
        Ok(sessions.len() as i32)
    }
}

/// Formats a duration as `1d 2h 3m 4s`, skipping the leading units that are zero
fn format_uptime(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    let parts = [
        (secs / 86_400, "d"),
        (secs / 3_600 % 24, "h"),
        (secs / 60 % 60, "m"),
        (secs % 60, "s"),
    ];
    let first = parts
        .iter()
        .position(|(value, _)| *value > 0)
        .unwrap_or(parts.len() - 1);
    parts[first..]
        .iter()
        .map(|(value, unit)| format!("{value}{unit}"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn register(dispatcher: &mut RawCommandDispatcher<ProxyCommandSource>) {
    dispatcher.register(
        command("status", "Shows proxy version, uptime and player counts")
            .requires(Requirement::from(PERMISSION))
            .executes(StatusCommandExecutor {
                started: Instant::now(),
            }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::ProxyServices;
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use uuid::Uuid;

    use crate::command::{CommandDispatcher, CommandSender};
    use crate::session::{PlayerAction, PlayerSession, SessionManager};
    use vine_config::permissions::UserPermissionConfig;
    use vine_config::{BackendConfig, Config, ForwardingMode};

    fn backend(address: &str) -> BackendConfig {
        BackendConfig {
            address: address.to_string(),
            forwarding: ForwardingMode::None,
            secret: None,
            motd: None,
            max_players: None,
            icon_path: None,
        }
    }

    #[test]
    fn test_format_uptime() {
        assert_eq!(format_uptime(Duration::from_secs(0)), "0s");
        assert_eq!(format_uptime(Duration::from_secs(59)), "59s");
        assert_eq!(format_uptime(Duration::from_secs(3_605)), "1h 0m 5s");
        assert_eq!(format_uptime(Duration::from_secs(90_061)), "1d 1h 1m 1s");
    }

    #[tokio::test]
    async fn test_status_reports_players_per_server() {
        let mut config = Config::default();
        config
            .servers
            .insert("lobby".to_string(), backend("127.0.0.1:25565"));
        config
            .servers
            .insert("survival".to_string(), backend("127.0.0.1:25566"));
        config.permissions.users.insert(
            "Admin".to_string(),
            UserPermissionConfig::Groups(vec!["admin".to_string()]),
        );

        let session_manager = Arc::new(SessionManager::new());
        let (tx, _) = mpsc::channel(16);
        let _guard = session_manager
            .try_register(PlayerSession {
                username: "Steve".to_string(),
                uuid: Uuid::new_v4(),
                current_server: "lobby".to_string(),
                action_tx: tx,
                client_ip: "127.0.0.1".to_string(),
                protocol_version: 765,
                locale: None,
                locale_override: None,
                plugin_waiting: Default::default(),
            })
            .unwrap();

        let (action_tx, mut action_rx) = mpsc::channel(16);
        let source = ProxyCommandSource {
            sender: CommandSender::Player {
                username: "Admin".to_string(),
                action_tx,
            },
            services: ProxyServices::from_config(config),
            session_manager,
        };

        let dispatcher = CommandDispatcher::new();
        assert_eq!(dispatcher.execute_input("status", &source).unwrap(), 1);

        match action_rx.recv().await.unwrap() {
            PlayerAction::Message(text) => {
                let s = text.to_pretty_console();
                assert!(s.contains(env!("CARGO_PKG_VERSION")));
                assert!(s.contains("lobby (1)"));
                assert!(s.contains("survival (0)"));
            }
            _ => panic!("Expected Message action"),
        }
    }

    #[test]
    fn test_status_needs_permission() {
        let (action_tx, _) = mpsc::channel(16);
        let source = ProxyCommandSource {
            sender: CommandSender::Player {
                username: "Steve".to_string(),
                action_tx,
            },
            services: ProxyServices::from_config(Config::default()),
            session_manager: Arc::new(SessionManager::new()),
        };
        assert!(
            CommandDispatcher::new()
                .execute_input("status", &source)
                .is_err()
        );
    }
}
