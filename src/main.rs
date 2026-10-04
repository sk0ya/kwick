// Kwick is a tray/GUI application. Keep the console subsystem disabled in
// every profile so registering a debug build for Windows startup cannot open
// a terminal window at logon.
#![windows_subsystem = "windows"]

use eframe::egui;

mod app;
mod clipboard;
mod config;
mod fonts;
mod history;
mod http;
mod hotkey;
mod icons;
mod instance;
mod launch;
mod lua_host;
mod matcher;
mod reading;
mod providers;
mod startup;
mod tray;
mod winctl;

fn main() -> eframe::Result {
    // --hidden: start resident without showing the window (used by startup registration)
    let start_visible = !std::env::args().any(|a| a == "--hidden");

    // Launching again just brings up the running instance.
    if instance::already_running() {
        if start_visible && !instance::signal_show() {
            use windows::core::w;
            use windows::Win32::UI::WindowsAndMessaging::{
                MessageBoxW, MB_ICONINFORMATION, MB_OK,
            };
            unsafe {
                MessageBoxW(
                    None,
                    w!("Kwick は既に起動しています。ホットキーまたはタスクトレイのアイコンから開けます。"),
                    w!("Kwick"),
                    MB_OK | MB_ICONINFORMATION,
                );
            }
        }
        return Ok(());
    }

    let cfg = config::load();
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([cfg.width, cfg.height])
        .with_decorations(false)
        .with_always_on_top()
        .with_resizable(false)
        .with_taskbar(false)
        .with_visible(start_visible);
    if let Some((rgba, w, h)) = icons::app_icon_rgba() {
        viewport = viewport.with_icon(egui::IconData {
            rgba,
            width: w as u32,
            height: h as u32,
        });
    }
    let options = eframe::NativeOptions {
        viewport,
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "Kwick",
        options,
        Box::new(move |cc| Ok(Box::new(app::KwickApp::new(cc, start_visible)))),
    )
}
