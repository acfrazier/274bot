//! Isolate Observation for posted collision + v1/v2 line-of-sight.

use crate::isolate_fb::SnapshotReader;
use api::line_of_sight::{
    flag_at, line_of_sight_v1, line_of_sight_v2, CollisionQuery, LineOfSightError,
};
use api::snapshot::WorldTile;
use std::cell::RefCell;
use std::sync::Arc;

thread_local! {
    static OBSERVATION: RefCell<Observation> = const { RefCell::new(Observation::empty()) };
}

#[derive(Clone, Debug)]
struct Observation {
    query: Option<CollisionQuery>,
}

impl Observation {
    const fn empty() -> Self {
        Self { query: None }
    }

    fn clear(&mut self) {
        *self = Self::empty();
    }

    fn update(&mut self, snap: &SnapshotReader<'_>) {
        if snap.has_ingame() && !snap.ingame() {
            self.clear();
            return;
        }
        if !snap.has_collision() {
            return;
        }
        let Some(c) = snap.collision() else {
            self.query = Some(CollisionQuery {
                available: false,
                base_x: 0,
                base_z: 0,
                level: 0,
                width: 0,
                height: 0,
                flags: Arc::from([]),
            });
            return;
        };
        self.query = Some(CollisionQuery {
            available: c.available(),
            base_x: c.base_x(),
            base_z: c.base_z(),
            level: c.level(),
            width: c.width(),
            height: c.height(),
            flags: Arc::from(c.flags()),
        });
    }
}

pub fn on_snapshot(snap: &SnapshotReader<'_>) {
    OBSERVATION.with(|obs| obs.borrow_mut().update(snap));
}

pub fn on_reset() {
    OBSERVATION.with(|obs| obs.borrow_mut().clear());
}

pub fn current() -> Option<CollisionQuery> {
    OBSERVATION.with(|obs| obs.borrow().query.clone())
}

pub fn query_v2(
    from: WorldTile,
    to: WorldTile,
    size: Option<i32>,
) -> Result<bool, LineOfSightError> {
    OBSERVATION.with(|obs| line_of_sight_v2(obs.borrow().query.as_ref(), from, to, size))
}

pub fn query_v1(from: WorldTile, to: WorldTile, size: Option<i32>) -> bool {
    OBSERVATION.with(|obs| line_of_sight_v1(obs.borrow().query.as_ref(), from, to, size))
}

pub fn raw_flag_at(index: i32) -> Option<i32> {
    OBSERVATION.with(|obs| obs.borrow().query.as_ref().and_then(|q| flag_at(q, index)))
}
