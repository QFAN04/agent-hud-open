//! Navigate an already-running Antigravity Desktop on Windows.
//!
//! The DevTools port file alone is not an authority: associate it with a
//! visible window belonging to the current user's installed Desktop process
//! before sending a navigation command to loopback.
use std::{
    collections::{HashMap, HashSet},
    ffi::c_void,
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    Security::{EqualSid, GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER},
    System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
        PROCESS_QUERY_LIMITED_INFORMATION,
    },
};

type Window = *mut c_void;

#[link(name = "user32")]
unsafe extern "system" {
    fn EnumWindows(callback: unsafe extern "system" fn(Window, isize) -> i32, data: isize) -> i32;
    fn IsWindowVisible(window: Window) -> i32;
    fn IsIconic(window: Window) -> i32;
    fn GetWindow(window: Window, command: u32) -> Window;
    fn GetWindowThreadProcessId(window: Window, process: *mut u32) -> u32;
    fn GetWindowTextLengthW(window: Window) -> i32;
    fn GetWindowTextW(window: Window, text: *mut u16, max_count: i32) -> i32;
    fn ShowWindow(window: Window, command: i32) -> i32;
    fn SetForegroundWindow(window: Window) -> i32;
}

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn token_user(process: HANDLE) -> Option<Vec<usize>> {
    unsafe {
        let mut token = ptr::null_mut();
        if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let token = Handle(token);
        let mut size = 0;
        GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut size);
        if size == 0 || size > 65536 {
            return None;
        }
        let mut data = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
        if GetTokenInformation(
            token.0,
            TokenUser,
            data.as_mut_ptr().cast(),
            size,
            &mut size,
        ) == 0
        {
            return None;
        }
        Some(data)
    }
}

fn same_sid(process: HANDLE, expected: &[usize]) -> bool {
    let Some(actual) = token_user(process) else {
        return false;
    };
    unsafe {
        let ours = &*expected.as_ptr().cast::<TOKEN_USER>();
        let theirs = &*actual.as_ptr().cast::<TOKEN_USER>();
        EqualSid(ours.User.Sid, theirs.User.Sid) != 0
    }
}

fn env_path(env: &HashMap<String, String>, key: &str) -> Option<PathBuf> {
    env.iter()
        .find(|(name, value)| name.eq_ignore_ascii_case(key) && !value.trim().is_empty())
        .map(|(_, value)| PathBuf::from(value))
}

fn desktop_candidates(home: &Path, env: &HashMap<String, String>) -> Vec<PathBuf> {
    let local = env_path(env, "LOCALAPPDATA").unwrap_or_else(|| home.join("AppData/Local"));
    let mut bases = vec![
        local.join("Programs/antigravity"),
        local.join("Programs/Antigravity IDE"),
    ];
    for key in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(root) = env_path(env, key) {
            bases.push(root.join("Antigravity"));
            bases.push(root.join("Antigravity IDE"));
        }
    }
    bases
        .into_iter()
        .flat_map(|base| ["Antigravity.exe", "Antigravity IDE.exe"].map(|exe| base.join(exe)))
        .collect()
}

fn temp_directories(env: &HashMap<String, String>) -> Vec<PathBuf> {
    ["TEMP", "TMP"]
        .into_iter()
        .filter_map(|key| env_path(env, key))
        .map(|dir| std::fs::canonicalize(&dir).unwrap_or(dir))
        .collect()
}

