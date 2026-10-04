use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    pub hotkey: String,
    pub max_results: usize,
    pub width: f32,
    pub height: f32,
    pub scan_start_menu: bool,
    pub scan_registered_apps: bool,
    pub scan_uwp_apps: bool,
    pub scan_path: bool,
    pub scan_chocolatey: bool,
    pub system_commands: bool,
    pub special_folders: bool,
    pub scan_folders: Vec<ScanFolder>,
    pub commands: Vec<CustomCommand>,
    pub web_searches: Vec<WebSearch>,
    /// Titles always ranked first when they match (action panel: 上位に固定).
    pub pinned: Vec<String>,
    /// Titles never shown (action panel: 候補から隠す).
    pub hidden: Vec<String>,
    pub prefixes: Prefixes,
    /// Remember copied text (memory only) for the clipboard mode.
    pub clipboard_history: bool,
    pub clipboard_history_size: usize,
}

/// What to type first to enter each search mode. An empty string turns the
/// mode off.
#[derive(Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Prefixes {
    pub windows: String,
    pub kill: String,
    pub clipboard: String,
    pub files: String,
    pub emoji: String,
}

impl Default for Prefixes {
    fn default() -> Self {
        Self {
            windows: "w ".into(),
            kill: "kill ".into(),
            clipboard: "cb ".into(),
            files: "f ".into(),
            emoji: ":".into(),
        }
    }
}

impl Prefixes {
    /// (key in config.toml, label, value) for each mode, in display order.
    pub fn entries_mut(&mut self) -> [(&'static str, &'static str, &mut String); 5] {
        [
            ("windows", "ウィンドウ切り替え", &mut self.windows),
            ("files", "ファイル検索 (Everything)", &mut self.files),
            ("clipboard", "クリップボード履歴", &mut self.clipboard),
            ("kill", "プロセスを終了", &mut self.kill),
            ("emoji", "絵文字", &mut self.emoji),
        ]
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: "alt+space".into(),
            max_results: 8,
            width: 640.0,
            height: 420.0,
            scan_start_menu: false,
            scan_registered_apps: true,
            scan_uwp_apps: true,
            scan_path: false,
            scan_chocolatey: false,
            system_commands: true,
            special_folders: true,
            scan_folders: Vec::new(),
            commands: Vec::new(),
            web_searches: Vec::new(),
            pinned: Vec::new(),
            hidden: Vec::new(),
            prefixes: Prefixes::default(),
            clipboard_history: true,
            clipboard_history_size: 50,
        }
    }
}

#[derive(Deserialize, Clone, PartialEq)]
pub struct ScanFolder {
    pub path: String,
    /// 何階層まで潜るか(1 = 直下のみ)。省略時は 3。
    #[serde(default = "default_scan_depth")]
    pub depth: usize,
    /// 対象拡張子(小文字・ドットなし)。省略時は exe/lnk/bat/cmd/url。
    #[serde(default)]
    pub extensions: Option<Vec<String>>,
}

impl Default for ScanFolder {
    fn default() -> Self {
        Self {
            path: String::new(),
            depth: default_scan_depth(),
            extensions: None,
        }
    }
}

fn default_scan_depth() -> usize {
    3
}

/// 省略可能な項目は空文字を「未設定」として扱う(設定 UI から
/// そのまま編集できるように Option ではなく String で持つ)。
#[derive(Deserialize, Clone, Default)]
pub struct CustomCommand {
    pub name: String,
    pub cmd: String,
    #[serde(default)]
    pub args: String,
    #[serde(default)]
    pub keyword: String,
    /// Run as soon as `keyword` is typed exactly, without Enter.
    #[serde(default)]
    pub instant: bool,
}

/// A web search ("keyword query", url with {query}) or, without {query}, a
/// quick link: a fixed URL/folder/file opened by name or keyword.
#[derive(Deserialize, Clone, Default)]
pub struct WebSearch {
    pub name: String,
    #[serde(default)]
    pub keyword: String,
    pub url: String,
}

