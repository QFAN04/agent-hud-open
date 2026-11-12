use super::app_targets::{app_target, AppTarget};
use super::apps::{running_executable, shortcut_targets};
use super::engines::find_store_packages;
use super::{current_env, home_dir, local_app, variable};
use std::{
    ffi::{c_void, OsStr},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    ptr,
};
use windows_sys::Win32::{
    Foundation::NO_ERROR,
    System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER,
        KEY_READ, REG_EXPAND_SZ, REG_SZ,
    },
};

struct RegKey(HKEY);
impl Drop for RegKey {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { RegCloseKey(self.0) };
        }
    }
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

fn open_key(root: HKEY, subkey: &str) -> Option<RegKey> {
    let subkey = wide(subkey);
    let mut handle: HKEY = ptr::null_mut();
    let status = unsafe { RegOpenKeyExW(root, subkey.as_ptr(), 0, KEY_READ, &mut handle) };
    (status == NO_ERROR && !handle.is_null()).then_some(RegKey(handle))
}

fn has_value(key: HKEY, name: &str) -> bool {
    let name = wide(name);
    let mut kind = 0u32;
    let mut size = 0u32;
    unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr(),
            ptr::null(),
            &mut kind,
            ptr::null_mut(),
            &mut size,
        ) == NO_ERROR
    }
}

fn query_string(key: HKEY, name: Option<&str>) -> Option<String> {
    let wide_name = name.map(wide);
    let name_ptr = wide_name.as_ref().map_or(ptr::null(), |v| v.as_ptr());
    let mut kind = 0u32;
    let mut size = 0u32;
    if unsafe {
        RegQueryValueExW(
            key,
            name_ptr,
            ptr::null(),
            &mut kind,
            ptr::null_mut(),
            &mut size,
        )
    } != NO_ERROR
        || (kind != REG_SZ && kind != REG_EXPAND_SZ)
        || size == 0
    {
        return None;
    }
    let mut buf = vec![0u16; (size as usize).div_ceil(2)];
    let mut len = (buf.len() * 2) as u32;
    if unsafe {
        RegQueryValueExW(
            key,
            name_ptr,
            ptr::null(),
            &mut kind,
            buf.as_mut_ptr().cast(),
            &mut len,
        )
    } != NO_ERROR
    {
        return None;
    }
    let units = (len as usize / 2).min(buf.len());
    let text = String::from_utf16_lossy(&buf[..units]);
    Some(text.trim_end_matches('\0').to_string())
}

pub(super) fn url_scheme(url: &str) -> Option<&str> {
    let (scheme, _) = url.split_once(':')?;
    let mut bytes = scheme.bytes();
    let first = bytes.next()?;
    if !first.is_ascii_alphabetic()
        || !bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
    {
        return None;
    }
    Some(scheme)
}

fn scheme_registered_in_root(root: HKEY, subkey: &str) -> bool {
    let Some(key) = open_key(root, subkey) else {
        return false;
    };
    has_value(key.0, "URL Protocol") || open_key(key.0, "shell\\open\\command").is_some()
}

pub(crate) fn registered_url(url: &str) -> bool {
    let Some(scheme) = url_scheme(url) else {
        return false;
    };
    scheme_registered_in_root(HKEY_CURRENT_USER, &format!("Software\\Classes\\{scheme}"))
        || scheme_registered_in_root(HKEY_CLASSES_ROOT, scheme)
}

fn parse_command_exe(command_line: &str) -> Option<PathBuf> {
    let trimmed = command_line.trim();
    let exe = if let Some(rest) = trimmed.strip_prefix('"') {
        rest.split_once('"')?.0
    } else {
        trimmed.split_whitespace().next()?
    };
    let path = PathBuf::from(exe);
    path.is_file().then_some(path)
}

