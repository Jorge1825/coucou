// Island windows: one per display, each placed on its own screen with its own
// position, dock side, window size (full panel / invisible wake strip),
// click-through and cursor poll.
//
// There is no notch on a PC, so an island is a black shape drawn at the top
// centre of a display inside a borderless, transparent, always-on-top window
// that never takes focus. Every display gets its own Mochi, and each one
// remembers where the user put it on that display, independently of the others.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, EventTarget, Manager, Monitor, PhysicalPosition, PhysicalSize,
    WebviewWindow,
};

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW,
};

/// Logical size of the full window — the largest island view, like the macOS panel.
pub const PANEL_W: f64 = 720.0;
pub const PANEL_H: f64 = 380.0;
/// Logical size of the invisible strip that wakes the island when it is hidden.
pub const STRIP_W: f64 = 240.0;
pub const STRIP_H: f64 = 6.0;

/// Logical width of the expanded island drawn inside the panel window.
const EXPANDED_W: f64 = 640.0;

/// The first island (main display). The others are `island-1`, `island-2`, …
pub const WINDOW_LABEL: &str = "island";
/// Beyond this many displays the extra ones go without a Mochi.
pub const MAX_ISLANDS: usize = 8;

pub fn label_for(index: usize) -> String {
    if index == 0 {
        WINDOW_LABEL.to_string()
    } else {
        format!("{WINDOW_LABEL}-{index}")
    }
}

fn is_island_label(label: &str) -> bool {
    label == WINDOW_LABEL || label.starts_with("island-")
}

// ── The islands ──────────────────────────────────────────────────────────────

/// Everything one island window owns.
pub struct IslandWin {
    pub label: String,
    pub gate: Arc<PollGate>,
    /// Key of the display this island lives on (`None` = no display: hidden).
    monitor: Mutex<Option<String>>,
    dragging: AtomicBool,
    /// -1 = docked to the left edge, 1 = right edge, 0 = free.
    dock: AtomicI32,
    /// WebView zoom correction, see `fit_zoom` in lib.rs.
    pub zoom: Mutex<f64>,
    /// Where the window was last placed, so the 60 Hz cursor poll never has to
    /// ask the UI thread (every window getter is a round trip to it, and three
    /// of them per tick per island made native window drags stutter).
    frame: Mutex<Option<Frame>>,
}

/// Window origin and size (physical px), its scale, and its display's rect.
#[derive(Clone, Copy)]
struct Frame {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    scale: f64,
    monitor: (i32, i32, i32, i32),
}

static ISLANDS: Mutex<Vec<Arc<IslandWin>>> = Mutex::new(Vec::new());

/// Registers an island window that already exists. Called once per window at launch.
pub fn register(label: &str) -> Arc<IslandWin> {
    let iw = Arc::new(IslandWin {
        label: label.to_string(),
        gate: Arc::new(PollGate::new()),
        monitor: Mutex::new(None),
        dragging: AtomicBool::new(false),
        dock: AtomicI32::new(0),
        zoom: Mutex::new(1.0),
        frame: Mutex::new(None),
    });
    ISLANDS.lock().unwrap().push(iw.clone());
    iw
}

pub fn get(label: &str) -> Option<Arc<IslandWin>> {
    ISLANDS.lock().unwrap().iter().find(|i| i.label == label).cloned()
}

pub fn all() -> Vec<Arc<IslandWin>> {
    ISLANDS.lock().unwrap().clone()
}

pub fn window_of(app: &AppHandle, iw: &IslandWin) -> Option<WebviewWindow> {
    app.get_webview_window(&iw.label)
}

/// Sends an event to every island (hooks, integrations, nudges, …): each
/// display's Mochi knows the same things.
pub fn emit_all<S: Serialize + Clone>(app: &AppHandle, event: &str, payload: S) {
    let _ = app.emit_filter(event, payload, |t| match t {
        EventTarget::WebviewWindow { label }
        | EventTarget::Webview { label }
        | EventTarget::Window { label } => is_island_label(label),
        _ => false,
    });
}

/// The island on the display under the cursor, else the first one shown.
pub fn label_under_cursor(app: &AppHandle) -> String {
    let islands = all();
    let under_cursor = cursor_physical().and_then(|(cx, cy)| {
        islands
            .iter()
            .find(|iw| monitor_of(app, iw).is_some_and(|m| monitor_contains(&m, cx, cy)))
    });
    under_cursor
        .or_else(|| islands.iter().find(|iw| monitor_of(app, iw).is_some()))
        .map(|iw| iw.label.clone())
        .unwrap_or_else(|| WINDOW_LABEL.to_string())
}

