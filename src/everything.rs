//! File search through Everything (voidtools), over its IPC protocol
//! (WM_COPYDATA, the same messages the Everything SDK sends). Nothing is
//! bundled: when Everything is not running, the mode says so.

use crate::providers::{Action, Item};
use eframe::egui;
use std::sync::mpsc::{self, Receiver, Sender};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Everything 1.4 / 1.5 alpha window classes.
const CLASSES: [PCWSTR; 2] = [
    w!("EVERYTHING_TASKBAR_NOTIFICATION"),
    w!("EVERYTHING_TASKBAR_NOTIFICATION_(1.5a)"),
];
const COPYDATA_QUERY2W: usize = 18;
const REQUEST_NAME: u32 = 0x1;
const REQUEST_PATH: u32 = 0x2;
const SORT_NAME_ASCENDING: u32 = 1;
const ITEM_FOLDER: u32 = 0x1;
/// Our id for the reply's COPYDATASTRUCT.dwData.
const REPLY_ID: usize = 0x4B57_4943;
const MAX_RESULTS: u32 = 50;

pub type Answer = (String, Result<Vec<Item>, String>);

/// Runs queries on a worker thread that owns the reply window; only the
/// newest pending query is sent.
pub struct Everything {
    tx: Sender<String>,
    rx: Receiver<Answer>,
}

impl Everything {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, worker_rx) = mpsc::channel::<String>();
        let (worker_tx, rx) = mpsc::channel::<Answer>();
        std::thread::Builder::new()
            .name("kwick-everything".into())
            .spawn(move || {
                while let Ok(mut query) = worker_rx.recv() {
                    // Skip queries that were superseded while we were busy.
                    while let Ok(newer) = worker_rx.try_recv() {
                        query = newer;
                    }
                    let result = search(&CLASSES, &query, MAX_RESULTS);
                    if worker_tx.send((query, result)).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .ok();
        Self { tx, rx }
    }

    pub fn request(&self, query: &str) {
        let _ = self.tx.send(query.to_string());
    }

    /// The most recent answer that arrived since the last poll.
    pub fn poll(&self) -> Option<Answer> {
        self.rx.try_iter().last()
    }
}

/// One query: find the Everything window, send QUERY2, wait for the reply.
pub fn search(classes: &[PCWSTR], query: &str, max: u32) -> Result<Vec<Item>, String> {
    let target = classes
        .iter()
        .find_map(|class| unsafe { FindWindowW(*class, PCWSTR::null()) }.ok())
        .filter(|h| !h.is_invalid())
        .ok_or("Everything が起動していません")?;
    let reply = ReplyWindow::create()?;

    // EVERYTHING_IPC_QUERY2 followed by the search text.
    let mut buf: Vec<u8> = Vec::new();
    for v in [
        reply.hwnd.0 as usize as u32,
        REPLY_ID as u32,
        0, // search flags
        0, // offset
        max,
        REQUEST_NAME | REQUEST_PATH,
        SORT_NAME_ASCENDING,
    ] {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    for unit in query.encode_utf16().chain(std::iter::once(0)) {
        buf.extend_from_slice(&unit.to_le_bytes());
    }
    let cds = COPYDATASTRUCT {
        dwData: COPYDATA_QUERY2W,
        cbData: buf.len() as u32,
        lpData: buf.as_mut_ptr() as *mut _,
    };
    let mut result = 0usize;
    let sent = unsafe {
        SendMessageTimeoutW(
            target,
            WM_COPYDATA,
            WPARAM(reply.hwnd.0 as usize),
            LPARAM(&cds as *const _ as isize),
            SMTO_ABORTIFHUNG,
            2000,
            Some(&mut result),
        )
    };
    if sent.0 == 0 || result == 0 {
        return Err("Everything に問い合わせできませんでした".into());
    }
    let list = reply.wait(std::time::Duration::from_secs(3))?;
    parse_list(&list)
}

thread_local! {
    static REPLY: std::cell::RefCell<Option<Vec<u8>>> = const { std::cell::RefCell::new(None) };
}

struct ReplyWindow {
    hwnd: HWND,
}

impl ReplyWindow {
    fn create() -> Result<Self, String> {
        unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
            if msg == WM_COPYDATA {
                let cds = &*(lp.0 as *const COPYDATASTRUCT);
                if cds.dwData == REPLY_ID && !cds.lpData.is_null() {
                    let data =
                        std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize)
                            .to_vec();
                    REPLY.with(|r| *r.borrow_mut() = Some(data));
                    return LRESULT(1);
                }
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        unsafe {
            let instance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
                .map_err(|e| e.to_string())?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                lpszClassName: w!("KwickEverythingReply"),
                ..Default::default()
            };
            RegisterClassW(&class); // fails harmlessly when already registered
            REPLY.with(|r| *r.borrow_mut() = None);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("KwickEverythingReply"),
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
            )
            .map_err(|e| e.to_string())?;
            Ok(Self { hwnd })
        }
    }

    /// Pump messages until the reply arrives (Everything answers with a
    /// WM_COPYDATA of its own once the search is done).
    fn wait(&self, timeout: std::time::Duration) -> Result<Vec<u8>, String> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Some(data) = REPLY.with(|r| r.borrow_mut().take()) {
                return Ok(data);
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return Err("Everything から応答がありません".into());
            }
            unsafe {
                let left = (deadline - now).as_millis() as u32;
                windows::Win32::UI::WindowsAndMessaging::MsgWaitForMultipleObjects(
                    None,
                    false,
                    left.min(100),
                    QS_ALLINPUT,
                );
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    DispatchMessageW(&msg);
                }
            }
        }
    }
}

