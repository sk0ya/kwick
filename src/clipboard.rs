use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

const CF_UNICODETEXT: u32 = 13;

/// Another process may hold the clipboard for a moment (clipboard managers
/// read it right after every change), so retry briefly.
fn open() -> bool {
    for _ in 0..10 {
        if unsafe { OpenClipboard(None) }.is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

/// Put text on the clipboard. Done through Win32 rather than egui so that it
/// takes effect immediately (egui applies copies at the end of the frame,
/// which is too late when we paste right after).
pub fn set_text(text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    if !open() {
        return false;
    }
    let ok = unsafe {
        (|| {
            EmptyClipboard().ok()?;
            let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).ok()?;
            let dst = GlobalLock(mem) as *mut u16;
            if dst.is_null() {
                return None;
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
            let _ = GlobalUnlock(mem);
            // On success the clipboard owns the memory.
            SetClipboardData(CF_UNICODETEXT, Some(HANDLE(mem.0))).ok()?;
            Some(())
        })()
        .is_some()
    };
    unsafe {
        let _ = CloseClipboard();
    }
    ok
}

/// The clipboard's text, if it holds any.
pub fn get_text() -> Option<String> {
    if !open() {
        return None;
    }
    let text = unsafe {
        (|| {
            let handle = GetClipboardData(CF_UNICODETEXT).ok()?;
            let mem = HGLOBAL(handle.0);
            let src = GlobalLock(mem) as *const u16;
            if src.is_null() {
                return None;
            }
            let mut len = 0;
            while *src.add(len) != 0 {
                len += 1;
            }
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(src, len));
            let _ = GlobalUnlock(mem);
            Some(text)
        })()
    };
    unsafe {
        let _ = CloseClipboard();
    }
    text
}

/// One remembered clipboard text.
#[derive(Clone)]
pub struct Clip {
    pub text: String,
    pub at: std::time::SystemTime,
}

/// Longer texts are not remembered (bounds memory; nobody pastes a whole
/// log file from a launcher).
const MAX_CLIP_BYTES: usize = 64 * 1024;

/// Text clipboard history, newest first. Kept in memory only: copied
/// passwords and the like never reach the disk.
#[derive(Clone, Default)]
pub struct ClipHistory {
    inner: std::sync::Arc<std::sync::Mutex<HistoryState>>,
}

#[derive(Default)]
struct HistoryState {
    clips: std::collections::VecDeque<Clip>,
    capacity: usize,
    enabled: bool,
    listening: bool,
}

impl ClipHistory {
    /// Apply config: start listening the first time it is enabled; when
    /// disabled, forget everything and ignore further changes.
    pub fn configure(&self, enabled: bool, capacity: usize) {
        let start = {
            let mut s = self.inner.lock().unwrap();
            s.enabled = enabled;
            s.capacity = capacity.max(1);
            if !enabled {
                s.clips.clear();
            }
            while s.clips.len() > s.capacity {
                s.clips.pop_back();
            }
            let start = enabled && !s.listening;
            s.listening |= start;
            start
        };
        if start {
            let history = self.clone();
            std::thread::Builder::new()
                .name("kwick-clipboard".into())
                .spawn(move || listen(history))
                .ok();
        }
    }

    pub fn clips(&self) -> Vec<Clip> {
        self.inner.lock().unwrap().clips.iter().cloned().collect()
    }

    pub fn clear(&self) {
        self.inner.lock().unwrap().clips.clear();
    }

    fn push(&self, text: String) {
        if text.trim().is_empty() || text.len() > MAX_CLIP_BYTES {
            return;
        }
        let mut s = self.inner.lock().unwrap();
        if !s.enabled {
            return;
        }
        s.clips.retain(|c| c.text != text);
        s.clips.push_front(Clip {
            text,
            at: std::time::SystemTime::now(),
        });
        while s.clips.len() > s.capacity {
            s.clips.pop_back();
        }
    }
}

/// Message-only window that receives WM_CLIPBOARDUPDATE. Event driven: no
/// polling, nothing runs until something is copied.
fn listen(history: ClipHistory) {
    use windows::core::w;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::DataExchange::AddClipboardFormatListener;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW,
        HWND_MESSAGE, MSG, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLIPBOARDUPDATE, WNDCLASSW,
    };

    thread_local! {
        static HISTORY: std::cell::RefCell<Option<ClipHistory>> =
            const { std::cell::RefCell::new(None) };
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        if msg == WM_CLIPBOARDUPDATE {
            if !is_excluded() {
                if let Some(text) = get_text() {
                    HISTORY.with(|h| {
                        if let Some(h) = h.borrow().as_ref() {
                            h.push(text);
                        }
                    });
                }
            }
            return LRESULT(0);
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }

    HISTORY.with(|h| *h.borrow_mut() = Some(history));
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            return;
        };
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: w!("KwickClipboardListener"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("KwickClipboardListener"),
            w!(""),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()),
            None,
        ) else {
            return;
        };
        if AddClipboardFormatListener(hwnd).is_err() {
            return;
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
    }
}

