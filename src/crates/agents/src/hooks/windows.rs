//! Windows launch forms with explicit documented argv/shell contracts. A client
//! whose Windows launch is unsupported is refused instead of getting a POSIX
//! command there.
use super::configuration::{configuration, patch_configuration};
use super::{hooks_of, Hooks, Launch, Layout};
use serde_json::{json, Value};

fn validate_windows_hook(
    source: &str,
    flag: &str,
    executable: &str,
) -> Result<(&'static str, &'static Hooks), String> {
    let (source, hooks) = hooks_of(source)?;
    if executable.is_empty() || executable.contains(['\r', '\n', '\0']) {
        return Err("The Windows hook executable path is invalid".into());
    }
    if !flag.starts_with("--")
        || flag.len() <= 2
        || !flag
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
    {
        return Err("The Windows hook flag is invalid".into());
    }
    Ok((source, hooks))
}

fn quoted_command(source: &str, flag: &str, executable: &str) -> Result<String, String> {
    if executable.contains(['"', '%']) || executable.ends_with('\\') {
        return Err("The Windows hook executable path is invalid".into());
    }
    Ok(format!("\"{executable}\" {flag} {source}"))
}

/// Format the command line stored in a Windows hook configuration or text file.
pub fn windows_hook_command(source: &str, flag: &str, executable: &str) -> Result<String, String> {
    let (source, hooks) = validate_windows_hook(source, flag, executable)?;
    match hooks.launch.windows.value() {
        Some(Launch::QuotedCommand) => quoted_command(source, flag, executable),
        _ => Err(format!(
            "{source} has no verified Windows hook launch contract; its configuration was not changed"
        )),
    }
}

/// Render one handler using the client's Windows launch contract. Both public
/// callbacks and product-specific callbacks use this representation.
pub fn windows_hook_handler(source: &str, flag: &str, executable: &str) -> Result<Value, String> {
    let (source, hooks) = validate_windows_hook(source, flag, executable)?;
    match hooks.launch.windows.value() {
        Some(Launch::QuotedCommand) => {
            let command = quoted_command(source, flag, executable)?;
            match hooks.layout {
                Layout::Handlers(handler) if !handler.typed => {
                    Ok(json!({ handler.command_key: command }))
                }
                Layout::Handlers(handler) => {
                    Ok(json!({ "type": "command", handler.command_key: command }))
                }
                Layout::Text(_) => Ok(json!({ "command": command })),
                Layout::Groups
                | Layout::NestedGroups
                | Layout::Events { .. }
                | Layout::OwnedEntry => Ok(json!({ "type": "command", "command": command })),
            }
        }
        Some(Launch::CommandArgs) => Ok(json!({
            "type": "command", "command": executable, "args": [flag, source]
        })),
        Some(Launch::ExecArgs) => Ok(json!({"exec": executable, "args": [flag, source]})),
        Some(Launch::PowerShell) => Ok(json!({
            "type": "command",
            "command": format!("'{}' {flag} {source}", executable.replace('\'', "'\\''")),
            "commandWindows": powershell(executable, flag, source)
        })),
        _ => Err(format!(
            "{source} has no verified Windows hook launch contract; its configuration was not changed"
        )),
    }
}

pub fn patch_configuration_for_windows(
    source: &str,
    flag: &str,
    current: &Value,
    executable: Option<&str>,
) -> Result<Value, String> {
    let description = configuration(source, flag)?;
    let source = description.source.as_str();
    let (_, hooks) = hooks_of(source)?;
    let launch = hooks.launch.windows.value();
    if launch.is_none() && executable.is_some() {
        return Err(format!(
            "{source} has no verified Windows hook launch contract; its configuration was not changed"
        ));
    }
    let command_key = match &hooks.layout {
        Layout::Handlers(handler) => handler.command_key,
        _ => "command",
    };
    let exec = matches!(launch, Some(Launch::ExecArgs));
    let mut current = current.clone();
    for event in &description.events {
        visit_handlers_mut(&mut current, &hooks.layout, event, |handler| {
            normalize(handler, hooks, source, flag, command_key, exec);
        });
    }
    let marker = executable.map(|path| match launch {
        Some(Launch::QuotedCommand) => format!("\"{path}\" {flag} {source}"),
        _ => format!("'{}' {flag} {source}", path.replace('\'', "'\\''")),
    });
    let mut updated = patch_configuration(source, flag, &current, marker.as_deref())?;
    let Some(executable) = executable else {
        return Ok(updated);
    };
    let windows_handler = windows_hook_handler(source, flag, executable)?;
    let extra = windows_handler.as_object().unwrap();
    for event in &description.events {
        visit_handlers_mut(&mut updated, &hooks.layout, event, |handler| {
            if handler[command_key].as_str() != marker.as_deref() {
                return;
            }
            let Some(fields) = handler.as_object_mut() else {
                return;
            };
            if exec {
                fields.remove(command_key);
                fields.remove("powershell");
                fields.remove("command");
            }
            fields.extend(extra.clone());
        });
    }
    Ok(updated)
}

