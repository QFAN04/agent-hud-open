//! Only typed client-owned destinations become fixed native URL schemes. No
//! command, arbitrary URL or callback-declared host is accepted as navigation.
use crate::hooks::origins::is_uuid;
use agent_hud_agents::{Discovery, Os, ALL};
use serde_json::{json, Value};
mod antigravity;
#[cfg(windows)]
#[path = "navigation/antigravity_windows.rs"]
mod antigravity_host;
#[cfg(not(windows))]
#[path = "navigation/antigravity_unix.rs"]
mod antigravity_host;
mod claude_job;
pub(super) fn open_antigravity(
    home: &std::path::Path,
    id: &str,
    cwd: Option<&str>,
) -> Result<bool, String> {
    antigravity_host::open(home, id, cwd)
}
/// A Claude Code background job moves into the desktop app; false when this
/// host cannot, so the caller falls back.
pub(super) fn open_claude_job(home: &std::path::Path, target: &Value) -> Result<bool, String> {
    claude_job::open(
        home,
        target["id"].as_str().unwrap_or(""),
        target["job"].as_str().unwrap_or(""),
    )
}
pub(super) use crate::os::{activate_app, app_running};
fn resolve_claude_desktop_local_id(id: &str, cowork: bool) -> Option<String> {
    let home = crate::os::home_dir().ok()?;
    let folder = if cowork {
        "local-agent-mode-sessions"
    } else {
        "claude-code-sessions"
    };
    let env = std::env::vars().collect::<crate::os::Environment>();
    let mut matched = Vec::new();
    for org in crate::os::claude_desktop_bases(&home, &env)
        .into_iter()
        .filter_map(|base| std::fs::read_dir(base.join(folder)).ok())
        .flat_map(|entries| entries.flatten())
    {
        let Ok(acct_dir) = std::fs::read_dir(org.path()) else {
            continue;
        };
        for acct in acct_dir.flatten() {
            let Ok(files) = std::fs::read_dir(acct.path()) else {
                continue;
            };
            for file in files.flatten() {
                let path = file.path();
                if path.extension().and_then(|s| s.to_str()) != Some("json") {
                    continue;
                }
                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                if !stem.strip_prefix("local_").is_some_and(is_uuid) {
                    continue;
                }
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                let Ok(meta) = serde_json::from_slice::<Value>(&bytes) else {
                    continue;
                };
                if meta["cliSessionId"].as_str() == Some(id)
                    && meta["sessionId"].as_str() == Some(stem)
                    && meta["isArchived"] != true
                {
                    matched.push(stem.to_owned());
                }
            }
        }
    }
    matched.sort();
    matched.dedup();
    (matched.len() == 1).then(|| matched.pop()).flatten()
}
/// Token-shaped Claude Projects identifiers accepted by the project deep
/// link. Keep project and thread identifiers separate from local session IDs.
fn claude_project_id<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let suffix = value.strip_prefix(prefix)?;
    (suffix.len() >= 6
        && suffix.len() <= 120
        && suffix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_'))
    .then_some(value)
}
fn is_absolute_workspace_path(path: &str) -> bool {
    if path.starts_with('/') {
        return true;
    }
    if let Some(rest) = path.strip_prefix(r"\\") {
        return rest.chars().any(|c| c != '\\');
    }
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

fn cursor_file_url(cwd: &str) -> Option<String> {
    let mut url = reqwest::Url::parse("cursor://file").ok()?;
    let path = if cwd.starts_with('/') {
        cwd.to_owned()
    } else if let Some(unc) = cwd.strip_prefix(r"\\?\UNC\") {
        format!("//{}", unc.replace('\\', "/"))
    } else if let Some(local) = cwd.strip_prefix(r"\\?\") {
        let forward = local.replace('\\', "/");
        if forward.starts_with('/') {
            forward
        } else {
            format!("/{forward}")
        }
    } else if let Some(unc) = cwd.strip_prefix(r"\\") {
        format!("//{}", unc.replace('\\', "/"))
    } else {
        format!("/{}", cwd.replace('\\', "/"))
    };
    url.set_path(&path);
    Some(url.into())
}

/// The folder of a local Cursor session (one with no cloud `bc-` id), which
/// opens in Cursor itself where the host can; its `cursor://file` link is
/// the fallback, and Cursor asks before it follows one.
pub(super) fn cursor_folder(target: &Value) -> Option<&std::path::Path> {
    if !matches!(
        target["kind"].as_str(),
        Some("cursor-agent" | "cursorAgent")
    ) || target["id"]
        .as_str()
        .is_some_and(|id| id.starts_with("bc-"))
    {
        return None;
    }
    let cwd = target["cwd"].as_str()?.trim();
    is_absolute_workspace_path(cwd).then(|| std::path::Path::new(cwd))
}

pub fn session_url(target: &Value) -> Option<String> {
    session_url_on(target, crate::os::OS, crate::os::registered_url)
}
pub(super) fn app_target(discovery: &Discovery) -> Option<Value> {
    let target = json!({"kind":discovery.app_navigation?.kind});
    session_url(&target).map(|_| target)
}
fn session_url_on(target: &Value, os: Os, registered: impl Fn(&str) -> bool) -> Option<String> {
    let kind = target["kind"].as_str()?;
    if let Some(navigation) = ALL
        .iter()
        .filter_map(|agent| agent.discovery.app_navigation)
        .find(|navigation| navigation.kind == kind)
    {
        let url = navigation.url.on(os).value()?;
        return registered(url).then(|| (*url).to_owned());
    }
    let id = target["id"].as_str()?;
    match kind {
        "claude-project-thread" if matches!(os, Os::MacOs | Os::Windows) => {
            let project = claude_project_id(target["projectId"].as_str()?, "chan_")?;
            let thread = claude_project_id(id, "cmsg_")?;
            (!thread.starts_with("cmsg_pending_"))
                .then(|| format!("claude://claude.ai/code/project/{project}?thread={thread}"))
        }
        // As Open does, the link also launches the Codex app when it is closed.
        "codex-thread" | "codexThread"
            if is_uuid(id)
                && (app_running("com.openai.codex")
                    || crate::os::registered_bundle("com.openai.codex")) =>
        {
            Some(format!("codex://threads/{id}"))
        }
        "claude-desktop"
        | "claude-desktop-session"
        | "claudeDesktopSession"
        | "claude-cowork"
        | "claude-cowork-session"
        | "claudeCoworkSession" => {
            let cowork = matches!(
                target["kind"].as_str(),
                Some("claude-cowork" | "claude-cowork-session" | "claudeCoworkSession")
            );
            let local_id = if id.strip_prefix("local_").is_some_and(is_uuid) {
                Some(id.to_owned())
            } else if is_uuid(id) {
                resolve_claude_desktop_local_id(id, cowork)
            } else {
                None
            }?;
            let section = if cowork { "cowork" } else { "epitaxy" };
            Some(format!("claude://claude.ai/{section}/{local_id}"))
        }
        "iterm-session" | "iTermSession" if !id.trim().is_empty() && id.len() <= 4096 => {
            let mut url = reqwest::Url::parse("iterm2:reveal").ok()?;
            url.query_pairs_mut().append_pair("sessionid", id);
            Some(url.into())
        }
        "grok-bot-agent" | "grokBotAgent"
            if !id.is_empty()
                && id.len() <= 128
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')) =>
        {
            let mut url = reqwest::Url::parse("grokbot://app/v1/agent").ok()?;
            url.query_pairs_mut().append_pair("id", id);
            Some(url.into())
        }
        // A cloud agent opens in Cursor's Agents window by its `bc-` id. Cursor
        // has no link for a local session (its composers open only through
        // internal commands), so that one opens its folder in Cursor.
        "cursor-agent" | "cursorAgent" => {
            if id.len() <= 128
                && id.starts_with("bc-")
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            {
                let mut url =
                    reqwest::Url::parse("cursor://anysphere.cursor-deeplink/agent").ok()?;
                url.query_pairs_mut().append_pair("id", id);
                return Some(url.into());
            }
            let cwd = target["cwd"].as_str()?.trim();
            if !is_absolute_workspace_path(cwd) {
                return None;
            }
            cursor_file_url(cwd)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
