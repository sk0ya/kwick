use super::{Action, Item};

/// Packaged (UWP / Microsoft Store) apps: Calculator, Settings, Windows
/// Terminal, Teams... They have no .lnk in the Start Menu folders, so they
/// are read from the shell's Applications folder (`shell:AppsFolder`) and
/// launched through it by their AppUserModelID.
///
/// Desktop apps listed there as well are skipped; the other providers
/// already cover them with a real path.
pub fn scan() -> Vec<Item> {
    match unsafe { enumerate() } {
        Ok(items) => items,
        Err(e) => {
            eprintln!("kwick: AppsFolder enumeration failed: {e}");
            Vec::new()
        }
    }
}

unsafe fn enumerate() -> windows::core::Result<Vec<Item>> {
    use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{
        BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, SHGetKnownFolderItem,
        KF_FLAG_DEFAULT, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
    };

    // Scans run on a worker thread of their own.
    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    let folder: IShellItem = SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)?;
    let items: IEnumShellItems = folder.BindToHandler(None, &BHID_EnumItems)?;

    let take = |p: windows::core::PWSTR| {
        let s = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    };

    let mut out = Vec::new();
    loop {
        let mut batch: [Option<IShellItem>; 1] = [None];
        let mut fetched = 0;
        if items.Next(&mut batch, Some(&mut fetched)).is_err() || fetched == 0 {
            break;
        }
        let Some(item) = batch[0].take() else { break };
        let Ok(aumid) = item.GetDisplayName(SIGDN_PARENTRELATIVEPARSING).map(|p| take(p)) else {
            continue;
        };
        if !is_packaged(&aumid) {
            continue;
        }
        let Ok(name) = item.GetDisplayName(SIGDN_NORMALDISPLAY).map(|p| take(p)) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let target = format!(r"shell:AppsFolder\{aumid}");
        let mut entry = Item::new(name.clone(), "Microsoft Store アプリ", Action::Open(target));
        // The family name ("Microsoft.WindowsTerminal") often carries the
        // English name of a localized title.
        let family = aumid.split('_').next().unwrap_or("").replace('.', " ");
        entry.key = format!("{name} {family}");
        out.push(entry);
    }
    Ok(out)
}

/// "Publisher.App_hash!EntryPoint" (packaged) vs a path or a known-folder
/// GUID path ("{6D80...}\notepad.exe") for desktop apps.
fn is_packaged(aumid: &str) -> bool {
    aumid.contains('!') && !aumid.contains('\\')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_ids_only() {
        assert!(is_packaged("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"));
        assert!(!is_packaged(r"{6D809377-6AF0-444B-8957-A3773F02200E}\app\a.exe"));
        assert!(!is_packaged("Microsoft.Windows.Explorer"));
    }

    #[test]
    fn lists_some_store_apps() {
        // Every Windows 10/11 install ships packaged apps (Settings, etc.).
        let items = scan();
        assert!(!items.is_empty());
        assert!(items
            .iter()
            .all(|it| matches!(&it.action, Action::Open(t) if t.starts_with("shell:AppsFolder\\"))));
    }
}
