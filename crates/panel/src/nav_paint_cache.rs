use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use api::snapshot::WorldTile;
use nav::router::{Leg, Route};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Facts {
    pub settings_generation: u64,
    pub active: bool,
    pub base_x: i32,
    pub base_z: i32,
    pub here: Option<WorldTile>,
    pub route: u64,
    pub click: Option<WorldTile>,
    pub trail: u64,
    pub run_on: bool,
}

#[derive(Default)]
pub(crate) struct Cache {
    slots: HashMap<String, Facts>,
}

impl Cache {
    pub fn invalidate(&mut self, name: &str) {
        self.slots.remove(name);
    }

    /// Record this frame's visible facts and report whether the caller must
    /// publish. The changed state is committed atomically with the decision,
    /// so identical frames cannot request duplicate materialization.
    pub fn begin_frame(&mut self, name: &str, facts: Facts) -> bool {
        if self.slots.get(name) == Some(&facts) {
            return false;
        }
        self.slots.insert(name.to_owned(), facts);
        true
    }

    /// Correct a fact changed as a side effect of publication (trail
    /// retirement) without requesting a second publication.
    pub fn record(&mut self, name: &str, facts: Facts) {
        self.slots.insert(name.to_owned(), facts);
    }
}

fn tile_hash<H: Hasher>(tile: WorldTile, state: &mut H) {
    tile.x.hash(state);
    tile.z.hash(state);
    tile.level.hash(state);
}

/// Fingerprint only the route facts visible in the overlay. Requirements and
/// tick cost can change without changing path, caption, or hull geometry.
pub(crate) fn route_fingerprint(route: Option<&Route>) -> u64 {
    let mut state = DefaultHasher::new();
    let Some(route) = route else {
        return state.finish();
    };
    for leg in &route.legs {
        match leg {
            Leg::Walk { tiles } => {
                0u8.hash(&mut state);
                tiles.len().hash(&mut state);
                for &tile in tiles {
                    tile_hash(tile, &mut state);
                }
            }
            Leg::Transport { edge } => {
                1u8.hash(&mut state);
                edge.kind.hash(&mut state);
                tile_hash(edge.at, &mut state);
                tile_hash(edge.to, &mut state);
                edge.loc_id.hash(&mut state);
            }
        }
    }
    state.finish()
}

pub(crate) fn trail_fingerprint(path: &[(i32, i32)]) -> u64 {
    let mut state = DefaultHasher::new();
    path.hash(&mut state);
    state.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        Facts {
            settings_generation: 2,
            active: true,
            base_x: 3200,
            base_z: 3200,
            here: Some(WorldTile {
                x: 3201,
                z: 3202,
                level: 0,
            }),
            route: 7,
            click: None,
            trail: 11,
            run_on: false,
        }
    }

    #[test]
    fn per_frame_gate_republishes_visible_changes_and_clears_once() {
        let mut cache = Cache::default();
        let mut publishes = 0;
        let baseline = facts();

        for candidate in [
            baseline,
            baseline,
            Facts {
                here: Some(WorldTile {
                    x: 3202,
                    ..baseline.here.unwrap()
                }),
                ..baseline
            },
            Facts {
                route: 8,
                ..baseline
            },
            Facts {
                base_x: 3264,
                ..baseline
            },
            Facts {
                settings_generation: 3,
                ..baseline
            },
            Facts {
                active: false,
                ..baseline
            },
            Facts {
                active: false,
                ..baseline
            },
        ] {
            if cache.begin_frame("alice", candidate) {
                publishes += 1;
            }
        }

        assert_eq!(
            publishes, 6,
            "initial paint, each visible change, and one inactive clear publish"
        );
    }
}
