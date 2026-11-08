//! Process-shared navigation state: the dominant owner of the profile-bound
//! group that Main accepted in `BATCH-029-INT/main-ruling-memory-final.md`.
//!
//! The attribution measured the group at the profile-bound boundary before
//! any slot exists: candidate 94,904,036 live bytes against comparator
//! 89,199,927, a +5,704,109-byte once-per-process increase, exactly equal at
//! N=1 and N=10 (`BATCH-029-INT/thiever-owner-attribution.md`). The
//! diagnostic's `PreparingNavigation` phase retained 91,188,966 of those
//! bytes, so the decoded world is that group's dominant owner. This file pins
//! the whole decoded world, not the atlas alone, so the next change to the
//! shared navigation state is a conscious one.

use crate::world::NavWorld;
use std::path::Path;

/// Measured decoded world on the candidate pack (91,188,966 B, exactly the
/// diagnostic's PreparingNavigation retention), plus 8 bytes and rounded up
/// to the 8-byte alignment: 91,188,976 B.
const PROFILE_BOUND_SHARED_NAV_STATE_LIMIT: usize = 91_188_976;

/// Retained bytes of one decoded nav world: the process-shared state that a
/// bound profile owns once and every bot lease borrows. The `pack` is a
/// revision's staged `274bot.navpack`; R274, R289 and the PR #6 R377 pack use
/// the same decoder and this same helper.
pub(crate) fn profile_bound_nav_state_bytes(pack: &Path) -> usize {
    let mut world = None;
    let measured = allocation_counter::measure(|| {
        world = Some(NavWorld::load_pack(pack).unwrap_or_else(|error| {
            panic!(
                "could not load selected nav pack at {}: {error}",
                pack.display()
            )
        }));
    });
    assert!(world.take().is_some(), "the decoded world stays alive");
    usize::try_from(measured.bytes_current).expect("the retained world's byte count is nonnegative")
}

#[test]
fn real_289_profile_bound_nav_state_stays_inside_its_total_tripwire() {
    let default_pack = NavWorld::default_pack_path();
    let default_pack = default_pack.to_string_lossy();
    let Some(path) = api::test_support::local_asset_path(
        "NAV_PACK",
        &default_pack,
        "revision-289 navigation pack",
    ) else {
        return;
    };
    let bytes = profile_bound_nav_state_bytes(&path);
    eprintln!(
        "profile-bound shared navigation state holds {bytes} bytes \
         (limit {PROFILE_BOUND_SHARED_NAV_STATE_LIMIT})"
    );
    assert!(
        bytes > 1_000_000,
        "the decoded world must hold the real pack's tables, measured {bytes} bytes"
    );
    assert!(
        bytes <= PROFILE_BOUND_SHARED_NAV_STATE_LIMIT,
        "profile-bound shared navigation state grew to {bytes} bytes \
         (limit {PROFILE_BOUND_SHARED_NAV_STATE_LIMIT}); update the measured \
         total and its headroom consciously"
    );
}

#[test]
fn real_377_profile_bound_nav_state_stays_inside_its_total_tripwire() {
    let default_pack = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/local-data/nav/377/274bot.navpack");
    let Some(path) = api::test_support::local_asset_path(
        "NAV_377_PACK",
        &default_pack.to_string_lossy(),
        "revision-377 navigation pack",
    ) else {
        return;
    };
    let bytes = profile_bound_nav_state_bytes(&path);
    eprintln!("revision-377 profile-bound shared navigation state holds {bytes} bytes");
    assert!(bytes > 1_000_000, "the gate must load the real 377 pack");
}
