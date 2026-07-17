use super::{Action, Item};
use crate::config::ScanFolder;
use std::collections::HashSet;
use walkdir::WalkDir;

const DEFAULT_EXTENSIONS: &[&str] = &["exe", "lnk", "bat", "cmd", "url"];

/// Scan user-configured folders ([[scan_folders]] in config.toml).
pub fn scan(folders: &[ScanFolder]) -> Vec<Item> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut items = Vec::new();
    for folder in folders {
        let extensions: Vec<String> = match &folder.extensions {
            Some(exts) => exts.iter().map(|e| e.to_ascii_lowercase()).collect(),
            None => DEFAULT_EXTENSIONS.iter().map(|e| e.to_string()).collect(),
        };
        let walker = WalkDir::new(&folder.path)
            .max_depth(folder.depth.max(1))
            .into_iter()
            .flatten();
        for entry in walker {
            let path = entry.path();
            let Some(ext) = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase())
            else {
                continue;
            };
            if !extensions.iter().any(|e| e == &ext) {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if !seen.insert(path.display().to_string().to_ascii_lowercase()) {
                continue;
            }
            let mut item = Item::new(
                name,
                path.display().to_string(),
                Action::Open(path.display().to_string()),
            );
            // User picked these folders explicitly; rank alongside Start Menu apps.
            item.rank_boost = 40;
            items.push(item);
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(path: &std::path::Path, depth: usize, exts: Option<Vec<String>>) -> ScanFolder {
        ScanFolder {
            path: path.display().to_string(),
            depth,
            extensions: exts,
        }
    }

    #[test]
    fn scans_default_extensions_up_to_depth() {
        let dir = std::env::temp_dir().join(format!("kwick-test-{}", std::process::id()));
        let sub = dir.join("sub");
        let deep = sub.join("deep");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(dir.join("tool.exe"), b"").unwrap();
        std::fs::write(dir.join("note.txt"), b"").unwrap();
        std::fs::write(sub.join("script.bat"), b"").unwrap();
        std::fs::write(deep.join("hidden.exe"), b"").unwrap();

        // depth 2 = root + sub; deep/ is excluded, .txt is not a default ext.
        let items = scan(&[folder(&dir, 2, None)]);
        let mut titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
        titles.sort();
        assert_eq!(titles, ["script", "tool"]);

        // Custom extension filter only picks .txt.
        let items = scan(&[folder(&dir, 1, Some(vec!["txt".into()]))]);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "note");
        assert_eq!(items[0].rank_boost, 40);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
