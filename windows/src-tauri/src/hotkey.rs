// A global shortcut that opens Mochi's chat from anywhere. It uses Win32
// RegisterHotKey directly, so there is no extra dependency, and nothing is read
// from the keyboard except that one combination: Windows only wakes us when it is
// pressed.
//
// RegisterHotKey ties a registration to the thread that made it, and that
// thread has to pump messages to hear WM_HOTKEY. So one small thread owns the
// registration for the life of the app and is told what to register through a
// channel plus a wake-up message.

use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use tauri::{AppHandle, Emitter};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
    MOD_SHIFT, MOD_WIN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, PeekMessageW, PostThreadMessageW, MSG, PM_NOREMOVE, WM_APP, WM_HOTKEY, WM_USER,
};

pub const DEFAULT: &str = "Ctrl+Alt+M";

const HOTKEY_ID: i32 = 0x4D43;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Combo {
    ctrl: bool,
    alt: bool,
    shift: bool,
    win: bool,
    vk: u32,
}

impl Combo {
    fn modifiers(&self) -> HOT_KEY_MODIFIERS {
        let mut m = MOD_NOREPEAT;
        if self.ctrl {
            m |= MOD_CONTROL;
        }
        if self.alt {
            m |= MOD_ALT;
        }
        if self.shift {
            m |= MOD_SHIFT;
        }
        if self.win {
            m |= MOD_WIN;
        }
        m
    }
}

/// "ctrl + alt + m" → the combination. At least one modifier is required, so a
/// plain letter can never be taken away from the keyboard.
pub fn parse(text: &str) -> Result<Combo, String> {
    let mut combo = Combo { ctrl: false, alt: false, shift: false, win: false, vk: 0 };
    let mut key: Option<u32> = None;
    for raw in text.split('+') {
        let token = raw.trim().to_ascii_lowercase();
        match token.as_str() {
            "" => return Err("Empty key in the shortcut.".into()),
            "ctrl" | "control" => combo.ctrl = true,
            "alt" => combo.alt = true,
            "shift" => combo.shift = true,
            "win" | "super" | "meta" | "cmd" => combo.win = true,
            other => {
                if key.is_some() {
                    return Err("Use only one key besides Ctrl, Alt, Shift and Win.".into());
                }
                key = Some(key_code(other).ok_or_else(|| format!("Unknown key: {raw}"))?);
            }
        }
    }
    combo.vk = key.ok_or("Add a key besides the modifiers.")?;
    if !(combo.ctrl || combo.alt || combo.shift || combo.win) {
        return Err("Add at least one of Ctrl, Alt, Shift or Win.".into());
    }
    Ok(combo)
}

fn key_code(token: &str) -> Option<u32> {
    let bytes = token.as_bytes();
    if bytes.len() == 1 && bytes[0].is_ascii_alphanumeric() {
        return Some(bytes[0].to_ascii_uppercase() as u32); // VK_A..VK_Z, VK_0..VK_9
    }
    if let Some(n) = token.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
        if (1..=24).contains(&n) {
            return Some(0x70 + n - 1); // VK_F1..
        }
    }
    match token {
        "space" => Some(0x20),
        "enter" | "return" => Some(0x0D),
        "tab" => Some(0x09),
        _ => None,
    }
}

fn key_name(vk: u32) -> String {
    match vk {
        0x20 => "Space".into(),
        0x0D => "Enter".into(),
        0x09 => "Tab".into(),
        0x70..=0x87 => format!("F{}", vk - 0x70 + 1),
        v => (v as u8 as char).to_string(),
    }
}

/// The spelling stored in settings and shown in the UI.
pub fn canonical(combo: &Combo) -> String {
    let mut parts: Vec<String> = Vec::new();
    if combo.ctrl {
        parts.push("Ctrl".into());
    }
    if combo.alt {
        parts.push("Alt".into());
    }
    if combo.shift {
        parts.push("Shift".into());
    }
    if combo.win {
        parts.push("Win".into());
    }
    parts.push(key_name(combo.vk));
    parts.join("+")
}

struct Request {
    combo: Option<Combo>,
    reply: SyncSender<Result<(), String>>,
}

struct Handle {
    tx: Mutex<Sender<Request>>,
    thread_id: u32,
}

static HANDLE: OnceLock<Handle> = OnceLock::new();

