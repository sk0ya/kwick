use crate::config::{self, Config};
use crate::history::History;
use crate::hotkey::HotkeyInput;
use crate::icons::IconCache;
use crate::launch::{self, shell_open};
use crate::lua_host::LuaHost;
use crate::matcher::Ranker;
use crate::providers::{self, Action, Item, Shortcut, SubAction};
use crate::tray::TrayFlags;
use crate::winctl::WindowCtl;
use eframe::egui;
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::SystemTime;

#[derive(PartialEq)]
enum View {
    Search,
    Settings,
}

/// The settings view needs far more room than the palette, so it gets its own
/// window size; `config.width`/`height` stay the size of the search palette.
const SETTINGS_SIZE: egui::Vec2 = egui::vec2(760.0, 720.0);

pub struct KwickApp {
    config: Config,
    /// Indexed items: start menu apps, PATH executables, builtins, custom commands.
    indexed: Vec<Item>,
    /// How many of `indexed` (from the end) come from the config file.
    config_item_count: usize,
    lua: LuaHost,
    ranker: Ranker,

    query: String,
    results: Vec<Item>,
    selected: usize,
    needs_search: bool,
    /// Action panel (Ctrl+K) for the selected result.
    panel: Option<Panel>,
    /// Shift+Enter: the query box is collecting arguments for a command.
    args_prompt: Option<ArgsPrompt>,
    /// Result to run right away: its `instant` keyword was typed.
    instant_pending: Option<usize>,
    /// The query was replaced programmatically; put the cursor at its end.
    cursor_to_end: bool,
    clip_history: crate::clipboard::ClipHistory,
    currency: providers::convert::Currency,
    /// Exchange rates were being downloaded at the last search.
    rates_pending: bool,

    view: View,
    /// Hotkey text being edited in the settings view (applied on lost focus).
    hotkey_draft: String,
    /// Comma-separated extension lists being typed, one per scan folder.
    ext_drafts: Vec<String>,
    /// Cached HKCU Run state so the settings checkbox doesn't hit the registry
    /// every frame.
    startup_enabled: bool,
    settings_status: Option<String>,
    /// Edits not yet written to config.toml (see `flush_settings`).
    settings_dirty: bool,
    settings_rescan: bool,

    /// A scan runs away from the egui thread. The old index remains usable
    /// until the worker returns a complete replacement.
    scan_rx: Option<Receiver<Vec<Item>>>,
    scan_again: bool,
    /// Config/plugin files are hot-reloaded only when their stamps change.
    reload_stamp: ReloadStamp,
    egui_ctx: egui::Context,
    window_size: Option<egui::Vec2>,

    icons: IconCache,
    ctl: Arc<WindowCtl>,
    /// Visibility as of the previous frame, to detect "just shown".
    last_visible: bool,
    /// True until the first frame has been through `update`.
    first_frame: bool,
    history: History,
    had_focus: bool,
    /// IME composition (未確定文字列) in progress: Enter/Esc/arrows belong to the IME.
    ime_composing: bool,
    hotkey_notice: Option<String>,
    /// Currently registered hotkey, kept so it can be unregistered on change.
    active_hotkey: Option<Binding>,
    tray_flags: Arc<TrayFlags>,
    _tray: Option<tray_icon::TrayIcon>,
    hotkey_manager: GlobalHotKeyManager,
    hotkey_input: HotkeyInput,
}

struct Panel {
    item: Item,
    actions: Vec<SubAction>,
    selected: usize,
}

struct ArgsPrompt {
    cmd: String,
    item: Item,
    /// The search query, restored when the prompt ends.
    saved_query: String,
}

const QUERY_ID: &str = "kwick-query";

/// A hotkey that is registered with the OS right now.
struct Binding {
    hotkey: HotKey,
    spec: String,
}

#[derive(Clone, PartialEq, Eq)]
struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
}

#[derive(Clone, PartialEq, Eq)]
struct ReloadStamp {
    config: FileStamp,
    plugins: Vec<(String, FileStamp)>,
}

fn file_stamp(path: &Path) -> FileStamp {
    let metadata = std::fs::metadata(path).ok();
    FileStamp {
        modified: metadata.as_ref().and_then(|m| m.modified().ok()),
        len: metadata.map(|m| m.len()).unwrap_or(0),
    }
}

fn reload_stamp() -> ReloadStamp {
    let config_path = config::config_dir().join("config.toml");
    let mut plugins = Vec::new();
    if let Ok(entries) = std::fs::read_dir(config::plugin_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("lua") {
                continue;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            plugins.push((name, file_stamp(&path)));
        }
    }
    plugins.sort_by(|a, b| a.0.cmp(&b.0));
    ReloadStamp {
        config: file_stamp(&config_path),
        plugins,
    }
}

/// The settings that change what the background scan produces.
fn scan_key(config: &Config) -> (Vec<bool>, Vec<config::ScanFolder>) {
    (
        vec![
            config.scan_start_menu,
            config.scan_registered_apps,
            config.scan_uwp_apps,
            config.scan_path,
            config.scan_chocolatey,
            config.system_commands,
            config.special_folders,
        ],
        config.scan_folders.clone(),
    )
}

/// Try the configured hotkey, then fallbacks (the configured one is often
/// taken by another launcher). Returns (active binding, user-facing notice).
fn register_hotkey(
    manager: &GlobalHotKeyManager,
    configured: &str,
) -> (Option<Binding>, Option<String>) {
    let fallbacks = ["ctrl+alt+space", "ctrl+shift+space", "ctrl+alt+k"];
    let candidates = std::iter::once(configured).chain(fallbacks.into_iter());
    let mut first_error: Option<String> = None;
    for (i, spec) in candidates.enumerate() {
        let hotkey: HotKey = match spec.parse() {
            Ok(h) => h,
            Err(e) => {
                if i == 0 {
                    first_error = Some(format!("'{spec}' を解釈できません: {e}"));
                }
                continue;
            }
        };
        match manager.register(hotkey) {
            Ok(()) => {
                let notice = if i == 0 {
                    None
                } else {
                    Some(format!(
                        "'{configured}' は使用できないため、ホットキーは '{spec}' になっています"
                    ))
                };
                return (
                    Some(Binding {
                        hotkey,
                        spec: spec.to_string(),
                    }),
                    notice,
                );
            }
            Err(e) => {
                // Space bindings also have an independent keyboard hook and
                // key-state monitor. Keep the requested binding on conflicts.
                if i == 0 && hotkey.key == global_hotkey::hotkey::Code::Space {
                    return (
                        Some(Binding {
                            hotkey,
                            spec: spec.to_string(),
                        }),
                        Some(format!(
                            "'{spec}' のOS登録に失敗したため、直接キー監視で起動します ({e})"
                        )),
                    );
                }
                if i == 0 {
                    first_error = Some(format!("'{spec}' は他のアプリが使用中です ({e})"));
                }
            }
        }
    }
    let err = first_error.unwrap_or_default();
    (
        None,
        Some(format!(
            "ホットキーを登録できませんでした: {err}。タスクトレイのアイコンから開けます"
        )),
    )
}

