use crate::config::{self, Config};
use crate::history::History;
use crate::icons::IconCache;
use crate::launch::{self, shell_open};
use crate::lua_host::LuaHost;
use crate::matcher::Ranker;
use crate::providers::{self, Action, Item};
use crate::tray::TrayFlags;
use crate::winctl::WindowCtl;
use eframe::egui;
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[derive(PartialEq)]
enum View {
    Search,
    Settings,
}

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

    view: View,
    /// Hotkey text being edited in the settings view (applied on lost focus).
    hotkey_draft: String,
    /// Cached HKCU Run state so the settings checkbox doesn't hit the registry
    /// every frame.
    startup_enabled: bool,
    settings_status: Option<String>,

    icons: IconCache,
    ctl: Arc<WindowCtl>,
    /// Visibility as of the previous frame, to detect "just shown".
    last_visible: bool,
    history: History,
    had_focus: bool,
    hotkey_notice: Option<String>,
    /// Currently registered hotkey, kept so it can be unregistered on change.
    active_hotkey: Option<Binding>,
    tray_flags: Arc<TrayFlags>,
    _tray: Option<tray_icon::TrayIcon>,
    hotkey_manager: GlobalHotKeyManager,
}

/// A hotkey that is registered with the OS right now.
struct Binding {
    hotkey: HotKey,
    spec: String,
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

