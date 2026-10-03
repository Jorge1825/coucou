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
