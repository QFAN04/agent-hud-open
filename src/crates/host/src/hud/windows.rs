//! The Windows HUD host: a WebView2 window positioned in physical desktop
//! pixels, with Win32 button and modifier state. Geometry, hover and drag state
//! stay in the shared owner.
pub(super) use super::generic::{
    global_pointer, panel_builder, place, set_drag_cursor, set_ignore_cursor, shown,
};
use super::PointerKeys;
#[path = "windows/displays.rs"]
mod displays;
#[path = "windows/page.rs"]
mod page;
pub(super) use displays::{measure_screens, screen};
pub(super) use page::watch_page;

/// Windows reports global physical desktop pixels, with a per-window DPI scale.
pub(super) const GLOBAL_POINTS: bool = false;
pub(super) const COORDINATE_SPACE: &str = "monitor-scaled-physical-top-left";

pub(super) fn native_drag() -> bool {
    true
}

pub(super) fn fixed_canvas() -> bool {
    false
}

pub(super) fn edge_hud_unsupported() -> Option<&'static str> {
    None
}

/// The window that had the keyboard before the HUD took it for typing.
pub(super) struct TextFocusOrigin(isize);

fn hwnd(window: &tauri::WebviewWindow) -> Result<windows::Win32::Foundation::HWND, String> {
    let raw = window.hwnd().map_err(|error| error.to_string())?.0 as isize;
    Ok(windows::Win32::Foundation::HWND(
        raw as *mut core::ffi::c_void,
    ))
}

/// Gives the HUD the keyboard for a text field and hands focus into WebView2.
/// Runs on the UI thread (via `with_webview`) so `AttachThreadInput`, `SetFocus`,
/// and `ICoreWebView2Controller::MoveFocus` apply to the HUD's owning thread
/// even when the HUD already held the foreground (`GetForegroundWindow == target`)
/// or when `set_focusable(true)`'s async style update has not drained yet.
pub(super) fn begin_text_focus(
    window: &tauri::WebviewWindow,
) -> Result<Option<TextFocusOrigin>, String> {
    use std::{sync::mpsc, time::Duration};
    use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC;
    use windows::Win32::{
        System::Threading::{AttachThreadInput, GetCurrentThreadId},
        UI::{
            Input::KeyboardAndMouse::SetFocus,
            WindowsAndMessaging::{
                BringWindowToTop, GetForegroundWindow, GetWindowLongW, GetWindowThreadProcessId,
                SetForegroundWindow, SetWindowLongW, SetWindowPos, GWL_EXSTYLE, SWP_FRAMECHANGED,
                SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_EX_NOACTIVATE,
            },
        },
    };
    let target_hwnd = hwnd(window)?.0 as isize;
    let fallback = window.clone();
    let (sender, receiver) = mpsc::sync_channel(1);
    window
        .with_webview(move |platform| {
            let result = (|| -> Result<Option<TextFocusOrigin>, String> {
                let target =
                    windows::Win32::Foundation::HWND(target_hwnd as *mut core::ffi::c_void);
                unsafe {
                    let previous = GetForegroundWindow();
                    let origin = (!previous.is_invalid() && previous != target)
                        .then_some(TextFocusOrigin(previous.0 as isize));
                    // Ensure WS_EX_NOACTIVATE is cleared synchronously before SetForegroundWindow
                    // if set_focusable(true)'s queued Tao window update has not run yet.
                    let ex_style = GetWindowLongW(target, GWL_EXSTYLE) as u32;
                    if (ex_style & WS_EX_NOACTIVATE.0) != 0 {
                        SetWindowLongW(
                            target,
                            GWL_EXSTYLE,
                            (ex_style & !WS_EX_NOACTIVATE.0) as i32,
                        );
                        let _ = SetWindowPos(
                            target,
                            None,
                            0,
                            0,
                            0,
                            0,
                            SWP_NOMOVE
                                | SWP_NOSIZE
                                | SWP_NOZORDER
                                | SWP_NOACTIVATE
                                | SWP_FRAMECHANGED,
                        );
                    }
                    let ours = GetCurrentThreadId();
                    let theirs = if previous.is_invalid() {
                        0
                    } else {
                        GetWindowThreadProcessId(previous, None)
                    };
                    let attached = theirs != 0
                        && theirs != ours
                        && AttachThreadInput(ours, theirs, true).as_bool();
                    let _ = BringWindowToTop(target);
                    let _ = SetForegroundWindow(target);
                    let _ = SetFocus(Some(target));
                    if attached {
                        let _ = AttachThreadInput(ours, theirs, false);
                    }
                    if GetForegroundWindow() != target {
                        // The last resort Tauri offers.
                        super::generic::begin_text_focus(&fallback)?;
                    }
                    platform
                        .controller()
                        .MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC)
                        .map_err(|error| error.to_string())?;
                    Ok(origin)
                }
            })();
            let _ = sender.send(result);
        })
        .map_err(|error| error.to_string())?;
    receiver
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "WebView2 did not acquire text focus in time".to_owned())?
}

