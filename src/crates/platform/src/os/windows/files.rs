//! Owner-only files on Windows: the owner SID and a protected per-user DACL.
use super::acl;
use std::{
    fs,
    io::{self, Read},
    path::Path,
};

/// A directory only this user can open, created when absent. A reparse point
/// or another user's directory is refused.
pub(crate) fn private_directory(path: &Path) -> Result<(), String> {
    acl::private_directory(path)
}

pub(crate) fn owned_by_user(path: &Path, _metadata: &fs::Metadata) -> bool {
    acl::owned_path(path)
}

/// A client's transcript this user may read: owned by this user, or by the
/// owner Windows gives what this user's elevated programs create (the
/// Administrators group), as an elevated Claude Code's transcripts are. Only
/// reads of transcripts accept that; the app's own folders never do.
pub(crate) fn user_transcript(path: &Path, metadata: &fs::Metadata) -> bool {
    owned_by_user(path, metadata) || acl::owned_by_default_owner(path)
}

/// Files inherit the private directory's protected DACL, so ownership is the check.
pub(crate) fn owner_only(path: &Path, metadata: &fs::Metadata) -> bool {
    owned_by_user(path, metadata)
}

/// New files have an explicit current-user owner even in elevated processes.
pub(crate) fn private_file(path: &Path, create_new: bool) -> io::Result<fs::File> {
    acl::private_file(path, create_new)
}

/// A private file opened at its end for appending. No hook that appends
/// (Cursor's) is launched on Windows yet, so writers do not overlap here.
pub(crate) fn private_append(path: &Path) -> io::Result<fs::File> {
    use std::io::{Seek, SeekFrom};
    let mut file = acl::private_file(path, false)?;
    file.seek(SeekFrom::End(0))?;
    Ok(file)
}

/// A hook payload from stdin, or Err after five seconds or past `limit` bytes.
/// Windows console and pipe handles cannot be polled, so a thread reads.
///
/// A payload is one JSON object, and clients on Windows (Claude Code among
/// them) may write it without closing stdin, so the payload is taken as soon
/// as it is a complete JSON value instead of waiting out the five seconds.
pub(crate) fn read_stdin(bytes: &mut Vec<u8>, limit: usize) -> Result<(), ()> {
    use std::time::{Duration, Instant};
    let (sender, receiver) = std::sync::mpsc::channel::<Option<Vec<u8>>>();
    std::thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            match stdin.read(&mut chunk) {
                Ok(0) | Err(_) => {
                    let _ = sender.send(None);
                    return;
                }
                Ok(count) => {
                    if sender.send(Some(chunk[..count].to_vec())).is_err() {
                        return;
                    }
                }
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut input = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(left) {
            Ok(Some(chunk)) => {
                input.extend_from_slice(&chunk);
                if input.len() > limit {
                    return Err(());
                }
                if complete_json(&input) {
                    break;
                }
            }
            // End of input: whatever arrived is the payload.
            Ok(None) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return Err(()),
        }
    }
    *bytes = input;
    Ok(())
}

/// Whether the bytes are one whole JSON value, surrounding whitespace aside.
fn complete_json(bytes: &[u8]) -> bool {
    serde_json::from_slice::<serde::de::IgnoredAny>(bytes).is_ok()
}

fn executable_str(executable: Option<&Path>) -> Result<Option<&str>, String> {
    executable
        .map(|path| {
            path.to_str()
                .ok_or_else(|| "The hook executable path is not valid UTF-8".to_owned())
        })
        .transpose()
}

/// Format and patch one JSON hook configuration using the client's Windows
/// launch contract, or remove Agent HUD's handlers when `executable` is `None`.
pub(crate) fn hook_configuration(
    source: &str,
    flag: &str,
    current: &serde_json::Value,
    executable: Option<&Path>,
) -> Result<serde_json::Value, String> {
    let executable = executable_str(executable)?;
    agent_hud_agents::patch_configuration_for_windows(source, flag, current, executable)
}

