use std::path::Path;

pub(super) fn same_executable(running: &Path, canonical: &Path) -> bool {
    running == canonical
}
