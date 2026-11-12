//! Starting new sessions from the phone, the HUD or an MCP caller: the
//! folders each client offers, one start per request identity, and moving a
//! Claude Code background job into the desktop app once its turn stopped.
use super::process::{NativeProcessRunner, ProcessRunner};
use super::*;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

mod clients;
mod commands;
mod handoff;
mod offers;
mod start;
mod trust;
use commands::environment;
pub use handoff::ClaudeMove;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartConfig {
    pub home: PathBuf,
    pub claude_config: PathBuf,
    pub codex_data: PathBuf,
    pub claude_executable: Option<PathBuf>,
    pub codex_executable: Option<PathBuf>,
    #[serde(default)]
    pub antigravity_executable: Option<PathBuf>,
    /// Not read by starts, which look for the desktop apps on disk at every
    /// start; kept so a saved configuration keeps its shape.
    #[serde(default)]
    pub claude_desktop: bool,
    #[serde(default)]
    pub codex_desktop: bool,
    pub task_root: PathBuf,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
}
impl StartConfig {
    /// How a message reaches Codex's queue when no shared daemon runs: the
    /// Codex this computer starts sessions with, in the environment it starts
    /// them in.
    pub fn codex_queue(&self) -> Option<QueueRoute> {
        Some(QueueRoute {
            executable: self.codex_executable.clone()?,
            environment: environment(self),
            applications: self.applications(),
        })
    }
    /// Where desktop apps are installed: `/Applications` and `~/Applications`
    /// on macOS; empty where they are not looked for yet (Linux, Windows).
    pub fn applications(&self) -> Vec<PathBuf> {
        host::applications(&self.home)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRequest {
    pub provider: Provider,
    pub request_id: String,
    pub text: String,
    /// None is not an offered independent task; empty string explicitly selects it.
    pub folder: String,
    /// The new Codex thread's name; its first line of text when not given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartOffer {
    pub provider: Provider,
    pub app: String,
    pub folders: Vec<String>,
    pub without_folder: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartReceipt {
    pub provider: Provider,
    pub request_id: String,
    pub session_id: Option<String>,
    pub thread_id: Option<String>,
    pub state: String,
    pub reason: Option<String>,
    pub job_id: Option<String>,
    pub handoff_pending: bool,
}

/// Whether a started Claude job can move into the desktop app now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HandoffCheck {
    /// Its turn still runs, or nothing says it stopped yet; look again later.
    Wait,
    /// Its turn stopped and nothing else holds the session.
    Ready,
    /// It will never move: the reason is kept on its receipt.
    Drop(String),
}

pub struct SessionStarts {
    config: StartConfig,
    receipts: PathBuf,
    offers: Vec<StartOffer>,
    runner: Arc<dyn ProcessRunner>,
    /// Where desktop apps are installed: `/Applications` and `~/Applications`.
    applications: Vec<PathBuf>,
}
impl SessionStarts {
    pub fn new(config: StartConfig, receipts: PathBuf) -> Result<Self> {
        Self::with_runner(config, receipts, Arc::new(NativeProcessRunner))
    }
    pub fn with_runner(
        config: StartConfig,
        receipts: PathBuf,
        runner: Arc<dyn ProcessRunner>,
    ) -> Result<Self> {
        if !config.home.is_absolute()
            || !config.claude_config.is_absolute()
            || !config.codex_data.is_absolute()
            || !config.task_root.is_absolute()
            || !receipts.is_absolute()
        {
            return Err(Failure::InvalidInput(
                "Explicit start paths must be absolute".into(),
            ));
        }
        private_dir(&receipts)?;
        let applications = config.applications();
        Ok(Self {
            config,
            receipts,
            offers: Vec::new(),
            runner,
            applications,
        })
    }
    /// Tests look for desktop apps in fixture folders instead of this computer's.
    #[cfg(any(test, feature = "fixtures"))]
    pub fn set_application_folders(&mut self, folders: Vec<PathBuf>) {
        self.applications = folders;
    }
    fn executable(&self, provider: Provider) -> Option<&Path> {
        match provider {
            Provider::Claude => self.config.claude_executable.as_deref(),
            Provider::Codex => self.config.codex_executable.as_deref(),
            Provider::Antigravity => self.config.antigravity_executable.as_deref(),
            Provider::DeepSeek
            | Provider::Cursor
            | Provider::Trae
            | Provider::WorkBuddy
            | Provider::ZCode
            | Provider::Qoder
            | Provider::Kimi
            | Provider::Pi
            | Provider::OpenCode => None,
        }
    }
    /// Whether the client's desktop app is installed, looked up on disk each
    /// time: `.app` bundles on macOS or desktop executables / Store packages
    /// on Windows inside `self.applications`.
    fn desktop(&self, provider: Provider) -> bool {
        match provider {
            Provider::Antigravity => antigravity_desktop_installed(&self.applications),
            Provider::Claude => has_desktop_app(
                &self.applications,
                &["Claude.app"],
                &[
                    "Claude.exe",
                    "claude.exe",
                    "claude-desktop.exe",
                    "AnthropicClaude/Claude.exe",
                    "AnthropicClaude/claude.exe",
                    "Claude/Claude.exe",
                    "Claude/claude.exe",
                ],
                &["Claude_"],
            ),
            Provider::Codex => has_desktop_app(
                &self.applications,
                &["Codex.app", "ChatGPT.app"],
                &[
                    "Codex.exe",
                    "ChatGPT.exe",
                    "chatgpt-desktop.exe",
                    "Codex/Codex.exe",
                    "OpenAI Codex/Codex.exe",
                    "ChatGPT/ChatGPT.exe",
                ],
                &["OpenAI.Codex_", "OpenAI.ChatGPT"],
            ),
            Provider::DeepSeek
            | Provider::Cursor
            | Provider::Trae
            | Provider::WorkBuddy
            | Provider::ZCode
            | Provider::Qoder
            | Provider::Kimi
            | Provider::Pi
            | Provider::OpenCode => false,
        }
    }
}

fn has_desktop_app(
    applications: &[PathBuf],
    bundles: &[&str],
    exes: &[&str],
    store_prefixes: &[&str],
) -> bool {
    applications.iter().any(|folder| {
        bundles.iter().any(|name| folder.join(name).is_dir())
            || exes
                .iter()
                .any(|exe| is_executable_entry(&folder.join(exe)))
            || (!store_prefixes.is_empty() && has_store_package(folder, store_prefixes))
    })
}

fn is_executable_entry(path: &Path) -> bool {
    // App execution aliases under `%LOCALAPPDATA%\Microsoft\WindowsApps` are
    // reparse points where `is_file()` can fail; non-dir symlink metadata covers
    // both regular binaries and WindowsApps aliases.
    fs::symlink_metadata(path).is_ok_and(|meta| !meta.is_dir())
}

fn has_store_package(folder: &Path, prefixes: &[&str]) -> bool {
    has_store_package_in(folder, prefixes)
        || has_store_package_in(&folder.join("Packages"), prefixes)
}

fn has_store_package_in(root: &Path, prefixes: &[&str]) -> bool {
    let Ok(entries) = fs::read_dir(root) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        if prefixes
            .iter()
            .any(|prefix| lower.starts_with(&prefix.to_ascii_lowercase()))
        {
            return true;
        }
    }
    false
}

/// A JSON file under 1 MiB, or null when it is missing, larger or not JSON.
fn read_json(path: &Path) -> Value {
    fs::read(path)
        .ok()
        .filter(|bytes| bytes.len() < 1024 * 1024)
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null)
}

