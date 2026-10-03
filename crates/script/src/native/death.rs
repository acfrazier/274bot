//! Shared death-message latch and the content-derived normal respawn square.
use api::snapshot::{SnapshotView, WorldTile};

pub const DEATH_NEEDLE_A: &str = "oh dear";
pub const DEATH_NEEDLE_B: &str = "you are dead";

/// Normal-player respawn area derived from the 289 death continuation in
/// `scripts/player/scripts/death.rs2` (the death message at line 27, the
/// `map_findsquare(0_50_50_21_18, 0, 2, ...)` teleport at line 41, and stat
/// reset at line 52). The Gatherer contract uses a 3-tile square around the
/// content's Lumbridge center, `(3221, 3218, 0)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RespawnSquare {
    center: WorldTile,
    radius: u8,
}

impl RespawnSquare {
    pub fn contains(&self, tile: WorldTile) -> bool {
        tile.level == self.center.level
            && tile.x.abs_diff(self.center.x) <= u32::from(self.radius)
            && tile.z.abs_diff(self.center.z) <= u32::from(self.radius)
    }
}

pub const RESPAWN_SQUARE: RespawnSquare = RespawnSquare {
    center: WorldTile {
        x: 3221,
        z: 3218,
        level: 0,
    },
    radius: 3,
};

#[derive(Default)]
pub struct DeathLatch {
    baseline: bool,
    last_seq: i32,
}

impl DeathLatch {
    pub fn from_watermark(watermark: Option<i32>) -> Self {
        match watermark {
            Some(last_seq) => Self {
                baseline: true,
                last_seq,
            },
            None => Self::default(),
        }
    }

    pub fn watermark(&self) -> Option<i32> {
        self.baseline.then_some(self.last_seq)
    }

    pub fn observe(&mut self, snapshot: SnapshotView<'_>) -> bool {
        let Some(lines) = snapshot.chat_lines(0) else {
            return false;
        };
        self.observe_lines(|| {
            lines
                .value
                .iter()
                .map(|line| (line.sequence, line.text.as_str()))
        })
    }

    /// Observe a borrowed chat ring, shared by native and v1 compatibility
    /// recovery. A lower head reconciles a replacement ring in this pass.
    pub fn observe_lines<'a, I, F>(&mut self, mut lines: F) -> bool
    where
        F: FnMut() -> I,
        I: Iterator<Item = (i32, &'a str)>,
    {
        let max_seq = lines()
            .map(|(sequence, _)| sequence)
            .max()
            .unwrap_or(self.last_seq);
        if !self.baseline {
            self.last_seq = max_seq;
            self.baseline = true;
            return false;
        }

        if max_seq < self.last_seq {
            self.last_seq = 0;
        }
        let mut newest = self.last_seq;
        let mut death = false;
        for (sequence, text) in lines().filter(|(sequence, _)| *sequence > self.last_seq) {
            newest = newest.max(sequence);
            death |= is_death_line(text);
        }
        self.last_seq = newest;
        death
    }
}

pub fn is_death_line(text: &str) -> bool {
    let bytes = text.as_bytes();
    let Some(at) = bytes
        .windows(DEATH_NEEDLE_A.len())
        .position(|part| part.eq_ignore_ascii_case(DEATH_NEEDLE_A.as_bytes()))
    else {
        return false;
    };
    bytes[at + DEATH_NEEDLE_A.len()..]
        .windows(DEATH_NEEDLE_B.len())
        .any(|part| part.eq_ignore_ascii_case(DEATH_NEEDLE_B.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::quest_progress::EvidenceStamp;
    use api::selected::RunKey;
    use api::snapshot::{ChatLineView, GameSnapshot, SnapshotView};

    fn stamp() -> EvidenceStamp {
        EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        }
    }

    fn line(sequence: i32, text: &str) -> ChatLineView {
        ChatLineView {
            type_: 0,
            username: None,
            text: text.into(),
            sequence,
        }
    }

    fn snapshot(lines: Vec<ChatLineView>) -> GameSnapshot {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_chat_lines(lines);
        snapshot
    }

    #[test]
    fn only_a_new_death_line_latches() {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_chat_lines(vec![ChatLineView {
            type_: 0,
            username: None,
            text: "Oh dear, you are dead!".into(),
            sequence: 1,
        }]);
        let stamp = EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        };
        let view = SnapshotView::new(Some(&snapshot), stamp);
        let mut latch = DeathLatch::default();
        assert!(!latch.observe(view), "seeded line is baseline");
        assert_eq!(latch.watermark(), Some(1));
        snapshot.seed_chat_lines(vec![
            ChatLineView {
                type_: 0,
                username: None,
                text: "Oh dear, you are dead!".into(),
                sequence: 2,
            },
            ChatLineView {
                type_: 0,
                username: None,
                text: "Oh dear, you are dead!".into(),
                sequence: 1,
            },
        ]);
        let view = SnapshotView::new(Some(&snapshot), stamp);
        assert!(latch.observe(view), "new sequence latches");
        assert_eq!(latch.watermark(), Some(2));
        assert!(!latch.observe(view), "duplicate does not relatch");
        assert!(is_death_line("Oh dear, you are dead!"));
        assert!(!is_death_line("oh dear"));
    }

    #[test]
    fn watermark_none_and_initialized_empty_distinguish_baselines() {
        let empty = snapshot(Vec::new());
        let mut fresh = DeathLatch::from_watermark(None);
        assert!(!fresh.observe(SnapshotView::new(Some(&empty), stamp())));
        assert_eq!(fresh.watermark(), Some(0));

        let gap = snapshot(vec![line(1, "Oh dear, you are dead!")]);
        let mut recreated = DeathLatch::from_watermark(Some(0));
        assert!(recreated.observe(SnapshotView::new(Some(&gap), stamp())));
        assert_eq!(recreated.watermark(), Some(1));

        let newer = snapshot(vec![line(5, "Welcome back")]);
        let mut advanced = DeathLatch::from_watermark(Some(4));
        assert!(!advanced.observe(SnapshotView::new(Some(&newer), stamp())));
        assert_eq!(advanced.watermark(), Some(5));
        assert!(!advanced.observe(SnapshotView::new(Some(&newer), stamp())));
        assert_eq!(advanced.watermark(), Some(5));
    }

    #[test]
    fn head_regression_reconciles_the_replacement_ring_in_place() {
        let death_ring = snapshot(vec![
            line(2, "Oh dear, you are dead!"),
            line(1, "Welcome back"),
        ]);
        let mut death = DeathLatch::from_watermark(Some(57));
        assert!(death.observe(SnapshotView::new(Some(&death_ring), stamp())));
        assert_eq!(death.watermark(), Some(2));

        let nondeath_ring = snapshot(vec![line(1, "Welcome back")]);
        let mut nondeath = DeathLatch::from_watermark(Some(57));
        assert!(!nondeath.observe(SnapshotView::new(Some(&nondeath_ring), stamp())));
        assert_eq!(nondeath.watermark(), Some(1));
    }

    #[test]
    fn respawn_square_contains_the_contract_bounds_on_its_plane() {
        assert!(RESPAWN_SQUARE.contains(WorldTile {
            x: 3224,
            z: 3221,
            level: 0,
        }));
        assert!(RESPAWN_SQUARE.contains(WorldTile {
            x: 3218,
            z: 3215,
            level: 0,
        }));
        assert!(!RESPAWN_SQUARE.contains(WorldTile {
            x: 3225,
            z: 3218,
            level: 0,
        }));
        assert!(!RESPAWN_SQUARE.contains(WorldTile {
            x: 3221,
            z: 3218,
            level: 1,
        }));
    }
}
