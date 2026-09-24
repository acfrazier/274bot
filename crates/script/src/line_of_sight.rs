//! v1/v2 line-of-sight over the posted collision table in the isolate scene.

use crate::observed::{self, Scene};
use api::line_of_sight::{
    flag_at, line_of_sight_v1, line_of_sight_v2, CollisionQuery, LineOfSightError,
};
use api::snapshot::WorldTile;

/// The posted collision table. A logout forgets the session: only a table
/// posted since login counts.
fn query(scene: &Scene) -> Option<&CollisionQuery> {
    scene.since_login().collision()
}

pub fn current() -> Option<CollisionQuery> {
    observed::with(|scene| query(scene).cloned())
}

pub fn query_v2(
    from: WorldTile,
    to: WorldTile,
    size: Option<i32>,
) -> Result<bool, LineOfSightError> {
    observed::with(|scene| line_of_sight_v2(query(scene), from, to, size))
}

pub fn query_v1(from: WorldTile, to: WorldTile, size: Option<i32>) -> bool {
    observed::with(|scene| line_of_sight_v1(query(scene), from, to, size))
}

pub fn raw_flag_at(index: i32) -> Option<i32> {
    observed::with(|scene| query(scene).and_then(|q| flag_at(q, index)))
}
