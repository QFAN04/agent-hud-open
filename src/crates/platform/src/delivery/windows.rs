//! What sending and starting need from Windows: folders and files closed to
//! other users, standard application folders, npm's command folder, and
//! `rundll32` for links.
use super::{Failure, Result};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

/// Makes `path` a folder only this user can open (`crate::os::private_directory`).
pub fn private_dir(path: &Path) -> Result<()> {
    crate::os::private_directory(path).map_err(Failure::Io)
}

/// Options that create a new file to write. It takes the protection of the
/// private folder it is made in.
pub(crate) fn new_file_options() -> fs::OpenOptions {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    options
}

/// Whether an environment key names the command search path: `PATH` in any
/// case, as Windows reads it.
pub(crate) fn is_path_key(key: &str) -> bool {
    key.eq_ignore_ascii_case("PATH")
}

/// Where command lines are installed that a GUI app's `PATH` lacks: npm's
/// global folder under `%APPDATA%`.
pub(crate) fn command_folders() -> Vec<PathBuf> {
    std::env::var_os("APPDATA")
        .map(|appdata| PathBuf::from(appdata).join("npm"))
        .into_iter()
        .collect()
}

/// Where desktop apps are installed on Windows: per-user `%LOCALAPPDATA%\Programs`,
/// `%LOCALAPPDATA%`, Store execution aliases under `WindowsApps`, and the
/// system `%ProgramFiles%` / `%ProgramFiles(x86)%` roots.
pub(crate) fn applications(home: &Path) -> Vec<PathBuf> {
    applications_in(
        home,
        env_path("LOCALAPPDATA"),
        env_path("ProgramFiles"),
        env_path("ProgramW6432"),
        env_path("ProgramFiles(x86)"),
    )
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn applications_in(
    home: &Path,
    local_app_env: Option<PathBuf>,
    program_files: Option<PathBuf>,
    program_w6432: Option<PathBuf>,
    program_files_x86: Option<PathBuf>,
) -> Vec<PathBuf> {
    let mut folders = Vec::new();
    let local_app = local_app_env.unwrap_or_else(|| home.join("AppData/Local"));
    for root in [local_app, home.join("AppData/Local")] {
        push_unique(&mut folders, root.join("Programs"));
        push_unique(&mut folders, root.join("Microsoft/WindowsApps"));
        push_unique(&mut folders, root);
    }
    push_unique(
        &mut folders,
        program_files.unwrap_or_else(|| PathBuf::from(r"C:\Program Files")),
    );
    if let Some(w6432) = program_w6432 {
        push_unique(&mut folders, w6432);
    }
    if let Some(x86) = program_files_x86 {
        push_unique(&mut folders, x86);
    }
    folders
}

fn push_unique(folders: &mut Vec<PathBuf>, path: PathBuf) {
    let candidate = path.to_string_lossy().replace('/', "\\");
    if folders.iter().any(|existing| {
        existing
            .to_string_lossy()
            .replace('/', "\\")
            .eq_ignore_ascii_case(&candidate)
    }) {
        return;
    }
    folders.push(path);
}

/// The command that opens a client link: `url.dll`'s protocol handler.
pub(crate) fn link_opener(uri: &str) -> Result<(PathBuf, Vec<String>)> {
    Ok((
        std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or_else(|| Failure::Unavailable("Windows system directory".into()))?
            .join("System32/rundll32.exe"),
        vec!["url.dll,FileProtocolHandler".to_string(), uri.to_string()],
    ))
}

/// Where tests make their folders: the system's temporary folder.
#[cfg(any(test, feature = "fixtures"))]
pub(crate) fn fixture_root() -> PathBuf {
    std::env::temp_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applications_in_falls_back_to_home_and_deduplicates_case_insensitively() {
        let home = Path::new(r"C:\Users\alice");
        let fallback = applications_in(home, None, None, None, None);
        assert_eq!(
            fallback,
            vec![
                home.join("AppData/Local").join("Programs"),
                home.join("AppData/Local").join("Microsoft/WindowsApps"),
                home.join("AppData/Local"),
                PathBuf::from(r"C:\Program Files"),
            ]
        );

        let custom = applications_in(
            home,
            Some(PathBuf::from(r"c:\users\alice\appdata\local")),
            Some(PathBuf::from(r"C:\Program Files")),
            Some(PathBuf::from(r"c:\program files")),
            Some(PathBuf::from(r"C:\Program Files (x86)")),
        );
        assert_eq!(
            custom,
            vec![
                PathBuf::from(r"c:\users\alice\appdata\local\Programs"),
                PathBuf::from(r"c:\users\alice\appdata\local\Microsoft/WindowsApps"),
                PathBuf::from(r"c:\users\alice\appdata\local"),
                PathBuf::from(r"C:\Program Files"),
                PathBuf::from(r"C:\Program Files (x86)"),
            ]
        );
    }
}
