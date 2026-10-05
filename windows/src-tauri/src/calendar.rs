// Calendar: upcoming meetings from an iCal (.ics) address.
//
// Google Calendar ("Secret address in iCal format") and Outlook ("Publish a
// calendar" → ICS link) both hand one out for free, no developer account or API
// key. The address is a secret, so it lives in the Credential Manager like the
// API keys. It is fetched every few minutes while the feature is on; only the
// next couple of days are kept, in memory.
//
// A small iCal reader, no dependency: events, all-day events, recurrence
// (DAILY / WEEKLY with BYDAY / MONTHLY with BYDAY or BYMONTHDAY / YEARLY, with
// INTERVAL, COUNT and UNTIL), EXDATE, moved instances (RECURRENCE-ID) and
// cancelled ones. Times with a TZID are read as local time — right for one's
// own calendar, off by the difference for an event created in another zone.

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::island;
use crate::log;

pub const SECRET_KEY: &str = "calendar-ics";
const EVERY: Duration = Duration::from_secs(10 * 60);
const IDLE_EVERY: Duration = Duration::from_secs(60);
/// How far ahead (and back, for meetings in progress) instances are kept.
const AHEAD_HOURS: i64 = 48;
const BEHIND_HOURS: i64 = 12;
const MAX_ICS_BYTES: usize = 8 * 1024 * 1024;

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CalEvent {
    pub title: String,
    /// Unix milliseconds.
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    pub location: String,
    /// Meeting link (Meet, Teams, Zoom, Webex…) when there is one.
    pub link: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CalendarUpdate {
    pub events: Vec<CalEvent>,
    pub error: Option<String>,
    pub configured: bool,
}

// ── Local date-times ─────────────────────────────────────────────────────────

/// A wall-clock date-time with no zone attached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ldt {
    pub y: i32,
    pub mo: u32,
    pub d: u32,
    pub h: u32,
    pub mi: u32,
    pub s: u32,
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

fn days_in_month(y: i32, m: u32) -> u32 {
    let next = if m == 12 { days_from_civil(y + 1, 1, 1) } else { days_from_civil(y, m + 1, 1) };
    (next - days_from_civil(y, m, 1)) as u32
}

impl Ldt {
    fn date(y: i32, mo: u32, d: u32) -> Self {
        Ldt { y, mo, d, h: 0, mi: 0, s: 0 }
    }
    fn days(&self) -> i64 {
        days_from_civil(self.y, self.mo, self.d)
    }
    /// Same time of day on another date.
    fn on_day(&self, days: i64) -> Self {
        let (y, mo, d) = civil_from_days(days);
        Ldt { y, mo, d, ..*self }
    }
    /// 0 = Monday … 6 = Sunday.
    fn weekday(&self) -> u32 {
        ((self.days() + 3).rem_euclid(7)) as u32
    }
    /// Seconds as if this wall-clock time were UTC (for ordering and durations).
    fn naive_secs(&self) -> i64 {
        self.days() * 86400 + (self.h * 3600 + self.mi * 60 + self.s) as i64
    }
    fn from_naive_secs(secs: i64) -> Self {
        let days = secs.div_euclid(86400);
        let rem = secs.rem_euclid(86400) as u32;
        let (y, mo, d) = civil_from_days(days);
        Ldt { y, mo, d, h: rem / 3600, mi: rem / 60 % 60, s: rem % 60 }
    }
}

// ── Zone conversion (Windows' own rules for the local zone) ──────────────────

mod zone {
    use super::Ldt;
    use windows::Win32::Foundation::SYSTEMTIME;
    use windows::Win32::System::Time::{SystemTimeToTzSpecificLocalTime, TzSpecificLocalTimeToSystemTime};

    fn st(t: &Ldt) -> SYSTEMTIME {
        SYSTEMTIME {
            wYear: t.y as u16,
            wMonth: t.mo as u16,
            wDay: t.d as u16,
            wHour: t.h as u16,
            wMinute: t.mi as u16,
            wSecond: t.s as u16,
            ..Default::default()
        }
    }

    fn ldt(s: &SYSTEMTIME) -> Ldt {
        Ldt { y: s.wYear as i32, mo: s.wMonth as u32, d: s.wDay as u32, h: s.wHour as u32, mi: s.wMinute as u32, s: s.wSecond as u32 }
    }

    /// Local wall clock → Unix seconds.
    pub fn local_to_unix(t: &Ldt) -> i64 {
        let mut utc = SYSTEMTIME::default();
        if unsafe { TzSpecificLocalTimeToSystemTime(None, &st(t), &mut utc) }.is_ok() {
            ldt(&utc).naive_secs()
        } else {
            t.naive_secs()
        }
    }

    /// UTC wall clock → local wall clock.
    pub fn utc_to_local(t: &Ldt) -> Ldt {
        let mut local = SYSTEMTIME::default();
        if unsafe { SystemTimeToTzSpecificLocalTime(None, &st(t), &mut local) }.is_ok() {
            ldt(&local)
        } else {
            *t
        }
    }

    pub fn now_local() -> Ldt {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        utc_to_local(&Ldt::from_naive_secs(secs))
    }
}

// ── iCal parsing ─────────────────────────────────────────────────────────────

struct Prop {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

/// Unfolds continuation lines and splits `NAME;PARAM=V:value`.
fn props(text: &str) -> Vec<Prop> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if (raw.starts_with(' ') || raw.starts_with('\t')) && !lines.is_empty() {
            lines.last_mut().unwrap().push_str(&raw[1..]);
        } else {
            lines.push(raw.to_string());
        }
    }
    lines
        .into_iter()
        .filter_map(|line| {
            // The value starts at the first ':' outside a quoted parameter.
            let mut quoted = false;
            let colon = line.char_indices().find(|&(_, c)| {
                if c == '"' {
                    quoted = !quoted;
                }
                c == ':' && !quoted
            })?.0;
            let (head, value) = (&line[..colon], &line[colon + 1..]);
            let mut parts = head.split(';');
            let name = parts.next()?.trim().to_ascii_uppercase();
            let params = parts
                .filter_map(|p| p.split_once('='))
                .map(|(k, v)| (k.to_ascii_uppercase(), v.trim_matches('"').to_string()))
                .collect();
            Some(Prop { name, params, value: value.to_string() })
        })
        .collect()
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push(' '),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A date or date-time value: (wall clock, all day, given in UTC).
fn parse_when(value: &str, params: &[(String, String)]) -> Option<(Ldt, bool, bool)> {
    let v = value.trim();
    let is_date = params.iter().any(|(k, v)| k == "VALUE" && v.eq_ignore_ascii_case("DATE")) || v.len() == 8;
    let num = |a: usize, b: usize| v.get(a..b)?.parse::<u32>().ok();
    let y = v.get(0..4)?.parse::<i32>().ok()?;
    let (mo, d) = (num(4, 6)?, num(6, 8)?);
    if is_date {
        return Some((Ldt::date(y, mo, d), true, false));
    }
    if v.as_bytes().get(8) != Some(&b'T') {
        return None;
    }
    let (h, mi, s) = (num(9, 11)?, num(11, 13)?, num(13, 15).unwrap_or(0));
    Some((Ldt { y, mo, d, h, mi, s }, false, v.ends_with('Z')))
}

/// Wall clock in the local zone for a parsed value.
fn to_local((t, _all_day, utc): (Ldt, bool, bool)) -> Ldt {
    if utc { zone::utc_to_local(&t) } else { t }
}

/// `PT1H30M`, `P1D`… → seconds.
fn parse_duration(v: &str) -> Option<i64> {
    let v = v.trim();
    let (sign, v) = match v.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, v.strip_prefix('+').unwrap_or(v)),
    };
    let v = v.strip_prefix('P')?;
    let mut total = 0i64;
    let mut num = String::new();
    for c in v.chars() {
        match c {
            '0'..='9' => num.push(c),
            'T' => {}
            'W' | 'D' | 'H' | 'M' | 'S' => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total += n * match c {
                    'W' => 604800,
                    'D' => 86400,
                    'H' => 3600,
                    'M' => 60,
                    _ => 1,
                };
            }
            _ => return None,
        }
    }
    Some(sign * total)
}

