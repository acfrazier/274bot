//! Registry: the compiled rewrite cards the picker lists and Start keys
//! on. `Sherlock` is the first wired port — a compiled `Script` over this
//! crate's own clue machine — and it is listed only in a `load` build,
//! because that is the build whose module tree carries the machine and the
//! interact wire it enqueues on. WalkTo is **host nav** (panel picker +
//! traveller), reserved at Load but never a compiled card. Whales are
//! recognized but never listed; `Counter` is a test fixture, never listed.
//! The rest of the catalog is loaded JS (`JsLibrary` cards), not compiled
//! rows.

use crate::ctx::Script;

#[cfg(test)]
use crate::ctx::ScriptCtx;

/// A picker id: the exact string the picker shows and Start keys on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompiledId(pub &'static str);

/// The wired cards, in picker order. `Sherlock` is the only port so far;
/// a build without `load` has no clue machine to run and lists nothing.
const COMPILED_IDS: &[CompiledId] = &[
    #[cfg(feature = "load")]
    CompiledId("Sherlock"),
];

/// Whale names: real 274 scripts out of scope this campaign. Recognized so
/// the panel can explain why a name is not listed.
const WHALE_IDS: &[&str] = &[
    "GatheringBot",
    "Woodcutter",
    "AIOQuester",
    "AIOTeleport",
    "ClueSolver",
    "AutoFighter",
    "GreenDragon",
    "FireGiant",
    "MossGiant",
    "ChaosDruidKiller",
    "RockCrab",
    "HillGiant",
    "ChickenKiller",
    "CowKiller",
    "BrimhavenAgility",
    "WildyAgility",
    "DuelArena",
    "NatureCrafter",
    "RuneCrafter",
    "MuleCrafter",
    "FlaxRunner",
    "ShopRunner",
];

/// The names the picker shows, in order: the wired compiled cards, then
/// the loaded JS catalog (which is not this list).
pub fn compiled_ids() -> &'static [CompiledId] {
    COMPILED_IDS
}

/// The wired compiled card whose picker id is `name`.
pub fn compiled_id(name: &str) -> Option<CompiledId> {
    compiled_ids().iter().copied().find(|id| id.0 == name)
}


/// True when `name` is a known whale script (listed in the 274 client but
/// out of scope for this host).
pub fn is_whale(name: &str) -> bool {
    WHALE_IDS.contains(&name)
}

/// Start a compiled rewrite by picker id. `None` for an id with no port.
/// WalkTo is not a card (host nav). `Counter` is `cfg(test)` only.
pub fn factory(id: CompiledId) -> Option<fn() -> Box<dyn Script>> {
    match id.0 {
        #[cfg(feature = "load")]
        "Sherlock" => Some(sherlock_factory),
        #[cfg(test)]
        "Counter" => Some(counter_factory),
        _ => None,
    }
}

/// One fresh solve-clue card; its own session state starts empty.
#[cfg(feature = "load")]
fn sherlock_factory() -> Box<dyn Script> {
    Box::new(crate::sherlock::Sherlock::default())
}

/// The `Counter` fixture: `tick` counts, never sends. Present so unit
/// tests can prove the factory wiring without a real script.
#[cfg(test)]
mod test_counter {
    use super::*;

    #[derive(Default)]
    pub struct Counter(pub u32);

    impl Script for Counter {
        fn name(&self) -> &str {
            "Counter"
        }
        fn tick(&mut self, _ctx: &mut ScriptCtx<'_>) {
            self.0 += 1;
        }
    }
}

#[cfg(test)]
fn counter_factory() -> Box<dyn Script> {
    Box::new(test_counter::Counter::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_ids_lists_the_wired_cards_and_nothing_else() {
        let names: Vec<&str> = compiled_ids().iter().map(|id| id.0).collect();
        #[cfg(feature = "load")]
        assert_eq!(names, ["Sherlock"], "Sherlock is the only wired card");
        #[cfg(not(feature = "load"))]
        assert!(
            names.is_empty(),
            "no clue machine without `load`: {names:?}"
        );
        #[cfg(feature = "load")]
        assert_eq!(compiled_id("Sherlock"), Some(CompiledId("Sherlock")));
        assert_eq!(compiled_id("BoneBurier"), None);
        assert_eq!(compiled_id("ClueSolver"), None);

        // Host nav, unported ports, whales and the Load catalog are never
        // compiled rows.
        for name in ["WalkTo", "BoneBurier", "Counter", "ClueSolver", "Load"] {
            assert!(!names.contains(&name), "{name} is not a card: {names:?}");
        }
    }

    #[test]
    fn factory_wires_sherlock_and_leaves_host_nav_alone() {
        #[cfg(feature = "load")]
        {
            let make = factory(CompiledId("Sherlock")).expect("Sherlock is wired");
            assert_eq!(make().name(), "Sherlock");
            // Every Start is its own card: one factory call is one instance.
            let mut first = make();
            let second = make();
            assert_eq!(first.name(), second.name());
            assert_eq!(first.name(), "Sherlock");
            // The card ticks with no slot wired: nothing dispatched, no panic.
            let mut driver = crate::ctx::test_support::NullDriver::default();
            first.tick(&mut ScriptCtx {
                driver: &mut driver,
                tick: 1,
                here: None,
                walk: None,
                walk_with: None,
                inv: None,
                snapshot: None,
                obj_names: None,
                compiled: crate::ctx::CompiledTick::default(),
            });
        }
        assert!(
            factory(CompiledId("WalkTo")).is_none(),
            "host nav is not a card"
        );
        assert!(factory(CompiledId("BoneBurier")).is_none(), "unported");
        assert!(
            factory(CompiledId("ClueSolver")).is_none(),
            "a whale stays with its own card"
        );
    }

    #[test]
    fn factory_counter_is_some_and_ticks() {
        let make = factory(CompiledId("Counter")).expect("Counter is the cfg-test constructor");
        let mut script = make();
        assert_eq!(script.name(), "Counter");
        let mut driver = crate::ctx::test_support::NullDriver::default();
        script.tick(&mut ScriptCtx {
            driver: &mut driver,
            tick: 1,
            here: None,
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: None,
            obj_names: None,
            compiled: crate::ctx::CompiledTick::default(),
        });
        assert_eq!(script.name(), "Counter");
    }
}