fn event_entries_mut<'a>(
    root: &'a mut Value,
    layout: &Layout,
    event: &str,
) -> Option<&'a mut Vec<Value>> {
    let container = match layout {
        Layout::Groups | Layout::Handlers(_) => root.get_mut("hooks"),
        Layout::NestedGroups => root.get_mut("hooks").and_then(|h| h.get_mut("events")),
        Layout::Events { .. } => Some(root),
        Layout::OwnedEntry => root.get_mut("agent-hud"),
        Layout::Text(_) => None,
    }?;
    container.get_mut(event).and_then(Value::as_array_mut)
}

fn visit_handlers_mut(
    root: &mut Value,
    layout: &Layout,
    event: &str,
    mut visit: impl FnMut(&mut Value),
) {
    let Some(entries) = event_entries_mut(root, layout, event) else {
        return;
    };
    if matches!(layout, Layout::Handlers(_)) {
        for handler in entries {
            visit(handler);
        }
        return;
    }
    for group in entries {
        if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
            for handler in handlers {
                visit(handler);
            }
        }
    }
}

fn normalize(
    handler: &mut Value,
    hooks: &Hooks,
    source: &str,
    flag: &str,
    command_key: &str,
    exec: bool,
) {
    let matches_args = std::iter::once(source)
        .chain(hooks.aliases.iter().copied())
        .any(|name| handler["args"] == json!([flag, name]));
    if !matches_args {
        return;
    }
    let path_key = if exec { "exec" } else { command_key };
    let path = handler[path_key]
        .as_str()
        .filter(|p| !p.is_empty())
        .map(str::to_owned);
    let Some(path) = path else {
        return;
    };
    let Some(fields) = handler.as_object_mut() else {
        return;
    };
    fields.remove("exec");
    fields.remove("args");
    fields.insert(
        command_key.into(),
        json!(format!("'{}' {flag} {source}", path.replace('\'', "'\\''"))),
    );
}
fn powershell(executable: &str, flag: &str, source: &str) -> String {
    use base64::Engine;
    let script = format!(
        "[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false); & '{}' {flag} {source}",
        executable.replace('\'', "''")
    );
    let bytes = script
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    format!(
        "powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand {}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn remote_callbacks_use_the_same_windows_launch_contract() {
        let executable = r"C:\Users\O'Brien\Agent HUD.exe";
        let claude = windows_hook_handler("claude", "--remote-hook", executable).unwrap();
        assert_eq!(claude["command"], executable);
        assert_eq!(claude["args"], json!(["--remote-hook", "claude"]));
        let qoder = windows_hook_handler("qoder", "--remote-hook", executable).unwrap();
        assert_eq!(
            qoder,
            json!({"type": "command", "command": executable, "args": ["--remote-hook", "qoder"]})
        );
        let cursor = windows_hook_handler("cursor", "--remote-hook", executable).unwrap();
        assert_eq!(
            cursor,
            json!({"command": "\"C:\\Users\\O'Brien\\Agent HUD.exe\" --remote-hook cursor"})
        );
        let antigravity = windows_hook_handler("antigravity", "--remote-hook", executable).unwrap();
        assert_eq!(
            antigravity,
            json!({"type": "command", "command": "\"C:\\Users\\O'Brien\\Agent HUD.exe\" --remote-hook antigravity"})
        );
        let codex = windows_hook_handler("codex", "--remote-hook", executable).unwrap();
        let encoded = codex["commandWindows"]
            .as_str()
            .unwrap()
            .split_whitespace()
            .last()
            .unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        let words = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|bytes| u16::from_le_bytes(*bytes))
            .collect::<Vec<_>>();
        let script = String::from_utf16(&words).unwrap();
        assert!(script.ends_with("& 'C:\\Users\\O''Brien\\Agent HUD.exe' --remote-hook codex"));
        assert!(windows_hook_handler("codex", "--remote-hook; exit", executable).is_err());
        assert!(windows_hook_handler("claude", "--remote-hook", "bad\npath").is_err());
        assert!(windows_hook_handler("goose", "--remote-hook", executable).is_err());
    }

    #[test]
    fn windows_hook_command_quotes_and_validates_all_text_and_quoted_clients() {
        let exe = r"C:\Program Files (x86)\O'Brien\Agent HUD.exe";
        for source in [
            "antigravity",
            "codebuddy",
            "workbuddy",
            "cursor",
            "devin",
            "droid",
            "hermes",
            "kimi",
            "qwen",
            "trae",
            "trae-cn",
            "zcode",
        ] {
            let cmd = windows_hook_command(source, "--completion-hook", exe).unwrap();
            assert_eq!(cmd, format!("\"{exe}\" --completion-hook {source}"));
            assert!(cmd.ends_with(&format!(" --completion-hook {source}")));
        }
        for unsupported in [
            "claude",
            "codex",
            "copilot",
            "qoder",
            "qoder-cn",
            "qoder-work",
            "goose",
        ] {
            assert!(windows_hook_command(unsupported, "--completion-hook", exe).is_err());
        }
        for bad_path in [
            "",
            r#"C:\bad"quote\Agent.exe"#,
            r"C:\Users\%USERNAME%\Agent.exe",
            "C:\\trailing\\",
            "C:\\bad\rpath",
            "C:\\bad\npath",
            "C:\\bad\0path",
        ] {
            assert!(windows_hook_command("cursor", "--completion-hook", bad_path).is_err());
        }
        for bad_flag in ["", "no-dashes", "--", "--bad_flag", "--bad flag"] {
            assert!(windows_hook_command("cursor", bad_flag, exe).is_err());
        }
    }

    #[test]
    fn patch_configuration_for_windows_supports_every_layout_variant() {
        let exe = r"C:\Program Files\Agent HUD\Agent HUD.exe";
        let moved = r"C:\Users\O'Brien\Agent HUD.exe";

        // Layout::Groups with QuotedCommand (Devin)
        let devin_orig = json!({
            "hooks": {
                "Stop": [{"hooks": [{"type": "command", "command": "echo user"}]}]
            }
        });
        let devin_added =
            patch_configuration_for_windows("devin", "--completion-hook", &devin_orig, Some(exe))
                .unwrap();
        let devin_expected = format!("\"{exe}\" --completion-hook devin");
        assert_eq!(
            devin_added["hooks"]["Stop"][0]["hooks"][0]["command"],
            "echo user"
        );
        assert_eq!(
            devin_added["hooks"]["Stop"][1]["hooks"][0]["type"],
            "command"
        );
        assert_eq!(
            devin_added["hooks"]["Stop"][1]["hooks"][0]["command"],
            devin_expected
        );
        assert_eq!(
            patch_configuration_for_windows("devin", "--completion-hook", &devin_added, Some(exe))
                .unwrap(),
            devin_added
        );
        let devin_removed =
            patch_configuration_for_windows("devin", "--completion-hook", &devin_added, None)
                .unwrap();
        assert_eq!(devin_removed, devin_orig);

        // Layout::Groups with CommandArgs + alias (QoderWork)
        let qoder_orig = json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "*",
                    "hooks": [{"type": "command", "command": r"C:\Old\Agent.exe", "args": ["--permission-hook", "qoderWork"]}]
                }]
            }
        });
        let qoder_added = patch_configuration_for_windows(
            "qoder-work",
            "--permission-hook",
            &qoder_orig,
            Some(exe),
        )
        .unwrap();
        assert_eq!(
            qoder_added["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            exe
        );
        assert_eq!(
            qoder_added["hooks"]["PreToolUse"][0]["hooks"][0]["args"],
            json!(["--permission-hook", "qoder-work"])
        );
        let qoder_removed =
            patch_configuration_for_windows("qoder-work", "--permission-hook", &qoder_added, None)
                .unwrap();
        assert!(qoder_removed["hooks"].get("PreToolUse").is_none());

        // Layout::NestedGroups (ZCode)
        let zcode_orig = json!({});
        let zcode_added =
            patch_configuration_for_windows("zcode", "--completion-hook", &zcode_orig, Some(exe))
                .unwrap();
        assert_eq!(zcode_added["hooks"]["enabled"], true);
        assert_eq!(
            zcode_added["hooks"]["events"]["Stop"][0]["hooks"][0]["command"],
            format!("\"{exe}\" --completion-hook zcode")
        );
        let zcode_removed =
            patch_configuration_for_windows("zcode", "--completion-hook", &zcode_added, None)
                .unwrap();
        assert!(zcode_removed["hooks"].get("events").is_none());

        // Layout::Events (Droid)
        let droid_orig = json!({"Stop": [{"hooks": [{"type": "command", "command": "keep"}]}]});
        let droid_added =
            patch_configuration_for_windows("droid", "--completion-hook", &droid_orig, Some(exe))
                .unwrap();
        assert_eq!(droid_added["Stop"][0]["hooks"][0]["command"], "keep");
        assert_eq!(
            droid_added["Stop"][1]["hooks"][0]["command"],
            format!("\"{exe}\" --completion-hook droid")
        );
        assert_eq!(
            patch_configuration_for_windows("droid", "--completion-hook", &droid_added, None)
                .unwrap(),
            droid_orig
        );

        // Layout::OwnedEntry (Antigravity)
        let ag_orig = json!({"other": 1});
        let ag_added = patch_configuration_for_windows(
            "antigravity",
            "--completion-hook",
            &ag_orig,
            Some(exe),
        )
        .unwrap();
        assert_eq!(
            ag_added["agent-hud"]["Stop"][0]["command"],
            format!("\"{exe}\" --completion-hook antigravity")
        );
        let ag_moved = patch_configuration_for_windows(
            "antigravity",
            "--completion-hook",
            &ag_added,
            Some(moved),
        )
        .unwrap();
        assert_eq!(
            ag_moved["agent-hud"]["Stop"][0]["command"],
            format!("\"{moved}\" --completion-hook antigravity")
        );
        assert_eq!(
            patch_configuration_for_windows("antigravity", "--completion-hook", &ag_moved, None)
                .unwrap(),
            ag_orig
        );

        // Layout::Handlers with typed: false (Cursor) and ExecArgs (Copilot)
        let cursor_orig = json!({"hooks": {"stop": [{"command": "user-stop"}]}});
        let cursor_added =
            patch_configuration_for_windows("cursor", "--completion-hook", &cursor_orig, Some(exe))
                .unwrap();
        assert_eq!(cursor_added["version"], 1);
        assert_eq!(cursor_added["hooks"]["stop"][0]["command"], "user-stop");
        let cursor_entry = &cursor_added["hooks"]["stop"][1];
        assert_eq!(
            cursor_entry["command"],
            format!("\"{exe}\" --completion-hook cursor")
        );
        assert!(cursor_entry.get("type").is_none());
        let cursor_removed =
            patch_configuration_for_windows("cursor", "--completion-hook", &cursor_added, None)
                .unwrap();
        assert_eq!(
            cursor_removed["hooks"]["stop"],
            json!([{"command": "user-stop"}])
        );

        let copilot_added =
            patch_configuration_for_windows("copilot", "--completion-hook", &json!({}), Some(exe))
                .unwrap();
        assert_eq!(copilot_added["hooks"]["agentStop"][0]["exec"], exe);
        assert_eq!(
            copilot_added["hooks"]["agentStop"][0]["args"],
            json!(["--completion-hook", "copilot"])
        );
        assert_eq!(
            patch_configuration_for_windows("copilot", "--completion-hook", &copilot_added, None)
                .unwrap(),
            json!({})
        );

        // Layout::Text and unsupported Goose refuse install, permit removal
        assert!(patch_configuration_for_windows(
            "hermes",
            "--completion-hook",
            &json!({}),
            Some(exe)
        )
        .is_err());
        assert!(patch_configuration_for_windows(
            "goose",
            "--completion-hook",
            &json!({}),
            Some(exe)
        )
        .is_err());
        let goose_old = json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "'/old' --completion-hook goose"}]}]}});
        assert_eq!(
            patch_configuration_for_windows("goose", "--completion-hook", &goose_old, None)
                .unwrap(),
            json!({})
        );
    }
}
