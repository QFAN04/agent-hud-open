//! Unix builds discover running Antigravity servers directly from process inspection.
use super::Server;

pub(super) fn servers() -> Vec<Server> {
    Vec::new()
}
