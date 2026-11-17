//! The Codex shared CLI daemon (`app-server-control.sock` and
//! `CODEX_APP_SERVER_USE_LOCAL_DAEMON`) is disabled on Windows in Codex Desktop;
//! Windows only exposes the desktop `\\.\pipe\codex-ipc` endpoint, so a CLI TUI
//! has no daemon-served port to look up here.
use std::{collections::HashMap, path::Path};

pub(crate) fn daemon_tui_port_for(
    _thread_id: &str,
    _home: &Path,
    _env: &HashMap<String, String>,
) -> Option<u16> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_has_no_shared_cli_daemon_port() {
        assert_eq!(
            daemon_tui_port_for(
                "thread-1",
                Path::new(r"C:\Users\demo\.codex"),
                &HashMap::new()
            ),
            None
        );
    }
}
