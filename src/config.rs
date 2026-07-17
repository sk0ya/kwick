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
            commands: Vec::new(),
            web_searches: Vec::new(),
        }
    }
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

const DEFAULT_CONFIG: &str = r#"# Kwick 設定ファイル
# ウィンドウを表示するたびに再読み込みされます。

hotkey = "alt+space"
max_results = 8

# スタートメニューのアプリ (.lnk/.url) を検索対象に含めるか。
scan_start_menu = false

# PATH 上の実行ファイル (.exe/.bat/.cmd/.com) を検索対象に含めるか。
# true にすると CLI ツールなども起動できますが、システムの exe が大量に候補に入ります。
scan_path = false

# よく使う Windows ツール(リモートデスクトップ、タスクマネージャー等)は常に検索対象です。

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
