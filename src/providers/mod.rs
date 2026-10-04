pub mod apps;
pub mod folders;
pub mod pathbin;
pub mod registered;
pub mod shellfolders;
pub mod systools;
pub mod uwp;
pub mod winlist;

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
    /// Put text on the clipboard
    Copy(String),
    /// Run elevated (ShellExecute "runas")
    RunAs { cmd: String, args: Option<String> },
    /// Show the file selected in Explorer (shortcuts: their target)
    Reveal(String),
    /// Prompt for arguments, then run `cmd` with them
    AskArgs(String),
    /// Always rank this title first when it matches
    Pin(String),
    Unpin(String),
    /// Drop this title from the results for good
    Hide(String),
    /// Forget launch history and learned queries for this title
    Forget(String),
    /// Bring a top-level window to the front
    Focus(isize),
    CloseWindow(isize),
    /// End these processes
    Kill(Vec<u32>),
    /// Replace the query text (enter a mode such as "w ", complete a path)
    SetQuery(String),
}

/// Keyboard shortcut bound to an entry of the action panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shortcut {
    Enter,
    CtrlEnter,
    ShiftEnter,
    CtrlShiftEnter,
    CtrlC,
}

impl Shortcut {
    pub fn label(self) -> &'static str {
        match self {
            Shortcut::Enter => "Enter",
            Shortcut::CtrlEnter => "Ctrl+Enter",
            Shortcut::ShiftEnter => "Shift+Enter",
            Shortcut::CtrlShiftEnter => "Ctrl+Shift+Enter",
            Shortcut::CtrlC => "Ctrl+C",
        }
    }
}

