//! Frozen `COOK_LOCATIONS` / `resolveCookLocation` over the host bank roster
//! and the selected cook surfaces.
//!
//! Frozen `buildCookLocations(BANK_LOCATIONS)` (`data/cookLocations.ts:118-132`)
//! pairs every bank, in catalog order, with a curated camp surface
//! (`curatedPlan`, `:92-116`) or else the nearest placed surface within
//! `MAX_SURFACE_CHEB` of the bank tile on its plane, ovens first
//! (`nearestCookSurface` / `betterSurface`, `:54-78`; `derivePlan`, `:80-89`).
//! `resolveCookLocation` (`api/cooking/CookLocations.ts:25-49`) then picks a
//! named location only when unlocked, or for `Auto` the unlocked one whose
//! bank approach is nearest in a straight x/z line.

use api::cook_locations::{
    CookSurface, CookSurfaceKind, COOK_CAMPS, DERIVED_ARRIVE_RADIUS, MAX_SURFACE_CHEB,
};
use api::named_banks::NamedBank;
use api::snapshot::WorldTile;

/// Frozen `CUSTOM_LOCATION` (`data/cookLocations.ts:14`), lowercased.
pub(crate) const CUSTOM_LOCATION: &str = "custom";

/// Frozen `CookSurfacePlan` (`data/cookLocations.ts:24-37`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CookPlan {
    pub stand: WorldTile,
    pub approach: Option<WorldTile>,
    pub loc_name: String,
    pub kind: CookSurfaceKind,
    pub loc: WorldTile,
    pub arrive_radius: i32,
    pub label: String,
}

/// Frozen `CookLocation` (`data/cookLocations.ts:39-47`); `bank` is the
/// index of the paired bank in the roster it was built from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CookLocation {
    pub bank: usize,
    pub name: &'static str,
    pub surface: Option<CookPlan>,
    pub verified: bool,
}

/// Frozen `curatedPlan` (`data/cookLocations.ts:98-116`).
fn curated(bank: &str) -> Option<CookPlan> {
    let camp = COOK_CAMPS.iter().find(|camp| camp.bank == bank)?;
    Some(CookPlan {
        stand: camp.stand,
        approach: camp.approach,
        loc_name: camp.loc_name.to_string(),
        kind: camp.kind,
        loc: camp.loc,
        arrive_radius: 0,
        label: camp.label.to_string(),
    })
}

/// Frozen `nearestCookSurface(origin, MAX_SURFACE_CHEB)`: same plane, within
/// the Chebyshev radius, an oven beats a fire, else the strictly nearer one.
fn nearest_surface(origin: WorldTile, surfaces: &[CookSurface]) -> Option<&CookSurface> {
    let mut best: Option<(&CookSurface, i32)> = None;
    for surface in surfaces {
        if surface.level != origin.level {
            continue;
        }
        let d = (origin.x - surface.x)
            .abs()
            .max((origin.z - surface.z).abs());
        if d > MAX_SURFACE_CHEB {
            continue;
        }
        let better = best.is_none_or(|(current, current_d)| {
            let oven = surface.kind == CookSurfaceKind::Oven;
            let current_oven = current.kind == CookSurfaceKind::Oven;
            if oven != current_oven {
                oven
            } else {
                d < current_d
            }
        });
        if better {
            best = Some((surface, d));
        }
    }
    best.map(|(surface, _)| surface)
}

/// Frozen `derivePlan` (`data/cookLocations.ts:80-89`; the stand is one tile
/// south, `rangeStandFromLoc`, `data/cookingRanges.ts:35-37`).
fn derived(surface: &CookSurface) -> CookPlan {
    CookPlan {
        stand: WorldTile {
            x: surface.x,
            z: surface.z - 1,
            level: surface.level,
        },
        approach: None,
        loc_name: surface.name.clone(),
        kind: surface.kind,
        loc: surface.tile(),
        arrive_radius: DERIVED_ARRIVE_RADIUS,
        label: format!("{} @ {},{}", surface.name, surface.x, surface.z),
    }
}

/// Frozen `buildCookLocations(banks)`: one location per bank, in order.
pub(crate) fn build(banks: &[NamedBank], surfaces: &[CookSurface]) -> Vec<CookLocation> {
    banks
        .iter()
        .enumerate()
        .map(|(index, bank)| {
            let curated = curated(bank.name);
            let verified = curated.is_some();
            CookLocation {
                bank: index,
                name: bank.name,
                surface: curated.or_else(|| nearest_surface(bank.tile, surfaces).map(derived)),
                verified,
            }
        })
        .collect()
}