/// Sends an event to the island on the display under the cursor (tray "Open",
/// a second launch): only the Mochi the user is looking at should answer.
pub fn emit_focused<S: Serialize + Clone>(app: &AppHandle, event: &str, payload: S) {
    let target = label_under_cursor(app);
    let _ = app.emit_to(target.as_str(), event, payload);
}

pub fn dock(iw: &IslandWin) -> i32 {
    iw.dock.load(Ordering::SeqCst)
}

// ── Displays ─────────────────────────────────────────────────────────────────

/// Stable name of a display (`\\.\DISPLAY2`), so a saved position survives the
/// displays being reordered.
fn monitor_key(m: &Monitor) -> String {
    m.name()
        .cloned()
        .unwrap_or_else(|| format!("{},{}", m.position().x, m.position().y))
}

/// The main display first, then the others left to right, top to bottom.
pub fn ordered_monitors(app: &AppHandle) -> Vec<Monitor> {
    let mut monitors = app.available_monitors().unwrap_or_default();
    let primary = app.primary_monitor().ok().flatten().map(|m| monitor_key(&m));
    monitors.sort_by_key(|m| {
        let p = m.position();
        (Some(monitor_key(m)) != primary, p.x, p.y)
    });
    monitors
}

/// (one island per display?, which display otherwise)
fn screen_prefs(app: &AppHandle) -> (bool, String) {
    app.try_state::<crate::Shared>()
        .map(|s| {
            let s = s.settings.lock().unwrap();
            (s.all_screens, s.screen.clone())
        })
        .unwrap_or((true, "primary".into()))
}

/// The display an island lives on right now, or `None` when it has none
/// (fewer displays than islands, or the extra islands in single-display mode).
pub fn monitor_of(app: &AppHandle, iw: &IslandWin) -> Option<Monitor> {
    let (all_screens, pref) = screen_prefs(app);
    if !all_screens {
        return if iw.label == WINDOW_LABEL { target_monitor(app, &pref) } else { None };
    }
    let key = iw.monitor.lock().unwrap().clone()?;
    app.available_monitors().ok()?.into_iter().find(|m| monitor_key(m) == key)
}

/// Hands each island its display, places the ones that have one and hides the
/// rest. Run at launch, when settings change and when displays come and go.
pub fn assign_monitors(app: &AppHandle) {
    let monitors = ordered_monitors(app);
    let islands = all();
    if monitors.len() > islands.len() {
        crate::log::line(format!(
            "{} displays but {} islands — restart Coucou to cover the new ones",
            monitors.len(),
            islands.len()
        ));
    }
    for (i, iw) in islands.iter().enumerate() {
        *iw.monitor.lock().unwrap() = monitors.get(i).map(monitor_key);
        let Some(win) = window_of(app, iw) else { continue };
        let collapsed = iw.gate.collapsed.load(Ordering::Relaxed);
        if monitor_of(app, iw).is_some() {
            apply_geometry(app, iw, collapsed);
            if !win.is_visible().unwrap_or(false) {
                let _ = win.show();
            }
            iw.gate.forget_ignore_state();
            iw.gate.set_active(!collapsed);
        } else {
            iw.gate.set_active(false);
            let _ = win.hide();
        }
    }
}

// ── User placement ───────────────────────────────────────────────────────────

/// Offset (logical px) of each display's island from its default top-centre
/// spot, keyed by display.
static POSITIONS: LazyLock<Mutex<HashMap<String, (f64, f64)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn offset(key: &str) -> (f64, f64) {
    POSITIONS.lock().unwrap().get(key).copied().unwrap_or((0.0, 0.0))
}

fn set_offset(key: &str, value: (f64, f64)) {
    POSITIONS.lock().unwrap().insert(key.to_string(), value);
}

fn position_path() -> std::path::PathBuf {
    crate::settings::config_dir().join("position.json")
}

/// Reads the saved positions. A file from the single-island days holds one
/// `[x, y]`; it becomes the main display's position.
pub fn load_positions(app: &AppHandle) {
    let Ok(bytes) = std::fs::read(position_path()) else { return };
    if let Ok(map) = serde_json::from_slice::<HashMap<String, (f64, f64)>>(&bytes) {
        *POSITIONS.lock().unwrap() = map;
    } else if let Ok(xy) = serde_json::from_slice::<(f64, f64)>(&bytes) {
        if let Some(m) = ordered_monitors(app).first() {
            set_offset(&monitor_key(m), xy);
        }
    }
}

