use super::{Action, Item};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Scan executables on PATH.
pub fn scan() -> Vec<Item> {
    let Some(path_var) = std::env::var_os("PATH") else {
        return Vec::new();
    };
    let mut seen: HashSet<String> = HashSet::new();
    let mut items = Vec::new();
    for dir in std::env::split_paths(&path_var) {
        scan_dir(&dir, &mut seen, &mut items);
    }
    items
}

/// Scan Chocolatey's shim folder (%ChocolateyInstall%\bin, default
/// C:\ProgramData\chocolatey\bin). Independent of `scan_path` so choco tools can
/// be indexed without pulling in every exe on PATH.
pub fn scan_chocolatey() -> Vec<Item> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut items = Vec::new();
    scan_dir(&chocolatey_bin(), &mut seen, &mut items);
    for item in &mut items {
        item.subtitle = format!("Chocolatey - {}", item.subtitle);
        // User opted in explicitly; rank alongside Start Menu apps.
        item.rank_boost = 40;
    }
    items
}

fn chocolatey_bin() -> PathBuf {
    let root = match std::env::var_os("ChocolateyInstall") {
        Some(dir) => PathBuf::from(dir),
        None => match std::env::var_os("ProgramData") {
            Some(dir) => PathBuf::from(dir).join("chocolatey"),
            None => PathBuf::from(r"C:\ProgramData\chocolatey"),
        },
    };
    root.join("bin")
}

/// Add every executable directly under `dir`, skipping names already in `seen`.
fn scan_dir(dir: &Path, seen: &mut HashSet<String>, items: &mut Vec<Item>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
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
