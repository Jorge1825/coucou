// Mochi's long-term memory: a JSON file, %APPDATA%\Coucou\memory.json, holding
// short notes about the user. It never leaves the machine except as part of the
// chat requests the user already configured, and it can be read, added to,
// deleted from and cleared in the settings window.
//
// Notes come from two places: Mochi itself (the `remember` tool — when the user
// asks, or when it decides a durable fact is worth keeping) and the user (the
// settings window). Every new note goes through `clean`, which refuses anything
// that looks like a credential; existing notes are never rewritten or dropped
// by adding another.
//
// The file used to be memory.txt, one note per line. It is imported once and the
// old file is kept next to it as memory.txt.migrated.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const MAX_NOTES: usize = 80;
const MAX_NOTE_CHARS: usize = 280;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Note {
    pub id: u64,
    pub text: String,
    /// Local time it was saved, "YYYY-MM-DDTHH:MM".
    pub created: String,
    /// "mochi" (saved by the assistant) or "user" (typed in the settings window).
    pub source: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct File {
    version: u32,
    notes: Vec<Note>,
}

/// What `add` did, so the model can be told the truth about it.
#[derive(Debug, PartialEq, Eq)]
pub enum Saved {
    New,
    AlreadyKnown,
}

fn dir() -> PathBuf {
    crate::settings::config_dir()
}

fn json_path() -> PathBuf {
    dir().join("memory.json")
}

fn legacy_path() -> PathBuf {
    dir().join("memory.txt")
}

// ── Pure rules (unit tested) ──────────────────────────────────────────────────

/// Words that, on their own, mark a note as being about a secret.
const SENSITIVE_WORDS: &[&str] = &[
    "password", "passwords", "passwd", "pwd", "passphrase", "contraseña", "contraseñas",
    "contrasena", "contrasenas", "clave", "claves", "secret", "secrets", "secreto", "secretos",
    "token", "tokens", "credential", "credentials", "credencial", "credenciales", "pin",
    "cvv", "cvc", "otp", "apikey",
];

const SENSITIVE_PHRASES: &[&str] = &[
    "api key", "api-key", "private key", "clave privada", "seed phrase", "frase semilla",
    "bearer ", "recovery code", "código de verificación", "codigo de verificacion",
    "numero de tarjeta", "número de tarjeta", "card number",
];

/// Prefixes of well-known secret formats (OpenAI/Anthropic, GitHub, Slack, AWS,
/// Google, JWT). Matched at the start of a word.
const SECRET_PREFIXES: &[&str] = &[
    "sk-", "ghp_", "gho_", "ghs_", "github_pat_", "xoxb-", "xoxp-", "akia", "aiza", "eyj", "re_",
];

pub fn looks_sensitive(note: &str) -> bool {
    let lower = note.to_lowercase();
    if SENSITIVE_PHRASES.iter().any(|p| lower.contains(p)) {
        return true;
    }
    let words: Vec<&str> = lower
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-'))
        .filter(|w| !w.is_empty())
        .collect();
    if words.iter().any(|w| SENSITIVE_WORDS.contains(w)) {
        return true;
    }
    // A word shaped like a key: long, no spaces, letters and digits mixed — or a
    // known secret prefix.
    for word in note.split_whitespace() {
        let w = word.trim_matches(|c: char| !c.is_alphanumeric());
        let wl = w.to_lowercase();
        if SECRET_PREFIXES.iter().any(|p| wl.starts_with(p)) && w.len() >= 12 {
            return true;
        }
        if w.chars().count() >= 24
            && w.chars().any(|c| c.is_ascii_digit())
            && w.chars().any(|c| c.is_alphabetic())
        {
            return true;
        }
    }
    // Card / account numbers: a long run of digits once spaces and dashes go.
    let mut run = 0;
    for c in note.chars() {
        if c.is_ascii_digit() {
            run += 1;
            if run >= 12 {
                return true;
            }
        } else if c != ' ' && c != '-' {
            run = 0;
        }
    }
    false
}

/// Normalises one note, or says why it cannot be kept.
pub fn clean(note: &str) -> Result<String, String> {
    let one_line = note.split_whitespace().collect::<Vec<_>>().join(" ");
    let one_line = one_line.trim().trim_start_matches(['-', '•', '*']).trim().to_string();
    if one_line.is_empty() {
        return Err("Empty note.".into());
    }
    if one_line.chars().count() > MAX_NOTE_CHARS {
        return Err(format!("Too long: keep notes under {MAX_NOTE_CHARS} characters."));
    }
    if looks_sensitive(&one_line) {
        return Err("Not saved: it looks like a password, key or other secret.".into());
    }
    Ok(one_line)
}

/// Adds a note to `notes`, newest last. Existing notes are never touched, except
/// that the oldest go once the cap is passed.
pub fn add_to(notes: &mut Vec<Note>, text: &str, source: &str, now: &str) -> Result<Saved, String> {
    let text = clean(text)?;
    if notes.iter().any(|n| n.text.eq_ignore_ascii_case(&text)) {
        return Ok(Saved::AlreadyKnown);
    }
    let id = notes.iter().map(|n| n.id).max().unwrap_or(0) + 1;
    notes.push(Note { id, text, created: now.to_string(), source: source.to_string() });
    while notes.len() > MAX_NOTES {
        notes.remove(0);
    }
    Ok(Saved::New)
}

/// The old one-note-per-line text → notes. Lines are kept as they are (they were
/// vetted when written); blanks and bullets go.
pub fn import_text(text: &str, now: &str) -> Vec<Note> {
    text.lines()
        .map(|l| l.trim().trim_start_matches(['-', '•', '*']).trim())
        .filter(|l| !l.is_empty())
        .enumerate()
        .map(|(i, l)| Note {
            id: i as u64 + 1,
            text: l.chars().take(MAX_NOTE_CHARS).collect(),
            created: now.to_string(),
            source: "user".to_string(),
        })
        .collect()
}

/// The text of each note the model may be shown — anything that would not pass
/// today's filter (say, a line written before a rule existed) stays on disk but
/// is not sent anywhere.
pub fn shareable(notes: &[Note]) -> Vec<String> {
    notes.iter().filter(|n| !looks_sensitive(&n.text)).map(|n| n.text.clone()).collect()
}

// ── Disk ──────────────────────────────────────────────────────────────────────

/// Every note on disk. A file that cannot be parsed is set aside as
/// memory.json.broken rather than overwritten, and the memory starts empty.
fn read_all() -> Vec<Note> {
    let path = json_path();
    match std::fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice::<File>(&bytes) {
            Ok(file) => file.notes,
            Err(err) => {
                crate::log::line(format!("memory.json unreadable ({err}); kept as memory.json.broken"));
                let _ = std::fs::rename(&path, dir().join("memory.json.broken"));
                Vec::new()
            }
        },
        Err(_) => migrate_legacy(),
    }
}

