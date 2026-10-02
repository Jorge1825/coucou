// Coucou for Windows — app wiring and the commands the island calls.

mod claude;
mod context;
mod files;
mod hooks;
mod hotkey;
mod integrations;
mod island;
mod log;
mod memory;
mod pipe;
mod proactive;
mod providers;
mod reminders;
mod screen;
mod secrets;
mod settings;
mod spotify;
mod tray;
mod win_user;

use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_autostart::{ManagerExt, MacosLauncher};

use claude::{Chat, ChatContext, ChatReply};
use files::DroppedFile;
use hooks::{HookPreview, HookStatus};
use island::{IslandWin, ScreenInfo};
use pipe::Pending;
use settings::Settings;

/// Keeps spawned helpers from flashing a console window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct Shared {
    pub settings: Mutex<Settings>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootInfo {
    settings: Settings,
    screen: ScreenInfo,
    version: String,
    hook_path: String,
    /// -1 / 1 when the island is docked to the left / right screen edge.
    dock: i32,
    /// The main display's island. Event sounds play only there, so four
    /// displays don't chime four times.
    lead: bool,
}

/// The island a command came from. Every island window calls the same commands;
/// this is how each one only ever moves itself.
fn caller(window: &WebviewWindow) -> Option<Arc<IslandWin>> {
    island::get(window.label())
}

#[tauri::command]
fn boot(app: AppHandle, window: WebviewWindow, shared: State<Shared>) -> BootInfo {
    let mut settings = shared.settings.lock().unwrap().clone();
    // The real state of ~/.claude/settings.json wins over whatever we stored.
    settings.hooks_installed = hooks::status().installed;
    let me = caller(&window);
    let screen = match &me {
        Some(iw) => island::screen_info(&app, iw),
        None => ScreenInfo { x: 0.0, y: 0.0, width: 1920.0, height: 1080.0, scale: 1.0 },
    };
    BootInfo {
        settings,
        screen,
        version: env!("CARGO_PKG_VERSION").to_string(),
        hook_path: settings::hook_exe_path().to_string_lossy().to_string(),
        dock: me.as_deref().map(island::dock).unwrap_or(0),
        lead: window.label() == island::WINDOW_LABEL,
    }
}

#[tauri::command]
fn save_settings(app: AppHandle, shared: State<Shared>, settings: Settings) {
    let (screen_changed, autostart_changed) = {
        let mut current = shared.settings.lock().unwrap();
        let screen_changed =
            current.screen != settings.screen || current.all_screens != settings.all_screens;
        let autostart_changed = current.autostart != settings.autostart;
        *current = settings.clone();
        (screen_changed, autostart_changed)
    };
    if let Err(err) = settings::save(&settings) {
        eprintln!("[coucou] could not save settings: {err}");
    }
    if autostart_changed {
        let manager = app.autolaunch();
        let result = if settings.autostart { manager.enable() } else { manager.disable() };
        if let Err(err) = result {
            eprintln!("[coucou] autostart: {err}");
        }
    }
    if screen_changed {
        island::assign_monitors(&app);
    }
    // Keep every window in step (islands ⇄ settings window).
    let _ = app.emit("settings-changed", settings);
}

/// Hidden island → shrink the window to the invisible wake strip and park the
/// cursor poll; anything else → full panel and 60 Hz polling.
#[tauri::command]
fn set_collapsed(app: AppHandle, window: WebviewWindow, collapsed: bool) {
    let Some(iw) = caller(&window) else { return };
    iw.gate.collapsed.store(collapsed, Ordering::Relaxed);
    island::apply_geometry(&app, &iw, collapsed);
    // The wake strip must always take the mouse, and a resize invalidates the flag.
    island::set_ignore_cursor(&window, false);
    iw.gate.forget_ignore_state();
    // An island without a display has nothing to poll for.
    iw.gate.set_active(!collapsed && island::monitor_of(&app, &iw).is_some());
    // The poll is parked while collapsed, so it can't prepare the drop target.
    island::schedule_unblock_drops(&app);
}

