//! Typing a path browses the file system: "C:\Us" lists the matching
//! entries of C:\, Tab on a folder steps into it.

use super::{Action, Item};
use std::path::{Path, PathBuf};

/// Whether the query is a path rather than a search: a drive ("C:\",
/// "d:/"), a UNC share, the home folder ("~\") or an environment variable
/// ("%APPDATA%\").
pub fn is_path(q: &str) -> bool {
    let b = q.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/'))
        || q.starts_with(r"\\")
        || q.starts_with("~\\")
        || q.starts_with("~/")
        || (q.starts_with('%') && (q[1..].contains("%\\") || q[1..].contains("%/")))
}

/// Expand "~" and %VARS%, normalize slashes.
pub fn expand(q: &str) -> String {
    let mut s = q.replace('/', "\\");
    if let Some(rest) = s.strip_prefix('~') {
        if let Some(home) = dirs::home_dir() {
            s = format!("{}{rest}", home.display());
        }
    }
    if s.starts_with('%') {
        if let Some(end) = s[1..].find('%') {
            let name = &s[1..=end];
            if let Ok(value) = std::env::var(name) {
                s = format!("{value}{}", &s[end + 2..]);
            }
        }
    }
    s
}

/// (folder to list, partial name typed after the last separator)
pub fn split(expanded: &str) -> (PathBuf, String) {
    match expanded.rfind('\\') {
        Some(i) => (PathBuf::from(&expanded[..=i]), expanded[i + 1..].to_string()),
        None => (PathBuf::from(expanded), String::new()),
    }
}

/// Entries of `dir`, folders first. Hidden/system entries are skipped.
pub fn list(dir: &Path) -> Vec<Item> {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut folders = Vec::new();
    let mut files = Vec::new();
    for entry in entries.flatten().take(5000) {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.file_attributes() & (HIDDEN | SYSTEM) != 0 {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path().display().to_string();
        let is_dir = meta.is_dir();
        let subtitle = if is_dir {
            "フォルダ (Tab で中へ)".to_string()
        } else {
            format_size(meta.len())
        };
        let mut item = Item::new(name, subtitle, Action::Open(path.clone()));
        item.icon_path = Some(path);
        if is_dir {
            folders.push(item.transient());
        } else {
            files.push(item.transient());
        }
    }
    folders.sort_by_key(|a| a.title.to_lowercase());
    files.sort_by_key(|a| a.title.to_lowercase());
    folders.extend(files);
    folders
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

/// What Tab puts in the query box for an item: a folder's path plus "\" to
/// step into it, a file's full path.
pub fn completion(item: &Item) -> Option<String> {
    let path = item.file_path()?;
    if Path::new(path).is_dir() {
        let mut p = path.trim_end_matches('\\').to_string();
        p.push('\\');
        Some(p)
    } else {
        Some(path.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_paths() {
        assert!(is_path(r"C:\Us"));
        assert!(is_path("d:/x"));
        assert!(is_path(r"\\server\share"));
        assert!(is_path(r"~\Downloads"));
        assert!(is_path(r"%APPDATA%\kwick"));
        assert!(!is_path("chrome"));
        assert!(!is_path("c:"));
        assert!(!is_path("100%"));
    }

    #[test]
    fn expands_and_splits() {
        std::env::set_var("KWICK_TEST_DIR", r"D:\Data");
        assert_eq!(expand(r"%KWICK_TEST_DIR%\x"), r"D:\Data\x");
        assert_eq!(expand("C:/Users/a"), r"C:\Users\a");
        let (dir, partial) = split(r"C:\Windows\Sys");
        assert_eq!(dir, PathBuf::from(r"C:\Windows\"));
        assert_eq!(partial, "Sys");
    }

    #[test]
    fn lists_folders_first() {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
        let items = list(Path::new(&windir));
        let first_file = items
            .iter()
            .position(|it| !Path::new(it.file_path().unwrap()).is_dir())
            .unwrap();
        assert!(items[first_file..]
            .iter()
            .all(|it| !Path::new(it.file_path().unwrap()).is_dir()));
        assert!(items.iter().any(|it| it.title.eq_ignore_ascii_case("System32")));
    }
}
