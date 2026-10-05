// Goals: what the user is working towards, kept in %APPDATA%\Coucou\goals.json.
//
// Mochi can add a goal when the user states one, tick its steps off when they
// report progress, and mark it done — through the chat's `add_goal` and
// `update_goal` tools. The user sees and edits the same list in Settings. Active
// goals are shown to the model in the chat (a few lines) so it can coach: say
// how close a deadline is, suggest the next step.
//
// Like the notes, nothing here leaves the machine except as part of the chat
// requests the user already configured, and anything that looks like a secret
// is refused.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

const MAX_ACTIVE: usize = 12;
const MAX_STEPS: usize = 12;
const MAX_TEXT_CHARS: usize = 160;
/// Goals shown to the model, soonest deadline first.
pub const SHOWN_TO_MODEL: usize = 5;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Step {
    pub text: String,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Goal {
    pub id: u64,
    pub title: String,
    /// "YYYY-MM-DD", when the user gave a deadline.
    pub due: Option<String>,
    pub steps: Vec<Step>,
    /// Local time it was created, "YYYY-MM-DDTHH:MM".
    pub created: String,
    pub done: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct File {
    version: u32,
    goals: Vec<Goal>,
}

// ── Pure rules (unit tested) ──────────────────────────────────────────────────

fn clean_text(text: &str) -> Result<String, String> {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let one_line = one_line.trim().trim_start_matches(['-', '•', '*']).trim().to_string();
    if one_line.is_empty() {
        return Err("Empty text.".into());
    }
    if one_line.chars().count() > MAX_TEXT_CHARS {
        return Err(format!("Too long: keep it under {MAX_TEXT_CHARS} characters."));
    }
    if crate::memory::looks_sensitive(&one_line) {
        return Err("Not saved: it looks like a password, key or other secret.".into());
    }
    Ok(one_line)
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn parse_date(s: &str) -> Option<(i64, i64, i64)> {
    let b = s.as_bytes();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let (y, m, d) = (s.get(0..4)?.parse().ok()?, s.get(5..7)?.parse().ok()?, s.get(8..10)?.parse().ok()?);
    let days_in_month = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        _ => return None,
    };
    ((1..=days_in_month).contains(&d) && (2000..=2100).contains(&y)).then_some((y, m, d))
}

pub fn valid_date(s: &str) -> bool {
    s.len() == 10 && parse_date(s).is_some()
}

/// Whole days from `from` to `to` (both "YYYY-MM-DD" or longer ISO strings);
/// negative when `to` is in the past.
pub fn days_between(from: &str, to: &str) -> Option<i64> {
    let (a, b) = (parse_date(from)?, parse_date(to)?);
    Some(days_from_civil(b.0, b.1, b.2) - days_from_civil(a.0, a.1, a.2))
}

fn active(goals: &[Goal]) -> usize {
    goals.iter().filter(|g| !g.done).count()
}

/// Adds a goal and returns its id.
pub fn add_to(
    goals: &mut Vec<Goal>,
    title: &str,
    due: Option<&str>,
    steps: &[String],
    now: &str,
) -> Result<u64, String> {
    let title = clean_text(title)?;
    if active(goals) >= MAX_ACTIVE {
        return Err(format!("Already {MAX_ACTIVE} active goals: finish or delete one first."));
    }
    let due = match due.map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) if valid_date(d) => Some(d.to_string()),
        Some(_) => return Err("The deadline must be a date like 2026-11-30.".into()),
        None => None,
    };
    if steps.len() > MAX_STEPS {
        return Err(format!("At most {MAX_STEPS} steps."));
    }
    let steps = steps
        .iter()
        .map(|s| clean_text(s).map(|text| Step { text, done: false }))
        .collect::<Result<Vec<_>, _>>()?;
    if goals.iter().any(|g| !g.done && g.title.eq_ignore_ascii_case(&title)) {
        return Err("That goal already exists.".into());
    }
    let id = goals.iter().map(|g| g.id).max().unwrap_or(0) + 1;
    goals.push(Goal { id, title, due, steps, created: now.to_string(), done: false });
    Ok(id)
}

pub enum Change {
    /// 1-based index, as the model and the user count them.
    StepDone(usize),
    AddStep(String),
    Complete,
}

