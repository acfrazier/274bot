// Task 2: the rust-first compiled smoke list is abandoned. `compiled_ids()`
// is empty; `factory` is None for every id until a port is wired. Whales
// are recognized for picker messages but never reserve Load; WalkTo is host
// nav, reserved at Load but never a compiled card.

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
fn compiled_ids_is_empty_no_abandoned_smokes() {
    let names: Vec<_> = script::compiled_ids().iter().map(|i| i.0).collect();
    assert!(
        names.is_empty(),
        "rust-first compiled smokes are abandoned: {names:?}"
    );
    assert!(!names.contains(&"BoneBurier"));
    assert!(!names.contains(&"WalkTo"));
    assert!(!names.contains(&"Counter"));
}

#[test]
fn factory_none_for_leftover_ids_until_wired() {
    // BoneBurier and WalkTo are not factory cards: the shim catalog is
    // loaded JS (JsLibrary cards), never a compiled constructor.
    assert!(script::factory(script::CompiledId("BoneBurier")).is_none());
    assert!(script::factory(script::CompiledId("WalkTo")).is_none());
}

#[test]
fn is_whale_accepts_all_whales_rejects_unknown() {
    for w in WHALES {
        assert!(script::is_whale(w), "{w} must be a whale");
    }
    assert!(!script::is_whale("BoneBurier"));
    assert!(!script::is_whale("Counter"));
    assert!(!script::is_whale("totally-unknown"));
}

#[test]
fn is_reserved_is_walk_to_only() {
    assert!(
        script::is_reserved("WalkTo"),
        "WalkTo is host nav and stays reserved at Load"
    );
    assert!(!script::is_reserved("BoneBurier"));
    assert!(!script::is_reserved("SmithingBot"));
    assert!(!script::is_reserved("GatheringBot"));
    assert!(!script::is_reserved("totally-unknown"));
}
