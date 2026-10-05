// Mochi watching the user's websites and Dokploy instances, and speaking up
// when something goes down or breaks (and again when it recovers).
//
// Off by default: until the user turns it on AND lists something to watch, this
// loop only sleeps and re-reads the settings — no network, no work. When on,
// every N minutes it calls exactly the URLs the user configured, nothing else.
// A Dokploy API key lives in the Credential Manager (one per instance), is only
// ever sent to that instance as `x-api-key`, and is never logged.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::proactive::Nudge;
use crate::settings::{DokployTarget, Settings};
use crate::{island, secrets};

const TICK: Duration = Duration::from_secs(15);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Failures in a row before the user is told: one blip is not an outage.
const FAILS_BEFORE_ALERT: u32 = 2;
/// Dokploy resource lists that carry a status.
const SERVICE_LISTS: &[&str] = &["applications", "compose", "postgres", "mysql", "mariadb", "mongo", "redis"];
/// The field holding that status (applications and databases vs. compose).
const STATUS_FIELDS: &[&str] = &["applicationStatus", "composeStatus"];

// ── Pure rules (unit tested) ──────────────────────────────────────────────────

/// What the alert logic decided for one target after one check.
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Down,
    Recovered,
}

#[derive(Debug, Default)]
pub struct Tracker {
    fails: u32,
    alerted: bool,
}

impl Tracker {
    /// Feeds one check result. Alerts after `FAILS_BEFORE_ALERT` failures in a
    /// row, once per incident, and reports the recovery once. A target that was
    /// healthy from the start never produces an event.
    pub fn step(&mut self, healthy: bool) -> Event {
        if healthy {
            self.fails = 0;
            if self.alerted {
                self.alerted = false;
                return Event::Recovered;
            }
            return Event::None;
        }
        self.fails = self.fails.saturating_add(1);
        if self.fails >= FAILS_BEFORE_ALERT && !self.alerted {
            self.alerted = true;
            return Event::Down;
        }
        Event::None
    }
}

/// "example.com" → "https://example.com"; empty or garbage → None.
pub fn normalize_url(input: &str) -> Option<String> {
    let s = input.trim().trim_end_matches('/');
    if s.is_empty() || s.contains(char::is_whitespace) {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(s.to_string());
    }
    if s.contains("://") {
        return None;
    }
    Some(format!("https://{s}"))
}

/// One thing that was checked: a stable key for the state machine, a name for
/// the user, and `Some(reason)` when it is broken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub key: String,
    pub label: String,
    pub problem: Option<String>,
}

/// A website answering with a transport error or a 5xx is down; 4xx is reachable.
pub fn site_problem(outcome: Result<u16, String>) -> Option<String> {
    match outcome {
        Ok(code) if code >= 500 => Some(format!("HTTP {code}")),
        Ok(_) => None,
        Err(e) => Some(e),
    }
}

fn items<'a>(v: &'a Value, field: &str) -> &'a [Value] {
    v.get(field).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

/// Dokploy's `project.all` → one observation per service that carries a status.
/// Tolerant: unknown shapes yield fewer observations, never a panic. Newer
/// Dokploy versions nest services under `environments`; older ones keep them on
/// the project itself — both are walked.
pub fn parse_dokploy(instance: &str, body: &Value) -> Vec<Observation> {
    let mut out = Vec::new();
    for project in body.as_array().map(Vec::as_slice).unwrap_or(&[]) {
        let pname = project.get("name").and_then(Value::as_str).unwrap_or("project");
        collect_services(instance, pname, project, &mut out);
        for env in items(project, "environments") {
            collect_services(instance, pname, env, &mut out);
        }
    }
    out
}

fn collect_services(instance: &str, project: &str, holder: &Value, out: &mut Vec<Observation>) {
    for list in SERVICE_LISTS {
        for svc in items(holder, list) {
            let Some(status) = STATUS_FIELDS.iter().find_map(|f| svc.get(*f).and_then(Value::as_str)) else {
                continue;
            };
            let name = svc.get("name").and_then(Value::as_str).unwrap_or("service");
            let id = svc
                .get("applicationId")
                .or_else(|| svc.get("composeId"))
                .and_then(Value::as_str)
                .unwrap_or(name);
            out.push(Observation {
                key: format!("dokploy:{instance}:{project}:{list}:{id}"),
                label: format!("{project} / {name} ({instance})"),
                problem: status.eq_ignore_ascii_case("error").then(|| "status is error".to_string()),
            });
        }
    }
}

