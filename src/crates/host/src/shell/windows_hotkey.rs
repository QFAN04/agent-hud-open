//! Global Ctrl+Alt+H shortcut through Win32 `RegisterHotKey` on a dedicated
//! hidden message-only window thread, matching the tray menu's Alt+CmdOrCtrl+H.
use std::{
    ffi::c_void,
    sync::mpsc,
    thread::{self, JoinHandle},
};
use tauri::Manager;
use windows::core::{w, PCWSTR};
use windows::Win32::{
    Foundation::{
        GetLastError, ERROR_CLASS_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::KeyboardAndMouse::{
            RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL,
            MOD_NOREPEAT, VK_H,
        },
        WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
            PostMessageW, RegisterClassW, TranslateMessage, UnregisterClassW, HWND_MESSAGE, MSG,
            WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_HOTKEY, WNDCLASSW,
        },
    },
};

const HOTKEY_ID: i32 = 1;
const HOTKEY_MODIFIERS: HOT_KEY_MODIFIERS =
    HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_ALT.0 | MOD_NOREPEAT.0);
const HOTKEY_VK: u32 = VK_H.0 as u32;
const HOTKEY_CLASS: PCWSTR = w!("AgentHudHotkeyWindow");

struct HotkeyGuard {
    hwnd: usize,
    thread: Option<JoinHandle<()>>,
}

impl Drop for HotkeyGuard {
    fn drop(&mut self) {
        if self.hwnd != 0 {
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(self.hwnd as *mut c_void)),
                    WM_CLOSE,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(super) fn install(app: &tauri::AppHandle) {
    if app.try_state::<HotkeyGuard>().is_some() {
        return;
    }
    let app_handle = app.clone();
    let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<usize, String>>(1);
    let Ok(thread) = thread::Builder::new()
        .name("agent-hud-hotkey".into())
        .spawn(move || run_loop(app_handle, ready_tx))
    else {
        eprintln!("Agent HUD hotkey (Ctrl+Alt+H) thread failed to spawn");
        return;
    };

    match ready_rx.recv() {
        Ok(Ok(hwnd)) => {
            app.manage(HotkeyGuard {
                hwnd,
                thread: Some(thread),
            });
        }
        Ok(Err(error)) => {
            eprintln!("Agent HUD hotkey (Ctrl+Alt+H) unavailable: {error}");
            let _ = thread.join();
        }
        Err(_) => {
            eprintln!("Agent HUD hotkey (Ctrl+Alt+H) listener failed to initialize");
            let _ = thread.join();
        }
    }
}

fn run_loop(app: tauri::AppHandle, ready_tx: mpsc::SyncSender<Result<usize, String>>) {
    let (hwnd, instance) = match create_hotkey_window() {
        Ok(pair) => {
            let _ = ready_tx.send(Ok(pair.0 .0 as usize));
            pair
        }
        Err(error) => {
            let _ = ready_tx.send(Err(error));
            return;
        }
    };

    let mut msg = MSG::default();
    loop {
        let status = unsafe { GetMessageW(&mut msg, None, 0, 0).0 };
        if status <= 0 {
            break;
        }
        if msg.message == WM_HOTKEY && matches_hotkey(msg.wParam.0) {
            crate::coordinator::toggle_glow(&app);
            continue;
        }
        if msg.message == WM_CLOSE && msg.hwnd == hwnd {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    unsafe {
        let _ = UnregisterHotKey(Some(hwnd), HOTKEY_ID);
        let _ = DestroyWindow(hwnd);
        let _ = UnregisterClassW(HOTKEY_CLASS, Some(instance));
    }
}

fn create_hotkey_window() -> Result<(HWND, HINSTANCE), String> {
    let instance: HINSTANCE = unsafe { GetModuleHandleW(None) }
        .map_err(|error| error.to_string())?
        .into();
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wnd_proc),
        hInstance: instance,
        lpszClassName: HOTKEY_CLASS,
        ..Default::default()
    };
    if unsafe { RegisterClassW(&wc) } == 0
        && unsafe { GetLastError() } != ERROR_CLASS_ALREADY_EXISTS
    {
        return Err("RegisterClassW failed for hotkey window".into());
    }
    let hwnd = match unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            HOTKEY_CLASS,
            w!(""),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance),
            None,
        )
    } {
        Ok(hwnd) => hwnd,
        Err(error) => {
            unsafe {
                let _ = UnregisterClassW(HOTKEY_CLASS, Some(instance));
            }
            return Err(error.to_string());
        }
    };
    if let Err(error) =
        unsafe { RegisterHotKey(Some(hwnd), HOTKEY_ID, HOTKEY_MODIFIERS, HOTKEY_VK) }
    {
        unsafe {
            let _ = DestroyWindow(hwnd);
            let _ = UnregisterClassW(HOTKEY_CLASS, Some(instance));
        }
        return Err(error.to_string());
    }
    Ok((hwnd, instance))
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

const fn matches_hotkey(wparam: usize) -> bool {
    wparam == HOTKEY_ID as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_SHIFT, MOD_WIN};

    #[test]
    fn hotkey_matches_ctrl_alt_h_contract() {
        assert!(HOTKEY_MODIFIERS.contains(MOD_CONTROL));
        assert!(HOTKEY_MODIFIERS.contains(MOD_ALT));
        assert!(HOTKEY_MODIFIERS.contains(MOD_NOREPEAT));
        assert!(!HOTKEY_MODIFIERS.contains(MOD_SHIFT));
        assert!(!HOTKEY_MODIFIERS.contains(MOD_WIN));
        assert_eq!(HOTKEY_VK, u32::from(b'H'));
        assert!(matches_hotkey(HOTKEY_ID as usize));
        assert!(!matches_hotkey(0));
        assert!(!matches_hotkey(2));
    }
}
