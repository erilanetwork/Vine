use std::net::IpAddr;
use std::time::Duration;

use pumpkin_command::Requirement;
use pumpkin_command::argument_builder::{ArgumentBuilder, argument, command, literal};
use pumpkin_command::argument_types::core::string::StringArgumentType;
use pumpkin_command::context::command_context::CommandContext;
use pumpkin_command::dispatcher::CommandDispatcher as RawCommandDispatcher;
use pumpkin_command::node::{CommandExecutor, CommandExecutorResult};
use pumpkin_command::suggestion::provider::{SuggestionProvider, SuggestionProviderResult};
use pumpkin_command::suggestion::suggestions::SuggestionsBuilder;
use tracing::info;
use vine_i18n::translate;
use vine_moderation::{Ban, format_duration, now, parse_duration};

use super::{CommandSender, ProxyCommandSource, command_error};
use crate::session::{PlayerAction, PlayerSession, send_reliably};

const BAN_PERMISSION: &str = "vine.command.ban";
const BANIP_PERMISSION: &str = "vine.command.banip";
const KICK_PERMISSION: &str = "vine.command.kick";
const WHITELIST_PERMISSION: &str = "vine.command.whitelist";
const MAINTENANCE_PERMISSION: &str = "vine.command.maintenance";
pub const MAINTENANCE_BYPASS: &str = "vine.maintenance.bypass";
pub const WHITELIST_BYPASS: &str = "vine.whitelist.bypass";

/// The kick message for a banned player, in their language
pub fn ban_message(locale: Option<&str>, ban: &Ban) -> String {
    let reason = ban
        .reason
        .clone()
        .unwrap_or_else(|| translate(locale, "vine.ban.no_reason", &[]));
    match ban.expires {
        Some(expires) => {
            let left = format_duration(Duration::from_secs(expires.saturating_sub(now())));
            translate(locale, "vine.ban.temporary", &[&left, &reason])
        }
        None => translate(locale, "vine.ban.permanent", &[&reason]),
    }
}

fn sender_name(source: &ProxyCommandSource) -> String {
    match &source.sender {
        CommandSender::Console => "CONSOLE".to_string(),
        CommandSender::Player { username, .. } => username.clone(),
    }
}

fn kick(session: &PlayerSession, message: String) {
    send_reliably(&session.action_tx, PlayerAction::Disconnect(message));
}

/// Splits `[duration] [reason...]`. The first word only counts as a duration when it parses as
/// one, so `/ban Steve griefing` is a permanent ban with a reason.
fn duration_and_reason(rest: Option<&str>) -> (Option<Duration>, Option<String>) {
    let rest = rest.map(str::trim).unwrap_or_default();
    let (first, tail) = rest.split_once(' ').unwrap_or((rest, ""));
    match parse_duration(first) {
        Some(duration) => (
            Some(duration),
            Some(tail.trim())
                .filter(|t| !t.is_empty())
                .map(String::from),
        ),
        None => (None, Some(rest).filter(|r| !r.is_empty()).map(String::from)),
    }
}

fn new_ban(source: &ProxyCommandSource, rest: Option<&str>) -> Ban {
    let (duration, reason) = duration_and_reason(rest);
    let created = now();
    Ban {
        reason,
        source: sender_name(source),
        created,
        expires: duration.map(|d| created.saturating_add(d.as_secs())),
    }
}

fn describe(source: &ProxyCommandSource, ban: &Ban) -> String {
    match ban.expires {
        Some(expires) => format_duration(Duration::from_secs(expires.saturating_sub(ban.created))),
        None => source.tr("vine.command.ban.forever", &[]),
    }
}

struct OnlinePlayers;

impl SuggestionProvider<ProxyCommandSource> for OnlinePlayers {
    fn suggest(
        &self,
        context: &CommandContext<ProxyCommandSource>,
        mut builder: SuggestionsBuilder,
    ) -> SuggestionProviderResult {
        let remaining = builder.remaining_lowercase().to_string();
        for session in context.source.session_manager.list_sessions() {
            if session
                .username
                .to_ascii_lowercase()
                .starts_with(&remaining)
            {
                builder = builder.suggest(session.username);
            }
        }
        builder.build()
    }
}

struct BanCommand;

