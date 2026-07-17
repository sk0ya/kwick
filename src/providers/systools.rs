use super::{Action, Item};
use std::path::PathBuf;

/// Curated, commonly used Windows tools. Most of these have no Start Menu
/// shortcut (or are buried in "Windows Tools"), and scanning PATH for them
/// drags in hundreds of unrelated exes — so we list them explicitly.
/// The key carries English/Japanese aliases so both spellings match.
pub fn scan() -> Vec<Item> {
    let windir =
        PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into()));
    let sys32 = windir.join("System32");

    // (title, match aliases, file relative to System32)
    const SYS32_TOOLS: &[(&str, &str, &str)] = &[
        ("リモート デスクトップ接続", "remote desktop connection mstsc rdp", "mstsc.exe"),
        ("タスク マネージャー", "task manager taskmgr", "taskmgr.exe"),
        ("タスク スケジューラ", "task scheduler taskschd", "taskschd.msc"),
        ("デバイス マネージャー", "device manager devmgmt", "devmgmt.msc"),
        ("サービス", "services", "services.msc"),
        ("イベント ビューアー", "event viewer eventvwr", "eventvwr.msc"),
        ("ディスクの管理", "disk management diskmgmt", "diskmgmt.msc"),
        ("コンピューターの管理", "computer management compmgmt", "compmgmt.msc"),
        ("コントロール パネル", "control panel", "control.exe"),
        ("コマンド プロンプト", "command prompt cmd", "cmd.exe"),
        ("システム構成", "system configuration msconfig", "msconfig.exe"),
        ("リソース モニター", "resource monitor resmon", "resmon.exe"),
        ("システム情報", "system information msinfo32", "msinfo32.exe"),
        ("メモ帳", "notepad", "notepad.exe"),
        ("電卓", "calculator calc", "calc.exe"),
        ("ペイント", "paint mspaint", "mspaint.exe"),
        ("Snipping Tool", "snipping 切り取り screenshot", "SnippingTool.exe"),
        (
            "PowerShell",
            "powershell",
            r"WindowsPowerShell\v1.0\powershell.exe",
        ),
    ];

    let mut items = Vec::new();
    for (title, aliases, file) in SYS32_TOOLS {
        push_file(&mut items, title, aliases, sys32.join(file));
    }
    push_file(
        &mut items,
        "レジストリ エディター",
        "registry editor regedit",
        windir.join("regedit.exe"),
    );

    // Control Panel applets: no "open" verb on .cpl, so go through control.exe.
    const CPL_TOOLS: &[(&str, &str, &str)] = &[
        (
            "プログラムと機能",
            "programs features uninstall appwiz アンインストール",
            "appwiz.cpl",
        ),
        ("ネットワーク接続", "network connections ncpa", "ncpa.cpl"),
        ("システムのプロパティ", "system properties sysdm", "sysdm.cpl"),
    ];
    for (title, aliases, cpl) in CPL_TOOLS {
        let cpl_path = sys32.join(cpl);
        if !cpl_path.exists() {
            continue;
        }
        let mut item = Item::new(
            *title,
            cpl_path.display().to_string(),
            Action::Exec {
                cmd: sys32.join("control.exe").display().to_string(),
                args: Some((*cpl).to_string()),
            },
        );
        item.icon_path = Some(cpl_path.display().to_string());
        finish(&mut item, title, aliases);
        items.push(item);
    }

    let mut env = Item::new(
        "環境変数の編集",
        "システム環境変数を編集",
        Action::Exec {
            cmd: sys32.join("rundll32.exe").display().to_string(),
            args: Some("sysdm.cpl,EditEnvironmentVariables".into()),
        },
    );
    env.icon_path = Some(sys32.join("sysdm.cpl").display().to_string());
    finish(&mut env, "環境変数の編集", "environment variables env");
    items.push(env);

    let mut settings = Item::new("設定", "Windows の設定", Action::Open("ms-settings:".into()));
    finish(&mut settings, "設定", "settings ms-settings");
    items.push(settings);

    items
}

/// Power / session commands (shutdown, restart, sleep...). Gated by
/// `system_commands` in config.toml (default: true).
pub fn power_items() -> Vec<Item> {
    let windir =
        PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into()));
    let sys32 = windir.join("System32");
    let shutdown = sys32.join("shutdown.exe").display().to_string();
    let rundll = sys32.join("rundll32.exe").display().to_string();

    // (title, subtitle, match aliases, cmd, args)
    let entries: &[(&str, &str, &str, &str, &str)] = &[
        (
            "シャットダウン",
            "PC の電源を切る",
            "shutdown power off",
            &shutdown,
            "/s /t 0",
        ),
        ("再起動", "PC を再起動する", "restart reboot", &shutdown, "/r /t 0"),
        (
            "スリープ",
            "PC をスリープ状態にする",
            "sleep suspend",
            &rundll,
            "powrprof.dll,SetSuspendState 0,1,0",
        ),
        ("休止状態", "PC を休止状態にする", "hibernate", &shutdown, "/h"),
        (
            "サインアウト",
            "現在のユーザーをサインアウトする",
            "sign out signout logoff logout",
            &shutdown,
            "/l",
        ),
        (
            "ロック",
            "画面をロックする",
            "lock workstation",
            &rundll,
            "user32.dll,LockWorkStation",
        ),
    ];

    let mut items = Vec::new();
    for (title, subtitle, aliases, cmd, args) in entries {
        let mut item = Item::new(
            *title,
            *subtitle,
            Action::Exec {
                cmd: (*cmd).to_string(),
                args: Some((*args).to_string()),
            },
        );
        finish(&mut item, title, aliases);
        items.push(item);
    }
    items
}

fn push_file(items: &mut Vec<Item>, title: &str, aliases: &str, path: PathBuf) {
    if !path.exists() {
        return;
    }
    let p = path.display().to_string();
    let mut item = Item::new(title, p.clone(), Action::Open(p));
    finish(&mut item, title, aliases);
    items.push(item);
}

fn finish(item: &mut Item, title: &str, aliases: &str) {
    item.key = format!("{title} {aliases}");
    // Same boost as Start Menu apps: curated tools outrank raw PATH exes.
    item.rank_boost = 40;
}