fn save_positions() {
    let _ = std::fs::create_dir_all(crate::settings::config_dir());
    if let Ok(json) = serde_json::to_vec(&*POSITIONS.lock().unwrap()) {
        let _ = std::fs::write(position_path(), json);
    }
}

/// The island is about to retract: it never floats mid-screen, so it docks to the
/// nearest of the left, right and top edges of its display.
pub fn dock_nearest(app: &AppHandle, iw: &IslandWin, collapsed: bool) {
    if iw.dragging.load(Ordering::SeqCst) {
        return;
    }
    let Some(m) = monitor_of(app, iw) else { return };
    let key = monitor_key(&m);
    let scale = m.scale_factor();
    let ms = m.size();

    let (odx, ody) = offset(&key);
    let half = EXPANDED_W / 2.0 * scale;
    let mid = ms.width as f64 / 2.0;
    // Centre of the island, and its top, relative to this display.
    let cx = (mid + odx * scale).clamp(half.min(mid), (ms.width as f64 - half).max(mid));
    let top = (ody * scale).max(0.0);

    let gap_left = cx - half;
    let gap_right = ms.width as f64 - half - cx;
    let (dx, dy) = if gap_left <= gap_right && gap_left <= top {
        ((half - mid) / scale, ody) // left edge, keep the height
    } else if gap_right <= top {
        ((ms.width as f64 - half - mid) / scale, ody) // right edge
    } else {
        (odx, 0.0) // top edge, keep the horizontal position
    };
    if (dx, dy) == (odx, ody) {
        return;
    }
    set_offset(&key, (dx, dy));
    apply_geometry(app, iw, collapsed);
    save_positions();
}

/// Puts the island back at the top centre of its display.
pub fn reset_position(app: &AppHandle, iw: &IslandWin, collapsed: bool) {
    if let Some(m) = monitor_of(app, iw) {
        set_offset(&monitor_key(&m), (0.0, 0.0));
        save_positions();
    }
    apply_geometry(app, iw, collapsed);
}

// ── Shaking the island ───────────────────────────────────────────────────────

const SHAKE_REVERSALS: usize = 4;
const SHAKE_WINDOW_MS: u64 = 1400;
/// How far (logical px) the window must travel back to count as a reversal.
const SHAKE_SWING: f64 = 24.0;

/// Tells a shake from a normal drag: the window going back and forth along one
/// axis, several times, quickly. Pure, so it is unit tested.
#[derive(Default)]
struct AxisShake {
    /// Farthest point reached in the current direction.
    extreme: Option<f64>,
    /// Current direction: 1 or -1 (0 until the first stroke is long enough).
    dir: i32,
    reversals: Vec<u64>,
}

impl AxisShake {
    /// Feeds one position sample (same units as `min_swing`). True once the
    /// window has reversed `SHAKE_REVERSALS` times within `SHAKE_WINDOW_MS`.
    fn feed(&mut self, pos: f64, t_ms: u64, min_swing: f64) -> bool {
        let Some(extreme) = self.extreme else {
            self.extreme = Some(pos);
            return false;
        };
        match self.dir {
            0 => {
                if (pos - extreme).abs() >= min_swing {
                    self.dir = if pos > extreme { 1 } else { -1 };
                    self.extreme = Some(pos);
                }
            }
            d => {
                let beyond = (pos - extreme) * d as f64;
                if beyond > 0.0 {
                    self.extreme = Some(pos); // still going: push the extreme out
                } else if -beyond >= min_swing {
                    // Came back far enough: a reversal.
                    self.dir = -d;
                    self.extreme = Some(pos);
                    self.reversals.push(t_ms);
                }
            }
        }
        self.reversals.retain(|&r| t_ms.saturating_sub(r) <= SHAKE_WINDOW_MS);
        self.reversals.len() >= SHAKE_REVERSALS
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Default)]
struct ShakeDetector {
    x: AxisShake,
    y: AxisShake,
}

impl ShakeDetector {
    fn feed(&mut self, x: f64, y: f64, t_ms: u64, min_swing: f64) -> bool {
        let sx = self.x.feed(x, t_ms, min_swing);
        let sy = self.y.feed(y, t_ms, min_swing);
        if sx || sy {
            self.x.reset();
            self.y.reset();
            return true;
        }
        false
    }
}

