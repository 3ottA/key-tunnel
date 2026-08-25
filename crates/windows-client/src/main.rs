#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("remote-input-client only supports Windows");

mod config;
mod mapping;
mod status;
mod transport;

use config::Config;
use remote_input_protocol::{flags, KeyState};
use std::env;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Instant;
use transport::{Outbound, RawKey};
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const HOTKEY_TOGGLE_ID: i32 = 1;
const HOTKEY_EMERGENCY_ID: i32 = 2;
const WM_RIB_TOGGLE: u32 = WM_APP + 1;
const WM_RIB_EMERGENCY: u32 = WM_APP + 2;
const WM_RIB_FAIL_OPEN: u32 = WM_APP + 3;
const QUEUE_CAPACITY: usize = 4096;

static SENDER: OnceLock<SyncSender<Outbound>> = OnceLock::new();
static ACTIVE: AtomicBool = AtomicBool::new(false);
static CONNECTED: AtomicBool = AtomicBool::new(false);
static FAIL_OPEN: AtomicBool = AtomicBool::new(false);
static WAS_REMOTE: AtomicBool = AtomicBool::new(false);
static DROPPED: AtomicU64 = AtomicU64::new(0);
static START: OnceLock<Instant> = OnceLock::new();
static UI_THREAD: AtomicU32 = AtomicU32::new(0);
static TOGGLE_VK: AtomicU32 = AtomicU32::new(0);
static TOGGLE_MODS: AtomicU32 = AtomicU32::new(0);
static EMERGENCY_VK: AtomicU32 = AtomicU32::new(0);
static EMERGENCY_MODS: AtomicU32 = AtomicU32::new(0);
static TOGGLE_TRIGGER_DOWN: AtomicBool = AtomicBool::new(false);
static EMERGENCY_TRIGGER_DOWN: AtomicBool = AtomicBool::new(false);
static PRESSED: [AtomicU64; 12] = [const { AtomicU64::new(0) }; 12];

fn main() {
    if let Err(error) = run() {
        show_error(&format!("Remote Input Bridge could not start:\n{error}"));
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    START.set(Instant::now()).ok();
    let path = parse_config_arg();
    let config = Config::load(&path)?;
    let toggle = Hotkey::parse(&config.toggle_hotkey)?;
    let emergency_hotkey = Hotkey::parse(&config.emergency_hotkey)?;
    if toggle == emergency_hotkey {
        return Err("toggle_hotkey and emergency_hotkey must differ".into());
    }
    TOGGLE_VK.store(toggle.vk, Ordering::Release);
    TOGGLE_MODS.store(toggle.modifiers, Ordering::Release);
    EMERGENCY_VK.store(emergency_hotkey.vk, Ordering::Release);
    EMERGENCY_MODS.store(emergency_hotkey.modifiers, Ordering::Release);

    unsafe {
        if RegisterHotKey(
            std::ptr::null_mut(),
            HOTKEY_TOGGLE_ID,
            toggle.modifiers | MOD_NOREPEAT,
            toggle.vk,
        ) == 0
        {
            return Err(format!("toggle hotkey is unavailable: {}", config.toggle_hotkey).into());
        }
        if RegisterHotKey(
            std::ptr::null_mut(),
            HOTKEY_EMERGENCY_ID,
            emergency_hotkey.modifiers | MOD_NOREPEAT,
            emergency_hotkey.vk,
        ) == 0
        {
            UnregisterHotKey(std::ptr::null_mut(), HOTKEY_TOGGLE_ID);
            return Err(format!(
                "emergency hotkey is unavailable: {}",
                config.emergency_hotkey
            )
            .into());
        }
    }
    let hook =
        unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), std::ptr::null_mut(), 0) };
    if hook.is_null() {
        return Err("SetWindowsHookExW failed".into());
    }

    let status = Arc::new(status::Status::new());
    status.update(|s| s.state = "CONNECTING".into());
    let (tx, rx) = std::sync::mpsc::sync_channel(QUEUE_CAPACITY);
    SENDER
        .set(tx)
        .map_err(|_| "internal sender initialized twice")?;
    let transport_status = status.clone();
    let transport_config = config.clone();
    thread::spawn(move || {
        transport::run(transport_config, rx, &CONNECTED, &ACTIVE, transport_status)
    });
    UI_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
    let timer = unsafe { SetTimer(std::ptr::null_mut(), 0, 250, None) };

    let mut message: MSG = unsafe { std::mem::zeroed() };
    loop {
        if FAIL_OPEN.swap(false, Ordering::AcqRel) {
            fail_open_ui(&status, config.notify_on_toggle);
        }
        let result = unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) };
        if result <= 0 {
            break;
        }
        match message.message {
            WM_HOTKEY if message.wParam == HOTKEY_TOGGLE_ID as usize => {
                toggle_mode(&status, config.notify_on_toggle)
            }
            WM_HOTKEY if message.wParam == HOTKEY_EMERGENCY_ID as usize => {
                emergency(&status, config.notify_on_toggle)
            }
            WM_RIB_TOGGLE => toggle_mode(&status, config.notify_on_toggle),
            WM_RIB_EMERGENCY => emergency(&status, config.notify_on_toggle),
            WM_RIB_FAIL_OPEN => fail_open_ui(&status, config.notify_on_toggle),
            WM_TIMER if WAS_REMOTE.load(Ordering::Acquire) && !ACTIVE.load(Ordering::Acquire) => {
                fail_open_ui(&status, config.notify_on_toggle)
            }
            _ => unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            },
        }
    }
    ACTIVE.store(false, Ordering::Release);
    let _ = SENDER.get().unwrap().try_send(Outbound::Shutdown);
    unsafe {
        UnhookWindowsHookEx(hook);
        if timer != 0 {
            KillTimer(std::ptr::null_mut(), timer);
        }
        UnregisterHotKey(std::ptr::null_mut(), HOTKEY_TOGGLE_ID);
        UnregisterHotKey(std::ptr::null_mut(), HOTKEY_EMERGENCY_ID);
    }
    Ok(())
}

