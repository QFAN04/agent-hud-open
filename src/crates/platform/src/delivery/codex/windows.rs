//! Codex on Windows: the desktop app's owner pipe (`\\.\pipe\codex-ipc`)
//! for start, steer, and interrupt, and Codex's queue through a short-lived
//! app-server. Windows has no shared CLI daemon, so a thread only a terminal
//! CLI holds takes no message here.
use super::*;
use crate::clients::codex::desktop_io::{DesktopIpc, IpcError};
use serde_json::{json, Value};

/// Agent HUD sends to Codex on this host.
pub(crate) const SUPPORTED: bool = true;
/// What a send may ask for: start, steer, and interrupt through the desktop
/// app, and Codex's queue.
pub(crate) const OPERATIONS: &[Operation] = &[
    Operation::Automatic,
    Operation::Start,
    Operation::Steer,
    Operation::Enqueue,
    Operation::Interrupt,
];

pub(super) fn deliver(
    data: &Path,
    target: &SessionTarget,
    request: &SendRequest,
    timeout: Duration,
    queue: Option<&QueueRoute>,
) -> Result<Receipt> {
    let thread = target
        .thread_id
        .as_deref()
        .filter(|thread| valid_uuid(thread))
        .ok_or_else(|| Failure::InvalidInput("Codex thread identity".into()))?;
    if request.operation == Operation::Interrupt {
        return deliver_interrupt(data, target, request, thread, timeout);
    }
    if request.operation != Operation::Enqueue {
        return super::desktop::deliver(data, target, request, thread, timeout)?.ok_or_else(|| {
            Failure::Unavailable(
                "No active Codex Desktop owner; Windows CLI owner control is unavailable".into(),
            )
        });
    }
    let route = queue.ok_or_else(|| Failure::Unavailable("Codex executable".into()))?;
    let message_id = format!("agent-hud:{}", request.request_id);
    enqueue_stdio(route, data, thread, &request.text, &message_id, timeout)?;
    Ok(Receipt {
        provider: Provider::Codex,
        session_id: target.session_id.clone(),
        thread_id: target.thread_id.clone(),
        request_id: request.request_id.clone(),
        client_request_id: message_id,
        turn_id: None,
        owner_client_id: None,
        state: DeliveryState::Queued,
        at_ms: millis(SystemTime::now()),
        reason: Some("Queued until Codex opens the thread".into()),
    })
}

/// Windows has no shared CLI daemon to start a thread in: the caller starts
/// it through a short-lived app-server, with or without the desktop app.
pub(crate) fn start_in_daemon(
    _data: &Path,
    _folder: &Path,
    _request: &StartRequest,
    _message_id: &str,
    _timeout: Duration,
) -> Option<Result<StartReceipt>> {
    None
}

/// Queues a new thread's first message through a short-lived app-server.
pub(crate) fn queue_created(
    route: &QueueRoute,
    data: &Path,
    thread: &str,
    text: &str,
    message_id: &str,
    timeout: Duration,
) -> Result<()> {
    enqueue_stdio(route, data, thread, text, message_id, timeout)
}

fn interrupt_request_args(thread: &str, expected: Option<&str>) -> Result<(i64, Value)> {
    if expected.is_some_and(str::is_empty) {
        return Err(Failure::InvalidInput("expectedTurnId".into()));
    }
    Ok(match expected {
        Some(turn) => (
            4,
            json!({"conversationId": thread, "mode": "user-stop", "expectedTurnId": turn}),
        ),
        None => (3, json!({"conversationId": thread, "mode": "user-stop"})),
    })
}

fn parse_interrupt_response(
    response: &Value,
    owner: &str,
    expected: Option<&str>,
) -> Result<(String, Option<String>)> {
    if response["handledByClientId"] != owner {
        return Err(Failure::AcceptanceUncertain);
    }
    let accepted = &response["result"]["result"];
    if accepted["ok"] != true {
        return Err(Failure::AcceptanceUncertain);
    }
    let Some(turn) = accepted["interruptedTurnId"]
        .as_str()
        .filter(|value| !value.is_empty())
    else {
        return Err(Failure::Rejected(if expected.is_some() {
            "Desktop no longer owns the expected active turn".into()
        } else {
            "Desktop thread has no active turn to interrupt".into()
        }));
    };
    if expected.is_some_and(|wanted| wanted != turn) {
        return Err(Failure::AcceptanceUncertain);
    }
    let warning = accepted["goalPauseError"]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    Ok((turn.to_owned(), warning))
}