/// `KWICK_CONFIG_DIR` overrides the location, so a development build can run
/// next to the resident instance with its own config and history.
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("KWICK_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("kwick")
}

pub fn plugin_dir() -> PathBuf {
    config_dir().join("plugins")
}

pub fn load() -> Config {
    ensure_default_files();
    let path = config_dir().join("config.toml");
    match std::fs::read_to_string(&path) {
        Ok(text) => match toml::from_str(&text) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("kwick: config.toml parse error: {e}");
                Config::default()
            }
        },
        Err(_) => Config::default(),
    }
}

/// 設定 UI での変更を config.toml に書き戻す。
///
/// toml_edit で既存の文書を書き換えるので、キーに付いたコメントや並び順は
/// 残る。未記載のキーは追記されるため、設定項目が増えても既存の
/// config.toml が古いままにならない。
/// [[scan_folders]] / [[commands]] / [[web_searches]] は UI の内容で作り直す
/// (見出しコメントは引き継ぐが、表の内側に書かれたコメントは失われる)。
pub fn save(config: &Config) -> Result<(), String> {
    ensure_default_files();
    let path = config_dir().join("config.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| DEFAULT_CONFIG.to_string());
    let updated = apply(&text, config)?;
    std::fs::write(&path, updated).map_err(|e| format!("config.toml を保存できません: {e}"))
}

fn apply(text: &str, config: &Config) -> Result<String, String> {
    use toml_edit::{value, Array, ArrayOfTables, DocumentMut, Table};

    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e| format!("config.toml を解釈できません: {e}"))?;
    doc["hotkey"] = value(config.hotkey.as_str());
    doc["max_results"] = value(config.max_results as i64);
    doc["width"] = value(config.width as f64);
    doc["height"] = value(config.height as f64);
    doc["scan_start_menu"] = value(config.scan_start_menu);
    doc["scan_registered_apps"] = value(config.scan_registered_apps);
    doc["scan_uwp_apps"] = value(config.scan_uwp_apps);
    doc["scan_path"] = value(config.scan_path);
    doc["scan_chocolatey"] = value(config.scan_chocolatey);
    doc["system_commands"] = value(config.system_commands);
    doc["special_folders"] = value(config.special_folders);
    doc["clipboard_history"] = value(config.clipboard_history);
    doc["clipboard_history_size"] = value(config.clipboard_history_size as i64);
    for (key, titles) in [("pinned", &config.pinned), ("hidden", &config.hidden)] {
        if titles.is_empty() {
            doc.remove(key);
        } else {
            doc[key] = value(titles.iter().map(String::as_str).collect::<Array>());
        }
    }
    {
        let mut prefixes = config.prefixes.clone();
        if !doc.contains_table("prefixes") {
            doc["prefixes"] = toml_edit::table();
        }
        for (key, _, text) in prefixes.entries_mut() {
            doc["prefixes"][key] = value(text.as_str());
        }
    }

    let mut folders = ArrayOfTables::new();
    for folder in config
        .scan_folders
        .iter()
        .filter(|f| !f.path.trim().is_empty())
    {
        let mut table = Table::new();
        table["path"] = value(folder.path.trim());
        table["depth"] = value(folder.depth as i64);
        if let Some(extensions) = &folder.extensions {
            let mut array = Array::new();
            for ext in extensions {
                array.push(ext.as_str());
            }
            table["extensions"] = value(array);
        }
        folders.push(table);
    }
    set_tables(&mut doc, "scan_folders", folders);

    let mut commands = ArrayOfTables::new();
    for command in config.commands.iter().filter(|c| !c.name.trim().is_empty()) {
        let mut table = Table::new();
        table["name"] = value(command.name.trim());
        table["cmd"] = value(command.cmd.trim());
        for (key, text) in [("args", &command.args), ("keyword", &command.keyword)] {
            if !text.trim().is_empty() {
                table[key] = value(text.trim());
            }
        }
        if command.instant {
            table["instant"] = value(true);
        }
        commands.push(table);
    }
    set_tables(&mut doc, "commands", commands);

    let mut searches = ArrayOfTables::new();
    for search in config.web_searches.iter().filter(|w| {
        !w.url.trim().is_empty() && !(w.keyword.trim().is_empty() && w.name.trim().is_empty())
    }) {
        let mut table = Table::new();
        table["name"] = value(search.name.trim());
        if !search.keyword.trim().is_empty() {
            table["keyword"] = value(search.keyword.trim());
        }
        table["url"] = value(search.url.trim());
        searches.push(table);
    }
    set_tables(&mut doc, "web_searches", searches);

    Ok(doc.to_string())
}