fn toggle_mode(status: &status::Status, notify: bool) {
    if ACTIVE.swap(false, Ordering::AcqRel) {
        WAS_REMOTE.store(false, Ordering::Release);
        clear_pressed();
        let _ = SENDER.get().unwrap().try_send(Outbound::ReleaseAll);
        status.update(|s| {
            s.state = "LOCAL".into();
            s.remote = false;
        });
        if notify {
            beep(0x00000040);
        }
    } else if CONNECTED.load(Ordering::Acquire) {
        clear_pressed();
        WAS_REMOTE.store(true, Ordering::Release);
        ACTIVE.store(true, Ordering::Release);
        status.update(|s| {
            s.state = "REMOTE".into();
            s.remote = true;
            s.connected = true;
        });
        if notify {
            beep(0x00000030);
        }
    } else if notify {
        beep(0x00000010);
    }
}

fn emergency(status: &status::Status, notify: bool) {
    ACTIVE.store(false, Ordering::Release);
    WAS_REMOTE.store(false, Ordering::Release);
    clear_pressed();
    let _ = SENDER.get().unwrap().try_send(Outbound::Emergency);
    status.update(|s| {
        s.state = "LOCAL".into();
        s.remote = false;
    });
    if notify {
        beep(0x00000010);
    }
}

fn fail_open_ui(status: &status::Status, notify: bool) {
    WAS_REMOTE.store(false, Ordering::Release);
    ACTIVE.store(false, Ordering::Release);
    clear_pressed();
    let _ = SENDER
        .get()
        .and_then(|sender| sender.try_send(Outbound::ReleaseAll).ok());
    status.update(|s| {
        s.remote = false;
        s.dropped_events = DROPPED.load(Ordering::Relaxed);
        if s.connected {
            s.state = "LOCAL".into();
        }
    });
    if notify {
        beep(0x00000010);
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }
    let event = &*(lparam as *const KBDLLHOOKSTRUCT);
    if event.flags & 0x10 != 0 {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }
    let down = wparam as u32 == WM_KEYDOWN || wparam as u32 == WM_SYSKEYDOWN;
    let up = wparam as u32 == WM_KEYUP || wparam as u32 == WM_SYSKEYUP;
    if !down && !up {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }
    if event.vkCode == EMERGENCY_VK.load(Ordering::Acquire) {
        if up && EMERGENCY_TRIGGER_DOWN.swap(false, Ordering::AcqRel) {
            return 1;
        }
        if down && modifiers_held(EMERGENCY_MODS.load(Ordering::Acquire)) {
            if !EMERGENCY_TRIGGER_DOWN.swap(true, Ordering::AcqRel) {
                PostThreadMessageW(UI_THREAD.load(Ordering::Acquire), WM_RIB_EMERGENCY, 0, 0);
            }
            return 1;
        }
    }
    if event.vkCode == TOGGLE_VK.load(Ordering::Acquire) {
        if up && TOGGLE_TRIGGER_DOWN.swap(false, Ordering::AcqRel) {
            return 1;
        }
        if down && modifiers_held(TOGGLE_MODS.load(Ordering::Acquire)) {
            if !TOGGLE_TRIGGER_DOWN.swap(true, Ordering::AcqRel) {
                PostThreadMessageW(UI_THREAD.load(Ordering::Acquire), WM_RIB_TOGGLE, 0, 0);
            }
            return 1;
        }
    }
    if !ACTIVE.load(Ordering::Acquire) {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }
    let extended = event.flags & 1 != 0;
    let e1_pause = event.vkCode == 0x13;
    if let Some(key) = if e1_pause {
        Some(mapping::pause())
    } else {
        mapping::to_linux(event.scanCode, extended)
    } {
        let state = key_state(key, down);
        let mut wire_flags = if extended { flags::EXTENDED_E0 } else { 0 };
        if e1_pause {
            wire_flags = flags::EXTENDED_E1;
        }
        let raw = RawKey {
            code: key,
            state,
            flags: wire_flags,
            timestamp_micros: START
                .get()
                .map(|v| v.elapsed().as_micros() as u64)
                .unwrap_or(0),
        };
        if let Some(sender) = SENDER.get() {
            if let Err(TrySendError::Full(_)) = sender.try_send(Outbound::Key(raw)) {
                DROPPED.fetch_add(1, Ordering::Relaxed);
                ACTIVE.store(false, Ordering::Release);
                FAIL_OPEN.store(true, Ordering::Release);
                PostThreadMessageW(UI_THREAD.load(Ordering::Acquire), WM_RIB_FAIL_OPEN, 0, 0);
            }
        }
    }
    1
}