impl CommandExecutor<ProxyCommandSource> for BanCommand {
    fn execute(&self, context: &CommandContext<ProxyCommandSource>) -> CommandExecutorResult {
        let source = &context.source;
        let target = StringArgumentType::get(context, "player")?;
        let ban = new_ban(source, StringArgumentType::get(context, "reason").ok());
        let online = source.session_manager.get_session(target);
        let name = online.as_ref().map_or(target, |s| s.username.as_str());

        source
            .services
            .moderation
            .bans
            .ban_player(name, online.as_ref().map(|s| s.uuid), ban.clone())
            .map_err(command_error)?;
        info!(
            "{} banned {} ({})",
            ban.source,
            name,
            describe(source, &ban)
        );
        if let Some(session) = &online {
            kick(session, ban_message(session.effective_locale(), &ban));
        }
        source.reply("vine.command.ban.done", &[&name, &describe(source, &ban)]);
        Ok(1)
    }
}

struct UnbanCommand;

impl CommandExecutor<ProxyCommandSource> for UnbanCommand {
    fn execute(&self, context: &CommandContext<ProxyCommandSource>) -> CommandExecutorResult {
        let source = &context.source;
        let target = StringArgumentType::get(context, "player")?;
        let removed = source
            .services
            .moderation
            .bans
            .unban_player(target)
            .map_err(command_error)?;
        if !removed {
            return Err(command_error(
                source.tr("vine.command.unban.not_banned", &[&target]),
            ));
        }
        info!("{} unbanned {}", sender_name(source), target);
        source.reply("vine.command.unban.done", &[&target]);
        Ok(1)
    }
}

struct BanIpCommand;

impl CommandExecutor<ProxyCommandSource> for BanIpCommand {
    fn execute(&self, context: &CommandContext<ProxyCommandSource>) -> CommandExecutorResult {
        let source = &context.source;
        let target = StringArgumentType::get(context, "target")?;
        // a player name bans whatever address they're on right now
        let ip: IpAddr = match target.parse() {
            Ok(ip) => ip,
            Err(_) => source
                .session_manager
                .get_session(target)
                .and_then(|s| s.client_ip.parse().ok())
                .ok_or_else(|| {
                    command_error(source.tr("vine.command.banip.unknown", &[&target]))
                })?,
        };
        let ban = new_ban(source, StringArgumentType::get(context, "reason").ok());
        source
            .services
            .moderation
            .bans
            .ban_ip(ip, ban.clone())
            .map_err(command_error)?;
        info!(
            "{} banned IP {} ({})",
            ban.source,
            ip,
            describe(source, &ban)
        );

        let ip_text = ip.to_string();
        let mut kicked = 0;
        for session in source.session_manager.list_sessions() {
            if session.client_ip == ip_text {
                kick(&session, ban_message(session.effective_locale(), &ban));
                kicked += 1;
            }
        }
        source.reply(
            "vine.command.banip.done",
            &[&ip, &describe(source, &ban), &kicked],
        );
        Ok(1)
    }
}

struct UnbanIpCommand;

impl CommandExecutor<ProxyCommandSource> for UnbanIpCommand {
    fn execute(&self, context: &CommandContext<ProxyCommandSource>) -> CommandExecutorResult {
        let source = &context.source;
        let target = StringArgumentType::get(context, "ip")?;
        let ip: IpAddr = target
            .parse()
            .map_err(|_| command_error(source.tr("vine.command.banip.invalid", &[&target])))?;
        let removed = source
            .services
            .moderation
            .bans
            .unban_ip(ip)
            .map_err(command_error)?;
        if !removed {
            return Err(command_error(
                source.tr("vine.command.unban.not_banned", &[&ip]),
            ));
        }
        info!("{} unbanned IP {}", sender_name(source), ip);
        source.reply("vine.command.unban.done", &[&ip]);
        Ok(1)
    }
}

struct BanListCommand;

impl CommandExecutor<ProxyCommandSource> for BanListCommand {
    fn execute(&self, context: &CommandContext<ProxyCommandSource>) -> CommandExecutorResult {
        let source = &context.source;
        let bans = &source.services.moderation.bans;
        let entry = |name: &str, ban: &Ban| {
            let left = match ban.expires {
                Some(expires) => {
                    format_duration(Duration::from_secs(expires.saturating_sub(now())))
                }
                None => source.tr("vine.command.ban.forever", &[]),
            };
            source.tr("vine.command.banlist.entry", &[&name, &left])
        };
        let players: Vec<String> = bans
            .players()
            .iter()
            .map(|b| entry(&b.name, &b.ban))
            .collect();
        let ips: Vec<String> = bans
            .ips()
            .iter()
            .map(|b| entry(&b.ip.to_string(), &b.ban))
            .collect();
        source.reply(
            "vine.command.banlist.players",
            &[&players.len(), &players.join(", ")],
        );
        source.reply("vine.command.banlist.ips", &[&ips.len(), &ips.join(", ")]);
        Ok((players.len() + ips.len()) as i32)
    }
}

