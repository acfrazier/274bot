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
    let lower = text.to_ascii_lowercase();
    let Some(at) = lower.find(DEATH_NEEDLE_A) else {
        return false;
    };
    lower[at + DEATH_NEEDLE_A.len()..].contains(DEATH_NEEDLE_B)
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::quest_progress::EvidenceStamp;
    use api::selected::RunKey;
    use api::snapshot::{ChatLineView, GameSnapshot, SnapshotView};

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
        assert!(!latch.observe(view), "duplicate does not relatch");
        assert!(is_death_line("Oh dear, you are dead!"));
        assert!(!is_death_line("oh dear"));
    }
}
