use std::sync::{Arc, Weak};

use pumpkin_command::Requirement;
use pumpkin_command::argument_builder::{ArgumentBuilder, argument, command, literal};
use pumpkin_command::argument_types::core::string::StringArgumentType;
use pumpkin_command::context::command_context::CommandContext;
use pumpkin_command::node::{CommandExecutor, CommandExecutorResult};
use pumpkin_command::suggestion::provider::{SuggestionProvider, SuggestionProviderResult};
use pumpkin_command::suggestion::suggestions::SuggestionsBuilder;

use tracing::{info, warn};

use super::PluginManager;
use crate::command::{CommandDispatcher, ProxyCommandSource};

const PERMISSION: &str = "vine.command.plugins";
const RELOAD_PERMISSION: &str = "vine.command.reload";

pub fn register(dispatcher: &CommandDispatcher, manager: &Arc<PluginManager>) {
    let manager = Arc::downgrade(manager);
    dispatcher.register(
        command("vine", "Manages the proxy's plugins and config")
            // either node opens /vine, the subcommands check their own
            .requires(Requirement::from(|source: &ProxyCommandSource| {
                source.has_permission(PERMISSION) || source.has_permission(RELOAD_PERMISSION)
            }))
            .then(
                literal("config")
                    .requires(Requirement::from(RELOAD_PERMISSION))
                    .then(literal("reload").executes(ReloadConfig)),
            )
            .then(
                literal("plugins")
                    .requires(Requirement::from(PERMISSION))
                    .executes(ListPlugins {
                        manager: manager.clone(),
                    }),
            )
            .then(
                literal("reload")
                    .requires(Requirement::from(PERMISSION))
                    .executes(ReloadPlugins {
                        manager: manager.clone(),
                    })
                    .then(
                        argument("plugin", StringArgumentType::SingleWord)
                            .suggests(PluginNames {
                                manager: manager.clone(),
                            })
                            .executes(ReloadPlugins { manager }),
                    ),
            ),
    );
}

// Runs off the dispatcher lock, since reloading registers and deactivates commands
fn spawn_reply<F, Fut>(context: &CommandContext<ProxyCommandSource>, task: F)
where
    F: FnOnce(Arc<ProxyCommandSource>) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(task(context.source.clone()));
}

struct ListPlugins {
    manager: Weak<PluginManager>,
}

impl CommandExecutor<ProxyCommandSource> for ListPlugins {
    fn execute(&self, context: &CommandContext<ProxyCommandSource>) -> CommandExecutorResult {
        let manager = self.manager.clone();
        spawn_reply(context, move |source| async move {
            let Some(manager) = manager.upgrade() else {
                return;
            };
            let plugins = manager.plugin_list().await;
            let listed = plugins
                .iter()
                .map(|(metadata, disabled)| {
                    let state = if *disabled {
                        source.tr("vine.command.plugins.disabled", &[])
                    } else {
                        String::new()
                    };
                    source.tr(
                        "vine.command.plugins.entry",
                        &[&metadata.name, &metadata.version, &state],
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            source.reply("vine.command.plugins", &[&plugins.len(), &listed]);
        });
        Ok(1)
    }
}

struct ReloadPlugins {
    manager: Weak<PluginManager>,
}

impl CommandExecutor<ProxyCommandSource> for ReloadPlugins {
    fn execute(&self, context: &CommandContext<ProxyCommandSource>) -> CommandExecutorResult {
        let manager = self.manager.clone();
        let name = StringArgumentType::get(context, "plugin")
            .ok()
            .map(str::to_string);
        spawn_reply(context, move |source| async move {
            let Some(manager) = manager.upgrade() else {
                return;
            };
            let names = match name {
                Some(name) => vec![name],
                None => manager.plugin_names().await,
            };
            if names.is_empty() {
                source.reply("vine.command.plugins.none", &[]);
                return;
            }
            for name in names {
                match manager.reload_plugin(&name).await {
                    Ok(metadata) => source.reply(
                        "vine.command.plugins.reloaded",
                        &[&metadata.name, &metadata.version],
                    ),
                    Err(e) => source.reply("vine.command.plugins.reload_failed", &[&e]),
                }
            }
        });
        Ok(1)
    }
}

/// Reads vine.toml again. Settings that only apply when the proxy starts are reported instead
/// of silently ignored.
struct ReloadConfig;

impl CommandExecutor<ProxyCommandSource> for ReloadConfig {
    fn execute(&self, context: &CommandContext<ProxyCommandSource>) -> CommandExecutorResult {
        let source = &context.source;
        let old = source.config();
        match source.services.config.reload() {
            Ok(new) => {
                info!("Reloaded '{}'", source.services.config.path().display());
                source.reply("vine.command.config.reloaded", &[]);
                let restart = old.restart_required(&new);
                if !restart.is_empty() {
                    source.reply("vine.command.config.restart", &[&restart.join(", ")]);
                }
                Ok(1)
            }
            Err(e) => {
                warn!("Failed to reload the config: {e}");
                source.reply("vine.command.config.failed", &[&e]);
                Ok(0)
            }
        }
    }
}

struct PluginNames {
    manager: Weak<PluginManager>,
}

impl SuggestionProvider<ProxyCommandSource> for PluginNames {
    fn suggest(
        &self,
        _context: &CommandContext<ProxyCommandSource>,
        mut builder: SuggestionsBuilder,
    ) -> SuggestionProviderResult {
        if let Some(manager) = self.manager.upgrade() {
            let remaining = builder.remaining_lowercase().to_string();
            for name in manager.plugin_names_now() {
                if name.to_ascii_lowercase().starts_with(&remaining) {
                    builder = builder.suggest(name);
                }
            }
        }
        builder.build()
    }
}
