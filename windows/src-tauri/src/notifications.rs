// Windows notifications → the island.
//
// Reads the toasts in the Action Center through Windows' own
// UserNotificationListener. Desktop apps get no change event from it, so the
// list is read every couple of seconds and only ids not seen before are sent on.
//
// Windows asks the user first: until "Notification access" is on in
// Settings → Privacy & security → Notifications, the status stays Denied and
// nothing is read. The contents never leave this machine and never reach the
// log — only the island shows them.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};
use windows::Foundation::Size;
use windows::Storage::Streams::DataReader;
use windows::UI::Notifications::Management::{
    UserNotificationListener, UserNotificationListenerAccessStatus as Access,
};
use windows::UI::Notifications::{KnownNotificationBindings, NotificationKinds, UserNotification};

use crate::island;
use crate::log;

/// While access is allowed and the feature is on.
const EVERY: Duration = Duration::from_secs(2);
/// While access is denied or the feature is off: only watch for that to change.
const IDLE_EVERY: Duration = Duration::from_secs(8);
/// App logos bigger than this are left out.
const MAX_ICON: u32 = 64 * 1024;
/// Caps on what is remembered per app, so a long session can't grow them forever.
const MAX_APPS: usize = 50;
const MAX_CACHED_ICONS: usize = 32;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OsNotification {
    pub id: u32,
    pub app: String,
    pub title: String,
    pub body: String,
    /// Unix milliseconds.
    pub time: i64,
    /// `data:` URL of the app's logo, when Windows has one.
    pub icon: Option<String>,
    /// Everything is silenced: show it in the list, don't interrupt.
    pub muted: bool,
    /// Windows itself says not now: Do not disturb, or an app is full screen.
    pub quiet: bool,
}

/// Apps seen this session, for the per-app mute list in Settings.
static SEEN_APPS: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Logo per app name.
static ICONS: std::sync::LazyLock<Mutex<HashMap<String, Option<String>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static STATUS: Mutex<&'static str> = Mutex::new("unknown");

fn listener() -> Option<UserNotificationListener> {
    UserNotificationListener::Current().ok()
}

fn status_word(s: Access) -> &'static str {
    match s {
        Access::Allowed => "allowed",
        Access::Denied => "denied",
        _ => "unspecified",
    }
}

/// "allowed", "denied", "unspecified" or "unavailable".
pub fn status() -> String {
    match listener().and_then(|l| l.GetAccessStatus().ok()) {
        Some(s) => status_word(s).to_string(),
        None => "unavailable".to_string(),
    }
}

/// Asks Windows for access. For a desktop app this only succeeds once the user
/// has turned "Notification access" on; the island then offers to open that page.
pub fn request_access() -> String {
    let Some(l) = listener() else { return "unavailable".into() };
    match l.RequestAccessAsync().and_then(|op| op.get()) {
        Ok(s) => status_word(s).to_string(),
        Err(_) => status(),
    }
}

pub fn seen_apps() -> Vec<String> {
    SEEN_APPS.lock().unwrap().clone()
}

/// (enabled, everything muted, apps muted one by one)
fn prefs(app: &AppHandle) -> (bool, bool, Vec<String>) {
    app.try_state::<crate::Shared>()
        .map(|s| {
            let s = s.settings.lock().unwrap();
            (s.notifications, s.notifications_muted, s.notifications_muted_apps.clone())
        })
        .unwrap_or((false, false, Vec::new()))
}

pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        let mut known: Option<HashSet<u32>> = None;
        let mut delay = IDLE_EVERY;
        loop {
            std::thread::sleep(delay);
            delay = IDLE_EVERY;
            let (enabled, muted, muted_apps) = prefs(&app);
            if !enabled || crate::integrations::PAUSED.load(Ordering::Relaxed) {
                known = None;
                continue;
            }
            let Some(l) = listener() else { continue };
            let word = l.GetAccessStatus().map(status_word).unwrap_or("unspecified");
            {
                let mut last = STATUS.lock().unwrap();
                if *last != word {
                    *last = word;
                    island::emit_all(&app, "os-notifications-status", word.to_string());
                }
            }
            if word != "allowed" {
                known = None;
                continue;
            }
            delay = EVERY;

            let list = match l
                .GetNotificationsAsync(NotificationKinds::Toast)
                .and_then(|op| op.get())
            {
                Ok(list) => list,
                Err(err) => {
                    log::line(format!("notifications: could not read the list: {err}"));
                    continue;
                }
            };
            let current: Vec<UserNotification> = list.into_iter().collect();
            let ids: HashSet<u32> = current.iter().filter_map(|n| n.Id().ok()).collect();

            // First read after (re)enabling: these were already there. Only what
            // arrives from now on is news.
            let Some(seen) = known.replace(ids.clone()) else { continue };
            let mut quiet: Option<bool> = None;
            for n in current {
                let Ok(id) = n.Id() else { continue };
                if seen.contains(&id) {
                    continue;
                }
                let Some(mut item) = read(&n, id) else { continue };
                remember_app(&item.app);
                if muted_apps.iter().any(|a| a == &item.app) {
                    continue;
                }
                item.muted = muted;
                item.quiet = *quiet.get_or_insert_with(system_quiet);
                island::emit_all(&app, "os-notification", item);
            }
        }
    });
}

fn remember_app(name: &str) {
    let mut apps = SEEN_APPS.lock().unwrap();
    if !apps.iter().any(|a| a == name) {
        if apps.len() >= MAX_APPS {
            apps.remove(0);
        }
        apps.push(name.to_string());
    }
}

fn read(n: &UserNotification, id: u32) -> Option<OsNotification> {
    let display = n.AppInfo().ok().and_then(|a| a.DisplayInfo().ok());
    let app = display
        .as_ref()
        .and_then(|d| d.DisplayName().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Windows".to_string());

    let texts: Vec<String> = n
        .Notification()
        .and_then(|t| t.Visual())
        .and_then(|v| v.GetBinding(&KnownNotificationBindings::ToastGeneric()?))
        .and_then(|b| b.GetTextElements())
        .map(|els| els.into_iter().filter_map(|e| e.Text().ok()).map(|s| s.to_string()).collect())
        .unwrap_or_default();
    let mut texts = texts.into_iter().filter(|s| !s.trim().is_empty());
    let title = texts.next().unwrap_or_else(|| app.clone());
    let body = texts.collect::<Vec<_>>().join("\n");

    // 100 ns ticks since 1601 → Unix milliseconds.
    let time = n
        .CreationTime()
        .map(|t| (t.UniversalTime - 116_444_736_000_000_000) / 10_000)
        .unwrap_or(0);

    let icon = {
        let mut cache = ICONS.lock().unwrap();
        if cache.len() >= MAX_CACHED_ICONS && !cache.contains_key(&app) {
            cache.clear();
        }
        cache
            .entry(app.clone())
            .or_insert_with(|| display.as_ref().and_then(logo))
            .clone()
    };

    Some(OsNotification { id, app, title, body, time, icon, muted: false, quiet: false })
}

/// Windows says now is not the time: something runs full screen (a game, a
/// video, a presentation), the screen is locked, or Do not disturb is on.
fn system_quiet() -> bool {
    use windows::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_ACCEPTS_NOTIFICATIONS};
    let busy = unsafe { SHQueryUserNotificationState() }
        .map(|s| s != QUNS_ACCEPTS_NOTIFICATIONS)
        .unwrap_or(false);
    busy || do_not_disturb()
}

/// Windows 11's Do not disturb switch turns toast banners off globally.
fn do_not_disturb() -> bool {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let mut value: u32 = 1;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Notifications\\Settings"),
            w!("NOC_GLOBAL_SETTING_TOASTS_ENABLED"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut core::ffi::c_void),
            Some(&mut size),
        )
    };
    status.is_ok() && value == 0
}

fn logo(display: &windows::ApplicationModel::AppDisplayInfo) -> Option<String> {
    let reference = display.GetLogo(Size { Width: 32.0, Height: 32.0 }).ok()?;
    let stream = reference.OpenReadAsync().ok()?.get().ok()?;
    let size = u32::try_from(stream.Size().ok()?).ok()?;
    if size == 0 || size > MAX_ICON {
        return None;
    }
    let mime = stream.ContentType().map(|s| s.to_string()).unwrap_or_default();
    let mime = if mime.starts_with("image/") { mime } else { "image/png".into() };
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    reader.LoadAsync(size).ok()?.get().ok()?;
    let mut bytes = vec![0u8; size as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    Some(format!("data:{mime};base64,{}", crate::claude::base64_for(&bytes)))
}