/// Replace an array of tables, carrying over the comment that introduced it.
fn set_tables(doc: &mut toml_edit::DocumentMut, key: &str, mut tables: toml_edit::ArrayOfTables) {
    let heading = doc
        .get(key)
        .and_then(|item| item.as_array_of_tables())
        .and_then(|old| old.get(0))
        .and_then(|first| first.decor().prefix().cloned());
    if tables.is_empty() {
        doc.remove(key);
        return;
    }
    if let (Some(heading), Some(first)) = (heading, tables.get_mut(0)) {
        first.decor_mut().set_prefix(heading);
    }
    doc[key] = toml_edit::Item::ArrayOfTables(tables);
}

const DEFAULT_CONFIG: &str = r#"# Kwick 設定ファイル
# ウィンドウを表示するたびに再読み込みされます。
# すべての項目は "Kwick: Settings" の設定画面からも編集できます
# (キーに付けたコメントは保持されますが、[[...]] の表の内側に書いた
#  コメントは設定画面から保存すると失われます)。

hotkey = "alt+space"
max_results = 8

# スタートメニューのアプリ (.lnk/.url) を検索対象に含めるか。
scan_start_menu = false

# Windows の App Paths に登録されたアプリを検索対象に含めるか。
# PATH と違い、起動用に登録された GUI アプリ中心なので候補が増えすぎません。
scan_registered_apps = true

# Microsoft Store (UWP) アプリ(電卓、Windows Terminal など)を検索対象に含めるか。
scan_uwp_apps = true

# PATH 上の実行ファイル (.exe/.bat/.cmd/.com) を検索対象に含めるか。
# true にすると CLI ツールなども起動できますが、システムの exe が大量に候補に入ります。
scan_path = false

# Chocolatey の shim フォルダ (%ChocolateyInstall%\bin、既定では
# C:\ProgramData\chocolatey\bin) を検索対象に含めるか。
# scan_path とは独立しているので、PATH 全体を取り込まずに choco で入れた
# ツールだけを候補に追加できます。
scan_chocolatey = false

# よく使う Windows ツール(リモートデスクトップ、タスクマネージャー等)は常に検索対象です。

# 電源系コマンド(シャットダウン、再起動、スリープ、休止状態、サインアウト、ロック)を
# 検索対象に含めるか。
system_commands = true

# 主要なフォルダ(ダウンロード、デスクトップ、AppData、Temp、Program Files など)を
# 検索対象に含めるか。false にすると候補から消えます。
special_folders = true

# コピーしたテキストを覚えておき、「cb 」で検索して貼り付けられるようにするか。
# 履歴はメモリ上にだけ保持し、ディスクには書きません。パスワードマネージャーが
# 「履歴に残さない」印を付けたコピーは記録しません。
clipboard_history = true
clipboard_history_size = 50

# 検索モードに入るためのプレフィックス。空文字 "" にするとそのモードを無効にします。
[prefixes]
windows = "w "      # 開いているウィンドウに切り替え
files = "f "        # ファイル検索 (Everything が必要)
clipboard = "cb "   # クリップボード履歴
kill = "kill "      # プロセスを終了
emoji = ":"         # 絵文字

# --- スキャンフォルダ(任意のフォルダを検索対象に追加) ---
# path 直下から depth 階層まで走査し、extensions の拡張子を候補に追加します。
# [[scan_folders]]
# path = 'D:\Tools'
# depth = 3                              # 省略時は 3(1 = 直下のみ)
# extensions = ["exe", "lnk", "bat"]     # 省略時は exe/lnk/bat/cmd/url

