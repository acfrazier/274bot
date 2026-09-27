//! Rust-owned `buryOneInFight`, the `fight-bury` [`crate::machine`] family.
//!
//! Frozen `api/combat/fightUpkeep.ts:24-37`: skip the tick our swing began
//! (`:25`, the module `AttackClock` of [`crate::attack_clock`]: the
//! tick-costing bury would stall the attack), take the first
//! backpack row named `boneName` (`:28`), press its `Bury` op (`:33`, the
//! `InvItem.interact` op-list match of `api/inventory/Inventory.ts:45-56`),
//! then report only a burial the backpack confirms: `Inventory.used()` below
//! its count before the click within `BURY_CONFIRM_TICKS` (`:7`, `:36`).
//! JavaScript starts one row and awaits its boolean.

use crate::machine::{Begin, Cx, Family, Step};
use crate::observed::{self, ItemRow, Scene};
use crate::shim::InteractReq;
use serde::Deserialize;

/// Frozen `BURY_CONFIRM_TICKS`.
pub const BURY_CONFIRM_TICKS: u32 = 3;

/// The frozen `InvItem.interact` op the fight loop presses.
const BURY: &str = "bury";

/// Frozen `Inventory.used()`: the occupied backpack slots posted since login.
fn used(scene: &Scene) -> usize {
    scene.since_login().inv().map_or(0, |rows| {
        rows.iter()
            .filter(|row| row.name.as_deref().is_some_and(|name| !name.is_empty()))
            .count()
    })
}

/// Frozen `Inventory.first(name)`: the first backpack row whose name matches
/// case-insensitively (ASCII: item names are ASCII).
fn first<'a>(scene: &'a Scene, name: &str) -> Option<&'a ItemRow> {
    scene.since_login().inv()?.iter().find(|row| {
        row.name
            .as_deref()
            .is_some_and(|got| got.eq_ignore_ascii_case(name))
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BuryArgs {
    bone_name: String,
}

/// One bury: the op went out at begin; each step watches the backpack.
pub(crate) struct BuryInFight {
    before: usize,
    ticks_left: u32,
}

impl Family for BuryInFight {
    const NAME: &'static str = "fight-bury";
    /// One bury at a time: a newer call replaces the one in flight.
    const EXCLUSIVE: bool = true;
    type Args = BuryArgs;
    type Output = bool;

    fn begin(args: BuryArgs, cx: &mut Cx<'_>) -> Begin<Self> {
        if crate::attack_clock::swing_started_this_tick() {
            return Begin::Done(false);
        }
        observed::with(|scene| {
            let Some(bones) = first(scene, &args.bone_name) else {
                return Begin::Done(false);
            };
            let Some(op) = bones.ops.iter().find(|op| op.eq_ignore_ascii_case(BURY)) else {
                return Begin::Done(false);
            };
            let before = used(scene);
            cx.emit(InteractReq::Held {
                name: bones.name_or_empty().to_string(),
                action: op.to_string(),
            });
            Begin::Run(Self {
                before,
                ticks_left: BURY_CONFIRM_TICKS,
            })
        })
    }

    /// Frozen `delayUntilTicks(() => used() < before, 3)`: one check per
    /// tick, the last on the third tick after the click.
    fn step(&mut self, _cx: &mut Cx<'_>) -> Step<bool> {
        if observed::with(used) < self.before {
            return Step::Done(true);
        }
        self.ticks_left = self.ticks_left.saturating_sub(1);
        if self.ticks_left == 0 {
            Step::Done(false)
        } else {
            Step::Wait
        }
    }
}
