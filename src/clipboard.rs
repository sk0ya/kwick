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