/// Hands the move to Windows (smooth, native), then, once the button is released,
/// remembers where the island ended up and snaps it back inside its display.
pub fn begin_drag(app: &AppHandle, iw: Arc<IslandWin>) {
    let Some(win) = window_of(app, &iw) else { return };
    if iw.dragging.swap(true, Ordering::SeqCst) {
        return;
    }
    if win.start_dragging().is_err() {
        iw.dragging.store(false, Ordering::SeqCst);
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        // Watch the window while it is being dragged: shaking it makes Mochi dizzy.
        let started = std::time::Instant::now();
        let mut shake = ShakeDetector::default();
        let mut shaken = false;
        while left_button_down() {
            std::thread::sleep(Duration::from_millis(25));
            if shaken {
                continue;
            }
            if let (Some(win), Some(f)) = (window_of(&app, &iw), *iw.frame.lock().unwrap()) {
                if let Ok(pos) = win.outer_position() {
                    let swing = SHAKE_SWING * f.scale;
                    let t = started.elapsed().as_millis() as u64;
                    if shake.feed(pos.x as f64, pos.y as f64, t, swing) {
                        shaken = true;
                        let _ = app.emit_to(iw.label.as_str(), "shaken", ());
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
        if let (Some(win), Some(m)) = (window_of(&app, &iw), monitor_of(&app, &iw)) {
            if let (Ok(pos), Ok(size)) = (win.outer_position(), win.outer_size()) {
                let scale = m.scale_factor();
                let mp = m.position();
                let ms = m.size();
                // Open, the island stays wherever the user drops it on its own
                // display. It only docks to an edge when it is about to retract
                // (see `dock_nearest`).
                let cx = pos.x as f64 + size.width as f64 / 2.0;
                let dx = (cx - (mp.x as f64 + ms.width as f64 / 2.0)) / scale;
                let dy = (pos.y - mp.y) as f64 / scale;
                set_offset(&monitor_key(&m), (dx, dy));
            }
        }
        iw.dragging.store(false, Ordering::SeqCst);
        apply_geometry(&app, &iw, iw.gate.collapsed.load(Ordering::Relaxed));
        save_positions();
    });
}

/// Margin around the island that still counts as "on the island", in logical px.
/// Wider than the macOS 6 pt because a click must never be swallowed.
const HIT_MARGIN: f64 = 14.0;

#[derive(Serialize, Clone)]
pub struct CursorPayload {
    pub x: f64,
    pub y: f64,
}

#[derive(Serialize, Clone)]
pub struct ScreenInfo {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

/// The island shape in window-logical coordinates, pushed by the front end.
/// The poll thread owns the click-through decision so it lands in the same 16 ms
/// tick as the cursor read — an IPC round trip here loses clicks.
#[derive(Clone, Copy, Default)]
pub struct IslandRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Wakes / parks the cursor poll thread so a hidden island costs literally nothing.
pub struct PollGate {
    active: Mutex<bool>,
    cv: Condvar,
    pub collapsed: AtomicBool,
    pub rect: Mutex<IslandRect>,
    /// Mirrors the window flag so we only call into Win32 when it changes.
    ignoring: AtomicBool,
}

impl PollGate {
    pub fn new() -> Self {
        Self {
            active: Mutex::new(false),
            cv: Condvar::new(),
            collapsed: AtomicBool::new(true),
            rect: Mutex::new(IslandRect::default()),
            ignoring: AtomicBool::new(false),
        }
    }

    pub fn set_rect(&self, rect: IslandRect) {
        *self.rect.lock().unwrap() = rect;
    }

    /// Forces the next poll tick to re-apply the flag (after a window resize).
    pub fn forget_ignore_state(&self) {
        self.ignoring.store(false, Ordering::Relaxed);
    }

    pub fn set_active(&self, on: bool) {
        let mut guard = self.active.lock().unwrap();
        *guard = on;
        self.cv.notify_all();
    }

    fn wait_until_active(&self) {
        let mut guard = self.active.lock().unwrap();
        while !*guard {
            guard = self.cv.wait(guard).unwrap();
        }
    }

    fn is_active(&self) -> bool {
        *self.active.lock().unwrap()
    }
}

fn cursor_physical() -> Option<(f64, f64)> {
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p).ok()? };
    Some((p.x as f64, p.y as f64))
}

/// Lets dropped files reach the app again.
///
/// wry installs its drop target by walking the webview's child windows **once**,
/// when the webview is created. WebView2 creates `Chrome_RenderWidgetHostHWND`
/// later and registers its own target on it; being the innermost window, that one
/// wins, and since the page has no HTML5 drop handler it refuses everything — the
/// "no drop" cursor, with nothing reaching Tauri. Revoking it makes OLE fall
/// through to the target wry registered on the parent widget, which is the one
/// that feeds Tauri's drag events.
///
/// Cheap and idempotent, so it is simply re-run whenever a drag might be starting.
pub fn unblock_webview_drops(app: &AppHandle) {
    for iw in all() {
        let Some(win) = app.get_webview_window(&iw.label) else { continue };
        let Some(hwnd) = hwnd_of(&win) else { continue };
        crate::dropzone::install(app, &iw.label, hwnd);
    }
}

/// Re-runs `unblock_webview_drops` now and again shortly after, because WebView2
/// can re-register its own target once a resize or a late-created render widget
/// settles. The cursor poll only does this on a button press, and it is parked
/// while the island is hidden — which is exactly when a drag from Explorer starts.
pub fn schedule_unblock_drops(app: &AppHandle) {
    let handle = app.clone();
    std::thread::spawn(move || {
        for delay_ms in [0u64, 250, 1000] {
            std::thread::sleep(Duration::from_millis(delay_ms));
            let h = handle.clone();
            let _ = handle.run_on_main_thread(move || unblock_webview_drops(&h));
        }
    });
}

/// True while the left mouse button is held — the only signal we get that a
/// drag might be in flight before it reaches the window.
fn left_button_down() -> bool {
    unsafe { (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0 }
}

fn monitor_contains(m: &Monitor, x: f64, y: f64) -> bool {
    let p = m.position();
    let s = m.size();
    x >= p.x as f64
        && x < (p.x + s.width as i32) as f64
        && y >= p.y as f64
        && y < (p.y + s.height as i32) as f64
}

/// Single-display mode: the primary display, or the one under the cursor.
fn target_monitor(app: &AppHandle, pref: &str) -> Option<Monitor> {
    let monitors = app.available_monitors().ok()?;
    if pref == "cursor" {
        if let Some((cx, cy)) = cursor_physical() {
            if let Some(m) = monitors.iter().find(|m| monitor_contains(m, cx, cy)) {
                return Some(m.clone());
            }
        }
    }
    app.primary_monitor()
        .ok()
        .flatten()
        .or_else(|| monitors.into_iter().next())
}

/// Position and size, in physical pixels, of the display the island lives on.
pub fn target_rect(app: &AppHandle, iw: &IslandWin) -> Option<(i32, i32, i32, i32)> {
    let m = monitor_of(app, iw)?;
    let (p, s) = (m.position(), m.size());
    Some((p.x, p.y, s.width as i32, s.height as i32))
}

pub fn screen_info(app: &AppHandle, iw: &IslandWin) -> ScreenInfo {
    match monitor_of(app, iw) {
        Some(m) => {
            let scale = m.scale_factor();
            let p = m.position();
            let s = m.size();
            ScreenInfo {
                x: p.x as f64 / scale,
                y: p.y as f64 / scale,
                width: s.width as f64 / scale,
                height: s.height as f64 / scale,
                scale,
            }
        }
        None => ScreenInfo { x: 0.0, y: 0.0, width: 1920.0, height: 1080.0, scale: 1.0 },
    }
}

/// Places and sizes one island window. `collapsed` picks the wake strip instead
/// of the panel. An island without a display is simply hidden.
pub fn apply_geometry(app: &AppHandle, iw: &IslandWin, collapsed: bool) {
    let Some(win) = window_of(app, iw) else { return };
    let Some(m) = monitor_of(app, iw) else {
        let _ = win.hide();
        return;
    };

    let scale = m.scale_factor();
    let mp = *m.position();
    let ms = *m.size();

    // Where the user left this display's island, as a logical offset from the
    // default spot (top centre of the display). Clamped so the whole 640 px
    // island stays on this display whatever its size or scale.
    let (odx, ody) = offset(&monitor_key(&m));
    let half = EXPANDED_W / 2.0 * scale;
    let mid = mp.x as f64 + ms.width as f64 / 2.0;
    let left_lim = (mp.x as f64 + half).min(mid);
    let right_lim = (mp.x as f64 + ms.width as f64 - half).max(mid);
    let cx = (mid + odx * scale).clamp(left_lim, right_lim);
    let max_y = (ms.height as f64 - PANEL_H * scale).max(0.0);
    let y = mp.y + (ody * scale).clamp(0.0, max_y).round() as i32;

    // Pushed against a side edge = docked there.
    let side = if odx < 0.0 && cx <= left_lim + 0.5 {
        -1
    } else if odx > 0.0 && cx >= right_lim - 0.5 {
        1
    } else {
        0
    };
    if iw.dock.swap(side, Ordering::SeqCst) != side {
        let _ = app.emit_to(iw.label.as_str(), "dock", side);
    }

    // Retracted: the wake strip lies along the screen edge it is docked to
    // instead of across the top.
    let (lw, lh) = match (collapsed, side) {
        (false, _) => (PANEL_W, PANEL_H),
        (true, 0) => (STRIP_W, STRIP_H),
        (true, _) => (STRIP_H, STRIP_W),
    };
    let pw = (lw * scale).round().max(1.0) as u32;
    let ph = (lh * scale).round().max(1.0) as u32;
    let x = if collapsed && side < 0 {
        mp.x
    } else if collapsed && side > 0 {
        mp.x + ms.width as i32 - pw as i32
    } else {
        (cx - pw as f64 / 2.0).round() as i32
    };

    let _ = win.set_size(PhysicalSize::new(pw, ph));
    let _ = win.set_position(PhysicalPosition::new(x, y));
    // Moving across displays can rescale the window: re-assert the physical size.
    let _ = win.set_size(PhysicalSize::new(pw, ph));
    let _ = win.set_always_on_top(true);
    *iw.frame.lock().unwrap() = Some(Frame {
        x,
        y,
        w: pw,
        h: ph,
        scale,
        monitor: (mp.x, mp.y, ms.width as i32, ms.height as i32),
    });

    // Windows can clamp or lag a resize (DPI change between displays, the window
    // coming out of the 240 px strip). A panel narrower than the 640 px island
    // gets cut on both sides, so check what we actually got and insist once.
    if let Ok(got) = win.inner_size() {
        if got.width != pw || got.height != ph {
            crate::log::line(format!(
                "{}: window size mismatch: wanted {pw}x{ph}, got {}x{} (scale {scale}, monitor {}x{})",
                iw.label, got.width, got.height, ms.width, ms.height
            ));
            let _ = win.set_size(PhysicalSize::new(pw, ph));
            let _ = win.set_position(PhysicalPosition::new(x, y));
        }
    }
}

fn hwnd_of(win: &WebviewWindow) -> Option<HWND> {
    let raw = win.hwnd().ok()?.0 as isize;
    if raw == 0 {
        return None;
    }
    Some(HWND(raw as *mut _))
}

/// Shows the system "open file" dialog, owned by the island that asked so it
/// comes to the front. Blocks until the user chooses or cancels, so call it off
/// the UI thread.
pub fn pick_file(owner: Option<WebviewWindow>) -> Option<String> {
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, FOS_FORCEFILESYSTEM, SIGDN_FILESYSPATH,
    };

    let owner = owner.as_ref().and_then(hwnd_of);
    unsafe {
        let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let path = (|| -> Option<String> {
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
            let options = dialog.GetOptions().ok()?;
            dialog.SetOptions(options | FOS_FORCEFILESYSTEM).ok()?;
            dialog.Show(owner).ok()?; // Err on cancel
            let item = dialog.GetResult().ok()?;
            let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
            let path = name.to_string().ok();
            CoTaskMemFree(Some(name.0 as *const _));
            path
        })();
        if init.is_ok() {
            CoUninitialize();
        }
        path
    }
}

/// WS_EX_NOACTIVATE keeps clicks from stealing focus; WS_EX_TOOLWINDOW keeps the
/// island out of Alt-Tab.
pub fn make_non_activating(win: &WebviewWindow) {
    let Some(hwnd) = hwnd_of(win) else { return };
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let want = ex | WS_EX_NOACTIVATE.0 as isize | WS_EX_TOOLWINDOW.0 as isize;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, want);
    }
}

