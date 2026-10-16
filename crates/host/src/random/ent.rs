//! Stop forced Ent chopping without claiming a nearby Ent known to belong to
//! another player. `macro_event_ent.rs2:18` adds the NPC for 60 ticks and
//! `:20` calls `p_opnpc(1)`. `PlayerOps.ts:404-415` stores that interaction;
//! `PathingEntity.ts:514-531` exposes it as `face_entity`, which the snapshot
//! decodes at `decode.rs:1397-1410`. Until that FACE_ENTITY update reaches the
//! client, ownership is not observable: the five-tile fallback is a bounded
//! guess, not proof. Cap every hold at the content's 60 ticks and never
//! reacquire an exhausted visible slot.
//!
//! An ordinary host walk cancels the interaction:
//! `MoveClickHandler.ts:26-32` → `Player.ts:969-972` → `clearInteraction`.
//! Holding the script prevents it from selecting the Ent again; slot removal
//! or type change releases it to select the restored tree.

use api::ent::is_ent_npc_id;
use api::game_data::SelectedGameData;
use api::interact::{walk, Driver};
use api::query::npc_by_index;
use api::snapshot::{ActorKind, GameSnapshot};

/// Size-3 footprint approach (`antimacro.npc:1285,1325`) plus the two engine
/// running steps in `PathingEntity.ts:144-148`; a fallback range, not ownership.
const ENT_OWNERSHIP_RADIUS: i32 = 5;
/// Content `macro_event_ent.rs2:18` gives each Ent exactly 60 ticks.
const MAX_ENT_HOLD_TICKS: u8 = 60;

/// One latched Ent: the client NPC slot, its cache type id, and its current
/// world tile. No allocation; the display name is deliberately not stored
/// (it is a tree name, not an identity).
#[derive(Debug, PartialEq, Eq)]
struct EntSight {
    index: usize,
    npc_type: usize,
    tile: (i32, i32),
}

pub(super) struct EntGuard {
    active: Option<EntSight>,
    active_ticks: u8,
    /// An expired slot stays excluded until the client observes its reversion
    /// or removal; otherwise the proximity fallback could immediately relatch it.
    expired: Option<(usize, usize)>,
    stop_sent: bool,
}

impl EntGuard {
    /// Idle guard, no latch and no stop outstanding.
    pub(super) fn new() -> Self {
        Self {
            active: None,
            active_ticks: 0,
            expired: None,
            stop_sent: false,
        }
    }

    /// Retain the exact slot until reversion or the 60-tick content bound;
    /// only fresh ticks send or retry.
    pub(super) fn tick<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        game_data: Option<&SelectedGameData>,
        enabled: bool,
        fresh: bool,
    ) -> bool {
        if let Some((index, npc_type)) = self.expired {
            let still_visible = npc_by_index(snap.npcs(), index).is_some_and(|npc| {
                npc.r#type == Some(npc_type) && is_ent_slot(npc.r#type, game_data)
            });
            if still_visible {
                self.stop_sent = false;
                return false;
            }
            self.expired = None;
        }

        if let Some(latch) = self.active.take() {
            if let Some(npc) = npc_by_index(snap.npcs(), latch.index) {
                if npc.r#type == Some(latch.npc_type) && is_ent_slot(npc.r#type, game_data) {
                    let player_target = snap.local_player().and_then(|lp| lp.player.actor.target);
                    let target_conflict = match player_target {
                        Some(target) => {
                            target.kind != ActorKind::Npc || target.index != latch.index
                        }
                        None => another_player_targets_ent(snap, latch.index),
                    };
                    if !target_conflict {
                        if fresh && self.active_ticks >= MAX_ENT_HOLD_TICKS {
                            self.active_ticks = 0;
                            self.expired = Some((latch.index, latch.npc_type));
                            self.stop_sent = false;
                            return false;
                        }
                        if fresh {
                            self.active_ticks = self.active_ticks.saturating_add(1);
                        }
                        let tile = (npc.tile.x, npc.tile.z);
                        self.active = Some(EntSight {
                            index: latch.index,
                            npc_type: latch.npc_type,
                            tile,
                        });
                        if !enabled {
                            self.stop_sent = false;
                            return false;
                        }
                        if !self.stop_sent && fresh && stop_step(driver, snap, tile) {
                            self.stop_sent = true;
                        }
                        return true;
                    }
                }
            }
            self.active_ticks = 0;
            self.stop_sent = false;
        }

        if !enabled {
            self.stop_sent = false;
            return false;
        }
        let Some(seen) = detect_ent(snap, game_data) else {
            return false;
        };
        let tile = seen.tile;
        self.active = Some(seen);
        self.active_ticks = if fresh { 1 } else { 0 };
        if fresh && stop_step(driver, snap, tile) {
            self.stop_sent = true;
        }
        true
    }
}

