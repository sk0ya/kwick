//! Windows スタートアップ登録。
//! HKCU\Software\Microsoft\Windows\CurrentVersion\Run に自分自身を書き込む。

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RRF_RT_REG_SZ,
};

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const VALUE_NAME: PCWSTR = w!("Kwick");

/// 登録するコマンドライン。ログオン時はウィンドウを出さず常駐だけさせる。
fn command_utf16() -> Option<Vec<u16>> {
    let exe = std::env::current_exe().ok()?;
    let cmd = format!("\"{}\" --hidden", exe.display());
    Some(cmd.encode_utf16().chain(std::iter::once(0)).collect())
}

pub fn is_enabled() -> bool {
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE_NAME,
            RRF_RT_REG_SZ,
            None,
            None,
            None,
        )
        .is_ok()
    }
}

/// 登録/解除。成功なら true。未登録状態での解除は成功扱い。
pub fn set_enabled(enable: bool) -> bool {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, Some(0), KEY_SET_VALUE, &mut key).is_err() {
            return false;
        }
        let ok = if enable {
            match command_utf16() {
                Some(wide) => {
                    let bytes =
                        std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2);
                    RegSetValueExW(key, VALUE_NAME, None, REG_SZ, Some(bytes)).is_ok()
                }
                None => false,
            }
        } else {
            let err = RegDeleteValueW(key, VALUE_NAME);
            err.is_ok() || err == ERROR_FILE_NOT_FOUND
        };
        let _ = RegCloseKey(key);
        ok
    }
}
