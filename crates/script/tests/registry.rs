// Task 2: compiled picker id list. The v1 smoke names are listed so the
// picker can show them; `factory` is None until a script is actually
// ported (Start errors "not ported" in a later panel task). Whales and
// the `Counter` test fixture must never appear in `compiled_ids`.

const SMOKES: &[&str] = &[
    "WalkTo",
    "BoneBurier",
    "HerbCleaner",
    "FlaxPicker",
    "Firemaker",
    "GemCutter",
    "BankFletcher",
    "CookBot",
    "DartFletcher",
    "VialFiller",
    "Barcrawl",
    "FlaxSpinner",
    "SmelterBot",
    "SmithingBot",
];

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
fn picker_lists_smoke_not_whales() {
    let names: Vec<_> = script::compiled_ids().iter().map(|i| i.0).collect();
    assert!(names.contains(&"WalkTo"));
    assert!(names.contains(&"BoneBurier"));
    assert!(!names
        .iter()
        .any(|n| *n == "GatheringBot" || *n == "AIOQuester"));
}

#[test]
fn compiled_ids_are_exactly_the_v1_smoke_names() {
    let names: Vec<_> = script::compiled_ids().iter().map(|i| i.0).collect();
    assert_eq!(
        names,
        SMOKES.to_vec(),
        "picker list must be the 14 v1 smoke names"
    );
    assert!(
        !names.iter().any(|n| WHALES.contains(n)),
        "whale names must not appear in compiled_ids"
    );
    assert!(
        !names.contains(&"Counter"),
        "Counter is a test fixture, not a picker id"
    );
}

#[test]
fn factory_some_only_for_ported_ids() {
    // Task 8: WalkTo is the first ported script; every other picker id
    // still has no constructor until its port lands.
    for id in script::compiled_ids() {
        let ported = id.0 == "WalkTo";
        assert_eq!(
            script::factory(*id).is_some(),
            ported,
            "{} must {} be startable",
            id.0,
            if ported { "" } else { "not " }
        );
    }
}

#[test]
fn is_whale_accepts_all_whales_rejects_smokes() {
    for w in WHALES {
        assert!(script::is_whale(w), "{w} must be a whale");
    }
    for s in SMOKES {
        assert!(!script::is_whale(s), "{s} must not be a whale");
    }
    assert!(!script::is_whale("Counter"));
    assert!(!script::is_whale("totally-unknown"));
}
