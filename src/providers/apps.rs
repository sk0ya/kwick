use super::{Action, Item};
use std::collections::HashSet;
use std::path::PathBuf;
use walkdir::WalkDir;

/// Scan Start Menu (user + common) for .lnk / .url shortcuts.
pub fn scan() -> Vec<Item> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        roots.push(PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    if let Some(program_data) = std::env::var_os("ProgramData") {
        roots.push(PathBuf::from(program_data).join(r"Microsoft\Windows\Start Menu\Programs"));
    }

    let mut seen: HashSet<String> = HashSet::new();
    let mut items = Vec::new();
    for root in roots {
        for entry in WalkDir::new(&root).into_iter().flatten() {
            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase());
            if !matches!(ext.as_deref(), Some("lnk") | Some("url")) {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if name.to_ascii_lowercase().contains("uninstall") {
                continue;
            }
            if !seen.insert(name.to_ascii_lowercase()) {
                continue;
            }
            items.push(Item::new(
                name,
                "アプリ",
                Action::Open(path.display().to_string()),
            ));
        }
    }
    items
}