pub fn alert_text(label: &str, event: &Event, problem: &str) -> Option<String> {
    let text = match event {
        Event::None => return None,
        Event::Down => format!("{label} is down: {problem}"),
        Event::Recovered => format!("{label} is back up"),
    };
    Some(text.chars().take(200).collect())
}

// ── Checks (network) ──────────────────────────────────────────────────────────

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())
}

/// Short description of a transport failure. Never the error itself: reqwest
/// errors embed the URL, which is noise in an alert.
fn describe(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        "timed out".into()
    } else if err.is_connect() {
        "connection failed".into()
    } else {
        "request failed".into()
    }
}

async fn check_site(client: &reqwest::Client, url: &str) -> Observation {
    let outcome = match client.get(url).send().await {
        Ok(r) => Ok(r.status().as_u16()),
        Err(e) => Err(describe(&e)),
    };
    Observation { key: format!("site:{url}"), label: url.to_string(), problem: site_problem(outcome) }
}

pub fn secret_key(target: &DokployTarget) -> String {
    format!("dokploy-key:{}", target.name.trim())
}

/// All observations for one Dokploy instance: the instance itself, plus each of
/// its services when it answers. `None` = no key saved or bad URL, nothing to check.
async fn check_dokploy(client: &reqwest::Client, target: &DokployTarget) -> Option<Vec<Observation>> {
    let base = normalize_url(&target.url)?;
    let key = secrets::get(&secret_key(target))?;
    let name = target.name.trim();
    let instance = |problem: Option<String>| Observation {
        key: format!("dokploy:{name}"),
        label: format!("Dokploy {name}"),
        problem,
    };
    let resp = match client.get(format!("{base}/api/project.all")).header("x-api-key", key).send().await {
        Ok(r) => r,
        Err(e) => return Some(vec![instance(Some(describe(&e)))]),
    };
    let code = resp.status().as_u16();
    if code == 401 || code == 403 || code >= 500 {
        return Some(vec![instance(Some(format!("HTTP {code}")))]);
    }
    let mut obs = vec![instance(None)];
    if let Ok(body) = resp.json::<Value>().await {
        obs.extend(parse_dokploy(name, &body));
    }
    Some(obs)
}

async fn run_checks(settings: &Settings) -> Vec<Observation> {
    let Ok(client) = client() else { return Vec::new() };
    let mut out = Vec::new();
    for site in settings.monitor_sites.iter().filter_map(|s| normalize_url(s)) {
        out.push(check_site(&client, &site).await);
    }
    for target in &settings.monitor_dokploy {
        if target.name.trim().is_empty() {
            continue;
        }
        if let Some(obs) = check_dokploy(&client, target).await {
            out.extend(obs);
        }
    }
    out
}

pub fn has_targets(s: &Settings) -> bool {
    !s.monitor_sites.is_empty() || !s.monitor_dokploy.is_empty()
}

// ── Test now (settings window) ────────────────────────────────────────────────

#[derive(Serialize)]
pub struct CheckResult {
    pub label: String,
    pub ok: bool,
    pub detail: String,
}

/// One-off check for the "Test now" button; leaves the alert state alone.
pub async fn check_now(settings: &Settings) -> Vec<CheckResult> {
    let mut results: Vec<CheckResult> = run_checks(settings)
        .await
        .into_iter()
        .map(|o| CheckResult {
            ok: o.problem.is_none(),
            detail: o.problem.unwrap_or_else(|| "OK".into()),
            label: o.label,
        })
        .collect();
    for t in &settings.monitor_dokploy {
        if !t.name.trim().is_empty() && !secrets::present(&secret_key(t)) {
            results.push(CheckResult {
                label: format!("Dokploy {}", t.name.trim()),
                ok: false,
                detail: "no API key saved".into(),
            });
        }
    }
    results
}