#[derive(Default, Clone, Debug)]
pub struct Rule {
    pub freq: String,
    pub interval: i64,
    pub count: Option<u32>,
    pub until: Option<Ldt>,
    /// (ordinal, weekday 0=Mon): ordinal 0 = every such day.
    pub by_day: Vec<(i32, u32)>,
    pub by_month_day: Vec<i32>,
}

fn weekday_code(s: &str) -> Option<u32> {
    ["MO", "TU", "WE", "TH", "FR", "SA", "SU"].iter().position(|d| *d == s).map(|i| i as u32)
}

fn parse_rule(v: &str) -> Option<Rule> {
    let mut r = Rule { interval: 1, ..Default::default() };
    for part in v.split(';') {
        let (k, val) = part.split_once('=')?;
        match k.to_ascii_uppercase().as_str() {
            "FREQ" => r.freq = val.to_ascii_uppercase(),
            "INTERVAL" => r.interval = val.parse::<i64>().ok().filter(|n| *n > 0).unwrap_or(1),
            "COUNT" => r.count = val.parse().ok(),
            "UNTIL" => r.until = parse_when(val, &[]).map(to_local),
            "BYDAY" => {
                r.by_day = val
                    .split(',')
                    .filter_map(|d| {
                        let d = d.trim().to_ascii_uppercase();
                        let (num, code) = d.split_at(d.len().saturating_sub(2));
                        Some((num.parse::<i32>().unwrap_or(0), weekday_code(code)?))
                    })
                    .collect()
            }
            "BYMONTHDAY" => r.by_month_day = val.split(',').filter_map(|d| d.trim().parse().ok()).collect(),
            _ => {}
        }
    }
    (!r.freq.is_empty()).then_some(r)
}

