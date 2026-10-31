//! Fixture-only escape from an engine combat logout refusal, after script Stop.
//!
//! R289 `content/scripts/login_logout/logout.rs2:26-30` locks logout for 16
//! game ticks. `skill_combat/scripts/player/auto_retaliate.rs2:1-7` renews it
//! on NPC hits even when the bot script has stopped. Only a fresh exact system
//! refusal permits this helper to move its own account to the mainland landing
//! tile. It never changes the world, HP, NPCs, or the completed cell's result.

use std::time::Instant;

use api::interact::{cheat, tele_args, Driver};
use api::snapshot::{GameSnapshot, WorldTile};

pub const COMBAT_LOGOUT_TICKS: u32 = 16;
pub const COMBAT_LOGOUT_REFUSAL: &str =
    "You can't log out until 10 seconds after the end of combat.";
pub const LOGOUT_SAFE_TILE: WorldTile = WorldTile {
    x: 3220,
    z: 3220,
    level: 0,
};

#[derive(Debug)]
enum Escape {
    Unneeded,
    Pending,
    Sent,
    Arrived(u32),
}

/// Construct after Stop acknowledgement and immediately before the first logout.
/// The caller retains its original deadline and logout retry schedule. Both
/// TUI live cells and the Quester host cells use this same escape gate.
#[derive(Debug)]
pub struct CombatLogout {
    chat_baseline: i32,
    refusal_sequence: Option<i32>,
    escape: Escape,
    deadline: Instant,
}

impl CombatLogout {
    pub fn new(snapshot: &GameSnapshot, deadline: Instant) -> Self {
        Self {
            chat_baseline: crate::tutorial::chat_baseline(snapshot),
            refusal_sequence: None,
            escape: Escape::Unneeded,
            deadline,
        }
    }

    /// Observe the engine reply and the actual safe-tile arrival. Render polls
    /// and wall time cannot consume any of the content's 16 game ticks.
    pub fn allows_retry(&mut self, snapshot: &GameSnapshot, now: Instant) -> bool {
        if now >= self.deadline {
            return false;
        }
        if matches!(self.escape, Escape::Sent)
            && snapshot.tile() == Some((LOGOUT_SAFE_TILE.x, LOGOUT_SAFE_TILE.z, LOGOUT_SAFE_TILE.level))
        {
            self.escape = Escape::Arrived(snapshot.tick());
            eprintln!(
                "[live-logout] safe tile observed tick={} wait_game_ticks={COMBAT_LOGOUT_TICKS}",
                snapshot.tick()
            );
        }
        match self.escape {
            Escape::Unneeded => true,
            Escape::Pending | Escape::Sent => false,
            Escape::Arrived(tick) => snapshot.tick().wrapping_sub(tick) >= COMBAT_LOGOUT_TICKS,
        }
    }

    /// Run only on this account's client thread. A refused cheat is a fixture
    /// failure, not permission to bypass the provider's admission check.
    pub fn send_escape<D: Driver>(&mut self, driver: &mut D, now: Instant) -> Result<bool, String> {
        if now >= self.deadline || !matches!(self.escape, Escape::Pending) {
            return Ok(false);
        }
        let sent = cheat(
            driver,
            &tele_args(LOGOUT_SAFE_TILE.level, LOGOUT_SAFE_TILE.x, LOGOUT_SAFE_TILE.z),
        );
        if !sent.is_sent() {
            return Err(format!("fixture combat logout escape refused: {sent:?}"));
        }
        self.escape = Escape::Sent;
        Ok(true)
    }

    pub fn refusal_sequence(&self) -> Option<i32> {
        self.refusal_sequence
    }

    pub fn escape_sent(&self) -> bool {
        matches!(self.escape, Escape::Sent | Escape::Arrived(_))
    }

    pub fn safe_arrival_tick(&self) -> Option<u32> {
        match self.escape {
            Escape::Arrived(tick) => Some(tick),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::snapshot::ChatLineView;
    use std::time::Duration;

    fn refusal(sequence: i32, type_: i32) -> ChatLineView {
        ChatLineView {
            type_,
            username: None,
            text: COMBAT_LOGOUT_REFUSAL.to_owned(),
            sequence,
        }
    }

    #[test]
    fn fresh_combat_refusal_blocks_retry_until_escape_and_sixteen_observed_ticks() {
        let now = Instant::now();
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_tick(29);
        let mut logout = CombatLogout::new(&snapshot, now + Duration::from_secs(30));
        snapshot.seed_chat_lines(vec![refusal(11, 0)]);
        snapshot.seed_tick(31);
        assert!(!logout.allows_retry(&snapshot, now));
        assert!(matches!(logout.escape, Escape::Pending));
        assert_eq!(logout.refusal_sequence(), Some(11));
        snapshot.seed_tick(47);
        assert!(!logout.allows_retry(&snapshot, now));
        // Successful account-only send is followed by observed arrival, not a
        // guessed elapsed wait from the request or the last failed logout.
        logout.escape = Escape::Sent;
        assert!(!logout.allows_retry(&snapshot, now));
        snapshot.seed_tile(LOGOUT_SAFE_TILE);
        assert!(!logout.allows_retry(&snapshot, now));
        assert_eq!(logout.safe_arrival_tick(), Some(47));
        for tick in [47, 47, 48, 62] {
            snapshot.seed_tick(tick);
            assert!(!logout.allows_retry(&snapshot, now));
        }
        snapshot.seed_tick(63);
        assert!(logout.allows_retry(&snapshot, now));
    }

    #[test]
    fn stale_player_and_unrelated_chat_do_not_authorize_fixture_teleport() {
        let now = Instant::now();
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_chat_lines(vec![refusal(10, 0)]);
        let mut logout = CombatLogout::new(&snapshot, now + Duration::from_secs(30));
        for lines in [
            vec![refusal(10, 0)],
            vec![refusal(11, 2)],
            vec![ChatLineView {
                text: "You can't log out during a duel.".to_owned(),
                ..refusal(12, 0)
            }],
        ] {
            snapshot.seed_chat_lines(lines);
            assert!(logout.allows_retry(&snapshot, now));
            assert!(!logout.escape_sent());
            assert_eq!(logout.refusal_sequence(), None);
        }
    }

    #[test]
    fn escape_wait_keeps_the_original_deadline_and_handles_tick_wrap() {
        let now = Instant::now();
        let deadline = now + Duration::from_secs(30);
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_tile(LOGOUT_SAFE_TILE);
        snapshot.seed_tick(u32::MAX - 7);
        let mut logout = CombatLogout::new(&snapshot, deadline);
        logout.escape = Escape::Sent;
        assert!(!logout.allows_retry(&snapshot, now));
        snapshot.seed_tick(7);
        assert!(!logout.allows_retry(&snapshot, now));
        snapshot.seed_tick(8);
        assert!(logout.allows_retry(&snapshot, now));
        assert!(!logout.allows_retry(&snapshot, deadline));
        assert_eq!(logout.deadline, deadline);
    }
}
