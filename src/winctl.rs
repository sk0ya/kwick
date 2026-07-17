use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    SetForegroundWindow, ShowWindow, SW_HIDE, SW_SHOW,
};

/// Shows/hides the launcher window via Win32 directly, bypassing egui.
///
/// This is deliberate: while the window is hidden Windows delivers no
/// WM_PAINT, so eframe's `update()` never runs and a ViewportCommand sent
/// from a background thread would never be processed. Calling ShowWindow
/// from the hotkey/tray threads works regardless of the event loop state.
pub struct WindowCtl {
    hwnd: AtomicIsize,
    visible: AtomicBool,
}

impl WindowCtl {
    pub fn new(hwnd: isize, visible: bool) -> Self {
        Self {
            hwnd: AtomicIsize::new(hwnd),
            visible: AtomicBool::new(visible),
        }
    }

    fn hwnd(&self) -> Option<HWND> {
        match self.hwnd.load(Ordering::SeqCst) {
            0 => None,
            h => Some(HWND(h as *mut _)),
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible.load(Ordering::SeqCst)
    }

    pub fn show(&self) {
        let Some(hwnd) = self.hwnd() else { return };
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            // The thread that registered the global hotkey is allowed to
            // steal foreground; failure just means we show unfocused.
            let _ = SetForegroundWindow(hwnd);
        }
        self.visible.store(true, Ordering::SeqCst);
    }

    pub fn hide(&self) {
        let Some(hwnd) = self.hwnd() else { return };
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
        self.visible.store(false, Ordering::SeqCst);
    }

    pub fn toggle(&self) {
        if self.is_visible() {
            self.hide();
        } else {
            self.show();
        }
    }
}
