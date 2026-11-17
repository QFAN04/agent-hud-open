use super::*;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

#[cfg(unix)]
#[path = "antigravity_cli_tests_unix.rs"]
mod antigravity_cli;
#[cfg(windows)]
#[path = "antigravity_cli_tests_windows.rs"]
mod antigravity_cli;

fn isolated(root: &Path) -> Runtime {
    fs::create_dir_all(root.join("home")).unwrap();
    Runtime::with_sources(Vec::new(), root.join("app-data")).unwrap()
}

fn non_temp_owner_dir() -> tempfile::TempDir {
    let temp_root = std::env::temp_dir();
    let canonical_temp = std::fs::canonicalize(&temp_root).unwrap_or(temp_root);
    let disallowed = |path: &Path| {
        let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let text = resolved.to_string_lossy().to_lowercase();
        let temp_text = canonical_temp.to_string_lossy().to_lowercase();
        let temp_base = temp_text.trim_end_matches('\\');
        let parts: Vec<String> = resolved
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
            .collect();
        resolved.starts_with(&canonical_temp)
            || (!temp_base.is_empty()
                && (text == temp_base || text.starts_with(&format!(r"{temp_base}\"))))
            || parts
                .iter()
                .any(|s| matches!(s.as_str(), "temp" | "target" | "windowsapps"))
            || parts.windows(4).any(|w| {
                w[0] == "packages" && !w[1].is_empty() && w[2] == "localcache" && w[3] == "local"
            })
    };
    let env_roots: Vec<PathBuf> = ["LOCALAPPDATA", "USERPROFILE"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect();
    if !env_roots.is_empty() {
        for candidate in env_roots
            .into_iter()
            .chain(std::iter::once(PathBuf::from(env!("CARGO_MANIFEST_DIR"))))
        {
            if candidate.is_dir() && !disallowed(&candidate) {
                if let Ok(dir) = tempfile::Builder::new()
                    .prefix("agent-hud-owner-")
                    .tempdir_in(&candidate)
                {
                    return dir;
                }
            }
        }
    }
    tempfile::Builder::new()
        .prefix("agent-hud-owner-")
        .tempdir()
        .unwrap()
}

#[allow(dead_code)] // Used by antigravity_cli_tests_unix.rs.
fn delegate(runtime: &Runtime, root: &Path) {
    delegate_with_executable(runtime, root, root);
}

fn delegate_with_executable(runtime: &Runtime, root: &Path, executable_root: &Path) {
    // A regular fixture file satisfies the fixed-owner identity check. It is
    // deliberately never launched; this tests installation and persistence.
    let executable = executable_root.join("fixed hook owner");
    fs::write(&executable, b"fixture only; never execute").unwrap();
    runtime
        .hooks_handle()
        .lock()
        .unwrap()
        .delegate_owner(hooks::InstallContext {
            home: root.join("home"),
            environment: HashMap::new(),
            canonical_executable: Some(executable.clone()),
            executable,
        })
        .unwrap();
}

fn status(runtime: &mut Runtime) -> Value {
    runtime
        .request("antigravity_cli_status", json!({}))
        .unwrap()
}

fn set_paths(runtime: &mut Runtime, value: Value) -> Result<Value, String> {
    runtime.request(
        "set_setting",
        json!({"key":"antigravityCliSettings","value":value}),
    )
}

fn read(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[test]
fn antigravity_cli_rejects_invalid_lists_and_missing_owners_without_file_changes() {
    let temp = tempfile::tempdir().unwrap();
    let mut runtime = isolated(temp.path());
    let path = temp.path().join("home/profile/settings.json");
    assert_eq!(status(&mut runtime)["installable"], false);
    assert!(status(&mut runtime)["notice"]
        .as_str()
        .is_some_and(|text| !text.is_empty()));
    assert!(set_paths(&mut runtime, json!([path])).is_err());
    assert!(!path.exists());

    let owner = non_temp_owner_dir();
    delegate_with_executable(&runtime, temp.path(), owner.path());
    for invalid in [
        json!(null),
        json!("settings.json"),
        json!([1]),
        json!([""]),
        json!([" /path"]),
        json!(["/path\0invalid"]),
    ] {
        assert!(set_paths(&mut runtime, invalid).is_err());
        assert!(!path.exists());
        assert!(runtime.core.antigravity_cli_settings().is_empty());
        assert!(!temp.path().join("app-data/settings.json").exists());
    }
}
