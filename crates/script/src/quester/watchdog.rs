//! Compact `u64` no-progress watchdog (AIO WARN at 3, PARK at 8).

use api::selected::FactKey;
use api::WorldTile;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchdogAction {
    None,
    Warn,
    Park,
}

#[derive(Default)]
pub struct Watchdog {
    last: u64,
    same: u8,
}

impl Watchdog {
    pub fn unchanged(&self) -> u8 {
        self.same
    }

    pub fn observe(
        &mut self,
        stage: Option<&FactKey>,
        tile: Option<WorldTile>,
        inv: &[(i32, i32)],
        worn: &[i32],
        combat_xp: i32,
    ) -> WatchdogAction {
        let signature = signature(stage, tile, inv, worn, combat_xp);
        if signature == self.last {
            self.same = self.same.saturating_add(1);
        } else {
            self.last = signature;
            self.same = 0;
        }
        match self.same {
            8.. => WatchdogAction::Park,
            3.. => WatchdogAction::Warn,
            _ => WatchdogAction::None,
        }
    }
}

fn signature(
    stage: Option<&FactKey>,
    tile: Option<WorldTile>,
    inv: &[(i32, i32)],
    worn: &[i32],
    combat_xp: i32,
) -> u64 {
    let mut hasher = rustc_hash();
    if let Some(stage) = stage {
        stage.0.hash(&mut hasher);
    }
    if let Some(tile) = tile {
        tile.x.hash(&mut hasher);
        tile.z.hash(&mut hasher);
        tile.level.hash(&mut hasher);
    }
    inv.hash(&mut hasher);
    worn.hash(&mut hasher);
    combat_xp.hash(&mut hasher);
    hasher.finish()
}

fn rustc_hash() -> impl Hasher {
    std::collections::hash_map::DefaultHasher::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::selected::FactKey;

    #[test]
    fn parks_after_eight_unchanged_signatures() {
        let mut dog = Watchdog::default();
        let stage = FactKey::new("cook:1");
        let mut last = WatchdogAction::None;
        for _ in 0..9 {
            last = dog.observe(Some(&stage), None, &[], &[], 0);
        }
        assert_eq!(last, WatchdogAction::Park);
        let mut dog = Watchdog::default();
        let mut last = WatchdogAction::None;
        for _ in 0..4 {
            last = dog.observe(Some(&stage), None, &[(1, 1)], &[], 0);
        }
        assert_eq!(last, WatchdogAction::Warn);
    }

    #[test]
    fn parks_over_an_unchanged_empty_game_snapshot() {
        use api::quest_progress::EvidenceStamp;
        use api::selected::RunKey;
        use api::snapshot::{GameSnapshot, SnapshotView};

        let stamp = EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        };
        let snapshot = GameSnapshot::new();
        let view = SnapshotView::new(Some(&snapshot), stamp);
        let mut inv = [(0, 0); 28];
        let mut inv_len = 0usize;
        if let Some(rows) = view.inventory() {
            for item in rows.value.iter().take(28) {
                inv[inv_len] = (item.def.id, item.count);
                inv_len += 1;
            }
        }
        let mut worn = [0; 14];
        let mut worn_len = 0usize;
        if let Some(rows) = view.equipment() {
            for item in rows.value.iter().take(14) {
                worn[worn_len] = item.def.id;
                worn_len += 1;
            }
        }
        let tile = view.here().map(|obs| obs.value);
        let xp = view
            .stats()
            .map(|stats| stats.value.iter().map(|s| s.xp).sum())
            .unwrap_or(0);
        let mut dog = Watchdog::default();
        let mut last = WatchdogAction::None;
        for _ in 0..9 {
            last = dog.observe(None, tile, &inv[..inv_len], &worn[..worn_len], xp);
        }
        assert_eq!(last, WatchdogAction::Park);
    }
}