/// First run after the switch from memory.txt: import it, keep the old file.
fn migrate_legacy() -> Vec<Note> {
    let Ok(text) = std::fs::read_to_string(legacy_path()) else { return Vec::new() };
    let notes = import_text(&text, &crate::reminders::now());
    if write_all(&notes).is_ok() {
        let _ = std::fs::rename(legacy_path(), dir().join("memory.txt.migrated"));
        crate::log::line(format!("memory: imported {} note(s) from memory.txt", notes.len()));
    }
    notes
}

/// Writes through a temporary file so a crash never leaves half a memory.
fn write_all(notes: &[Note]) -> Result<(), String> {
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    let json = serde_json::to_vec_pretty(&File { version: 1, notes: notes.to_vec() })
        .map_err(|e| e.to_string())?;
    let tmp = dir().join("memory.json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, json_path()).map_err(|e| e.to_string())
}

/// All notes, for the settings window.
pub fn list() -> Vec<Note> {
    read_all()
}

/// The notes to show the model.
pub fn load() -> Vec<String> {
    shareable(&read_all())
}

pub fn add(text: &str, source: &str) -> Result<Saved, String> {
    let mut notes = read_all();
    let outcome = add_to(&mut notes, text, source, &crate::reminders::now())?;
    if outcome == Saved::New {
        write_all(&notes)?;
    }
    Ok(outcome)
}

pub fn delete(id: u64) -> Result<(), String> {
    let mut notes = read_all();
    notes.retain(|n| n.id != id);
    write_all(&notes)
}

pub fn clear() -> Result<(), String> {
    write_all(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-10-01T12:00";

    fn texts(notes: &[Note]) -> Vec<&str> {
        notes.iter().map(|n| n.text.as_str()).collect()
    }

    #[test]
    fn keeps_ordinary_facts() {
        for ok in [
            "The user's name is Ana and she prefers answers in Spanish.",
            "Works on a Tauri app called Coucou.",
            "Prefiere respuestas cortas.",
            "Birthday is on March 3rd.",
            "Likes spinning classes", // contains "pin" inside a word
        ] {
            assert!(clean(ok).is_ok(), "should keep: {ok}");
        }
    }

    #[test]
    fn refuses_credentials() {
        for bad in [
            "My password is hunter2",
            "mi contraseña es 1234",
            "The API key is stored here",
            "sk-ant-api03-abcdefghijklmnop",
            "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            "token: abc",
            "Card 4111 1111 1111 1111",
            "eyJhbGciOiJIUzI1NiJ9.payload.sig",
            "Her PIN is 4821",
            "AKIAIOSFODNN7EXAMPLE",
        ] {
            assert!(clean(bad).is_err(), "should refuse: {bad}");
        }
    }

    #[test]
    fn long_mixed_blobs_are_refused() {
        assert!(clean("remember a8Fk29sLq0Zx81mNb7Vc4Rt6Yh").is_err());
    }

    #[test]
    fn adds_with_metadata_and_dedupes() {
        let mut notes = Vec::new();
        assert_eq!(add_to(&mut notes, "Likes tea", "mochi", NOW).unwrap(), Saved::New);
        assert_eq!(add_to(&mut notes, "likes TEA", "user", NOW).unwrap(), Saved::AlreadyKnown);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].source, "mochi");
        assert_eq!(notes[0].created, NOW);
        add_to(&mut notes, "Lives in Quito", "user", NOW).unwrap();
        assert_ne!(notes[0].id, notes[1].id);
    }

    #[test]
    fn caps_by_dropping_the_oldest() {
        let mut notes = Vec::new();
        for i in 0..100 {
            add_to(&mut notes, &format!("fact number {i}"), "mochi", NOW).unwrap();
        }
        assert_eq!(notes.len(), MAX_NOTES);
        assert_eq!(notes.last().unwrap().text, "fact number 99");
        // Ids keep growing, so deleting by id never hits the wrong note.
        let ids: Vec<u64> = notes.iter().map(|n| n.id).collect();
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn adding_never_alters_existing_notes() {
        // A line the filter would refuse today (written before a rule existed)
        // survives an unrelated add.
        let mut notes = import_text("likes tea\nthe clave is on the desk", NOW);
        add_to(&mut notes, "works at night", "mochi", NOW).unwrap();
        assert_eq!(texts(&notes), vec!["likes tea", "the clave is on the desk", "works at night"]);
    }

    #[test]
    fn notes_are_one_line_and_bounded() {
        assert_eq!(clean("  - hello\n  world ").unwrap(), "hello world");
        assert!(clean(&"x ".repeat(200)).is_err());
        assert!(clean("   ").is_err());
    }

    #[test]
    fn imports_the_old_text_file() {
        let notes = import_text("- likes tea\n\n  lives in Quito  \n• works at night", NOW);
        assert_eq!(texts(&notes), vec!["likes tea", "lives in Quito", "works at night"]);
        assert!(notes.iter().all(|n| n.source == "user" && n.created == NOW));
        assert_eq!(notes.iter().map(|n| n.id).collect::<Vec<_>>(), vec![1, 2, 3]);
    }

    #[test]
    fn the_model_only_sees_shareable_notes() {
        let notes = import_text("likes tea\nthe clave is on the desk", NOW);
        assert_eq!(shareable(&notes), vec!["likes tea"]);
    }

    #[test]
    fn json_round_trips() {
        let mut notes = Vec::new();
        add_to(&mut notes, "Likes tea", "mochi", NOW).unwrap();
        let bytes = serde_json::to_vec(&File { version: 1, notes: notes.clone() }).unwrap();
        let back: File = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.notes, notes);
    }
}
