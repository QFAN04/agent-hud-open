//! OS-specific launch forms and presets for client hooks.
use crate::{PerOs, Support};

/// How a hook's command is written into the settings file.
pub enum Launch {
    /// A POSIX command line: `'<agent-hud>' <flag> <client>`.
    Command,
    /// A double-quoted command line: `"<agent-hud>" <flag> <client>`.
    QuotedCommand,
    /// The executable in `command`, the flag and client in `args`.
    CommandArgs,
    /// A PowerShell line in `commandWindows`, beside the POSIX command.
    PowerShell,
    /// `exec` and `args` in place of the handler's command.
    ExecArgs,
}

/// A hook launched as a POSIX command on macOS and Linux, and not launched on
/// Windows until its contract there is documented.
pub const POSIX_ONLY: PerOs<Launch> = PerOs {
    macos: Support::Verified(Launch::Command),
    windows: Support::Unsupported("no verified Windows hook launch contract"),
    linux: Support::Unverified(Launch::Command),
};

/// A hook launched as a POSIX command on macOS and Linux, and as a double-quoted
/// command line on Windows.
pub const QUOTED_COMMAND: PerOs<Launch> = PerOs {
    macos: Support::Verified(Launch::Command),
    windows: Support::Unverified(Launch::QuotedCommand),
    linux: Support::Unverified(Launch::Command),
};
