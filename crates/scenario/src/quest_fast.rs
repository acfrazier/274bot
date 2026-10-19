//! Shared sending and fresh confirmation for the world-wide tick-speed cheat.

use api::interact::cheat;
use api::snapshot::GameSnapshot;
use client::client::Client;

/// The cheat body for one world-speed request (`::speed <ms>`).
fn tick_speed_command(ms: u32) -> String {
    format!("speed {ms}")
}

/// The initial world-speed request a scenario asks for, if any.
///
/// A scenario that requests no speed returns `None`. So does a request for
/// the speed the engine's newest exact system-chat confirmation already
/// reports: the command would change nothing. That confirmation is the
/// runner's only runtime statement of the engine's current tick; with none
/// in the chat ring the caller's explicit request is honoured, because
/// Engine Q can be left at a fast rate by an interrupted run.
pub(crate) fn initial_speed_request(
    snapshot: &GameSnapshot,
    engine_speed_ms: Option<u32>,
) -> Option<u32> {
    let requested = engine_speed_ms?;
    match crate::proof::latest_world_speed_confirmation(snapshot) {
        Some((current, _)) if current == requested => None,
        _ => Some(requested),
    }
}

/// Send the initial world-speed request through `send` (the cheat body
/// without the `::` prefix) when the scenario asks for a change. Returns the
/// requested speed and the chat-sequence baseline of the command, or `None`
/// when nothing needed sending.
pub(crate) fn request_initial_tick_speed(
    snapshot: &GameSnapshot,
    engine_speed_ms: Option<u32>,
    send: &mut dyn FnMut(&str) -> bool,
) -> Result<Option<(u32, i32)>, String> {
    let Some(ms) = initial_speed_request(snapshot, engine_speed_ms) else {
        return Ok(None);
    };
    let baseline = send_tick_speed_command(snapshot, ms, send)?;
    Ok(Some((ms, baseline)))
}

/// Send one world-speed command through `send` and capture the chat
/// sequence that precedes it.
fn send_tick_speed_command(
    snapshot: &GameSnapshot,
    expected_ms: u32,
    send: &mut dyn FnMut(&str) -> bool,
) -> Result<i32, String> {
    let baseline = snapshot
        .chat_lines()
        .first()
        .map_or(0, |line| line.sequence);
    if !send(&tick_speed_command(expected_ms)) {
        return Err("world speed command was refused".into());
    }
    Ok(baseline)
}

/// Send one world-speed command and capture the chat sequence that precedes it.
pub(crate) fn send_tick_speed(
    client: &mut Client,
    snapshot: &GameSnapshot,
    expected_ms: u32,
) -> Result<i32, String> {
    send_tick_speed_command(snapshot, expected_ms, &mut |cmd| {
        cheat(client, cmd).is_sent()
    })
}

/// Check for a fresh exact confirmation of a world-speed command.
pub(crate) fn tick_speed_confirmed(
    snapshot: &GameSnapshot,
    baseline: i32,
    expected_ms: u32,
) -> bool {
    crate::proof::world_speed_confirmation_after(snapshot, baseline)
        .is_some_and(|(actual, _)| actual == expected_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::snapshot::ChatLineView;

    fn request_recorded(
        snapshot: &GameSnapshot,
        engine_speed_ms: Option<u32>,
        sent: &mut Vec<String>,
    ) -> Result<Option<(u32, i32)>, String> {
        let mut sink = |cmd: &str| {
            sent.push(format!("::{cmd}"));
            true
        };
        request_initial_tick_speed(snapshot, engine_speed_ms, &mut sink)
    }

    #[test]
    fn initial_speed_request_is_opt_in_and_skips_a_known_matching_speed() {
        let mut snapshot = GameSnapshot::new();
        let mut sent = Vec::new();

        assert_eq!(
            request_recorded(&snapshot, None, &mut sent).unwrap(),
            None,
            "a scenario that does not ask for a speed sends nothing"
        );
        assert!(
            sent.is_empty(),
            "the default path emits no ::speed: {sent:?}"
        );

        assert_eq!(
            request_recorded(&snapshot, Some(300), &mut sent).unwrap(),
            Some((300, 0)),
            "an explicit non-default tick sends one request"
        );
        assert_eq!(sent, vec!["::speed 300".to_string()]);

        sent.clear();
        snapshot.seed_chat_lines(vec![ChatLineView {
            type_: 0,
            username: None,
            text: "World speed was changed to 600ms".into(),
            sequence: 9,
        }]);
        assert_eq!(
            request_recorded(&snapshot, Some(600), &mut sent).unwrap(),
            None,
            "600 explicitly requested on an engine confirmed at 600 sends nothing"
        );
        assert!(
            sent.is_empty(),
            "a matching known speed emits no ::speed: {sent:?}"
        );

        assert_eq!(
            request_recorded(&snapshot, Some(300), &mut sent).unwrap(),
            Some((300, 9)),
            "a request that differs from the confirmed speed is still sent"
        );
        assert_eq!(sent, vec!["::speed 300".to_string()]);
    }

    #[test]
    fn tick_speed_confirmation_requires_a_fresh_exact_system_message() {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_chat_lines(vec![ChatLineView {
            type_: 0,
            username: None,
            text: "World speed was changed to 300ms".into(),
            sequence: 12,
        }]);

        assert!(tick_speed_confirmed(&snapshot, 11, 300));
        assert!(!tick_speed_confirmed(&snapshot, 12, 300));
        assert!(!tick_speed_confirmed(&snapshot, 11, 600));
    }
}
