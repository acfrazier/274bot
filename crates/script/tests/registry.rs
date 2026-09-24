// R018: the compiled card list. `Sherlock` is the first wired rust-first
// port; every other smoke id stays unwired. Whales are recognized for
// picker messages but never reserve Load; WalkTo is host nav, reserved at
// Load but never a compiled card.

const WHALES: &[&str] = &[
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

#[test]
fn compiled_ids_lists_only_the_wired_card() {
    let names: Vec<_> = script::compiled_ids().iter().map(|i| i.0).collect();
    // A build without `load` has no clue machine, so it lists no card.
    #[cfg(feature = "load")]
    assert_eq!(names, ["Sherlock"], "Sherlock is the only wired card");
    #[cfg(not(feature = "load"))]
    assert!(names.is_empty(), "no port without `load`: {names:?}");
    assert!(!names.contains(&"BoneBurier"));
    assert!(!names.contains(&"WalkTo"));
    assert!(!names.contains(&"Counter"));
}

#[test]
fn factory_wires_sherlock_and_none_for_leftover_ids() {
    // BoneBurier and WalkTo are not factory cards: the shim catalog is
    // loaded JS (JsLibrary cards), never a compiled constructor, and a
    // whale keeps its own card.
    #[cfg(feature = "load")]
    {
        let make = script::factory(script::CompiledId("Sherlock")).expect("Sherlock is wired");
        assert_eq!(make().name(), "Sherlock");
    }
    assert!(script::factory(script::CompiledId("BoneBurier")).is_none());
    assert!(script::factory(script::CompiledId("WalkTo")).is_none());
    assert!(script::factory(script::CompiledId("ClueSolver")).is_none());
}

#[test]
fn is_whale_accepts_all_whales_rejects_unknown() {
    for w in WHALES {
        assert!(script::is_whale(w), "{w} must be a whale");
    }
    assert!(!script::is_whale("BoneBurier"));
    assert!(!script::is_whale("Counter"));
    assert!(!script::is_whale("Sherlock"));
    assert!(!script::is_whale("totally-unknown"));
}

#[test]
fn a_whale_is_never_a_compiled_card() {
    let names: Vec<_> = script::compiled_ids().iter().map(|i| i.0).collect();
    for w in WHALES {
        assert!(script::is_whale(w));
        assert!(!names.contains(w), "{w} must not be listed as compiled");
    }
}