/// Rounded square with the item's first letter, for items without a file icon.
fn fallback_icon(ui: &mut egui::Ui, title: &str, size: egui::Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let initial = title
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".into());
    let painter = ui.painter();
    painter.rect_filled(rect, 6.0, ui.visuals().faint_bg_color);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initial,
        egui::FontId::proportional(15.0),
        ui.visuals().strong_text_color(),
    );
}

/// Settings-view section heading.
fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(egui::RichText::new(title).strong());
    ui.add_space(2.0);
}

/// Dimmed explanatory line, indented to sit under a checkbox's label.
fn hint(ui: &mut egui::Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(20.0);
        note(ui, text);
    });
}

/// Dimmed explanatory line at the current indent.
fn note(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).weak().size(11.0));
}

fn size_drag(value: &mut f32) -> egui::DragValue<'_> {
    egui::DragValue::new(value).speed(4.0).range(240.0..=1600.0)
}

/// What the settings view changed while it was being built.
///
/// Saving is deferred until a widget is done being interacted with, so typing
/// in a text field doesn't rewrite config.toml on every keystroke.
#[derive(Default)]
struct Edits {
    /// Something changed and still has to reach config.toml.
    dirty: bool,
    /// Save now.
    flush: bool,
    /// The change affects what gets indexed.
    rescan: bool,
}

impl Edits {
    /// A widget that settles when you leave it: text field, drag value.
    fn field(&mut self, response: &egui::Response, rescan: bool) {
        if response.changed() {
            self.dirty = true;
            self.rescan |= rescan;
        }
        if response.lost_focus() || response.drag_stopped() {
            self.flush = true;
        }
    }

    /// A widget whose single click is the entire change: checkbox, button.
    fn toggle(&mut self, response: &egui::Response, rescan: bool) {
        if response.changed() {
            self.mark(rescan);
        }
    }

    fn mark(&mut self, rescan: bool) {
        self.dirty = true;
        self.flush = true;
        self.rescan |= rescan;
    }
}

/// Header for an editable list, with the button that appends a row.
fn list_section(ui: &mut egui::Ui, title: &str, description: &str, add: &str) -> bool {
    section(ui, title);
    note(ui, description);
    ui.add_space(2.0);
    let mut added = false;
    ui.horizontal(|ui| {
        added = ui.button(add).clicked();
    });
    added
}

/// One row of an editable list; returns true when its 削除 button was pressed.
fn list_row(ui: &mut egui::Ui, index: usize, contents: impl FnOnce(&mut egui::Ui)) -> bool {
    let mut remove = false;
    ui.push_id(index, |ui| {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| contents(ui));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                    remove = ui.button("削除").clicked();
                });
            });
        });
    });
    remove
}

fn labeled(
    ui: &mut egui::Ui,
    label: &str,
    width: f32,
    text: &mut String,
    hint: &str,
) -> egui::Response {
    let mut response = None;
    ui.horizontal(|ui| {
        ui.label(label);
        response = Some(
            ui.add(
                egui::TextEdit::singleline(text)
                    .desired_width(width)
                    .hint_text(hint),
            ),
        );
    });
    response.expect("horizontal ran")
}

fn scan_folders_ui(
    ui: &mut egui::Ui,
    config: &mut Config,
    ext_drafts: &mut Vec<String>,
    edits: &mut Edits,
) {
    if list_section(
        ui,
        "スキャンフォルダ",
        "指定したフォルダの中身を検索対象に追加します",
        "+ フォルダを追加",
    ) {
        config.scan_folders.push(config::ScanFolder::default());
        ext_drafts.push(String::new());
        edits.mark(true);
    }
    // The drafts hold the comma-separated text being typed; reparsing the real
    // Vec<String> on every frame would fight with the cursor.
    ext_drafts.resize(config.scan_folders.len(), String::new());

    let mut remove = None;
    for (i, folder) in config.scan_folders.iter_mut().enumerate() {
        let draft = &mut ext_drafts[i];
        if list_row(ui, i, |ui| {
            edits.field(
                &labeled(ui, "パス", 300.0, &mut folder.path, r"D:\Tools"),
                true,
            );
            ui.horizontal(|ui| {
                ui.label("階層");
                edits.field(
                    &ui.add(egui::DragValue::new(&mut folder.depth).range(1..=8)),
                    true,
                );
                ui.add_space(8.0);
                ui.label("拡張子");
                let response = ui.add(
                    egui::TextEdit::singleline(draft)
                        .desired_width(190.0)
                        .hint_text("exe, lnk, bat, cmd, url"),
                );
                if response.lost_focus() {
                    let parsed: Vec<String> = draft
                        .split(',')
                        .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
                        .filter(|e| !e.is_empty())
                        .collect();
                    folder.extensions = (!parsed.is_empty()).then_some(parsed);
                    *draft = folder
                        .extensions
                        .as_ref()
                        .map(|e| e.join(", "))
                        .unwrap_or_default();
                }
                edits.field(&response, true);
            });
        }) {
            remove = Some(i);
        }
    }
    if let Some(i) = remove {
        config.scan_folders.remove(i);
        ext_drafts.remove(i);
        edits.mark(true);
    }
}

fn commands_ui(ui: &mut egui::Ui, config: &mut Config, edits: &mut Edits) {
    if list_section(
        ui,
        "カスタムコマンド",
        "任意のコマンドを候補に追加します (キーワードでも検索できます)",
        "+ コマンドを追加",
    ) {
        config.commands.push(config::CustomCommand::default());
        edits.mark(false);
    }

    let mut remove = None;
    for (i, command) in config.commands.iter_mut().enumerate() {
        if list_row(ui, i, |ui| {
            edits.field(
                &labeled(ui, "名前", 240.0, &mut command.name, "Shutdown PC"),
                false,
            );
            edits.field(
                &labeled(ui, "コマンド", 240.0, &mut command.cmd, "shutdown"),
                false,
            );
            ui.horizontal(|ui| {
                ui.label("引数");
                edits.field(
                    &ui.add(
                        egui::TextEdit::singleline(&mut command.args)
                            .desired_width(150.0)
                            .hint_text("/s /t 0"),
                    ),
                    false,
                );
                ui.add_space(8.0);
                ui.label("キーワード");
                edits.field(
                    &ui.add(
                        egui::TextEdit::singleline(&mut command.keyword)
                            .desired_width(80.0)
                            .hint_text("sd"),
                    ),
                    false,
                );
                ui.add_space(8.0);
                edits.toggle(
                    &ui.checkbox(&mut command.instant, "即実行")
                        .on_hover_text("キーワードを打ち終えた時点で Enter なしで実行します"),
                    false,
                );
            });
        }) {
            remove = Some(i);
        }
    }
    if let Some(i) = remove {
        config.commands.remove(i);
        edits.mark(false);
    }
}