/// Occurrence starts of `rule` from `start`, ending at `to` (inclusive).
/// Pure wall-clock arithmetic, so daylight-saving changes keep the meeting at
/// the same local hour, as calendars do.
pub fn expand(start: Ldt, rule: &Rule, to: Ldt) -> Vec<Ldt> {
    const MAX_STEPS: usize = 20_000;
    let mut out = Vec::new();
    let mut produced = 0u32;
    let until = rule.until;
    let mut push = |t: Ldt, out: &mut Vec<Ldt>| -> bool {
        if t < start {
            return true;
        }
        if until.is_some_and(|u| t > u) || t > to {
            return false;
        }
        if rule.count.is_some_and(|c| produced >= c) {
            return false;
        }
        produced += 1;
        out.push(t);
        true
    };
    let weekdays: Vec<u32> = if rule.by_day.is_empty() {
        vec![start.weekday()]
    } else {
        rule.by_day.iter().map(|(_, d)| *d).collect()
    };
    match rule.freq.as_str() {
        "DAILY" => {
            let mut day = start.days();
            for _ in 0..MAX_STEPS {
                let t = start.on_day(day);
                if rule.by_day.is_empty() || weekdays.contains(&t.weekday()) {
                    if !push(t, &mut out) {
                        break;
                    }
                } else if t > to {
                    break;
                }
                day += rule.interval;
            }
        }
        "WEEKLY" => {
            let week0 = start.days() - start.weekday() as i64;
            let mut days: Vec<u32> = weekdays.clone();
            days.sort_unstable();
            days.dedup();
            'weeks: for k in 0..MAX_STEPS as i64 {
                let monday = week0 + k * 7 * rule.interval;
                for wd in &days {
                    if !push(start.on_day(monday + *wd as i64), &mut out) {
                        break 'weeks;
                    }
                }
            }
        }
        "MONTHLY" => {
            let (mut y, mut m) = (start.y, start.mo);
            'months: for _ in 0..MAX_STEPS {
                let dim = days_in_month(y, m);
                let mut days: Vec<u32> = Vec::new();
                if !rule.by_day.is_empty() {
                    for (ord, wd) in &rule.by_day {
                        let first = days_from_civil(y, m, 1);
                        let all: Vec<u32> = (0..dim)
                            .filter(|i| ((first + *i as i64 + 3).rem_euclid(7)) as u32 == *wd)
                            .map(|i| i + 1)
                            .collect();
                        match *ord {
                            0 => days.extend(all),
                            n if n > 0 => days.extend(all.get(n as usize - 1)),
                            n => days.extend(all.len().checked_sub((-n) as usize).and_then(|i| all.get(i))),
                        }
                    }
                } else if !rule.by_month_day.is_empty() {
                    for md in &rule.by_month_day {
                        let d = if *md < 0 { dim as i32 + 1 + md } else { *md };
                        if d >= 1 && d as u32 <= dim {
                            days.push(d as u32);
                        }
                    }
                } else if start.d <= dim {
                    days.push(start.d);
                }
                days.sort_unstable();
                days.dedup();
                for d in days {
                    if !push(Ldt { y, mo: m, d, ..start }, &mut out) {
                        break 'months;
                    }
                }
                let next = m as i64 - 1 + rule.interval;
                y += (next / 12) as i32;
                m = (next % 12) as u32 + 1;
                if Ldt::date(y, m, 1) > to {
                    break;
                }
            }
        }
        "YEARLY" => {
            let mut y = start.y;
            for _ in 0..MAX_STEPS {
                if start.d <= days_in_month(y, start.mo) && !push(Ldt { y, ..start }, &mut out) {
                    break;
                }
                y += rule.interval as i32;
                if Ldt::date(y, 1, 1) > to {
                    break;
                }
            }
        }
        _ => {
            push(start, &mut out);
        }
    }
    out
}

