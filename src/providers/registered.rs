use super::{Action, Item};
use std::collections::HashSet;
use std::path::PathBuf;
use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY, RRF_RT_REG_EXPAND_SZ,
    RRF_RT_REG_SZ,
};

const APP_PATHS: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\App Paths");
const FRIENDLY_NAME: PCWSTR = w!("FriendlyAppName");

/// Scan Windows' App Paths registry entries.
///
/// Unlike PATH, App Paths contains applications registered as launchable
/// programs, so it usually adds GUI applications without importing every
/// compiler/system helper executable.
pub fn scan() -> Vec<Item> {
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            scan_root(root, view, &mut seen, &mut items);
        }
    }
    items
}

fn scan_root(
    root: HKEY,
    view: windows::Win32::System::Registry::REG_SAM_FLAGS,
    seen: &mut HashSet<String>,
    items: &mut Vec<Item>,
) {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(root, APP_PATHS, Some(0), KEY_READ | view, &mut key) != ERROR_SUCCESS {
            return;
        }

        let mut index = 0;
        loop {
            let mut name = [0u16; 512];
            let mut name_len = (name.len() - 1) as u32;
            let status = RegEnumKeyExW(
                key,
                index,
                Some(PWSTR(name.as_mut_ptr())),
                &mut name_len,
                None,
                None,
                None,
                None,
            );
            if status == ERROR_NO_MORE_ITEMS {
                break;
            }
            index += 1;
            if status != ERROR_SUCCESS {
                continue;
            }

            let subkey_name = String::from_utf16_lossy(&name[..name_len as usize]);
            let subkey_wide: Vec<u16> = subkey_name
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let mut app_key = HKEY::default();
            if RegOpenKeyExW(
                key,
                PCWSTR(subkey_wide.as_ptr()),
                Some(0),
                KEY_READ,
                &mut app_key,
            ) != ERROR_SUCCESS
            {
                continue;
            }
            let path = read_value(app_key, PCWSTR::null()).and_then(parse_executable_path);
            let friendly = read_value(app_key, FRIENDLY_NAME);
            let _ = RegCloseKey(app_key);

            let Some(path) = path else { continue };
            let Some(file_name) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let dedupe_key = path.to_string_lossy().to_ascii_lowercase();
            if !seen.insert(dedupe_key) {
                continue;
            }

            let title = friendly
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| file_name.to_string());
            let mut item = Item::new(
                title.clone(),
                path.display().to_string(),
                Action::Open(path.display().to_string()),
            );
            item.key = format!("{title} {file_name} {subkey_name}");
            // Start Menu apps still win when the same title is present.
            item.rank_boost = 35;
            items.push(item);
        }
        let _ = RegCloseKey(key);
    }
}

unsafe fn read_value(key: HKEY, value_name: PCWSTR) -> Option<String> {
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ;
    let mut bytes = 0u32;
    let status = RegGetValueW(
        key,
        PCWSTR::null(),
        value_name,
        flags,
        None,
        None,
        Some(&mut bytes),
    );
    if status != ERROR_SUCCESS && bytes == 0 {
        return None;
    }
    let mut data = vec![0u16; (bytes as usize / 2) + 1];
    let mut data_bytes = (data.len() * 2) as u32;
    if !RegGetValueW(
        key,
        PCWSTR::null(),
        value_name,
        flags,
        None,
        Some(data.as_mut_ptr().cast()),
        Some(&mut data_bytes),
    )
    .is_ok()
    {
        return None;
    }
    let len = data_bytes as usize / 2;
    Some(
        String::from_utf16_lossy(&data[..len.min(data.len())])
            .trim_end_matches('\0')
            .to_string(),
    )
}

fn parse_executable_path(value: String) -> Option<PathBuf> {
    let value = value.trim();
    let path = if let Some(quoted) = value.strip_prefix('"') {
        quoted.split('"').next().unwrap_or_default()
    } else {
        let lower = value.to_ascii_lowercase();
        [".exe", ".com", ".bat", ".cmd"]
            .iter()
            .filter_map(|ext| lower.find(ext).map(|i| &value[..i + ext.len()]))
            .next()
            .unwrap_or(value)
    };
    let path = PathBuf::from(path.trim());
    if path.is_file() {
        Some(path)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::parse_executable_path;

    #[test]
    fn parses_quoted_path_with_arguments() {
        let exe = std::env::current_exe().unwrap();
        let path = parse_executable_path(format!(r#""{}" --flag"#, exe.display())).unwrap();
        assert_eq!(path, exe);
    }
}
