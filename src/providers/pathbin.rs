use super::{Action, Item};
use std::collections::HashSet;

/// Scan executables on PATH.
pub fn scan() -> Vec<Item> {
    let Some(path_var) = std::env::var_os("PATH") else {
        return Vec::new();
    };
    let mut seen: HashSet<String> = HashSet::new();
    let mut items = Vec::new();
    for dir in std::env::split_paths(&path_var) {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase());
            if !matches!(ext.as_deref(), Some("exe") | Some("bat") | Some("cmd") | Some("com")) {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if !seen.insert(name.to_ascii_lowercase()) {
                continue;
            }
            items.push(Item::new(
                name,
                path.display().to_string(),
                Action::Open(path.display().to_string()),
            ));
        }
    }
    items
}
