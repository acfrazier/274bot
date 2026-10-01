//! Death latch: only a *new* "Oh dear, you are dead!" line latches.
use api::snapshot::SnapshotView;

pub const DEATH_NEEDLE_A: &str = "oh dear";
pub const DEATH_NEEDLE_B: &str = "you are dead";

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
        let max_seq = lines
            .value
            .iter()
            .map(|line| line.sequence)
            .max()
            .unwrap_or(self.last_seq);
        if !self.baseline {
            self.last_seq = max_seq;
            self.baseline = true;
            return false;
        }

        // A lower head means the client/ring was replaced. Reconcile the new
        // ring in this same observation rather than waiting for another tick.
        if max_seq < self.last_seq {
            self.last_seq = 0;
        }
        let mut newest = self.last_seq;
        let mut death = false;
        for line in lines
            .value
            .iter()
            .filter(|line| line.sequence > self.last_seq)
        {
            newest = newest.max(line.sequence);
            death |= is_death_line(&line.text);
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
}
