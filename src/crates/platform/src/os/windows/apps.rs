use super::acl;
use super::app_targets::{app_target, AppTarget};
use super::associations::{find_app_executable, registered_url, shell_open};
use super::engines::find_store_packages;
use super::process_win32::{process_image_path, process_sid, OwnedHandle};
use super::{current_env, home_dir, local_app, shortcut, variable, Environment};
use std::{
    collections::HashSet,
    mem::{size_of, zeroed},
    path::{Path, PathBuf},
};
use windows_sys::{
    core::BOOL,
    Win32::{
        Foundation::{CloseHandle, HWND, INVALID_HANDLE_VALUE, LPARAM},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
        UI::WindowsAndMessaging::{
            EnumWindows, GetWindow, GetWindowTextLengthW, GetWindowThreadProcessId, IsIconic,
            IsWindowVisible, SetForegroundWindow, ShowWindow, GW_OWNER, SW_RESTORE,
        },
    },
};

fn wide_name(buf: &[u16; 260]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

fn looks_like_cli_binary(path: &Path) -> bool {
    let text = path
        .to_string_lossy()
        .to_ascii_lowercase()
        .replace('\\', "/");
    [
        "/.local/bin/",
        "/.local/share/claude/",
        "/node_modules/",
        "/npm/",
        "/pnpm/",
        "/scoop/shims/",
        "/.codex/.sandbox-bin/",
        "/.claude/local/",
        "/claude/claude-code/",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

fn is_desktop_process(pid: u32, target: &AppTarget, owner: Option<&str>) -> bool {
    let Ok(handle) = OwnedHandle::open_limited(pid) else {
        return false;
    };
    let same_user = owner.is_none_or(|sid| process_sid(handle.0).as_deref() == Some(sid));
    if !same_user {
        return false;
    }
    !target.shared_cli_name
        || process_image_path(handle.0).is_none_or(|exe| !looks_like_cli_binary(&exe))
}

fn matching_app_pids(target: &AppTarget) -> HashSet<u32> {
    let mut pids = HashSet::new();
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snap.is_null() || snap == INVALID_HANDLE_VALUE {
        return pids;
    }
    let owner = acl::sid().ok();
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..unsafe { zeroed() }
    };
    if unsafe { Process32FirstW(snap, &mut entry) } != 0 {
        loop {
            let exe = wide_name(&entry.szExeFile);
            if target
                .exes
                .iter()
                .any(|wanted| exe.eq_ignore_ascii_case(wanted))
                && is_desktop_process(entry.th32ProcessID, target, owner.as_deref())
            {
                pids.insert(entry.th32ProcessID);
            }
            if unsafe { Process32NextW(snap, &mut entry) } == 0 {
                break;
            }
        }
    }
    unsafe { CloseHandle(snap) };
    pids
}

struct WindowSearch<'a> {
    pids: &'a HashSet<u32>,
    found: Option<HWND>,
}

unsafe extern "system" fn enum_app_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let search = &mut *(lparam as *mut WindowSearch<'_>);
    if IsWindowVisible(hwnd) == 0
        || !GetWindow(hwnd, GW_OWNER).is_null()
        || GetWindowTextLengthW(hwnd) <= 0
    {
        return 1;
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if search.pids.contains(&pid) {
        search.found = Some(hwnd);
        return 0;
    }
    1
}

fn find_app_window(pids: &HashSet<u32>) -> Option<HWND> {
    if pids.is_empty() {
        return None;
    }
    let mut search = WindowSearch { pids, found: None };
    unsafe {
        EnumWindows(Some(enum_app_window), (&raw mut search) as LPARAM);
    }
    search.found
}

pub(super) fn running_executable(target: &AppTarget) -> Option<PathBuf> {
    matching_app_pids(target).into_iter().find_map(|pid| {
        let handle = OwnedHandle::open_limited(pid).ok()?;
        let path = process_image_path(handle.0)?;
        (path.is_absolute() && path.is_file()).then_some(path)
    })
}

fn matches_target_exe(path: &Path, target: &AppTarget) -> bool {
    let named = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            target
                .exes
                .iter()
                .any(|want| want.eq_ignore_ascii_case(name))
        });
    named && (!target.shared_cli_name || !looks_like_cli_binary(path))
}

