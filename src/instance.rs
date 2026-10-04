use crate::winctl::WindowCtl;
use eframe::egui;
use std::sync::Arc;
use windows::core::HSTRING;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE,
    INFINITE,
};

/// A separate config dir (KWICK_CONFIG_DIR) is a separate instance.
fn name(base: &str) -> HSTRING {
    if std::env::var_os("KWICK_CONFIG_DIR").is_some() {
        HSTRING::from(format!("{base}-Dev"))
    } else {
        HSTRING::from(base)
    }
}

/// A second resident instance would silently fight over the global hotkey,
/// so refuse to start if one is already running.
pub fn already_running() -> bool {
    unsafe {
        // Leak the handle on purpose: it must live as long as the process.
        let _ = CreateMutexW(None, false, &name("Kwick-SingleInstance"));
        GetLastError() == ERROR_ALREADY_EXISTS
    }
}

/// Ask the running instance to show its window (launching kwick.exe again
/// is a natural way to bring it up). Returns false if it could not be told.
pub fn signal_show() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};
    unsafe {
        let Ok(event) = OpenEventW(EVENT_MODIFY_STATE, false, &name("Kwick-Show")) else {
            return false;
        };
        // We were started by the user, so we may hand the foreground over.
        let _ = AllowSetForegroundWindow(ASFW_ANY);
        SetEvent(event).is_ok()
    }
}

/// Wait for `signal_show` from later launches and show the launcher.
pub fn listen_show(ctl: Arc<WindowCtl>, ctx: egui::Context) {
    let Ok(event) = (unsafe { CreateEventW(None, false, false, &name("Kwick-Show")) }) else {
        return;
    };
    let event = event.0 as isize;
    std::thread::Builder::new()
        .name("kwick-show-event".into())
        .spawn(move || loop {
            let handle = windows::Win32::Foundation::HANDLE(event as *mut _);
            if unsafe { WaitForSingleObject(handle, INFINITE) }.0 != 0 {
                break;
            }
            ctl.show();
            ctx.request_repaint();
        })
        .ok();
}
