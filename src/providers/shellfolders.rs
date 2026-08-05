use super::{Action, Item};
use std::collections::HashSet;
use std::path::PathBuf;

/// Frequently visited Windows folders (Downloads, AppData, Temp, Program
/// Files...). They have no Start Menu entry and are tedious to reach through
/// Explorer, so we list them explicitly. Gated by `special_folders` in
/// config.toml (default: true).
///
/// User folders go through `dirs`, which asks the shell for the known folder
/// and therefore follows OneDrive redirection; the rest come from environment
/// variables. Entries whose folder does not exist are dropped.
pub fn scan() -> Vec<Item> {
    let home = dirs::home_dir();
    let appdata = env_path("APPDATA");
    let windir = env_path("WINDIR").or_else(|| Some(PathBuf::from(r"C:\Windows")));

    // (title, match aliases, path)
    let candidates: Vec<(&str, &str, Option<PathBuf>)> = vec![
        ("ホーム", "home user profile userprofile", home.clone()),
        ("デスクトップ", "desktop", dirs::desktop_dir()),
        ("ダウンロード", "downloads download", dirs::download_dir()),
        (
            "ドキュメント",
            "documents my documents",
            dirs::document_dir(),
        ),
        ("ピクチャ", "pictures photos images", dirs::picture_dir()),
        ("ビデオ", "videos movies", dirs::video_dir()),
        ("ミュージック", "music audio", dirs::audio_dir()),
        ("OneDrive", "onedrive", env_path("OneDrive")),
        (
            "AppData",
            "appdata アプリデータ",
            home.as_ref().map(|h| h.join("AppData")),
        ),
        ("AppData\\Roaming", "appdata roaming", appdata.clone()),
        (
            "AppData\\Local",
            "appdata local localappdata",
            env_path("LOCALAPPDATA"),
        ),
        (
            "AppData\\LocalLow",
            "appdata locallow",
            home.as_ref().map(|h| h.join(r"AppData\LocalLow")),
        ),
        ("Temp", "temp tmp local temp 一時ファイル", env_path("TEMP")),
        (
            "スタートアップ",
            "startup 自動起動",
            appdata
                .as_ref()
                .map(|a| a.join(r"Microsoft\Windows\Start Menu\Programs\Startup")),
        ),
        (
            "最近使ったファイル",
            "recent items 履歴",
            appdata
                .as_ref()
                .map(|a| a.join(r"Microsoft\Windows\Recent")),
        ),
        ("Program Files", "program files", env_path("ProgramFiles")),
        (
            "Program Files (x86)",
            "program files x86",
            env_path("ProgramFiles(x86)"),
        ),
        ("ProgramData", "programdata", env_path("ProgramData")),
        ("Public", "public users public 共有", env_path("PUBLIC")),
        ("Windows", "windows windir", windir.clone()),
        (
            "System32",
            "system32 system",
            windir.as_ref().map(|w| w.join("System32")),
        ),
    ];

    let mut seen: HashSet<String> = HashSet::new();
    let mut items = Vec::new();
    for (title, aliases, path) in candidates {
        let Some(path) = path else { continue };
        if !path.is_dir() {
            continue;
        }
        let path = path.display().to_string();
        // %ProgramFiles(x86)% == %ProgramFiles% on 32-bit, OneDrive may be the
        // home of the redirected user folders, ...
        if !seen.insert(path.to_ascii_lowercase()) {
            continue;
        }
        let mut item = Item::new(title, path.clone(), Action::Open(path));
        item.key = format!("{title} {aliases}");
        // Below Start Menu apps and curated tools (40): typing an app name that
        // happens to look like a folder should still put the app first.
        item.rank_boost = 30;
        items.push(item);
    }
    items
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_existing_folders_without_duplicates() {
        let items = scan();

        // TEMP and the user profile exist on any Windows box the app runs on.
        let temp = items.iter().find(|i| i.title == "Temp").expect("Temp");
        assert!(matches!(&temp.action, Action::Open(p) if p == &temp.subtitle));
        assert!(temp.key.contains("tmp"));
        assert_eq!(temp.rank_boost, 30);
        assert!(items.iter().any(|i| i.title == "ホーム"));

        let mut paths: Vec<String> = items
            .iter()
            .map(|i| i.subtitle.to_ascii_lowercase())
            .collect();
        let count = paths.len();
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), count, "同じフォルダが二重に出ている");
    }
}