/// One entry of an item's action panel (Ctrl+K / right click).
#[derive(Clone)]
pub struct SubAction {
    pub label: String,
    pub shortcut: Option<Shortcut>,
    pub action: Action,
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
    /// Extra entries for the action panel, after the built-in ones.
    pub extra: Vec<(String, Action)>,
    /// Launches are recorded in the history (and learned for the query).
    /// Off for one-off results such as calculator answers or web searches.
    pub remember: bool,
    /// Keyword that, typed exactly, puts this item first.
    pub alias: String,
    /// Run as soon as the alias is typed, without Enter (opt-in per command).
    pub instant: bool,
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
            extra: Vec::new(),
            remember: true,
            alias: String::new(),
            instant: false,
        }
    }

    /// A transient result (calculator answer, web search...): not recorded
    /// in the history.
    pub fn transient(mut self) -> Self {
        self.remember = false;
        self
    }

    /// The file this item launches, when it is one (for "open location",
    /// "copy path", "run as administrator").
    pub fn file_path(&self) -> Option<&str> {
        let path = match &self.action {
            Action::Open(p) => p.as_str(),
            Action::Exec { cmd, .. } => cmd.as_str(),
            _ => return None,
        };
        let p = std::path::Path::new(path);
        (p.is_absolute() && p.exists()).then_some(path)
    }

    /// The entries of this item's action panel, the default action first.
    pub fn actions(&self, pinned: bool, in_history: bool) -> Vec<SubAction> {
        let mut out = Vec::new();
        let mut add = |label: &str, shortcut: Option<Shortcut>, action: Action| {
            out.push(SubAction {
                label: label.to_string(),
                shortcut,
                action,
            });
        };
        let primary = match &self.action {
            Action::Copy(_) => "コピー",
            Action::Url(_) => "ブラウザで開く",
            Action::Focus(_) => "このウィンドウに切り替え",
            Action::Kill(_) => "プロセスを終了",
            Action::SetQuery(_) => "このモードで検索",
            _ => "開く",
        };
        add(primary, Some(Shortcut::Enter), self.action.clone());

        let file = self.file_path().map(str::to_string);
        let is_dir = file
            .as_deref()
            .is_some_and(|f| std::path::Path::new(f).is_dir());
        let launchable = match &self.action {
            Action::Exec { .. } => true,
            Action::Open(_) => file.is_some() && !is_dir,
            _ => false,
        };
        if launchable {
            let (cmd, args) = match &self.action {
                Action::Exec { cmd, args } => (cmd.clone(), args.clone()),
                Action::Open(path) => (path.clone(), None),
                _ => unreachable!(),
            };
            add(
                "管理者として実行",
                Some(Shortcut::CtrlShiftEnter),
                Action::RunAs {
                    cmd: cmd.clone(),
                    args,
                },
            );
            add("引数を指定して実行", Some(Shortcut::ShiftEnter), Action::AskArgs(cmd));
        }
        if let Some(file) = &file {
            add("ファイルの場所を開く", Some(Shortcut::CtrlEnter), Action::Reveal(file.clone()));
            add("パスをコピー", Some(Shortcut::CtrlC), Action::Copy(file.clone()));
        } else if let Action::Url(url) = &self.action {
            add("URL をコピー", Some(Shortcut::CtrlC), Action::Copy(url.clone()));
        }
        for (label, action) in &self.extra {
            add(label, None, action.clone());
        }
        if self.remember {
            if pinned {
                add("固定を解除", None, Action::Unpin(self.title.clone()));
            } else {
                add("上位に固定", None, Action::Pin(self.title.clone()));
            }
            add("候補から隠す", None, Action::Hide(self.title.clone()));
            if in_history {
                add("履歴から削除", None, Action::Forget(self.title.clone()));
            }
        }
        out
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
            item.alias = c.keyword.trim().to_string();
            item.instant = c.instant;
        }
        items.push(item);
    }
    // Entry points to the search modes, so they can be found by name.
    let p = &config.prefixes;
    for (prefix, title, aliases) in [
        (&p.windows, "ウィンドウ切り替え", "window switch switcher alt tab"),
        (&p.files, "ファイル検索 (Everything)", "file search everything find"),
        (&p.clipboard, "クリップボード履歴", "clipboard history paste"),
        (&p.kill, "プロセスを終了", "kill process taskkill task end"),
        (&p.emoji, "絵文字", "emoji 記号 symbol"),
    ] {
        if prefix.is_empty() {
            continue;
        }
        let mut item = Item::new(
            title,
            format!("「{prefix}」に続けて入力"),
            Action::SetQuery(prefix.clone()),
        );
        item.key = format!("{title} {aliases}");
        item.rank_boost = 100;
        items.push(item);
    }
    // Quick links: a [[web_searches]] entry without {query} is a fixed
    // target (URL, folder, file) opened by its name or keyword.
    for w in config
        .web_searches
        .iter()
        .filter(|w| !w.url.contains("{query}") && !w.url.trim().is_empty())
    {
        let target = w.url.trim().to_string();
        let mut item = Item::new(w.name.clone(), target.clone(), Action::Open(target.clone()));
        if !target.contains("://") {
            item.icon_path = Some(target);
        } else {
            item.icon_path = None;
        }
        if !w.keyword.trim().is_empty() {
            item.key = format!("{} {}", item.title, w.keyword);
            item.alias = w.keyword.trim().to_string();
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
    let mut existing: std::collections::HashSet<String> =
        items.iter().map(|it| it.title.to_ascii_lowercase()).collect();
    for item in extra {
        if existing.insert(item.title.to_ascii_lowercase()) {
            items.push(item);
        }
    }
}

/// App Paths often contains entries such as `notepad.exe` and `PowerShell.exe`
/// that are already represented by the curated system-tool items. Compare both
/// directions because the registered item may have a friendly name while its
/// executable name is present only in its key.
fn conflicts_with_curated_tool(item: &Item, tools: &[Item]) -> bool {
    let item_title = item.title.to_ascii_lowercase();
    let item_key = item.key.to_ascii_lowercase();
    tools.iter().any(|tool| {
        let tool_title = tool.title.to_ascii_lowercase();
        let tool_key = tool.key.to_ascii_lowercase();
        tool_key.contains(&item_title) || item_key.contains(&tool_title)
    })
}

/// Heavy scan: registered apps + start menu apps + system tools + 主要なフォルダ
/// + custom folders + Chocolatey shims + PATH executables + builtins.
pub fn scan_indexed(config: &Config) -> Vec<Item> {
    let tools = systools::scan();
    let mut items: Vec<Item> = Vec::new();
    if config.scan_start_menu {
        // Start Menu carries English shortcuts for some curated tools
        // ("Task Scheduler", "Remote Desktop Connection", ...). The curated
        // entry's key contains those English names, so drop the Start Menu
        // duplicate and show only the curated one.
        let tool_keys: Vec<String> = tools.iter().map(|t| t.key.to_lowercase()).collect();
        extend_deduped(&mut items, apps::scan().into_iter().filter(|it| {
            let title = it.title.to_lowercase();
            it.title.chars().count() < 4 || !tool_keys.iter().any(|k| k.contains(&title))
        }).collect());
    }
    let registered_apps = if config.scan_registered_apps {
        registered::scan()
            .into_iter()
            .filter(|it| !conflicts_with_curated_tool(it, &tools))
            .collect()
    } else {
        Vec::new()
    };
    // Store apps that duplicate a curated tool (Win11's packaged メモ帳,
    // 設定...) are dropped in favor of the curated entry.
    let store_apps: Vec<Item> = if config.scan_uwp_apps {
        uwp::scan()
            .into_iter()
            .filter(|it| !conflicts_with_curated_tool(it, &tools))
            .collect()
    } else {
        Vec::new()
    };
    items.extend(tools);
    extend_deduped(&mut items, registered_apps);
    extend_deduped(&mut items, store_apps);
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
    crate::reading::annotate(&mut items);
    items
}