/// Meeting link in free text: known meeting services first, then any https URL.
fn find_link(texts: &[&str]) -> Option<String> {
    const KNOWN: [&str; 7] = [
        "meet.google.com", "teams.microsoft.com", "teams.live.com", "zoom.us", "webex.com", "whereby.com", "meet.jit.si",
    ];
    let mut urls: Vec<String> = Vec::new();
    for text in texts {
        let mut rest = *text;
        while let Some(i) = rest.find("https://") {
            let tail = &rest[i..];
            let end = tail
                .find(|c: char| c.is_whitespace() || "<>\"'),;".contains(c))
                .unwrap_or(tail.len());
            urls.push(tail[..end].to_string());
            rest = &tail[end..];
        }
    }
    urls.iter()
        .find(|u| KNOWN.iter().any(|k| u.contains(k)))
        .or_else(|| urls.first())
        .cloned()
}

/// Everything happening between `from` and `to` (local wall clock), sorted.
pub fn parse(ics: &str, from: Ldt, to: Ldt) -> Vec<CalEvent> {
    #[derive(Default)]
    struct Raw {
        uid: String,
        summary: String,
        location: String,
        texts: Vec<String>,
        start: Option<(Ldt, bool, bool)>,
        end: Option<(Ldt, bool, bool)>,
        duration: Option<i64>,
        rule: Option<Rule>,
        exdates: Vec<Ldt>,
        recurrence_id: Option<Ldt>,
        cancelled: bool,
    }
    let mut events: Vec<Raw> = Vec::new();
    let mut cur: Option<Raw> = None;
    let mut depth = 0; // inside nested blocks such as VALARM
    for p in props(ics) {
        match (p.name.as_str(), p.value.trim().to_ascii_uppercase().as_str()) {
            ("BEGIN", "VEVENT") => cur = Some(Raw::default()),
            ("END", "VEVENT") => {
                if let Some(e) = cur.take() {
                    events.push(e);
                }
            }
            ("BEGIN", _) if cur.is_some() => depth += 1,
            ("END", _) if cur.is_some() && depth > 0 => depth -= 1,
            _ => {
                let Some(e) = cur.as_mut() else { continue };
                if depth > 0 {
                    continue;
                }
                match p.name.as_str() {
                    "UID" => e.uid = p.value.clone(),
                    "SUMMARY" => e.summary = unescape(&p.value),
                    "LOCATION" => {
                        e.location = unescape(&p.value);
                        e.texts.push(e.location.clone());
                    }
                    "DESCRIPTION" | "URL" | "X-GOOGLE-CONFERENCE" | "X-MICROSOFT-SKYPETEAMSMEETINGURL" => {
                        e.texts.push(unescape(&p.value))
                    }
                    "DTSTART" => e.start = parse_when(&p.value, &p.params),
                    "DTEND" => e.end = parse_when(&p.value, &p.params),
                    "DURATION" => e.duration = parse_duration(&p.value),
                    "RRULE" => e.rule = parse_rule(&p.value),
                    "EXDATE" => e.exdates.extend(
                        p.value.split(',').filter_map(|v| parse_when(v, &p.params)).map(to_local),
                    ),
                    "RECURRENCE-ID" => e.recurrence_id = parse_when(&p.value, &p.params).map(to_local),
                    "STATUS" => e.cancelled = p.value.trim().eq_ignore_ascii_case("CANCELLED"),
                    _ => {}
                }
            }
        }
    }

    // Instances moved or cancelled one by one replace their slot in the series.
    let overridden: HashSet<(String, Ldt)> = events
        .iter()
        .filter_map(|e| Some((e.uid.clone(), e.recurrence_id?)))
        .collect();

    let mut out = Vec::new();
    for e in &events {
        if e.cancelled {
            continue;
        }
        let Some(start_raw) = e.start else { continue };
        let all_day = start_raw.1;
        let start = to_local(start_raw);
        let length = match (e.end, e.duration) {
            (Some(end), _) => to_local(end).naive_secs() - start.naive_secs(),
            (None, Some(d)) => d,
            (None, None) => if all_day { 86400 } else { 3600 },
        }
        .max(0);
        // Look back far enough to catch what is still running at `from`.
        let window_to = to;
        let starts = match (&e.rule, e.recurrence_id) {
            (Some(rule), None) => expand(start, rule, window_to),
            _ => vec![start],
        };
        let link = find_link(&e.texts.iter().map(String::as_str).collect::<Vec<_>>());
        for s in starts {
            let end = Ldt::from_naive_secs(s.naive_secs() + length);
            if end < from || s > to {
                continue;
            }
            if e.recurrence_id.is_none()
                && (e.exdates.iter().any(|x| x.days() == s.days() && (all_day || (x.h, x.mi) == (s.h, s.mi)))
                    || overridden.contains(&(e.uid.clone(), s)))
            {
                continue;
            }
            out.push(CalEvent {
                title: if e.summary.is_empty() { "(no title)".into() } else { e.summary.clone() },
                start: zone::local_to_unix(&s) * 1000,
                end: zone::local_to_unix(&end) * 1000,
                all_day,
                location: e.location.clone(),
                link: link.clone(),
            });
        }
    }
    out.sort_by_key(|e| e.start);
    out.dedup();
    out
}

