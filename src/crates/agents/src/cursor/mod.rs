//! Cursor: sessions from its own database, tokens from its `stop` hook.
pub mod account;
mod hooks;
mod periods;
pub mod plans;
pub mod sessions;
mod sign_in;

use crate::{Agent, Base, Discovery, Location, PerOs, Source, Support};

/// Cursor's session headers, in its state database.
const HEADERS: Source = Source {
    reads_as: "cursor",
    location: account::STATE,
};
/// The turn log the `stop` hook appends in Agent HUD's own data folder
/// (`hooks::turn_log`): `folder`, named after the app's identifier on that
/// system, or the one `AGENTHUD_DATA_DIR` names. Cursor keeps a turn's tokens
/// nowhere else on disk.
const fn turn_log(base: Base, folder: &'static [&'static str]) -> Source {
    Source {
        reads_as: "cursor",
        location: Location {
            base,
            env: &["AGENTHUD_DATA_DIR"],
            folders: folder,
            profiles: None,
            path: "clients/cursor",
            not_if: None,
        },
    }
}
static MAC_SOURCES: [Source; 2] = [HEADERS, turn_log(Base::AppData, &["app.agenthud.mac"])];
static WINDOWS_SOURCES: [Source; 2] = [
    HEADERS,
    turn_log(Base::Home, &["AppData/Local/app.agenthud.windows"]),
];
static LINUX_SOURCES: [Source; 2] = [HEADERS, turn_log(Base::DataHome, &["app.agenthud.linux"])];

// Brings Cursor forward without a session, for one with no folder or cloud id.
static APP_NAVIGATION: crate::discovery::AppNavigation = crate::discovery::AppNavigation {
    kind: "cursor-app",
    url: PerOs {
        macos: Support::Verified("cursor://"),
        windows: Support::Unverified("cursor://"),
        linux: Support::Unsupported("Cursor's desktop link is not verified on Linux"),
    },
};

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

/// Where a Cursor session opens: a cloud agent by its `bc-` id, a local one
/// by its folder. None when neither is known, so Cursor's app link stands in.
pub fn navigation_target(id: &str, cwd: Option<&str>) -> Option<serde_json::Value> {
    let cloud = id.len() <= 128
        && id.starts_with("bc-")
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'));
    let cwd = cwd
        .map(str::trim)
        .filter(|cwd| is_absolute_workspace_path(cwd));
    (cloud || cwd.is_some()).then(|| serde_json::json!({"kind":"cursor-agent","id":id,"cwd":cwd}))
}

/// Cursor's macOS bundle identifier. A local session's folder opens in it
/// directly: Cursor asks before it opens a folder from a `cursor://file` link.
pub const BUNDLE: &str = "com.todesktop.230313mzl4w4u92";

/// `navigation_target` for a session's id as the engine keeps it, `cursor:<id>`.
fn session_target(id: &str, cwd: Option<&str>, _client: Option<&str>) -> Option<serde_json::Value> {
    navigation_target(id.strip_prefix("cursor:")?, cwd)
}

static DISCOVERY: Discovery = Discovery {
    storage: &[
        Location::app_data(&["Cursor"], ""),
        Location::home(&[], &[".cursor"], ""),
    ],
    settings: &[
        Location::app_data(&["Cursor"], "User/settings.json"),
        Location::app_data(&["Cursor"], "User/globalStorage/state.vscdb"),
    ],
    sessions: &[
        Location::home(&[], &[".cursor"], "chats"),
        Location::app_data(&["Cursor"], "User/workspaceStorage"),
    ],
    commands: &["cursor", "cursor-agent"],
    apps: &["Cursor"],
    bundles: &[BUNDLE],
    session_target: Some(session_target),
    app_navigation: Some(&APP_NAVIGATION),
    ..Discovery::NONE
};

pub static AGENT: Agent = Agent {
    id: "cursor",
    name: "Cursor",
    sessions: true,
    hooks: Some(&hooks::HOOKS),
    observer: None,
    channel: None,
    program: None,
    replies: None,
    turns: None,
    items: None,
    sources: PerOs {
        macos: Support::Verified(&MAC_SOURCES),
        windows: Support::Unverified(&WINDOWS_SOURCES),
        linux: Support::Unverified(&LINUX_SOURCES),
    },
    discovery: &DISCOVERY,
};

#[cfg(test)]
mod tests {
    use super::{navigation_target, APP_NAVIGATION, WINDOWS_SOURCES};
    use crate::Support;
    use serde_json::json;

    #[test]
    fn a_session_gets_its_own_target_only_with_a_folder_or_cloud_id() {
        assert_eq!(
            navigation_target("bc-1234", None),
            Some(json!({"kind":"cursor-agent","id":"bc-1234","cwd":null}))
        );
        assert_eq!(
            navigation_target("3e8f", Some(" /Users/a/p ")),
            Some(json!({"kind":"cursor-agent","id":"3e8f","cwd":"/Users/a/p"}))
        );
        assert_eq!(
            navigation_target("3e8f", Some(r" C:\Users\a\p ")),
            Some(json!({"kind":"cursor-agent","id":"3e8f","cwd":r"C:\Users\a\p"}))
        );
        assert_eq!(
            navigation_target("3e8f", Some(" d:/work/proj ")),
            Some(json!({"kind":"cursor-agent","id":"3e8f","cwd":"d:/work/proj"}))
        );
        assert_eq!(
            navigation_target("3e8f", Some(r" \\server\share\proj ")),
            Some(json!({"kind":"cursor-agent","id":"3e8f","cwd":r"\\server\share\proj"}))
        );
        assert_eq!(navigation_target("3e8f", None), None);
        assert_eq!(navigation_target("3e8f", Some("relative")), None);
        assert_eq!(navigation_target("3e8f", Some("C:")), None);
        assert_eq!(navigation_target("3e8f", Some("C:relative")), None);
        assert_eq!(navigation_target("3e8f", Some(r"1:\bad")), None);
        assert_eq!(navigation_target("3e8f", Some(r"\\")), None);
        assert_eq!(navigation_target("bc-a/b", None), None);
    }

    #[test]
    fn windows_enables_turn_log_and_app_navigation() {
        assert_eq!(WINDOWS_SOURCES.len(), 2);
        assert_eq!(WINDOWS_SOURCES[1].location.base, crate::Base::Home);
        assert_eq!(
            WINDOWS_SOURCES[1].location.folders,
            &["AppData/Local/app.agenthud.windows"]
        );
        assert_eq!(
            WINDOWS_SOURCES[1].location.relative(),
            "AppData/Local/app.agenthud.windows/clients/cursor"
        );
        assert!(matches!(
            APP_NAVIGATION.url.windows,
            Support::Unverified("cursor://")
        ));
    }
}