fn normalize_for_compare(path: &Path) -> String {
    let text = path.to_string_lossy().replace('/', "\\");
    text.strip_prefix(r"\\?\")
        .unwrap_or(&text)
        .to_ascii_lowercase()
}

fn matches_desktop_path(canonical: &Path, installed: &[PathBuf], temp_dirs: &[PathBuf]) -> bool {
    let actual = normalize_for_compare(canonical);
    if installed
        .iter()
        .any(|expected| actual == normalize_for_compare(expected))
    {
        return true;
    }
    let Some(file_name) = canonical.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if !(file_name.eq_ignore_ascii_case("Antigravity.exe")
        || file_name.eq_ignore_ascii_case("Antigravity IDE.exe"))
    {
        return false;
    }
    !temp_dirs.iter().any(|temp| {
        let prefix = normalize_for_compare(temp);
        !prefix.is_empty() && Path::new(&actual).starts_with(Path::new(&prefix))
    })
}

fn installed_desktop(process: HANDLE, installed: &[PathBuf], temp_dirs: &[PathBuf]) -> bool {
    let mut name = vec![0u16; 32768];
    let mut size = name.len() as u32;
    if unsafe { QueryFullProcessImageNameW(process, 0, name.as_mut_ptr(), &mut size) } == 0 {
        return false;
    }
    let name = String::from_utf16_lossy(&name[..size as usize]);
    let Ok(canonical) = std::fs::canonicalize(name) else {
        return false;
    };
    matches_desktop_path(&canonical, installed, temp_dirs)
}

fn window_title(window: Window) -> String {
    let len = unsafe { GetWindowTextLengthW(window) };
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; len as usize + 1];
    let copied = unsafe { GetWindowTextW(window, buf.as_mut_ptr(), buf.len() as i32) };
    if copied <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..copied as usize])
}

struct Candidates<'a> {
    current_user: &'a [usize],
    installed: &'a [PathBuf],
    temp_dirs: &'a [PathBuf],
    windows: Vec<(Window, u32, String)>,
}