/// Returns the keyboard to the window that had it, if it is still there.
pub(super) fn end_text_focus(
    window: &tauri::WebviewWindow,
    previous: Option<TextFocusOrigin>,
) -> Result<(), String> {
    use std::{sync::mpsc, time::Duration};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, IsWindow, SetForegroundWindow,
    };
    let Some(TextFocusOrigin(previous)) = previous else {
        return super::generic::end_text_focus(window, None);
    };
    let target_hwnd = hwnd(window)?.0 as isize;
    let (sender, receiver) = mpsc::sync_channel(1);
    window
        .run_on_main_thread(move || {
            let target = windows::Win32::Foundation::HWND(target_hwnd as *mut core::ffi::c_void);
            let previous = windows::Win32::Foundation::HWND(previous as *mut core::ffi::c_void);
            unsafe {
                // Only while the HUD still holds the keyboard: a window the person
                // has since clicked into keeps it.
                if GetForegroundWindow() == target && IsWindow(Some(previous)).as_bool() {
                    let _ = SetForegroundWindow(previous);
                }
            }
            let _ = sender.send(());
        })
        .map_err(|error| error.to_string())?;
    receiver
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "Windows HUD focus release did not complete in time".to_owned())
}

/// Whether the HUD still holds Windows keyboard focus while a text lease is
/// active. `tao`'s `is_focused()` turns false when `WM_KILLFOCUS` fires on the
/// outer HWND as focus enters WebView2's child window, so check foreground and
/// ownership against the HUD `HWND` instead.
pub(super) fn is_text_focus_active(window: &tauri::WebviewWindow) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetForegroundWindow, IsChild, GA_ROOTOWNER,
    };
    let Ok(target) = hwnd(window) else {
        return super::generic::is_text_focus_active(window);
    };
    unsafe {
        let foreground = GetForegroundWindow();
        !foreground.is_invalid()
            && (foreground == target
                || IsChild(target, foreground).as_bool()
                || GetAncestor(foreground, GA_ROOTOWNER) == target)
    }
}

fn key_down(virtual_key: i32) -> bool {
    // Query only button/modifier state. The low "pressed since last call" bit is
    // deliberately ignored; no keyboard event stream is installed.
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetAsyncKeyState(virtual_key: i32) -> i16;
    }
    unsafe { GetAsyncKeyState(virtual_key) < 0 }
}

pub(super) fn left_mouse_down() -> Result<bool, String> {
    Ok(key_down(mouse_button_keys().0))
}

/// GetAsyncKeyState reports physical buttons; the HUD follows the user's
/// primary button just as the WebView's pointer events do.
fn mouse_button_keys() -> (i32, i32) {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SWAPBUTTON};
    logical_mouse_buttons(unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0)
}

fn logical_mouse_buttons(swapped: bool) -> (i32, i32) {
    if swapped {
        (0x02, 0x01)
    } else {
        (0x01, 0x02)
    }
}

pub(super) fn pointer_keys() -> PointerKeys {
    let (primary, secondary) = mouse_button_keys();
    let buttons = [
        (primary, 1),
        (secondary, 2),
        (0x04, 4),
        (0x05, 8),
        (0x06, 16),
    ]
    .into_iter()
    .fold(0, |bits, (key, mask)| {
        bits | if key_down(key) { mask } else { 0 }
    });
    PointerKeys {
        buttons: Some(buttons),
        alt: Some(key_down(0x12)),
        ctrl: Some(key_down(0x11)),
        meta: Some(key_down(0x5b) || key_down(0x5c)),
        shift: Some(key_down(0x10)),
    }
}
