use std::collections::HashMap;
use std::path::PathBuf;

/// Launch-count history, persisted to %APPDATA%\kwick\history.toml.
/// Used to boost frequently used items and to fill the empty-query view.
pub struct History {
    counts: HashMap<String, u32>,
    path: PathBuf,
}

impl History {
    pub fn load() -> Self {
        let path = crate::config::config_dir().join("history.toml");
        let counts = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default();
        Self { counts, path }
    }

    pub fn bump(&mut self, key: &str) {
        *self.counts.entry(key.to_string()).or_insert(0) += 1;
        if let Ok(text) = toml::to_string(&self.counts) {
            let _ = std::fs::write(&self.path, text);
        }
    }

    pub fn count(&self, key: &str) -> u32 {
        self.counts.get(key).copied().unwrap_or(0)
    }

    /// Score bonus added on top of the fuzzy-match score, capped so that
    /// history never completely drowns out match quality.
    pub fn bonus(&self, key: &str) -> u32 {
        self.count(key).min(20) * 15
    }
}
