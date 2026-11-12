#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
mod host_windows;
#[cfg(target_os = "windows")]
use host_windows as host;

#[cfg(not(target_os = "windows"))]
mod host_unix;
#[cfg(not(target_os = "windows"))]
use host_unix as host;

fn main() {
    agent_hud_host::run_with_options(tauri::generate_context!(), None, host::options());
}
