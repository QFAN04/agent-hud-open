//! Codex threads take a message from the desktop app that has them open, from
//! the shared CLI daemon on macOS and Linux, or through Codex's own queue.
use super::*;
use std::path::Path;

mod desktop;
// How a send reaches Codex: the desktop app's socket and the shared CLI
// daemon on macOS and Linux; the desktop app's pipe and the queue on Windows.
#[cfg(unix)]
#[path = "unix/mod.rs"]
mod transport;
#[cfg(windows)]
#[path = "windows.rs"]
mod transport;
pub(super) use transport::{queue_created, start_in_daemon, OPERATIONS, SUPPORTED};

/// Host injection point for isolated fixtures or a shared native state adapter.
pub trait CodexTransport: Send + Sync {
    fn deliver(
        &self,
        data: &Path,
        target: &SessionTarget,
        request: &SendRequest,
        timeout: Duration,
    ) -> Result<Receipt>;
}
/// The installed clients. With `queue` set, an `enqueue` that finds no
/// shared daemon goes into Codex's queue through a short-lived app-server.
#[derive(Clone, Debug, Default)]
pub struct NativeCodexTransport {
    pub queue: Option<QueueRoute>,
}
/// How a message reaches Codex's queue without a shared daemon, and the
/// folders its desktop app is looked for in to wake a thread nobody has open.
#[derive(Clone, Debug)]
pub struct QueueRoute {
    pub executable: std::path::PathBuf,
    pub environment: std::collections::BTreeMap<String, String>,
    pub applications: Vec<std::path::PathBuf>,
}
impl CodexTransport for NativeCodexTransport {
    fn deliver(
        &self,
        data: &Path,
        target: &SessionTarget,
        request: &SendRequest,
        timeout: Duration,
    ) -> Result<Receipt> {
        transport::deliver(data, target, request, timeout, self.queue.as_ref())
    }
}

/// Adds a message to Codex's queue through a short-lived app-server.
fn enqueue_stdio(
    route: &QueueRoute,
    data: &Path,
    thread: &str,
    text: &str,
    message_id: &str,
    timeout: Duration,
) -> Result<()> {
    let mut server =
        process::StdioServer::open(&route.executable, data, &route.environment, timeout)?;
    let value = server.mutation(
        "thread/queue/add",
        serde_json::json!({"threadId":thread,"clientUserMessageId":message_id,
        "input":[{"type":"text","text":text,"text_elements":[]}]}),
    )?;
    server.end();
    if value["queuedSubmission"].is_object() {
        Ok(())
    } else {
        Err(Failure::AcceptanceUncertain)
    }
}

/// Whether an app has the thread open now: Codex holds the thread's writer
/// lock for as long as it has the thread loaded (the desktop app for hours
/// after its last turn). Asked without waiting, and let go at once.
pub fn thread_open(data: &Path, thread: &str) -> bool {
    let Ok(file) = std::fs::File::open(
        data.join("thread-writer-locks")
            .join(format!("{thread}.lock")),
    ) else {
        return false;
    };
    match file.try_lock_shared() {
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(std::fs::TryLockError::WouldBlock) => true,
        Err(std::fs::TryLockError::Error(_)) => false,
    }
}
/// Whether Codex's desktop app is installed in one of `applications`:
/// `Codex.app` or `ChatGPT.app` on macOS, or Windows executables and Store
/// `OpenAI.Codex*` packages under the Windows application roots.
pub fn desktop_installed(applications: &[std::path::PathBuf]) -> bool {
    const MAC_BUNDLES: &[&str] = &["Codex.app", "ChatGPT.app"];
    const WINDOWS_EXES: &[&str] = &[
        "Codex.exe",
        "ChatGPT.exe",
        "chatgpt-desktop.exe",
        "Codex/Codex.exe",
        "OpenAI Codex/Codex.exe",
        "ChatGPT/ChatGPT.exe",
    ];
    applications.iter().any(|folder| {
        MAC_BUNDLES.iter().any(|app| folder.join(app).is_dir())
            || WINDOWS_EXES
                .iter()
                .any(|exe| is_executable_entry(&folder.join(exe)))
            || has_codex_store_package(folder)
            || has_codex_store_package(&folder.join("Packages"))
    })
}