fn deliver_interrupt(
    data: &Path,
    target: &SessionTarget,
    request: &SendRequest,
    thread: &str,
    timeout: Duration,
) -> Result<Receipt> {
    let expected = request.expected_turn_id.as_deref();
    let (version, params) = interrupt_request_args(thread, expected)?;
    let unavailable = || {
        Failure::Unavailable(
            "No active Codex Desktop owner; Windows CLI owner control is unavailable".into(),
        )
    };
    let mut connection =
        match DesktopIpc::connect(&crate::clients::codex::desktop_io::endpoint(data), timeout) {
            Ok(value) => value,
            Err(IpcError::Unavailable) => return Err(unavailable()),
            Err(error) => return Err(error.into()),
        };
    let initialized = connection.request(
        "initialize",
        0,
        "initializing-client",
        None,
        json!({"clientType": "agent-hud"}),
        None,
    )?;
    let client = initialized["result"]["clientId"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Failure::Protocol("Desktop initialization identity".into()))?
        .to_string();
    let discovery = match connection.request(
        "thread-owner-discovery",
        1,
        &client,
        None,
        json!({"hostId": "local", "conversationId": thread}),
        None,
    ) {
        Ok(value) => value,
        Err(IpcError::Rejected(reason)) if reason == "no-client-found" => {
            return Err(unavailable());
        }
        Err(error) => return Err(error.into()),
    };
    let owner = discovery["handledByClientId"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Failure::Protocol("Desktop owner identity".into()))?
        .to_string();
    connection.renew(timeout);
    let mut wrote = false;
    let response = connection
        .request(
            "thread-follower-interrupt-turn",
            version,
            &client,
            Some(&owner),
            params,
            Some(&mut || wrote = true),
        )
        .map_err(|error| match &error {
            IpcError::Rejected(reason)
                if !matches!(
                    reason.as_str(),
                    "request-timeout" | "server-closed" | "client-disconnected"
                ) =>
            {
                Failure::from(error)
            }
            _ if wrote => Failure::AcceptanceUncertain,
            _ => Failure::from(error),
        })?;
    let (turn, warning) = parse_interrupt_response(&response, &owner, expected)?;
    let mut receipt = super::desktop::base_receipt(
        target,
        request,
        DeliveryState::Interrupted,
        Some(turn),
        Some(owner),
    );
    receipt.reason = warning;
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_operations_include_interrupt() {
        assert!(OPERATIONS.contains(&Operation::Interrupt));
    }

    #[test]
    fn interrupt_request_selects_wire_version_by_expected_turn() {
        let (v3, params3) = interrupt_request_args("thread-1", None).unwrap();
        assert_eq!(v3, 3);
        assert_eq!(
            params3,
            json!({"conversationId": "thread-1", "mode": "user-stop"})
        );

        let (v4, params4) = interrupt_request_args("thread-1", Some("turn-9")).unwrap();
        assert_eq!(v4, 4);
        assert_eq!(
            params4,
            json!({"conversationId": "thread-1", "mode": "user-stop", "expectedTurnId": "turn-9"})
        );

        assert!(matches!(
            interrupt_request_args("thread-1", Some("")),
            Err(Failure::InvalidInput(_))
        ));
    }

    #[test]
    fn parse_interrupt_response_validates_owner_and_turn_id() {
        let ok = json!({
            "handledByClientId": "owner-1",
            "result": {"result": {"ok": true, "interruptedTurnId": "turn-9"}}
        });
        assert_eq!(
            parse_interrupt_response(&ok, "owner-1", Some("turn-9")).unwrap(),
            ("turn-9".to_owned(), None)
        );

        let with_warning = json!({
            "handledByClientId": "owner-1",
            "result": {"result": {"ok": true, "interruptedTurnId": "turn-9", "goalPauseError": "goal busy"}}
        });
        assert_eq!(
            parse_interrupt_response(&with_warning, "owner-1", None).unwrap(),
            ("turn-9".to_owned(), Some("goal busy".to_owned()))
        );

        let idle = json!({
            "handledByClientId": "owner-1",
            "result": {"result": {"ok": true, "interruptedTurnId": null}}
        });
        assert!(matches!(
            parse_interrupt_response(&idle, "owner-1", None),
            Err(Failure::Rejected(_))
        ));

        let wrong_owner = json!({
            "handledByClientId": "other",
            "result": {"result": {"ok": true, "interruptedTurnId": "turn-9"}}
        });
        assert!(matches!(
            parse_interrupt_response(&wrong_owner, "owner-1", None),
            Err(Failure::AcceptanceUncertain)
        ));
    }
}