/// Temporarily allow activation so a text field inside the island can be typed in.
pub fn set_activating(win: &WebviewWindow, activating: bool) {
    let Some(hwnd) = hwnd_of(win) else { return };
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let want = if activating {
            ex & !(WS_EX_NOACTIVATE.0 as isize)
        } else {
            ex | WS_EX_NOACTIVATE.0 as isize
        };
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, want);
    }
}

/// Position, size and scale of every display. Any change here means the islands
/// have to be handed out and placed again.
type Layout = Vec<(String, i32, i32, u32, u32, u64)>;

fn layout_key(app: &AppHandle) -> Option<Layout> {
    let monitors = app.available_monitors().ok()?;
    let mut layout: Layout = monitors
        .iter()
        .map(|m| {
            let (p, s) = (m.position(), m.size());
            (monitor_key(m), p.x, p.y, s.width, s.height, m.scale_factor().to_bits())
        })
        .collect();
    layout.sort();
    Some(layout)
}

/// Last display layout seen by any island's poll.
static LAYOUT: Mutex<Option<Layout>> = Mutex::new(None);
/// When the layout was last checked — once every LAYOUT_EVERY for all islands
/// together, not twice a second per island.
static LAYOUT_CHECKED: Mutex<Option<std::time::Instant>> = Mutex::new(None);
const LAYOUT_EVERY: Duration = Duration::from_secs(2);

