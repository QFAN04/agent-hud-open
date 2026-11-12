//! Antigravity sessions take messages and start new cascades through the
//! running local Antigravity language server (`crate::clients::antigravity::service`),
//! or fall back to the `agy` CLI when starting a session with no language server running.
use super::*;
use cascade_config::resolve_cascade_config;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

mod cascade_config;
// How a message reaches an existing cascade: straight to the language
// server on macOS and Linux; to the service that opened the session on
// Windows, where interrupting is not adapted.
#[cfg(unix)]
#[path = "unix.rs"]
mod transport;
#[cfg(windows)]
#[path = "windows.rs"]
mod transport;
pub(super) use transport::{NOTICE, OPERATIONS};

/// Host injection point for isolated fixtures or the running local language server.
pub trait AntigravityTransport: Send + Sync {
    fn deliver(
        &self,
        target: &SessionTarget,
        request: &SendRequest,
        timeout: Duration,
    ) -> Result<Receipt>;
}

/// Delivers messages to an existing Antigravity cascade through its local language server.
#[derive(Clone, Debug, Default)]
pub struct NativeAntigravityTransport;

impl AntigravityTransport for NativeAntigravityTransport {
    fn deliver(
        &self,
        target: &SessionTarget,
        request: &SendRequest,
        _timeout: Duration,
    ) -> Result<Receipt> {
        let cascade_id = super::antigravity_cascade_id(&target.session_id);
        if !valid_uuid(cascade_id) {
            return Err(Failure::InvalidInput("Antigravity session identity".into()));
        }
        let message_id = format!("agent-hud:{}", request.request_id);
        match request.operation {
            Operation::Interrupt => {
                transport::interrupt(cascade_id)?;
                Ok(Receipt {
                    provider: Provider::Antigravity,
                    session_id: target.session_id.clone(),
                    thread_id: target.thread_id.clone(),
                    request_id: request.request_id.clone(),
                    client_request_id: message_id,
                    turn_id: None,
                    owner_client_id: None,
                    state: DeliveryState::Interrupted,
                    at_ms: millis(SystemTime::now()),
                    reason: None,
                })
            }
            Operation::Steer | Operation::Enqueue | Operation::Start | Operation::Automatic => {
                let (strategy, state) = match request.operation {
                    Operation::Steer => (
                        "MESSAGE_DELIVERY_STRATEGY_NEXT_INVOCATION",
                        DeliveryState::Steered,
                    ),
                    Operation::Enqueue => {
                        ("MESSAGE_DELIVERY_STRATEGY_WHEN_IDLE", DeliveryState::Queued)
                    }
                    Operation::Start | Operation::Automatic => (
                        "MESSAGE_DELIVERY_STRATEGY_UNSPECIFIED",
                        DeliveryState::Started,
                    ),
                    Operation::Interrupt => unreachable!(),
                };
                transport::send(cascade_id, &request.text, strategy)?;
                Ok(Receipt {
                    provider: Provider::Antigravity,
                    session_id: target.session_id.clone(),
                    thread_id: target.thread_id.clone(),
                    request_id: request.request_id.clone(),
                    client_request_id: message_id,
                    turn_id: (state == DeliveryState::Steered)
                        .then(|| request.expected_turn_id.clone())
                        .flatten(),
                    owner_client_id: None,
                    state,
                    at_ms: millis(SystemTime::now()),
                    reason: None,
                })
            }
        }
    }
}

/// Whether Antigravity's desktop or IDE app is installed in one of `applications`.
pub fn desktop_installed(applications: &[PathBuf]) -> bool {
    const MAC_BUNDLES: &[&str] = &["Antigravity.app", "Antigravity IDE.app"];
    const WINDOWS_EXES: &[&str] = &[
        "Antigravity.exe",
        "Antigravity IDE.exe",
        "antigravity/Antigravity.exe",
        "Antigravity/Antigravity.exe",
        "Antigravity IDE/Antigravity.exe",
        "Antigravity IDE/Antigravity IDE.exe",
    ];
    applications.iter().any(|folder| {
        MAC_BUNDLES.iter().any(|app| folder.join(app).is_dir())
            || WINDOWS_EXES.iter().any(|exe| folder.join(exe).is_file())
    })
}

/// Starts a new Antigravity cascade in `folder` through the running language server
/// and sends `request.text` as its first message.
pub fn start_cascade(folder: &Path, request: &StartRequest) -> Result<StartReceipt> {
    let uri = workspace_uri(folder);
    let started = call(
        "StartCascade",
        json!({
            "metadata": metadata(),
            "source": "CORTEX_TRAJECTORY_SOURCE_INTERACTIVE_CASCADE",
            "workspaceUris": [uri],
        }),
    )?;
    let id = started["cascadeId"]
        .as_str()
        .filter(|id| valid_uuid(id))
        .ok_or(Failure::AcceptanceUncertain)?
        .to_owned();
    let sent = call(
        "SendUserCascadeMessage",
        message_body(
            &id,
            &request.text,
            "MESSAGE_DELIVERY_STRATEGY_UNSPECIFIED",
            false,
        ),
    );
    Ok(StartReceipt {
        provider: Provider::Antigravity,
        request_id: request.request_id.clone(),
        session_id: Some(id),
        thread_id: None,
        state: if sent.is_ok() { "started" } else { "uncertain" }.into(),
        reason: sent.err().map(|e| e.to_string()),
        job_id: None,
        handoff_pending: false,
    })
}

