//! Shared sending and fresh confirmation for the world-wide tick-speed cheat.

use api::interact::cheat;
use api::snapshot::GameSnapshot;
use client::client::Client;

/// Capture the chat sequence that precedes a world-speed command.
///
/// The runner records this before it sends, so the confirm step accepts only
/// a confirmation with a greater sequence. A line already in the ring — an
/// earlier run of this client, or the confirmation of an earlier request —
/// can never confirm a send made now.
pub(crate) fn chat_baseline(snapshot: &GameSnapshot) -> i32 {
    snapshot
        .chat_lines()
        .first()
        .map_or(0, |line| line.sequence)
}

/// Send one `::speed <ms>` command. The caller records [`chat_baseline`]
/// first and sends only when a scenario explicitly requests a tick.
pub(crate) fn send_tick_speed(client: &mut Client, expected_ms: u32) -> Result<(), String> {
    if !cheat(client, &format!("speed {expected_ms}")).is_sent() {
        return Err("world speed command was refused".into());
    }
    Ok(())
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

    #[test]
    fn chat_baseline_is_the_newest_chat_sequence() {
        let mut snapshot = GameSnapshot::new();
        assert_eq!(chat_baseline(&snapshot), 0, "an empty ring marks zero");
        snapshot.seed_chat_lines(vec![
            ChatLineView {
                type_: 0,
                username: None,
                text: "newest".into(),
                sequence: 21,
            },
            ChatLineView {
                type_: 0,
                username: None,
                text: "older".into(),
                sequence: 20,
            },
        ]);
        assert_eq!(chat_baseline(&snapshot), 21);
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
