#[derive(Clone, Copy)]
pub(super) struct AppTarget {
    pub(super) bundle_id: &'static str,
    pub(super) exes: &'static [&'static str],
    pub(super) folders: &'static [&'static str],
    pub(super) url_scheme: Option<&'static str>,
    pub(super) store_prefix: Option<&'static str>,
    pub(super) accepts_folder: bool,
    pub(super) shared_cli_name: bool,
}

const TARGETS: &[AppTarget] = &[
    AppTarget {
        bundle_id: "com.anthropic.claudefordesktop",
        exes: &["Claude.exe", "claude.exe"],
        folders: &["AnthropicClaude", "Claude"],
        url_scheme: Some("claude"),
        store_prefix: Some("Claude_"),
        accepts_folder: false,
        shared_cli_name: true,
    },
    AppTarget {
        bundle_id: "com.openai.codex",
        exes: &["Codex.exe", "ChatGPT.exe"],
        folders: &["Codex", "OpenAI Codex", "ChatGPT"],
        url_scheme: Some("codex"),
        store_prefix: Some("OpenAI.Codex_"),
        accepts_folder: false,
        shared_cli_name: true,
    },
    AppTarget {
        bundle_id: "com.google.antigravity",
        exes: &["Antigravity.exe", "Antigravity IDE.exe"],
        folders: &["antigravity", "Antigravity", "Antigravity IDE"],
        url_scheme: Some("antigravity"),
        store_prefix: None,
        accepts_folder: true,
        shared_cli_name: false,
    },
    AppTarget {
        bundle_id: "com.todesktop.230313mzl4w4u92",
        exes: &["Cursor.exe"],
        folders: &["Cursor"],
        url_scheme: Some("cursor"),
        store_prefix: None,
        accepts_folder: true,
        shared_cli_name: false,
    },
    AppTarget {
        bundle_id: "com.exafunction.windsurf",
        exes: &["Windsurf.exe"],
        folders: &["Windsurf"],
        url_scheme: Some("windsurf"),
        store_prefix: None,
        accepts_folder: true,
        shared_cli_name: false,
    },
    AppTarget {
        bundle_id: "com.trae.app",
        exes: &["Trae.exe"],
        folders: &["Trae"],
        url_scheme: Some("trae"),
        store_prefix: None,
        accepts_folder: true,
        shared_cli_name: false,
    },
    AppTarget {
        bundle_id: "cn.trae.app",
        exes: &["Trae CN.exe", "Trae.exe"],
        folders: &["Trae CN"],
        url_scheme: Some("trae-cn"),
        store_prefix: None,
        accepts_folder: true,
        shared_cli_name: false,
    },
    AppTarget {
        bundle_id: "com.qoder.app",
        exes: &["Qoder.exe"],
        folders: &["Qoder"],
        url_scheme: Some("qoder"),
        store_prefix: None,
        accepts_folder: true,
        shared_cli_name: false,
    },
    AppTarget {
        bundle_id: "com.anysphere.sand",
        exes: &["Sand.exe"],
        folders: &["Sand"],
        url_scheme: None,
        store_prefix: None,
        accepts_folder: false,
        shared_cli_name: false,
    },
    AppTarget {
        bundle_id: "com.microsoft.VSCode",
        exes: &["Code.exe"],
        folders: &["Microsoft VS Code"],
        url_scheme: Some("vscode"),
        store_prefix: None,
        accepts_folder: true,
        shared_cli_name: false,
    },
    AppTarget {
        bundle_id: "com.microsoft.VSCodeInsiders",
        exes: &["Code - Insiders.exe"],
        folders: &["Microsoft VS Code Insiders"],
        url_scheme: Some("vscode-insiders"),
        store_prefix: None,
        accepts_folder: true,
        shared_cli_name: false,
    },
];

pub(super) fn app_target(bundle_id: &str) -> Option<&'static AppTarget> {
    let id = if bundle_id == "com.openai.chat" {
        "com.openai.codex"
    } else {
        bundle_id
    };
    TARGETS.iter().find(|entry| entry.bundle_id == id)
}
