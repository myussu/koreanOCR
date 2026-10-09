//! Synthetic keyboard input, used only to press Ctrl+C for the clipboard fallback.

use std::mem::size_of;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_C,
    VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, time: 0, dwExtraInfo: 0 },
        },
    }
}

fn is_down(vk: VIRTUAL_KEY) -> bool {
    // SAFETY: GetAsyncKeyState has no preconditions. The high bit is "currently down".
    unsafe { (GetAsyncKeyState(vk.0 as i32) as u16) & 0x8000 != 0 }
}

/// Press Ctrl+C. The user is usually still holding the modifiers of our own hotkey
/// (Ctrl+Alt+T), so Alt/Shift/Win are released first or the target would see Ctrl+Alt+C.
pub fn send_ctrl_c() {
    let mut inputs: Vec<INPUT> = Vec::new();
    for modifier in [VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN] {
        if is_down(modifier) {
            inputs.push(key(modifier, true));
        }
    }
    let ctrl_was_down = is_down(VK_CONTROL);
    if !ctrl_was_down {
        inputs.push(key(VK_CONTROL, false));
    }
    inputs.push(key(VK_C, false));
    inputs.push(key(VK_C, true));
    if !ctrl_was_down {
        inputs.push(key(VK_CONTROL, true));
    }
    // SAFETY: `inputs` is a valid slice of INPUT structures.
    unsafe {
        SendInput(&inputs, size_of::<INPUT>() as i32);
    }
}