/// A location setting that names something to resolve (frozen
/// `resolveCookLocation`, `api/cooking/CookLocations.ts:30-34`): blank and
/// `Custom` are `None`, the caller falls back to its tile settings.
pub(crate) enum Wanted {
    Named(String),
    Auto,
}

pub(crate) fn wanted(setting: &str) -> Option<Wanted> {
    let wanted = setting.trim().to_lowercase();
    if wanted.is_empty() || wanted == CUSTOM_LOCATION {
        None
    } else if wanted == "auto" {
        Some(Wanted::Auto)
    } else {
        Some(Wanted::Named(wanted))
    }
}

/// Frozen `resolveCookLocation(setting, from, unlocked)` (`:34-48`): the index
/// of the chosen location, `Ok(None)` for unknown or locked. `from` is read
/// only for `Auto`; `unlocked` may fail (a script predicate threw).
pub(crate) fn resolve<C, E>(
    locations: &[CookLocation],
    banks: &[NamedBank],
    wanted: Wanted,
    cx: &mut C,
    from: impl FnOnce(&mut C) -> Result<WorldTile, E>,
    mut unlocked: impl FnMut(&mut C, usize) -> Result<bool, E>,
) -> Result<Option<usize>, E> {
    if let Wanted::Named(name) = wanted {
        let Some(index) = locations
            .iter()
            .position(|location| location.name.to_lowercase() == name)
        else {
            return Ok(None);
        };
        return Ok(unlocked(cx, index)?.then_some(index));
    }
    let from = from(cx)?;
    let mut best: Option<(usize, i64)> = None;
    for (index, location) in locations.iter().enumerate() {
        if !unlocked(cx, index)? {
            continue;
        }
        let distance =
            crate::bank_select::air_distance_squared(from, banks[location.bank].air_tile());
        if best.is_none_or(|(_, best)| distance < best) {
            best = Some((index, distance));
        }
    }
    Ok(best.map(|(index, _)| index))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface(x: i32, z: i32, level: i32, kind: CookSurfaceKind) -> CookSurface {
        CookSurface {
            x,
            z,
            level,
            name: if kind == CookSurfaceKind::Oven {
                "Range".into()
            } else {
                "Fireplace".into()
            },
            debugname: "range".into(),
            kind,
        }
    }

    fn bank(name: &'static str, x: i32, z: i32) -> NamedBank {
        NamedBank::new(name, WorldTile { x, z, level: 0 })
    }

    /// Frozen `betterSurface`: an oven within the radius wins over a nearer
    /// fire; beyond the radius or on another plane nothing pairs.
    #[test]
    fn a_bank_pairs_with_the_nearest_oven_within_twenty_tiles() {
        let surfaces = [
            surface(3203, 3200, 0, CookSurfaceKind::Fire),
            surface(3215, 3200, 0, CookSurfaceKind::Oven),
            surface(3201, 3200, 1, CookSurfaceKind::Oven),
            surface(3230, 3200, 0, CookSurfaceKind::Oven),
        ];
        let banks = [bank("Somewhere", 3200, 3200), bank("Far", 3000, 3000)];
        let built = build(&banks, &surfaces);
        let plan = built[0].surface.as_ref().expect("paired");
        assert_eq!(plan.loc, surfaces[1].tile());
        assert_eq!(
            plan.stand,
            WorldTile {
                x: 3215,
                z: 3199,
                level: 0
            }
        );
        assert_eq!(plan.arrive_radius, DERIVED_ARRIVE_RADIUS);
        assert_eq!(plan.label, "Range @ 3215,3200");
        assert!(!built[0].verified);
        assert_eq!(built[1].surface, None, "nothing within the radius");
    }

    #[test]
    fn a_curated_bank_keeps_its_hand_walked_surface() {
        let surfaces = [surface(2810, 3441, 0, CookSurfaceKind::Oven)];
        let built = build(&[bank("Catherby", 2809, 3441)], &surfaces);
        let camp = COOK_CAMPS
            .iter()
            .find(|camp| camp.bank == "Catherby")
            .unwrap();
        let plan = built[0].surface.as_ref().unwrap();
        assert_eq!(
            (plan.stand, plan.loc, plan.arrive_radius),
            (camp.stand, camp.loc, 0)
        );
        assert!(built[0].verified);
    }
}