fn web_searches_ui(ui: &mut egui::Ui, config: &mut Config, edits: &mut Edits) {
    if list_section(
        ui,
        "Web 検索 / クイックリンク",
        "「キーワード + スペース + 検索語」で検索します。URL の {query} が検索語に置き換わります。\
         {query} を含まない URL・フォルダ・ファイルは名前かキーワードで開くリンクになります",
        "+ 検索を追加",
    ) {
        config.web_searches.push(config::WebSearch::default());
        edits.mark(false);
    }

    let mut remove = None;
    for (i, search) in config.web_searches.iter_mut().enumerate() {
        if list_row(ui, i, |ui| {
            ui.horizontal(|ui| {
                ui.label("名前");
                edits.field(
                    &ui.add(
                        egui::TextEdit::singleline(&mut search.name)
                            .desired_width(150.0)
                            .hint_text("Google"),
                    ),
                    false,
                );
                ui.add_space(8.0);
                ui.label("キーワード");
                edits.field(
                    &ui.add(
                        egui::TextEdit::singleline(&mut search.keyword)
                            .desired_width(60.0)
                            .hint_text("g"),
                    ),
                    false,
                );
            });
            edits.field(
                &labeled(
                    ui,
                    "URL",
                    330.0,
                    &mut search.url,
                    "https://www.google.com/search?q={query}",
                ),
                false,
            );
        }) {
            remove = Some(i);
        }
    }
    if let Some(i) = remove {
        config.web_searches.remove(i);
        edits.mark(false);
    }
}

fn win32_hwnd(cc: &eframe::CreationContext<'_>) -> isize {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match cc.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
        _ => {
            eprintln!("kwick: could not get Win32 window handle");
            0
        }
    }
}

impl KwickApp {
    pub fn new(cc: &eframe::CreationContext<'_>, start_visible: bool) -> Self {
        crate::fonts::install_japanese_fallback(&cc.egui_ctx);

        // Keep the launcher easy on the eyes regardless of the OS theme.
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = egui::Color32::from_rgb(10, 10, 12);
        visuals.window_fill = egui::Color32::from_rgb(10, 10, 12);
        visuals.extreme_bg_color = egui::Color32::from_rgb(4, 4, 6);
        visuals.faint_bg_color = egui::Color32::from_rgb(28, 28, 32);
        visuals.selection.bg_fill = egui::Color32::from_rgb(45, 72, 110);
        visuals.selection.stroke.color = egui::Color32::from_rgb(225, 235, 255);
        cc.egui_ctx.set_visuals(visuals);

        let config = config::load();

        let hotkey_manager = GlobalHotKeyManager::new().expect("failed to init global hotkey");
        let (active_hotkey, hotkey_notice) = register_hotkey(&hotkey_manager, &config.hotkey);
        if let Some(notice) = &hotkey_notice {
            eprintln!("kwick: {notice}");
        }

        let ctl = Arc::new(WindowCtl::new(win32_hwnd(cc), start_visible));
        crate::instance::listen_show(ctl.clone(), cc.egui_ctx.clone());
        let hotkey_input = HotkeyInput::new(
            cc.egui_ctx.clone(),
            ctl.clone(),
            active_hotkey.as_ref().map(|binding| binding.hotkey),
        );

        // Show straight from the hotkey thread: while the window is hidden
        // the event loop gets no paint events, so this cannot go through the
        // app's update().
        {
            let hotkey_input = hotkey_input.clone();
            std::thread::spawn(move || {
                let receiver = GlobalHotKeyEvent::receiver();
                while let Ok(event) = receiver.recv() {
                    if event.state == HotKeyState::Pressed {
                        hotkey_input.on_registered_event(event.id);
                    }
                }
            });
        }

        let tooltip = match &active_hotkey {
            Some(b) => format!("Kwick ({})", b.spec),
            None => "Kwick".to_string(),
        };
        let (tray, tray_flags) = crate::tray::init(cc.egui_ctx.clone(), &tooltip, ctl.clone());

        let mut indexed = providers::scan_indexed(&config);
        let config_items = providers::config_items(&config);
        let config_item_count = config_items.len();
        indexed.extend(config_items);

        let lua = LuaHost::new(&config::plugin_dir(), cc.egui_ctx.clone());
        let reload_stamp = reload_stamp();
        let clip_history = crate::clipboard::ClipHistory::default();
        clip_history.configure(config.clipboard_history, config.clipboard_history_size);

        Self {
            hotkey_draft: config.hotkey.clone(),
            config,
            indexed,
            config_item_count,
            lua,
            ranker: Ranker::new(),
            query: String::new(),
            results: Vec::new(),
            selected: 0,
            needs_search: start_visible,
            panel: None,
            args_prompt: None,
            instant_pending: None,
            cursor_to_end: false,
            clip_history,
            currency: Default::default(),
            rates_pending: false,
            view: View::Search,
            ext_drafts: Vec::new(),
            startup_enabled: crate::startup::is_enabled(),
            settings_status: None,
            settings_dirty: false,
            settings_rescan: false,
            scan_rx: None,
            scan_again: false,
            reload_stamp,
            egui_ctx: cc.egui_ctx.clone(),
            window_size: None,
            icons: IconCache::new(cc.egui_ctx.clone()),
            ctl,
            last_visible: start_visible,
            first_frame: true,
            history: History::load(),
            had_focus: false,
            ime_composing: false,
            hotkey_notice,
            active_hotkey,
            tray_flags,
            _tray: tray,
            hotkey_manager,
            hotkey_input,
        }
    }

    /// Track IME composition from this frame's events. Returns true when the
    /// IME owns the keyboard this frame (composing now, or it just ended).
    fn update_ime_state(&mut self, ctx: &egui::Context) -> bool {
        let mut busy = self.ime_composing;
        ctx.input(|i| {
            for event in &i.events {
                let egui::Event::Ime(ime) = event else {
                    continue;
                };
                match ime {
                    egui::ImeEvent::Preedit(text) => {
                        self.ime_composing = !text.is_empty();
                        busy = true;
                    }
                    egui::ImeEvent::Commit(_) => {
                        self.ime_composing = false;
                        busy = true;
                    }
                    egui::ImeEvent::Disabled => self.ime_composing = false,
                    egui::ImeEvent::Enabled => {}
                }
            }
        });
        busy
    }

    /// Reset state when the window (re)appears.
    fn on_shown(&mut self, ctx: &egui::Context) {
        let stamp = reload_stamp();
        if self.reload_stamp != stamp {
            self.reload_config(ctx, stamp);
        }
        self.query.clear();
        self.results.clear();
        self.selected = 0;
        self.panel = None;
        self.args_prompt = None;
        self.had_focus = false;
        self.ime_composing = false;
        self.needs_search = true; // populate the most-used view
        self.view = View::Search;
        self.hotkey_draft = self.config.hotkey.clone();
        self.settings_status = None;
        self.resize(ctx, egui::vec2(self.config.width, self.config.height));
    }

    fn resize(&mut self, ctx: &egui::Context, size: egui::Vec2) {
        if self.window_size == Some(size) {
            return;
        }
        self.window_size = Some(size);
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
    }

