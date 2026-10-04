// Preferences, stored as plain JSON in %APPDATA%\Coucou\settings.json.
// No secret ever lands here — API keys live in the Windows Credential Manager.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub sound_enabled: bool,
    pub sound_volume: f64,
    pub auto_close_interval: f64,
    pub absence_interval: f64,
    pub active_integrations: Vec<String>,
    /// "primary" = the main display, "cursor" = whichever display the mouse is on.
    /// Only used when `all_screens` is off.
    pub screen: String,
    /// One Mochi on every display, each placed independently. On by default.
    #[serde(default = "default_true")]
    pub all_screens: bool,
    pub autostart: bool,
    pub hooks_installed: bool,
    /// Model used by the chat. Changeable in the settings window.
    /// Defaulted explicitly so a settings.json written by an older build still loads.
    #[serde(default = "default_model")]
    pub model: String,
    /// AI provider preset: anthropic (default), openai, openrouter, groq,
    /// deepseek, ollama or custom. Unknown values fall back to anthropic.
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Base URL override; empty = the preset's own URL. Never a secret.
    #[serde(default)]
    pub provider_url: String,
    /// API format for the custom preset: "anthropic" or "openai".
    #[serde(default = "default_provider_format")]
    pub provider_format: String,
    /// What Mochi wears. Plain ids understood by the front end ("none" = nothing).
    #[serde(default = "default_none")]
    pub mochi_hat: String,
    #[serde(default = "default_none")]
    pub mochi_face: String,
    #[serde(default = "default_none")]
    pub mochi_neck: String,
    /// Mochi may interrupt on its own (a periodic check-in with the model).
    /// Off by default: it makes background requests to the configured provider.
    #[serde(default)]
    pub proactive: bool,
    /// Minutes between check-ins when `proactive` is on.
    #[serde(default = "default_proactive_minutes")]
    pub proactive_minutes: u32,
    /// Show what Spotify is playing and its controls. Read locally through
    /// Windows' media sessions — no account, no network.
    #[serde(default = "default_true")]
    pub spotify: bool,
    /// Transparency, 0…1 (1 = opaque): the island's black background, the
    /// cards inside it, and the whole island while the mouse is elsewhere.
    #[serde(default = "default_one")]
    pub island_opacity: f64,
    #[serde(default = "default_one")]
    pub card_opacity: f64,
    #[serde(default = "default_one")]
    pub idle_opacity: f64,
    /// Global shortcut that opens the chat; empty = none.
    #[serde(default = "default_chat_hotkey")]
    pub chat_hotkey: String,
    /// Seconds in the compact island before it hides completely; 0 = never.
    /// Never by default: a fully hidden island is easy to lose.
    #[serde(default)]
    pub hide_after: u32,
    /// Show Windows notifications in the island (needs Windows' own permission).
    #[serde(default = "default_true")]
    pub notifications: bool,
    /// Silenced: notifications are listed, but never pop the island or chime.
    #[serde(default)]
    pub notifications_muted: bool,
    /// What a notification does on screen: "discreet" (a small banner on the
    /// compact island for a few seconds), "expand" (open the island) or "bell"
    /// (nothing on screen, only the dot on the bell).
    #[serde(default = "default_notifications_style")]
    pub notifications_style: String,
    /// A soft chime of our own. Off by default: the app that sent the
    /// notification usually makes its own sound already.
    #[serde(default)]
    pub notifications_chime: bool,
    /// Quiet hours: between these hours (local, 0–23) nothing pops up.
    #[serde(default)]
    pub notifications_quiet: bool,
    #[serde(default = "default_quiet_from")]
    pub notifications_quiet_from: u32,
    #[serde(default = "default_quiet_to")]
    pub notifications_quiet_to: u32,
    /// Apps whose notifications are dropped entirely, by display name.
    #[serde(default)]
    pub notifications_muted_apps: Vec<String>,
    /// Interface language: "auto" (follow Windows), "en", "es", "ru" or "zh".
    /// Mochi's chat answers in it too.
    #[serde(default = "default_language")]
    pub language: String,
    /// Smart clipboard: suggestions when text is copied, and a short history.
    #[serde(default)]
    pub clipboard: ClipboardPrefs,
    /// Battery, CPU, memory and network at a glance — only when something is off.
    #[serde(default)]
    pub system: SystemPrefs,
    /// Claude Code: one pill per session, and what each finished run changed.
    #[serde(default)]
    pub sessions: SessionPrefs,
    /// Upcoming meetings from an iCal (.ics) address.
    #[serde(default)]
    pub calendar: CalendarPrefs,
    /// Mochi reacting to your day: breaks, celebrations, birthday, seasons.
    #[serde(default)]
    pub day: DayPrefs,
    /// What Mochi puts on by itself: pyjamas at night, umbrella in the rain, gamer gear.
    #[serde(default)]
    pub outfits: OutfitPrefs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClipboardPrefs {
    /// Off by default: reading the clipboard is something the user opts into.
    pub enabled: bool,
    /// Show the little banner with actions when text is copied.
    pub suggest: bool,
    /// Shorter copies are ignored.
    pub min_chars: u32,
    /// Offered actions: explain, summarize, translate, fix.
    pub actions: Vec<String>,
    /// Target language for "translate": "auto" = the interface language.
    pub translate_to: String,
    /// How many copies the history keeps, and for how long (minutes).
    pub history: u32,
    pub keep_minutes: u32,
}