struct KickCommand;

impl CommandExecutor<ProxyCommandSource> for KickCommand {
    fn execute(&self, context: &CommandContext<ProxyCommandSource>) -> CommandExecutorResult {
        let source = &context.source;
        let target = StringArgumentType::get(context, "player")?;
        let session = source
            .session_manager
            .get_session(target)
            .ok_or_else(|| command_error(source.tr("vine.command.send.not_online", &[&target])))?;
        let locale = session.effective_locale();
        let reason = StringArgumentType::get(context, "reason")
            .ok()
            .map(str::to_string)
            .unwrap_or_else(|| translate(locale, "vine.command.kick.default", &[]));
        info!(
            "{} kicked {}: {}",
            sender_name(source),
            session.username,
            reason
        );
        kick(&session, reason);
        source.reply("vine.command.kick.done", &[&session.username]);
        Ok(1)
    }
}

/// Flips `[section] enabled` in vine.toml and reloads it, so the switch survives a restart
fn set_enabled(source: &ProxyCommandSource, section: &str, enabled: bool) -> Result<(), String> {
    let config = &source.services.config;
    vine_config::set_config_value(config.path(), section, "enabled", enabled)?;
    config.reload().map(|_| ())
}

struct MaintenanceToggle(bool);

impl CommandExecutor<ProxyCommandSource> for MaintenanceToggle {
    fn execute(&self, context: &CommandContext<ProxyCommandSource>) -> CommandExecutorResult {
        let source = &context.source;
        set_enabled(source, "maintenance", self.0)
            .map_err(|e| command_error(source.tr("vine.command.config.failed", &[&e])))?;
        info!(
            "{} turned maintenance {}",
            sender_name(source),
            on_off(self.0)
        );
        if !self.0 {
            source.reply("vine.command.maintenance.off", &[]);
            return Ok(1);
        }
        let config = source.config();
        let mut kicked = 0;
        for session in source.session_manager.list_sessions() {
            if !config
                .permissions
                .has_permission(&session.username, MAINTENANCE_BYPASS)
            {
                kick(
                    &session,
                    translate(session.effective_locale(), "vine.login.maintenance", &[]),
                );
                kicked += 1;
            }
        }
        source.reply("vine.command.maintenance.on", &[&kicked]);
        Ok(1)
    }
}

fn on_off(enabled: bool) -> &'static str {
    if enabled { "on" } else { "off" }
}

struct WhitelistToggle(bool);

impl CommandExecutor<ProxyCommandSource> for WhitelistToggle {
    fn execute(&self, context: &CommandContext<ProxyCommandSource>) -> CommandExecutorResult {
        let source = &context.source;
        set_enabled(source, "whitelist", self.0)
            .map_err(|e| command_error(source.tr("vine.command.config.failed", &[&e])))?;
        info!(
            "{} turned the whitelist {}",
            sender_name(source),
            on_off(self.0)
        );
        if !self.0 {
            source.reply("vine.command.whitelist.off", &[]);
            return Ok(1);
        }
        let config = source.config();
        let whitelist = &source.services.moderation.whitelist;
        let mut kicked = 0;
        for session in source.session_manager.list_sessions() {
            if !whitelist.contains(&session.username, session.uuid)
                && !config
                    .permissions
                    .has_permission(&session.username, WHITELIST_BYPASS)
            {
                kick(
                    &session,
                    translate(
                        session.effective_locale(),
                        "vine.login.not_whitelisted",
                        &[],
                    ),
                );
                kicked += 1;
            }
        }
        source.reply("vine.command.whitelist.on", &[&kicked]);
        Ok(1)
    }
}

struct WhitelistAdd;

impl CommandExecutor<ProxyCommandSource> for WhitelistAdd {
    fn execute(&self, context: &CommandContext<ProxyCommandSource>) -> CommandExecutorResult {
        let source = &context.source;
        let target = StringArgumentType::get(context, "player")?;
        let online = source.session_manager.get_session(target);
        let name = online.as_ref().map_or(target, |s| s.username.as_str());
        let added = source
            .services
            .moderation
            .whitelist
            .add(name, online.as_ref().map(|s| s.uuid))
            .map_err(command_error)?;
        let key = if added {
            "vine.command.whitelist.added"
        } else {
            "vine.command.whitelist.already"
        };
        source.reply(key, &[&name]);
        Ok(1)
    }
}

struct WhitelistRemove;