    fn hide_window(&mut self) {
        // Don't lose a settings edit that was still being typed.
        self.flush_settings();
        self.ctl.hide();
        self.last_visible = false;
    }

    /// Reload config/plugins only after their files changed.
    fn reload_config(&mut self, ctx: &egui::Context, stamp: ReloadStamp) {
        let scan_before = scan_key(&self.config);
        self.config = config::load();
        self.clip_history.configure(
            self.config.clipboard_history,
            self.config.clipboard_history_size,
        );
        if scan_key(&self.config) != scan_before {
            self.request_rescan();
        } else {
            self.refresh_config_items();
        }
        self.lua = LuaHost::new(&config::plugin_dir(), ctx.clone());
        self.reload_stamp = stamp;
    }

    /// Start one background scan. If a second request arrives while it is
    /// running, the latest config is scanned again after the first result.
    fn request_rescan(&mut self) {
        if self.scan_rx.is_some() {
            self.scan_again = true;
            return;
        }
        let config = self.config.clone();
        let ctx = self.egui_ctx.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let indexed = providers::scan_indexed(&config);
            let _ = tx.send(indexed);
            ctx.request_repaint();
        });
        self.scan_rx = Some(rx);
    }

    fn poll_rescan(&mut self) {
        let result = match self.scan_rx.as_ref() {
            Some(rx) => match rx.try_recv() {
                Ok(indexed) => Some(Ok(indexed)),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err(())),
            },
            None => return,
        };
        let Some(result) = result else { return };

        self.scan_rx = None;
        if let Ok(indexed) = result {
            self.indexed = indexed;
            self.config_item_count = 0;
            self.refresh_config_items();
            self.needs_search = true;
        }
        if self.scan_again {
            self.scan_again = false;
            self.request_rescan();
        }
    }

    /// Rebuild only the cheap part of the index: the items from config.toml.
    fn refresh_config_items(&mut self) {
        self.indexed
            .truncate(self.indexed.len() - self.config_item_count);
        let config_items = providers::config_items(&self.config);
        self.config_item_count = config_items.len();
        self.indexed.extend(config_items);
    }

    fn open_settings(&mut self, ctx: &egui::Context) {
        self.view = View::Settings;
        self.resize(ctx, SETTINGS_SIZE);
        self.hotkey_draft = self.config.hotkey.clone();
        self.startup_enabled = crate::startup::is_enabled();
        self.settings_status = None;
        self.ext_drafts = self
            .config
            .scan_folders
            .iter()
            .map(|f| {
                f.extensions
                    .as_ref()
                    .map(|e| e.join(", "))
                    .unwrap_or_default()
            })
            .collect();
    }

    fn close_settings(&mut self, ctx: &egui::Context) {
        self.flush_settings();
        self.view = View::Search;
        self.query.clear();
        self.needs_search = true;
        self.resize(ctx, egui::vec2(self.config.width, self.config.height));
    }

    /// Write pending settings changes to config.toml and refresh the index.
    ///
    /// Edits are batched rather than saved on every keystroke: a text field is
    /// only written out once it is left.
    fn flush_settings(&mut self) {
        if !self.settings_dirty {
            return;
        }
        self.settings_dirty = false;
        self.clip_history.configure(
            self.config.clipboard_history,
            self.config.clipboard_history_size,
        );
        match config::save(&self.config) {
            Ok(()) => {
                self.settings_status = None;
                self.reload_stamp = reload_stamp();
            }
            Err(e) => self.settings_status = Some(e),
        }
        if std::mem::take(&mut self.settings_rescan) {
            self.request_rescan();
        } else {
            self.refresh_config_items();
        }
    }

    /// Swap the global hotkey for `spec`, falling back like startup does.
    fn apply_hotkey(&mut self, spec: &str) {
        if self.active_hotkey.as_ref().is_some_and(|b| b.spec == spec) {
            return;
        }
        if let Some(old) = self.active_hotkey.take() {
            let _ = self.hotkey_manager.unregister(old.hotkey);
        }
        let (binding, notice) = register_hotkey(&self.hotkey_manager, spec);
        self.hotkey_input
            .set_hotkey(binding.as_ref().map(|binding| binding.hotkey));
        self.active_hotkey = binding;
        self.hotkey_notice = notice;
    }

    fn settings_ui(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let mut edits = Edits::default();
        let mut close = false;

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("設定").heading());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = ui.button("閉じる (Esc)").clicked();
                if ui.button("config.toml を開く").clicked() {
                    let path = config::config_dir().join("config.toml");
                    launch::open_in_editor(&path.display().to_string());
                }
            });
        });
        ui.separator();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                section(ui, "検索対象");
                for (label, hint_text, flag) in [
                    (
                        "スタートメニューのアプリ",
                        "インストール済みアプリのショートカット",
                        &mut self.config.scan_start_menu,
                    ),
                    (
                        "登録済みアプリ (App Paths)",
                        "Windows に起動用として登録された GUI アプリ",
                        &mut self.config.scan_registered_apps,
                    ),
                    (
                        "Microsoft Store アプリ",
                        "電卓、Windows Terminal など (UWP)",
                        &mut self.config.scan_uwp_apps,
                    ),
                    (
                        "PATH 上の実行ファイル",
                        "CLI ツールも起動できるが候補が大量に増える",
                        &mut self.config.scan_path,
                    ),
                    (
                        "Chocolatey のツール",
                        r"%ChocolateyInstall%\bin (既定: C:\ProgramData\chocolatey\bin)",
                        &mut self.config.scan_chocolatey,
                    ),
                    (
                        "電源系コマンド",
                        "シャットダウン、再起動、スリープ、ロックなど",
                        &mut self.config.system_commands,
                    ),
                    (
                        "主要なフォルダ",
                        "ダウンロード、デスクトップ、AppData、Temp、Program Files など",
                        &mut self.config.special_folders,
                    ),
                ] {
                    edits.toggle(&ui.checkbox(flag, label), true);
                    hint(ui, hint_text);
                }

                section(ui, "検索モード");
                note(ui, "プレフィックスに続けて入力すると各モードで検索します。空にすると無効");
                egui::Grid::new("kwick-settings-modes")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        for (_, label, prefix) in self.config.prefixes.entries_mut() {
                            ui.label(label);
                            edits.field(
                                &ui.add(egui::TextEdit::singleline(prefix).desired_width(80.0)),
                                false,
                            );
                            ui.end_row();
                        }
                    });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    edits.toggle(
                        &ui.checkbox(&mut self.config.clipboard_history, "クリップボード履歴"),
                        false,
                    );
                    ui.label("件数");
                    edits.field(
                        &ui.add(
                            egui::DragValue::new(&mut self.config.clipboard_history_size)
                                .range(1..=500),
                        ),
                        false,
                    );
                });
                hint(ui, "コピーしたテキストをメモリ上にだけ保持します (ディスクには保存しません)");

                {
                    let Self {
                        config, ext_drafts, ..
                    } = &mut *self;
                    scan_folders_ui(ui, config, ext_drafts, &mut edits);
                    commands_ui(ui, config, &mut edits);
                    web_searches_ui(ui, config, &mut edits);
                }

                section(ui, "表示");
                egui::Grid::new("kwick-settings-display")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("最大表示件数");
                        edits.field(
                            &ui.add(
                                egui::DragValue::new(&mut self.config.max_results).range(1..=30),
                            ),
                            false,
                        );
                        ui.end_row();

                        ui.label("検索窓のサイズ");
                        ui.horizontal(|ui| {
                            let w = ui.add(size_drag(&mut self.config.width));
                            ui.label("×");
                            let h = ui.add(size_drag(&mut self.config.height));
                            edits.field(&w, false);
                            edits.field(&h, false);
                            note(ui, "設定画面を閉じると反映されます");
                        });
                        ui.end_row();
                    });

                section(ui, "起動");
                egui::Grid::new("kwick-settings-startup")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("ホットキー");
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.hotkey_draft).desired_width(160.0),
                        );
                        // Applied when the field is left, by Enter or by clicking away.
                        if response.lost_focus() {
                            let spec = self.hotkey_draft.trim().to_ascii_lowercase();
                            if spec.is_empty() {
                                self.hotkey_draft = self.config.hotkey.clone();
                            } else if spec != self.config.hotkey {
                                self.config.hotkey = spec.clone();
                                self.hotkey_draft = spec.clone();
                                self.apply_hotkey(&spec);
                                edits.mark(false);
                            }
                        }
                        ui.end_row();

                        ui.label("");
                        note(ui, "例: alt+space / ctrl+shift+space (Enter で適用)");
                        ui.end_row();
                    });
                ui.add_space(2.0);
                if ui
                    .checkbox(&mut self.startup_enabled, "Windows ログオン時に自動起動")
                    .changed()
                    && !crate::startup::set_enabled(self.startup_enabled)
                {
                    self.startup_enabled = !self.startup_enabled;
                    self.settings_status = Some("スタートアップ登録に失敗しました".into());
                }

                if let Some(status) = &self.settings_status {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(status)
                            .color(ui.visuals().error_fg_color)
                            .size(11.0),
                    );
                }
                ui.add_space(8.0);
            });

        self.settings_dirty |= edits.dirty;
        self.settings_rescan |= edits.rescan;
        if edits.flush {
            self.flush_settings();
        }
        if close {
            self.close_settings(ctx);
        }
    }

    fn is_pinned(&self, title: &str) -> bool {
        self.config.pinned.iter().any(|t| t == title)
    }

    fn is_hidden(&self, title: &str) -> bool {
        self.config.hidden.iter().any(|t| t == title)
    }

    fn search(&mut self) {
        self.results.clear();
        self.selected = 0;
        self.panel = None;
        self.instant_pending = None;
        let query = self.query.trim().to_string();
        if self.args_prompt.is_some() {
            return;
        }
        if query.is_empty() {
            // Empty query: pinned items, then the most-used ones.
            let mut used: Vec<&Item> = self
                .indexed
                .iter()
                .filter(|it| {
                    (self.is_pinned(&it.title) || self.history.count(&it.title) > 0)
                        && !self.is_hidden(&it.title)
                })
                .collect();
            used.sort_by_key(|it| {
                (
                    !self.is_pinned(&it.title),
                    std::cmp::Reverse(self.history.count(&it.title)),
                )
            });
            self.results = used
                .into_iter()
                .take(self.config.max_results)
                .cloned()
                .collect();
            return;
        }

        // Search modes entered by a prefix ("w " windows, "kill " ...).
        let raw = self.query.trim_start().to_string();
        let prefixes = self.config.prefixes.clone();
        if let Some(rest) = mode_rest(&raw, &prefixes.windows) {
            let items = providers::winlist::list(self.ctl.raw_hwnd());
            self.show_filtered(items, rest);
            return;
        }
        if let Some(rest) = mode_rest(&raw, &prefixes.kill) {
            let items = providers::winlist::processes();
            self.show_filtered(items, rest);
            return;
        }
        if let Some(rest) = mode_rest(&raw, &prefixes.clipboard) {
            let items = clip_items(&self.clip_history.clips(), self.config.clipboard_history);
            self.show_filtered(items, rest);
            return;
        }

        // Inline answers: unit/currency conversion, dates.
        let mut answers = providers::convert::query(&query, &self.currency, &self.egui_ctx);
        self.rates_pending = self.currency.is_fetching();
        self.results.append(&mut answers);
        if self.config.system_commands {
            self.results.append(&mut providers::sysops::query(&query));
        }

        // Web searches: "keyword rest-of-query"
        for ws in &self.config.web_searches {
            if ws.keyword.is_empty() || !ws.url.contains("{query}") {
                continue; // quick link: an indexed item instead
            }
            let Some(rest) = query.strip_prefix(&ws.keyword) else {
                continue;
            };
            let Some(rest) = rest.strip_prefix(' ') else {
                continue;
            };
            let rest = rest.trim();
            if rest.is_empty() {
                continue;
            }
            // URLs get the query percent-encoded; a path or command line
            // template ("C:\Notes\{query}.md") gets it verbatim.
            let is_url = ws.url.starts_with("http://") || ws.url.starts_with("https://");
            let target = if is_url {
                ws.url.replace("{query}", &urlencoding::encode(rest))
            } else {
                ws.url.replace("{query}", rest)
            };
            let action = if is_url {
                Action::Url(target.clone())
            } else {
                Action::Open(target.clone())
            };
            self.results.push(
                Item::new(format!("{}: {}", ws.name, rest), target, action).transient(),
            );
        }

        // Lua plugins decide their own relevance; they go on top.
        let mut lua_items = self.lua.query(&query);
        self.results.append(&mut lua_items);

        // Fuzzy-matched indexed items, boosted by pins, what was picked for
        // this query before, and launch history.
        let remaining = self
            .config
            .max_results
            .saturating_sub(self.results.len().min(2));
        let learned = self.history.learned_bonuses(&query);
        let Self {
            config,
            history,
            ranker,
            indexed,
            results,
            ..
        } = self;
        for idx in ranker.rank(indexed, &query, remaining, |it| {
            if config.hidden.contains(&it.title) {
                return None;
            }
            let pin = if config.pinned.contains(&it.title) { 2000 } else { 0 };
            let learned = learned.get(&it.title).copied().unwrap_or(0);
            let alias = if !it.alias.is_empty() && it.alias.eq_ignore_ascii_case(&query) {
                5000
            } else {
                0
            };
            Some(alias + pin + learned + history.bonus(&it.title) + it.rank_boost)
        }) {
            results.push(indexed[idx].clone());
        }

        // An exact keyword marked `instant` runs without Enter.
        let instant = results
            .iter()
            .position(|it| it.instant && it.alias.eq_ignore_ascii_case(&query));
        self.instant_pending = instant;
    }

    /// Results of a search mode: everything in order for an empty query,
    /// otherwise fuzzy-ranked.
    fn show_filtered(&mut self, mut items: Vec<Item>, query: &str) {
        const MODE_MAX: usize = 50;
        crate::reading::annotate_kana(&mut items);
        let query = query.trim();
        if query.is_empty() {
            self.results = items.into_iter().take(MODE_MAX).collect();
            return;
        }
        let order = self.ranker.rank(&items, query, MODE_MAX, |_| Some(0));
        self.results = order.into_iter().map(|i| items[i].clone()).collect();
    }

    fn actions_for(&self, item: &Item) -> Vec<SubAction> {
        item.actions(
            self.is_pinned(&item.title),
            self.history.count(&item.title) > 0,
        )
    }

    /// Run the selected result's action bound to `shortcut`, if it has one.
    fn trigger(&mut self, ctx: &egui::Context, shortcut: Shortcut) {
        let Some(item) = self.results.get(self.selected).cloned() else {
            return;
        };
        if let Some(sub) = self
            .actions_for(&item)
            .into_iter()
            .find(|a| a.shortcut == Some(shortcut))
        {
            self.perform(ctx, &item, sub.action);
        }
    }

    /// Record a launch: history count plus the query it was picked for.
    fn remember(&mut self, item: &Item) {
        if !item.remember {
            return;
        }
        self.history.bump(&item.title);
        let query = self.query.trim().to_string();
        self.history.learn(&query, &item.title);
    }

    /// Take a result off the list (its window was closed, its process ended).
    fn drop_result(&mut self, item: &Item) {
        if let Some(i) = self.results.iter().position(|r| r.title == item.title) {
            self.results.remove(i);
            self.selected = self.selected.min(self.results.len().saturating_sub(1));
        }
    }

    /// Persist a pin/hide change and refresh the list in place.
    fn save_lists(&mut self) {
        if let Err(e) = config::save(&self.config) {
            eprintln!("kwick: {e}");
        }
        self.reload_stamp = reload_stamp();
        let keep = self.selected;
        self.search();
        self.selected = keep.min(self.results.len().saturating_sub(1));
    }

    fn perform(&mut self, ctx: &egui::Context, item: &Item, action: Action) {
        self.panel = None;
        // Actions that keep the launcher open.
        match &action {
            Action::Reload => {
                self.request_rescan();
                self.needs_search = true;
                return;
            }
            Action::Quit => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                std::process::exit(0);
            }
            Action::OpenSettings => {
                self.remember(item);
                self.open_settings(ctx);
                return;
            }
            Action::AskArgs(cmd) => {
                self.args_prompt = Some(ArgsPrompt {
                    cmd: cmd.clone(),
                    item: item.clone(),
                    saved_query: std::mem::take(&mut self.query),
                });
                self.results.clear();
                return;
            }
            Action::Pin(title) => {
                if !self.is_pinned(title) {
                    self.config.pinned.push(title.clone());
                }
                self.save_lists();
                return;
            }
            Action::Unpin(title) => {
                self.config.pinned.retain(|t| t != title);
                self.save_lists();
                return;
            }
            Action::Hide(title) => {
                if !self.is_hidden(title) {
                    self.config.hidden.push(title.clone());
                }
                self.config.pinned.retain(|t| t != title);
                self.save_lists();
                return;
            }
            Action::Forget(title) => {
                self.history.remove(title);
                let keep = self.selected;
                self.search();
                self.selected = keep.min(self.results.len().saturating_sub(1));
                return;
            }
            Action::SetQuery(text) => {
                self.remember(item);
                self.query = text.clone();
                self.cursor_to_end = true;
                self.needs_search = true;
                return;
            }
            Action::CloseWindow(hwnd) => {
                providers::winlist::close(*hwnd);
                self.drop_result(item);
                return;
            }
            Action::Kill(pids) => {
                providers::winlist::kill(pids);
                self.drop_result(item);
                return;
            }
            Action::ClearClipboardHistory => {
                self.clip_history.clear();
                self.needs_search = true;
                return;
            }
            _ => {}
        }
        if matches!(
            action,
            Action::Open(_) | Action::Exec { .. } | Action::RunAs { .. } | Action::OpenConfig
        ) {
            self.remember(item);
        }
        // Hide first so focus lands on whatever we launch.
        self.hide_window();
        match action {
            Action::Open(path) => shell_open(&path, None),
            Action::Exec { cmd, args } => shell_open(&cmd, args.as_deref()),
            Action::Url(url) => shell_open(&url, None),
            Action::OpenConfig => {
                let path = config::config_dir().join("config.toml");
                launch::open_in_editor(&path.display().to_string());
            }
            Action::Lua(idx) => self.lua.run(idx),
            Action::RegisterStartup => launch::set_startup(true),
            Action::UnregisterStartup => launch::set_startup(false),
            Action::Copy(text) => {
                crate::clipboard::set_text(&text);
            }
            Action::RunAs { cmd, args } => launch::run_as(&cmd, args.as_deref()),
            Action::Reveal(path) => launch::reveal(&path),
            Action::Focus(hwnd) => providers::winlist::focus(hwnd),
            Action::Paste(text) => crate::clipboard::paste(&text, self.ctl.previous()),
            Action::System(op) => providers::sysops::run(op),
            Action::Quit
            | Action::ClearClipboardHistory
            | Action::SetQuery(_)
            | Action::CloseWindow(_)
            | Action::Kill(_)
            | Action::Reload
            | Action::OpenSettings
            | Action::AskArgs(_)
            | Action::Pin(_)
            | Action::Unpin(_)
            | Action::Hide(_)
            | Action::Forget(_) => unreachable!(),
        }
    }

    /// Enter in the argument prompt: run the command with what was typed.
    fn submit_args(&mut self, ctx: &egui::Context) {
        let Some(prompt) = self.args_prompt.take() else {
            return;
        };
        let args = self.query.trim().to_string();
        self.query = prompt.saved_query;
        let action = Action::Exec {
            cmd: prompt.cmd,
            args: (!args.is_empty()).then_some(args),
        };
        self.perform(ctx, &prompt.item, action);
    }
}