/// Cursor poll period while the cursor is on this island's display.
const POLL_NEAR: Duration = Duration::from_millis(16);
/// …and while it is on another display: nothing to track, just notice it coming back.
const POLL_FAR: Duration = Duration::from_millis(120);

fn check_layout(app: &AppHandle) {
    {
        let mut checked = LAYOUT_CHECKED.lock().unwrap();
        if checked.is_some_and(|t| t.elapsed() < LAYOUT_EVERY) {
            return;
        }
        *checked = Some(std::time::Instant::now());
    }
    let Some(now) = layout_key(app) else { return };
    let changed = {
        let mut seen = LAYOUT.lock().unwrap();
        let changed = seen.as_ref().is_some_and(|s| *s != now);
        *seen = Some(now);
        changed
    };
    if changed {
        crate::log::line("display layout changed — repositioning".to_string());
        assign_monitors(app);
        emit_all(app, "screen-changed", ());
    }
}

/// The window's frame: the cached one, or — while Windows is moving the window
/// under a drag, or before the first placement — read once from the window.
fn frame_of(iw: &IslandWin, win: &WebviewWindow) -> Option<Frame> {
    let cached = *iw.frame.lock().unwrap();
    if let Some(f) = cached {
        if !iw.dragging.load(Ordering::Relaxed) {
            return Some(f);
        }
        let pos = win.outer_position().ok()?;
        return Some(Frame { x: pos.x, y: pos.y, ..f });
    }
    let pos = win.outer_position().ok()?;
    let size = win.inner_size().ok()?;
    let scale = win.scale_factor().unwrap_or(1.0);
    Some(Frame { x: pos.x, y: pos.y, w: size.width, h: size.height, scale, monitor: (i32::MIN, i32::MIN, i32::MAX, i32::MAX) })
}