/// Format and patch one text-based hook configuration using the client's
/// Windows launch contract, or remove Agent HUD's handlers when `executable` is `None`.
pub(crate) fn hook_text(
    source: &str,
    flag: &str,
    original: &str,
    executable: Option<&Path>,
) -> Result<String, String> {
    let command = executable_str(executable)?
        .map(|path| agent_hud_agents::hooks::windows_hook_command(source, flag, path))
        .transpose()?;
    agent_hud_agents::patch_text(source, flag, original, command.as_deref())
}

#[cfg(test)]
mod tests {
    use super::{acl, complete_json, hook_text, owned_by_user, user_transcript};
    use std::path::Path;

    /// A file this process writes is owned by the user, or, when elevated, by
    /// the Administrators group: a transcript read accepts both, the app's own
    /// ownership check only the user.
    #[test]
    fn a_transcript_this_process_writes_is_readable_whether_or_not_it_is_elevated() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.jsonl");
        std::fs::write(&path, b"{}\n").unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        assert!(user_transcript(&path, &metadata));
        assert!(owned_by_user(&path, &metadata) || acl::owned_by_default_owner(&path));
        assert!(!user_transcript(&temp.path().join("missing"), &metadata));
    }

    #[test]
    fn a_payload_is_taken_once_its_json_is_whole() {
        assert!(complete_json(b"{\"a\":{\"b\":[1,2]}}\r\n"));
        assert!(!complete_json(b"{\"a\":{\"b\":[1,2]"));
        assert!(!complete_json(b""));
        assert!(!complete_json(b"{\"text\":\"a } inside\""));
    }

    #[test]
    fn windows_text_hooks_install_update_and_remove_for_hermes_and_kimi() {
        let exe = Path::new(r"C:\Program Files\Agent HUD\Agent HUD.exe");
        let moved = Path::new(r"C:\Users\O'Brien\Agent HUD.exe");

        let hermes_orig = "model: deepseek-chat\nhooks:\n  on_session_end:\n    - command: notify\n      timeout: 5\n";
        let hermes_added =
            hook_text("hermes", "--completion-hook", hermes_orig, Some(exe)).unwrap();
        assert!(hermes_added.contains("--completion-hook hermes"));
        assert!(hermes_added.contains("Agent HUD.exe"));
        assert_eq!(
            hook_text("hermes", "--completion-hook", &hermes_added, Some(exe)).unwrap(),
            hermes_added
        );
        let hermes_moved =
            hook_text("hermes", "--completion-hook", &hermes_added, Some(moved)).unwrap();
        assert!(hermes_moved.contains("O'Brien"));
        assert!(!hermes_moved.contains("Program Files"));
        assert_eq!(
            hook_text("hermes", "--completion-hook", &hermes_moved, None).unwrap(),
            hermes_orig
        );

        let kimi_orig = "default_model = \"kimi-code/kimi-for-coding\"\n\n[[hooks]]\nevent = \"Stop\"\ncommand = \"notify done\"\n";
        let kimi_added = hook_text("kimi", "--completion-hook", kimi_orig, Some(exe)).unwrap();
        assert!(kimi_added.contains("--completion-hook kimi"));
        assert!(kimi_added.contains("Agent HUD.exe"));
        assert_eq!(
            hook_text("kimi", "--completion-hook", &kimi_added, Some(exe)).unwrap(),
            kimi_added
        );
        let kimi_moved = hook_text("kimi", "--completion-hook", &kimi_added, Some(moved)).unwrap();
        assert!(kimi_moved.contains("O'Brien"));
        assert!(!kimi_moved.contains("Program Files"));
        let kimi_removed = hook_text("kimi", "--completion-hook", &kimi_moved, None).unwrap();
        assert!(!kimi_removed.contains("--completion-hook"));
        assert!(kimi_removed.contains("notify done"));

        assert!(hook_text("goose", "--completion-hook", "", Some(exe)).is_err());
    }
}
