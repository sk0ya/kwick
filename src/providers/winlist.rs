use super::{Action, Item};
use std::collections::BTreeMap;
use windows::core::BOOL;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, TerminateProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindow, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, PostMessageW, SetForegroundWindow,
    ShowWindow, GWL_EXSTYLE, GW_OWNER, SW_RESTORE, WM_CLOSE, WS_EX_TOOLWINDOW,
};

/// Top-level windows as they appear in Alt+Tab, front to back (EnumWindows
/// walks the Z order). `own` is the launcher itself, left out.
pub fn list(own: isize) -> Vec<Item> {
    let mut hwnds: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut hwnds as *mut _ as isize));
    }
    let mut out = Vec::new();
    for hwnd in hwnds {
        if hwnd.0 as isize == own || !is_switchable(hwnd) {
            continue;
        }
        let title = window_text(hwnd);
        if title.is_empty() {
            continue;
        }
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        let exe = process_path(pid);
        let exe_name = exe
            .as_deref()
            .and_then(|p| std::path::Path::new(p).file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut item = Item::new(title.clone(), exe_name.clone(), Action::Focus(hwnd.0 as isize));
        item.key = format!("{title} {exe_name}");
        item.icon_path = exe;
        item.extra = vec![
            ("ウィンドウを閉じる".into(), Action::CloseWindow(hwnd.0 as isize)),
            ("プロセスを終了".into(), Action::Kill(vec![pid])),
        ];
        out.push(item.transient());
    }
    out
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let hwnds = &mut *(lparam.0 as *mut Vec<HWND>);
    hwnds.push(hwnd);
    true.into()
}

/// The Alt+Tab rules: visible, unowned, not a tool window, and not cloaked
/// (UWP frames of suspended apps and windows on other virtual desktops are
/// "visible" but cloaked).
fn is_switchable(hwnd: HWND) -> bool {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return false;
        }
        if GetWindow(hwnd, GW_OWNER).is_ok_and(|owner| !owner.is_invalid()) {
            return false;
        }
        if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }
        let mut cloaked: u32 = 0;
        let ok = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut _,
            std::mem::size_of::<u32>() as u32,
        );
        !(ok.is_ok() && cloaked != 0)
    }
}

fn window_text(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..n as usize])
    }
}

fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(process);
        ok.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// Bring a window to the front, restoring it if minimized.
pub fn focus(hwnd: isize) {
    let hwnd = HWND(hwnd as *mut _);
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Ask a window to close, as its close button would.
pub fn close(hwnd: isize) {
    unsafe {
        let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

pub fn kill(pids: &[u32]) {
    for &pid in pids {
        unsafe {
            if let Ok(process) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                let _ = TerminateProcess(process, 1);
                let _ = CloseHandle(process);
            }
        }
    }
}

/// Running processes grouped by executable name, for "kill <name>". Enter
/// ends every process of that name, like `taskkill /im`.
pub fn processes() -> Vec<Item> {
    let own = std::process::id();
    let mut groups: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return Vec::new();
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut ok = Process32FirstW(snapshot, &mut entry).is_ok();
        while ok {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            // PID 0/4 are the idle and System processes.
            if entry.th32ProcessID > 4 && entry.th32ProcessID != own {
                groups.entry(name).or_default().push(entry.th32ProcessID);
            }
            ok = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
    }
    groups
        .into_iter()
        .map(|(name, pids)| {
            // Processes we may not query (services, other users) have no
            // path; they are usually not ours to end either, but keep them
            // listed so the attempt is possible.
            let path = pids.iter().find_map(|&pid| process_path(pid));
            let subtitle = match pids.len() {
                1 => format!("PID {} を終了", pids[0]),
                n => format!("{n} 個のプロセスをすべて終了"),
            };
            let mut item = Item::new(name, subtitle, Action::Kill(pids));
            item.icon_path = path;
            item.transient()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_processes_without_self() {
        let items = processes();
        assert!(!items.is_empty());
        let own = std::process::id();
        assert!(items
            .iter()
            .all(|it| !matches!(&it.action, Action::Kill(p) if p.contains(&own))));
    }
}
