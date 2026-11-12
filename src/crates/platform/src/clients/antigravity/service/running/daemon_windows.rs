//! Fast verified Antigravity daemon discovery on Windows via
//! `~/.gemini/antigravity/daemon/hub_*.json`, validated against a live
//! same-user language-server process and its loopback listening ports.
use super::{is_server_executable, Server};
use serde::Deserialize;
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    ptr,
    time::SystemTime,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, STILL_ACTIVE},
    Security::{EqualSid, GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER},
    System::Threading::{
        GetCurrentProcess, GetExitCodeProcess, OpenProcess, OpenProcessToken,
        QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    },
};

const MAX_HUB_BYTES: u64 = 16 * 1024;
const MAX_HUB_FILES: usize = 64;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawHubEntry {
    pid: u32,
    https_port: Option<u16>,
    http_port: Option<u16>,
    #[serde(default)]
    csrf_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HubEntry {
    pid: u32,
    https_port: Option<u16>,
    http_port: Option<u16>,
    csrf_token: String,
}

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

pub(super) fn servers() -> Vec<Server> {
    let Ok(home) = crate::os::home_dir() else {
        return Vec::new();
    };
    let Some(current_user) = token_user(unsafe { GetCurrentProcess() }) else {
        return Vec::new();
    };
    let dirs = daemon_dirs(&home);
    discover_in_dirs(
        &dirs,
        |pid| verify_running_server(pid, &current_user),
        crate::os::listening_ports,
    )
}

fn daemon_dirs(home: &Path) -> Vec<PathBuf> {
    ["antigravity", "antigravity-ide", "antigravity-cli"]
        .into_iter()
        .map(|name| home.join(".gemini").join(name).join("daemon"))
        .collect()
}

fn discover_in_dirs(
    dirs: &[PathBuf],
    mut verify_pid: impl FnMut(u32) -> bool,
    mut listening_ports: impl FnMut(u32) -> Result<Vec<u16>, String>,
) -> Vec<Server> {
    let mut files = Vec::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if !(name.starts_with("hub_") && name.ends_with(".json")) {
                continue;
            }
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if !meta.is_file() || meta.len() > MAX_HUB_BYTES {
                continue;
            }
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            files.push((modified, path));
        }
    }
    files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    files.truncate(MAX_HUB_FILES);

    let mut seen_pids = HashSet::new();
    let mut result = Vec::new();
    for (_, path) in files {
        let Some(bytes) = read_bounded(&path) else {
            continue;
        };
        let Some(entry) = parse_hub_entry(&bytes) else {
            continue;
        };
        if seen_pids.contains(&entry.pid) || !verify_pid(entry.pid) {
            continue;
        }
        let Ok(ports) = listening_ports(entry.pid) else {
            continue;
        };
        let Some(server) = verified_server(entry, &ports) else {
            continue;
        };
        seen_pids.insert(server.pid);
        result.push(server);
    }
    result.sort_by_key(|server| (server.priority, server.pid));
    result
}

fn read_bounded(path: &Path) -> Option<Vec<u8>> {
    let file = fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_HUB_BYTES + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= MAX_HUB_BYTES).then_some(bytes)
}

fn parse_hub_entry(bytes: &[u8]) -> Option<HubEntry> {
    let raw: RawHubEntry = serde_json::from_slice(bytes).ok()?;
    if raw.pid == 0
        || raw.csrf_token.is_empty()
        || raw.csrf_token.len() > 1024
        || raw
            .csrf_token
            .bytes()
            .any(|b| matches!(b, 0 | b'\r' | b'\n'))
    {
        return None;
    }
    let https_port = raw.https_port.filter(|p| *p > 0);
    let http_port = raw.http_port.filter(|p| *p > 0);
    if https_port.is_none() && http_port.is_none() {
        return None;
    }
    Some(HubEntry {
        pid: raw.pid,
        https_port,
        http_port,
        csrf_token: raw.csrf_token,
    })
}