/// The front end pushes the island shape; Rust decides click-through from it.
#[tauri::command]
fn set_island_rect(window: WebviewWindow, x: f64, y: f64, width: f64, height: f64) {
    if let Some(iw) = caller(&window) {
        iw.gate.set_rect(island::IslandRect { x, y, w: width, h: height });
    }
}

#[tauri::command]
fn focus_window(window: WebviewWindow, focused: bool) {
    island::set_activating(&window, focused);
    if focused {
        let _ = window.set_focus();
    }
}

#[tauri::command]
fn reposition(app: AppHandle, window: WebviewWindow) {
    let Some(iw) = caller(&window) else { return };
    let collapsed = iw.gate.collapsed.load(Ordering::Relaxed);
    island::apply_geometry(&app, &iw, collapsed);
}

#[tauri::command]
fn reset_position(app: AppHandle, window: WebviewWindow) {
    let Some(iw) = caller(&window) else { return };
    let collapsed = iw.gate.collapsed.load(Ordering::Relaxed);
    island::reset_position(&app, &iw, collapsed);
}

/// The island is retracting: dock it to the nearest edge of its display.
#[tauri::command]
fn dock_nearest(app: AppHandle, window: WebviewWindow) {
    let Some(iw) = caller(&window) else { return };
    let collapsed = iw.gate.collapsed.load(Ordering::Relaxed);
    island::dock_nearest(&app, &iw, collapsed);
}

/// The island's background was dragged: let Windows move the window.
#[tauri::command]
fn begin_drag(app: AppHandle, window: WebviewWindow) {
    if let Some(iw) = caller(&window) {
        island::begin_drag(&app, iw);
    }
}

/// The panel is laid out in CSS px against the window's own scale. If the webview
/// reports a different devicePixelRatio (it ends up bigger than the window and gets
/// cut), correct its zoom so one CSS px is one logical px again.
#[tauri::command]
fn fit_zoom(window: WebviewWindow, dpr: f64) {
    let Some(iw) = caller(&window) else { return };
    let scale = window.scale_factor().unwrap_or(1.0);
    if dpr <= 0.0 || (dpr - scale).abs() / scale < 0.02 {
        return;
    }
    let mut zoom = iw.zoom.lock().unwrap();
    *zoom *= scale / dpr;
    log::line(format!("{}: webview dpr {dpr} != window scale {scale} — zoom {}", iw.label, *zoom));
    let _ = window.set_zoom(*zoom);
}

#[tauri::command]
fn open_url(url: String) {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return;
    }
    let _ = Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", &url])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

/// "Open terminal" opens the working folder in VS Code when `code` is on PATH,
/// and falls back to Explorer otherwise.
#[tauri::command]
fn open_in_vscode(path: Option<String>) -> bool {
    // No `cmd /C` anywhere near this. The path is a project folder chosen by
    // whoever is using Claude Code, and cmd would happily read `&`, `^` and `%`
    // in a folder name as syntax. Finding the launcher ourselves and handing the
    // path over as a separate argument keeps it a path.
    if let Some(code) = find_on_path("code") {
        let mut cmd = Command::new(code);
        if let Some(p) = path.as_deref().filter(|p| !p.is_empty()) {
            cmd.arg(p);
        }
        if cmd.creation_flags(CREATE_NO_WINDOW).spawn().is_ok() {
            return true;
        }
    }
    if let Some(p) = path.as_deref().filter(|p| !p.is_empty()) {
        let _ = Command::new("explorer").arg(p).spawn();
    }
    false
}

