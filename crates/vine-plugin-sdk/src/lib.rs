//! Write Vine proxy plugins in Rust.
//!
//! A plugin is a type implementing [`Plugin`], registered with [`register_plugin!`] and built
//! for `wasm32-wasip2` as a `cdylib`. Drop the resulting `.wasm` into the proxy's `plugins/`
//! folder and run `/vine reload` (or restart).
//!
//! ```rust,ignore
//! use vine_plugin_sdk::{Context, Event, EventPriority, EventType, Plugin, PluginMetadata};
//!
//! struct Hello;
//!
//! impl Plugin for Hello {
//!     fn new() -> Self {
//!         Hello
//!     }
//!
//!     fn metadata(&self) -> PluginMetadata {
//!         PluginMetadata::new("hello", env!("CARGO_PKG_VERSION"))
//!     }
//!
//!     fn on_load(&self, context: &Context) -> Result<(), String> {
//!         context.on(EventType::PlayerJoin, EventPriority::Normal, |event| {
//!             if let Event::PlayerJoin(join) = &event {
//!                 vine_plugin_sdk::proxy::broadcast(&format!("Welcome, {}!", join.username));
//!             }
//!             event
//!         });
//!         Ok(())
//!     }
//! }
//!
//! vine_plugin_sdk::register_plugin!(Hello);
//! ```
//!
//! Plugins run inside a WASI sandbox. Files go in the data folder from
//! [`Context::data_folder`], read and written with plain `std::fs`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

// generated bindings, clippy has opinions about code nobody here wrote
#[allow(clippy::all)]
mod wit {
    wit_bindgen::generate!({
        // the plugin crate defines init-plugin through register_plugin!, that's how the SDK
        // learns which type the plugin is
        skip: ["init-plugin"],
        path: "../vine-plugin-wit/wit",
        world: "plugin",
    });

    use super::Component;
    export!(Component);
}

pub use wit::vine::plugin::event::{
    Event, EventType, PlayerChatEvent, PlayerCommandEvent, PlayerDisconnectEvent, PlayerJoinEvent,
    PlayerLoginEvent, PlayerPreLoginEvent, PluginMessageEvent, PluginMessageSource, ProxyPingEvent,
    ServerConnectEvent, ServerConnectedEvent,
};
pub use wit::vine::plugin::metadata::PluginMetadata;
pub use wit::vine::plugin::types::{
    CommandSender, CommandSenderType, EventPriority, PlayerInfo, ServerInfo,
};

/// Querying and acting on players and servers
pub mod proxy {
    pub use crate::wit::vine::plugin::proxy::*;
}

/// Logging to the proxy console, prefixed with the plugin's name
pub mod log {
    pub use crate::wit::vine::plugin::logging::Level;
    use crate::wit::vine::plugin::logging::log;

    pub fn debug(message: &str) {
        log(Level::Debug, message);
    }

    pub fn info(message: &str) {
        log(Level::Info, message);
    }

    pub fn warn(message: &str) {
        log(Level::Warn, message);
    }

    pub fn error(message: &str) {
        log(Level::Error, message);
    }
}

/// Translated messages. Keys without a namespace are the plugin's own.
pub mod translation {
    pub use crate::wit::vine::plugin::translation::*;
}

impl PluginMetadata {
    /// Metadata with just a name and version, fill in the rest as needed
    pub fn new(name: &str, version: &str) -> Self {
        Self {
            name: name.to_string(),
            version: version.to_string(),
            authors: Vec::new(),
            description: None,
            dependencies: Vec::new(),
        }
    }
}

impl CommandSender {
    /// Sends the sender a message, `§` color codes included
    pub fn reply(&self, message: &str) {
        wit::vine::plugin::command::reply(self, message);
    }

    pub fn is_console(&self) -> bool {
        self.sender_type == CommandSenderType::Console
    }
}

/// A Vine plugin. The proxy creates it once with `new`, asks for its metadata, then calls
/// `on_load`, where it registers its event handlers and commands.
pub trait Plugin: Send + Sync + 'static {
    fn new() -> Self
    where
        Self: Sized;

    fn metadata(&self) -> PluginMetadata;

    fn on_load(&self, _context: &Context) -> Result<(), String> {
        Ok(())
    }

    fn on_unload(&self, _context: &Context) -> Result<(), String> {
        Ok(())
    }
}

type EventHandler = Arc<dyn Fn(Event) -> Event + Send + Sync>;
type CommandHandler = Arc<dyn Fn(&CommandSender, &[String]) -> Result<i32, String> + Send + Sync>;
type SuggestionHandler = Arc<dyn Fn(&CommandSender, &[String]) -> Vec<String> + Send + Sync>;

#[derive(Default)]
struct Handlers {
    events: HashMap<u32, EventHandler>,
    commands: HashMap<u32, (CommandHandler, Option<SuggestionHandler>)>,
}

static HANDLERS: OnceLock<Mutex<Handlers>> = OnceLock::new();
static NEXT_ID: AtomicU32 = AtomicU32::new(1);