impl CommandExecutor<ProxyCommandSource> for WhitelistRemove {
    fn execute(&self, context: &CommandContext<ProxyCommandSource>) -> CommandExecutorResult {
        let source = &context.source;
        let target = StringArgumentType::get(context, "player")?;
        let removed = source
            .services
            .moderation
            .whitelist
            .remove(target)
            .map_err(command_error)?;
        if !removed {
            return Err(command_error(
                source.tr("vine.command.whitelist.missing", &[&target]),
            ));
        }
        source.reply("vine.command.whitelist.removed", &[&target]);
        Ok(1)
    }
}

struct WhitelistList;

impl CommandExecutor<ProxyCommandSource> for WhitelistList {
    fn execute(&self, context: &CommandContext<ProxyCommandSource>) -> CommandExecutorResult {
        let source = &context.source;
        let names = source.services.moderation.whitelist.names();
        let state = on_off(source.config().whitelist.enabled);
        source.reply(
            "vine.command.whitelist.list",
            &[&state, &names.len(), &names.join(", ")],
        );
        Ok(names.len() as i32)
    }
}

pub fn register(dispatcher: &mut RawCommandDispatcher<ProxyCommandSource>) {
    dispatcher.register(
        command(
            "ban",
            "Bans a player from the proxy, optionally for a while",
        )
        .requires(Requirement::from(BAN_PERMISSION))
        .then(
            argument("player", StringArgumentType::SingleWord)
                .suggests(OnlinePlayers)
                .executes(BanCommand)
                .then(argument("reason", StringArgumentType::GreedyPhrase).executes(BanCommand)),
        ),
    );
    dispatcher.register(
        command("unban", "Lifts a player's ban")
            .requires(Requirement::from(BAN_PERMISSION))
            .then(argument("player", StringArgumentType::SingleWord).executes(UnbanCommand)),
    );
    dispatcher.register(
        command("banip", "Bans an IP address, or the one a player is on")
            .requires(Requirement::from(BANIP_PERMISSION))
            .then(
                argument("target", StringArgumentType::SingleWord)
                    .suggests(OnlinePlayers)
                    .executes(BanIpCommand)
                    .then(
                        argument("reason", StringArgumentType::GreedyPhrase).executes(BanIpCommand),
                    ),
            ),
    );
    dispatcher.register(
        command("unbanip", "Lifts an IP ban")
            .requires(Requirement::from(BANIP_PERMISSION))
            .then(argument("ip", StringArgumentType::SingleWord).executes(UnbanIpCommand)),
    );
    dispatcher.register(
        command("banlist", "Lists banned players and IP addresses")
            .requires(Requirement::from(BAN_PERMISSION))
            .executes(BanListCommand),
    );
    dispatcher.register(
        command("kick", "Disconnects a player from the proxy")
            .requires(Requirement::from(KICK_PERMISSION))
            .then(
                argument("player", StringArgumentType::SingleWord)
                    .suggests(OnlinePlayers)
                    .executes(KickCommand)
                    .then(
                        argument("reason", StringArgumentType::GreedyPhrase).executes(KickCommand),
                    ),
            ),
    );
    dispatcher.register(
        command("maintenance", "Turns maintenance mode on or off")
            .requires(Requirement::from(MAINTENANCE_PERMISSION))
            .then(literal("on").executes(MaintenanceToggle(true)))
            .then(literal("off").executes(MaintenanceToggle(false))),
    );
    dispatcher.register(
        command("whitelist", "Manages the proxy whitelist")
            .requires(Requirement::from(WHITELIST_PERMISSION))
            .then(literal("on").executes(WhitelistToggle(true)))
            .then(literal("off").executes(WhitelistToggle(false)))
            .then(literal("list").executes(WhitelistList))
            .then(
                literal("add").then(
                    argument("player", StringArgumentType::SingleWord)
                        .suggests(OnlinePlayers)
                        .executes(WhitelistAdd),
                ),
            )
            .then(literal("remove").then(
                argument("player", StringArgumentType::SingleWord).executes(WhitelistRemove),
            )),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_duration_and_reason() {
        assert_eq!(
            duration_and_reason(Some("1d spamming chat")),
            (
                Some(Duration::from_secs(86_400)),
                Some("spamming chat".to_string())
            )
        );
        assert_eq!(
            duration_and_reason(Some("griefing the spawn")),
            (None, Some("griefing the spawn".to_string()))
        );
        assert_eq!(
            duration_and_reason(Some("2h")),
            (Some(Duration::from_secs(7_200)), None)
        );
        assert_eq!(duration_and_reason(None), (None, None));
    }
}
