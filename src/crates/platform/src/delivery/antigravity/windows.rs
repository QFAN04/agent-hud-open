//! Antigravity on Windows: messages and interrupts go to the verified
//! service that opened the session
//! (`crate::clients::antigravity::service::message_session`).
use super::cascade_config::{default_cascade_config_from_status, trajectory_cascade_config};
use super::*;

/// Supported delivery operations on Windows.
pub(crate) const OPERATIONS: &[Operation] = &[
    Operation::Automatic,
    Operation::Start,
    Operation::Steer,
    Operation::Enqueue,
    Operation::Interrupt,
];
pub(crate) const NOTICE: &str = "Sends to and interrupts an existing uniquely identified session on its running Antigravity local language server.";

/// Interrupts an existing Antigravity cascade on the verified service that opened it.
pub(super) fn interrupt(cascade_id: &str) -> Result<()> {
    use crate::clients::antigravity::service::message_session;
    message_session(cascade_id)
        .map_err(windows_message_error)?
        .interrupt()
        .map(|_| ())
        .map_err(windows_message_error)
}

/// Read a model configuration from the same verified owner that will receive
/// the message, and never retry an uncertain write on another service.
pub(super) fn send(cascade_id: &str, text: &str, strategy: &str) -> Result<()> {
    use crate::clients::antigravity::service::message_session;
    let session = message_session(cascade_id).map_err(windows_message_error)?;
    let config = session
        .trajectory()
        .ok()
        .as_ref()
        .and_then(trajectory_cascade_config)
        .or_else(|| {
            session
                .user_status()
                .ok()
                .as_ref()
                .and_then(default_cascade_config_from_status)
        });
    session
        .send(&message_body_with_config(
            cascade_id, text, strategy, config,
        ))
        .map(|_| ())
        .map_err(windows_message_error)
}

fn windows_message_error(error: crate::clients::antigravity::service::MessageError) -> Failure {
    use crate::clients::antigravity::service::MessageError;
    match error {
        MessageError::Unavailable(reason) => Failure::Unavailable(reason),
        MessageError::Invalid(reason) => Failure::InvalidInput(reason),
        MessageError::Uncertain => Failure::AcceptanceUncertain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clients::antigravity::service::MessageError;

    #[test]
    fn windows_delivery_advertises_interrupt_and_maps_errors() {
        assert!(OPERATIONS.contains(&Operation::Interrupt));
        assert!(OPERATIONS.contains(&Operation::Steer));
        assert!(OPERATIONS.contains(&Operation::Enqueue));
        assert!(NOTICE.contains("interrupt"));
        assert_eq!(
            windows_message_error(MessageError::Uncertain),
            Failure::AcceptanceUncertain
        );
        assert_eq!(
            windows_message_error(MessageError::Invalid("bad".into())),
            Failure::InvalidInput("bad".into())
        );
        assert_eq!(
            windows_message_error(MessageError::Unavailable("none".into())),
            Failure::Unavailable("none".into())
        );
    }
}
