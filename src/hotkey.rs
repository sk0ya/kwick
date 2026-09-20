use crate::winctl::WindowCtl;
use eframe::egui;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock, RwLock};
use std::thread;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, KBDLLHOOKSTRUCT, LLKHF_ALTDOWN, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP,
    WM_SYSKEYDOWN, WM_SYSKEYUP,
};

static HOOK_STATE: OnceLock<Arc<HookState>> = OnceLock::new();

/// Combines RegisterHotKey notifications with a low-level keyboard hook.
/// The hook observes the key stream before an application such as SandS can
/// rewrite it; RegisterHotKey remains as a fallback if hook installation fails.
#[derive(Clone)]
pub struct HotkeyInput {
    state: Arc<HookState>,
}

struct HookState {
    hotkey: RwLock<Option<HotKey>>,
    held_modifier_keys: AtomicU32,
    space_down: AtomicBool,
    last_hook_event: Mutex<Option<Instant>>,
    trigger_tx: mpsc::Sender<()>,
}

impl HotkeyInput {
    pub fn new(ctx: egui::Context, ctl: Arc<WindowCtl>, hotkey: Option<HotKey>) -> Self {
        let (trigger_tx, trigger_rx) = mpsc::channel();
        let state = Arc::new(HookState {
            hotkey: RwLock::new(hotkey),
            held_modifier_keys: AtomicU32::new(0),
            space_down: AtomicBool::new(false),
            last_hook_event: Mutex::new(None),
            trigger_tx,
        });

        thread::Builder::new()
            .name("kwick-hotkey-toggle".into())
            .spawn(move || {
                while trigger_rx.recv().is_ok() {
                    ctl.toggle();
                    ctx.request_repaint();
                }
            })
            .expect("failed to start hotkey toggle thread");

        let hook_state = state.clone();
        thread::Builder::new()
            .name("kwick-keyboard-hook".into())
            .spawn(move || run_keyboard_hook(hook_state))
            .expect("failed to start keyboard hook thread");

        Self { state }
    }

    pub fn set_hotkey(&self, hotkey: Option<HotKey>) {
        if let Ok(mut current) = self.state.hotkey.write() {
            *current = hotkey;
        }
        self.state.held_modifier_keys.store(0, Ordering::SeqCst);
        self.state.space_down.store(false, Ordering::SeqCst);
        if let Ok(mut last) = self.state.last_hook_event.lock() {
            *last = None;
        }
    }

    /// Route RegisterHotKey events through the same toggle worker. The hook
    /// normally sees the physical/injected key first, so suppress its paired
    /// WM_HOTKEY notification rather than toggling twice.
    pub fn on_registered_event(&self, id: u32) {
        let matches_active = self
            .state
            .hotkey
            .read()
            .ok()
            .and_then(|hotkey| hotkey.map(|hotkey| hotkey.id() == id))
            .unwrap_or(false);
        if !matches_active {
            return;
        }

        let hook_was_recent = self
            .state
            .last_hook_event
            .lock()
            .ok()
            .and_then(|last| *last)
            .is_some_and(|last| last.elapsed() < Duration::from_millis(250));
        if !hook_was_recent {
            let _ = self.state.trigger_tx.send(());
        }
    }
}

fn run_keyboard_hook(state: Arc<HookState>) {
    if HOOK_STATE.set(state).is_err() {
        eprintln!("kwick: keyboard hook state was already initialized");
        return;
    }

    let module = unsafe { GetModuleHandleW(None) };
    let hook = match module {
        Ok(module) => unsafe {
            SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(low_level_keyboard_proc),
                Some(module.into()),
                0,
            )
        },
        Err(error) => {
            eprintln!("kwick: failed to get module handle for keyboard hook: {error}");
            return;
        }
    };
    let hook = match hook {
        Ok(hook) => hook,
        Err(error) => {
            eprintln!("kwick: failed to install keyboard hook: {error}");
            return;
        }
    };

    let mut message = MSG::default();
    unsafe {
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        let _ = UnhookWindowsHookEx(hook);
    }
}

