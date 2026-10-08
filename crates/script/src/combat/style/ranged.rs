//! Ranged-only mechanics; upkeep and action admission belong to the shared core.
use crate::combat::request::{ActorRef, RangedMode};
use crate::combat::tables::{CombatTables, StyleMask};
use api::game_data::{RangedAmmoFamily, RangedWeaponFact};
use api::snapshot::ActorView;

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

/// Consume the local player's selected ranged attack animation. Rewinds in
/// the same sequence mark a restarted swing; an animation aimed at another
/// actor is excluded just as it is for melee.
pub fn onset(
    local: &ActorView,
    engaged: Option<ActorRef>,
    tables: &CombatTables,
    last_id: &mut i32,
    last_frame: &mut i32,
) -> bool {
    let animation = local.animation;
    let frame = local.animation_frame;
    let changed = animation != *last_id || (animation == *last_id && frame < *last_frame);
    *last_id = animation;
    *last_frame = frame;
    if !changed || animation < 0 {
        return false;
    }
    if local
        .target
        .is_some_and(|target| !engaged.is_some_and(|actor| actor.matches(target)))
    {
        return false;
    }
    tables
        .style_seq(animation)
        .is_some_and(|mask| mask.contains(StyleMask::RANGED))
}
