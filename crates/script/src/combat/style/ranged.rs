//! Ranged-only mechanics; upkeep and action admission belong to the shared core.
use crate::combat::frame::Frame;
use crate::combat::request::{ActorRef, RangedMode};
use crate::combat::tables::CombatTables;
use api::game_data::{RangedAmmoFamily, RangedWeaponFact};

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

pub fn range(base: u8, mode: RangedMode) -> u8 {
    base.min(10)
        .saturating_add(if mode == RangedMode::LongRange { 2 } else { 0 })
        .min(10)
}

/// Consume the published launch, never a later hit-bar mask. `t1` identifies the
/// projectile but marks flight start (41 cycles after a bow launch, 32 thrown).
/// Waiting for `t1` would lose launches whose shooter moves before flight starts.
pub fn onset(frame: &Frame<'_>, engaged: Option<ActorRef>, seen: &mut i32) -> bool {
    let launch = frame
        .projectiles
        .iter()
        .filter(|projectile| {
            projectile.src == frame.here
                && engaged.is_some_and(|actor| {
                    projectile
                        .target
                        .is_some_and(|target| actor.matches(target))
                })
                && projectile.t1.saturating_sub(frame.loop_cycle) <= 41
                && frame.loop_cycle.saturating_sub(projectile.t1) <= 60
                && (*seen < 0 || projectile.t1.wrapping_sub(*seen) > 0)
        })
        .map(|projectile| projectile.t1)
        .max();
    if let Some(launch) = launch {
        *seen = launch;
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rapid_and_long_range_respect_engine_limits() {
        assert_eq!(rate(4, RangedMode::Rapid), 3);
        assert_eq!(rate(3, RangedMode::Rapid), 2);
        assert_eq!(rate(4, RangedMode::Accurate), 4);
        assert_eq!(range(7, RangedMode::LongRange), 9);
        assert_eq!(range(10, RangedMode::LongRange), 10);
        assert_eq!(range(12, RangedMode::Rapid), 10);
    }
}