/// Our own `where`: walks %PATH% against %PATHEXT%, no shell involved.
/// Rust quotes arguments correctly for `.cmd`/`.bat` targets since 1.77, so
/// spawning `code.cmd` directly is safe.
fn find_on_path(stem: &str) -> Option<std::path::PathBuf> {
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let dirs = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&dirs) {
        for ext in exts.split(';').filter(|e| !e.is_empty()) {
            let candidate = dir.join(format!("{stem}{}", ext.to_lowercase()));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// Tray → Pause. Paused means paused: the pollers stop talking to the network,
/// not just the island stopping showing things.
#[tauri::command]
fn set_paused(paused: bool) {
    integrations::set_paused(paused);
}

// ── Claude Code hooks ─────────────────────────────────────────────────────────

#[tauri::command]
fn hooks_status() -> HookStatus {
    hooks::status()
}

/// Returns the diff the user has to look at before anything is written.
#[tauri::command]
fn hooks_preview(install: bool) -> Result<HookPreview, String> {
    hooks::preview(install)
}

/// Only ever called from an explicit click in the settings window.
#[tauri::command]
fn hooks_apply(
    app: AppHandle,
    shared: State<Shared>,
    install: bool,
    fingerprint: String,
) -> Result<String, String> {
    // The fingerprint comes from the preview the user actually looked at, so a
    // settings.json that changed in between is refused rather than overwritten.
    let backup = hooks::write(install, &fingerprint)?;
    let updated = {
        let mut current = shared.settings.lock().unwrap();
        current.hooks_installed = install;
        let _ = settings::save(&current);
        current.clone()
    };
    let _ = app.emit("settings-changed", updated);
    Ok(backup)
}

/// One screenshot of the calling island's display, only when the user asks for it.
/// Changes the shortcut that opens the chat. Empty removes it. The new one is
/// only saved if Windows accepted it, so a refused combination never sticks.
#[tauri::command]
fn set_chat_hotkey(app: AppHandle, shared: State<Shared>, combo: String) -> Result<String, String> {
    let canonical = hotkey::set(&combo)?;
    let updated = {
        let mut current = shared.settings.lock().unwrap();
        current.chat_hotkey = canonical.clone();
        let _ = settings::save(&current);
        current.clone()
    };
    let _ = app.emit("settings-changed", updated);
    Ok(canonical)
}

/// One screenshot of the island's display, only when the user asks for it.
#[tauri::command]
async fn capture_screen(app: AppHandle, window: WebviewWindow) -> Result<DroppedFile, String> {
    let iw = caller(&window).ok_or("No screen to capture.")?;
    let rect = island::target_rect(&app, &iw).ok_or("No screen to capture.")?;
    tauri::async_runtime::spawn_blocking(move || screen::capture(rect))
        .await
        .map_err(|e| e.to_string())?
}

/// Native file chooser for the drop zone; `None` when the user cancels.
#[tauri::command]
async fn pick_file(window: WebviewWindow) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || island::pick_file(Some(window)))
        .await
        .ok()
        .flatten()
}

#[tauri::command]
fn approval_decision(app: AppHandle, request_id: String, decision: String) {
    pipe::answer(&app, &request_id, &decision);
}

/// The island has the card on screen, so the long wait for a human may begin.
/// Until this arrives the relay only waits a few hundred milliseconds, which is
/// what stops a paused or unresponsive island from freezing Claude Code.
#[tauri::command]
fn approval_ack(app: AppHandle, request_id: String) {
    pipe::acknowledge(&app, &request_id);
}

/// Nobody can act on this request — the island is paused, or another card is
/// already up. Claude Code falls back to asking in the terminal immediately.
#[tauri::command]
fn approval_decline(app: AppHandle, request_id: String) {
    pipe::decline(&app, &request_id);
}

// ── Chat, files and secrets ───────────────────────────────────────────────────

/// One chat turn. The API key and any file bytes stay on the Rust side.
#[tauri::command]
async fn chat_send(
    shared: State<'_, Shared>,
    chat: State<'_, Chat>,
    query: String,
    context: Option<ChatContext>,
) -> Result<ChatReply, String> {
    let settings = shared.settings.lock().unwrap().clone();
    let target = providers::target(&settings)?;
    claude::send(&chat, &target, &settings.model, query, context).await
}

