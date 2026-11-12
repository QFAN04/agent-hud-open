//! Windows wrapper command for Antigravity CLI's `statusLine.command`.
//! Antigravity executes `statusLine.command` via `cmd.exe /c`, so wrapping in
//! `powershell.exe -EncodedCommand` keeps stdin bytes raw, silences the
//! callback, isolates the renderer command line under `cmd.exe /d /s /c`,
//! and preserves the renderer's exact stdout, stderr, and exit code.
use base64::Engine;

const POWERSHELL_PREFIX: &str =
    "powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ";

fn ps_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn quote_msvc_arg(arg: &str) -> String {
    let mut quoted = String::from("\"");
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                for _ in 0..(backslashes * 2 + 1) {
                    quoted.push('\\');
                }
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                for _ in 0..backslashes {
                    quoted.push('\\');
                }
                backslashes = 0;
                quoted.push(ch);
            }
        }
    }
    for _ in 0..(backslashes * 2) {
        quoted.push('\\');
    }
    quoted.push('"');
    quoted
}

fn encode_powershell(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    format!(
        "{POWERSHELL_PREFIX}{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

fn decode_powershell(command: &str) -> Option<String> {
    let encoded = command.trim().strip_prefix(POWERSHELL_PREFIX)?;
    if encoded.is_empty() || encoded.contains(char::is_whitespace) {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    let chunks = bytes.chunks_exact(2);
    if !chunks.remainder().is_empty() {
        return None;
    }
    let words: Vec<u16> = chunks
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();
    String::from_utf16(&words).ok()
}

pub(super) fn validate_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.contains(['\r', '\n', '"']) {
        return Err("The callback path contains unsupported characters".into());
    }
    Ok(())
}

pub(super) fn callback_display(executable: &str, flag: &str, data: &str, settings: &str) -> String {
    format!(
        "{} {flag} {} {}",
        quote_msvc_arg(executable),
        quote_msvc_arg(data),
        quote_msvc_arg(settings),
    )
}

pub(super) fn command(
    executable: &str,
    flag: &str,
    data: &str,
    settings: &str,
    renderer: Option<&str>,
) -> Result<String, String> {
    let callback_args = format!(
        "{flag} {} {}",
        quote_msvc_arg(data),
        quote_msvc_arg(settings),
    );
    let mut script = format!(
        "$ProgressPreference='SilentlyContinue'\n\
         $in=[System.Console]::OpenStandardInput()\n\
         $buf=[System.IO.MemoryStream]::new()\n\
         $in.CopyTo($buf)\n\
         $bytes=$buf.ToArray()\n\
         try{{\n\
         $p=[System.Diagnostics.ProcessStartInfo]::new({exe},{args})\n\
         $p.UseShellExecute=$false\n\
         $p.RedirectStandardInput=$true\n\
         $p.RedirectStandardOutput=$true\n\
         $p.RedirectStandardError=$true\n\
         $p.CreateNoWindow=$true\n\
         $c=[System.Diagnostics.Process]::Start($p)\n\
         $so=$c.StandardOutput.BaseStream.CopyToAsync([System.IO.Stream]::Null)\n\
         $se=$c.StandardError.BaseStream.CopyToAsync([System.IO.Stream]::Null)\n\
         try{{$c.StandardInput.BaseStream.Write($bytes,0,$bytes.Length);$c.StandardInput.BaseStream.Close()}}catch{{}}\n\
         [System.Threading.Tasks.Task]::WaitAll(@([System.Threading.Tasks.Task]$so,[System.Threading.Tasks.Task]$se))\n\
         $c.WaitForExit()\n\
         }}catch{{}}\n",
        exe = ps_quote(executable),
        args = ps_quote(&callback_args),
    );
    if let Some(renderer) = renderer {
        if renderer.contains(['\r', '\n', '\0']) {
            return Err("The existing CLI statusLine command is unavailable".into());
        }
        let cmd_args = format!("/d /s /c \"{renderer}\"");
        script.push_str(&format!(
            "$q=[System.Diagnostics.ProcessStartInfo]::new('cmd.exe',{cmd_args})\n\
             $q.UseShellExecute=$false\n\
             $q.RedirectStandardInput=$true\n\
             $q.RedirectStandardOutput=$true\n\
             $q.RedirectStandardError=$true\n\
             $q.CreateNoWindow=$true\n\
             $r=[System.Diagnostics.Process]::Start($q)\n\
             $os=[System.IO.MemoryStream]::new()\n\
             $es=[System.IO.MemoryStream]::new()\n\
             $so=$r.StandardOutput.BaseStream.CopyToAsync($os)\n\
             $se=$r.StandardError.BaseStream.CopyToAsync($es)\n\
             try{{$r.StandardInput.BaseStream.Write($bytes,0,$bytes.Length);$r.StandardInput.BaseStream.Close()}}catch{{}}\n\
             [System.Threading.Tasks.Task]::WaitAll(@([System.Threading.Tasks.Task]$so,[System.Threading.Tasks.Task]$se))\n\
             $r.WaitForExit()\n\
             $out=[System.Console]::OpenStandardOutput()\n\
             $err=[System.Console]::OpenStandardError()\n\
             $ob=$os.ToArray();if($ob.Length){{$out.Write($ob,0,$ob.Length);$out.Flush()}}\n\
             $eb=$es.ToArray();if($eb.Length){{$err.Write($eb,0,$eb.Length);$err.Flush()}}\n\
             exit $r.ExitCode\n",
            cmd_args = ps_quote(&cmd_args),
        ));
    } else {
        script.push_str("exit 0\n");
    }
    Ok(encode_powershell(&script))
}

pub(super) fn contains_flag(command: &str, flag: &str) -> bool {
    command.contains(flag) || decode_powershell(command).is_some_and(|script| script.contains(flag))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_detects_windows_statusline_wrapper_commands() {
        let exe = r"C:\Users\O'Brien\Agent HUD.exe";
        let data = r"C:\Users\O'Brien\AppData\Local\Agent HUD";
        let settings = r"C:\Users\O'Brien\.gemini\antigravity-cli\settings.json";
        let display = callback_display(exe, "--antigravity-statusline-hook", data, settings);
        assert_eq!(
            display,
            "\"C:\\Users\\O'Brien\\Agent HUD.exe\" --antigravity-statusline-hook \"C:\\Users\\O'Brien\\AppData\\Local\\Agent HUD\" \"C:\\Users\\O'Brien\\.gemini\\antigravity-cli\\settings.json\""
        );

        let standalone =
            command(exe, "--antigravity-statusline-hook", data, settings, None).unwrap();
        assert!(standalone.starts_with(POWERSHELL_PREFIX));
        assert!(contains_flag(&standalone, "--antigravity-statusline-hook"));
        let standalone_ps = decode_powershell(&standalone).unwrap();
        assert!(standalone_ps.contains("'C:\\Users\\O''Brien\\Agent HUD.exe'"));
        assert!(standalone_ps.contains("exit 0"));
        assert!(!standalone_ps.contains("cmd.exe"));

        let wrapped = command(
            exe,
            "--antigravity-statusline-hook",
            data,
            settings,
            Some("\"C:\\Tools\\O'Brien\\render.exe\" --json | findstr ok"),
        )
        .unwrap();
        assert!(contains_flag(&wrapped, "--antigravity-statusline-hook"));
        let wrapped_ps = decode_powershell(&wrapped).unwrap();
        assert!(wrapped_ps
            .contains("/d /s /c \"\"C:\\Tools\\O''Brien\\render.exe\" --json | findstr ok\""));
        assert!(wrapped_ps.contains("exit $r.ExitCode"));

        assert!(validate_path("bad\"quote").is_err());
        assert!(validate_path("bad\nline").is_err());
        assert!(command(
            exe,
            "--antigravity-statusline-hook",
            data,
            settings,
            Some("echo 1\necho 2")
        )
        .is_err());
    }

    #[cfg(windows)]
    #[test]
    fn live_windows_statusline_plans_and_executes_preserving_bytes_and_exit_code() {
        use super::super::super::{status, METADATA};
        use super::super::plan_file;
        use crate::hooks::InstallContext;
        use serde_json::{json, Value};
        use std::{
            collections::HashMap,
            fs,
            io::Write,
            process::{Command, Stdio},
        };

        let status_payload = status(None);
        assert_eq!(status_payload["installable"], false);
        assert!(!status_payload["notice"]
            .as_str()
            .unwrap()
            .contains("unavailable on this system"));

        let temp = tempfile::tempdir().unwrap();
        let cb_dir = temp.path().join("cb O'Brien");
        let rnd_dir = temp.path().join("rnd O'Brien");
        let data = temp.path().join("data O'Brien");
        fs::create_dir_all(&cb_dir).unwrap();
        fs::create_dir_all(&rnd_dir).unwrap();
        fs::create_dir_all(&data).unwrap();

        let cb_flag = temp.path().join("cb_flag.txt");
        let cb_settings = temp.path().join("cb_settings.txt");
        let cb_in = temp.path().join("cb_stdin.txt");
        let callback_cmd = cb_dir.join("hook.cmd");
        let cb_ps = temp.path().join("cb.ps1");
        fs::write(
            &cb_ps,
            format!(
                "$in=[Console]::In.ReadToEnd()\n\
                 [IO.File]::WriteAllText('{}',$args[0])\n\
                 [IO.File]::WriteAllText('{}',$args[2])\n\
                 [IO.File]::WriteAllText('{}',$in)\n\
                 [Console]::Write('NOISY_CB_STDOUT')\n\
                 [Console]::Error.Write('NOISY_CB_STDERR')\n\
                 exit 19\n",
                cb_flag.to_str().unwrap().replace('\'', "''"),
                cb_settings.to_str().unwrap().replace('\'', "''"),
                cb_in.to_str().unwrap().replace('\'', "''"),
            ),
        )
        .unwrap();
        fs::write(
            &callback_cmd,
            format!(
                "@echo off\r\npowershell -NoProfile -ExecutionPolicy Bypass -File \"{}\" %*\r\nexit /b %ERRORLEVEL%\r\n",
                cb_ps.to_str().unwrap()
            ),
        )
        .unwrap();

        let rnd_cmd = rnd_dir.join("render.cmd");
        let rnd_ps = temp.path().join("rnd.ps1");
        fs::write(
            &rnd_ps,
            "$ms=[IO.MemoryStream]::new()\n\
             [Console]::OpenStandardInput().CopyTo($ms)\n\
             $b=$ms.ToArray()\n\
             $pfx=[Text.Encoding]::UTF8.GetBytes('original:')\n\
             $out=[Console]::OpenStandardOutput()\n\
             $out.Write($pfx,0,$pfx.Length)\n\
             $out.Write($b,0,$b.Length)\n\
             $out.Flush()\n\
             $err=[Console]::OpenStandardError()\n\
             $eb=[Text.Encoding]::UTF8.GetBytes('rnd_err')\n\
             $err.Write($eb,0,$eb.Length)\n\
             $err.Flush()\n\
             exit 7\n",
        )
        .unwrap();
        fs::write(
            &rnd_cmd,
            format!(
                "@echo off\r\npowershell -NoProfile -ExecutionPolicy Bypass -File \"{}\"\r\nexit /b %ERRORLEVEL%\r\n",
                rnd_ps.to_str().unwrap()
            ),
        )
        .unwrap();

        let settings_path = temp.path().join("settings 'one'.json");
        let orig_cmd = format!("\"{}\" --ignored", rnd_cmd.to_str().unwrap());
        let original = json!({
            "statusLine": {
                "type": "command",
                "command": orig_cmd,
                "padding": 4,
                "stack_with_default": false,
            },
            "other": "keep"
        });
        fs::write(
            &settings_path,
            serde_json::to_vec_pretty(&original).unwrap(),
        )
        .unwrap();

        let context = InstallContext {
            home: temp.path().to_owned(),
            environment: HashMap::new(),
            canonical_executable: Some(callback_cmd.clone()),
            executable: callback_cmd,
        };
        let change = plan_file(&context, &data, &settings_path, true)
            .unwrap()
            .expect("should plan change");
        let installed: Value = serde_json::from_slice(change.updated.as_ref().unwrap()).unwrap();
        assert_eq!(installed["statusLine"]["padding"], 4);
        assert_eq!(installed["statusLine"]["stack_with_default"], false);
        assert_eq!(installed["other"], "keep");
        assert!(installed.get(METADATA).is_some());

        let cmd_str = installed["statusLine"]["command"].as_str().unwrap();
        let payload = "{\"email\":\"a@b.c\",\"trailing\":true}\n".as_bytes();
        let mut child = Command::new("cmd.exe")
            .args(["/d", "/s", "/c", cmd_str])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(payload).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(7));
        assert_eq!(out.stdout, [b"original:", payload].concat());
        assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "rnd_err");
        assert_eq!(
            fs::read_to_string(&cb_flag).unwrap(),
            "--antigravity-statusline-hook"
        );
        assert_eq!(
            fs::read_to_string(&cb_settings).unwrap(),
            settings_path.canonicalize().unwrap().to_str().unwrap()
        );
        assert_eq!(
            fs::read_to_string(&cb_in).unwrap().trim(),
            "{\"email\":\"a@b.c\",\"trailing\":true}"
        );

        // Orphaned metadata conflict check detects -EncodedCommand flag.
        let mut orphan = installed.clone();
        orphan.as_object_mut().unwrap().remove(METADATA);
        fs::write(&settings_path, serde_json::to_vec(&orphan).unwrap()).unwrap();
        assert!(plan_file(&context, &data, &settings_path, true).is_err());

        // Restores exact original statusLine when disabled.
        fs::write(&settings_path, change.updated.unwrap()).unwrap();
        let uninstall = plan_file(&context, &data, &settings_path, false)
            .unwrap()
            .expect("should plan uninstall");
        let restored: Value = serde_json::from_slice(uninstall.updated.as_ref().unwrap()).unwrap();
        assert_eq!(restored, original);
    }
}
