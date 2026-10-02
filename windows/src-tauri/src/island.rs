// Island window: placement on the chosen display, the two window sizes
// (full panel / invisible wake strip), click-through and the cursor poll.
//
// There is no notch on a PC, so the island is a black shape drawn at the top
// centre of the main display inside a borderless, transparent, always-on-top
// window that never takes focus.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Monitor, PhysicalPosition, PhysicalSize, WebviewWindow};

use windows::Win32::Foundation::{HWND, POINT};
use windows::core::BOOL;
use windows::Win32::Foundation::LPARAM;
use windows::Win32::System::Ole::RevokeDragDrop;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{EnumChildWindows, GetClassNameW};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW,
};

/// Logical size of the full window — the largest island view, like the macOS panel.
pub const PANEL_W: f64 = 720.0;
pub const PANEL_H: f64 = 320.0;
/// Logical size of the invisible strip that wakes the island when it is hidden.
pub const STRIP_W: f64 = 240.0;
pub const STRIP_H: f64 = 6.0;

/// Logical width of the expanded island drawn inside the panel window.
const EXPANDED_W: f64 = 640.0;

pub const WINDOW_LABEL: &str = "island";

// ── User placement ───────────────────────────────────────────────────────────

/// Offset (logical px) of the island from its default top-centre spot.
static OFFSET: Mutex<(f64, f64)> = Mutex::new((0.0, 0.0));
static DRAGGING: AtomicBool = AtomicBool::new(false);
/// -1 = docked to the left edge, 1 = right edge, 0 = free.
static DOCK: AtomicI32 = AtomicI32::new(0);

pub fn dock() -> i32 {
    DOCK.load(Ordering::SeqCst)
}

fn position_path() -> std::path::PathBuf {
    crate::settings::config_dir().join("position.json")
}

pub fn load_position() {
    if let Ok(bytes) = std::fs::read(position_path()) {
        if let Ok((x, y)) = serde_json::from_slice::<(f64, f64)>(&bytes) {
            *OFFSET.lock().unwrap() = (x, y);
        }
    }
}

fn save_position() {
    let _ = std::fs::create_dir_all(crate::settings::config_dir());
    if let Ok(json) = serde_json::to_vec(&*OFFSET.lock().unwrap()) {
        let _ = std::fs::write(position_path(), json);
    }
}

/// The island is about to retract: it never floats mid-screen, so it docks to the
/// nearest of the left, right and top edges of its display.
pub fn dock_nearest(app: &AppHandle, pref: &str, collapsed: bool) {
    if DRAGGING.load(Ordering::SeqCst) {
        return;
    }
    let Some(m) = target_monitor(app, pref) else { return };
    let scale = m.scale_factor();
    let ms = m.size();

    let (odx, ody) = *OFFSET.lock().unwrap();
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
    *OFFSET.lock().unwrap() = (dx, dy);
    apply_geometry(app, pref, collapsed);
    save_position();
}