// ── The loop ──────────────────────────────────────────────────────────────────

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last_run: Option<Instant> = None;
        let mut trackers: HashMap<String, Tracker> = HashMap::new();

        loop {
            tokio::time::sleep(TICK).await;

            let Some(shared) = app.try_state::<crate::Shared>() else { continue };
            let settings = shared.settings.lock().unwrap().clone();
            if !settings.monitor || !has_targets(&settings) {
                // Off: forget everything so turning it back on starts clean.
                last_run = None;
                trackers.clear();
                continue;
            }
            let every = Duration::from_secs(u64::from(settings.monitor_minutes.clamp(1, 60)) * 60);
            if last_run.is_some_and(|t| t.elapsed() < every) {
                continue;
            }
            last_run = Some(Instant::now());

            let observations = run_checks(&settings).await;
            crate::log::line(format!("monitor: checked {} targets", observations.len()));
            for o in observations {
                let event = trackers.entry(o.key).or_default().step(o.problem.is_none());
                let Some(text) = alert_text(&o.label, &event, o.problem.as_deref().unwrap_or("")) else {
                    continue;
                };
                // Alerts ignore quiet hours: the user asked to be told.
                crate::log::line(format!("nudge (alert): {event:?}"));
                let kind = if event == Event::Down { "alert" } else { "recovered" };
                island::emit_all(&app, "nudge", Nudge { text, kind });
                // Let each one land before the next replaces it.
                tokio::time::sleep(Duration::from_secs(6)).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn needs_two_failures_and_alerts_once() {
        let mut t = Tracker::default();
        assert_eq!(t.step(false), Event::None);
        assert_eq!(t.step(false), Event::Down);
        assert_eq!(t.step(false), Event::None);
        assert_eq!(t.step(false), Event::None);
    }

    #[test]
    fn a_blip_does_not_alert() {
        let mut t = Tracker::default();
        assert_eq!(t.step(false), Event::None);
        assert_eq!(t.step(true), Event::None);
        assert_eq!(t.step(false), Event::None);
    }

    #[test]
    fn recovery_is_announced_once_then_a_new_incident_alerts_again() {
        let mut t = Tracker::default();
        t.step(false);
        t.step(false);
        assert_eq!(t.step(true), Event::Recovered);
        assert_eq!(t.step(true), Event::None);
        t.step(false);
        assert_eq!(t.step(false), Event::Down);
    }

    #[test]
    fn healthy_from_the_start_is_silent() {
        let mut t = Tracker::default();
        for _ in 0..5 {
            assert_eq!(t.step(true), Event::None);
        }
    }

    #[test]
    fn urls_are_normalized() {
        assert_eq!(normalize_url("example.com"), Some("https://example.com".into()));
        assert_eq!(normalize_url(" http://a.io/ "), Some("http://a.io".into()));
        assert_eq!(normalize_url("ftp://a.io"), None);
        assert_eq!(normalize_url("  "), None);
        assert_eq!(normalize_url("not a url"), None);
    }

    #[test]
    fn site_status_rules() {
        assert_eq!(site_problem(Ok(200)), None);
        assert_eq!(site_problem(Ok(404)), None);
        assert_eq!(site_problem(Ok(503)), Some("HTTP 503".into()));
        assert_eq!(site_problem(Err("timed out".into())), Some("timed out".into()));
    }

    #[test]
    fn dokploy_errors_name_project_and_service() {
        let body = json!([{
            "name": "shop",
            "applications": [
                {"applicationId": "a1", "name": "web", "applicationStatus": "error"},
                {"applicationId": "a2", "name": "api", "applicationStatus": "done"}
            ],
            "compose": [{"composeId": "c1", "name": "stack", "composeStatus": "running"}],
            "postgres": [{"name": "db", "applicationStatus": "error"}]
        }]);
        let obs = parse_dokploy("vps1", &body);
        assert_eq!(obs.len(), 4);
        let bad: Vec<_> = obs.iter().filter(|o| o.problem.is_some()).map(|o| o.label.as_str()).collect();
        assert_eq!(bad, vec!["shop / web (vps1)", "shop / db (vps1)"]);
    }

    #[test]
    fn dokploy_nested_environments_are_walked() {
        let body = json!([{"name": "p", "environments": [{"applications": [{"name": "x", "applicationStatus": "error"}]}]}]);
        assert_eq!(parse_dokploy("v", &body).len(), 1);
    }

    #[test]
    fn dokploy_odd_shapes_never_panic() {
        for body in [json!(null), json!({}), json!([1, "a", null]), json!([{"applications": "nope"}]), json!([{"applications": [{}]}])] {
            let _ = parse_dokploy("v", &body);
        }
    }

    #[test]
    fn alert_texts() {
        assert_eq!(alert_text("a.com", &Event::Down, "HTTP 502"), Some("a.com is down: HTTP 502".into()));
        assert_eq!(alert_text("a.com", &Event::Recovered, ""), Some("a.com is back up".into()));
        assert_eq!(alert_text("a.com", &Event::None, ""), None);
    }
}
