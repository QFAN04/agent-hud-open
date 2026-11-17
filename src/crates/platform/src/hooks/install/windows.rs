use std::path::{Path, PathBuf};

fn clean_windows_path(path: &Path) -> String {
    let slashed = path.to_string_lossy().replace('/', "\\");
    let mut stripped = if slashed
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"\\?\UNC\"))
    {
        format!(r"\\{}", &slashed[8..])
    } else if let Some(rest) = slashed.strip_prefix(r"\\?\") {
        rest.to_owned()
    } else {
        slashed
    };
    while stripped.len() > 2
        && stripped.ends_with('\\')
        && !(stripped.len() == 3
            && stripped
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphabetic)
            && stripped.as_bytes().get(1) == Some(&b':'))
    {
        stripped.pop();
    }
    stripped
}

fn normalize_key(path: &Path) -> String {
    clean_windows_path(path).to_lowercase()
}

fn is_windows_absolute(cleaned: &str) -> bool {
    let bytes = cleaned.as_bytes();
    if bytes.first().is_some_and(u8::is_ascii_alphabetic)
        && bytes.get(1) == Some(&b':')
        && bytes.get(2) == Some(&b'\\')
    {
        return true;
    }
    cleaned.strip_prefix(r"\\").is_some_and(|rest| {
        let mut parts = rest.split('\\');
        parts.next().is_some_and(|server| !server.is_empty())
            && parts.next().is_some_and(|share| !share.is_empty())
    })
}

fn path_segments(cleaned: &str) -> Vec<&str> {
    let body = if cleaned
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphabetic)
        && cleaned.as_bytes().get(1) == Some(&b':')
        && cleaned.as_bytes().get(2) == Some(&b'\\')
    {
        &cleaned[3..]
    } else {
        cleaned.trim_start_matches('\\')
    };
    body.split('\\').filter(|s| !s.is_empty()).collect()
}

fn disallowed_path_with_temp(path: &Path, temp_dir: Option<&Path>) -> bool {
    let cleaned = clean_windows_path(path);
    if !is_windows_absolute(&cleaned) {
        return true;
    }
    let segments = path_segments(&cleaned);
    if segments.iter().any(|seg| {
        seg.eq_ignore_ascii_case("target")
            || seg.eq_ignore_ascii_case("temp")
            || seg.eq_ignore_ascii_case("windowsapps")
    }) {
        return true;
    }
    if segments.windows(4).any(|w| {
        w[0].eq_ignore_ascii_case("packages")
            && !w[1].is_empty()
            && w[2].eq_ignore_ascii_case("localcache")
            && w[3].eq_ignore_ascii_case("local")
    }) {
        return true;
    }
    if let Some(temp) = temp_dir {
        let cleaned_temp = clean_windows_path(temp);
        if is_windows_absolute(&cleaned_temp) {
            let key = cleaned.to_lowercase();
            let temp_key = cleaned_temp.to_lowercase();
            let base = temp_key.trim_end_matches('\\');
            if key == temp_key || key.starts_with(&format!(r"{base}\")) {
                return true;
            }
        }
    }
    false
}

pub(super) fn disallowed_path(path: &Path) -> bool {
    let temp = std::env::temp_dir();
    disallowed_path_with_temp(path, Some(&temp))
}

fn same_executable_with(
    running: &Path,
    resolved_fixed: &Path,
    temp_dir: Option<&Path>,
    canonicalize: impl Fn(&Path) -> Option<PathBuf>,
) -> bool {
    if disallowed_path_with_temp(running, temp_dir) {
        return false;
    }
    let Some(resolved_running) = canonicalize(running) else {
        return false;
    };
    if disallowed_path_with_temp(&resolved_running, temp_dir) {
        return false;
    }
    normalize_key(&resolved_running) == normalize_key(resolved_fixed)
}

pub(super) fn same_executable(running: &Path, _fixed: &Path, resolved_fixed: &Path) -> bool {
    let temp = std::env::temp_dir();
    same_executable_with(running, resolved_fixed, Some(&temp), |p| {
        std::fs::canonicalize(p).ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashMap, fs};

    #[test]
    fn installed_path_is_allowed_while_target_temp_and_msix_paths_are_rejected() {
        let temp = Path::new(r"C:\Users\Alice\AppData\Local\ScratchTmp");
        for allowed in [
            r"C:\Users\Alice\AppData\Local\Agent HUD\Agent HUD.exe",
            r"\\?\c:/users/alice/AppData/Local/Agent HUD/Agent HUD.exe",
            r"\\server\share\Agent HUD\Agent HUD.exe",
            r"\\?\UNC\server\share\Agent HUD\Agent HUD.exe",
        ] {
            assert!(
                !disallowed_path_with_temp(Path::new(allowed), Some(temp)),
                "must allow {allowed}"
            );
        }

        for rejected in [
            r"Agent HUD.exe",
            r"C:Agent HUD.exe",
            r"\Agent HUD.exe",
            r"/Applications/Agent HUD.app/Contents/MacOS/Agent HUD",
            r"C:\src\agent-hud\target\debug\Agent HUD.exe",
            r"C:\Users\Alice\AppData\Local\Temp\Agent HUD\Agent HUD.exe",
            r"C:\Users\Alice\AppData\Local\ScratchTmp\Agent HUD\Agent HUD.exe",
            r"C:\Program Files\WindowsApps\AgentHUD_1.0\Agent HUD.exe",
            r"C:\Users\Alice\AppData\Local\Packages\AgentHUD_123\LocalCache\Local\Agent HUD\Agent HUD.exe",
        ] {
            assert!(
                disallowed_path_with_temp(Path::new(rejected), Some(temp)),
                "must reject {rejected}"
            );
        }
    }

    #[test]
    fn canonical_path_matching_handles_case_slashes_and_verbatim_prefixes() {
        let temp = Path::new(r"C:\Users\Alice\AppData\Local\Temp");
        let resolved_fixed = Path::new(r"\\?\C:\Users\Alice\AppData\Local\Agent HUD\Agent HUD.exe");
        let running = Path::new(r"c:/users/alice/appdata/local/agent hud/agent hud.exe");
        assert!(same_executable_with(
            running,
            resolved_fixed,
            Some(temp),
            |_| Some(PathBuf::from(
                r"\\?\c:\Users\Alice\AppData\Local\Agent HUD\Agent HUD.exe"
            )),
        ));
        assert!(!same_executable_with(
            Path::new(r"C:\Users\Alice\AppData\Local\Target\Agent HUD.exe"),
            resolved_fixed,
            Some(temp),
            |_| Some(resolved_fixed.to_path_buf()),
        ));
        assert!(!same_executable_with(
            running,
            resolved_fixed,
            Some(temp),
            |_| Some(PathBuf::from(
                r"\\?\C:\Users\Alice\AppData\Local\Packages\AgentHUD_1\LocalCache\Local\Agent HUD\Agent HUD.exe"
            )),
        ));
    }

    #[test]
    fn canonical_rejects_executables_inside_temp_even_when_present_on_disk() {
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join("Agent HUD.exe");
        fs::write(&exe, b"stub").unwrap();
        let context = super::super::InstallContext {
            home: temp.path().to_path_buf(),
            environment: HashMap::new(),
            executable: exe.clone(),
            canonical_executable: Some(exe),
        };
        assert!(super::super::canonical(&context).is_err());
    }
}