/// Whether a cache NPC type id is an Ent, plus the selected-content
/// `macro_ent_*` config when game data is available (fail closed on an
/// unbound id). Name never consulted.
fn is_ent_slot(type_id: Option<usize>, game_data: Option<&SelectedGameData>) -> bool {
    let Some(type_id) = type_id else {
        return false;
    };
    let Ok(id_i32) = i32::try_from(type_id) else {
        return false;
    };
    if !is_ent_npc_id(id_i32) {
        return false;
    }
    if let Some(data) = game_data {
        let Some(row) = data.npc_name(id_i32) else {
            return false;
        };
        if !row.config.starts_with("macro_ent_") {
            return false;
        }
    }
    true
}

/// Prefer the content's observable `p_opnpc` target. Proximity only applies
/// while the local target is unknown and no remote player visibly targets the
/// candidate Ent; such a fallback is still only a bounded guess.
fn detect_ent(snap: &GameSnapshot, game_data: Option<&SelectedGameData>) -> Option<EntSight> {
    let (px, pz, _) = snap.tile()?;
    let player_target = snap.local_player().and_then(|lp| lp.player.actor.target);
    if let Some(target) = player_target {
        if target.kind != ActorKind::Npc {
            return None;
        }
        let npc = npc_by_index(snap.npcs(), target.index)?;
        if !is_ent_slot(npc.r#type, game_data) {
            return None;
        }
        return Some(EntSight {
            index: npc.index,
            npc_type: npc.r#type?,
            tile: (npc.tile.x, npc.tile.z),
        });
    }

    let mut best: Option<EntSight> = None;
    let mut best_dist = ENT_OWNERSHIP_RADIUS + 1;
    for npc in snap.npcs() {
        let Some(type_id) = npc.r#type else {
            continue;
        };
        if !is_ent_slot(Some(type_id), game_data) || another_player_targets_ent(snap, npc.index) {
            continue;
        }
        let dist = (npc.tile.x - px).abs().max((npc.tile.z - pz).abs());
        if dist > ENT_OWNERSHIP_RADIUS {
            continue;
        }
        if dist < best_dist {
            best_dist = dist;
            best = Some(EntSight {
                index: npc.index,
                npc_type: type_id,
                tile: (npc.tile.x, npc.tile.z),
            });
        }
    }
    best
}

/// The snapshot includes remote players' FACE_ENTITY targets. If one visibly
/// targets this Ent, a proximity-only local sighting is not our ownership.
fn another_player_targets_ent(snap: &GameSnapshot, ent_index: usize) -> bool {
    snap.players().iter().any(|player| {
        player
            .actor
            .target
            .is_some_and(|target| target.kind == ActorKind::Npc && target.index == ent_index)
    })
}

/// One stop step away from the Ent tile through the player (1 tile, or
/// (+1,0) when stacked). Returns the walk's acceptance; a refusal retries on
/// the next fresh pass. No tile available → false.
fn stop_step<D: Driver>(driver: &mut D, snap: &GameSnapshot, ent: (i32, i32)) -> bool {
    let Some((px, pz, _)) = snap.tile() else {
        return false;
    };
    let dx = (px - ent.0).signum();
    let dz = (pz - ent.1).signum();
    let (tx, tz) = if dx == 0 && dz == 0 {
        (px + 1, pz)
    } else {
        (px + dx, pz + dz)
    };
    walk(driver, tx, tz)
}
