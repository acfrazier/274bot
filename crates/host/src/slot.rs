//! Per-slot drain pump: diffs `Client.gens` across 20 ms frames to
//! synthesize `on_server_tick` and to mark which snapshot families changed.
//!
//! A real `Client` is too heavy to construct in host unit tests (it unpacks
//! the cache and connects), so the pump is tested against `ClientGens`
//! snapshots directly; the slot thread in `lib.rs` feeds it real `gens`.

use client::client::ClientGens;

/// Outcome of one drain (one 20 ms frame): whether a `PLAYER_INFO` packet
/// applied this frame and the generation snapshot the frame ended on.
#[derive(Clone, Copy)]
pub struct DrainResult {
    pub player_info: bool,
    pub gens: ClientGens,
}

/// A `PLAYER_INFO` applied this drain — synthesize `on_server_tick` (274 has
/// no tick-end packet; the player gen bump is the tick edge).
pub fn should_emit_tick(player_info_this_drain: bool) -> bool {
    player_info_this_drain
}

/// Snapshot families; each field is true when that family's gen moved
/// between two snapshots.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DirtyFamilies {
    pub npc: bool,
    pub player: bool,
    pub inv: bool,
    pub varp: bool,
    pub stat: bool,
    pub chat: bool,
    pub scene: bool,
}

/// Families whose generation moved from `last` to `current`.
pub fn dirty_families(last: ClientGens, current: ClientGens) -> DirtyFamilies {
    DirtyFamilies {
        npc: last.npc != current.npc,
        player: last.player != current.player,
        inv: last.inv != current.inv,
        varp: last.varp != current.varp,
        stat: last.stat != current.stat,
        chat: last.chat != current.chat,
        scene: last.scene != current.scene,
    }
}

/// Per-slot generation tracker. One per slot thread; `drain` is fed the
/// client's `gens` after each `mainloop` pass and commits the new snapshot.
#[derive(Default)]
pub struct Pump {
    last: ClientGens,
}

impl Pump {
    pub fn new() -> Self {
        Self::default()
    }

    /// Diff `gens` against the last snapshot, commit it, and report the
    /// drain result. `player_info` is true iff the player gen moved.
    pub fn drain(&mut self, gens: ClientGens) -> DrainResult {
        let player_info = gens.player != self.last.player;
        self.last = gens;
        DrainResult { player_info, gens }
    }

    /// The snapshot committed by the most recent [`Pump::drain`].
    pub fn last(&self) -> ClientGens {
        self.last
    }

    /// Families whose gen moved since the last committed snapshot.
    pub fn dirty(&self, gens: ClientGens) -> DirtyFamilies {
        dirty_families(self.last, gens)
    }
}

#[cfg(test)]
mod tests {
    use client::client::ClientGens;

    use super::{DirtyFamilies, Pump, dirty_families, should_emit_tick};

    fn gens() -> ClientGens {
        ClientGens::default()
    }

    #[test]
    fn quiet_frame_emits_no_tick() {
        let mut pump = Pump::new();
        let result = pump.drain(gens());
        assert!(!should_emit_tick(result.player_info));
    }

    #[test]
    fn player_info_drain_emits_tick() {
        let mut g = gens();
        g.player = 1;
        let mut pump = Pump::new();
        let result = pump.drain(g);
        assert!(should_emit_tick(result.player_info));
        assert_eq!(result.gens.player, 1);
    }

    #[test]
    fn npc_only_drain_marks_npc_family_dirty_without_tick() {
        let before = gens();
        let mut after = before;
        after.npc = 1;

        let dirty = dirty_families(before, after);
        assert!(dirty.npc, "NPC gen moved: npc family must be dirty");
        assert!(!dirty.player);
        assert!(!dirty.inv);
        assert!(!dirty.varp);
        assert!(!dirty.stat);
        assert!(!dirty.chat);
        assert!(!dirty.scene);

        let mut pump = Pump::new();
        let result = pump.drain(after);
        assert!(!should_emit_tick(result.player_info));
    }

    #[test]
    fn player_gen_since_last_drain_is_the_tick_edge() {
        let mut pump = Pump::new();
        let mut g = gens();
        g.player = 1;
        assert!(should_emit_tick(pump.drain(g).player_info));

        // NPC gen moves this drain; player did not: no tick, npc dirty.
        let mut quiet = g;
        quiet.npc = 1;
        let result = pump.drain(quiet);
        assert!(!should_emit_tick(result.player_info));
        let dirty = dirty_families(g, quiet);
        assert!(dirty.npc);
        assert!(!dirty.player);
    }

    #[test]
    fn unchanged_gens_after_tick_emit_no_second_tick() {
        let mut pump = Pump::new();
        let mut g = gens();
        g.player = 1;
        pump.drain(g);
        // Same snapshot again: no new tick, no dirty families.
        let result = pump.drain(g);
        assert!(!should_emit_tick(result.player_info));
        assert_eq!(dirty_families(result.gens, g), DirtyFamilies::default());
    }
}
