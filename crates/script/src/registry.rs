//! Compiled registry: rust-first rewrite cards on 274bot `api`.
//! WalkTo is **host nav** (panel picker + traveller), not a script card.
//! `factory` is `None` until a rewrite is wired. Whales are recognized but
//! never listed; `Counter` is a test fixture, never listed.

use crate::ctx::Script;

#[cfg(test)]
use crate::ctx::ScriptCtx;

/// A picker id: the exact string the picker shows and Start keys on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompiledId(pub &'static str);

/// v1 smoke names in picker order. Whales and the `Counter` fixture are
/// deliberately absent.
const COMPILED_IDS: &[CompiledId] = &[
    CompiledId("BoneBurier"),
    CompiledId("HerbCleaner"),
    CompiledId("FlaxPicker"),
    CompiledId("Firemaker"),
    CompiledId("GemCutter"),
    CompiledId("BankFletcher"),
    CompiledId("CookBot"),
    CompiledId("DartFletcher"),
    CompiledId("VialFiller"),
    CompiledId("Barcrawl"),
    CompiledId("FlaxSpinner"),
    CompiledId("SmelterBot"),
    CompiledId("SmithingBot"),
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

/// The names the picker shows, in order. This task: v1 smoke names only.
pub fn compiled_ids() -> &'static [CompiledId] {
    COMPILED_IDS
}

/// True when `name` is a known whale script (listed in the 274 client but
/// out of scope for this host).
pub fn is_whale(name: &str) -> bool {
    WHALE_IDS.contains(&name)
}

/// Start a compiled rewrite by picker id. `None` until that rewrite is
/// wired. WalkTo is not a card (host nav). `Counter` is `cfg(test)` only.
pub fn factory(id: CompiledId) -> Option<fn() -> Box<dyn Script>> {
    match id.0 {
        #[cfg(test)]
        "Counter" => Some(counter_factory),
        _ => None,
    }
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
        });
        assert_eq!(script.name(), "Counter");
    }
}
