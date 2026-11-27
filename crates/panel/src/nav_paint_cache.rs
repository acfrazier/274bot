use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use api::snapshot::WorldTile;
use nav_planner::router::{Leg, Route};

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

    /// Copy the small fact key before borrowing the route owner. The slot's
    /// frame callback compares it without holding either process-wide lock.
    pub fn last_frame(&self, name: &str) -> Option<Facts> {
        self.slots.get(name).copied()
    }

    /// Record the completed paint, including any trail retirement. Each slot
    /// owns its sequential frame callback.
    pub fn record(&mut self, name: &str, facts: Facts) {
        self.slots.insert(name.to_owned(), facts);
    }
}

/// Per slot: the script route the installed nav paint draws, and the frame
/// mailbox generation already credited. Only a frame the slot's renderer
/// published after the paint was installed counts as a painted frame.
#[cfg(feature = "scenario-luau")]
#[derive(Default)]
pub(crate) struct ScriptPaintCredits {
    slots: HashMap<String, (u64, u64)>,
}

#[cfg(feature = "scenario-luau")]
impl ScriptPaintCredits {
    /// A publish replaced the slot's nav paint. It draws script route
    /// `route`, or no script route. `generation` is the slot's frame
    /// mailbox generation before any frame painted with the new paint.
    pub fn installed(&mut self, name: &str, route: Option<u64>, generation: u64) {
        match route {
            Some(route) => {
                self.slots.insert(name.to_owned(), (route, generation));
            }
            None => self.forget(name),
        }
    }

    /// The slot's frame mailbox reads `generation`. Returns the installed
    /// script route and the number of frames painted with it since the
    /// last look.
    pub fn painted(&mut self, name: &str, generation: u64) -> Option<(u64, u64)> {
        let (route, seen) = self.slots.get_mut(name)?;
        let frames = generation.checked_sub(*seen).filter(|&frames| frames > 0)?;
        *seen = generation;
        Some((*route, frames))
    }

    pub fn forget(&mut self, name: &str) {
        self.slots.remove(name);
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
            if cache.last_frame("alice") != Some(candidate) {
                cache.record("alice", candidate);
                publishes += 1;
            }
        }

        assert_eq!(
            publishes, 6,
            "initial paint, each visible change, and one inactive clear publish"
        );
    }

    // Failure modes: a published script paint is credited although no frame
    // was painted after it; a frame painted before the publish counts; one
    // painted frame is credited twice; a frame painted after the paint
    // stopped drawing the script route still counts; a later route keeps the
    // earlier route's credit.
    #[cfg(feature = "scenario-luau")]
    #[test]
    fn only_frames_painted_after_a_script_paint_publish_credit_its_route() {
        let mut credits = ScriptPaintCredits::default();
        // Mailbox generation 4 before the publish: nothing painted yet.
        credits.installed("alice", Some(7), 4);
        assert_eq!(credits.painted("alice", 4), None, "published, not painted");
        // Two frames painted with the installed route since the publish.
        assert_eq!(credits.painted("alice", 6), Some((7, 2)));
        assert_eq!(credits.painted("alice", 6), None, "each frame credits once");
        // The paint moves to the next route of the walk.
        credits.installed("alice", Some(8), 6);
        assert_eq!(credits.painted("alice", 7), Some((8, 1)));
        // A publish that no longer draws a script route ends the credit.
        credits.installed("alice", None, 7);
        assert_eq!(credits.painted("alice", 9), None);
        assert_eq!(credits.painted("bob", 9), None, "never installed");
    }
}
