//! Shared sending and fresh confirmation for the world-wide tick-speed cheat.

use api::interact::cheat;
use api::snapshot::GameSnapshot;
use client::client::Client;

/// Send one world-speed command and capture the chat sequence that precedes it.
pub(crate) fn send_tick_speed(
    client: &mut Client,
    snapshot: &GameSnapshot,
    expected_ms: u32,
) -> Result<i32, String> {
    let baseline = snapshot
        .chat_lines()
        .first()
        .map_or(0, |line| line.sequence);
    if !cheat(client, &format!("speed {expected_ms}")).is_sent() {
        return Err("world speed command was refused".into());
    }
    Ok(baseline)
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
