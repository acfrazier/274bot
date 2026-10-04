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

/// Consume the published launch, never a later hit-bar mask. `t1` identifies the
/// projectile but marks flight start (41 cycles after a bow launch, 32 thrown).
/// The client records the source tile, not the firing player index; a co-located
/// player therefore makes a tile-matched launch ambiguous.
fn another_player_at_tile(
    mut players: impl Iterator<Item = (usize, api::snapshot::WorldTile)>,
    local_index: usize,
    here: api::snapshot::WorldTile,
) -> bool {
    players.any(|(index, tile)| index != local_index && tile == here)
}

pub fn ambiguous_shooter(frame: &Frame<'_>) -> bool {
    another_player_at_tile(
        frame
            .players
            .iter()
            .map(|player| (player.index, player.network)),
        frame.me(),
        frame.local.player.network,
    )
}

pub fn onset(frame: &Frame<'_>, engaged: Option<ActorRef>, seen: &mut i32) -> bool {
    // Waiting for `t1` itself would lose launches whose shooter moves before
    // flight starts.
    let launch = frame
        .projectiles
        .iter()
        .filter(|projectile| {
            projectile.src == frame.local.player.network
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

    // Consume matching evidence while ambiguous so it cannot be re-attributed
    // after the other player leaves; it is not a confirmed local launch.
    if ambiguous_shooter(frame) {
        if let Some(launch) = launch {
            *seen = launch;
        }
        return false;
    }

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

    fn same_tile_players() -> impl Iterator<Item = (usize, api::snapshot::WorldTile)> {
        let here = api::snapshot::WorldTile {
            x: 2600,
            z: 3370,
            level: 0,
        };
        [(7, here), (12, here)].into_iter()
    }

    #[test]
    fn another_player_on_local_tile_makes_launch_attribution_ambiguous() {
        let here = api::snapshot::WorldTile {
            x: 2600,
            z: 3370,
            level: 0,
        };
        assert!(another_player_at_tile(same_tile_players(), 7, here));
    }

    #[test]
    fn local_index_is_not_treated_as_another_shooter() {
        let here = api::snapshot::WorldTile {
            x: 2600,
            z: 3370,
            level: 0,
        };
        assert!(!another_player_at_tile([(7, here)].into_iter(), 7, here));
        assert!(!another_player_at_tile(
            [(12, api::snapshot::WorldTile { x: 2601, ..here })].into_iter(),
            7,
            here
        ));
    }
}
