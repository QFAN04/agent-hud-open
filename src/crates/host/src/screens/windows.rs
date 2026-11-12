//! Display changes on Windows: observes `WM_DISPLAYCHANGE` and taskbar work-area
//! updates (`WM_SETTINGCHANGE` with `SPI_SETWORKAREA`) on a hidden top-level
//! window, keeping a gentle fallback poll for safety.
use std::{
    cell::RefCell,
    ffi::c_void,
    sync::{mpsc, Mutex, PoisonError},
    thread::{self, JoinHandle},
    time::Duration,
};
use tauri::Manager;
use windows::core::{w, PCWSTR};
use windows::Win32::{
    Foundation::{
        GetLastError, ERROR_CLASS_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
        PostMessageW, RegisterClassW, TranslateMessage, UnregisterClassW, MSG, SPI_SETWORKAREA,
        WINDOW_EX_STYLE, WM_CLOSE, WM_DISPLAYCHANGE, WM_SETTINGCHANGE, WNDCLASSW, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW, WS_POPUP,
    },
};

pub const POLL: Option<Duration> = Some(Duration::from_secs(30));

const SCREEN_CLASS: PCWSTR = w!("AgentHudDisplayWatcher");

thread_local! {
    static DISPLAY_TX: RefCell<Option<mpsc::SyncSender<()>>> = const { RefCell::new(None) };
}

struct ScreenNotifications(Mutex<Option<ScreenWatcher>>);

struct ScreenWatcher {
    hwnd: usize,
    window_thread: Option<JoinHandle<()>>,
    notify_thread: Option<JoinHandle<()>>,
}

impl ScreenWatcher {
    fn stop(&mut self) {
        if self.hwnd != 0 {
            let hwnd = self.hwnd;
            self.hwnd = 0;
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(hwnd as *mut c_void)),
                    WM_CLOSE,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
        if let Some(thread) = self.window_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.notify_thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for ScreenWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn install(app: &tauri::App) -> Result<(), String> {
    if app.try_state::<ScreenNotifications>().is_some() {
        return Ok(());
    }
    let handle = app.handle().clone();
    let (notify_tx, notify_rx) = mpsc::sync_channel::<()>(1);
    let notify_thread = thread::Builder::new()
        .name("agent-hud-display-notify".into())
        .spawn(move || {
            while notify_rx.recv().is_ok() {
                crate::coordinator::screens_changed(&handle);
            }
        })
        .map_err(|error| error.to_string())?;

    let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<usize, String>>(1);
    let window_thread = match thread::Builder::new()
        .name("agent-hud-display-watch".into())
        .spawn(move || run_window_loop(notify_tx, ready_tx))
    {
        Ok(thread) => thread,
        Err(error) => {
            let _ = notify_thread.join();
            eprintln!("Agent HUD display watcher failed to spawn; falling back to poll: {error}");
            return Ok(());
        }
    };

    match ready_rx.recv() {
        Ok(Ok(hwnd)) => {
            app.manage(ScreenNotifications(Mutex::new(Some(ScreenWatcher {
                hwnd,
                window_thread: Some(window_thread),
                notify_thread: Some(notify_thread),
            }))));
        }
        Ok(Err(error)) => {
            let _ = window_thread.join();
            let _ = notify_thread.join();
            eprintln!("Agent HUD display watcher unavailable; falling back to poll: {error}");
        }
        Err(_) => {
            let _ = window_thread.join();
            let _ = notify_thread.join();
            eprintln!("Agent HUD display watcher failed to initialize; falling back to poll");
        }
    }
    Ok(())
}

pub fn shutdown(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<ScreenNotifications>() {
        if let Some(mut watcher) = state
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            watcher.stop();
        }
    }
}

fn run_window_loop(
    notify_tx: mpsc::SyncSender<()>,
    ready_tx: mpsc::SyncSender<Result<usize, String>>,
) {
    DISPLAY_TX.with(|cell| {
        *cell.borrow_mut() = Some(notify_tx);
    });
    let (hwnd, instance) = match create_top_level_listener() {
        Ok(pair) => {
            let _ = ready_tx.send(Ok(pair.0 .0 as usize));
            pair
        }
        Err(error) => {
            DISPLAY_TX.with(|cell| {
                *cell.borrow_mut() = None;
            });
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
        if msg.message == WM_CLOSE && msg.hwnd == hwnd {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    unsafe {
        let _ = DestroyWindow(hwnd);
        let _ = UnregisterClassW(SCREEN_CLASS, Some(instance));
    }
    DISPLAY_TX.with(|cell| {
        *cell.borrow_mut() = None;
    });
}

fn create_top_level_listener() -> Result<(HWND, HINSTANCE), String> {
    let instance: HINSTANCE = unsafe { GetModuleHandleW(None) }
        .map_err(|error| error.to_string())?
        .into();
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wnd_proc),
        hInstance: instance,
        lpszClassName: SCREEN_CLASS,
        ..Default::default()
    };
    if unsafe { RegisterClassW(&wc) } == 0
        && unsafe { GetLastError() } != ERROR_CLASS_ALREADY_EXISTS
    {
        return Err("RegisterClassW failed for display watcher".into());
    }
    // Broadcast messages like WM_DISPLAYCHANGE only reach top-level windows,
    // not HWND_MESSAGE windows. Keep it hidden and non-activating.
    match unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0),
            SCREEN_CLASS,
            w!(""),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )
    } {
        Ok(hwnd) => Ok((hwnd, instance)),
        Err(error) => {
            unsafe {
                let _ = UnregisterClassW(SCREEN_CLASS, Some(instance));
            }
            Err(error.to_string())
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if is_display_change_message(msg, wparam) {
        DISPLAY_TX.with(|cell| {
            if let Some(tx) = cell.borrow().as_ref() {
                let _ = tx.try_send(());
            }
        });
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

const fn is_display_change_message(msg: u32, wparam: WPARAM) -> bool {
    msg == WM_DISPLAYCHANGE || (msg == WM_SETTINGCHANGE && wparam.0 == SPI_SETWORKAREA.0 as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_display_and_work_area_broadcasts() {
        assert!(is_display_change_message(WM_DISPLAYCHANGE, WPARAM(0)));
        assert!(is_display_change_message(
            WM_SETTINGCHANGE,
            WPARAM(SPI_SETWORKAREA.0 as usize)
        ));
        assert!(!is_display_change_message(WM_SETTINGCHANGE, WPARAM(0)));
        assert!(!is_display_change_message(WM_CLOSE, WPARAM(0)));
    }
}