/// Starts the listener thread and registers `initial` (empty = no shortcut).
pub fn start(app: AppHandle, initial: &str) {
    let (tx, rx) = mpsc::channel::<Request>();
    let (ready_tx, ready_rx) = mpsc::sync_channel::<u32>(1);

    std::thread::spawn(move || listen(app, rx, ready_tx));

    let Ok(thread_id) = ready_rx.recv_timeout(Duration::from_secs(5)) else {
        crate::log::line("hotkey: listener did not start");
        return;
    };
    let _ = HANDLE.set(Handle { tx: Mutex::new(tx), thread_id });
    if !initial.trim().is_empty() {
        if let Err(err) = set(initial) {
            crate::log::line(format!("hotkey: could not register {initial}: {err}"));
        }
    }
}

/// Changes the shortcut. Empty text removes it. Returns the canonical spelling.
pub fn set(text: &str) -> Result<String, String> {
    let combo = if text.trim().is_empty() { None } else { Some(parse(text)?) };
    let handle = HANDLE.get().ok_or("The shortcut listener is not running.")?;
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    handle
        .tx
        .lock()
        .unwrap()
        .send(Request { combo, reply: reply_tx })
        .map_err(|_| "The shortcut listener stopped.".to_string())?;
    unsafe {
        let _ = PostThreadMessageW(handle.thread_id, WM_APP, Default::default(), Default::default());
    }
    reply_rx
        .recv_timeout(Duration::from_secs(3))
        .map_err(|_| "The shortcut listener did not answer.".to_string())??;
    Ok(combo.map(|c| canonical(&c)).unwrap_or_default())
}

fn listen(app: AppHandle, rx: Receiver<Request>, ready: SyncSender<u32>) {
    // Registered shortcut, remembered so a refused new one can be rolled back.
    let mut current: Option<Combo> = None;

    unsafe {
        // Touching the queue creates it, so PostThreadMessage can reach us.
        let mut msg = MSG::default();
        let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
        let _ = ready.send(GetCurrentThreadId());

        loop {
            let got = GetMessageW(&mut msg, None, 0, 0).0;
            if got <= 0 {
                break;
            }
            match msg.message {
                WM_HOTKEY => {
                    let _ = app.emit_to(crate::island::WINDOW_LABEL, "tray", "chat");
                }
                WM_APP => {
                    while let Ok(req) = rx.try_recv() {
                        let _ = req.reply.send(apply(&mut current, req.combo));
                    }
                }
                _ => {}
            }
        }
    }
}

unsafe fn apply(current: &mut Option<Combo>, wanted: Option<Combo>) -> Result<(), String> {
    let previous = current.take();
    if previous.is_some() {
        let _ = UnregisterHotKey(None, HOTKEY_ID);
    }
    let Some(next) = wanted else { return Ok(()) };
    match RegisterHotKey(None, HOTKEY_ID, next.modifiers(), next.vk) {
        Ok(()) => {
            *current = Some(next);
            Ok(())
        }
        Err(_) => {
            // Refused (taken by another app): put the old one back.
            if let Some(old) = previous {
                if RegisterHotKey(None, HOTKEY_ID, old.modifiers(), old.vk).is_ok() {
                    *current = Some(old);
                }
            }
            Err(format!(
            "{} is already used by another app. Try a different combination.",
                canonical(&next)
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{canonical, parse};

    #[test]
    fn parses_and_normalises() {
        let c = parse("ctrl + alt + m").unwrap();
        assert_eq!(canonical(&c), "Ctrl+Alt+M");
        assert_eq!(canonical(&parse("Shift+Win+f5").unwrap()), "Shift+Win+F5");
        assert_eq!(canonical(&parse("control+space").unwrap()), "Ctrl+Space");
    }

    #[test]
    fn refuses_unsafe_or_malformed_shortcuts() {
        assert!(parse("M").is_err(), "a bare letter would steal typing");
        assert!(parse("Ctrl+Alt").is_err(), "no key");
        assert!(parse("Ctrl+A+B").is_err(), "two keys");
        assert!(parse("Ctrl+Banana").is_err(), "unknown key");
        assert!(parse("Ctrl++").is_err(), "empty key");
        assert!(parse("Ctrl+F25").is_err(), "no such function key");
    }
}