/// Clipboard history as results, newest first.
fn clip_items(clips: &[crate::clipboard::Clip], enabled: bool) -> Vec<Item> {
    if !enabled {
        return vec![Item::new(
            "クリップボード履歴は無効です",
            "設定の clipboard_history を有効にしてください",
            Action::OpenSettings,
        )
        .transient()];
    }
    if clips.is_empty() {
        return vec![Item::new(
            "クリップボード履歴は空です",
            "テキストをコピーするとここに並びます (メモリ上だけに保持)",
            Action::Copy(String::new()),
        )
        .transient()];
    }
    let now = std::time::SystemTime::now();
    clips
        .iter()
        .map(|clip| {
            let first = clip
                .text
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("");
            let mut title: String = first.chars().take(80).collect();
            if first.chars().count() > 80 {
                title.push('…');
            }
            let lines = clip.text.lines().count();
            let secs = now.duration_since(clip.at).map(|d| d.as_secs()).unwrap_or(0);
            let ago = match secs {
                0..=59 => "たった今".to_string(),
                60..=3599 => format!("{} 分前", secs / 60),
                3600..=86399 => format!("{} 時間前", secs / 3600),
                _ => format!("{} 日前", secs / 86400),
            };
            let subtitle = if lines > 1 {
                format!("{lines} 行 · {ago}")
            } else {
                format!("{} 文字 · {ago}", clip.text.chars().count())
            };
            let mut item = Item::new(title, subtitle, Action::Paste(clip.text.clone()));
            // Match on the whole text, not just the first line (bounded).
            item.key = clip.text.chars().take(2000).collect();
            item.extra = vec![
                ("コピーだけする".into(), Action::Copy(clip.text.clone())),
                (
                    "クリップボード履歴をすべて消去".into(),
                    Action::ClearClipboardHistory,
                ),
            ];
            item.transient()
        })
        .collect()
}