impl Drop for ReplyWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn read_u32(data: &[u8], at: usize) -> Option<u32> {
    data.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// A length-prefixed, NUL-terminated UTF-16 string; returns it and the
/// offset just past it.
fn read_text(data: &[u8], at: usize) -> Option<(String, usize)> {
    let len = read_u32(data, at)? as usize;
    let start = at + 4;
    let bytes = data.get(start..start + len * 2)?;
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some((String::from_utf16_lossy(&units), start + (len + 1) * 2))
}

/// EVERYTHING_IPC_LIST2: header (totitems, numitems, offset, request_flags,
/// sort_type), numitems × (flags, data_offset), then each item's requested
/// fields in bit order.
pub fn parse_list(data: &[u8]) -> Result<Vec<Item>, String> {
    let bad = || "Everything の応答を解釈できません".to_string();
    let count = read_u32(data, 4).ok_or_else(bad)? as usize;
    let flags = read_u32(data, 12).ok_or_else(bad)?;
    if flags & (REQUEST_NAME | REQUEST_PATH) != REQUEST_NAME | REQUEST_PATH {
        return Err(bad());
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let entry = 20 + i * 8;
        let item_flags = read_u32(data, entry).ok_or_else(bad)?;
        let offset = read_u32(data, entry + 4).ok_or_else(bad)? as usize;
        let (name, next) = read_text(data, offset).ok_or_else(bad)?;
        let (path, _) = read_text(data, next).ok_or_else(bad)?;
        let full = format!("{}\\{name}", path.trim_end_matches('\\'));
        let subtitle = if item_flags & ITEM_FOLDER != 0 {
            format!("{path}  (フォルダ)")
        } else {
            path.clone()
        };
        let mut item = Item::new(name, subtitle, Action::Open(full.clone()));
        item.icon_path = Some(full);
        out.push(item.transient());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a LIST2 reply the way Everything does.
    fn list2(entries: &[(&str, &str, bool)]) -> Vec<u8> {
        let mut head = Vec::new();
        let n = entries.len() as u32;
        for v in [n, n, 0, REQUEST_NAME | REQUEST_PATH, SORT_NAME_ASCENDING] {
            head.extend_from_slice(&v.to_le_bytes());
        }
        let mut body = Vec::new();
        let data_start = 20 + entries.len() * 8;
        for (name, path, folder) in entries {
            head.extend_from_slice(&(*folder as u32).to_le_bytes());
            head.extend_from_slice(&((data_start + body.len()) as u32).to_le_bytes());
            for text in [name, path] {
                let units: Vec<u16> = text.encode_utf16().collect();
                body.extend_from_slice(&(units.len() as u32).to_le_bytes());
                for u in units.iter().chain(std::iter::once(&0)) {
                    body.extend_from_slice(&u.to_le_bytes());
                }
            }
        }
        head.extend(body);
        head
    }

    #[test]
    fn parses_reply() {
        let data = list2(&[("メモ.txt", r"C:\Users\a", false), ("src", r"D:\", true)]);
        let items = parse_list(&data).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "メモ.txt");
        assert!(matches!(&items[0].action, Action::Open(p) if p == r"C:\Users\a\メモ.txt"));
        assert!(matches!(&items[1].action, Action::Open(p) if p == r"D:\src"));
        assert!(items[1].subtitle.contains("フォルダ"));
        assert!(parse_list(&data[..30]).is_err());
    }

    /// A stand-in Everything window: answers QUERY2 with a fixed list,
    /// echoing the search text as the first name.
    #[test]
    fn round_trip_with_fake_everything() {
        unsafe extern "system" fn fake(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
            if msg == WM_COPYDATA {
                let cds = &*(lp.0 as *const COPYDATASTRUCT);
                if cds.dwData != COPYDATA_QUERY2W {
                    return LRESULT(0);
                }
                let q = std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize);
                let reply_hwnd = read_u32(q, 0).unwrap() as usize;
                let reply_id = read_u32(q, 4).unwrap() as usize;
                let units: Vec<u16> = q[28..]
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .take_while(|&u| u != 0)
                    .collect();
                let text = String::from_utf16_lossy(&units);
                let mut data = list2(&[(&text, r"C:\x", false)]);
                let reply = COPYDATASTRUCT {
                    dwData: reply_id,
                    cbData: data.len() as u32,
                    lpData: data.as_mut_ptr() as *mut _,
                };
                // Everything replies asynchronously; a direct send works too.
                SendMessageW(
                    HWND(reply_hwnd as *mut _),
                    WM_COPYDATA,
                    Some(WPARAM(hwnd.0 as usize)),
                    Some(LPARAM(&reply as *const _ as isize)),
                );
                return LRESULT(1);
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        let (ready_tx, ready_rx) = mpsc::channel();
        std::thread::spawn(move || unsafe {
            let instance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap();
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(fake),
                hInstance: instance.into(),
                lpszClassName: w!("KwickFakeEverything"),
                ..Default::default()
            });
            let _hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("KwickFakeEverything"),
                w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .unwrap();
            ready_tx.send(()).unwrap();
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                DispatchMessageW(&msg);
            }
        });
        ready_rx.recv().unwrap();
        let items = search(&[w!("KwickFakeEverything")], "report 2026", 10).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "report 2026");

        assert!(search(&[w!("KwickNoSuchWindow")], "x", 10).is_err());
    }
}
