use super::*;

fn process(pid: u32, executable: &str, command: &str) -> crate::os::ProcessInfo {
    crate::os::ProcessInfo {
        pid,
        executable: executable.into(),
        command: command.into(),
    }
}

#[test]
fn discovers_all_authenticated_native_and_cli_servers() {
    let found = parse_servers(vec![
        process(
            1,
            "/Applications/Antigravity.app/language_server",
            "server --csrf_token fixture-1",
        ),
        process(
            2,
            "/u/.gemini/antigravity-cli/language_server",
            "server --csrf_token fixture-2",
        ),
        process(3, "/bin/agy", "agy"),
        process(
            4,
            "/unrelated/language_server",
            "server --csrf_token fixture-4",
        ),
        process(
            5,
            "/Applications/Antigravity IDE.app/language-server",
            "server --csrf_token fixture-5",
        ),
    ]);
    assert_eq!(
        found.iter().map(|server| server.pid).collect::<Vec<_>>(),
        [1, 2, 5]
    );
}

#[test]
fn discovers_windows_language_server_and_cli_with_native_paths() {
    let found = parse_servers(vec![
        process(
            1,
            r"C:\Program Files\Antigravity\language_server.exe",
            "server --app_data_dir antigravity --csrf_token fixture",
        ),
        process(2, r"C:\Tools\agy.exe", "agy.exe --csrf_token fixture"),
        process(
            3,
            r"C:\Other\language_server.exe",
            "server --app_data_dir unrelated --csrf_token fixture",
        ),
    ]);
    assert_eq!(
        found.iter().map(|server| server.pid).collect::<Vec<_>>(),
        [1, 2]
    );
}

#[test]
fn tokenless_cli_fallback_uses_the_executable_not_command_arguments() {
    let root = tempfile::tempdir().unwrap();
    let executable = root.path().join("installed/agy");
    let executable = executable.to_str().unwrap();
    let found = cli_from(vec![
        process(1, "/bin/sh", "sh /bin/agy"),
        process(2, executable, "agy"),
    ]);
    assert_eq!(found.as_deref(), Some(executable));
    assert!(cli_from(vec![process(1, "agy", "agy")]).is_none());
}

#[test]
fn parses_windows_arch_suffixed_and_ide_language_server_paths() {
    let found = parse_servers(vec![
        process(
            54156,
            r"C:\Users\alice\AppData\Local\Programs\antigravity\resources\bin\language_server.exe",
            "language_server.exe --csrf_token=tok-win --app_data_dir antigravity",
        ),
        process(
            54157,
            r"C:\Program Files\Antigravity\resources\bin\language_server_windows_x64.exe",
            "language_server_windows_x64.exe --csrf_token tok-x64",
        ),
        process(
            54158,
            r"C:\Program Files\Antigravity IDE\resources\bin\language_server_windows_x64.exe",
            "language_server_windows_x64.exe --csrf_token=tok-ide",
        ),
        process(
            54159,
            r"C:\Other\language_server_windows_x64.exe",
            "language_server_windows_x64.exe --csrf_token tok-ignore",
        ),
    ]);
    assert_eq!(
        found.iter().map(|server| server.pid).collect::<Vec<_>>(),
        [54156, 54157, 54158]
    );
    assert_eq!(found[0].token, "tok-win");
    assert_eq!(found[1].token, "tok-x64");
    assert_eq!(found[2].token, "tok-ide");
}
