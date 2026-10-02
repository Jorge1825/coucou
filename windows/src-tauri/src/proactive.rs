// Mochi speaking up on its own, without the user opening anything.
//
// One background loop, woken every few seconds, does two things:
//   1. fires reminders that have come due (reminders.rs) — always on;
//   2. if the user turned it on in Settings, every N minutes asks the configured
//      model whether it has anything genuinely worth interrupting for.
//
// The check-in sends the model only the clock, Mochi's saved notes and the
// pending reminders — never screen contents, files or chat history — and only to
// the provider the user configured. It is off by default, silent at night, and
// skipped entirely when there is nothing to base it on.

use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::{claude, island, memory, providers, reminders};

const TICK: Duration = Duration::from_secs(15);
/// No unprompted check-ins from 22:00 to 08:00. Reminders still fire.
const QUIET_FROM: u32 = 22;
const QUIET_UNTIL: u32 = 8;
/// How many of its own recent interruptions Mochi is shown, to avoid repeating.
const RECENT_KEPT: usize = 5;

#[derive(Serialize, Clone)]
pub struct Nudge {
    pub text: String,
    /// "reminder" (set by the user or by Mochi earlier) or "mochi" (its own call).
    pub kind: &'static str,
}

const SYSTEM: &str = "You are Mochi, a small personal assistant that lives at the top of the user's screen. \
You are deciding whether to interrupt the user right now. An interruption costs their attention, so do it only \
when it is genuinely useful: a heads-up about something coming up that fits what you know about them, a time-relevant \
nudge, something they asked you to keep an eye on. Most of the time the right answer is to stay quiet. \
If you stay quiet, reply with exactly NOTHING. Otherwise reply with one short sentence in the user's language, \
plain text, no markdown. Never mention passwords or secrets. Do not repeat anything you already said recently.";

// ── Pure rules (unit tested) ──────────────────────────────────────────────────

/// Hour (0–23) of a "YYYY-MM-DDTHH:MM" string.
fn hour_of(now: &str) -> Option<u32> {
    now.get(11..13)?.parse().ok()
}

pub fn in_quiet_hours(now: &str) -> bool {
    match hour_of(now) {
        Some(h) => h >= QUIET_FROM || h < QUIET_UNTIL,
        None => false,
    }
}

/// What the model answered → the sentence to show, or None to stay quiet.
pub fn reply_to_nudge(reply: &str) -> Option<String> {
    let text = reply.trim().trim_matches('"').trim();
    if text.is_empty() || text.to_uppercase().starts_with("NOTHING") {
        return None;
    }
    if memory::looks_sensitive(text) {
        return None;
    }
    Some(text.chars().take(240).collect())
}

fn context(now: &str, notes: &[String], pending: &[reminders::Reminder], recent: &[String]) -> String {
    let mut out = format!("Local date and time: {now}\n");
    out.push_str("\nWhat you remember about the user:\n");
    for n in notes {
        out.push_str(&format!("- {n}\n"));
    }
    if !pending.is_empty() {
        out.push_str("\nReminders already set (they will fire on their own):\n");
        for r in pending {
            out.push_str(&format!("- {} {}\n", r.due, r.text));
        }
    }
    if !recent.is_empty() {
        out.push_str("\nWhat you already said recently:\n");
        for r in recent {
            out.push_str(&format!("- {r}\n"));
        }
    }
    out.push_str("\nShould you interrupt the user now?");
    out
}

// ── The loop ──────────────────────────────────────────────────────────────────

fn deliver(app: &AppHandle, text: String, kind: &'static str) {
    crate::log::line(format!("nudge ({kind}): {} chars", text.chars().count()));
    let _ = app.emit_to(island::WINDOW_LABEL, "nudge", Nudge { text, kind });
}

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last_check = Instant::now();
        let mut recent: Vec<String> = Vec::new();

        loop {
            tokio::time::sleep(TICK).await;

            for r in reminders::take_due() {
                deliver(&app, r.text, "reminder");
                // Let each one land before the next replaces it.
                tokio::time::sleep(Duration::from_secs(6)).await;
            }

            let Some(shared) = app.try_state::<crate::Shared>() else { continue };
            let settings = shared.settings.lock().unwrap().clone();
            if !settings.proactive {
                last_check = Instant::now();
                continue;
            }
            let every = Duration::from_secs(u64::from(settings.proactive_minutes.clamp(10, 240)) * 60);
            if last_check.elapsed() < every {
                continue;
            }
            last_check = Instant::now();

            let now = reminders::now();
            if in_quiet_hours(&now) {
                continue;
            }
            let notes = memory::load();
            let pending = reminders::load();
            if notes.is_empty() && pending.is_empty() {
                continue; // nothing to base a nudge on
            }
            let Ok(target) = providers::target(&settings) else { continue };
            let prompt = context(&now, &notes, &pending, &recent);
            match claude::one_shot(&target, &settings.model, SYSTEM, &prompt).await {
                Ok(reply) => {
                    if let Some(text) = reply_to_nudge(&reply) {
                        recent.push(text.clone());
                        if recent.len() > RECENT_KEPT {
                            recent.remove(0);
                        }
                        deliver(&app, text, "mochi");
                    }
                }
                Err(err) => crate::log::line(format!("proactive check failed: {err}")),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_hours_cover_the_night() {
        assert!(in_quiet_hours("2026-10-01T23:30"));
        assert!(in_quiet_hours("2026-10-02T02:00"));
        assert!(in_quiet_hours("2026-10-02T07:59"));
        assert!(!in_quiet_hours("2026-10-02T08:00"));
        assert!(!in_quiet_hours("2026-10-02T15:00"));
        assert!(!in_quiet_hours("2026-10-02T21:59"));
    }

    #[test]
    fn nothing_means_silence() {
        assert_eq!(reply_to_nudge("NOTHING"), None);
        assert_eq!(reply_to_nudge("  nothing. "), None);
        assert_eq!(reply_to_nudge(""), None);
        assert_eq!(
            reply_to_nudge("Your call with Ana is in an hour."),
            Some("Your call with Ana is in an hour.".into())
        );
    }

    #[test]
    fn secrets_are_never_shown() {
        assert_eq!(reply_to_nudge("Your password is hunter2"), None);
    }

    #[test]
    fn context_lists_what_it_knows() {
        let c = context("2026-10-01T12:00", &["Likes tea".into()], &[], &["Drink water".into()]);
        assert!(c.contains("Likes tea"));
        assert!(c.contains("Drink water"));
        assert!(!c.contains("Reminders already set"));
    }
}