/// The text after a mode prefix, if `query` starts with it (case-insensitive).
fn mode_rest<'a>(query: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() {
        return None;
    }
    let head = query.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &query[prefix.len()..])
}

/// Keys the launcher claims before the query box sees them.
#[derive(Default)]
struct Keys {
    esc: bool,
    up: bool,
    down: bool,
    tab: bool,
    delete: bool,
    /// Backspace in an empty query box.
    back: bool,
    action_panel: bool,
    submit: Option<Shortcut>,
}

/// Pull navigation keys out of this frame's events. Modifiers are matched
/// exactly (egui's `consume_key` would also take Ctrl+Enter for Enter).
/// Ctrl+C becomes "copy path" only when no query text is selected.
fn take_keys(input: &mut egui::InputState, query_empty: bool, text_selected: bool) -> Keys {
    use egui::{Event, Key};
    let mut keys = Keys::default();
    input.events.retain(|event| {
        if let Event::Copy = event {
            if !text_selected {
                keys.submit = Some(Shortcut::CtrlC);
                return false;
            }
            return true;
        }
        let Event::Key {
            key,
            pressed: true,
            modifiers: m,
            ..
        } = event
        else {
            return true;
        };
        let plain = m.is_none();
        let ctrl = m.command && !m.alt;
        match key {
            Key::Escape if plain => keys.esc = true,
            Key::ArrowUp if plain => keys.up = true,
            Key::ArrowDown if plain => keys.down = true,
            Key::Tab if plain => keys.tab = true,
            Key::Delete if plain && query_empty => keys.delete = true,
            Key::Backspace if plain && query_empty => keys.back = true,
            Key::K if ctrl && !m.shift => keys.action_panel = true,
            Key::Enter if !m.alt => {
                keys.submit = Some(match (m.command, m.shift) {
                    (true, true) => Shortcut::CtrlShiftEnter,
                    (true, false) => Shortcut::CtrlEnter,
                    (false, true) => Shortcut::ShiftEnter,
                    (false, false) => Shortcut::Enter,
                })
            }
            _ => return true,
        }
        false
    });
    keys
}

