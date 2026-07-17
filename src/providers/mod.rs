pub mod apps;
pub mod pathbin;

use crate::config::Config;

#[derive(Clone)]
pub enum Action {
    /// ShellExecute a path (.lnk, .exe, folder, URL...)
    Open(String),
    Exec { cmd: String, args: Option<String> },
    Url(String),
    /// Index into the Lua host's current callback list
    Lua(usize),
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
}

impl Item {
    pub fn new(title: impl Into<String>, subtitle: impl Into<String>, action: Action) -> Self {
        let title = title.into();
        Self {
            key: title.clone(),
            title,
            subtitle: subtitle.into(),
            action,
        }
    }
}

/// Items that come from the config file (rebuilt on every show).
pub fn config_items(config: &Config) -> Vec<Item> {
    let mut items = Vec::new();
    for c in &config.commands {
        let subtitle = match &c.args {
            Some(a) => format!("{} {}", c.cmd, a),
            None => c.cmd.clone(),
        };
        let mut item = Item::new(
            c.name.clone(),
            subtitle,
            Action::Exec {
                cmd: c.cmd.clone(),
                args: c.args.clone(),
            },
        );
        if let Some(kw) = &c.keyword {
            item.key = format!("{} {}", item.title, kw);
        }
        items.push(item);
    }
    items
}

pub fn builtin_items() -> Vec<Item> {
    let cfg_dir = crate::config::config_dir().display().to_string();
    vec![
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

/// Heavy scan: start menu apps + PATH executables + builtins.
pub fn scan_indexed() -> Vec<Item> {
    let mut items = apps::scan();
    items.extend(pathbin::scan());
    items.extend(builtin_items());
    items
}
