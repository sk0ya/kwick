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

/// Open a file/shortcut/folder/URL with its default handler, optionally with args.
pub fn shell_open(file: &str, params: Option<&str>) {
    let file = HSTRING::from(file);
    let params = params.map(HSTRING::from);
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
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
