// Weather, for Mochi's umbrella: Open-Meteo, free and keyless.
//
// Only while Settings → Mochi's outfits → "Umbrella when it rains" is on and a
// city was picked. The city is looked up once (geocoding), then the current
// weather is read every half hour. Nothing else is sent: just coordinates.

use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::island;

const EVERY: Duration = Duration::from_secs(30 * 60);
const IDLE_EVERY: Duration = Duration::from_secs(60);

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Weather {
    /// WMO weather code.
    pub code: i64,
    pub temperature: f64,
    pub rain: bool,
    pub snow: bool,
    pub storm: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub name: String,
    pub country: String,
    pub latitude: f64,
    pub longitude: f64,
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(10)).build().unwrap_or_default()
}

/// Settings → city search. Returns the best match.
pub async fn find_city(name: &str, language: &str) -> Result<Place, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 80 {
        return Err("Type a city name.".into());
    }
    let lang = if language.len() == 2 { language } else { "en" };
    let json: Value = client()
        .get("https://geocoding-api.open-meteo.com/v1/search")
        .query(&[("name", name), ("count", "1"), ("language", lang), ("format", "json")])
        .send()
        .await
        .map_err(|_| "Could not reach the weather service.".to_string())?
        .json()
        .await
        .map_err(|_| "Unexpected answer from the weather service.".to_string())?;
    let r = json.get("results").and_then(|r| r.get(0)).ok_or("City not found.")?;
    Ok(Place {
        name: r.get("name").and_then(Value::as_str).unwrap_or(name).to_string(),
        country: r.get("country").and_then(Value::as_str).unwrap_or_default().to_string(),
        latitude: r.get("latitude").and_then(Value::as_f64).ok_or("City not found.")?,
        longitude: r.get("longitude").and_then(Value::as_f64).ok_or("City not found.")?,
    })
}

async fn current(lat: f64, lon: f64) -> Option<Weather> {
    let json: Value = client()
        .get("https://api.open-meteo.com/v1/forecast")
        .query(&[
            ("latitude", format!("{lat:.3}")),
            ("longitude", format!("{lon:.3}")),
            ("current", "weather_code,temperature_2m".to_string()),
        ])
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    let c = json.get("current")?;
    let code = c.get("weather_code")?.as_i64()?;
    Some(Weather {
        code,
        temperature: c.get("temperature_2m").and_then(Value::as_f64).unwrap_or(0.0),
        rain: matches!(code, 51..=67 | 80..=82),
        snow: matches!(code, 71..=77 | 85 | 86),
        storm: matches!(code, 95..=99),
    })
}

/// (on, latitude, longitude)
fn wanted(app: &AppHandle) -> Option<(f64, f64)> {
    let shared = app.try_state::<crate::Shared>()?;
    let s = shared.settings.lock().unwrap();
    let o = &s.outfits;
    (o.auto && o.weather && !o.city.is_empty()).then_some((o.latitude, o.longitude))
}

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last: Option<((f64, f64), std::time::Instant)> = None;
        loop {
            let paused = crate::integrations::PAUSED.load(Ordering::Relaxed);
            if let Some(at) = wanted(&app).filter(|_| !paused) {
                let due = last.is_none_or(|(p, t)| p != at || t.elapsed() >= EVERY);
                if due {
                    if let Some(w) = current(at.0, at.1).await {
                        island::emit_all(&app, "weather", w);
                    }
                    last = Some((at, std::time::Instant::now()));
                }
            }
            tokio::time::sleep(IDLE_EVERY).await;
        }
    });
}