        // Toggle straight from the hotkey thread: while the window is hidden
        // the event loop gets no paint events, so this cannot go through the
        // app's update().
        {
            let ctl = ctl.clone();
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                let receiver = GlobalHotKeyEvent::receiver();
                while let Ok(event) = receiver.recv() {
                    if event.state == HotKeyState::Pressed {
                        ctl.toggle();
                        ctx.request_repaint();
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
            view: View::Search,
            startup_enabled: crate::startup::is_enabled(),
            settings_status: None,
            icons: IconCache::new(cc.egui_ctx.clone()),
            ctl,
            last_visible: start_visible,
            history: History::load(),
            had_focus: false,
            hotkey_notice,
            active_hotkey,
            tray_flags,
            _tray: tray,
            hotkey_manager,
        }
    }

    /// Reset state when the window (re)appears.
    fn on_shown(&mut self, ctx: &egui::Context) {
        self.reload_config(ctx);
        self.query.clear();
        self.results.clear();
        self.selected = 0;
        self.had_focus = false;
        self.needs_search = true; // populate the most-used view
        self.view = View::Search;
        self.hotkey_draft = self.config.hotkey.clone();
        self.settings_status = None;
    }

    fn hide_window(&mut self) {
        self.ctl.hide();
        self.last_visible = false;
    }

    /// Cheap reload on every show: config file, custom commands, Lua plugins.
    fn reload_config(&mut self, ctx: &egui::Context) {
        let scan_before = (
            self.config.scan_start_menu,
            self.config.scan_path,
            self.config.scan_chocolatey,
            self.config.scan_folders.clone(),
        );
        self.config = config::load();
        if (
            self.config.scan_start_menu,
            self.config.scan_path,
            self.config.scan_chocolatey,
            self.config.scan_folders.clone(),
        ) != scan_before
        {
            self.rescan_index();
        } else {
            self.indexed
                .truncate(self.indexed.len() - self.config_item_count);
            let config_items = providers::config_items(&self.config);
            self.config_item_count = config_items.len();
            self.indexed.extend(config_items);
        }
        self.lua = LuaHost::new(&config::plugin_dir(), ctx.clone());
    }

    fn rescan_index(&mut self) {
        self.indexed = providers::scan_indexed(&self.config);
        let config_items = providers::config_items(&self.config);
        self.config_item_count = config_items.len();
        self.indexed.extend(config_items);
    }

    fn open_settings(&mut self) {
        self.view = View::Settings;
        self.hotkey_draft = self.config.hotkey.clone();
        self.startup_enabled = crate::startup::is_enabled();
        self.settings_status = None;
    }

    fn close_settings(&mut self) {
        self.view = View::Search;
        self.query.clear();
        self.needs_search = true;
    }

    /// Persist the scalar settings; `rescan` when a scan source was toggled.
    fn save_settings(&mut self, rescan: bool) {
        match config::save(&self.config) {
            Ok(()) => self.settings_status = None,
            Err(e) => self.settings_status = Some(e),
        }
        if rescan {
            self.rescan_index();
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
        self.active_hotkey = binding;
        self.hotkey_notice = notice;
    }

    fn settings_ui(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let mut changed = false;
        let mut rescan = false;

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("設定").heading());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("閉じる (Esc)").clicked() {
                    self.view = View::Search;
                }
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
                ] {
                    if ui.checkbox(flag, label).changed() {
                        changed = true;
                        rescan = true;
                    }
                    hint(ui, hint_text);
                }

                section(ui, "表示");
                let mut size_changed = false;
                egui::Grid::new("kwick-settings-display")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("最大表示件数");
                        changed |= ui
                            .add(egui::DragValue::new(&mut self.config.max_results).range(1..=30))
                            .changed();
                        ui.end_row();

                        ui.label("ウィンドウサイズ");
                        ui.horizontal(|ui| {
                            size_changed |= ui.add(size_drag(&mut self.config.width)).changed();
                            ui.label("×");
                            size_changed |= ui.add(size_drag(&mut self.config.height)).changed();
                        });
                        ui.end_row();
                    });
                if size_changed {
                    changed = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                        self.config.width,
                        self.config.height,
                    )));
                }

                section(ui, "起動");
                egui::Grid::new("kwick-settings-startup")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("ホットキー");
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.hotkey_draft)
                                .desired_width(160.0),
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
                                changed = true;
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

                ui.add_space(10.0);
                ui.separator();
                note(
                    ui,
                    "スキャンフォルダ・カスタムコマンド・Web 検索は config.toml で編集します",
                );
                if let Some(status) = &self.settings_status {
                    ui.label(
                        egui::RichText::new(status)
                            .color(ui.visuals().error_fg_color)
                            .size(11.0),
                    );
                }
            });

        if changed {
            self.save_settings(rescan);
        }
    }

    fn search(&mut self) {
        self.results.clear();
        self.selected = 0;
        let query = self.query.trim().to_string();
        if query.is_empty() {
            // Empty query: show most-used items.
            let mut used: Vec<&Item> = self
                .indexed
                .iter()
                .filter(|it| self.history.count(&it.title) > 0)
                .collect();
            used.sort_by_key(|it| std::cmp::Reverse(self.history.count(&it.title)));
            self.results = used
                .into_iter()
                .take(self.config.max_results)
                .cloned()
                .collect();
            return;
        }

        // Web searches: "keyword rest-of-query"
        for ws in &self.config.web_searches {
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
            let url = ws.url.replace("{query}", &urlencoding::encode(rest));
            self.results.push(Item::new(
                format!("{}: {}", ws.name, rest),
                url.clone(),
                Action::Url(url),
            ));
        }

        // Lua plugins decide their own relevance; they go on top.
        let mut lua_items = self.lua.query(&query);
        self.results.append(&mut lua_items);

        // Fuzzy-matched indexed items, boosted by launch history.
        let remaining = self.config.max_results.saturating_sub(self.results.len().min(2));
        let history = &self.history;
        for idx in self
            .ranker
            .rank(&self.indexed, &query, remaining, |it| {
                history.bonus(&it.title) + it.rank_boost
            })
        {
            self.results.push(self.indexed[idx].clone());
        }
    }

    fn execute_selected(&mut self, ctx: &egui::Context) {
        let Some(item) = self.results.get(self.selected) else {
            return;
        };
        let action = item.action.clone();
        let title = item.title.clone();
        match action {
            Action::Reload => {
                self.rescan_index();
                self.needs_search = true;
                return;
            }
            Action::Quit => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                std::process::exit(0);
            }
            Action::OpenSettings => {
                self.history.bump(&title);
                self.open_settings();
                return;
            }
            _ => {}
        }
        // Hide first so focus lands on whatever we launch.
        self.hide_window();
        match action {
            Action::Open(path) => {
                self.history.bump(&title);
                shell_open(&path, None);
            }
            Action::Exec { cmd, args } => {
                self.history.bump(&title);
                shell_open(&cmd, args.as_deref());
            }
            Action::Url(url) => shell_open(&url, None),
            Action::OpenConfig => {
                self.history.bump(&title);
                let path = config::config_dir().join("config.toml");
                launch::open_in_editor(&path.display().to_string());
            }
            Action::Lua(idx) => self.lua.run(idx),
            Action::RegisterStartup => launch::set_startup(true),
            Action::UnregisterStartup => launch::set_startup(false),
            Action::Quit | Action::Reload | Action::OpenSettings => unreachable!(),
        }
    }
}

