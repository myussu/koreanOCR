use crate::app_window::AppWindow;
use maru_core::{AppInfo, Rect};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClientRect, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
    IsIconic, IsWindowVisible,
};
use windows::core::BOOL;
use windows::core::PWSTR;

fn inspect(hwnd: HWND, exe: &str) -> Option<AppWindow> {
    // SAFETY: Win32 queries use owned, correctly sized buffers; the process handle is closed.
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return None;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut path = [0u16; 1024];
        let mut len = path.len() as u32;
        let result = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(path.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(process);
        result.ok()?;
        let path = String::from_utf16_lossy(&path[..len as usize]);
        let name = path.rsplit(['\\', '/']).next()?;
        if !name.eq_ignore_ascii_case(exe) {
            return None;
        }
        let mut client = RECT::default();
        GetClientRect(hwnd, &mut client).ok()?;
        let mut origin = POINT::default();
        if !ClientToScreen(hwnd, &mut origin).as_bool() {
            return None;
        }
        let rect = Rect::new(
            origin.x as f32,
            origin.y as f32,
            (client.right - client.left) as f32,
            (client.bottom - client.top) as f32,
        );
        if rect.is_empty() {
            return None;
        }
        let mut title = [0u16; 512];
        let n = GetWindowTextW(hwnd, &mut title);
        Some(AppWindow {
            app: AppInfo {
                exe: name.to_ascii_lowercase(),
                title: String::from_utf16_lossy(&title[..n.max(0) as usize]),
            },
            rect,
            identity: hwnd.0 as usize as u64,
        })
    }
}

struct Search<'a> {
    exe: &'a str,
    found: Option<AppWindow>,
}
unsafe extern "system" fn visit(hwnd: HWND, param: LPARAM) -> BOOL {
    // SAFETY: EnumWindows calls synchronously with the pointer to our live Search value.
    let search = unsafe { &mut *(param.0 as *mut Search<'_>) };
    search.found = inspect(hwnd, search.exe);
    BOOL::from(search.found.is_none())
}

pub fn find(exe: &str) -> Option<AppWindow> {
    // SAFETY: the callback cannot outlive Search, and never retains a window handle.
    unsafe {
        if let Some(window) = inspect(GetForegroundWindow(), exe) {
            return Some(window);
        }
        let mut search = Search { exe, found: None };
        let _ = EnumWindows(Some(visit), LPARAM(&mut search as *mut Search<'_> as isize));
        search.found
    }
}

pub fn find_by_id(exe: &str, identity: u64) -> Option<AppWindow> {
    let handle = usize::try_from(identity).ok()?;
    if handle == 0 {
        return None;
    }
    inspect(HWND(handle as *mut std::ffi::c_void), exe)
}
