use super::{variable, Environment, ProcessInfo};
mod acl;
mod antigravity;
mod app_targets;
mod apps;
mod associations;
mod codex;
mod cookies;
mod dpapi;
#[cfg(test)]
mod dpapi_tests;
mod engines;
#[cfg(test)]
mod engines_tests;
mod files;
pub(crate) mod pipe;
mod process;
mod process_ancestry;
mod process_net;
mod process_win32;
mod secrets;
mod shims;
mod shortcut;
pub(crate) use apps::{activate_app, app_running, launch_app};
pub(crate) use associations::{
    open_folder_in_app, open_session_url, open_url_in_app, registered_bundle, registered_url,
};
pub(crate) use engines::{
    application_installed, claude_desktop_bases, discovered_executable, engine_candidates,
    engine_executable, engine_path, executable_dirs,
};
pub(crate) use process::{
    codex_cli_on_port, parent_processes, process_runs, process_start, terminal_io,
};
/// No application bundles here: an MCP call is matched by its own parent.
pub(crate) fn app_ancestors(_pid: u32) -> Vec<u32> {
    Vec::new()
}
pub(crate) use secrets::{app_cookie, stored_secret, unseal_safe_storage};
pub(crate) const READS_PROCESS_ENVIRONMENT: bool = false;
/// Moves Claude Code background jobs into Claude Desktop via `claude stop` and `claude://resume`.
pub(crate) const MOVES_CLAUDE_JOBS: bool = true;
/// Claude Desktop Project threads open via `claude://claude.ai/code/project/...`.
pub(crate) const OPENS_CLAUDE_PROJECTS: bool = true;
pub(crate) use files::{
    hook_configuration, hook_text, owned_by_user, owner_only, private_append, private_directory,
    private_file, read_stdin, user_transcript,
};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
pub(crate) const VALIDATION: &str = "unverified";
pub(crate) const PORTABLE_VALIDATION: &str = "unverified";
/// The operating system whose facts the agents' descriptors give here.
pub(crate) const OS: agent_hud_agents::Os = agent_hud_agents::Os::Windows;
pub(crate) const LEGACY_MAC_HISTORY: bool = false;
pub(crate) fn launched_executable() -> std::io::Result<PathBuf> {
    std::env::current_exe()
}
pub(crate) fn home_dir() -> Result<PathBuf, String> {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .ok_or_else(|| "User home directory unavailable".into())
}
/// The folder Agent HUD's data folder is in, and that folder's name.
pub(crate) fn data_parent(home: &Path) -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("AppData/Local"))
}
pub(crate) const DATA_FOLDER: &str = crate::data_folder::WINDOWS;
pub(crate) fn caches_dir(home: &Path) -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("AppData/Local"))
}
pub(crate) fn app_support(home: &Path, env: &Environment) -> PathBuf {
    variable(env, "APPDATA").unwrap_or_else(|| home.join("AppData/Roaming"))
}
/// Windows environment names are case-insensitive even when captured in a map.
pub(crate) fn environment_value<'a>(env: &'a Environment, key: &str) -> Option<&'a str> {
    env.iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .map(|(_, value)| value.as_str())
}
pub(super) fn local_app(home: &Path, env: &Environment) -> PathBuf {
    variable(env, "LOCALAPPDATA").unwrap_or_else(|| home.join("AppData/Local"))
}

pub(super) fn current_env() -> Environment {
    std::env::vars().collect()
}
pub(crate) fn configure_command(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x08000000);
}
pub(crate) fn is_shell(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            [
                "cmd.exe",
                "powershell.exe",
                "pwsh.exe",
                "bash.exe",
                "sh.exe",
            ]
            .iter()
            .any(|shell| name.eq_ignore_ascii_case(shell))
        })
}
pub(crate) fn reveal_transcript(path: &Path) -> Result<(), String> {
    // Preserve literal argv. PowerShell -Command reparses trailing text, so this
    // one native Explorer operation intentionally stays a direct process call.
    let mut command = Command::new("explorer.exe");
    configure_command(&mut command);
    command
        .arg(format!("/select,{}", path.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|_| "Could not reveal transcript".into())
}
pub(crate) fn legacy_quota_database(_home: &Path) -> Option<PathBuf> {
    None
}
pub(crate) fn processes() -> Result<Vec<ProcessInfo>, String> {
    antigravity::processes()
}
pub(crate) fn listening_ports(pid: u32) -> Result<Vec<u16>, String> {
    antigravity::listening_ports(pid)
}
pub(crate) fn source_roots(home: &Path) -> Vec<agent_hud_core::SourceRoot> {
    super::portable_cli_roots(home)
}
/// Swift UserDefaults has no native source on this host.
pub(crate) fn legacy_preferences() -> Option<serde_json::Value> {
    None
}

#[cfg(test)]
mod tests {
    use super::super::path_entries;
    use super::*;

    #[test]
    fn captured_windows_environment_keeps_os_name_semantics() {
        let env = Environment::from([
            ("AppData".into(), "D:\\profile\\roaming".into()),
            ("Path".into(), "C:\\tools;D:\\bin".into()),
        ]);
        assert_eq!(
            app_support(Path::new("C:\\Users\\me"), &env),
            PathBuf::from("D:\\profile\\roaming")
        );
        assert_eq!(
            path_entries(&env),
            vec![PathBuf::from("C:\\tools"), PathBuf::from("D:\\bin")]
        );
    }
}