/// Emits `cursor` (window-logical coordinates) to its own island at ~60 Hz while
/// that island is visible and the cursor is on its display; slower, and silent,
/// while the cursor is on another display. Parked on a condvar the rest of the time.
pub fn spawn_cursor_poll(app: AppHandle, iw: Arc<IslandWin>) {
    let gate = iw.gate.clone();
    std::thread::spawn(move || {
        let mut was_down = false;
        loop {
            gate.wait_until_active();
            let mut last = (f64::MIN, f64::MIN);
            let mut away = false;
            while gate.is_active() {
                std::thread::sleep(if away { POLL_FAR } else { POLL_NEAR });

                // Monitors get plugged in, unplugged, rearranged and rescaled, and
                // an island pinned to coordinates that no longer exist is an island
                // nobody can reach.
                check_layout(&app);

                let Some(win) = window_of(&app, &iw) else { continue };
                let Some(f) = frame_of(&iw, &win) else { continue };
                let Some((cx, cy)) = cursor_physical() else { continue };
                let x = (cx - f.x as f64) / f.scale;
                let y = (cy - f.y as f64) / f.scale;
                if (x - last.0).abs() < 1.0 && (y - last.1).abs() < 1.0 {
                    continue;
                }
                last = (x, y);

                // Cursor on another display: tell the island once (so it sees the
                // mouse leave), make sure it lets clicks through, then go quiet.
                let (mx, my, mw, mh) = f.monitor;
                let on_display = cx >= mx as f64
                    && cy >= my as f64
                    && cx < mx as f64 + mw as f64
                    && cy < my as f64 + mh as f64;
                if !on_display {
                    if !away {
                        away = true;
                        if !gate.ignoring.swap(true, Ordering::Relaxed) {
                            let _ = win.set_ignore_cursor_events(true);
                        }
                        let _ = app.emit_to(iw.label.as_str(), "cursor", CursorPayload { x, y });
                    }
                    continue;
                }
                away = false;

                // Click-through: the window only takes the mouse over the island
                // shape. A small entry margin means the flag is already off by the
                // time a moving cursor reaches a button.
                let r = *gate.rect.lock().unwrap();
                let on_island = r.w > 0.0
                    && x >= r.x - HIT_MARGIN
                    && x <= r.x + r.w + HIT_MARGIN
                    && y >= r.y - HIT_MARGIN
                    && y <= r.y + r.h + HIT_MARGIN;

                // A file being dragged has to be able to find us. WS_EX_TRANSPARENT
                // — what click-through is on Windows — hides the window from
                // WindowFromPoint, so OLE finds no drop target and shows the "no
                // drop" cursor. macOS has no such problem: AppKit delivers drags to
                // registered destinations whatever ignoresMouseEvents says. So while
                // a button is held anywhere over the panel, the whole panel takes
                // the mouse, which also makes the drop zone as forgiving as the Mac's.
                // A press may be the start of a drag: make sure the drop target is
                // ours before the file arrives.
                let down = left_button_down();
                if down && !was_down {
                    let handle = app.clone();
                    let _ = app.run_on_main_thread(move || unblock_webview_drops(&handle));
                }
                was_down = down;

                let size = (f.w as f64 / f.scale, f.h as f64 / f.scale);
                let dragging = down && x >= 0.0 && x <= size.0 && y >= 0.0 && y <= size.1;

                let accept = on_island || dragging;
                if gate.ignoring.load(Ordering::Relaxed) == accept {
                    gate.ignoring.store(!accept, Ordering::Relaxed);
                    let _ = win.set_ignore_cursor_events(!accept);
                }

                let _ = app.emit_to(iw.label.as_str(), "cursor", CursorPayload { x, y });
            }
        }
    });
}

