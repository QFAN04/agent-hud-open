//! Windows host options for installed release builds.

use std::path::{Path, PathBuf};

mod lookup;

/// Host options on Windows: release copies installed into `%LOCALAPPDATA%\Agent HUD`
/// or an NSIS-registered folder (`HKCU\Software\agenthud\Agent HUD` or `Uninstall`
/// `InstallLocation`) claim ownership when paired with engine and MCP companions.
pub(crate) fn options() -> agent_hud_host::HostOptions {
    let exe = std::env::current_exe().ok();
    let dirs = lookup::approved_install_dirs();
    let temp = std::env::temp_dir();
    resolve_options(
        cfg!(debug_assertions),
        exe.as_deref(),
        &dirs,
        Some(&temp),
        |path| std::fs::canonicalize(path).ok(),
        has_packaged_resources,
    )
}

fn has_resource(dir: &Path, file_name: &str) -> bool {
    dir.join("resources").join(file_name).is_file() || dir.join(file_name).is_file()
}

fn has_packaged_resources(dir: &Path) -> bool {
    has_resource(dir, "agent-hud-engine.exe") && has_resource(dir, "agent-hud-mcp.exe")
}

fn strip_verbatim_and_normalize(path: &Path) -> String {
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
    strip_verbatim_and_normalize(path).to_lowercase()
}