fn registry_scheme_exe(scheme: &str) -> Option<PathBuf> {
    for (root, subkey) in [
        (
            HKEY_CURRENT_USER,
            format!("Software\\Classes\\{scheme}\\shell\\open\\command"),
        ),
        (HKEY_CLASSES_ROOT, format!("{scheme}\\shell\\open\\command")),
    ] {
        if let Some(exe) = open_key(root, &subkey)
            .and_then(|key| query_string(key.0, None))
            .and_then(|cmd| parse_command_exe(&cmd))
        {
            return Some(exe);
        }
    }
    None
}

pub(super) fn find_app_executable(target: &AppTarget) -> Option<PathBuf> {
    if let Some(exe) = target.url_scheme.and_then(registry_scheme_exe) {
        return Some(exe);
    }
    if let Some(exe) = running_executable(target) {
        return Some(exe);
    }
    let home = home_dir().ok()?;
    let env = current_env();
    let mut dirs = vec![
        local_app(&home, &env),
        local_app(&home, &env).join("Programs"),
        variable(&env, "ProgramFiles").unwrap_or_else(|| PathBuf::from("C:/Program Files")),
    ];
    if let Some(x86) = variable(&env, "ProgramFiles(x86)") {
        dirs.push(x86);
    }
    for base in dirs {
        for folder in target.folders {
            let root = base.join(folder);
            for exe in target.exes {
                let candidate = root.join(exe);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    shortcut_targets(target, &env).into_iter().next()
}

pub(crate) fn registered_bundle(id: &str) -> bool {
    let Some(target) = app_target(id) else {
        return false;
    };
    if find_app_executable(target).is_some() {
        return true;
    }
    if let (Some(prefix), Ok(home)) = (target.store_prefix, home_dir()) {
        let env = current_env();
        if !find_store_packages(&local_app(&home, &env), prefix).is_empty() {
            return true;
        }
    }
    target
        .url_scheme
        .is_some_and(|scheme| registered_url(&format!("{scheme}://")))
}

pub(crate) fn shell_open(target: &str, parameters: Option<&str>) -> bool {
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            window: *mut c_void,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show: i32,
        ) -> *mut c_void;
    }
    if target.contains('\0') || parameters.is_some_and(|v| v.contains('\0')) {
        return false;
    }
    let operation = wide("open");
    let target = wide(target);
    let params = parameters.map(wide);
    let params_ptr = params.as_ref().map_or(ptr::null(), |v| v.as_ptr());
    unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            operation.as_ptr(),
            target.as_ptr(),
            params_ptr,
            ptr::null(),
            1,
        ) as isize
            > 32
    }
}

pub(crate) fn open_session_url(url: &str) -> Result<(), String> {
    if shell_open(url, None) {
        Ok(())
    } else {
        Err("The native client could not open this session".into())
    }
}

pub(crate) fn open_folder_in_app(bundle_id: &str, folder: &Path) -> bool {
    let Some(target) = app_target(bundle_id) else {
        return false;
    };
    if !target.accepts_folder {
        return false;
    }
    let Some(exe) = find_app_executable(target) else {
        return false;
    };
    Command::new(exe)
        .arg(folder)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

pub(crate) fn open_url_in_app(bundle_id: &str, url: &str) -> bool {
    let Some(target) = app_target(bundle_id) else {
        return false;
    };
    if url_scheme(url).is_none() {
        return false;
    }
    if target
        .url_scheme
        .is_some_and(|scheme| url_scheme(url).is_some_and(|u| u.eq_ignore_ascii_case(scheme)))
        && registered_url(url)
    {
        return shell_open(url, None);
    }
    let Some(exe) = find_app_executable(target) else {
        return false;
    };
    Command::new(exe)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_scheme_rejects_empty_and_invalid_schemes_and_unknown_handlers() {
        assert_eq!(url_scheme("claude://resume?session=1"), Some("claude"));
        assert_eq!(
            url_scheme("vscode-insiders://file/x"),
            Some("vscode-insiders")
        );
        assert_eq!(url_scheme("no-colon"), None);
        assert_eq!(url_scheme("1bad://x"), None);
        assert_eq!(url_scheme("..\\evil://x"), None);
        assert!(!registered_url("agent-hud-nonexistent-scheme-999://open"));
        assert!(!registered_bundle("com.example.nonexistent"));
    }
}
