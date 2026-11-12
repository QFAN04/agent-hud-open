use std::path::Path;

pub(super) fn disallowed_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    !path.is_absolute()
        || text.starts_with("/Volumes/")
        || text.contains("/AppTranslocation/")
        || path.components().any(|part| part.as_os_str() == "target")
}

pub(super) fn same_executable(running: &Path, fixed: &Path, _resolved_fixed: &Path) -> bool {
    running == fixed
}