pub fn set_ignore_cursor(win: &WebviewWindow, ignore: bool) {
    let _ = win.set_ignore_cursor_events(ignore);
}

#[cfg(test)]
mod shake_tests {
    use super::*;

    /// Feeds a path of x positions, 40 ms apart, and says whether it shook.
    fn shakes(xs: &[f64]) -> bool {
        let mut d = ShakeDetector::default();
        xs.iter().enumerate().any(|(i, &x)| d.feed(x, 0.0, i as u64 * 40, 24.0))
    }

    #[test]
    fn a_plain_drag_is_not_a_shake() {
        let path: Vec<f64> = (0..60).map(|i| i as f64 * 10.0).collect();
        assert!(!shakes(&path));
    }

    #[test]
    fn a_drag_with_one_turn_is_not_a_shake() {
        let mut path: Vec<f64> = (0..20).map(|i| i as f64 * 10.0).collect();
        path.extend((0..20).map(|i| 190.0 - i as f64 * 10.0));
        assert!(!shakes(&path));
    }

    #[test]
    fn going_back_and_forth_is_a_shake() {
        // 60 px strokes, left and right, one every 5 samples (200 ms).
        let mut path = Vec::new();
        for _ in 0..6 {
            path.extend([0.0, 15.0, 30.0, 45.0, 60.0]);
            path.extend([45.0, 30.0, 15.0, 0.0]);
        }
        assert!(shakes(&path));
    }

    #[test]
    fn small_jitter_is_not_a_shake() {
        let path: Vec<f64> = (0..80).map(|i| if i % 2 == 0 { 100.0 } else { 108.0 }).collect();
        assert!(!shakes(&path));
    }

    #[test]
    fn slow_swaying_is_not_a_shake() {
        // The same big strokes, but 600 ms per sample: the reversals are too far apart.
        let mut d = ShakeDetector::default();
        let path = [0.0, 60.0, 0.0, 60.0, 0.0, 60.0, 0.0, 60.0];
        assert!(!path.iter().enumerate().any(|(i, &x)| d.feed(x, 0.0, i as u64 * 600, 24.0)));
    }
}