fn handlers() -> std::sync::MutexGuard<'static, Handlers> {
    HANDLERS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Handed to `on_load` and `on_unload`. Registrations made here stay until the plugin is
/// unloaded or reloaded.
pub struct Context(wit::vine::plugin::context::Context);

impl Context {
    /// Calls `handler` for every event of `event_type`. Return the event, changed or not; for
    /// cancellable events set `cancelled` to stop it. Monitor handlers only watch, what they
    /// return is ignored.
    pub fn on<F>(&self, event_type: EventType, priority: EventPriority, handler: F)
    where
        F: Fn(Event) -> Event + Send + Sync + 'static,
    {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        handlers().events.insert(id, Arc::new(handler));
        self.0.register_event(id, event_type, priority);
    }

    /// Registers `/name`. The handler gets the sender and the words after the command, and
    /// returns a result count or an error message for the sender. With a permission, only
    /// senders that have it see and run the command.
    pub fn command<F>(
        &self,
        name: &str,
        description: &str,
        aliases: &[&str],
        permission: Option<&str>,
        handler: F,
    ) where
        F: Fn(&CommandSender, &[String]) -> Result<i32, String> + Send + Sync + 'static,
    {
        self.register_command(
            name,
            description,
            aliases,
            permission,
            Arc::new(handler),
            None,
        );
    }

    /// Like [`Context::command`], with tab completion. `suggest` gets the words typed after
    /// the command so far and returns candidates for the word being typed.
    pub fn command_with_suggestions<F, S>(
        &self,
        name: &str,
        description: &str,
        aliases: &[&str],
        permission: Option<&str>,
        handler: F,
        suggest: S,
    ) where
        F: Fn(&CommandSender, &[String]) -> Result<i32, String> + Send + Sync + 'static,
        S: Fn(&CommandSender, &[String]) -> Vec<String> + Send + Sync + 'static,
    {
        self.register_command(
            name,
            description,
            aliases,
            permission,
            Arc::new(handler),
            Some(Arc::new(suggest)),
        );
    }

    fn register_command(
        &self,
        name: &str,
        description: &str,
        aliases: &[&str],
        permission: Option<&str>,
        handler: CommandHandler,
        suggest: Option<SuggestionHandler>,
    ) {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        handlers().commands.insert(id, (handler, suggest));
        let aliases: Vec<String> = aliases.iter().map(|a| a.to_string()).collect();
        self.0
            .register_command(id, name, description, &aliases, permission);
    }

    /// The plugin's data folder as the sandbox sees it, ready for `std::fs`
    pub fn data_folder(&self) -> String {
        self.0.get_data_folder()
    }
}

static PLUGIN: OnceLock<Box<dyn Plugin>> = OnceLock::new();

#[doc(hidden)]
pub fn __register(build: fn() -> Box<dyn Plugin>) {
    let _ = PLUGIN.set(build());
}

fn plugin() -> &'static dyn Plugin {
    // hmm, the host always calls init-plugin first, so this only fires for a broken build
    PLUGIN
        .get()
        .map(Box::as_ref)
        .expect("vine_plugin_sdk::register_plugin! was not used")
}

struct Component;

impl wit::Guest for Component {
    fn on_load(ctx: wit::vine::plugin::context::Context) -> Result<(), String> {
        plugin().on_load(&Context(ctx))
    }

    fn on_unload(ctx: wit::vine::plugin::context::Context) -> Result<(), String> {
        let result = plugin().on_unload(&Context(ctx));
        let mut handlers = handlers();
        handlers.events.clear();
        handlers.commands.clear();
        result
    }

    fn get_metadata() -> PluginMetadata {
        plugin().metadata()
    }

    fn handle_event(handler_id: u32, evt: Event) -> Event {
        // cloned out so a handler can register more handlers without deadlocking
        let handler = handlers().events.get(&handler_id).cloned();
        match handler {
            Some(handler) => handler(evt),
            None => evt,
        }
    }

    fn handle_command(
        command_id: u32,
        sender: CommandSender,
        args: Vec<String>,
    ) -> Result<i32, String> {
        let handler = handlers().commands.get(&command_id).map(|(h, _)| h.clone());
        match handler {
            Some(handler) => handler(&sender, &args),
            None => Err(format!("no handler for command {command_id}")),
        }
    }

    fn handle_command_suggestion(
        command_id: u32,
        sender: CommandSender,
        args: Vec<String>,
    ) -> Vec<String> {
        let suggest = handlers()
            .commands
            .get(&command_id)
            .and_then(|(_, s)| s.clone());
        suggest.map(|s| s(&sender, &args)).unwrap_or_default()
    }
}

/// Registers the plugin type. Use it once, at the root of the plugin crate.
#[macro_export]
macro_rules! register_plugin {
    ($plugin:ty) => {
        #[unsafe(export_name = "init-plugin")]
        pub extern "C" fn __vine_init_plugin() {
            $crate::__register(|| ::std::boxed::Box::new(<$plugin as $crate::Plugin>::new()));
        }
    };
}
