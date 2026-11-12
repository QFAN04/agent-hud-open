//! What Agent HUD writes into one CLI settings file: a status line that
//! hands agy's report to the callback before the user's own renderer runs,
//! the record that undoes it, and the plan for that file.
use super::super::{install, InstallContext};
use super::{configuration, conflict, FileChange, FLAG, METADATA};
use agent_hud_agents::Os;
use serde_json::{json, Value};
use std::path::Path;

mod windows;

struct Callback {
    display: String,
    executable: String,
    data: String,
    settings: String,
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn callback(context: &InstallContext, data: &Path, settings: &Path) -> Result<Callback, String> {
    let path = |path: &Path| {
        let text = path
            .to_str()
            .filter(|text| !text.contains('\0'))
            .ok_or_else(|| "The callback path is not valid UTF-8".to_owned())?;
        if matches!(crate::os::OS, Os::Windows) {
            windows::validate_path(text)?;
        }
        Ok(text.to_owned())
    };
    if !data.is_absolute() {
        return Err("The callback data directory must be absolute".into());
    }
    let executable = path(&context.executable)?;
    let data = path(data)?;
    let settings = path(settings)?;
    let display = match crate::os::OS {
        Os::Windows => windows::callback_display(&executable, FLAG, &data, &settings),
        Os::MacOs | Os::Linux => format!(
            "{} {FLAG} {} {}",
            quote(&executable),
            quote(&data),
            quote(&settings)
        ),
    };
    Ok(Callback {
        display,
        executable,
        data,
        settings,
    })
}

fn original_renderer(value: &Value) -> Result<&str, String> {
    if !value.is_object() || (!value["type"].is_null() && value["type"] != "command") {
        return Err("The existing CLI statusLine must be a command object".into());
    }
    value["command"]
        .as_str()
        .filter(|command| !command.trim().is_empty() && !command.contains('\0'))
        .ok_or_else(|| "The existing CLI statusLine command is unavailable".into())
}

fn wrapped_command(callback: &Callback, renderer: Option<&str>) -> Result<String, String> {
    match crate::os::OS {
        Os::Windows => windows::command(
            &callback.executable,
            FLAG,
            &callback.data,
            &callback.settings,
            renderer,
        ),
        Os::MacOs | Os::Linux => match renderer {
            Some(renderer) => Ok(format!(
                "agent_hud_input=$(cat; printf '.'); agent_hud_input=${{agent_hud_input%.}}; printf '%s' \"$agent_hud_input\" | {} >/dev/null 2>&1; printf '%s' \"$agent_hud_input\" | /bin/sh -c {}",
                callback.display,
                quote(renderer),
            )),
            None => Ok(format!("{} >/dev/null 2>&1 || :", callback.display)),
        },
    }
}

fn contains_flag(command: &str) -> bool {
    match crate::os::OS {
        Os::Windows => windows::contains_flag(command, FLAG),
        Os::MacOs | Os::Linux => command.contains(FLAG),
    }
}

fn installed_line(original: Option<&Value>, callback: &Callback) -> Result<Value, String> {
    match original {
        Some(original) if original.is_object() && original["enabled"] == false => {
            // Connecting enables collection, not a previously disabled renderer.
            // The built-in line remains visible; the exact disabled object is
            // retained in our metadata and restored when disconnecting.
            let mut installed = original.clone();
            installed["enabled"] = json!(true);
            installed["type"] = json!("command");
            installed["command"] = json!(wrapped_command(callback, None)?);
            installed["stack_with_default"] = json!(true);
            Ok(installed)
        }
        Some(original) => {
            let renderer = original_renderer(original)?;
            // A sentinel preserves trailing newlines through POSIX command
            // substitution. The payload stays in memory; no private stdin file.
            let command = wrapped_command(callback, Some(renderer))?;
            let mut installed = original.clone();
            installed["command"] = json!(command);
            Ok(installed)
        }
        None => Ok(json!({
            "type": "command",
            "command": wrapped_command(callback, None)?,
            "stack_with_default": true,
        })),
    }
}

fn metadata(original: Option<&Value>, installed: &Value, callback: &str, created: bool) -> Value {
    json!({"version":1,"callback":callback,"createdSettings":created,"hadStatusLine":original.is_some(),"originalStatusLine":original,"installedStatusLine":installed})
}

pub(super) fn plan_file(
    context: &InstallContext,
    data: &Path,
    path: &Path,
    enabled: bool,
) -> Result<Option<FileChange>, String> {
    let target = configuration(path)?;
    let original = install::contents(&target)?;
    let current: Value = match original.as_deref() {
        None | Some([]) => json!({}),
        Some(bytes) => {
            serde_json::from_slice(bytes).map_err(|_| "CLI settings are not valid JSON")?
        }
    };
    let mut updated = current
        .as_object()
        .cloned()
        .ok_or("CLI settings must be a JSON object")?;
    let command = callback(context, data, &target)?;
    let mut remove_file = false;
    if let Some(owned) = current.get(METADATA) {
        let had = owned["hadStatusLine"].as_bool().ok_or_else(conflict)?;
        let created = owned["createdSettings"].as_bool().ok_or_else(conflict)?;
        let previous = had.then(|| &owned["originalStatusLine"]);
        let installed = installed_line(previous, &command).map_err(|_| conflict())?;
        if *owned != metadata(previous, &installed, &command.display, created)
            || current.get("statusLine") != Some(&installed)
        {
            return Err(conflict());
        }
        if enabled {
            return Ok(None);
        }
        if let Some(previous) = previous {
            updated.insert("statusLine".into(), previous.clone());
        } else {
            updated.remove("statusLine");
        }
        updated.remove(METADATA);
        remove_file = created && updated.is_empty();
    } else if enabled {
        let previous = current.get("statusLine");
        if previous.is_some_and(|line| line["command"].as_str().is_some_and(contains_flag)) {
            return Err(conflict());
        }
        let installed = installed_line(previous, &command)?;
        updated.insert(
            METADATA.into(),
            metadata(previous, &installed, &command.display, original.is_none()),
        );
        updated.insert("statusLine".into(), installed);
    } else {
        return Ok(None);
    }
    let bytes = crate::json_text::rewrite(original.as_deref(), &Value::Object(updated)).unwrap();
    Ok(Some(FileChange {
        configuration: path.to_owned(),
        target,
        original,
        updated: (!remove_file).then_some(bytes),
    }))
}