pub fn update_in(goals: &mut [Goal], id: u64, change: Change) -> Result<String, String> {
    let goal = goals.iter_mut().find(|g| g.id == id).ok_or("No goal with that id.")?;
    match change {
        Change::Complete => {
            goal.done = true;
            for s in &mut goal.steps {
                s.done = true;
            }
            Ok(format!("Goal \"{}\" marked as done.", goal.title))
        }
        Change::StepDone(n) => {
            let step = n.checked_sub(1).and_then(|i| goal.steps.get_mut(i)).ok_or("No step with that number.")?;
            step.done = true;
            let (done, total) = progress(goal);
            if done == total {
                goal.done = true;
                return Ok(format!("All {total} steps done: goal \"{}\" is complete.", goal.title));
            }
            Ok(format!("Step done ({done}/{total})."))
        }
        Change::AddStep(text) => {
            if goal.steps.len() >= MAX_STEPS {
                return Err(format!("At most {MAX_STEPS} steps."));
            }
            goal.steps.push(Step { text: clean_text(&text)?, done: false });
            goal.done = false;
            Ok("Step added.".into())
        }
    }
}

pub fn progress(goal: &Goal) -> (usize, usize) {
    (goal.steps.iter().filter(|s| s.done).count(), goal.steps.len())
}

/// Active goals as short lines for the model: soonest deadline first, undated
/// ones after, newest first among those. `today` is "YYYY-MM-DD…".
pub fn lines_for_model(goals: &[Goal], today: &str) -> Vec<String> {
    let mut open: Vec<&Goal> = goals.iter().filter(|g| !g.done).collect();
    open.sort_by(|a, b| match (&a.due, &b.due) {
        (Some(x), Some(y)) => x.cmp(y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => b.id.cmp(&a.id),
    });
    open.into_iter()
        .take(SHOWN_TO_MODEL)
        .map(|g| {
            let mut line = format!("[{}] {}", g.id, g.title);
            if let Some(due) = &g.due {
                match days_between(today, due) {
                    Some(d) if d < 0 => line.push_str(&format!(" — was due {due} ({} days ago)", -d)),
                    Some(0) => line.push_str(" — due today"),
                    Some(d) => line.push_str(&format!(" — due {due} (in {d} days)")),
                    None => line.push_str(&format!(" — due {due}")),
                }
            }
            let (done, total) = progress(g);
            if total > 0 {
                line.push_str(&format!(" — {done}/{total} steps"));
                if let Some((i, s)) = g.steps.iter().enumerate().find(|(_, s)| !s.done) {
                    line.push_str(&format!("; next is step {}: {}", i + 1, s.text));
                }
            }
            line
        })
        .collect()
}

// ── Storage ───────────────────────────────────────────────────────────────────

static LOCK: Mutex<()> = Mutex::new(());

fn path() -> PathBuf {
    crate::settings::config_dir().join("goals.json")
}

fn read() -> Vec<Goal> {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|s| serde_json::from_str::<File>(&s).ok())
        .map(|f| f.goals)
        .unwrap_or_default()
}

fn write(goals: Vec<Goal>) -> Result<(), String> {
    let dir = crate::settings::config_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&File { version: 1, goals }).map_err(|e| e.to_string())?;
    std::fs::write(path(), json).map_err(|e| e.to_string())
}

pub fn list() -> Vec<Goal> {
    let _guard = LOCK.lock().unwrap();
    read()
}

pub fn add(title: &str, due: Option<&str>, steps: &[String]) -> Result<u64, String> {
    let _guard = LOCK.lock().unwrap();
    let mut goals = read();
    let id = add_to(&mut goals, title, due, steps, &crate::reminders::now())?;
    write(goals)?;
    Ok(id)
}

pub fn update(id: u64, change: Change) -> Result<String, String> {
    let _guard = LOCK.lock().unwrap();
    let mut goals = read();
    let message = update_in(&mut goals, id, change)?;
    write(goals)?;
    Ok(message)
}

