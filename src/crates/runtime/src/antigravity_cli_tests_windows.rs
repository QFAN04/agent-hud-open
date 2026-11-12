use super::*;

#[test]
fn antigravity_cli_is_supported_on_windows_and_requires_delegated_fixed_owner() {
    let temp = tempfile::tempdir().unwrap();
    let mut runtime = isolated(temp.path());
    let before = status(&mut runtime);
    assert_eq!(before["installable"], false);
    assert_eq!(
        before["defaultSettings"],
        json!(temp
            .path()
            .join("home/.gemini/antigravity-cli/settings.json"))
    );
    let notice = before["notice"].as_str().unwrap();
    assert!(
        !notice.contains("unavailable on this system") && !notice.contains("此系统暂不支持"),
        "unexpected unsupported notice: {notice}"
    );
    assert!(
        notice.contains("delegated") || notice.contains("尚未指定"),
        "expected missing fixed-owner notice, got: {notice}"
    );

    let path = temp.path().join("home/settings.json");
    let original = b"{ \"theme\" : \"dark\" }\n";
    fs::write(&path, original).unwrap();
    assert_eq!(set_paths(&mut runtime, json!([path])).unwrap_err(), notice);
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(runtime.core.antigravity_cli_settings().is_empty());
    assert!(!temp.path().join("app-data/settings.json").exists());
    assert_eq!(read(&path), json!({"theme":"dark"}));

    // A temporary executable in %TEMP% is rejected by Windows fixed-owner validation
    // rather than claiming Antigravity CLI status-line callbacks are unsupported.
    let temp_exe = temp.path().join("fixed hook owner");
    fs::write(&temp_exe, b"fixture only; never execute").unwrap();
    let err = runtime
        .hooks_handle()
        .lock()
        .unwrap()
        .delegate_owner(hooks::InstallContext {
            home: temp.path().join("home"),
            environment: HashMap::new(),
            canonical_executable: Some(temp_exe.clone()),
            executable: temp_exe,
        })
        .unwrap_err();
    assert!(!err.is_empty());
    assert!(set_paths(&mut runtime, json!([path])).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
}
