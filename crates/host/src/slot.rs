//! Per-slot drain pump: diffs `Client.gens` across 20 ms frames to
//! synthesize `on_server_tick` and to mark which snapshot families changed.
//!
//! A real `Client` is too heavy to construct in host unit tests (it unpacks
//! the cache and connects), so the pump is tested against `ClientGens`
//! snapshots directly; the slot thread in `lib.rs` feeds it real `gens`.

use client::client::ClientGens;

/// Outcome of one drain (one 20 ms frame): whether a `PLAYER_INFO` packet
/// applied this frame, the generation snapshot the frame ended on, and the
/// families that moved **this** drain (computed before `last` is committed).
#[derive(Clone, Copy)]
pub struct DrainResult {
    pub player_info: bool,
    pub gens: ClientGens,
    pub dirty: DirtyFamilies,
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
    pub iface: bool,
    pub camera: bool,
    pub map_flag: bool,
    pub world: bool,
}

impl DirtyFamilies {
    /// True if any family gen moved.
    pub fn any(self) -> bool {
        self.npc
            || self.player
            || self.inv
            || self.varp
            || self.stat
            || self.chat
            || self.scene
            || self.iface
            || self.camera
            || self.map_flag
            || self.world
    }
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
        iface: last.iface != current.iface,
        camera: last.camera != current.camera,
        map_flag: last.map_flag != current.map_flag,
        world: last.world != current.world,
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
    /// `dirty` is computed **before** committing `last` so callers can
    /// rebuild from the returned value after drain.
    pub fn drain(&mut self, gens: ClientGens) -> DrainResult {
        let dirty = dirty_families(self.last, gens);
        let player_info = gens.player != self.last.player;
        self.last = gens;
        DrainResult {
            player_info,
            gens,
            dirty,
        }
    }

    /// The snapshot committed by the most recent [`Pump::drain`].
    pub fn last(&self) -> ClientGens {
        self.last
    }

    /// Families whose gen moved versus uncommitted `gens`. Call **before**
    /// `drain`. After `drain` has committed `last`, this is empty — use
    /// [`DrainResult::dirty`] instead.
    pub fn dirty(&self, gens: ClientGens) -> DirtyFamilies {
        dirty_families(self.last, gens)
    }
}

#[cfg(test)]
mod tests {
    use client::client::ClientGens;

    use super::{dirty_families, should_emit_tick, DirtyFamilies, Pump};

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
        assert!(!dirty.iface);
        assert!(!dirty.camera);
        assert!(!dirty.map_flag);
        assert!(!dirty.world);

        let mut pump = Pump::new();
        let result = pump.drain(after);
        assert!(!should_emit_tick(result.player_info));
        assert!(
            result.dirty.npc,
            "post-drain DrainResult.dirty must see the npc bump"
        );
        assert!(!result.dirty.player);
        assert_eq!(
            pump.dirty(after),
            DirtyFamilies::default(),
            "Pump::dirty() after drain is the empty-trap; use DrainResult.dirty"
        );
    }

    #[test]
    fn new_family_gens_mark_their_families_dirty() {
        let before = gens();
        let mut after = before;
        after.iface = 1;
        after.camera = 1;
        after.map_flag = 1;
        after.world = 1;

        let dirty = dirty_families(before, after);
        assert!(dirty.iface, "iface gen moved: iface family must be dirty");
        assert!(
            dirty.camera,
            "camera gen moved: camera family must be dirty"
        );
        assert!(
            dirty.map_flag,
            "map_flag gen moved: map_flag family must be dirty"
        );
        assert!(dirty.world, "world gen moved: world family must be dirty");
        assert!(dirty.any(), "any() must cover the four new families");
        assert!(!dirty.npc);
        assert!(!dirty.player);
        assert!(!dirty.inv);
        assert!(!dirty.varp);
        assert!(!dirty.stat);
        assert!(!dirty.chat);
        assert!(!dirty.scene);
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
        assert!(result.dirty.npc);
        assert!(!result.dirty.player);
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
        assert_eq!(result.dirty, DirtyFamilies::default());
    }
}
