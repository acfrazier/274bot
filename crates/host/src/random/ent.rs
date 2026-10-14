//! Stop forced Ent chopping until the tree returns.
//!
//! `macro_event_ent.rs2:3-20` replaces the chopped loc with an immobile
//! `macro_ent_*` NPC for 60 ticks and calls `p_opnpc(1)`. Six swings break
//! the axe (:127-150). Identify the type, not its ordinary tree name.
//! An ordinary host walk cancels that interaction:
//! `MoveClickHandler.ts:26-32` → `Player.ts:969-972` → `clearInteraction`.
//! Holding the script prevents it from selecting the Ent again; slot removal
//! or type change releases it to select the restored tree.

use api::ent::is_ent_npc_id;
use api::game_data::SelectedGameData;
use api::interact::{walk, Driver};
use api::query::npc_by_index;
use api::snapshot::{ActorKind, GameSnapshot};

/// Size-3 footprint approach + two engine running steps per tick.
const ENT_OWNERSHIP_RADIUS: i32 = 5;

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
    stop_sent: bool,
}

impl EntGuard {
    /// Idle guard, no latch and no stop outstanding.
    pub(super) fn new() -> Self {
        Self {
            active: None,
            stop_sent: false,
        }
    }

    /// Retain the exact slot until reversion; only fresh ticks send or retry.
    pub(super) fn tick<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        game_data: Option<&SelectedGameData>,
        enabled: bool,
        fresh: bool,
    ) -> bool {
        if !enabled {
            self.active = None;
            self.stop_sent = false;
            return false;
        }
        if let Some(latch) = self.active.take() {
            if let Some(npc) = npc_by_index(snap.npcs(), latch.index) {
                if npc.r#type == Some(latch.npc_type) && is_ent_slot(npc.r#type, game_data) {
                    let tile = (npc.tile.x, npc.tile.z);
                    self.active = Some(EntSight {
                        index: latch.index,
                        npc_type: latch.npc_type,
                        tile,
                    });
                    if !self.stop_sent && fresh && stop_step(driver, snap, tile) {
                        self.stop_sent = true;
                    }
                    return true;
                }
            }
            self.active = None;
            self.stop_sent = false;
        }
        let Some(seen) = detect_ent(snap, game_data) else {
            return false;
        };
        let tile = seen.tile;
        self.active = Some(seen);
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

/// Prefer the content's `p_opnpc` player target; proximity covers the spawn
/// packet before facing updates. Size-3 approach plus two running steps
/// gives five tiles (`antimacro.npc:1285`; `PathingEntity.ts:135-152`).
fn detect_ent(snap: &GameSnapshot, game_data: Option<&SelectedGameData>) -> Option<EntSight> {
    let (px, pz, _) = snap.tile()?;
    let player_target = snap.local_player().and_then(|lp| lp.player.actor.target);
    let mut best: Option<EntSight> = None;
    let mut best_dist = ENT_OWNERSHIP_RADIUS + 1;
    for npc in snap.npcs() {
        let Some(type_id) = npc.r#type else {
            continue;
        };
        if !is_ent_slot(Some(type_id), game_data) {
            continue;
        }
        if player_target.is_some_and(|t| t.kind == ActorKind::Npc && t.index == npc.index) {
            return Some(EntSight {
                index: npc.index,
                npc_type: type_id,
                tile: (npc.tile.x, npc.tile.z),
            });
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