fn message_body(cascade_id: &str, text: &str, strategy: &str, existing: bool) -> Value {
    message_body_with_config(
        cascade_id,
        text,
        strategy,
        resolve_cascade_config(existing.then_some(cascade_id)),
    )
}

fn message_body_with_config(
    cascade_id: &str,
    text: &str,
    strategy: &str,
    config: Option<Value>,
) -> Value {
    let mut body = json!({
        "metadata": metadata(),
        "cascadeId": cascade_id,
        "items": [{ "text": text }],
        "deliveryStrategy": strategy,
    });
    if let Some(config) = config {
        body["cascadeConfig"] = config;
    }
    body
}

fn metadata() -> Value {
    json!({
        "ideName": "antigravity",
        "extensionName": "antigravity",
        "locale": "en",
        "ideVersion": "unknown",
    })
}

fn workspace_uri(folder: &Path) -> String {
    workspace_uri_from_str(&folder.to_string_lossy())
}

fn workspace_uri_from_str(raw: &str) -> String {
    let unc = raw
        .strip_prefix(r"\\?\UNC\")
        .or_else(|| raw.strip_prefix("//?/UNC/"))
        .or_else(|| {
            raw.strip_prefix(r"\\")
                .filter(|rest| !rest.starts_with("?\\"))
        })
        .or_else(|| {
            raw.strip_prefix("//")
                .filter(|rest| !rest.starts_with("?/"))
        });
    if let Some(unc) = unc {
        let mut parts = unc.splitn(2, ['\\', '/']);
        let host = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("").replace('\\', "/");
        if !host.is_empty() {
            if let Ok(mut url) = reqwest::Url::parse(&format!("file://{host}/")) {
                url.set_path(&format!("/{rest}"));
                return url.to_string();
            }
        }
    }
    let trimmed = raw
        .strip_prefix(r"\\?\")
        .or_else(|| raw.strip_prefix("//?/"))
        .unwrap_or(raw);
    let forward = trimmed.replace('\\', "/");
    let normalized = if forward.starts_with('/') {
        forward
    } else {
        format!("/{forward}")
    };
    if let Ok(mut url) = reqwest::Url::parse("file:///") {
        url.set_path(&normalized);
        return url.to_string();
    }
    format!("file://{normalized}")
}

fn call(method: &str, body: Value) -> Result<Value> {
    crate::clients::antigravity::service::call(method, &body).map_err(|error| {
        if error.contains("no running Antigravity service") {
            Failure::Unavailable(error)
        } else {
            Failure::Rejected(error)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn desktop_installed_accepts_mac_bundles_and_windows_executables() {
        let temp = tempfile::tempdir().unwrap();
        let apps = temp.path().join("apps");
        let programs = temp.path().join("Programs");
        fs::create_dir_all(apps.join("Antigravity.app")).unwrap(); // plain directory
        fs::create_dir_all(programs.join("antigravity")).unwrap(); // plain directory
        fs::write(programs.join("antigravity/Antigravity.exe"), b"").unwrap();

        assert!(desktop_installed(std::slice::from_ref(&apps)));
        assert!(desktop_installed(std::slice::from_ref(&programs)));
        assert!(desktop_installed(&[programs.join("antigravity")]));
        assert!(!desktop_installed(&[temp.path().join("missing")]));
    }

    #[test]
    fn workspace_uri_formats_unix_windows_drive_verbatim_and_unc_paths() {
        assert_eq!(
            workspace_uri_from_str("/Users/alice/Agent HUD"),
            "file:///Users/alice/Agent%20HUD"
        );
        assert_eq!(
            workspace_uri_from_str(r"C:\Users\alice\Agent HUD"),
            "file:///C:/Users/alice/Agent%20HUD"
        );
        assert_eq!(
            workspace_uri_from_str(r"\\?\C:\Users\alice\Agent HUD"),
            "file:///C:/Users/alice/Agent%20HUD"
        );
        assert_eq!(
            workspace_uri_from_str(r"\\fileserver\share\Agent HUD"),
            "file://fileserver/share/Agent%20HUD"
        );
        assert_eq!(
            workspace_uri_from_str(r"\\?\UNC\fileserver\share\Agent HUD"),
            "file://fileserver/share/Agent%20HUD"
        );
    }
}
