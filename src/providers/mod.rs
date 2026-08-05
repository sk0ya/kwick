pub mod apps;
pub mod folders;
pub mod pathbin;
pub mod shellfolders;
pub mod systools;

use crate::config::Config;

#[derive(Clone)]
pub enum Action {
    /// ShellExecute a path (.lnk, .exe, folder, URL...)
    Open(String),
    Exec { cmd: String, args: Option<String> },
    Url(String),
    /// Index into the Lua host's current callback list
    Lua(usize),
    /// Show the in-window settings view
    OpenSettings,
    /// Open config.toml in the user's editor
    OpenConfig,
    Quit,
    Reload,
    RegisterStartup,
    UnregisterStartup,
}

#[derive(Clone)]
pub struct Item {
    pub title: String,
    pub subtitle: String,
    /// Text used for fuzzy matching (usually the title, plus aliases)
    pub key: String,
    pub action: Action,
    /// File whose shell icon represents this item (None = letter fallback)
    pub icon_path: Option<String>,
    /// Added to the fuzzy score so e.g. Start Menu apps outrank raw PATH exes
    pub rank_boost: u32,
}

impl Item {
    pub fn new(title: impl Into<String>, subtitle: impl Into<String>, action: Action) -> Self {
        let title = title.into();
        let icon_path = match &action {
            Action::Open(path) => Some(path.clone()),
            Action::Exec { cmd, .. } => Some(cmd.clone()),
            _ => None,
        };
        Self {
            key: title.clone(),
            title,
            subtitle: subtitle.into(),
            action,
            icon_path,
            rank_boost: 0,
        }
    }
}

/// Items that come from the config file (rebuilt on every show).
pub fn config_items(config: &Config) -> Vec<Item> {
    let mut items = Vec::new();
    for c in &config.commands {
        let args = (!c.args.trim().is_empty()).then(|| c.args.clone());
        let subtitle = match &args {
            Some(a) => format!("{} {}", c.cmd, a),
            None => c.cmd.clone(),
        };
        let mut item = Item::new(
            c.name.clone(),
            subtitle,
            Action::Exec {
                cmd: c.cmd.clone(),
                args,
            },
        );
        if !c.keyword.trim().is_empty() {
            item.key = format!("{} {}", item.title, c.keyword);
        }
        items.push(item);
    }
    items
}

pub fn builtin_items() -> Vec<Item> {
    let cfg_dir = crate::config::config_dir().display().to_string();
    let cfg_file = crate::config::config_dir()
        .join("config.toml")
        .display()
        .to_string();
    let mut settings = Item::new("Kwick: Settings", "設定画面を開く", Action::OpenSettings);
    settings.key = "Kwick: Settings config 設定".into();
    let mut edit_config = Item::new("Kwick: Edit config.toml", cfg_file, Action::OpenConfig);
    edit_config.key = "Kwick: Edit config.toml 設定ファイル".into();
    vec![
        settings,
        edit_config,
        Item::new("Kwick: Open Config Folder", cfg_dir.clone(), Action::Open(cfg_dir)),
        Item::new("Kwick: Reload Index", "アプリ一覧を再スキャン", Action::Reload),
        Item::new(
            "Kwick: Register Startup",
            "Windows ログオン時に自動起動する",
            Action::RegisterStartup,
        ),
        Item::new(
            "Kwick: Unregister Startup",
            "自動起動を解除する",
            Action::UnregisterStartup,
        ),
        Item::new("Kwick: Quit", "Kwick を終了", Action::Quit),
    ]
}

/// Append `extra`, skipping entries whose name is already covered by `items`
/// (e.g. a Start Menu app or a Chocolatey shim of the same name).
fn extend_deduped(items: &mut Vec<Item>, extra: Vec<Item>) {
    let existing: std::collections::HashSet<String> =
        items.iter().map(|it| it.title.to_ascii_lowercase()).collect();
    items.extend(
        extra
            .into_iter()
            .filter(|it| !existing.contains(&it.title.to_ascii_lowercase())),
    );
}

/// Heavy scan: start menu apps + system tools + 主要なフォルダ + custom folders
/// + Chocolatey shims + PATH executables + builtins.
pub fn scan_indexed(config: &Config) -> Vec<Item> {
    let tools = systools::scan();
    let mut items: Vec<Item> = Vec::new();
    if config.scan_start_menu {
        // Start Menu carries English shortcuts for some curated tools
        // ("Task Scheduler", "Remote Desktop Connection", ...). The curated
        // entry's key contains those English names, so drop the Start Menu
        // duplicate and show only the curated one.
        let tool_keys: Vec<String> = tools.iter().map(|t| t.key.to_lowercase()).collect();
        items.extend(apps::scan().into_iter().filter(|it| {
            let title = it.title.to_lowercase();
            it.title.chars().count() < 4 || !tool_keys.iter().any(|k| k.contains(&title))
        }));
    }
    items.extend(tools);
    if config.system_commands {
        items.extend(systools::power_items());
    }
    if config.special_folders {
        items.extend(shellfolders::scan());
    }
    items.extend(folders::scan(&config.scan_folders));
    // Chocolatey shims come first so they survive the dedupe against raw PATH exes.
    if config.scan_chocolatey {
        extend_deduped(&mut items, pathbin::scan_chocolatey());
    }
    if config.scan_path {
        extend_deduped(&mut items, pathbin::scan());
    }
    items.extend(builtin_items());
    items
}