unsafe extern "system" fn low_level_keyboard_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 {
        if let Some(state) = HOOK_STATE.get() {
            let event = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            state.on_keyboard_event(wparam.0 as u32, event);
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

impl HookState {
    fn on_keyboard_event(&self, message: u32, event: &KBDLLHOOKSTRUCT) {
        let down = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN);
        let up = matches!(message, WM_KEYUP | WM_SYSKEYUP);
        if !down && !up {
            return;
        }

        let vk = event.vkCode;
        if let Some(bit) = modifier_key_bit(vk) {
            if down {
                self.held_modifier_keys.fetch_or(bit, Ordering::SeqCst);
            } else {
                self.held_modifier_keys.fetch_and(!bit, Ordering::SeqCst);
            }
            return;
        }

        if vk != 0x20 {
            return;
        }
        if up {
            self.space_down.store(false, Ordering::SeqCst);
            return;
        }

        // WM_SYSKEY* carries the current Alt state explicitly. This avoids
        // querying GetAsyncKeyState inside the hook, where Windows has not
        // necessarily updated the asynchronous state yet.
        let mut modifiers = normalized_modifiers(self.held_modifier_keys.load(Ordering::SeqCst));
        if event.flags.contains(LLKHF_ALTDOWN) {
            modifiers |= MOD_ALT;
        } else {
            modifiers &= !MOD_ALT;
        }
        if self.space_down.swap(true, Ordering::SeqCst) {
            return; // ignore keyboard auto-repeat
        }

        let Ok(hotkey) = self.hotkey.read() else {
            return;
        };
        let Some(hotkey) = *hotkey else { return };
        if hotkey.key != Code::Space || expected_modifiers(hotkey.mods) != modifiers {
            return;
        }

        if let Ok(mut last) = self.last_hook_event.lock() {
            *last = Some(Instant::now());
        }
        let _ = self.trigger_tx.send(());
    }
}

const MOD_SHIFT: u32 = 1 << 0;
const MOD_CONTROL: u32 = 1 << 1;
const MOD_ALT: u32 = 1 << 2;
const MOD_SUPER: u32 = 1 << 3;

fn modifier_key_bit(vk: u32) -> Option<u32> {
    match vk {
        0x10 => Some(1 << 0),
        0xA0 => Some(1 << 1),
        0xA1 => Some(1 << 2),
        0x11 => Some(1 << 3),
        0xA2 => Some(1 << 4),
        0xA3 => Some(1 << 5),
        0x12 => Some(1 << 6),
        0xA4 => Some(1 << 7),
        0xA5 => Some(1 << 8),
        0x5B => Some(1 << 9),
        0x5C => Some(1 << 10),
        _ => None,
    }
}

fn normalized_modifiers(held: u32) -> u32 {
    let mut modifiers = 0;
    if held & ((1 << 0) | (1 << 1) | (1 << 2)) != 0 {
        modifiers |= MOD_SHIFT;
    }
    if held & ((1 << 3) | (1 << 4) | (1 << 5)) != 0 {
        modifiers |= MOD_CONTROL;
    }
    if held & ((1 << 6) | (1 << 7) | (1 << 8)) != 0 {
        modifiers |= MOD_ALT;
    }
    if held & ((1 << 9) | (1 << 10)) != 0 {
        modifiers |= MOD_SUPER;
    }
    modifiers
}

fn expected_modifiers(modifiers: Modifiers) -> u32 {
    let mut expected = 0;
    if modifiers.contains(Modifiers::SHIFT) {
        expected |= MOD_SHIFT;
    }
    if modifiers.contains(Modifiers::CONTROL) {
        expected |= MOD_CONTROL;
    }
    if modifiers.contains(Modifiers::ALT) {
        expected |= MOD_ALT;
    }
    if modifiers.intersects(Modifiers::SUPER | Modifiers::META) {
        expected |= MOD_SUPER;
    }
    expected
}

#[cfg(test)]
mod tests {
    use super::{
        expected_modifiers, modifier_key_bit, normalized_modifiers, HookState, HotkeyInput,
        MOD_ALT, MOD_SHIFT, MOD_SUPER,
    };
    use global_hotkey::hotkey::{HotKey, Modifiers};
    use std::sync::atomic::{AtomicBool, AtomicU32};
    use std::sync::{mpsc, Mutex, RwLock};
    use windows::Win32::UI::WindowsAndMessaging::{
        KBDLLHOOKSTRUCT, LLKHF_ALTDOWN, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN,
    };

    #[test]
    fn tracks_left_and_right_modifier_keys_independently() {
        assert_eq!(modifier_key_bit(0xA0), Some(1 << 1));
        assert_eq!(modifier_key_bit(0xA1), Some(1 << 2));
        assert_eq!(modifier_key_bit(0xA2), Some(1 << 4));
        assert_eq!(modifier_key_bit(0xA3), Some(1 << 5));
        assert_eq!(modifier_key_bit(0xA4), Some(1 << 7));
        assert_eq!(modifier_key_bit(0xA5), Some(1 << 8));
        assert_eq!(normalized_modifiers((1 << 1) | (1 << 2)), MOD_SHIFT);
    }

    #[test]
    fn translates_configured_modifiers_to_hook_state() {
        let modifiers = Modifiers::ALT | Modifiers::SHIFT | Modifiers::SUPER;
        assert_eq!(
            expected_modifiers(modifiers),
            MOD_ALT | MOD_SHIFT | MOD_SUPER
        );
    }

    #[test]
    fn detects_alt_space_and_suppresses_its_registered_duplicate() {
        let hotkey: HotKey = "alt+space".parse().unwrap();
        let (trigger_tx, trigger_rx) = mpsc::channel();
        let state = std::sync::Arc::new(HookState {
            hotkey: RwLock::new(Some(hotkey)),
            held_modifier_keys: AtomicU32::new(0),
            space_down: AtomicBool::new(false),
            last_hook_event: Mutex::new(None),
            trigger_tx,
        });

        state.on_keyboard_event(
            WM_SYSKEYDOWN,
            &KBDLLHOOKSTRUCT {
                vkCode: 0x12,
                ..Default::default()
            },
        );
        state.on_keyboard_event(
            WM_KEYDOWN,
            &KBDLLHOOKSTRUCT {
                vkCode: 0x20,
                flags: LLKHF_ALTDOWN,
                ..Default::default()
            },
        );
        assert!(trigger_rx.try_recv().is_ok());

        let input = HotkeyInput { state };
        input.on_registered_event(hotkey.id());
        assert!(trigger_rx.try_recv().is_err());

        input.state.on_keyboard_event(
            WM_KEYUP,
            &KBDLLHOOKSTRUCT {
                vkCode: 0x20,
                ..Default::default()
            },
        );
        input.state.on_keyboard_event(
            WM_KEYUP,
            &KBDLLHOOKSTRUCT {
                vkCode: 0x12,
                ..Default::default()
            },
        );
        input.state.on_keyboard_event(
            WM_KEYDOWN,
            &KBDLLHOOKSTRUCT {
                vkCode: 0x20,
                ..Default::default()
            },
        );
        assert!(trigger_rx.try_recv().is_err());
    }
}