/// Password managers mark secrets so that clipboard history skips them.
fn is_excluded() -> bool {
    use windows::core::w;
    use windows::Win32::System::DataExchange::{
        IsClipboardFormatAvailable, RegisterClipboardFormatW,
    };
    unsafe {
        let marker = |name| {
            let format = RegisterClipboardFormatW(name);
            format != 0 && IsClipboardFormatAvailable(format).is_ok()
        };
        if marker(w!("ExcludeClipboardContentFromMonitorProcessing"))
            || marker(w!("Clipboard Viewer Ignore"))
        {
            return true;
        }
        // "CanIncludeInClipboardHistory" set to DWORD 0 also means "don't
        // keep this" (Windows' own Win+V history honors it).
        let format = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
        if format != 0 && IsClipboardFormatAvailable(format).is_ok() && open() {
            let allowed = (|| {
                let handle = GetClipboardData(format).ok()?;
                let ptr = GlobalLock(HGLOBAL(handle.0)) as *const u32;
                if ptr.is_null() {
                    return None;
                }
                let value = *ptr;
                let _ = GlobalUnlock(HGLOBAL(handle.0));
                Some(value != 0)
            })()
            .unwrap_or(true);
            let _ = CloseClipboard();
            return !allowed;
        }
        false
    }
}

/// Paste `text` into the window that was active before the launcher:
/// put it on the clipboard, give that window the focus back, press Ctrl+V.
pub fn paste(text: &str, target: isize) {
    if !set_text(text) {
        return;
    }
    std::thread::spawn(move || {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
            KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL,
        };
        use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
        unsafe {
            if target != 0 {
                let _ = SetForegroundWindow(HWND(target as *mut _));
            }
        }
        // Let the focus change land before the keystrokes.
        std::thread::sleep(std::time::Duration::from_millis(120));
        let key = |vk: VIRTUAL_KEY, up: bool| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    dwFlags: if up {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
                    ..Default::default()
                },
            },
        };
        let v = VIRTUAL_KEY(b'V' as u16);
        let inputs = [
            key(VK_CONTROL, false),
            key(v, false),
            key(v, true),
            key(VK_CONTROL, true),
        ];
        unsafe {
            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_dedupes_and_caps() {
        let h = ClipHistory::default();
        {
            let mut s = h.inner.lock().unwrap();
            s.enabled = true;
            s.capacity = 2;
            s.listening = true; // no listener thread in tests
        }
        h.push("a".into());
        h.push("b".into());
        h.push("a".into());
        h.push("   ".into());
        let texts: Vec<String> = h.clips().into_iter().map(|c| c.text).collect();
        assert_eq!(texts, ["a", "b"]);
        h.push("c".into());
        let texts: Vec<String> = h.clips().into_iter().map(|c| c.text).collect();
        assert_eq!(texts, ["c", "a"]);
        h.configure(false, 2);
        assert!(h.clips().is_empty());
    }
}