// ── Fetching ─────────────────────────────────────────────────────────────────

fn enabled(app: &AppHandle) -> bool {
    app.try_state::<crate::Shared>()
        .map(|s| s.settings.lock().unwrap().calendar.enabled)
        .unwrap_or(false)
}

/// `webcal://` is just https for calendar apps; anything else is refused.
fn normalise(url: &str) -> Option<String> {
    let url = url.trim();
    if let Some(rest) = url.strip_prefix("webcal://") {
        return Some(format!("https://{rest}"));
    }
    url.starts_with("https://").then(|| url.to_string())
}

async fn fetch() -> CalendarUpdate {
    let Some(url) = crate::secrets::get(SECRET_KEY).and_then(|u| normalise(&u)) else {
        return CalendarUpdate { events: vec![], error: None, configured: false };
    };
    let fail = |e: String| CalendarUpdate { events: vec![], error: Some(e), configured: true };
    let client = reqwest::Client::builder().timeout(Duration::from_secs(20)).build().unwrap_or_default();
    let res = match client.get(&url).send().await {
        Ok(r) => r,
        Err(_) => return fail("Could not reach the calendar address.".into()),
    };
    if !res.status().is_success() {
        return fail(format!("Calendar address answered {}", res.status().as_u16()));
    }
    let bytes = match res.bytes().await {
        Ok(b) if b.len() <= MAX_ICS_BYTES => b,
        Ok(_) => return fail("Calendar is too large.".into()),
        Err(_) => return fail("Could not download the calendar.".into()),
    };
    let text = String::from_utf8_lossy(&bytes);
    if !text.contains("BEGIN:VCALENDAR") {
        return fail("That address is not an iCal (.ics) calendar.".into());
    }
    let now = zone::now_local();
    let from = Ldt::from_naive_secs(now.naive_secs() - BEHIND_HOURS * 3600);
    let to = Ldt::from_naive_secs(now.naive_secs() + AHEAD_HOURS * 3600);
    let events = parse(&text, from, to);
    log::line(format!("calendar: {} upcoming events", events.len()));
    CalendarUpdate { events, error: None, configured: true }
}