impl eframe::App for KwickApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_rescan();
        if self.rates_pending && !self.currency.is_fetching() {
            self.rates_pending = false;
            self.needs_search = true;
        }
        if self.tray_flags.reload.swap(false, Ordering::SeqCst) {
            self.request_rescan();
            self.needs_search = true;
        }

        // Visibility is owned by WindowCtl (hotkey/tray threads flip it);
        // here we only react to transitions.
        let visible = self.ctl.is_visible();
        if visible && !self.last_visible {
            self.on_shown(ctx);
        }
        self.last_visible = visible;
        if !visible {
            // eframe shows the window itself right after the first frame is
            // painted, whatever `with_visible` asked for, so a `--hidden`
            // start would flash up an empty window and leave it there.
            // Paint one more frame and put it back where it belongs.
            if std::mem::take(&mut self.first_frame) {
                ctx.request_repaint();
            }
            self.ctl.enforce_hidden();
            return;
        }
        self.first_frame = false;

        // Hide when the window loses focus (after it first gained it).
        let focused = ctx.input(|i| i.focused);
        if focused {
            self.had_focus = true;
        } else if self.ctl.is_activating() {
            // A delayed focus-loss notification from before this show request
            // must not immediately hide the window again.
            self.had_focus = false;
        } else if self.had_focus {
            self.hide_window();
            return;
        }
        if self.ctl.is_activating() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }

        // Settings view: only Escape is claimed, so the widgets keep the rest.
        if self.view == View::Settings {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.close_settings(ctx);
            } else {
                egui::CentralPanel::default().show(ctx, |ui| self.settings_ui(ctx, ui));
                return;
            }
        }

        // Keyboard navigation (consume before TextEdit sees the keys).
        // Delete/Backspace are only claimed while the query is empty, so
        // they still edit text while typing.
        let query_empty = self.query.is_empty();
        let history_view = self.query.trim().is_empty() && self.args_prompt.is_none();
        // While composing, the IME still lets the raw key events through, so
        // Enter (確定) would also launch the selection. Skip navigation for the
        // whole frame that carries IME events, as the key and the commit may
        // land together.
        let ime_busy = self.update_ime_state(ctx);
        let query_id = egui::Id::new(QUERY_ID);
        let text_selected = egui::TextEdit::load_state(ctx, query_id)
            .and_then(|s| s.cursor.char_range())
            .is_some_and(|r| r.primary != r.secondary);
        let keys = if ime_busy {
            Keys::default()
        } else {
            ctx.input_mut(|i| take_keys(i, query_empty, text_selected))
        };

        if self.panel.is_some() {
            self.panel_keys(ctx, &keys);
        } else if self.args_prompt.is_some() {
            if keys.esc {
                if let Some(prompt) = self.args_prompt.take() {
                    self.query = prompt.saved_query;
                    self.needs_search = true;
                }
            } else if keys.submit.is_some() {
                self.submit_args(ctx);
            }
        } else {
            if keys.esc {
                self.hide_window();
                return;
            }
            if keys.down && !self.results.is_empty() {
                self.selected = (self.selected + 1) % self.results.len();
            }
            if keys.up && !self.results.is_empty() {
                self.selected = (self.selected + self.results.len() - 1) % self.results.len();
            }
            if keys.action_panel {
                self.open_panel();
            }
            if let Some(shortcut) = keys.submit {
                self.trigger(ctx, shortcut);
            }
            if keys.delete && history_view {
                if let Some(title) = self.results.get(self.selected).map(|it| it.title.clone()) {
                    let keep = self.selected;
                    self.history.remove(&title);
                    self.search();
                    self.selected = keep.min(self.results.len().saturating_sub(1));
                }
            }
        }
        if !self.ctl.is_visible() {
            self.last_visible = false;
            return;
        }

        let moved = keys.up || keys.down;
        let mut clicked: Option<usize> = None;
        let mut panel_clicked: Option<usize> = None;
        let mut menu_action: Option<(Item, Action)> = None;

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(prompt) = &self.args_prompt {
                ui.label(
                    egui::RichText::new(format!(
                        "{} に渡す引数を入力して Enter (Esc で戻る)",
                        prompt.item.title
                    ))
                    .weak()
                    .size(11.0),
                );
            }
            let hint = if self.args_prompt.is_some() {
                "引数…"
            } else {
                "検索…"
            };
            if std::mem::take(&mut self.cursor_to_end) {
                let mut state = egui::TextEdit::load_state(ctx, query_id).unwrap_or_default();
                let end = egui::text::CCursor::new(self.query.chars().count());
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::one(end)));
                state.store(ctx, query_id);
            }
            let edit = egui::TextEdit::singleline(&mut self.query)
                .id(query_id)
                .font(egui::TextStyle::Heading)
                .hint_text(hint)
                .desired_width(f32::INFINITY)
                .frame(false);
            let response = ui.add(edit);
            response.request_focus();
            if response.changed() || self.needs_search {
                self.needs_search = false;
                self.search();
            }

            ui.separator();

            // Split borrows: the icon cache is written to while results are read.
            let Self {
                results,
                icons,
                selected,
                hotkey_notice,
                lua,
                history,
                config,
                panel,
                ..
            } = self;

            if let Some(panel) = panel {
                ui.label(
                    egui::RichText::new(format!("アクション: {}", panel.item.title))
                        .weak()
                        .size(11.0),
                );
                ui.add_space(2.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (i, sub) in panel.actions.iter().enumerate() {
                            let fill = if i == panel.selected {
                                ui.visuals().selection.bg_fill
                            } else {
                                egui::Color32::TRANSPARENT
                            };
                            let response = egui::Frame::new()
                                .fill(fill)
                                .corner_radius(6.0)
                                .inner_margin(egui::Margin::symmetric(8, 6))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new(&sub.label).size(15.0));
                                        if let Some(shortcut) = sub.shortcut {
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.label(
                                                        egui::RichText::new(shortcut.label())
                                                            .weak()
                                                            .size(11.0),
                                                    );
                                                },
                                            );
                                        }
                                    });
                                })
                                .response
                                .interact(egui::Sense::click());
                            if i == panel.selected && moved {
                                response.scroll_to_me(None);
                            }
                            if response.clicked() {
                                panel_clicked = Some(i);
                            }
                        }
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("Enter で実行 / Esc で戻る")
                                .weak()
                                .size(10.0),
                        );
                    });
                return;
            }

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (i, item) in results.iter().enumerate() {
                        let is_selected = i == *selected;
                        let fill = if is_selected {
                            ui.visuals().selection.bg_fill
                        } else {
                            egui::Color32::TRANSPARENT
                        };
                        let frame_response = egui::Frame::new()
                            .fill(fill)
                            .corner_radius(6.0)
                            .inner_margin(egui::Margin::symmetric(8, 6))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    let icon_size = egui::vec2(28.0, 28.0);
                                    let texture =
                                        item.icon_path.as_deref().and_then(|p| icons.get(p));
                                    match texture {
                                        Some(tex) => {
                                            ui.add(
                                                egui::Image::new(&tex).fit_to_exact_size(icon_size),
                                            );
                                        }
                                        // Inline answers (calculator, conversions) get "=".
                                        None if matches!(item.action, Action::Copy(_)) => {
                                            fallback_icon(ui, "=", icon_size)
                                        }
                                        None => fallback_icon(ui, &item.title, icon_size),
                                    }
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y = 1.0;
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                egui::RichText::new(&item.title)
                                                    .strong()
                                                    .size(16.0),
                                            );
                                            if config.pinned.contains(&item.title) {
                                                ui.label(
                                                    egui::RichText::new("固定")
                                                        .weak()
                                                        .size(10.0),
                                                );
                                            }
                                        });
                                        ui.label(
                                            egui::RichText::new(&item.subtitle).weak().size(11.0),
                                        );
                                    });
                                });
                            })
                            .response;
                        let frame_response = frame_response.interact(egui::Sense::click());
                        if is_selected && moved {
                            frame_response.scroll_to_me(None);
                        }
                        if frame_response.clicked() {
                            clicked = Some(i);
                        }
                        frame_response.context_menu(|ui| {
                            let actions = item.actions(
                                config.pinned.contains(&item.title),
                                history.count(&item.title) > 0,
                            );
                            for sub in actions {
                                if ui.button(&sub.label).clicked() {
                                    menu_action = Some((item.clone(), sub.action));
                                    ui.close();
                                }
                            }
                        });
                    }

                    if !results.is_empty() {
                        ui.add_space(4.0);
                        let text = if history_view {
                            "Ctrl+K でアクション / Del で履歴から削除"
                        } else {
                            "Ctrl+K でアクション"
                        };
                        ui.label(egui::RichText::new(text).weak().size(10.0));
                    }

                    if let Some(notice) = hotkey_notice.as_deref() {
                        ui.separator();
                        ui.label(
                            egui::RichText::new(notice)
                                .color(ui.visuals().warn_fg_color)
                                .size(11.0),
                        );
                    }

                    if !lua.errors.is_empty() {
                        ui.separator();
                        for err in &lua.errors {
                            ui.label(
                                egui::RichText::new(err)
                                    .color(ui.visuals().error_fg_color)
                                    .size(11.0),
                            );
                        }
                    }
                });
        });

        if let Some(i) = self.instant_pending.take() {
            self.selected = i;
            self.trigger(ctx, Shortcut::Enter);
        } else if let Some(i) = panel_clicked {
            if let Some(panel) = self.panel.as_mut() {
                panel.selected = i;
            }
            self.run_panel_selection(ctx);
        } else if let Some(i) = clicked {
            self.selected = i;
            self.trigger(ctx, Shortcut::Enter);
        } else if let Some((item, action)) = menu_action {
            self.perform(ctx, &item, action);
        }
        if !self.ctl.is_visible() {
            self.last_visible = false;
        }
    }
}

