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
    pub scan_path: bool,
    pub scan_chocolatey: bool,
    pub system_commands: bool,
    pub scan_folders: Vec<ScanFolder>,
    pub commands: Vec<CustomCommand>,
    pub web_searches: Vec<WebSearch>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: "alt+space".into(),
            max_results: 8,
            width: 640.0,
            height: 420.0,
            scan_start_menu: false,
            scan_path: false,
            scan_chocolatey: false,
            system_commands: true,
            scan_folders: Vec::new(),
            commands: Vec::new(),
            web_searches: Vec::new(),
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

fn default_scan_depth() -> usize {
    3
}

#[derive(Deserialize, Clone)]
pub struct CustomCommand {
    pub name: String,
    pub cmd: String,
    #[serde(default)]
    pub args: Option<String>,
    #[serde(default)]
    pub keyword: Option<String>,
}

#[derive(Deserialize, Clone)]
pub struct WebSearch {
    pub name: String,
    pub keyword: String,
    pub url: String,
}

pub fn config_dir() -> PathBuf {
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

/// 設定 UI で編集できるスカラー項目を config.toml に書き戻す。
///
/// toml_edit で既存の文書を書き換えるので、コメントや
/// [[scan_folders]] / [[commands]] / [[web_searches]] はそのまま残る。
/// 未記載のキーは追記されるため、新しい設定項目が増えても
/// 既存の config.toml が古いままにならない。
pub fn save(config: &Config) -> Result<(), String> {
    ensure_default_files();
    let path = config_dir().join("config.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| DEFAULT_CONFIG.to_string());
    let updated = apply_scalars(&text, config)?;
    std::fs::write(&path, updated).map_err(|e| format!("config.toml を保存できません: {e}"))
}

fn apply_scalars(text: &str, config: &Config) -> Result<String, String> {
    use toml_edit::{value, DocumentMut};

    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e| format!("config.toml を解釈できません: {e}"))?;
    doc["hotkey"] = value(config.hotkey.as_str());
    doc["max_results"] = value(config.max_results as i64);
    doc["width"] = value(config.width as f64);
    doc["height"] = value(config.height as f64);
    doc["scan_start_menu"] = value(config.scan_start_menu);
    doc["scan_path"] = value(config.scan_path);
    doc["scan_chocolatey"] = value(config.scan_chocolatey);
    doc["system_commands"] = value(config.system_commands);
    Ok(doc.to_string())
}

const DEFAULT_CONFIG: &str = r#"# Kwick 設定ファイル
# ウィンドウを表示するたびに再読み込みされます。
# 主な項目は "Kwick: Settings" の設定画面からも変更できます
# (このファイルのコメントは保持されます)。

hotkey = "alt+space"
max_results = 8

# スタートメニューのアプリ (.lnk/.url) を検索対象に含めるか。
scan_start_menu = false

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
    fn apply_scalars_keeps_comments_and_tables() {
        // An old config.toml: no scan_chocolatey, and an array of tables last.
        let original = "\
# 見出しコメント
hotkey = \"alt+space\"
max_results = 8

# PATH を含めるか
scan_path = false

[[web_searches]]
name = \"Google\"
keyword = \"g\"
url = \"https://example.com/?q={query}\"
";
        let mut config = Config::default();
        config.max_results = 12;
        config.scan_path = true;
        config.scan_chocolatey = true;

        let out = apply_scalars(original, &config).unwrap();

        assert!(out.contains("# 見出しコメント"));
        assert!(out.contains("# PATH を含めるか"));
        assert!(out.contains("max_results = 12"));
        assert!(out.contains("scan_path = true"));
        // A key absent from the old file is added to the root table, i.e. before
        // [[web_searches]] — not swallowed by it.
        let choco = out.find("scan_chocolatey = true").expect("key added");
        assert!(choco < out.find("[[web_searches]]").unwrap());
        // The array of tables survives and still parses back.
        let reparsed: Config = toml::from_str(&out).unwrap();
        assert_eq!(reparsed.web_searches.len(), 1);
        assert!(reparsed.scan_chocolatey);
        assert_eq!(reparsed.max_results, 12);
    }
}
