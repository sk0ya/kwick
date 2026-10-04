//! Small system operations: volume, recycle bin, IP addresses.

use super::{Action, Item};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SysOp {
    /// Set the master volume to this percentage
    Volume(u8),
    /// Change the master volume by this many percentage points
    VolumeStep(i8),
    ToggleMute,
    EmptyRecycleBin,
}

/// Fixed items, listed with the power commands (`system_commands`).
pub fn items() -> Vec<Item> {
    let mut out = Vec::new();
    for (title, subtitle, aliases, op) in [
        ("音量ミュート切り替え", "ミュート / 解除", "mute volume ミュート", SysOp::ToggleMute),
        ("音量を上げる", "+10%", "volume up 音量", SysOp::VolumeStep(10)),
        ("音量を下げる", "-10%", "volume down 音量", SysOp::VolumeStep(-10)),
        (
            "ゴミ箱を空にする",
            "確認ダイアログが出ます",
            "empty recycle bin trash ごみばこ",
            SysOp::EmptyRecycleBin,
        ),
    ] {
        let mut item = Item::new(title, subtitle, Action::System(op));
        item.key = format!("{title} {aliases}");
        out.push(item);
    }
    out
}

/// Answers typed as commands: "vol 30" / "音量 30", "ip".
pub fn query(q: &str) -> Vec<Item> {
    let lower = q.trim().to_lowercase();
    let volume = ["vol ", "volume ", "音量 ", "音量"]
        .iter()
        .find_map(|p| lower.strip_prefix(p))
        .and_then(|rest| rest.trim().trim_end_matches('%').parse::<u8>().ok());
    if let Some(v) = volume.filter(|v| *v <= 100) {
        let current = audio::get().map(|(level, _)| format!("現在 {level}%")).unwrap_or_default();
        return vec![Item::new(
            format!("音量を {v}% にする"),
            current,
            Action::System(SysOp::Volume(v)),
        )
        .transient()];
    }
    if matches!(lower.as_str(), "ip" | "ipconfig" | "ip address" | "ipアドレス") {
        return ip_items();
    }
    Vec::new()
}

pub fn run(op: SysOp) {
    match op {
        SysOp::Volume(v) => audio::set(v as f32 / 100.0),
        SysOp::VolumeStep(step) => {
            if let Some((level, _)) = audio::get() {
                let target = (level as i32 + step as i32).clamp(0, 100);
                audio::set(target as f32 / 100.0);
            }
        }
        SysOp::ToggleMute => audio::toggle_mute(),
        SysOp::EmptyRecycleBin => unsafe {
            use windows::Win32::UI::Shell::{SHEmptyRecycleBinW, SHERB_NOSOUND};
            // Without SHERB_NOCONFIRMATION Windows asks first.
            let _ = SHEmptyRecycleBinW(None, windows::core::PCWSTR::null(), SHERB_NOSOUND);
        },
    }
}

mod audio {
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };

    fn endpoint() -> Option<IAudioEndpointVolume> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let devices: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let device = devices.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
            device.Activate(CLSCTX_ALL, None).ok()
        }
    }

    /// (level in percent, muted)
    pub fn get() -> Option<(u8, bool)> {
        let ep = endpoint()?;
        unsafe {
            let level = ep.GetMasterVolumeLevelScalar().ok()?;
            let muted = ep.GetMute().ok()?.as_bool();
            Some(((level * 100.0).round() as u8, muted))
        }
    }

    pub fn set(level: f32) {
        if let Some(ep) = endpoint() {
            unsafe {
                let _ = ep.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), std::ptr::null());
                if level > 0.0 {
                    let _ = ep.SetMute(false, std::ptr::null());
                }
            }
        }
    }

    pub fn toggle_mute() {
        if let Some(ep) = endpoint() {
            unsafe {
                if let Ok(muted) = ep.GetMute() {
                    let _ = ep.SetMute(!muted.as_bool(), std::ptr::null());
                }
            }
        }
    }
}

/// The machine's addresses on connected adapters (Enter copies one).
fn ip_items() -> Vec<Item> {
    addresses()
        .into_iter()
        .map(|(adapter, ip)| {
            Item::new(ip.clone(), format!("{adapter}  (Enter でコピー)"), Action::Copy(ip))
                .transient()
        })
        .collect()
}

fn addresses() -> Vec<(String, String)> {
    use std::net::{Ipv4Addr, Ipv6Addr};
    use windows::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
        GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows::Win32::Networking::WinSock::IpSuffixOriginRandom;
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR_IN, SOCKADDR_IN6};

    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    let mut size: u32 = 16 * 1024;
    let mut buf: Vec<u64> = Vec::new(); // u64 for alignment
    for _ in 0..3 {
        buf.resize((size as usize).div_ceil(8), 0);
        let ret = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC.0 as u32,
                flags,
                None,
                Some(buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH),
                &mut size,
            )
        };
        if ret == 0 {
            break;
        }
        if ret != 111 {
            // anything but ERROR_BUFFER_OVERFLOW
            return Vec::new();
        }
    }

    let mut out = Vec::new();
    unsafe {
        let mut adapter = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        while !adapter.is_null() {
            let a = &*adapter;
            if a.OperStatus == IfOperStatusUp && a.IfType != 24 {
                // 24 = loopback
                let name = a.FriendlyName.to_string().unwrap_or_default();
                let mut unicast = a.FirstUnicastAddress;
                while !unicast.is_null() {
                    let sockaddr = (*unicast).Address.lpSockaddr;
                    // IPv6 privacy addresses rotate constantly; the stable one is enough.
                    let temporary = (*unicast).SuffixOrigin == IpSuffixOriginRandom;
                    if !sockaddr.is_null() && !temporary {
                        let family = (*sockaddr).sa_family;
                        if family == AF_INET {
                            let v4 = &*(sockaddr as *const SOCKADDR_IN);
                            let ip = Ipv4Addr::from(u32::from_be(v4.sin_addr.S_un.S_addr));
                            if !ip.is_link_local() {
                                out.push((name.clone(), ip.to_string()));
                            }
                        } else if family == AF_INET6 {
                            let v6 = &*(sockaddr as *const SOCKADDR_IN6);
                            let ip = Ipv6Addr::from(v6.sin6_addr.u.Byte);
                            // Skip fe80:: link-local addresses.
                            if ip.segments()[0] & 0xffc0 != 0xfe80 {
                                out.push((name.clone(), ip.to_string()));
                            }
                        }
                    }
                    unicast = (*unicast).Next;
                }
            }
            adapter = a.Next;
        }
    }
    // Physical adapters before virtual switches/VPN, IPv4 before IPv6:
    // that is what people usually want to copy.
    let is_virtual = |name: &str| {
        let n = name.to_lowercase();
        ["vethernet", "virtual", "vmware", "wsl", "hyper-v", "vpn", "tailscale"]
            .iter()
            .any(|k| n.contains(k))
    };
    out.sort_by_key(|(name, ip)| (is_virtual(name), ip.contains(':')));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_query_parses() {
        let items = query("vol 30");
        assert!(matches!(items[0].action, Action::System(SysOp::Volume(30))));
        assert!(matches!(query("音量 5%")[0].action, Action::System(SysOp::Volume(5))));
        assert!(query("vol 300").is_empty());
        assert!(query("volume mixer").is_empty());
    }

    #[test]
    fn lists_addresses() {
        // Any test machine has at least one connected adapter.
        let addrs = addresses();
        assert!(addrs.iter().all(|(_, ip)| !ip.starts_with("127.")));
    }
}
