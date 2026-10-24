//! Greets players, adds a /hello command and puts the player count in the server list.
//!
//! Build it with `cargo build -p hello-plugin --target wasm32-wasip2 --release` and copy
//! `target/wasm32-wasip2/release/hello_plugin.wasm` into the proxy's `plugins/` folder.

use std::sync::atomic::{AtomicU32, Ordering};

use vine_plugin_sdk::{
    Context, Event, EventPriority, EventType, Plugin, PluginMetadata, log, proxy, translation,
};

static GREETED: AtomicU32 = AtomicU32::new(0);

struct Hello;

impl Plugin for Hello {
    fn new() -> Self {
        Hello
    }

    fn metadata(&self) -> PluginMetadata {
        PluginMetadata {
            authors: vec!["Vine".to_string()],
            description: Some("Says hello".to_string()),
            ..PluginMetadata::new("hello", env!("CARGO_PKG_VERSION"))
        }
    }

    fn on_load(&self, context: &Context) -> Result<(), String> {
        // defaults only, server owners can edit lang/hello/en_us.json afterwards
        translation::register_translations(
            "en_us",
            &[
                (
                    "welcome".to_string(),
                    "§aWelcome to the network, %s!".to_string(),
                ),
                (
                    "hello".to_string(),
                    "Hello, %s! %s players are online.".to_string(),
                ),
            ],
        );

        context.on(EventType::PlayerJoin, EventPriority::Normal, |event| {
            if let Event::PlayerJoin(join) = &event {
                GREETED.fetch_add(1, Ordering::Relaxed);
                let text = translation::translate(
                    Some(&join.uuid),
                    "welcome",
                    std::slice::from_ref(&join.username),
                );
                proxy::send_message(&join.uuid, &text);
            }
            event
        });

        context.on(EventType::ProxyPing, EventPriority::Normal, |mut event| {
            if let Event::ProxyPing(ping) = &mut event {
                ping.motd = format!(
                    "{}\n§7{} players online",
                    ping.motd.lines().next().unwrap_or_default(),
                    ping.online_players
                );
            }
            event
        });

        context.command_with_suggestions(
            "hello",
            "Says hello back",
            &["hi"],
            Some("hello.command"),
            |sender, args| {
                let name = args.first().cloned().unwrap_or_else(|| sender.name.clone());
                let text = translation::translate(
                    sender.player_uuid.as_deref(),
                    "hello",
                    &[name, proxy::get_player_count().to_string()],
                );
                sender.reply(&text);
                Ok(1)
            },
            |_, args| {
                let typed = args
                    .last()
                    .map(|a| a.to_ascii_lowercase())
                    .unwrap_or_default();
                proxy::get_players()
                    .into_iter()
                    .map(|p| p.username)
                    .filter(|name| name.to_ascii_lowercase().starts_with(&typed))
                    .collect()
            },
        );

        log::info("Hello plugin loaded");
        Ok(())
    }

    fn on_unload(&self, _context: &Context) -> Result<(), String> {
        log::info(&format!(
            "Greeted {} players",
            GREETED.load(Ordering::Relaxed)
        ));
        Ok(())
    }
}

vine_plugin_sdk::register_plugin!(Hello);