/// Pending reminders, soonest first, for the settings window.
#[tauri::command]
fn reminders_list() -> Vec<reminders::Reminder> {
    reminders::load()
}

#[tauri::command]
fn reminders_delete(id: u64) -> Result<(), String> {
    reminders::delete(id)
}

/// Everything Mochi remembers, oldest first, for the settings window.
#[tauri::command]
fn memory_list() -> Vec<memory::Note> {
    memory::list()
}

/// A note typed in the settings window. Refused if it looks like a secret.
#[tauri::command]
fn memory_add(text: String) -> Result<(), String> {
    memory::add(&text, "user").map(|_| ())
}

#[tauri::command]
fn memory_delete(id: u64) -> Result<(), String> {
    memory::delete(id)
}

#[tauri::command]
fn memory_clear() -> Result<(), String> {
    memory::clear()
}

/// The user pressed stop: abandon the request in flight.
#[tauri::command]
fn chat_cancel(chat: State<Chat>) {
    chat.cancel();
}

#[tauri::command]
fn chat_reset(chat: State<Chat>) {
    chat.reset();
}

/// Copies a dropped file into the inbox and reports its name back.
#[tauri::command]
fn ingest_file(path: String) -> Result<DroppedFile, String> {
    files::ingest(&path)
}

/// The island may only ask whether a key exists — never read it.
#[tauri::command]
fn secret_present(key: String) -> bool {
    secrets::present(&key)
}

#[tauri::command]
fn secret_set(key: String, value: String) -> Result<(), String> {
    secrets::set(&key, &value)
}

#[tauri::command]
fn secret_clear(key: String) -> Result<(), String> {
    secrets::clear(&key)
}

/// Opens the configured n8n instance — the URL lives in the Credential Manager.
#[tauri::command]
fn open_n8n() {
    if let Some(url) = secrets::get("n8n-url") {
        open_url(url);
    }
}

/// Refresh buttons in the integration cards.
#[tauri::command]
async fn refresh_integration(app: AppHandle, id: String) {
    integrations::poll_once(app, &id).await;
}

/// Play/pause, next, previous — only ever from a click in the island.
#[tauri::command]
async fn spotify_control(action: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || spotify::control(&action))
        .await
        .map_err(|e| e.to_string())?
}

/// Lets the island write to the same log as the Rust side.
#[tauri::command]
fn log_line(message: String) {
    log::line(format!("ui  {message}"));
}

// ── Settings window ───────────────────────────────────────────────────────────

/// WebView2 allows exactly one browser environment per app, and its options are
/// fixed by whichever webview is created first. Every window must therefore ask
/// for the *same* arguments as the island (see `additionalBrowserArgs` in
/// tauri.conf.json) — a mismatch makes the second window come up blank, with no
/// error anywhere.
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required";

/// In a dev build the pages are served by Vite, so the second window needs the
/// absolute dev URL; a bundled build resolves it inside the app bundle.
fn settings_page_url(app: &AppHandle) -> WebviewUrl {
    #[cfg(dev)]
    if let Some(mut base) = app.config().build.dev_url.clone() {
        base.set_path("/settings.html");
        return WebviewUrl::External(base);
    }
    let _ = app;
    WebviewUrl::App("settings.html".into())
}

/// The settings window is created hidden at launch and only ever shown and
/// hidden afterwards. A WebView2 window created later — on the main thread or
/// not — silently comes up blank in this app, so the window that works is the
/// one that exists before the island's webview does.
fn create_settings_window(app: &AppHandle) {
    let url = settings_page_url(app);
    match WebviewWindowBuilder::new(app, "settings", url)
        .additional_browser_args(BROWSER_ARGS)
        .title("Settings — Coucou")
        .inner_size(560.0, 680.0)
        .min_inner_size(460.0, 480.0)
        .resizable(true)
        .visible(false)
        .center()
        .build()
    {
        Ok(win) => {
            // Closing it must only hide it, or it could never be reopened.
            let hidden = win.clone();
            win.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = hidden.hide();
                    // Lets the page stop its animation loop while nobody can see it.
                    let _ = hidden.emit("settings-visible", false);
                }
            });
        }
        Err(err) => log::line(format!("settings window failed: {err}")),
    }
}

