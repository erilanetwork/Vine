//! Loads the hello example plugin, built with the SDK, into the real plugin host.
//!
//! The plugin has to be built for wasm32-wasip2 first:
//! `cargo build -p hello-plugin --target wasm32-wasip2 --release`. Point `VINE_TEST_PLUGIN` at
//! the .wasm to use another build; without one the test is skipped.

use std::path::PathBuf;
use std::sync::Arc;

use vine_config::{Config, PluginConfig};
use vine_plugin::command::CommandDispatcher;
use vine_plugin::plugin::PluginManager;
use vine_plugin::plugin::api::ProxyPingEvent;
use vine_plugin::services::ProxyServices;
use vine_plugin::session::SessionManager;

fn example_plugin() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("VINE_TEST_PLUGIN") {
        return Some(PathBuf::from(path));
    }
    let target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    let path = target.join("wasm32-wasip2/release/hello_plugin.wasm");
    path.exists().then_some(path)
}

#[tokio::test]
async fn test_sdk_plugin_registers_events_and_commands() {
    let Some(wasm) = example_plugin() else {
        eprintln!("skipping: build hello-plugin for wasm32-wasip2 or set VINE_TEST_PLUGIN");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(&wasm, dir.path().join("hello.wasm")).unwrap();

    let dispatcher = Arc::new(CommandDispatcher::new());
    let manager =
        PluginManager::new(dir.path().to_path_buf(), (&PluginConfig::default()).into()).unwrap();
    manager.init_proxy_context(
        ProxyServices::from_config(Config::default()),
        Arc::new(SessionManager::new()),
        dispatcher.clone(),
    );

    assert_eq!(manager.load_plugins().await.unwrap(), 1);
    let plugins = manager.plugin_list().await;
    assert_eq!(plugins[0].0.name, "hello");
    assert!(!plugins[0].1, "the plugin got disabled");
    assert!(dispatcher.has_command("hello"));
    assert!(dispatcher.has_command("hi"));

    let mut ping = ProxyPingEvent {
        client_ip: "127.0.0.1".to_string(),
        protocol_version: 769,
        virtual_host: "localhost".to_string(),
        motd: "A Vine Server".to_string(),
        max_players: 100,
        online_players: 3,
        version_name: "Vine".to_string(),
    };
    manager.fire_proxy_ping(&mut ping).await;
    assert_eq!(ping.motd, "A Vine Server\n§73 players online");

    manager.unload_plugins().await;
    assert!(manager.plugin_list().await.is_empty());
}