impl eframe::App for KwickApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.tray_flags.reload.swap(false, Ordering::SeqCst) {
            self.rescan_index();
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
            return;
        }

        // Hide when the window loses focus (after it first gained it).
        let focused = ctx.input(|i| i.focused);
        if focused {
            self.had_focus = true;
        } else if self.had_focus {
            self.hide_window();
            return;
        }

        // Settings view: only Escape is claimed, so the widgets keep the rest.
        if self.view == View::Settings {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.close_settings();
            } else {
                egui::CentralPanel::default().show(ctx, |ui| self.settings_ui(ctx, ui));
                return;
            }
        }

        // Keyboard navigation (consume before TextEdit sees the keys).
        // Delete is only claimed while the query is empty (the most-used
        // view), so it still edits text while typing a query.
        let history_view = self.query.trim().is_empty();
        let (esc, enter, up, down, del) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                history_view && i.consume_key(egui::Modifiers::NONE, egui::Key::Delete),
            )
        });
        if esc {
            self.hide_window();
            return;
        }
        if down && !self.results.is_empty() {
            self.selected = (self.selected + 1) % self.results.len();
        }
        if up && !self.results.is_empty() {
            self.selected = (self.selected + self.results.len() - 1) % self.results.len();
        }
        if enter {
            self.execute_selected(ctx);
            if !self.ctl.is_visible() {
                self.last_visible = false;
                return;
            }
        }
        if del {
            if let Some(title) = self.results.get(self.selected).map(|it| it.title.clone()) {
                let keep = self.selected;
                self.history.remove(&title);
                self.search();
                self.selected = keep.min(self.results.len().saturating_sub(1));
            }
        }

        let mut clicked: Option<usize> = None;
        let mut remove_from_history: Option<String> = None;

        egui::CentralPanel::default().show(ctx, |ui| {
            let edit = egui::TextEdit::singleline(&mut self.query)
                .font(egui::TextStyle::Heading)
                .hint_text("検索…")
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
                ..
            } = self;
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
                                    let texture = item
                                        .icon_path
                                        .as_deref()
                                        .and_then(|p| icons.get(p));
                                    match texture {
                                        Some(tex) => {
                                            ui.add(
                                                egui::Image::new(&tex)
                                                    .fit_to_exact_size(icon_size),
                                            );
                                        }
                                        None => fallback_icon(ui, &item.title, icon_size),
                                    }
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y = 1.0;
                                        ui.label(
                                            egui::RichText::new(&item.title)
                                                .strong()
                                                .size(16.0),
                                        );
                                        ui.label(
                                            egui::RichText::new(&item.subtitle)
                                                .weak()
                                                .size(11.0),
                                        );
                                    });
                                });
                            })
                            .response;
                        let frame_response = frame_response.interact(egui::Sense::click());
                        if is_selected && (up || down) {
                            frame_response.scroll_to_me(None);
                        }
                        if frame_response.clicked() {
                            clicked = Some(i);
                        }
                        if history.count(&item.title) > 0 {
                            frame_response.context_menu(|ui| {
                                if ui.button("履歴から削除").clicked() {
                                    remove_from_history = Some(item.title.clone());
                                    ui.close();
                                }
                            });
                        }
                    }

                    if history_view && !results.is_empty() {
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("Del または右クリックで履歴から削除")
                                .weak()
                                .size(10.0),
                        );
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

        if let Some(i) = clicked {
            self.selected = i;
            self.execute_selected(ctx);
        }
        if let Some(title) = remove_from_history {
            let keep = self.selected;
            self.history.remove(&title);
            if history_view {
                self.search();
                self.selected = keep.min(self.results.len().saturating_sub(1));
            }
        }
    }
}
