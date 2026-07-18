use crate::winctl::WindowCtl;
use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

#[derive(Default)]
pub struct TrayFlags {
    /// Index rescan must run on the app thread, so it goes through a flag.
    pub reload: AtomicBool,
}

/// Create the tray icon. Toggle/quit are handled directly on the forwarding
/// threads (see WindowCtl for why); reload is flagged for the app thread.
pub fn init(
    ctx: egui::Context,
    tooltip: &str,
    ctl: Arc<WindowCtl>,
) -> (Option<TrayIcon>, Arc<TrayFlags>) {
    let flags: Arc<TrayFlags> = Arc::default();

    let menu = Menu::new();
    let show = MenuItem::with_id("toggle", "表示 / 非表示", true, None);
    let settings = MenuItem::with_id("settings", "設定を開く", true, None);
    let reload = MenuItem::with_id("reload", "インデックス再読み込み", true, None);
    let startup = CheckMenuItem::with_id(
        "startup",
        "スタートアップに登録",
        true,
        crate::startup::is_enabled(),
        None,
    );
    let quit = MenuItem::with_id("quit", "終了", true, None);
    let _ = menu.append(&show);
    let _ = menu.append(&settings);
    let _ = menu.append(&reload);
    let _ = menu.append(&startup);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&quit);

    let mut builder = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(tooltip);
    let (rgba, w, h) = crate::icons::app_icon_rgba().unwrap_or_else(|| (icon_rgba(), 32, 32));
    if let Ok(icon) = tray_icon::Icon::from_rgba(rgba, w as u32, h as u32) {
        builder = builder.with_icon(icon);
    }
    let tray = match builder.build() {
        Ok(t) => Some(t),
        Err(e) => {
            eprintln!("kwick: failed to create tray icon: {e}");
            None
        }
    };

    {
        let flags = flags.clone();
        let ctx = ctx.clone();
        let ctl = ctl.clone();
        std::thread::spawn(move || {
            let rx = MenuEvent::receiver();
            while let Ok(event) = rx.recv() {
                match event.id.0.as_str() {
                    "toggle" => ctl.toggle(),
                    "settings" => {
                        let path = crate::config::config_dir().join("config.toml");
                        crate::launch::open_in_editor(&path.display().to_string());
                    }
                    "reload" => flags.reload.store(true, Ordering::SeqCst),
                    // クリック時のチェック表示は muda が自動で反転するので、
                    // レジストリ側も現在値の反転を書き込んで同期を保つ。
                    "startup" => {
                        let enable = !crate::startup::is_enabled();
                        if !crate::startup::set_enabled(enable) {
                            eprintln!("kwick: failed to update startup registration");
                        }
                    }
                    "quit" => std::process::exit(0),
                    _ => {}
                }
                ctx.request_repaint();
            }
        });
    }
    {
        std::thread::spawn(move || {
            let rx = TrayIconEvent::receiver();
            while let Ok(event) = rx.recv() {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    ctl.toggle();
                    ctx.request_repaint();
                }
            }
        });
    }

    (tray, flags)
}

/// Fallback 32x32 RGBA icon (white "K" on a blue rounded square), used only
/// if the embedded app icon (assets/icon.ico) fails to load.
fn icon_rgba() -> Vec<u8> {
    const ART: [&str; 16] = [
        "................",
        "................",
        "...##......##...",
        "...##.....##....",
        "...##....##.....",
        "...##...##......",
        "...##..##.......",
        "...#####........",
        "...#####........",
        "...##..##.......",
        "...##...##......",
        "...##....##.....",
        "...##.....##....",
        "...##......##...",
        "................",
        "................",
    ];
    const R: i32 = 6;
    let mut rgba = Vec::with_capacity(32 * 32 * 4);
    for y in 0i32..32 {
        for x in 0i32..32 {
            let nx = x.clamp(R, 31 - R);
            let ny = y.clamp(R, 31 - R);
            let (dx, dy) = (x - nx, y - ny);
            if dx * dx + dy * dy > R * R {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            let is_glyph =
                ART[(y / 2) as usize].as_bytes()[(x / 2) as usize] == b'#';
            if is_glyph {
                rgba.extend_from_slice(&[255, 255, 255, 255]);
            } else {
                rgba.extend_from_slice(&[56, 120, 240, 255]);
            }
        }
    }
    rgba
}