fn is_executable_entry(path: &Path) -> bool {
    // App execution aliases under `%LOCALAPPDATA%\Microsoft\WindowsApps` are
    // reparse points where `is_file()` can fail with access denied; non-dir
    // symlink metadata covers both regular binaries and WindowsApps aliases.
    std::fs::symlink_metadata(path).is_ok_and(|meta| !meta.is_dir())
}

fn has_codex_store_package(root: &Path) -> bool {
    std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(std::result::Result::ok)
        .any(|entry| {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                return false;
            };
            (name == "OpenAI.Codex" || name.starts_with("OpenAI.Codex_")) && entry.path().is_dir()
        })
}

/// Desktop's current-turn selector, including canonical history with empty turns.
pub fn current_desktop_turn(state: &serde_json::Value) -> Result<Option<(String, String)>> {
    use serde_json::Value;
    fn current(turns: Vec<&Value>) -> Result<Option<(String, String)>> {
        for turn in turns.into_iter().rev() {
            let status = turn["status"]
                .as_str()
                .ok_or_else(|| Failure::Protocol("Desktop turn status".into()))?;
            let id = turn
                .get("turnId")
                .filter(|v| !v.is_null())
                .or_else(|| turn.get("id"));
            if turn.get("turnId").is_some_and(|v| !v.is_null())
                || !matches!(status, "completed" | "failed")
            {
                let id = id
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| Failure::Protocol("Desktop current turn identity".into()))?;
                return Ok(Some((id.into(), status.into())));
            }
        }
        Ok(None)
    }
    let Some(history) = state
        .get("turnHistory")
        .filter(|v| v["kind"] == "canonical")
    else {
        let turns = state["turns"]
            .as_array()
            .ok_or_else(|| Failure::Protocol("Desktop turns".into()))?;
        return current(turns.iter().collect());
    };
    let islands = history["history"]["islands"]
        .as_array()
        .ok_or_else(|| Failure::Protocol("Desktop history islands".into()))?;
    let entities = history["history"]["entitiesByKey"]
        .as_object()
        .ok_or_else(|| Failure::Protocol("Desktop history entities".into()))?;
    let turns = |island: &Value| -> Result<Vec<&Value>> {
        island["entries"]
            .as_array()
            .ok_or_else(|| Failure::Protocol("Desktop history entries".into()))?
            .iter()
            .map(|entry| {
                let key = entry["value"]
                    .as_str()
                    .ok_or_else(|| Failure::Protocol("Desktop history key".into()))?;
                entities
                    .get(key)
                    .ok_or_else(|| Failure::Protocol("Desktop history entity".into()))
            })
            .collect()
    };
    if let Some(island) = islands
        .last()
        .filter(|v| v["newerBoundary"]["status"] == "exhausted")
    {
        if let Some(value) = current(turns(island)?)? {
            return Ok(Some(value));
        }
    }
    current(
        islands
            .iter()
            .map(turns)
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn desktop_installed_accepts_mac_bundles_windows_exes_and_store_packages() {
        let temp = tempfile::tempdir().unwrap();
        let mac = temp.path().join("mac");
        let programs = temp.path().join("Programs");
        let windows_apps = temp.path().join("WindowsApps");
        let local_app = temp.path().join("LocalApp");

        fs::create_dir_all(mac.join("Codex.app")).unwrap();
        assert!(desktop_installed(std::slice::from_ref(&mac)));

        fs::create_dir_all(programs.join("OpenAI Codex")).unwrap();
        fs::write(programs.join("OpenAI Codex/Codex.exe"), b"").unwrap();
        assert!(desktop_installed(std::slice::from_ref(&programs)));

        fs::create_dir_all(windows_apps.join("OpenAI.Codex_2p2nqsd0c76g0")).unwrap();
        assert!(desktop_installed(std::slice::from_ref(&windows_apps)));

        fs::create_dir_all(local_app.join("Packages/OpenAI.Codex_2p2nqsd0c76g0")).unwrap();
        assert!(desktop_installed(std::slice::from_ref(&local_app)));

        let chatgpt = temp.path().join("chatgpt");
        fs::create_dir_all(&chatgpt).unwrap();
        fs::write(chatgpt.join("ChatGPT.exe"), b"").unwrap();
        assert!(desktop_installed(std::slice::from_ref(&chatgpt)));

        assert!(!desktop_installed(&[temp.path().join("missing")]));
    }
}
