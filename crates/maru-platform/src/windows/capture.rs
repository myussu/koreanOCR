//! Screen capture with GDI `BitBlt`: ~1 ms for a selection-sized region and
//! works for every window the compositor shows (not exclusive-fullscreen games).
//! The overlay must opt out of capture (WDA_EXCLUDEFROMCAPTURE) so it never
//! appears in its own screenshots.

use std::ffi::c_void;

use image::RgbImage;
use maru_core::{Point, Rect};
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BitBlt, CAPTUREBLT, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDC, GetDIBits, HBITMAP, HDC, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

use super::fail;
use crate::Result;

pub fn cursor() -> Option<Point> {
    let mut p = POINT::default();
    // SAFETY: `p` is a valid out-pointer.
    unsafe { GetCursorPos(&mut p).ok()? };
    Some(Point { x: p.x as f32, y: p.y as f32 })
}

pub fn virtual_screen() -> Rect {
    // SAFETY: GetSystemMetrics has no preconditions.
    unsafe {
        Rect::new(
            GetSystemMetrics(SM_XVIRTUALSCREEN) as f32,
            GetSystemMetrics(SM_YVIRTUALSCREEN) as f32,
            GetSystemMetrics(SM_CXVIRTUALSCREEN) as f32,
            GetSystemMetrics(SM_CYVIRTUALSCREEN) as f32,
        )
    }
}

/// Releases the GDI objects used for one capture, in the right order, even on early return.
struct Gdi {
    screen: HDC,
    mem: HDC,
    bitmap: HBITMAP,
}

impl Drop for Gdi {
    fn drop(&mut self) {
        // SAFETY: every handle was created by the matching call in `grab` and is released once.
        unsafe {
            if !self.bitmap.is_invalid() {
                let _ = DeleteObject(self.bitmap.into());
            }
            if !self.mem.is_invalid() {
                let _ = DeleteDC(self.mem);
            }
            if !self.screen.is_invalid() {
                ReleaseDC(None, self.screen);
            }
        }
    }
}

pub fn grab(region: Rect) -> Result<RgbImage> {
    let bounds = virtual_screen();
    let clipped = Rect::new(
        region.x.max(bounds.x),
        region.y.max(bounds.y),
        (region.right().min(bounds.right()) - region.x.max(bounds.x)).max(0.0),
        (region.bottom().min(bounds.bottom()) - region.y.max(bounds.y)).max(0.0),
    );
    let (x, y) = (clipped.x.round() as i32, clipped.y.round() as i32);
    let (w, h) = (clipped.w.round() as i32, clipped.h.round() as i32);
    if w <= 0 || h <= 0 {
        return Err(crate::PlatformError::Failed(format!("capture region is empty or off-screen: {region:?}")));
    }

    // SAFETY: standard GDI screen-capture sequence; all handles are owned by `Gdi`.
    unsafe {
        let screen = GetDC(None);
        let mut gdi = Gdi { screen, mem: HDC::default(), bitmap: HBITMAP::default() };
        if screen.is_invalid() {
            return Err(fail("GetDC", "no screen device context"));
        }
        gdi.mem = CreateCompatibleDC(Some(screen));
        gdi.bitmap = CreateCompatibleBitmap(screen, w, h);
        if gdi.mem.is_invalid() || gdi.bitmap.is_invalid() {
            return Err(fail("GDI", "could not create a compatible bitmap"));
        }
        let previous = SelectObject(gdi.mem, gdi.bitmap.into());
        let blit = BitBlt(gdi.mem, 0, 0, w, h, Some(screen), x, y, SRCCOPY | CAPTUREBLT);

        let mut info = BITMAPINFO::default();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // negative = top-down rows
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut bgra = vec![0u8; (w as usize) * (h as usize) * 4];
        let rows = GetDIBits(gdi.mem, gdi.bitmap, 0, h as u32, Some(bgra.as_mut_ptr() as *mut c_void), &mut info, DIB_RGB_COLORS);
        SelectObject(gdi.mem, previous);
        blit.map_err(|e| fail("BitBlt", e))?;
        if rows == 0 {
            return Err(fail("GetDIBits", "no scan lines copied"));
        }

        let mut rgb = Vec::with_capacity((w as usize) * (h as usize) * 3);
        for px in bgra.chunks_exact(4) {
            rgb.extend_from_slice(&[px[2], px[1], px[0]]);
        }
        RgbImage::from_raw(w as u32, h as u32, rgb).ok_or_else(|| fail("capture", "pixel buffer size mismatch"))
    }
}
