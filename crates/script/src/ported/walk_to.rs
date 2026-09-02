//! Walk-hook fixture: when `here` is within `radius` the tick is a noop,
//! otherwise it queues one walk toward the target through `ctx.walk`.
//! UI WalkTo (panel picker + host nav traveller) ≠ compiled `WalkToBot`:
//! the port is a test fixture, not a registry card. Without a wired
//! `ctx.walk` hook the tick errors — never fake arrival.

use crate::ctx::{Script, ScriptCtx};

/// A world tile in the same coordinate space as `nav::tile::Tile`. The
/// `script` crate deliberately takes no `nav` dependency; host-play
/// converts between the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tile {
    pub x: i32,
    pub z: i32,
    pub level: i32,
}

/// The hardcoded v1 destination: rs2b0t's `destination` default is
/// Lumbridge centre, `arriveRadius` default is 3. The params editor is not
/// v1, so these are baked at `start` and the TS settings schema
/// (`destination`, `customTile`, `arriveRadius`) is documented in the port
/// note, not shipped.
pub const DEFAULT_TARGET: Tile = Tile {
    x: 3221,
    z: 3218,
    level: 0,
};
pub const DEFAULT_RADIUS: i32 = 3;

/// Compiled `WalkToBot`: walk toward `target` until `here` is within
/// `radius` (Chebyshev, the same distance `nav::tile::chebyshev` uses),
/// then noop.
pub struct WalkToBot {
    target: Tile,
    radius: i32,
}

impl WalkToBot {
    pub fn new(target: Tile, radius: i32) -> Self {
        WalkToBot { target, radius }
    }
}

/// Port constructor for compiled `WalkToBot` (not the panel's WalkTo
/// card). **Not registered in `registry::factory`**: UI WalkTo is host
/// nav, reserved at Load but never a compiled card. The constructor is
/// public for the port tests only.
pub fn factory() -> Box<dyn Script> {
    Box::new(WalkToBot::new(DEFAULT_TARGET, DEFAULT_RADIUS))
}

impl Script for WalkToBot {
    fn name(&self) -> &str {
        "WalkTo"
    }

    fn tick(&mut self, ctx: &mut ScriptCtx<'_>) {
        let Some((hx, hz, _)) = ctx.here else {
            // No observed player tile yet (the TS waits for
            // `Game.ingame() && Game.tile() !== null` before starting).
            return;
        };
        if (hx - self.target.x).abs().max((hz - self.target.z).abs()) <= self.radius {
            return;
        }
        let Some(walk) = ctx.walk.as_mut() else {
            // No traveller wired on this slot: error (the slot drops the
            // instance), do not fake arrival.
            panic!("Traversal/nav not on ctx");
        };
        walk(self.target.x, self.target.z, self.target.level);
    }
}