impl KwickApp {
    fn open_panel(&mut self) {
        let Some(item) = self.results.get(self.selected).cloned() else {
            return;
        };
        let actions = self.actions_for(&item);
        self.panel = Some(Panel {
            item,
            actions,
            selected: 0,
        });
    }

    fn panel_keys(&mut self, ctx: &egui::Context, keys: &Keys) {
        let Some(panel) = self.panel.as_mut() else {
            return;
        };
        if keys.esc || keys.action_panel || keys.back {
            self.panel = None;
            return;
        }
        let n = panel.actions.len();
        if keys.down && n > 0 {
            panel.selected = (panel.selected + 1) % n;
        }
        if keys.up && n > 0 {
            panel.selected = (panel.selected + n - 1) % n;
        }
        match keys.submit {
            Some(Shortcut::Enter) => self.run_panel_selection(ctx),
            Some(shortcut) => {
                if let Some(i) = panel.actions.iter().position(|a| a.shortcut == Some(shortcut)) {
                    panel.selected = i;
                    self.run_panel_selection(ctx);
                }
            }
            None => {}
        }
    }

    fn run_panel_selection(&mut self, ctx: &egui::Context) {
        let Some(panel) = self.panel.take() else {
            return;
        };
        if let Some(sub) = panel.actions.get(panel.selected) {
            self.perform(ctx, &panel.item, sub.action.clone());
        }
    }
}
