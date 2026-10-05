// Files dragged onto the island, taken over from wry.
//
// Tauri/wry registers its own OLE drop target when the webview is created, but
// on this window — transparent, non-activating, click-through while idle — the
// drag came back with the "not allowed" cursor and nothing ever reached the
// app. So the island registers its own target on every window of the webview
// instead, and says what it sees through an `island-drag` event. Same payload
// shape as Tauri's own drag events.
//
// It only answers for files (CF_HDROP): anything else is refused as before.
// Nothing is read until the user actually drops, apart from the file names that
// OLE hands over on entry.

use serde_json::json;
use tauri::{AppHandle, Emitter};
use windows::core::{implement, BOOL};
use windows::Win32::Foundation::{HWND, LPARAM, POINTL};
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::Ole::{
    IDropTarget, IDropTarget_Impl, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop, CF_HDROP, DROPEFFECT,
    DROPEFFECT_COPY, DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::EnumChildWindows;

pub const EVENT: &str = "island-drag";

/// File paths carried by the drag, or none when it is anything else.
unsafe fn paths_of(data: Option<&IDataObject>) -> Vec<String> {
    let Some(data) = data else { return Vec::new() };
    let format = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let Ok(mut medium) = data.GetData(&format) else { return Vec::new() };
    let hdrop = HDROP(medium.u.hGlobal.0 as _);
    let count = DragQueryFileW(hdrop, 0xFFFF_FFFF, None);
    let mut out = Vec::new();
    for i in 0..count {
        let len = DragQueryFileW(hdrop, i, None) as usize;
        let mut buf = vec![0u16; len + 1];
        DragQueryFileW(hdrop, i, Some(&mut buf));
        out.push(String::from_utf16_lossy(&buf[..len]));
    }
    ReleaseStgMedium(&mut medium);
    out
}

#[implement(IDropTarget)]
struct IslandDropTarget {
    app: AppHandle,
    label: String,
}

impl IslandDropTarget {
    fn tell(&self, kind: &str, paths: &[String]) {
        let _ = self.app.emit_to(self.label.as_str(), EVENT, json!({ "type": kind, "paths": paths }));
    }
}

#[allow(non_snake_case)]
impl IDropTarget_Impl for IslandDropTarget_Impl {
    fn DragEnter(
        &self,
        pdataobj: windows::core::Ref<'_, IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let paths = unsafe { paths_of(pdataobj.as_ref()) };
        unsafe {
            *effect = if paths.is_empty() { DROPEFFECT_NONE } else { DROPEFFECT_COPY };
        }
        if !paths.is_empty() {
            self.tell("enter", &paths);
        }
        Ok(())
    }

    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, _pt: &POINTL, _effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        // The effect chosen on entry stands; nothing needs saying on every pixel.
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        self.tell("leave", &[]);
        Ok(())
    }

    fn Drop(
        &self,
        pdataobj: windows::core::Ref<'_, IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let paths = unsafe { paths_of(pdataobj.as_ref()) };
        unsafe {
            *effect = if paths.is_empty() { DROPEFFECT_NONE } else { DROPEFFECT_COPY };
        }
        if !paths.is_empty() {
            self.tell("drop", &paths);
        }
        Ok(())
    }
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let list = &mut *(lparam.0 as *mut Vec<HWND>);
    list.push(hwnd);
    true.into()
}

/// Makes `top` and everything inside it answer drags with the island's target.
/// WebView2 can re-register its own on a child when it resizes or finishes
/// starting, so this is simply run again whenever a drag might be about to begin.
/// Must run on the thread that owns OLE for the window (the main thread).
pub fn install(app: &AppHandle, label: &str, top: HWND) {
    let mut windows: Vec<HWND> = vec![top];
    unsafe {
        let _ = EnumChildWindows(Some(top), Some(collect), LPARAM(&mut windows as *mut Vec<HWND> as isize));
    }
    let target: IDropTarget = IslandDropTarget { app: app.clone(), label: label.to_string() }.into();
    for hwnd in windows {
        unsafe {
            let _ = RevokeDragDrop(hwnd);
            let _ = RegisterDragDrop(hwnd, &target);
        }
    }
}
