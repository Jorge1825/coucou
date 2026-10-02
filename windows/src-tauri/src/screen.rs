// "Look at my screen": a single screenshot, taken only when the user asks for it
// (the eye button in the chat). Nothing here runs on a timer or in the
// background, and the picture goes nowhere except into the next chat message to
// the provider the user configured. It is saved in the local inbox so it follows
// the same one-week sweep as dropped files.

use std::time::{SystemTime, UNIX_EPOCH};

use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::GENERIC_WRITE;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
    ReleaseDC, SelectObject, SetStretchBltMode, StretchBlt, CAPTUREBLT, HALFTONE, HPALETTE, SRCCOPY,
};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatJpeg, IWICBitmapFrameEncode, IWICImagingFactory, WICBitmapEncoderNoCache,
    WICBitmapIgnoreAlpha,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};

use crate::files::{self, DroppedFile};

/// Longest side of the picture sent to the model. Plenty to read text on screen,
/// and keeps the upload to a few hundred KB.
const MAX_SIDE: i32 = 1920;

/// Captures the screen rectangle `(x, y, w, h)` (physical pixels) to a JPEG in the inbox.
pub fn capture(rect: (i32, i32, i32, i32)) -> Result<DroppedFile, String> {
    let (x, y, w, h) = rect;
    if w <= 0 || h <= 0 {
        return Err("No screen to capture.".into());
    }
    let scale = (MAX_SIDE as f64 / w.max(h) as f64).min(1.0);
    let dw = ((w as f64 * scale).round() as i32).max(1);
    let dh = ((h as f64 * scale).round() as i32).max(1);

    let dir = files::inbox_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let path = dir.join(format!("screen-{stamp}.jpg"));

    unsafe {
        let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let result = encode(x, y, w, h, dw, dh, &path);
        if init.is_ok() {
            CoUninitialize();
        }
        result?;
    }

    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    Ok(DroppedFile {
        name: "Screen".into(),
        path: path.to_string_lossy().to_string(),
        size,
    })
}

unsafe fn encode(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    dw: i32,
    dh: i32,
    path: &std::path::Path,
) -> Result<(), String> {
    let err = |what: &str, e: windows::core::Error| format!("screen capture ({what}): {e}");

    let screen = GetDC(None);
    if screen.is_invalid() {
        return Err("screen capture: no display context".into());
    }
    let mem = CreateCompatibleDC(Some(screen));
    let bitmap = CreateCompatibleBitmap(screen, dw, dh);
    let old = SelectObject(mem, bitmap.into());

    let copied = if dw == w && dh == h {
        BitBlt(mem, 0, 0, dw, dh, Some(screen), x, y, SRCCOPY | CAPTUREBLT).is_ok()
    } else {
        SetStretchBltMode(mem, HALFTONE);
        StretchBlt(mem, 0, 0, dw, dh, Some(screen), x, y, w, h, SRCCOPY | CAPTUREBLT).as_bool()
    };

    let result = (|| -> Result<(), String> {
        if !copied {
            return Err("screen capture: the copy failed".into());
        }
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| err("factory", e))?;
        let source = factory
            .CreateBitmapFromHBITMAP(bitmap, HPALETTE::default(), WICBitmapIgnoreAlpha)
            .map_err(|e| err("bitmap", e))?;
        let stream = factory.CreateStream().map_err(|e| err("stream", e))?;
        let wide: Vec<u16> = path.to_string_lossy().encode_utf16().chain(Some(0)).collect();
        stream
            .InitializeFromFilename(PCWSTR(wide.as_ptr()), GENERIC_WRITE.0)
            .map_err(|e| err("file", e))?;
        let encoder = factory
            .CreateEncoder(&GUID_ContainerFormatJpeg, std::ptr::null())
            .map_err(|e| err("encoder", e))?;
        encoder
            .Initialize(&stream.cast::<windows::Win32::System::Com::IStream>().map_err(|e| err("cast", e))?, WICBitmapEncoderNoCache)
            .map_err(|e| err("init", e))?;
        let mut frame: Option<IWICBitmapFrameEncode> = None;
        encoder.CreateNewFrame(&mut frame, std::ptr::null_mut()).map_err(|e| err("frame", e))?;
        let frame = frame.ok_or("screen capture: no frame")?;
        frame.Initialize(None).map_err(|e| err("frame init", e))?;
        frame.WriteSource(&source, std::ptr::null()).map_err(|e| err("write", e))?;
        frame.Commit().map_err(|e| err("commit frame", e))?;
        encoder.Commit().map_err(|e| err("commit", e))?;
        Ok(())
    })();

    SelectObject(mem, old);
    let _ = DeleteObject(bitmap.into());
    let _ = DeleteDC(mem);
    ReleaseDC(None, screen);
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn captures_a_valid_jpeg() {
        let shot = super::capture((0, 0, 1280, 720)).expect("capture");
        let bytes = std::fs::read(&shot.path).unwrap();
        let _ = std::fs::remove_file(&shot.path);
        assert_eq!(&bytes[..3], &[0xFF, 0xD8, 0xFF], "JPEG signature");
        assert!(bytes.len() > 5_000, "a real screen is more than {} bytes", bytes.len());
        assert_eq!(shot.size as usize, bytes.len());
    }
}
