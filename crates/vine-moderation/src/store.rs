use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// A list kept in a JSON file next to the config, rewritten whole on every change. Without a
/// path it only lives in memory.
pub(crate) struct JsonStore<T> {
    path: Option<PathBuf>,
    entries: Mutex<Vec<T>>,
}

impl<T: Serialize + DeserializeOwned + Clone> JsonStore<T> {
    pub(crate) fn load(path: &Path) -> Result<Self, String> {
        let entries = match std::fs::read_to_string(path) {
            Ok(text) if text.trim().is_empty() => Vec::new(),
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| format!("failed to read '{}': {e}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(format!("failed to read '{}': {e}", path.display())),
        };
        Ok(Self {
            path: Some(path.to_path_buf()),
            entries: Mutex::new(entries),
        })
    }

    pub(crate) fn in_memory() -> Self {
        Self {
            path: None,
            entries: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, Vec<T>> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Writes the entries out. Goes through a temporary file so a crash mid write can't leave
    /// half a ban list behind.
    pub(crate) fn save(&self, entries: &[T]) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let json = serde_json::to_string_pretty(entries).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)
            .and_then(|()| std::fs::rename(&tmp, path))
            .map_err(|e| format!("failed to write '{}': {e}", path.display()))
    }
}
