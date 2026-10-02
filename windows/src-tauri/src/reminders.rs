// Reminders Mochi sets for the user, and the clock that fires them.
//
// Stored as JSON in %APPDATA%\Coucou\reminders.json so they survive a restart; a
// reminder that came due while the app was closed fires at the next launch.
// Times are the user's local wall-clock time as "YYYY-MM-DDTHH:MM". The fixed
// width makes plain string comparison equal to time order, so no date library is
// needed.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use windows::Win32::System::SystemInformation::GetLocalTime;

const MAX_REMINDERS: usize = 50;
const MAX_TEXT_CHARS: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reminder {
    pub id: u64,
    pub text: String,
    /// Local time, "YYYY-MM-DDTHH:MM".
    pub due: String,
}

fn path() -> PathBuf {
    crate::settings::config_dir().join("reminders.json")
}

/// Local time now, in the same shape as `Reminder::due`.
pub fn now() -> String {
    let t = unsafe { GetLocalTime() };
    format!("{:04}-{:02}-{:02}T{:02}:{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute)
}

// ── Pure rules (unit tested) ──────────────────────────────────────────────────

/// True for exactly "YYYY-MM-DDTHH:MM" with plausible numbers.
pub fn valid_time(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 16 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' {
        return false;
    }
    let num = |a: usize, z: usize| s.get(a..z).and_then(|p| p.parse::<u32>().ok());
    match (num(0, 4), num(5, 7), num(8, 10), num(11, 13), num(14, 16)) {
        (Some(y), Some(mo), Some(d), Some(h), Some(mi)) => {
            y >= 2000 && (1..=12).contains(&mo) && (1..=31).contains(&d) && h < 24 && mi < 60
        }
        _ => false,
    }
}

pub fn is_due(due: &str, now: &str) -> bool {
    due <= now
}

fn clean_text(text: &str) -> Result<String, String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return Err("Empty reminder.".into());
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(format!("Too long: keep reminders under {MAX_TEXT_CHARS} characters."));
    }
    if crate::memory::looks_sensitive(&text) {
        return Err("Not saved: it looks like a password, key or other secret.".into());
    }
    Ok(text)
}

/// Adds a reminder to `list`. A reminder already set for the same time and text
/// is not duplicated.
pub fn add_to(list: &mut Vec<Reminder>, text: &str, due: &str, now: &str) -> Result<(), String> {
    if !valid_time(due) {
        return Err("Not saved: `when` must be local time as YYYY-MM-DDTHH:MM.".into());
    }
    if due < now {
        return Err("Not saved: that time is already in the past.".into());
    }
    let text = clean_text(text)?;
    if list.iter().any(|r| r.due == due && r.text.eq_ignore_ascii_case(&text)) {
        return Ok(());
    }
    if list.len() >= MAX_REMINDERS {
        return Err("Not saved: too many pending reminders.".into());
    }
    let id = list.iter().map(|r| r.id).max().unwrap_or(0) + 1;
    list.push(Reminder { id, text, due: due.to_string() });
    list.sort_by(|a, b| a.due.cmp(&b.due));
    Ok(())
}

/// Splits `list` into (due now, still pending).
pub fn split_due(list: Vec<Reminder>, now: &str) -> (Vec<Reminder>, Vec<Reminder>) {
    list.into_iter().partition(|r| is_due(&r.due, now))
}

// ── Disk ──────────────────────────────────────────────────────────────────────

pub fn load() -> Vec<Reminder> {
    std::fs::read(path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn store(list: &[Reminder]) -> Result<(), String> {
    std::fs::create_dir_all(crate::settings::config_dir()).map_err(|e| e.to_string())?;
    let json = serde_json::to_vec_pretty(list).map_err(|e| e.to_string())?;
    std::fs::write(path(), json).map_err(|e| e.to_string())
}

pub fn add(text: &str, due: &str) -> Result<(), String> {
    let mut list = load();
    add_to(&mut list, text, due, &now())?;
    store(&list)
}

pub fn delete(id: u64) -> Result<(), String> {
    let mut list = load();
    list.retain(|r| r.id != id);
    store(&list)
}

/// Removes and returns every reminder that has come due.
pub fn take_due() -> Vec<Reminder> {
    let (due, pending) = split_due(load(), &now());
    if !due.is_empty() {
        let _ = store(&pending);
    }
    due
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_the_time_format() {
        assert!(valid_time("2026-10-02T15:00"));
        assert!(!valid_time("2026-10-02 15:00"));
        assert!(!valid_time("2026-13-02T15:00"));
        assert!(!valid_time("2026-10-02T25:00"));
        assert!(!valid_time("tomorrow at 3"));
        assert!(!valid_time("2026-10-02T15:00:00"));
    }

    #[test]
    fn string_order_is_time_order() {
        assert!(is_due("2026-10-02T09:00", "2026-10-02T09:00"));
        assert!(is_due("2026-10-01T23:59", "2026-10-02T00:00"));
        assert!(!is_due("2026-10-02T09:01", "2026-10-02T09:00"));
        assert!(!is_due("2027-01-01T00:00", "2026-12-31T23:59"));
    }

    #[test]
    fn adds_sorted_and_without_duplicates() {
        let now = "2026-10-01T12:00";
        let mut list = Vec::new();
        add_to(&mut list, "Call Ana", "2026-10-02T15:00", now).unwrap();
        add_to(&mut list, "Pay rent", "2026-10-01T18:00", now).unwrap();
        add_to(&mut list, "call ana", "2026-10-02T15:00", now).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].text, "Pay rent");
        assert_ne!(list[0].id, list[1].id);
    }

    #[test]
    fn refuses_past_bad_and_sensitive() {
        let now = "2026-10-01T12:00";
        let mut list = Vec::new();
        assert!(add_to(&mut list, "x", "2026-09-01T10:00", now).is_err());
        assert!(add_to(&mut list, "x", "soon", now).is_err());
        assert!(add_to(&mut list, "   ", "2026-10-02T10:00", now).is_err());
        assert!(add_to(&mut list, "send my password to Ana", "2026-10-02T10:00", now).is_err());
        assert!(list.is_empty());
    }

    #[test]
    fn splits_due_from_pending() {
        let now = "2026-10-01T12:00";
        let mk = |id, due: &str| Reminder { id, text: "t".into(), due: due.into() };
        let (due, pending) =
            split_due(vec![mk(1, "2026-10-01T11:59"), mk(2, "2026-10-01T12:00"), mk(3, "2026-10-01T12:01")], now);
        assert_eq!(due.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(pending.iter().map(|r| r.id).collect::<Vec<_>>(), vec![3]);
    }
}