fn verified_server(entry: HubEntry, listening_ports: &[u16]) -> Option<Server> {
    let https_ok = entry
        .https_port
        .is_some_and(|port| listening_ports.contains(&port));
    let verified_http = entry
        .http_port
        .filter(|port| listening_ports.contains(port));
    if !https_ok && verified_http.is_none() {
        return None;
    }
    let extension = verified_http.map(|port| (port, entry.csrf_token.clone()));
    Some(Server {
        pid: entry.pid,
        token: entry.csrf_token,
        extension,
        priority: 0,
    })
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

fn verify_running_server(pid: u32, current_user: &[usize]) -> bool {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return false;
    }
    let process = Handle(process);
    let mut exit_code = 0u32;
    if unsafe { GetExitCodeProcess(process.0, &mut exit_code) } == 0
        || exit_code != STILL_ACTIVE as u32
    {
        return false;
    }
    if !same_sid(process.0, current_user) {
        return false;
    }
    let mut name = vec![0u16; 32768];
    let mut size = name.len() as u32;
    if unsafe { QueryFullProcessImageNameW(process.0, 0, name.as_mut_ptr(), &mut size) } == 0 {
        return false;
    }
    let executable = String::from_utf16_lossy(&name[..size as usize]);
    let normalized = executable.to_lowercase().replace('\\', "/");
    is_server_executable(&normalized)
        || matches!(normalized.rsplit('/').next(), Some("agy" | "agy.exe"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_and_rejects_malformed_hub_entries() {
        let ok = parse_hub_entry(
            br#"{"pid":54156,"httpsPort":9392,"httpPort":9393,"lsVersion":"2.31.0","csrfToken":"tok-abc"}"#,
        )
        .unwrap();
        assert_eq!(ok.pid, 54156);
        assert_eq!(ok.https_port, Some(9392));
        assert_eq!(ok.http_port, Some(9393));
        assert_eq!(ok.csrf_token, "tok-abc");

        for invalid in [
            br#"{"pid":0,"httpsPort":9392,"csrfToken":"tok"}"#.as_slice(),
            br#"{"pid":12,"httpsPort":0,"httpPort":0,"csrfToken":"tok"}"#.as_slice(),
            br#"{"pid":12,"httpsPort":9392,"csrfToken":""}"#.as_slice(),
            br#"{"pid":12,"httpsPort":9392,"csrfToken":"bad\ntoken"}"#.as_slice(),
            b"not json".as_slice(),
        ] {
            assert!(parse_hub_entry(invalid).is_none());
        }
    }

    #[test]
    fn discovers_only_verified_live_daemon_with_matching_listening_port() {
        let temp = tempfile::tempdir().unwrap();
        let daemon = temp.path().join(".gemini/antigravity/daemon");
        fs::create_dir_all(&daemon).unwrap();
        fs::write(
            daemon.join("hub_1001.json"),
            r#"{"pid":111,"httpsPort":1000,"httpPort":1001,"csrfToken":"stale-pid"}"#,
        )
        .unwrap();
        fs::write(
            daemon.join("hub_2001.json"),
            r#"{"pid":222,"httpsPort":2000,"httpPort":2001,"csrfToken":"wrong-port"}"#,
        )
        .unwrap();
        fs::write(
            daemon.join("hub_9393.json"),
            r#"{"pid":54156,"httpsPort":9392,"httpPort":9393,"csrfToken":"live-token"}"#,
        )
        .unwrap();
        fs::write(
            daemon.join("unrelated.json"),
            r#"{"pid":54156,"httpsPort":9392,"csrfToken":"x"}"#,
        )
        .unwrap();

        let servers = discover_in_dirs(
            &daemon_dirs(temp.path()),
            |pid| pid == 222 || pid == 54156,
            |pid| match pid {
                222 => Ok(vec![8888]),
                54156 => Ok(vec![4004, 9392, 9393]),
                _ => Err("dead".into()),
            },
        );
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].pid, 54156);
        assert_eq!(servers[0].token, "live-token");
        assert_eq!(servers[0].extension, Some((9393, "live-token".into())));
    }

    #[test]
    fn current_process_sid_verification_accepts_self_and_rejects_pid_zero() {
        let sid = token_user(unsafe { GetCurrentProcess() }).unwrap();
        assert!(same_sid(unsafe { GetCurrentProcess() }, &sid));
        assert!(!verify_running_server(0, &sid));
    }
}
