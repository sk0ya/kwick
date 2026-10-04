use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Register/unregister launching at Windows logon via the HKCU Run key.
pub fn set_startup(enable: bool) {
    use std::os::windows::process::CommandExt;
    let mut cmd = std::process::Command::new("reg");
    if enable {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        cmd.args([
            "add",
            RUN_KEY,
            "/v",
            "Kwick",
            "/t",
            "REG_SZ",
            "/d",
            &format!("\"{}\" --hidden", exe.display()),
            "/f",
        ]);
    } else {
        cmd.args(["delete", RUN_KEY, "/v", "Kwick", "/f"]);
    }
    let _ = cmd.creation_flags(CREATE_NO_WINDOW).status();
}

/// Open a text file in its associated editor, falling back to Notepad when
/// the extension (e.g. .toml) has no association.
pub fn open_in_editor(path: &str) {
    let file = HSTRING::from(path);
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &file,
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW returns a value <= 32 on failure (SE_ERR_NOASSOC etc.)
    if result.0 as isize <= 32 {
        shell_open("notepad", Some(path));
    }
}

/// Open a file/shortcut/folder/URL with its default handler, optionally with args.
pub fn shell_open(file: &str, params: Option<&str>) {
    shell_execute(w!("open"), file, params);
}

/// Run a program (or shortcut) elevated; Windows shows the UAC prompt.
pub fn run_as(file: &str, params: Option<&str>) {
    shell_execute(w!("runas"), file, params);
}

fn shell_execute(verb: PCWSTR, file: &str, params: Option<&str>) {
    let file = HSTRING::from(file);
    let params = params.map(HSTRING::from);
    unsafe {
        ShellExecuteW(
            None,
            verb,
            &file,
            params
                .as_ref()
                .map(|p| PCWSTR(p.as_ptr()))
                .unwrap_or(PCWSTR::null()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// Open Explorer with `path` selected. A shortcut reveals its target, which
/// is what "open file location" means for a Start Menu entry.
pub fn reveal(path: &str) {
    let target = if path.to_ascii_lowercase().ends_with(".lnk") {
        shortcut_target(path).unwrap_or_else(|| path.to_string())
    } else {
        path.to_string()
    };
    shell_open("explorer.exe", Some(&format!("/select,\"{target}\"")));
}

/// Resolve a .lnk to the file it points at (None for shell-only targets
/// such as Control Panel items).
fn shortcut_target(lnk: &str) -> Option<String> {
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED, STGM_READ,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::Interface;
    unsafe {
        // Already initialized on the UI thread (winit uses OLE); harmless then.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let file: IPersistFile = link.cast().ok()?;
        file.Load(&HSTRING::from(lnk), STGM_READ).ok()?;
        let mut buf = [0u16; 1024];
        link.GetPath(&mut buf, std::ptr::null_mut(), 0).ok()?;
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let target = String::from_utf16_lossy(&buf[..len]);
        (!target.is_empty() && std::path::Path::new(&target).exists()).then_some(target)
    }
}
