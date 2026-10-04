use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, IsIconic, IsWindowVisible, SetForegroundWindow, SetWindowPos,
    ShowWindowAsync, HWND_TOPMOST, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_SHOWWINDOW, SW_HIDE, SW_RESTORE, SW_SHOW,
};

/// Shows/hides the launcher window via Win32 directly, bypassing egui.
///
/// This is deliberate: while the window is hidden Windows delivers no
/// WM_PAINT, so eframe's `update()` never runs and a ViewportCommand sent
/// from a background thread would never be processed. Calling ShowWindowAsync
/// from the hotkey/tray threads works regardless of the event loop state.
pub struct WindowCtl {
    hwnd: AtomicIsize,
    visible: AtomicBool,
    activation: Mutex<Option<Instant>>,
    previous: AtomicIsize,
}

impl WindowCtl {
    pub fn new(hwnd: isize, visible: bool) -> Self {
        Self {
            hwnd: AtomicIsize::new(hwnd),
            visible: AtomicBool::new(visible),
            activation: Mutex::new(None),
            previous: AtomicIsize::new(0),
        }
    }

    fn hwnd(&self) -> Option<HWND> {
        match self.hwnd.load(Ordering::SeqCst) {
            0 => None,
            h => Some(HWND(h as *mut _)),
        }
    }

    pub fn raw_hwnd(&self) -> isize {
        self.hwnd.load(Ordering::SeqCst)
    }

    pub fn is_visible(&self) -> bool {
        self.visible.load(Ordering::SeqCst)
    }

    /// The window that was in front when the launcher was last shown (paste
    /// target for clipboard history).
    pub fn previous(&self) -> isize {
        self.previous.load(Ordering::SeqCst)
    }

    pub fn show(&self) {
        let Some(hwnd) = self.hwnd() else { return };
        if !self.is_visible() {
            let front = unsafe { GetForegroundWindow() };
            if front != hwnd {
                self.previous.store(front.0 as isize, Ordering::SeqCst);
            }
        }
        let mut activation = self.activation.lock().unwrap_or_else(|e| e.into_inner());
        *activation = Some(Instant::now());
        // Publish intent before Windows can deliver paint/focus messages.
        self.visible.store(true, Ordering::SeqCst);
        Self::show_native(hwnd);
    }

    fn show_native(hwnd: HWND) {
        unsafe {
            // Do not block the input worker on the GUI thread.
            let command = if IsIconic(hwnd).as_bool() {
                SW_RESTORE
            } else {
                SW_SHOW
            };
            let _ = ShowWindowAsync(hwnd, command);
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_ASYNCWINDOWPOS,
            );
            let _ = SetForegroundWindow(hwnd);
        }
    }

    pub fn is_activating(&self) -> bool {
        self.activation
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some_and(|at| at.elapsed() < Duration::from_millis(750))
    }

    pub fn retry_show(&self) {
        let activation = self.activation.lock().unwrap_or_else(|e| e.into_inner());
        if !self.is_visible()
            || !activation.is_some_and(|at| at.elapsed() < Duration::from_millis(750))
        {
            return;
        }
        let Some(hwnd) = self.hwnd() else { return };
        unsafe {
            if !IsWindowVisible(hwnd).as_bool()
                || IsIconic(hwnd).as_bool()
                || GetForegroundWindow() != hwnd
            {
                Self::show_native(hwnd);
            }
        }
    }

    pub fn hide(&self) {
        let Some(hwnd) = self.hwnd() else { return };
        let mut activation = self.activation.lock().unwrap_or_else(|e| e.into_inner());
        *activation = None;
        self.visible.store(false, Ordering::SeqCst);
        unsafe {
            let _ = ShowWindowAsync(hwnd, SW_HIDE);
        }
    }

    /// Re-hide the window if Windows is showing it while we consider it hidden.
    ///
    /// eframe always creates the window hidden and then calls
    /// `set_visible(true)` itself once the first frame has been painted,
    /// ignoring `ViewportBuilder::with_visible(false)`. Without this, a
    /// `--hidden` start would leave an empty (black) always-on-top window
    /// on screen that never repaints.
    pub fn enforce_hidden(&self) {
        let _activation = self.activation.lock().unwrap_or_else(|e| e.into_inner());
        if self.is_visible() {
            return;
        }
        let Some(hwnd) = self.hwnd() else { return };
        unsafe {
            if IsWindowVisible(hwnd).as_bool() {
                let _ = ShowWindowAsync(hwnd, SW_HIDE);
                settle_hidden(hwnd);
            }
        }
    }

}

static INSTANCE: std::sync::OnceLock<std::sync::Arc<WindowCtl>> = std::sync::OnceLock::new();

/// Make `ctl` the window that `wake` checks.
pub fn register(ctl: std::sync::Arc<WindowCtl>) {
    let _ = INSTANCE.set(ctl);
}

/// Request a repaint from a background thread, but only while the launcher
/// is shown (or being shown). A repaint requested while hidden can never
/// happen (no WM_PAINT for hidden windows) and leaves eframe polling at full
/// speed; whatever changed is picked up by the frame drawn on the next show.
pub fn wake(ctx: &eframe::egui::Context) {
    if INSTANCE.get().is_none_or(|ctl| ctl.is_visible()) {
        ctx.request_repaint();
    }
}

/// Let eframe go idle after we hid a window it had just shown.
///
/// Showing/hiding produces window events after which eframe wants one more
/// frame: it switches winit to `ControlFlow::Poll` and asks for a redraw.
/// A hidden window never gets WM_PAINT, so that redraw never happens and the
/// event loop spins at full speed (a `--hidden` start burned a whole core).
/// Delivering WM_PAINT ourselves runs that frame; eframe then goes back to
/// waiting.
fn settle_hidden(hwnd: HWND) {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutW, SMTO_ABORTIFHUNG, WM_PAINT};
    let raw = hwnd.0 as isize;
    std::thread::spawn(move || {
        for delay in [150, 600] {
            std::thread::sleep(Duration::from_millis(delay));
            unsafe {
                let _ = SendMessageTimeoutW(
                    HWND(raw as *mut _),
                    WM_PAINT,
                    WPARAM(0),
                    LPARAM(0),
                    SMTO_ABORTIFHUNG,
                    500,
                    None,
                );
            }
        }
    });
}

impl WindowCtl {
    pub fn toggle(&self) {
        if self.is_visible() {
            self.hide();
        } else {
            self.show();
        }
    }
}