impl Default for ClipboardPrefs {
    fn default() -> Self {
        Self {
            enabled: false,
            suggest: true,
            min_chars: 20,
            actions: vec!["explain".into(), "summarize".into(), "translate".into(), "fix".into()],
            translate_to: "auto".into(),
            history: 15,
            keep_minutes: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SystemPrefs {
    pub enabled: bool,
    /// Warn under this battery level (percent) while not charging; 0 = never.
    pub battery_low: u32,
    /// Warn when the CPU stays above this (percent) for a while; 0 = never.
    pub cpu_high: u32,
    /// Warn above this memory use (percent); 0 = never.
    pub memory_high: u32,
    /// Warn when the internet connection drops.
    pub offline: bool,
    /// Mochi shows it too (sweat, yawn…), not just the banner.
    pub react: bool,
}

impl Default for SystemPrefs {
    fn default() -> Self {
        Self { enabled: true, battery_low: 20, cpu_high: 90, memory_high: 90, offline: true, react: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionPrefs {
    /// One pill per Claude Code session instead of a single shared one.
    pub separate: bool,
    /// Most session pills at once.
    pub max: u32,
    /// An extra session pill leaves this long after its last activity (minutes).
    pub linger_minutes: u32,
    /// Show files changed and lines added/removed when a run finishes (git).
    pub git_summary: bool,
}

impl Default for SessionPrefs {
    fn default() -> Self {
        Self { separate: true, max: 4, linger_minutes: 30, git_summary: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CalendarPrefs {
    /// Off until an iCal address is set (it lives in the Credential Manager).
    pub enabled: bool,
    /// Banner this many minutes before a meeting; 0 = no banner.
    pub remind_minutes: u32,
    /// Countdown on the compact island during the last minutes.
    pub countdown: bool,
    /// Show all-day events too.
    pub all_day: bool,
}

impl Default for CalendarPrefs {
    fn default() -> Self {
        Self { enabled: false, remind_minutes: 5, countdown: true, all_day: false }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DayPrefs {
    pub enabled: bool,
    /// Suggest a break after this many minutes of continuous use; 0 = never.
    pub break_minutes: u32,
    /// Celebrate milestones (finished Claude Code runs).
    pub celebrate: bool,
    /// Your birthday as "MM-DD"; empty = not set.
    pub birthday: String,
    /// Seasonal outfits (scarf in winter, Halloween, Christmas…).
    pub seasonal: bool,
    /// "north" or "south" — decides when winter is.
    pub hemisphere: String,
}

impl Default for DayPrefs {
    fn default() -> Self {
        Self {
            enabled: true,
            break_minutes: 90,
            celebrate: true,
            birthday: String::new(),
            seasonal: true,
            hemisphere: "north".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OutfitPrefs {
    /// Mochi may change outfit by itself (overrides the hat you picked meanwhile).
    pub auto: bool,
    /// Nightcap between these hours (local, 0–23).
    pub night: bool,
    pub night_from: u32,
    pub night_to: u32,
    /// Umbrella when it rains, from Open-Meteo for `city`. Off until a city is set.
    pub weather: bool,
    pub city: String,
    pub latitude: f64,
    pub longitude: f64,
    /// Gamer headset while a game runs full screen.
    pub gamer: bool,
}

impl Default for OutfitPrefs {
    fn default() -> Self {
        Self {
            auto: true,
            night: true,
            night_from: 22,
            night_to: 7,
            weather: false,
            city: String::new(),
            latitude: 0.0,
            longitude: 0.0,
            gamer: true,
        }
    }
}

fn default_one() -> f64 {
    1.0
}

fn default_true() -> bool {
    true
}

fn default_notifications_style() -> String {
    "discreet".into()
}

fn default_quiet_from() -> u32 {
    22
}

fn default_quiet_to() -> u32 {
    8
}

fn default_language() -> String {
    "auto".into()
}

fn default_chat_hotkey() -> String {
    crate::hotkey::DEFAULT.into()
}

fn default_proactive_minutes() -> u32 {
    30
}

fn default_none() -> String {
    "none".into()
}

fn default_model() -> String {
    crate::claude::DEFAULT_MODEL.to_string()
}

fn default_provider() -> String {
    "anthropic".into()
}

fn default_provider_format() -> String {
    "openai".into()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sound_enabled: true,
            sound_volume: 0.12,
            auto_close_interval: 15.0,
            absence_interval: 180.0,
            active_integrations: vec![
                "integration_resend".into(),
                "integration_n8n".into(),
                "integration_vercel".into(),
                "integration_github".into(),
            ],
            screen: "primary".into(),
            all_screens: true,
            autostart: false,
            hooks_installed: false,
            model: default_model(),
            provider: default_provider(),
            provider_url: String::new(),
            provider_format: default_provider_format(),
            mochi_hat: default_none(),
            mochi_face: default_none(),
            mochi_neck: default_none(),
            proactive: false,
            proactive_minutes: default_proactive_minutes(),
            spotify: true,
            island_opacity: 1.0,
            card_opacity: 1.0,
            idle_opacity: 1.0,
            chat_hotkey: default_chat_hotkey(),
            hide_after: 0,
            notifications: true,
            notifications_muted: false,
            notifications_style: default_notifications_style(),
            notifications_chime: false,
            notifications_quiet: false,
            notifications_quiet_from: default_quiet_from(),
            notifications_quiet_to: default_quiet_to(),
            notifications_muted_apps: Vec::new(),
            language: default_language(),
            clipboard: ClipboardPrefs::default(),
            system: SystemPrefs::default(),
            sessions: SessionPrefs::default(),
            calendar: CalendarPrefs::default(),
            day: DayPrefs::default(),
            outfits: OutfitPrefs::default(),
        }
    }
}

/// %APPDATA%\Coucou
pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Coucou")
}

/// %LOCALAPPDATA%\Coucou — where coucou-hook.exe and the log live.
pub fn local_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Coucou")
}

pub fn hook_exe_path() -> PathBuf {
    local_dir().join("bin").join("coucou-hook.exe")
}

fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

pub fn load() -> Settings {
    match std::fs::read(settings_path()) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

pub fn save(settings: &Settings) -> std::io::Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let json = serde_json::to_vec_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(settings_path(), json)
}