fn shortcut_targets_in(roots: &[PathBuf], target: &AppTarget) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for resolved in roots.iter().flat_map(|root| shortcut::targets(root)) {
        if matches_target_exe(&resolved, target) && resolved.is_file() && !found.contains(&resolved)
        {
            found.push(resolved);
        }
    }
    found
}

/// Start menu and desktop shortcuts whose target executable matches `target`.
pub(super) fn shortcut_targets(target: &AppTarget, env: &Environment) -> Vec<PathBuf> {
    let start_menu = Path::new("Microsoft/Windows/Start Menu/Programs");
    let roots: Vec<PathBuf> = [
        variable(env, "APPDATA").map(|dir| dir.join(start_menu)),
        variable(env, "ProgramData").map(|dir| dir.join(start_menu)),
        variable(env, "USERPROFILE").map(|dir| dir.join("Desktop")),
        variable(env, "PUBLIC").map(|dir| dir.join("Desktop")),
    ]
    .into_iter()
    .flatten()
    .collect();
    shortcut_targets_in(&roots, target)
}

pub(crate) fn app_running(bundle_id: &str) -> bool {
    let Some(target) = app_target(bundle_id) else {
        return false;
    };
    !matching_app_pids(target).is_empty()
}

pub(crate) fn activate_app(bundle_id: &str) -> bool {
    let Some(target) = app_target(bundle_id) else {
        return false;
    };
    let pids = matching_app_pids(target);
    let Some(hwnd) = find_app_window(&pids) else {
        return false;
    };
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd) != 0
    }
}

pub(crate) fn launch_app(bundle_id: &str) -> bool {
    let Some(target) = app_target(bundle_id) else {
        return false;
    };
    if find_app_executable(target).is_some_and(|exe| shell_open(&exe.to_string_lossy(), None)) {
        return true;
    }
    let (Some(prefix), Some(scheme), Ok(home)) =
        (target.store_prefix, target.url_scheme, home_dir())
    else {
        return false;
    };
    let env = current_env();
    if find_store_packages(&local_app(&home, &env), prefix).is_empty() {
        return false;
    }
    let link = format!("{scheme}://");
    registered_url(&link) && shell_open(&link, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_bundle_is_neither_running_nor_activatable() {
        assert!(!app_running("com.example.nonexistent-app-id"));
        assert!(!activate_app("com.example.nonexistent-app-id"));
        assert!(!launch_app("com.example.nonexistent-app-id"));
    }

    #[test]
    fn cli_path_filter_distinguishes_claude_cli_from_desktop_app() {
        assert!(looks_like_cli_binary(Path::new(
            r"C:\Users\alice\AppData\Roaming\npm\node_modules\@anthropic-ai\claude-code\bin\claude.exe"
        )));
        assert!(looks_like_cli_binary(Path::new(
            r"C:\Users\alice\.local\share\claude\versions\2.1.0\claude.exe"
        )));
        assert!(!looks_like_cli_binary(Path::new(
            r"C:\Program Files\WindowsApps\Claude_1.0.0_x64__pzs8sxrjxfjjc\app\Claude.exe"
        )));
    }

    #[test]
    fn shortcut_matching_accepts_desktop_exes_and_filters_shared_clis() {
        let claude = app_target("com.anthropic.claudefordesktop").unwrap();
        assert!(!matches_target_exe(
            Path::new(r"C:\Users\alice\.local\share\claude\versions\2.1.0\claude.exe"),
            claude
        ));
        assert!(matches_target_exe(
            Path::new(r"D:\Apps\Claude\Claude.exe"),
            claude
        ));
        let chatgpt = app_target("com.openai.chat").unwrap();
        assert!(matches_target_exe(
            Path::new(r"D:\ChatGPT\app\ChatGPT.exe"),
            chatgpt
        ));
    }
}