pub fn delete(id: u64) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap();
    let mut goals = read();
    goals.retain(|g| g.id != id);
    write(goals)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-10-03T09:00";

    fn steps(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn dates_are_validated_and_counted() {
        assert!(valid_date("2026-02-28") && !valid_date("2026-02-30") && !valid_date("2026-13-01"));
        assert!(valid_date("2028-02-29") && !valid_date("2027-02-29"));
        assert!(!valid_date("30/11/2026") && !valid_date("2026-1-5"));
        assert_eq!(days_between("2026-10-03", "2026-11-02"), Some(30));
        assert_eq!(days_between("2026-12-31", "2027-01-01"), Some(1));
        assert_eq!(days_between("2026-10-03T09:00", "2026-10-01"), Some(-2));
        assert_eq!(days_between("nonsense", "2026-10-01"), None);
    }

    #[test]
    fn a_goal_is_added_with_steps_and_validated() {
        let mut g = Vec::new();
        let id = add_to(&mut g, " - Pass the calculus exam ", Some("2026-11-02"), &steps(&["Review ch. 4", "Do past papers"]), NOW).unwrap();
        assert_eq!(id, 1);
        assert_eq!(g[0].title, "Pass the calculus exam");
        assert_eq!(g[0].steps.len(), 2);
        assert!(add_to(&mut g, "x", Some("someday"), &[], NOW).is_err(), "bad deadline");
        assert!(add_to(&mut g, "", None, &[], NOW).is_err(), "empty title");
        assert!(add_to(&mut g, "pass the CALCULUS exam", None, &[], NOW).is_err(), "duplicate");
        assert!(add_to(&mut g, "Save my password hunter2", None, &[], NOW).is_err(), "secrets are refused");
        assert!(add_to(&mut g, "Learn Rust", None, &steps(&["my api key is sk-abcdefghijklmnop"]), NOW).is_err());
    }

    #[test]
    fn the_active_goal_cap_counts_only_open_goals() {
        let mut g = Vec::new();
        for i in 0..MAX_ACTIVE {
            add_to(&mut g, &format!("goal {i}"), None, &[], NOW).unwrap();
        }
        assert!(add_to(&mut g, "one too many", None, &[], NOW).is_err());
        update_in(&mut g, 1, Change::Complete).unwrap();
        assert!(add_to(&mut g, "now there is room", None, &[], NOW).is_ok());
    }

    #[test]
    fn finishing_the_last_step_completes_the_goal() {
        let mut g = Vec::new();
        let id = add_to(&mut g, "Ship the app", None, &steps(&["Write it", "Test it"]), NOW).unwrap();
        assert_eq!(update_in(&mut g, id, Change::StepDone(1)).unwrap(), "Step done (1/2).");
        assert!(!g[0].done);
        assert!(update_in(&mut g, id, Change::StepDone(2)).unwrap().contains("complete"));
        assert!(g[0].done);
        assert!(update_in(&mut g, id, Change::StepDone(9)).is_err(), "no such step");
        assert!(update_in(&mut g, 99, Change::Complete).is_err(), "no such goal");
        // Adding a step to a finished goal reopens it.
        update_in(&mut g, id, Change::AddStep("Release it".into())).unwrap();
        assert!(!g[0].done);
    }

    #[test]
    fn the_model_sees_the_soonest_goals_first_with_their_next_step() {
        let mut g = Vec::new();
        add_to(&mut g, "Someday goal", None, &[], NOW).unwrap();
        add_to(&mut g, "Later", Some("2026-12-01"), &[], NOW).unwrap();
        let soon = add_to(&mut g, "Pass the exam", Some("2026-10-10"), &steps(&["Review", "Practice"]), NOW).unwrap();
        update_in(&mut g, soon, Change::StepDone(1)).unwrap();
        let lines = lines_for_model(&g, "2026-10-03");
        assert_eq!(lines[0], "[3] Pass the exam — due 2026-10-10 (in 7 days) — 1/2 steps; next is step 2: Practice");
        assert!(lines[1].starts_with("[2] Later"));
        assert!(lines[2].starts_with("[1] Someday goal"));
        update_in(&mut g, soon, Change::Complete).unwrap();
        assert_eq!(lines_for_model(&g, "2026-10-03").len(), 2, "finished goals are not shown");
        // An overdue goal says so.
        let mut late = Vec::new();
        add_to(&mut late, "Late one", Some("2026-10-01"), &[], NOW).unwrap();
        assert!(lines_for_model(&late, "2026-10-03")[0].contains("2 days ago"));
    }
}
