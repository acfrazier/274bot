//! Ranged-only mechanics; upkeep and action admission belong to the shared core.
use crate::combat::request::{ActorRef, RangedMode};
use crate::combat::tables::{CombatTables, StyleMask};
use api::game_data::{RangedAmmoFamily, RangedWeaponFact};
use api::snapshot::LocalPlayerView;

pub fn weapon(tables: &CombatTables, id: i32) -> Option<&RangedWeaponFact> {
    let rows = tables.selected().ranged_weapons();
    rows.binary_search_by_key(&id, |row| row.obj_id)
        .ok()
        .map(|index| &rows[index])
}

pub fn thrown(weapon: &RangedWeaponFact) -> bool {
    matches!(
        weapon.ammo_family,
        RangedAmmoFamily::Thrown | RangedAmmoFamily::Javelin
    )
}

pub fn accepts(tables: &CombatTables, weapon: &RangedWeaponFact, ammo: i32) -> bool {
    if thrown(weapon) {
        return ammo == weapon.obj_id;
    }
    let rows = tables.selected().ranged_ammo();
    rows.binary_search_by_key(&ammo, |row| row.obj_id)
        .ok()
        .is_some_and(|index| {
            let ammo = &rows[index];
            ammo.family == weapon.ammo_family && ammo.levelrequire <= weapon.levelrequire
        })
}

pub fn rate(base: u8, mode: RangedMode) -> u8 {
    if mode == RangedMode::Rapid {
        base.saturating_sub(1).max(1)
    } else {
        base.max(1)
    }
}

/// Consume a new local PLAYER_INFO animation instruction, not rendered frames.
/// The server emits one selected ranged sequence per shot, even when the same
/// sequence is already playing or has visually expired before the snapshot.
pub fn onset(
    local: &LocalPlayerView,
    engaged: Option<ActorRef>,
    tables: &CombatTables,
    last_update: &mut Option<u32>,
) -> bool {
    let Some(update) = local.animation_update else {
        return false;
    };
    let changed = *last_update != Some(update.serial);
    *last_update = Some(update.serial);
    if !changed || update.sequence < 0 {
        return false;
    }
    if local
        .player
        .actor
        .target
        .is_some_and(|target| !engaged.is_some_and(|actor| actor.matches(target)))
    {
        return false;
    }
    tables
        .style_seq(update.sequence)
        .is_some_and(|mask| mask.contains(StyleMask::RANGED))
}
