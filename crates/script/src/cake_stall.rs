//! Baker stall selection and restock helpers, usable without the `load` feature.
//! Shared target facts stay in `api::cake_stall`; the snapshot-driven sequence
//! is compiled only with the isolate feature.

use api::cake_stall::{BakerStall, StallLoc, CAKE_ITEM_NAMES};
use api::snapshot::WorldTile;

#[cfg(feature = "load")]
mod runtime;
#[cfg(feature = "load")]
pub(crate) use runtime::{dispatch, on_hold, on_pause, on_reset, on_resume, on_snapshot};

/// Neighborhood around the selected stall tile used to reject the other stall.
pub const TARGET_RADIUS: i32 = 3;

fn names_match(actual: Option<&str>, want: &str) -> bool {
    actual
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .is_some_and(|name| name.eq_ignore_ascii_case(want.trim()))
}

fn has_op(actions: &[&str], want: &str) -> bool {
    let want = want.trim();
    actions.iter().any(|action| {
        let action = action.trim();
        !action.is_empty() && action != "hidden" && action.eq_ignore_ascii_case(want)
    })
}

fn chebyshev(a: WorldTile, x: i32, z: i32, level: i32) -> i32 {
    if a.level != level {
        return i32::MAX;
    }
    (a.x - x).abs().max((a.z - z).abs())
}

/// Pick the matching Baker stall: name, steal op, selected loc id, and
/// neighborhood of the posted stall tile. Closest *qualifying* row wins.
/// Missing facts yield no target.
pub fn select_baker_stall<'a>(
    facts: Option<&BakerStall>,
    locs: &'a [StallLoc<'a>],
) -> Option<&'a StallLoc<'a>> {
    let facts = facts?;
    locs.iter()
        .filter(|loc| {
            loc.id == facts.loc_id
                && names_match(loc.name, facts.name)
                && has_op(loc.actions, facts.op)
                && chebyshev(facts.stall, loc.x, loc.z, loc.level) <= TARGET_RADIUS
        })
        .min_by_key(|loc| loc.distance)
}

/// Restock is owed only when the pack can still take stall food.
pub fn needs_cake_restock(carried: i32, target: Option<i32>, pack_full: bool) -> bool {
    if pack_full {
        return false;
    }
    carried < target.unwrap_or(1)
}

/// Exact stall-food names; chocolate cake does not match chocolate slice.
pub fn counts_as_stall_food(name: &str) -> bool {
    CAKE_ITEM_NAMES
        .iter()
        .any(|want| name.trim().eq_ignore_ascii_case(want))
}