fn is_windows_absolute(cleaned: &str) -> bool {
    let bytes = cleaned.as_bytes();
    let is_drive = bytes.first().is_some_and(u8::is_ascii_alphabetic)
        && bytes.get(1) == Some(&b':')
        && bytes.get(2) == Some(&b'\\');
    if is_drive {
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

fn is_disallowed_path(path: &Path, temp_dir: Option<&Path>) -> bool {
    let cleaned = strip_verbatim_and_normalize(path);
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
        let cleaned_temp = strip_verbatim_and_normalize(temp);
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

fn resolve_options(
    is_debug: bool,
    current_exe: Option<&Path>,
    approved_dirs: &[PathBuf],
    temp_dir: Option<&Path>,
    canonicalize: impl Fn(&Path) -> Option<PathBuf>,
    has_resources: impl Fn(&Path) -> bool,
) -> agent_hud_host::HostOptions {
    if is_debug {
        return agent_hud_host::HostOptions::default();
    }
    let Some(exe) = current_exe else {
        return agent_hud_host::HostOptions::default();
    };
    if is_disallowed_path(exe, temp_dir) {
        return agent_hud_host::HostOptions::default();
    }
    let Some(resolved_exe) = canonicalize(exe) else {
        return agent_hud_host::HostOptions::default();
    };
    if is_disallowed_path(&resolved_exe, temp_dir) {
        return agent_hud_host::HostOptions::default();
    }
    let exe_key = normalize_key(&resolved_exe);
    for dir in approved_dirs {
        let expected = dir.join("Agent HUD.exe");
        if is_disallowed_path(&expected, temp_dir) {
            continue;
        }
        let Some(resolved_expected) = canonicalize(&expected) else {
            continue;
        };
        if is_disallowed_path(&resolved_expected, temp_dir)
            || normalize_key(&resolved_expected) != exe_key
        {
            continue;
        }
        let clean_expected = PathBuf::from(strip_verbatim_and_normalize(&resolved_expected));
        if clean_expected.parent().is_some_and(&has_resources) {
            return agent_hud_host::HostOptions {
                canonical_executable: Some(clean_expected),
            };
        }
    }
    agent_hud_host::HostOptions::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn installed_release_copy_in_real_local_appdata_claims_ownership() {
        let approved = vec![PathBuf::from(r"C:\Users\Alice\AppData\Local\Agent HUD")];
        let exe = Path::new(r"c:/users/alice/AppData/Local/Agent HUD/Agent HUD.exe");
        let temp = Path::new(r"C:\Users\Alice\AppData\Local\Temp");
        let options = resolve_options(
            false,
            Some(exe),
            &approved,
            Some(temp),
            |p| {
                let s = strip_verbatim_and_normalize(p);
                Some(PathBuf::from(format!(r"\\?\{s}")))
            },
            |_| true,
        );
        assert_eq!(
            options.canonical_executable,
            Some(PathBuf::from(
                r"C:\Users\Alice\AppData\Local\Agent HUD\Agent HUD.exe"
            ))
        );
    }

    #[test]
    fn custom_nsis_registry_dir_claims_ownership_unless_disallowed_or_incomplete() {
        let parsed = lookup::parse_registry_install_dir(r#"  "D:\Apps\Agent HUD"  "#).unwrap();
        assert_eq!(parsed, PathBuf::from(r"D:\Apps\Agent HUD"));
        assert!(lookup::parse_registry_install_dir(r#" "" "#).is_none());

        let approved = vec![
            PathBuf::from(r"C:\Users\Alice\AppData\Local\Agent HUD"),
            parsed,
            PathBuf::from(r"C:\src\target\release"),
            PathBuf::from(r"C:\Users\Alice\AppData\Local\Temp\Agent HUD"),
            PathBuf::from(r"C:\Program Files\WindowsApps\AgentHUD_1.0"),
            PathBuf::from(
                r"C:\Users\Alice\AppData\Local\Packages\AgentHUD_1\LocalCache\Local\Agent HUD",
            ),
        ];
        let temp = Path::new(r"C:\Users\Alice\AppData\Local\Temp");
        let verbatim = |p: &Path| Some(PathBuf::from(format!(r"\\?\{}", p.display())));

        let custom_exe = Path::new(r"d:\apps\agent hud\Agent HUD.exe");
        assert_eq!(
            resolve_options(
                false,
                Some(custom_exe),
                &approved,
                Some(temp),
                verbatim,
                |_| true
            )
            .canonical_executable,
            Some(PathBuf::from(r"D:\Apps\Agent HUD\Agent HUD.exe"))
        );
        assert!(resolve_options(
            false,
            Some(custom_exe),
            &approved,
            Some(temp),
            verbatim,
            |_| false
        )
        .canonical_executable
        .is_none());
        assert!(resolve_options(
            false,
            Some(Path::new(r"E:\Portable\Agent HUD.exe")),
            &approved,
            Some(temp),
            verbatim,
            |_| true,
        )
        .canonical_executable
        .is_none());
        for bad in [
            r"C:\src\target\release\Agent HUD.exe",
            r"C:\Users\Alice\AppData\Local\Temp\Agent HUD\Agent HUD.exe",
            r"C:\Program Files\WindowsApps\AgentHUD_1.0\Agent HUD.exe",
            r"C:\Users\Alice\AppData\Local\Packages\AgentHUD_1\LocalCache\Local\Agent HUD\Agent HUD.exe",
        ] {
            assert!(
                resolve_options(
                    false,
                    Some(Path::new(bad)),
                    &approved,
                    Some(temp),
                    verbatim,
                    |_| true
                )
                .canonical_executable
                .is_none(),
                "must reject {bad} even if registered"
            );
        }
    }

    #[test]
    fn debug_target_temp_msix_and_incomplete_copies_do_not_claim_ownership() {
        let approved = vec![PathBuf::from(r"C:\Users\Alice\AppData\Local\Agent HUD")];
        let installed = Path::new(r"C:\Users\Alice\AppData\Local\Agent HUD\Agent HUD.exe");
        let temp = Path::new(r"C:\Users\Alice\AppData\Local\ScratchTmp");
        let verbatim = |p: &Path| Some(PathBuf::from(format!(r"\\?\{}", p.display())));

        assert!(resolve_options(
            true,
            Some(installed),
            &approved,
            Some(temp),
            verbatim,
            |_| true
        )
        .canonical_executable
        .is_none());

        assert!(resolve_options(
            false,
            Some(installed),
            &approved,
            Some(temp),
            verbatim,
            |_| false
        )
        .canonical_executable
        .is_none());

        for bad in [
            r"C:\src\agent-hud\target\release\Agent HUD.exe",
            r"C:\Users\Alice\AppData\Local\Temp\Agent HUD\Agent HUD.exe",
            r"C:\Users\Alice\AppData\Local\ScratchTmp\Agent HUD\Agent HUD.exe",
            r"C:\Program Files\WindowsApps\AgentHUD_1.0_x64\Agent HUD.exe",
            r"C:\Users\Alice\AppData\Local\Packages\AgentHUD_123\LocalCache\Local\Agent HUD\Agent HUD.exe",
        ] {
            assert!(
                resolve_options(
                    false,
                    Some(Path::new(bad)),
                    &approved,
                    Some(temp),
                    verbatim,
                    |_| true,
                )
                .canonical_executable
                .is_none(),
                "must reject {bad}"
            );
        }

        // Even if the path looks installed, resolving via symlink into target/ fails.
        assert!(resolve_options(
            false,
            Some(installed),
            &approved,
            Some(temp),
            |_| Some(PathBuf::from(r"\\?\C:\src\target\release\Agent HUD.exe")),
            |_| true,
        )
        .canonical_executable
        .is_none());
    }

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("agent-hud-host-test-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn packaged_resources_accept_nested_resources_and_direct_siblings() {
        let dir = TestDir::new("resources");
        let nested = dir.0.join("nested");
        fs::create_dir_all(nested.join("resources")).unwrap();
        fs::write(nested.join("resources/agent-hud-engine.exe"), b"").unwrap();
        assert!(!has_packaged_resources(&nested));
        fs::write(nested.join("resources/agent-hud-mcp.exe"), b"").unwrap();
        assert!(has_packaged_resources(&nested));

        let flat = dir.0.join("flat");
        fs::create_dir_all(&flat).unwrap();
        fs::write(flat.join("agent-hud-engine.exe"), b"").unwrap();
        fs::write(flat.join("agent-hud-mcp.exe"), b"").unwrap();
        assert!(has_packaged_resources(&flat));
    }
}
