#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;

mod app;
mod config;
mod fonts;
mod history;
mod icons;
mod launch;
mod lua_host;
mod matcher;
mod providers;
mod startup;
mod tray;
mod winctl;

/// A second resident instance would silently fight over the global hotkey,
/// so refuse to start if one is already running.
fn already_running() -> bool {
    use windows::core::w;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;
    unsafe {
        // Leak the handle on purpose: it must live as long as the process.
        let _ = CreateMutexW(None, false, w!("Kwick-SingleInstance"));
        GetLastError() == ERROR_ALREADY_EXISTS
    }
}

fn main() -> eframe::Result {
    if already_running() {
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};
        unsafe {
            MessageBoxW(
                None,
                w!("Kwick は既に起動しています。ホットキーまたはタスクトレイのアイコンから開けます。"),
                w!("Kwick"),
                MB_OK | MB_ICONINFORMATION,
            );
        }
        return Ok(());
    }

    // --hidden: start resident without showing the window (used by startup registration)
    let start_visible = !std::env::args().any(|a| a == "--hidden");
    let cfg = config::load();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([cfg.width, cfg.height])
            .with_decorations(false)
            .with_always_on_top()
            .with_resizable(false)
            .with_taskbar(false)
            .with_visible(start_visible),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "Kwick",
        options,
        Box::new(move |cc| Ok(Box::new(app::KwickApp::new(cc, start_visible)))),
    )
}