#[cfg(test)]
mod desktop_tests {
    use super::*;

    fn test_starts(apps: Vec<PathBuf>) -> SessionStarts {
        let root = apps[0].parent().unwrap().to_path_buf();
        let home = root.join("home");
        let receipts = root.join("receipts");
        fs::create_dir_all(&home).unwrap(); // plain directory
        let config = StartConfig {
            home: home.clone(),
            claude_config: home.join(".claude"),
            codex_data: home.join(".codex"),
            claude_executable: None,
            codex_executable: None,
            antigravity_executable: None,
            claude_desktop: false,
            codex_desktop: false,
            task_root: home.join("tasks"),
            environment: BTreeMap::new(),
        };
        let mut starts = SessionStarts::new(config, receipts).unwrap();
        starts.set_application_folders(apps);
        starts
    }

    #[test]
    fn desktop_detects_windows_executables_and_store_packages_alongside_mac_bundles() {
        let temp = tempfile::tempdir().unwrap();
        let apps = temp.path().join("Apps");
        let win_apps = temp.path().join("WindowsApps");
        fs::create_dir_all(&apps).unwrap(); // plain directory
        fs::create_dir_all(&win_apps).unwrap(); // plain directory

        let starts = test_starts(vec![apps.clone(), win_apps.clone()]);
        assert!(!starts.desktop(Provider::Claude));
        assert!(!starts.desktop(Provider::Codex));
        assert!(!starts.desktop(Provider::Antigravity));

        fs::create_dir_all(apps.join("AnthropicClaude")).unwrap(); // plain directory
        fs::write(apps.join("AnthropicClaude/claude.exe"), b"").unwrap();
        fs::create_dir_all(win_apps.join("OpenAI.Codex_2p2nqsd0c76g0")).unwrap(); // plain directory
        fs::create_dir_all(apps.join("antigravity")).unwrap(); // plain directory
        fs::write(apps.join("antigravity/Antigravity.exe"), b"").unwrap();

        assert!(starts.desktop(Provider::Claude));
        assert!(starts.desktop(Provider::Codex));
        assert!(starts.desktop(Provider::Antigravity));

        fs::remove_file(apps.join("AnthropicClaude/claude.exe")).unwrap();
        assert!(!starts.desktop(Provider::Claude));
        fs::create_dir_all(win_apps.join("Claude_pzs8sxrjxfjjc")).unwrap(); // plain directory
        assert!(starts.desktop(Provider::Claude));

        // Store packages located under `%LOCALAPPDATA%\Packages` are also found.
        let local_app = temp.path().join("LocalApp");
        fs::create_dir_all(local_app.join("Packages/Claude_abc")).unwrap(); // plain directory
        fs::create_dir_all(local_app.join("Packages/OpenAI.Codex_abc")).unwrap(); // plain directory
        let pkg_starts = test_starts(vec![local_app]);
        assert!(pkg_starts.desktop(Provider::Claude));
        assert!(pkg_starts.desktop(Provider::Codex));
    }
}

#[cfg(test)]
mod tests;
