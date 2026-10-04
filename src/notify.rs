//! Desktop notifications for plugins (kwick.notify): a short-lived tray
//! icon carrying a balloon, which Windows 10/11 shows as a toast. No
//! AppUserModelID registration or WinRT needed.

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::*;

fn copy_into(dst: &mut [u16], text: &str) {
    let units: Vec<u16> = text.encode_utf16().take(dst.len() - 1).collect();
    dst[..units.len()].copy_from_slice(&units);
    dst[units.len()] = 0;
}

pub fn show(title: &str, text: &str) {
    let (title, text) = (title.to_string(), text.to_string());
    std::thread::spawn(move || unsafe {
        unsafe extern "system" fn wndproc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
            DefWindowProcW(h, m, w, l)
        }
        let Ok(instance) = windows::Win32::System::LibraryLoader::GetModuleHandleW(None) else {
            return;
        };
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: w!("KwickNotify"),
            ..Default::default()
        });
        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("KwickNotify"),
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
        let icon = LoadIconW(Some(instance.into()), windows::core::PCWSTR(1 as *const u16))
            .unwrap_or_default();
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 0x4B57,
            uFlags: NIF_ICON | NIF_TIP | NIF_INFO,
            hIcon: icon,
            dwInfoFlags: NIIF_INFO,
            ..Default::default()
        };
        copy_into(&mut data.szTip, "Kwick");
        copy_into(&mut data.szInfoTitle, &title);
        copy_into(&mut data.szInfo, &text);
        if Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
            // The toast lives on in the Action Center after the icon goes.
            std::thread::sleep(std::time::Duration::from_secs(6));
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        }
        let _ = DestroyWindow(hwnd);
    });
}
