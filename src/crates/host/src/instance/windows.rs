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

fn same_file_with(a: &Path, b: &Path, canonicalize: impl Fn(&Path) -> Option<PathBuf>) -> bool {
    let canonical = |path: &Path| canonicalize(path).unwrap_or_else(|| path.to_owned());
    normalize_windows_key(&canonical(a)) == normalize_windows_key(&canonical(b))
}

pub(super) fn same_file(a: &Path, b: &Path) -> bool {
    same_file_with(a, b, |path| std::fs::canonicalize(path).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn matches_verbatim_and_plain_case_insensitive_windows_paths() {
        let verbatim = Path::new(r"\\?\C:\Users\Alice\AppData\Local\Agent HUD\Agent HUD.exe");
        let plain = Path::new(r"c:/users/alice/AppData/Local/Agent HUD/agent hud.exe");
        assert!(same_file_with(verbatim, plain, |_| None));
        assert!(same_file_with(
            Path::new(r"\\?\UNC\Host\Share\Agent HUD.exe"),
            Path::new(r"\\host\share\agent hud.exe"),
            |_| None,
        ));
        assert!(same_file_with(verbatim, plain, |p| {
            (p == plain).then(|| verbatim.to_path_buf())
        }));
        assert!(!same_file_with(
            Path::new(r"C:\src\target\release\Agent HUD.exe"),
            plain,
            |_| None,
        ));
    }

    #[test]
    fn real_files_hand_over_across_verbatim_prefix_and_case_variants() {
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join("Agent HUD.exe");
        let other = temp.path().join("Other.exe");
        fs::write(&exe, b"a").unwrap();
        fs::write(&other, b"b").unwrap();

        let verbatim = PathBuf::from(format!(r"\\?\{}", exe.display()));
        assert!(super::super::same_file(&verbatim, Some(&exe)));
        assert!(!super::super::same_file(&other, Some(&exe)));
    }
}