fn key_state(key: u16, down: bool) -> KeyState {
    let index = usize::from(key / 64);
    let bit = 1u64 << (key % 64);
    if down {
        if PRESSED[index].fetch_or(bit, Ordering::AcqRel) & bit == 0 {
            KeyState::Down
        } else {
            KeyState::Repeat
        }
    } else {
        PRESSED[index].fetch_and(!bit, Ordering::AcqRel);
        KeyState::Up
    }
}
fn clear_pressed() {
    for word in &PRESSED {
        word.store(0, Ordering::Release);
    }
}

unsafe fn modifiers_held(modifiers: u32) -> bool {
    let held = |vk: u16| GetAsyncKeyState(i32::from(vk)) < 0;
    (modifiers & MOD_CONTROL == 0 || held(VK_CONTROL))
        && (modifiers & MOD_SHIFT == 0 || held(VK_SHIFT))
        && (modifiers & MOD_ALT == 0 || held(VK_MENU))
        && (modifiers & MOD_WIN == 0 || held(VK_LWIN) || held(VK_RWIN))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Hotkey {
    modifiers: u32,
    vk: u32,
}
impl Hotkey {
    fn parse(text: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let parts: Vec<_> = text
            .split('+')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if parts.len() < 2 {
            return Err(format!("invalid hotkey: {text}").into());
        }
        let mut modifiers = 0;
        for part in &parts[..parts.len() - 1] {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => modifiers |= MOD_CONTROL,
                "alt" => modifiers |= MOD_ALT,
                "shift" => modifiers |= MOD_SHIFT,
                "win" | "super" => modifiers |= MOD_WIN,
                _ => return Err(format!("unknown hotkey modifier: {part}").into()),
            }
        }
        let key = parts.last().unwrap().to_ascii_uppercase();
        let vk = match key.as_str() {
            "PAUSE" => 0x13,
            "INSERT" => 0x2d,
            "DELETE" => 0x2e,
            "HOME" => 0x24,
            "END" => 0x23,
            _ if key.starts_with('F') => {
                let n: u32 = key[1..].parse()?;
                if !(1..=24).contains(&n) {
                    return Err("function key must be F1..F24".into());
                }
                0x6f + n
            }
            _ if key.len() == 1 => key.as_bytes()[0] as u32,
            _ => return Err(format!("unsupported hotkey key: {key}").into()),
        };
        Ok(Self { modifiers, vk })
    }
}

fn parse_config_arg() -> std::path::PathBuf {
    let mut args = env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--config" {
            if let Some(path) = args.next() {
                return path.into();
            }
        }
    }
    config::default_config_path()
}

fn beep(kind: u32) {
    unsafe {
        MessageBeep(kind);
    }
}
fn show_error(message: &str) {
    let body: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    let title: Vec<u16> = "Remote Input Bridge"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            body.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(test)]
mod hotkey_tests {
    use super::*;

    #[test]
    fn parses_recommended_hotkeys() {
        assert_eq!(
            Hotkey::parse("Ctrl+Alt+Pause").unwrap(),
            Hotkey {
                modifiers: MOD_CONTROL | MOD_ALT,
                vk: 0x13
            }
        );
        assert_eq!(Hotkey::parse("Ctrl+Shift+F11").unwrap().vk, 0x7a);
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(Hotkey::parse("Ctrl+Banana").is_err());
    }
}