/// Fetch now and tell the islands (Refresh button, a new address).
pub async fn refresh(app: AppHandle) {
    let update = fetch().await;
    island::emit_all(&app, "calendar", update);
}

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut was_on = false;
        let mut since = std::time::Instant::now();
        loop {
            let on = enabled(&app) && !crate::integrations::PAUSED.load(Ordering::Relaxed);
            if on && (!was_on || since.elapsed() >= EVERY) {
                refresh(app.clone()).await;
                since = std::time::Instant::now();
            }
            was_on = on;
            tokio::time::sleep(IDLE_EVERY).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> Ldt {
        Ldt { y, mo, d, h, mi, s: 0 }
    }

    #[test]
    fn calendar_arithmetic_round_trips() {
        for days in [-1000, 0, 1, 365, 10_000, 20_000] {
            let (y, m, dd) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, dd), days);
        }
        assert_eq!(d(2026, 10, 5, 0, 0).weekday(), 0); // a Monday
        assert_eq!(days_in_month(2028, 2), 29);
    }

    #[test]
    fn weekly_rule_with_days() {
        let rule = parse_rule("FREQ=WEEKLY;BYDAY=MO,WE,FR;COUNT=5").unwrap();
        let got = expand(d(2026, 10, 5, 9, 30), &rule, d(2026, 12, 31, 0, 0));
        assert_eq!(
            got,
            vec![
                d(2026, 10, 5, 9, 30), d(2026, 10, 7, 9, 30), d(2026, 10, 9, 9, 30),
                d(2026, 10, 12, 9, 30), d(2026, 10, 14, 9, 30),
            ]
        );
    }

    #[test]
    fn monthly_nth_weekday_and_until() {
        let rule = parse_rule("FREQ=MONTHLY;BYDAY=-1FR;UNTIL=20261231T235959").unwrap();
        let got = expand(d(2026, 10, 30, 16, 0), &rule, d(2027, 6, 1, 0, 0));
        assert_eq!(got, vec![d(2026, 10, 30, 16, 0), d(2026, 11, 27, 16, 0), d(2026, 12, 25, 16, 0)]);
    }

    #[test]
    fn daily_interval_stops_at_window() {
        let rule = parse_rule("FREQ=DAILY;INTERVAL=2").unwrap();
        let got = expand(d(2026, 10, 1, 8, 0), &rule, d(2026, 10, 7, 23, 0));
        assert_eq!(got, vec![d(2026, 10, 1, 8, 0), d(2026, 10, 3, 8, 0), d(2026, 10, 5, 8, 0), d(2026, 10, 7, 8, 0)]);
    }

    #[test]
    fn parses_events_exdates_overrides_and_links() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a\r\nSUMMARY:Daily\\, team\r\n\
DTSTART:20261005T090000\r\nDTEND:20261005T091500\r\nRRULE:FREQ=DAILY\r\n\
EXDATE:20261006T090000\r\nDESCRIPTION:Join https://meet.google.com/abc-defg-hij\r\n  now\r\n\
BEGIN:VALARM\r\nTRIGGER:-PT10M\r\nEND:VALARM\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:a\r\nRECURRENCE-ID:20261007T090000\r\nSUMMARY:Daily (moved)\r\n\
DTSTART:20261007T100000\r\nDTEND:20261007T101500\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:b\r\nSUMMARY:Gone\r\nSTATUS:CANCELLED\r\nDTSTART:20261005T120000\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:c\r\nSUMMARY:Holiday\r\nDTSTART;VALUE=DATE:20261008\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let got = parse(ics, d(2026, 10, 5, 0, 0), d(2026, 10, 8, 23, 0));
        let titles: Vec<&str> = got.iter().map(|e| e.title.as_str()).collect();
        // 5th, (6th excluded), 7th moved to 10:00, 8th + the all-day holiday.
        assert_eq!(titles, vec!["Daily, team", "Daily (moved)", "Holiday", "Daily, team"]);
        assert_eq!(got[0].link.as_deref(), Some("https://meet.google.com/abc-defg-hij"));
        assert!(got.iter().any(|e| e.all_day));
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("PT1H30M"), Some(5400));
        assert_eq!(parse_duration("P1D"), Some(86400));
        assert_eq!(parse_duration("-PT10M"), Some(-600));
    }
}