/// Puts the island back at the top centre of the display.
pub fn reset_position(app: &AppHandle, pref: &str, collapsed: bool) {
    *OFFSET.lock().unwrap() = (0.0, 0.0);
    save_position();
    apply_geometry(app, pref, collapsed);
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
/// remembers where the island ended up and snaps it back inside the display.
pub fn begin_drag(app: &AppHandle, pref: String, gate: Arc<PollGate>) {
    let Some(win) = window(app) else { return };
    if DRAGGING.swap(true, Ordering::SeqCst) {
        return;
    }
    if win.start_dragging().is_err() {
        DRAGGING.store(false, Ordering::SeqCst);
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
            if let (Some(win), Some(m)) = (window(&app), target_monitor(&app, &pref)) {
                if let Ok(pos) = win.outer_position() {
                    let swing = SHAKE_SWING * m.scale_factor();
                    let t = started.elapsed().as_millis() as u64;
                    if shake.feed(pos.x as f64, pos.y as f64, t, swing) {
                        shaken = true;
                        let _ = app.emit_to(WINDOW_LABEL, "shaken", ());
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
        if let (Some(win), Some(m)) = (window(&app), target_monitor(&app, &pref)) {
            if let (Ok(pos), Ok(size)) = (win.outer_position(), win.outer_size()) {
                let scale = m.scale_factor();
                let mp = m.position();
                let ms = m.size();
                // Open, the island stays wherever the user drops it. It only docks
                // to an edge when it is about to retract (see `dock_nearest`).
                let cx = pos.x as f64 + size.width as f64 / 2.0;
                let dx = (cx - (mp.x as f64 + ms.width as f64 / 2.0)) / scale;
                let dy = (pos.y - mp.y) as f64 / scale;
                *OFFSET.lock().unwrap() = (dx, dy);
            }
        }
        DRAGGING.store(false, Ordering::SeqCst);
        apply_geometry(&app, &pref, gate.collapsed.load(Ordering::Relaxed));
        save_position();
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

pub fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(WINDOW_LABEL)
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
    for label in [WINDOW_LABEL, "settings"] {
        let Some(win) = app.get_webview_window(label) else { continue };
        let Some(hwnd) = hwnd_of(&win) else { continue };
        unsafe {
            let _ = EnumChildWindows(Some(hwnd), Some(revoke_render_widget), LPARAM(0));
        }
    }
}

unsafe extern "system" fn revoke_render_widget(hwnd: HWND, _: LPARAM) -> BOOL {
    let mut name = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, &mut name) };
    if len > 0 {
        let class = String::from_utf16_lossy(&name[..len as usize]);
        if class == "Chrome_RenderWidgetHostHWND" {
            let _ = unsafe { RevokeDragDrop(hwnd) };
        }
    }
    true.into()
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

/// The display the island lives on: the primary one, or the one under the cursor.
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

pub fn screen_info(app: &AppHandle, pref: &str) -> ScreenInfo {
    match target_monitor(app, pref) {
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

/// Places and sizes the window. `collapsed` picks the wake strip instead of the panel.
pub fn apply_geometry(app: &AppHandle, pref: &str, collapsed: bool) {
    let Some(win) = window(app) else { return };
    let Some(m) = target_monitor(app, pref) else { return };

    let scale = m.scale_factor();
    let mp = *m.position();
    let ms = *m.size();

    // Where the user left the island, as a logical offset from the default spot
    // (top centre of the display). Clamped so the whole 640 px island stays on
    // this display whatever its size or scale.
    let (odx, ody) = *OFFSET.lock().unwrap();
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
    if DOCK.swap(side, Ordering::SeqCst) != side {
        let _ = app.emit_to(WINDOW_LABEL, "dock", side);
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

    // Windows can clamp or lag a resize (DPI change between displays, the window
    // coming out of the 240 px strip). A panel narrower than the 640 px island
    // gets cut on both sides, so check what we actually got and insist once.
    if let Ok(got) = win.inner_size() {
        if got.width != pw || got.height != ph {
            crate::log::line(format!(
                "window size mismatch: wanted {pw}x{ph}, got {}x{} (scale {scale}, monitor {}x{})",
                got.width, got.height, ms.width, ms.height
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

/// Position, size and scale of the monitor the island lives on. Any change here
/// means the island has to be placed again.
fn current_screen_key(app: &AppHandle) -> Option<(i32, i32, u32, u32, u64)> {
    let pref = app
        .try_state::<crate::Shared>()
        .map(|s| s.settings.lock().unwrap().screen.clone())
        .unwrap_or_else(|| "primary".into());
    let m = target_monitor(app, &pref)?;
    let p = m.position();
    let size = m.size();
    Some((p.x, p.y, size.width, size.height, m.scale_factor().to_bits()))
}

/// Emits `cursor` (window-logical coordinates) at ~60 Hz while the island is
/// visible. Parked on a condvar the rest of the time.
pub fn spawn_cursor_poll(app: AppHandle, gate: Arc<PollGate>) {
    std::thread::spawn(move || {
        let mut was_down = false;
        // Remembered across wakes so a display change while hidden is noticed the
        // moment the island comes back.
        let mut last_screen: Option<(i32, i32, u32, u32, u64)> = None;
        loop {
            gate.wait_until_active();
            let mut last = (f64::MIN, f64::MIN);
            let mut ticks: u32 = 0;
            while gate.is_active() {
                std::thread::sleep(Duration::from_millis(16));

                // Monitors get plugged in, unplugged, rearranged and rescaled, and
                // an island pinned to coordinates that no longer exist is an island
                // nobody can reach. Checked about twice a second — the cursor poll
                // is already running, so this costs one monitor query.
                ticks = ticks.wrapping_add(1);
                if ticks % 30 == 0 {
                    let now = current_screen_key(&app);
                    if now.is_some() && now != last_screen {
                        let first = last_screen.is_none();
                        last_screen = now;
                        if !first {
                            crate::log::line("display layout changed — repositioning".to_string());
                            let _ = app.emit_to(WINDOW_LABEL, "screen-changed", ());
                        }
                    }
                }

                let Some(win) = window(&app) else { continue };
                let Ok(origin) = win.outer_position() else { continue };
                let scale = win.scale_factor().unwrap_or(1.0);
                let Some((cx, cy)) = cursor_physical() else { continue };
                let x = (cx - origin.x as f64) / scale;
                let y = (cy - origin.y as f64) / scale;
                let size = match win.inner_size() {
                    Ok(s) => (s.width as f64 / scale, s.height as f64 / scale),
                    Err(_) => (PANEL_W, PANEL_H),
                };
                if (x - last.0).abs() < 1.0 && (y - last.1).abs() < 1.0 {
                    continue;
                }
                last = (x, y);

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

                let dragging = down
                    && x >= 0.0
                    && x <= size.0
                    && y >= 0.0
                    && y <= size.1;

                let accept = on_island || dragging;
                if gate.ignoring.load(Ordering::Relaxed) == accept {
                    gate.ignoring.store(!accept, Ordering::Relaxed);
                    let _ = win.set_ignore_cursor_events(!accept);
                }

                let _ = win.emit("cursor", CursorPayload { x, y });
            }
        }
    });
}

pub fn set_ignore_cursor(app: &AppHandle, ignore: bool) {
    if let Some(win) = window(app) {
        let _ = win.set_ignore_cursor_events(ignore);
    }
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
