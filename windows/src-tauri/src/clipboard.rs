// Smart clipboard: notices copied text so the island can offer to explain,
// summarize, translate or fix it.
//
// Off by default. While on, it only compares Windows' clipboard sequence
// number (a counter, no data) and reads the text when that changes. Skipped:
// anything a password manager marks as private, anything that looks like a
// password or key, and what Coucou itself copied. Nothing is stored on disk or
// sent anywhere until the user clicks an action.

use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};
use windows::core::w;
use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, GetClipboardOwner, GetClipboardSequenceNumber,
    IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

use crate::island;

const EVERY: Duration = Duration::from_millis(600);
const IDLE_EVERY: Duration = Duration::from_secs(5);
/// Longer copies are cut: enough to act on, never a whole document in memory.
const MAX_CHARS: usize = 6000;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Copied {
    pub text: String,
    /// Unix milliseconds.
    pub at: i64,
    /// The island on the display with the cursor when it was copied — for
    /// "show suggestions on the screen with the mouse".
    pub cursor_island: String,
}

fn prefs(app: &AppHandle) -> (bool, usize) {
    app.try_state::<crate::Shared>()
        .map(|s| {
            let s = s.settings.lock().unwrap();
            (s.clipboard.enabled, s.clipboard.min_chars as usize)
        })
        .unwrap_or((false, 20))
}

pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_seq = unsafe { GetClipboardSequenceNumber() };
        loop {
            let (enabled, min_chars) = prefs(&app);
            let on = enabled && !crate::integrations::PAUSED.load(Ordering::Relaxed);
            std::thread::sleep(if on { EVERY } else { IDLE_EVERY });
            let seq = unsafe { GetClipboardSequenceNumber() };
            if !on || seq == last_seq {
                last_seq = seq;
                continue;
            }
            last_seq = seq;
            let Some(text) = read_text() else { continue };
            let text = text.trim();
            if text.chars().count() < min_chars || crate::memory::looks_sensitive(text) {
                continue;
            }
            let text: String = text.chars().take(MAX_CHARS).collect();
            let at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            let cursor_island = island::label_under_cursor(&app);
            island::emit_all(&app, "clipboard", Copied { text, at, cursor_island });
        }
    });
}

/// Formats password managers and other careful apps add to say "don't watch this".
fn marked_private() -> bool {
    unsafe {
        for name in [
            w!("ExcludeClipboardContentFromMonitorProcessing"),
            w!("Clipboard Viewer Ignore"),
        ] {
            let f = RegisterClipboardFormatW(name);
            if f != 0 && IsClipboardFormatAvailable(f).is_ok() {
                return true;
            }
        }
        // KeePass & co. also set this one to 0 for secrets.
        let f = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
        if f != 0 && IsClipboardFormatAvailable(f).is_ok() {
            if let Ok(h) = GetClipboardData(f) {
                let g = HGLOBAL(h.0);
                let p = GlobalLock(g) as *const u32;
                let allowed = !p.is_null() && *p != 0;
                let _ = GlobalUnlock(g);
                if !allowed {
                    return true;
                }
            }
        }
    }
    false
}

fn read_text() -> Option<String> {
    unsafe {
        // Copied from Coucou itself (the chat): nothing to suggest.
        if let Ok(owner) = GetClipboardOwner() {
            let mut pid = 0u32;
            GetWindowThreadProcessId(owner, Some(&mut pid));
            if pid == GetCurrentProcessId() {
                return None;
            }
        }
        if IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_err() {
            return None;
        }
        // Another app may hold the clipboard for a moment.
        let mut opened = false;
        for _ in 0..4 {
            if OpenClipboard(None).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        if !opened {
            return None;
        }
        let text = (|| {
            if marked_private() {
                return None;
            }
            let h: HANDLE = GetClipboardData(CF_UNICODETEXT.0 as u32).ok()?;
            let g = HGLOBAL(h.0);
            let p = GlobalLock(g) as *const u16;
            if p.is_null() {
                return None;
            }
            let max = GlobalSize(g) / 2;
            let mut len = 0usize;
            while len < max && *p.add(len) != 0 {
                len += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
            let _ = GlobalUnlock(g);
            Some(s)
        })();
        let _ = CloseClipboard();
        text
    }
}