pub fn show_settings_window(app: &AppHandle) {
    let Some(win) = app.get_webview_window("settings") else {
        log::line("settings window missing");
        return;
    };
    let _ = win.unminimize();
    let _ = win.show();
    let _ = win.set_focus();
    let _ = win.emit("settings-visible", true);
}

#[tauri::command]
fn open_settings_window(app: AppHandle) {
    show_settings_window(&app);
}

/// Same page and options as the `island` window in tauri.conf.json.
fn island_page_url(app: &AppHandle) -> WebviewUrl {
    #[cfg(dev)]
    if let Some(base) = app.config().build.dev_url.clone() {
        return WebviewUrl::External(base);
    }
    let _ = app;
    WebviewUrl::App("index.html".into())
}

/// One more island, for one more display. Like the settings window it has to
/// exist from launch (see create_settings_window): a webview created later comes
/// up blank, so a display plugged in after launch needs a restart.
fn create_island_window(app: &AppHandle, label: &str) {
    let result = WebviewWindowBuilder::new(app, label, island_page_url(app))
        .additional_browser_args(BROWSER_ARGS)
        .title("Coucou")
        .inner_size(island::STRIP_W, island::STRIP_H)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .maximizable(false)
        .minimizable(false)
        .closable(false)
        .build();
    if let Err(err) = result {
        log::line(format!("{label} window failed: {err}"));
    }
}

pub fn run() {
    let loaded = settings::load();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            island::emit_focused(app, "tray", "open".to_string());
        }))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .manage(Shared {
            settings: Mutex::new(loaded.clone()),
        })
        .manage(Pending::default())
        .manage(Chat::default())
        .invoke_handler(tauri::generate_handler![
            boot,
            save_settings,
            set_collapsed,
            set_island_rect,
            focus_window,
            reposition,
            fit_zoom,
            begin_drag,
            dock_nearest,
            reset_position,
            open_url,
            open_in_vscode,
            quit_app,
            hooks_status,
            hooks_preview,
            hooks_apply,
            approval_decision,
            approval_ack,
            approval_decline,
            log_line,
            chat_send,
            chat_reset,
            chat_cancel,
            memory_list,
            memory_add,
            memory_delete,
            memory_clear,
            reminders_list,
            reminders_delete,
            ingest_file,
            pick_file,
            capture_screen,
            set_chat_hotkey,
            secret_present,
            secret_set,
            secret_clear,
            refresh_integration,
            open_n8n,
            spotify_control,
            open_settings_window,
            set_paused,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            tray::build(&handle)?;
            // Before the island: see create_settings_window.
            create_settings_window(&handle);

            // One island per display, all created now (see create_island_window).
            let displays = island::ordered_monitors(&handle).len().clamp(1, island::MAX_ISLANDS);
            for n in 1..displays {
                create_island_window(&handle, &island::label_for(n));
            }
            for n in 0..displays {
                let label = island::label_for(n);
                let Some(win) = handle.get_webview_window(&label) else { continue };
                island::make_non_activating(&win);
                let iw = island::register(&label);
                // Rust starts every window at full size so the greeting has room.
                iw.gate.collapsed.store(false, Ordering::Relaxed);
                island::spawn_cursor_poll(handle.clone(), iw);
            }
            island::load_positions(&handle);
            island::assign_monitors(&handle);
            island::schedule_unblock_drops(&handle);
            hotkey::start(handle.clone(), &loaded.chat_hotkey);

            log::line(format!("--- Coucou {} started ---", env!("CARGO_PKG_VERSION")));
            hooks::ensure_hook_exe(&handle);
            pipe::start(handle.clone());
            proactive::start(handle.clone());
            integrations::start(handle.clone());
            spotify::start(handle.clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Coucou");
}
