use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::{
    ClipCursor, GetClientRect, GetClipCursor, GetForegroundWindow, GetSystemMetrics,
    GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SMTO_ABORTIFHUNG, SMTO_BLOCK,
    SMTO_ERRORONEXIT, SendMessageTimeoutW, SetForegroundWindow, WM_NULL,
};

use crate::input::{Backend, Bounds, Window};
use crate::{PlatformError, Result};

pub(crate) struct WindowsInput;

fn hwnd(handle: u64) -> HWND {
    HWND(handle as usize as *mut std::ffi::c_void)
}

fn bounds(rect: RECT) -> Bounds {
    Bounds {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

impl Backend for WindowsInput {
    fn thread(&self) -> u32 {
        // SAFETY: these identity queries take no pointers or owned resources.
        unsafe { GetCurrentThreadId() }
    }

    fn process(&self) -> u32 {
        // SAFETY: GetCurrentProcessId has no preconditions.
        unsafe { GetCurrentProcessId() }
    }

    fn foreground(&self) -> u64 {
        // SAFETY: querying the foreground window takes no borrowed resources.
        unsafe { GetForegroundWindow().0 as usize as u64 }
    }

    fn window(&self, handle: u64) -> Option<Window> {
        let handle = hwnd(handle);
        // SAFETY: Win32 validates the handle; all output buffers are owned stack values.
        unsafe {
            if !IsWindow(Some(handle)).as_bool()
                || !IsWindowVisible(handle).as_bool()
                || IsIconic(handle).as_bool()
            {
                return None;
            }
            let mut process = 0;
            let thread = GetWindowThreadProcessId(handle, Some(&mut process));
            if thread == 0 || process == 0 {
                return None;
            }
            let mut client = RECT::default();
            GetClientRect(handle, &mut client).ok()?;
            let mut origin = POINT::default();
            if !ClientToScreen(handle, &mut origin).as_bool() {
                return None;
            }
            let width = client.right - client.left;
            let height = client.bottom - client.top;
            if width <= 0 || height <= 0 {
                return None;
            }
            Some(Window {
                handle: handle.0 as usize as u64,
                process,
                thread,
                client: Bounds {
                    left: origin.x,
                    top: origin.y,
                    right: origin.x.saturating_add(width),
                    bottom: origin.y.saturating_add(height),
                },
            })
        }
    }

    fn desktop(&self) -> Result<Bounds> {
        // SAFETY: these system metrics require no pointers or borrowed resources.
        let (left, top, width, height) = unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
            )
        };
        if width <= 0 || height <= 0 {
            return Err(PlatformError::Failed(
                "Windows did not report an available desktop for Maru's controls.".into(),
            ));
        }
        Ok(Bounds {
            left,
            top,
            right: left.saturating_add(width),
            bottom: top.saturating_add(height),
        })
    }

    fn clip(&self) -> Result<Bounds> {
        let mut clip = RECT::default();
        // SAFETY: clip is a correctly sized, writable stack buffer.
        unsafe { GetClipCursor(&mut clip) }
            .map_err(|e| super::fail("Read pointer confinement", e))?;
        Ok(bounds(clip))
    }

    fn activate(&self, handle: u64) -> Result<()> {
        if self.foreground() == handle {
            return Ok(());
        }
        // SAFETY: the lease checks a live visible HWND before activation. This deliberately
        // uses no synthetic keypress or input-queue attachment to bypass Windows restrictions.
        let activated = unsafe { SetForegroundWindow(hwnd(handle)).as_bool() };
        if self.foreground() == handle {
            return Ok(());
        }
        if activated {
            // A different input queue activates asynchronously. Give it one bounded chance
            // to process that notification before treating an accepted return as a failure.
            // SAFETY: WM_NULL carries no data pointers; the wait never attaches input queues.
            unsafe {
                if GetWindowThreadProcessId(hwnd(handle), None) != GetCurrentThreadId() {
                    let _ = SendMessageTimeoutW(
                        hwnd(handle),
                        WM_NULL,
                        WPARAM(0),
                        LPARAM(0),
                        SMTO_ABORTIFHUNG | SMTO_ERRORONEXIT | SMTO_BLOCK,
                        200,
                        None,
                    );
                }
            }
        }
        if self.foreground() == handle {
            Ok(())
        } else {
            Err(PlatformError::Failed(
                "Windows could not focus Maru's controls or return focus to the previous window."
                    .into(),
            ))
        }
    }

    fn release_clip(&self) -> Result<()> {
        // SAFETY: None releases confinement and contains no borrowed pointer.
        unsafe { ClipCursor(None) }.map_err(|e| super::fail("Release pointer confinement", e))
    }

    fn confine(&self, clip: Bounds) -> Result<()> {
        let clip = RECT {
            left: clip.left,
            top: clip.top,
            right: clip.right,
            bottom: clip.bottom,
        };
        // SAFETY: ClipCursor reads the stack rectangle only during this call.
        unsafe { ClipCursor(Some(&clip)) }.map_err(|e| super::fail("Return pointer confinement", e))
    }
}
