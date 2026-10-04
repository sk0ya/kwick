//! Details of the selected result for the preview pane (Ctrl+P): file
//! facts, the head of a text file, a thumbnail for pictures, the full text
//! of a clipboard entry.

use crate::providers::{Action, Item};
use std::io::Read;
use std::path::Path;

pub struct Preview {
    /// Which result this describes (title + action target).
    pub key: String,
    pub title: String,
    pub facts: Vec<(&'static str, String)>,
    pub text: Option<String>,
    /// File to show a shell thumbnail of.
    pub thumbnail: Option<String>,
}

const TEXT_EXTS: &[&str] = &[
    "txt", "md", "log", "csv", "tsv", "json", "toml", "yaml", "yml", "xml", "ini", "cfg", "conf",
    "rs", "py", "js", "ts", "tsx", "jsx", "c", "h", "cpp", "hpp", "cs", "go", "java", "kt", "rb",
    "php", "lua", "sh", "ps1", "bat", "cmd", "html", "htm", "css", "scss", "sql", "gitignore",
];
const THUMB_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "bmp", "webp", "heic", "tif", "tiff", "ico", "svg", "mp4", "mov",
    "mkv", "avi", "wmv", "pdf", "psd",
];
const TEXT_BYTES: usize = 16 * 1024;
const TEXT_LINES: usize = 60;

pub fn key(item: &Item) -> String {
    let target = match &item.action {
        Action::Open(p) | Action::Url(p) | Action::Copy(p) | Action::Paste(p) => p.clone(),
        Action::Exec { cmd, args } => format!("{cmd} {}", args.as_deref().unwrap_or("")),
        Action::Focus(h) => h.to_string(),
        _ => String::new(),
    };
    format!("{}\u{0}{target}", item.title)
}

pub fn build(item: &Item) -> Preview {
    let mut p = Preview {
        key: key(item),
        title: item.title.clone(),
        facts: Vec::new(),
        text: None,
        thumbnail: None,
    };
    match &item.action {
        Action::Paste(text) | Action::Copy(text) if !text.is_empty() => {
            p.facts.push(("文字数", text.chars().count().to_string()));
            p.text = Some(text.chars().take(4000).collect());
            return p;
        }
        Action::Url(url) => {
            p.facts.push(("URL", url.clone()));
            return p;
        }
        _ => {}
    }
    let Some(path) = item.file_path() else {
        if !item.subtitle.is_empty() {
            p.facts.push(("説明", item.subtitle.clone()));
        }
        if let Action::Exec { cmd, args } = &item.action {
            p.facts.push(("コマンド", cmd.clone()));
            if let Some(args) = args {
                p.facts.push(("引数", args.clone()));
            }
        }
        return p;
    };
    let path = Path::new(path);
    p.facts.push(("パス", path.display().to_string()));
    let Ok(meta) = std::fs::metadata(path) else {
        return p;
    };
    if meta.is_dir() {
        let count = std::fs::read_dir(path).map(|d| d.take(10_000).count()).unwrap_or(0);
        p.facts.push(("中身", format!("{count} 項目")));
    } else {
        p.facts.push(("サイズ", crate::providers::pathnav::format_size(meta.len())));
    }
    if let Ok(modified) = meta.modified() {
        p.facts.push(("更新日時", crate::providers::convert::format_system_time(modified)));
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if meta.is_file() && TEXT_EXTS.contains(&ext.as_str()) {
        p.text = read_head(path);
    } else if meta.is_file() && THUMB_EXTS.contains(&ext.as_str()) {
        p.thumbnail = Some(path.display().to_string());
    }
    p
}

/// The first lines of a text file (bounded read; not UTF-8 → lossy).
fn read_head(path: &Path) -> Option<String> {
    let mut buf = Vec::with_capacity(TEXT_BYTES);
    std::fs::File::open(path)
        .ok()?
        .take(TEXT_BYTES as u64)
        .read_to_end(&mut buf)
        .ok()?;
    if buf.contains(&0) {
        return None; // binary after all
    }
    let text = String::from_utf8_lossy(&buf);
    Some(text.lines().take(TEXT_LINES).collect::<Vec<_>>().join("\n"))
}
