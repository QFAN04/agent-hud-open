use std::path::{Path, PathBuf};

fn normalize_windows_key(path: &Path) -> String {
    let slashed = path.to_string_lossy().replace('/', "\\");
    let mut stripped = if slashed
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"\\?\UNC\"))
    {
        format!(r"\\{}", &slashed[8..])
    } else if let Some(rest) = slashed.strip_prefix(r"\\?\") {
        rest.to_owned()
    } else {
        slashed
    };
    while stripped.len() > 2
        && stripped.ends_with('\\')
        && !(stripped.len() == 3
            && stripped
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphabetic)
            && stripped.as_bytes().get(1) == Some(&b':'))
    {
        stripped.pop();
    }
    stripped.to_lowercase()
}

fn same_executable_with(
    running: &Path,
    canonical: &Path,
    canonicalize: impl Fn(&Path) -> Option<PathBuf>,
) -> bool {
    let resolve = |path: &Path| canonicalize(path).unwrap_or_else(|| path.to_path_buf());
    normalize_windows_key(&resolve(running)) == normalize_windows_key(&resolve(canonical))
}

pub(super) fn same_executable(running: &Path, canonical: &Path) -> bool {
    same_executable_with(running, canonical, |path| std::fs::canonicalize(path).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn matches_case_slashes_and_verbatim_prefixes() {
        let canonical = Path::new(r"C:\Users\Alice\AppData\Local\Agent HUD\Agent HUD.exe");
        let running = Path::new(r"c:/users/alice/appdata/local/agent hud/agent hud.exe");
        let verbatim = |p: &Path| Some(PathBuf::from(format!(r"\\?\{}", p.display())));
        assert!(same_executable_with(running, canonical, verbatim));
        assert!(same_executable_with(
            Path::new(r"\\?\UNC\server\share\Agent HUD.exe"),
            Path::new(r"\\SERVER\Share\agent hud.exe"),
            |_| None,
        ));
        assert!(!same_executable_with(
            Path::new(r"C:\src\target\release\Agent HUD.exe"),
            canonical,
            verbatim,
        ));
    }

    #[test]
    fn real_files_compare_identically_across_verbatim_and_plain_paths() {
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join("Agent HUD.exe");
        let other = temp.path().join("Other.exe");
        fs::write(&exe, b"a").unwrap();
        fs::write(&other, b"b").unwrap();

        let verbatim = PathBuf::from(format!(r"\\?\{}", exe.display()));
        assert!(same_executable(&verbatim, &exe));
        assert!(!same_executable(&other, &exe));
    }
}