# --- カスタムコマンド(コード不要) ---
# [[commands]]
# name = "Shutdown PC"
# cmd = "shutdown"
# args = "/s /t 0"
# keyword = "sd"

# --- Web 検索("キーワード + スペース + 検索語" で起動) ---
[[web_searches]]
name = "Google"
keyword = "g"
url = "https://www.google.com/search?q={query}"

[[web_searches]]
name = "YouTube"
keyword = "yt"
url = "https://www.youtube.com/results?search_query={query}"
"#;

const CALC_PLUGIN: &str = r#"-- 電卓プラグイン(サンプル)
-- 数式を入力すると結果を表示し、Enter でクリップボードにコピーします。
kwick.register{
    name = "calc",
    on_query = function(q)
        local expr = q:match("^=%s*(.+)$")
        if not expr and q:match("^[%d%.%s%+%-%*/%%%(%)%^]+$") and q:match("[%+%-%*/%%%^]") then
            expr = q
        end
        if not expr then return {} end
        local f = load("return " .. expr, "calc", "t", { math = math })
        if not f then return {} end
        local ok, result = pcall(f)
        if not ok or type(result) ~= "number" then return {} end
        return {
            {
                title = tostring(result),
                subtitle = expr .. " =  (Enter でコピー)",
                run = function() kwick.copy(tostring(result)) end,
            },
        }
    end,
}
"#;

fn ensure_default_files() {
    let dir = config_dir();
    let plugins = plugin_dir();
    let _ = std::fs::create_dir_all(&plugins);
    let cfg = dir.join("config.toml");
    if !cfg.exists() {
        let _ = std::fs::write(&cfg, DEFAULT_CONFIG);
    }
    let calc = plugins.join("calc.lua");
    if !calc.exists() {
        let _ = std::fs::write(&calc, CALC_PLUGIN);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_keeps_comments_and_rewrites_tables() {
        // An old config.toml: no scan_chocolatey, and an array of tables last.
        let original = "\
# 見出しコメント
hotkey = \"alt+space\"
max_results = 8

# PATH を含めるか
scan_path = false

# Web 検索
[[web_searches]]
name = \"Google\"
keyword = \"g\"
url = \"https://example.com/?q={query}\"
";
        let mut config: Config = toml::from_str(original).unwrap();
        config.max_results = 12;
        config.scan_path = true;
        config.scan_chocolatey = true;
        config.web_searches.push(WebSearch {
            name: "YouTube".into(),
            keyword: "yt".into(),
            url: "https://example.net/?s={query}".into(),
        });
        config.scan_folders.push(ScanFolder {
            path: r"D:\Tools".into(),
            depth: 2,
            extensions: Some(vec!["exe".into()]),
        });
        // Blank rows are what an untouched "追加" leaves behind; drop them.
        config.commands.push(CustomCommand::default());

        let out = apply(original, &config).unwrap();

        assert!(out.contains("# 見出しコメント"));
        assert!(out.contains("# PATH を含めるか"));
        assert!(out.contains("max_results = 12"));
        assert!(out.contains("scan_path = true"));
        // A key absent from the old file is added to the root table, i.e. before
        // [[web_searches]] — not swallowed by it.
        let choco = out.find("scan_chocolatey = true").expect("key added");
        assert!(choco < out.find("[[web_searches]]").unwrap());
        // The comment introducing the array of tables is carried over.
        assert!(out.contains("# Web 検索\n[[web_searches]]"));
        assert!(!out.contains("[[commands]]"));

        let reparsed: Config = toml::from_str(&out).unwrap();
        assert!(reparsed.scan_chocolatey);
        assert_eq!(reparsed.max_results, 12);
        assert_eq!(reparsed.web_searches.len(), 2);
        assert_eq!(reparsed.web_searches[1].keyword, "yt");
        assert_eq!(reparsed.scan_folders.len(), 1);
        assert_eq!(reparsed.scan_folders[0].depth, 2);
        assert_eq!(
            reparsed.scan_folders[0].extensions.as_deref(),
            Some(["exe".to_string()].as_slice())
        );
    }
}
