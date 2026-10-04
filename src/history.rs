use std::collections::HashMap;
use std::path::PathBuf;

/// Upper bound on remembered queries, so learned.toml cannot grow forever.
const MAX_LEARNED_QUERIES: usize = 500;

/// Launch-count history, persisted to %APPDATA%\kwick\history.toml.
/// Used to boost frequently used items and to fill the empty-query view.
///
/// It also learns which item was picked for which query (learned.toml):
/// once "co" has launched VS Code, "co" ranks VS Code first.
pub struct History {
    counts: HashMap<String, u32>,
    path: PathBuf,
    /// lowercased query -> (title -> times picked)
    learned: HashMap<String, HashMap<String, u32>>,
    learned_path: PathBuf,
}

impl History {
    pub fn load() -> Self {
        let dir = crate::config::config_dir();
        Self::load_from(dir.join("history.toml"), dir.join("learned.toml"))
    }

    fn load_from(path: PathBuf, learned_path: PathBuf) -> Self {
        fn read<T: serde::de::DeserializeOwned + Default>(path: &PathBuf) -> T {
            std::fs::read_to_string(path)
                .ok()
                .and_then(|text| toml::from_str(&text).ok())
                .unwrap_or_default()
        }
        Self {
            counts: read(&path),
            path,
            learned: read(&learned_path),
            learned_path,
        }
    }

    pub fn bump(&mut self, key: &str) {
        *self.counts.entry(key.to_string()).or_insert(0) += 1;
        self.save();
    }

    /// Remember that `title` was picked for `query`.
    pub fn learn(&mut self, query: &str, title: &str) {
        let query = normalize(query);
        if query.is_empty() {
            return;
        }
        *self
            .learned
            .entry(query)
            .or_default()
            .entry(title.to_string())
            .or_insert(0) += 1;
        if self.learned.len() > MAX_LEARNED_QUERIES {
            // Drop the least used query.
            if let Some(victim) = self
                .learned
                .iter()
                .min_by_key(|(_, picks)| picks.values().sum::<u32>())
                .map(|(q, _)| q.clone())
            {
                self.learned.remove(&victim);
            }
        }
        self.save_learned();
    }

    /// Forget an item entirely (drops it from the most-used view and
    /// removes its score bonus and learned queries).
    pub fn remove(&mut self, key: &str) {
        if self.counts.remove(key).is_some() {
            self.save();
        }
        let mut changed = false;
        self.learned.retain(|_, picks| {
            changed |= picks.remove(key).is_some();
            !picks.is_empty()
        });
        if changed {
            self.save_learned();
        }
    }

    fn save(&self) {
        if let Ok(text) = toml::to_string(&self.counts) {
            let _ = std::fs::write(&self.path, text);
        }
    }

    fn save_learned(&self) {
        if let Ok(text) = toml::to_string(&self.learned) {
            let _ = std::fs::write(&self.learned_path, text);
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

    /// Per-title bonus for the current query, computed once per search.
    ///
    /// An item picked for exactly this query goes to the top; one picked for
    /// a longer query that starts with it ("code" when typing "co") gets a
    /// smaller push.
    pub fn learned_bonuses(&self, query: &str) -> HashMap<String, u32> {
        let query = normalize(query);
        let mut out: HashMap<String, u32> = HashMap::new();
        if query.is_empty() {
            return out;
        }
        for (learned, picks) in &self.learned {
            let exact = *learned == query;
            if !exact && !learned.starts_with(&query) {
                continue;
            }
            for (title, &n) in picks {
                let bonus = if exact {
                    1000 + n.min(10) * 30
                } else {
                    150 + n.min(10) * 10
                };
                let slot = out.entry(title.clone()).or_insert(0);
                *slot = (*slot).max(bonus);
            }
        }
        out
    }
}

fn normalize(query: &str) -> String {
    query.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_history() -> History {
        let dir = std::env::temp_dir().join(format!(
            "kwick-history-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::remove_file(dir.join("h.toml"));
        let _ = std::fs::remove_file(dir.join("l.toml"));
        History::load_from(dir.join("h.toml"), dir.join("l.toml"))
    }

    #[test]
    fn learned_query_ranks_exact_above_prefix() {
        let mut h = temp_history();
        h.learn("co", "Visual Studio Code");
        h.learn("Code ", "Visual Studio Code");
        h.learn("cod", "Codex");
        let b = h.learned_bonuses("CO");
        assert!(b["Visual Studio Code"] >= 1000);
        assert!(b["Codex"] < 1000 && b["Codex"] > 0);
        assert!(h.learned_bonuses("x").is_empty());

        h.remove("Visual Studio Code");
        assert!(!h.learned_bonuses("co").contains_key("Visual Studio Code"));
    }
}