unsafe extern "system" fn collect_window(window: Window, parameter: isize) -> i32 {
    let candidates = unsafe { &mut *(parameter as *mut Candidates<'_>) };
    // Ignore hidden and owned tool/dialog windows.
    if unsafe { IsWindowVisible(window) == 0 || !GetWindow(window, 4).is_null() } {
        return 1;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(window, &mut pid) };
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return 1;
    }
    let process = Handle(process);
    if same_sid(process.0, candidates.current_user)
        && installed_desktop(process.0, candidates.installed, candidates.temp_dirs)
    {
        candidates.windows.push((window, pid, window_title(window)));
    }
    1
}

fn pick_target_window<T: Copy>(
    windows: Vec<(T, u32, String)>,
    mut listens_on_port: impl FnMut(u32) -> bool,
    cwd: Option<&str>,
) -> Option<T> {
    let mut checked = HashMap::new();
    let verified = windows
        .into_iter()
        .filter(|(_, pid, _)| *checked.entry(*pid).or_insert_with(|| listens_on_port(*pid)))
        .collect::<Vec<_>>();
    let pids = verified
        .iter()
        .map(|(_, pid, _)| *pid)
        .collect::<HashSet<_>>();
    if pids.len() != 1 {
        return None;
    }
    if let Some(folder) = cwd
        .and_then(|path| Path::new(path).file_name())
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(|name| name.to_ascii_lowercase())
    {
        if let Some((window, _, _)) = verified
            .iter()
            .find(|(_, _, title)| title.to_ascii_lowercase().contains(&folder))
        {
            return Some(*window);
        }
    }
    verified
        .iter()
        .find(|(_, _, title)| !title.trim().is_empty())
        .or_else(|| verified.first())
        .map(|(window, _, _)| *window)
}

pub(super) fn open(home: &Path, id: &str, cwd: Option<&str>) -> Result<bool, String> {
    if !super::is_uuid(id) {
        return Ok(false);
    }
    let Some(port) = super::antigravity::devtools_port(home) else {
        return Ok(false);
    };
    let Some(current_user) = token_user(unsafe { GetCurrentProcess() }) else {
        return Ok(false);
    };
    let env = std::env::vars().collect::<HashMap<String, String>>();
    let installed: Vec<PathBuf> = desktop_candidates(home, &env)
        .into_iter()
        .filter_map(|path| {
            path.is_file()
                .then(|| std::fs::canonicalize(path).ok())
                .flatten()
        })
        .collect();
    let temp_dirs = temp_directories(&env);
    let mut candidates = Candidates {
        current_user: &current_user,
        installed: &installed,
        temp_dirs: &temp_dirs,
        windows: Vec::new(),
    };
    let ok = unsafe { EnumWindows(collect_window, (&raw mut candidates) as isize) } != 0;
    if !ok {
        return Ok(false);
    }
    let windows = candidates
        .windows
        .into_iter()
        .map(|(window, pid, title)| (window as usize, pid, title))
        .collect();
    let Some(window) = pick_target_window(
        windows,
        |pid| crate::os::listening_ports(pid).is_ok_and(|ports| ports.contains(&port)),
        cwd,
    ) else {
        return Ok(false);
    };
    let window = window as Window;
    if !super::antigravity::navigate(id, port)? {
        return Ok(false);
    }
    unsafe {
        if IsIconic(window) != 0 {
            ShowWindow(window, 9); // SW_RESTORE
        }
        Ok(SetForegroundWindow(window) != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_candidates_include_localappdata_and_program_files() {
        let mut env = HashMap::new();
        env.insert(
            "LOCALAPPDATA".into(),
            r"C:\Users\alice\AppData\Local".into(),
        );
        env.insert("ProgramFiles".into(), r"C:\Program Files".into());
        env.insert("ProgramFiles(x86)".into(), r"C:\Program Files (x86)".into());
        let candidates = desktop_candidates(Path::new(r"C:\Users\alice"), &env);
        assert!(candidates.iter().any(|p| normalize_for_compare(p)
            == r"c:\users\alice\appdata\local\programs\antigravity\antigravity.exe"));
        assert!(candidates
            .iter()
            .any(|p| normalize_for_compare(p) == r"c:\program files\antigravity\antigravity.exe"));
        assert!(candidates
            .iter()
            .any(|p| normalize_for_compare(p)
                == r"c:\program files\antigravity ide\antigravity ide.exe"));

        let fallback = desktop_candidates(Path::new(r"C:\Users\bob"), &HashMap::new());
        assert!(fallback.iter().any(|p| normalize_for_compare(p)
            == r"c:\users\bob\appdata\local\programs\antigravity\antigravity.exe"));
    }

    #[test]
    fn matches_desktop_path_accepts_installed_or_running_custom_and_rejects_temp() {
        let installed = vec![PathBuf::from(
            r"\\?\C:\Program Files\Antigravity\Antigravity.exe",
        )];
        let temps = vec![PathBuf::from(r"C:\Users\alice\AppData\Local\Temp")];
        assert!(matches_desktop_path(
            Path::new(r"C:\Program Files\Antigravity\Antigravity.exe"),
            &installed,
            &temps
        ));
        assert!(matches_desktop_path(
            Path::new(r"D:\Portable\Antigravity\Antigravity.exe"),
            &installed,
            &temps
        ));
        assert!(matches_desktop_path(
            Path::new(r"C:\Users\alice\AppData\Local\Temp2\Antigravity.exe"),
            &installed,
            &temps
        ));
        assert!(!matches_desktop_path(
            Path::new(r"C:\Users\alice\AppData\Local\Temp\Antigravity.exe"),
            &installed,
            &temps
        ));
        assert!(!matches_desktop_path(
            Path::new(r"C:\Program Files\Google\Chrome\chrome.exe"),
            &installed,
            &temps
        ));
    }

    #[test]
    fn pick_target_window_handles_multiple_windows_on_one_pid_and_rejects_ambiguous_pids() {
        let chosen = pick_target_window(
            vec![
                (1usize, 44676, "Other Workspace - Antigravity".to_owned()),
                (2usize, 44676, "Agent HUD - Antigravity".to_owned()),
                (3usize, 99999, "Unrelated".to_owned()),
            ],
            |pid| pid == 44676,
            Some(r"C:\src\Agent HUD"),
        );
        assert_eq!(chosen, Some(2));

        let ambiguous = pick_target_window(
            vec![
                (1usize, 44676, "A - Antigravity".to_owned()),
                (2usize, 44677, "B - Antigravity".to_owned()),
            ],
            |_| true,
            None,
        );
        assert_eq!(ambiguous, None);
    }
}
