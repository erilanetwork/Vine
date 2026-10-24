use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::watch;

use crate::Config;

/// The config the proxy is running with right now. Readers take a cheap snapshot with `get`,
/// and anything that has to react to a reload can `subscribe`.
pub struct SharedConfig {
    path: PathBuf,
    current: watch::Sender<Arc<Config>>,
}

impl SharedConfig {
    pub fn new(config: Config, path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            current: watch::Sender::new(Arc::new(config)),
        }
    }

    pub fn get(&self) -> Arc<Config> {
        self.current.borrow().clone()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn subscribe(&self) -> watch::Receiver<Arc<Config>> {
        self.current.subscribe()
    }

    /// Swaps in a new config, telling every subscriber
    pub fn replace(&self, config: Config) {
        self.current.send_replace(Arc::new(config));
    }

    /// Reads the file again and swaps it in. A broken file leaves the running config alone.
    pub fn reload(&self) -> Result<Arc<Config>, String> {
        let config = Config::load(&self.path).map_err(|e| e.to_string())?;
        self.replace(config);
        Ok(self.get())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reload_keeps_running_config_on_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vine.toml");
        let config = Config::load_or_create(&path).unwrap();
        let shared = SharedConfig::new(config, &path);
        let changes = shared.subscribe();

        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("A Vine Server", "Reloaded");
        std::fs::write(&path, text).unwrap();
        assert_eq!(shared.reload().unwrap().motd.text, "Reloaded");
        assert!(changes.has_changed().unwrap());

        std::fs::write(&path, "not toml [").unwrap();
        assert!(shared.reload().is_err());
        assert_eq!(shared.get().motd.text, "Reloaded");
    }
}
