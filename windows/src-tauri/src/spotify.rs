// Spotify — what's playing, and the play/pause/next/previous buttons.
//
// Goes through Windows' own media session manager (the same thing the volume
// flyout uses), so it works with the Spotify desktop app as installed, needs no
// account, no API key and makes no network call. Only the Spotify session is
// ever looked at; every other app's media stays invisible to us.

use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager_,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::DataReader;
use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;

use crate::log;

/// How often the session is read. Cheap (no I/O), and the front end
/// interpolates the progress bar in between.
const EVERY: Duration = Duration::from_millis(1500);

/// How often to look for Spotify while it isn't open.
const IDLE_EVERY: Duration = Duration::from_secs(5);

/// Album covers bigger than this are skipped rather than shipped to the webview.
const MAX_ART: u32 = 512 * 1024;

#[derive(Serialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct NowPlaying {
    /// Spotify has a media session at all (the app is open).
    pub active: bool,
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Milliseconds, as of the moment this was read.
    pub position_ms: u64,
    pub duration_ms: u64,
    /// `data:` URL of the cover, when Spotify hands one over.
    pub art: Option<String>,
}

/// The cover is only fetched again when the track changes.
static ART: Mutex<Option<(String, Option<String>)>> = Mutex::new(None);

fn enabled(app: &AppHandle) -> bool {
    app.try_state::<crate::Shared>()
        .map(|shared| shared.settings.lock().unwrap().spotify)
        .unwrap_or(false)
}

/// Reads the session on a dedicated thread: the WinRT calls block, and they have
/// nothing to do on the async runtime.
pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        let mut manager: Option<Manager_> = None;
        let mut last = NowPlaying::default();
        loop {
            // Spotify closed: nothing to follow closely, look again now and then.
            std::thread::sleep(if last.active { EVERY } else { IDLE_EVERY });
            if crate::integrations::PAUSED.load(Ordering::Relaxed) || !enabled(&app) {
                if last.active {
                    last = NowPlaying::default();
                    crate::island::emit_all(&app, "spotify", &last);
                }
                continue;
            }
            if manager.is_none() {
                manager = request_manager();
            }
            let Some(m) = manager.as_ref() else { continue };
            let now = read(m).unwrap_or_default();
            // Position moves on every read while playing; the front end only
            // needs it when something else changed or it drifted.
            let drifted = now.position_ms.abs_diff(expected(&last)) > 2500;
            let same = NowPlaying { position_ms: 0, ..now.clone() }
                == NowPlaying { position_ms: 0, ..last.clone() };
            if !same || drifted {
                crate::island::emit_all(&app, "spotify", &now);
            }
            last = now;
        }
    });
}

/// Where `last` should be by now if it kept playing.
fn expected(last: &NowPlaying) -> u64 {
    if last.playing {
        last.position_ms + EVERY.as_millis() as u64
    } else {
        last.position_ms
    }
}

fn request_manager() -> Option<Manager_> {
    match Manager_::RequestAsync().and_then(|op| op.get()) {
        Ok(m) => Some(m),
        Err(err) => {
            log::line(format!("spotify: no media session manager: {err}"));
            None
        }
    }
}

/// The Spotify session, whichever way it was installed (Spotify.exe or the
/// Store package `SpotifyAB.SpotifyMusic_…`).
fn spotify_session(manager: &Manager_) -> Option<Session> {
    let sessions = manager.GetSessions().ok()?;
    (0..sessions.Size().ok()?).find_map(|i| {
        let s = sessions.GetAt(i).ok()?;
        let id = s.SourceAppUserModelId().ok()?.to_string().to_lowercase();
        id.contains("spotify").then_some(s)
    })
}

fn read(manager: &Manager_) -> Option<NowPlaying> {
    let session = spotify_session(manager)?;
    let props = session.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
    let title = props.Title().map(|s| s.to_string()).unwrap_or_default();
    let artist = props.Artist().map(|s| s.to_string()).unwrap_or_default();
    let album = props.AlbumTitle().map(|s| s.to_string()).unwrap_or_default();

    let playing = session
        .GetPlaybackInfo()
        .and_then(|i| i.PlaybackStatus())
        .map(|s| s == Status::Playing)
        .unwrap_or(false);

    let (mut position_ms, mut duration_ms) = (0u64, 0u64);
    if let Ok(t) = session.GetTimelineProperties() {
        let ticks = |d: windows::Foundation::TimeSpan| (d.Duration.max(0) / 10_000) as u64;
        position_ms = t.Position().map(ticks).unwrap_or(0);
        duration_ms = t.EndTime().map(ticks).unwrap_or(0);
        // The position is as of LastUpdatedTime; bring it up to now.
        if playing {
            if let Ok(updated) = t.LastUpdatedTime() {
                let now = now_filetime();
                if now > updated.UniversalTime {
                    position_ms += ((now - updated.UniversalTime) / 10_000) as u64;
                }
            }
        }
        if duration_ms > 0 {
            position_ms = position_ms.min(duration_ms);
        }
    }

    let key = format!("{title}\u{1}{artist}\u{1}{album}");
    let art = {
        let mut cache = ART.lock().unwrap();
        match cache.as_ref() {
            Some((k, art)) if *k == key => art.clone(),
            _ => {
                let art = cover(&props);
                // Spotify sometimes publishes the title before the cover; only
                // remember a miss once the cover had a fair chance to arrive.
                if art.is_some() || title.is_empty() {
                    *cache = Some((key, art.clone()));
                }
                art
            }
        }
    };

    Some(NowPlaying { active: true, playing, title, artist, album, position_ms, duration_ms, art })
}

fn cover(
    props: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties,
) -> Option<String> {
    let thumb = props.Thumbnail().ok()?;
    let stream = thumb.OpenReadAsync().ok()?.get().ok()?;
    let size = u32::try_from(stream.Size().ok()?).ok()?;
    if size == 0 || size > MAX_ART {
        return None;
    }
    let mime = stream.ContentType().map(|s| s.to_string()).unwrap_or_default();
    let mime = if mime.starts_with("image/") { mime } else { "image/jpeg".into() };
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    reader.LoadAsync(size).ok()?.get().ok()?;
    let mut bytes = vec![0u8; size as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    Some(format!("data:{mime};base64,{}", crate::claude::base64_for(&bytes)))
}

/// 100 ns ticks since 1601 — the same clock as WinRT's DateTime.
fn now_filetime() -> i64 {
    let ft = unsafe { GetSystemTimeAsFileTime() };
    ((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64
}

/// A button in the island. `action` is one of play_pause / next / previous.
pub fn control(action: &str) -> Result<(), String> {
    let manager = request_manager().ok_or("Windows media controls unavailable.")?;
    let session = spotify_session(&manager).ok_or("Spotify is not open.")?;
    let op = match action {
        "play_pause" => session.TryTogglePlayPauseAsync(),
        "next" => session.TrySkipNextAsync(),
        "previous" => session.TrySkipPreviousAsync(),
        _ => return Err(format!("unknown action {action}")),
    };
    let ok = op.and_then(|o| o.get()).map_err(|e| e.to_string())?;
    if ok {
        Ok(())
    } else {
        Err("Spotify ignored the request.".into())
    }
}
