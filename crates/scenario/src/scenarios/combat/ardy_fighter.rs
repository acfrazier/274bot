use super::*;
const STEEL_ARROW_ID: i32 = 886;
const BODY_TALISMAN_ID: i32 = 1446;
pub(crate) const BLOOD_RUNE_ID: i32 = 565;
const CHAOS_RUNE_ID: i32 = 562;
/// ArdyFighter sets its combat style only when out of combat after the first
/// cake restock, so the first Guard fight can come before it. Frozen rs2b0t
/// at `00d39a17` on this cell's fixture (A/S/HP 40, Defence 1, Thieving 5,
/// Adamant scimitar, foodTarget 1, empty food) set Strength at 7.5 s and
/// 45.3 s; strong accounts at 44.6 s and 76.6 s. Worst style time 76.6 s =
/// 128 game ticks at 600 ms, + 30 game ticks for the first Strength hit, + 60
/// game ticks for one market↔East Ardougne booth round trip (~20 tiles each
/// way at one tile per game tick, plus open and deposit) = 218 game ticks;
/// the retained cap is 320 game ticks (192s). Used by the bank-Off
/// `ardy_fighter` cell and the seeded sibling's post-return watch. The
/// earned-kill bank cell does not use it: there a catch or a public pile
/// costs a whole bank trip before the style (see
/// `ARDY_FIGHTER_BANK_TRIP_SESSION_TICKS`).
const ARDY_FIGHTER_STYLE_WATCH_TICKS: u32 = 320;
/// One pre-style leg of `ardy_fighter_bank`: the game ticks between two
/// content progress signals before the selected style. The signals are a
/// deposit that ends a Loot-count trip and a successful steal (Thieving XP,
/// `stealing.dbrow` `experience` 160). A trip comes before the style whenever
/// a Guard sees the steal (content `stealing.rs2` `stealing_check_for_guard`:
/// a Guard within five tiles with line of sight attacks) or a public Bones
/// pile lies in the loot radius, because `loot=Bones` with `bankEveryItems=1`
/// makes either item a trip end and the card banks before it restocks or sets
/// its style. After fights the card can also eat each stolen food at once
/// (`foodTarget` 1, low HP) and steal again; each such steal is progress. The
/// longest leg is a deposit followed by a catch and the next deposit. Walking
/// pace is one tile per game tick, since long runs end with run energy 1 to 3:
/// - close the booth and walk back to the anchor: 1 + 42 (the 42-tile booth
///   route of the live nav trace, `FIX-ARDY-STYLE-025/runs/base-d1`);
/// - anchor to the stall stand (2668,3312): 7;
/// - restock: three owner refusals and the stand swap (frozen
///   `RESET_AFTER_REFUSALS` 3, each steal resolves within 5 game ticks) = 18,
///   plus one emptied-stall respawn (`stealing.dbrow` `respawn_ticks` 8) = 26;
/// - the caught fight: the Guard has 22 hitpoints (`ardougne_east.npc`), 18
///   scimitar swings at attack rate 4 = 72, plus retaliation start and death
///   = 77 (six live catches took 35 to 47 game ticks from steal to Bones);
/// - take the Bones: 3;
/// - walk the 42-tile route to the booth, open it and deposit: 47.
///
/// Live deposit-to-deposit legs took 65 to 128 game ticks (`base-d7`: 95, 65,
/// 119; `final-r10`: 93, 72, 128; `FIX-ARDY-STYLE-025/runs/*/timeline.txt`).
pub(crate) const ARDY_FIGHTER_BANK_TRIP_SESSION_TICKS: u32 = 43 + 7 + 26 + 77 + 3 + 47;
/// First Strength XP of `ardy_fighter_bank` after the card acknowledges the
/// selected style. Fight picks the nearest unengaged Guard within the leash
/// (12) of the anchor and the card stands within 7 of it: walk 19 + attack 1;
/// one EatFood first: 3; five scimitar swings (1 + 4 x 4) = 17; one retarget
/// when another player engages that Guard first: 19. The card sent its
/// Attack within two game ticks of the style in all eight live selections
/// of `FIX-ARDY-STYLE-025/runs`.
pub(crate) const ARDY_FIGHTER_BANK_STYLE_FIGHT_TICKS: u32 = 20 + 3 + 17 + 20;
const _: () = assert!(
    ARDY_FIGHTER_BANK_STYLE_FIGHT_TICKS < ARDY_FIGHTER_BANK_TRIP_SESSION_TICKS,
    "a fight window is shorter than a whole trip"
);
/// Native Strength mode of the worn Adamant scimitar: `combat_hacksword:hack1`
/// (`interface.pack` 2432) runs `~set_attackstyle(1)`
/// (`player_attackstyles.rs2:38`) on `com_mode` (varp 43).
pub(crate) const ARDY_FIGHTER_STRENGTH_STYLE: Proof = Proof::VarpExact { id: 43, value: 1 };
/// Partial cakes left after a bite (`obj.pack` `partial_cake`, `cake_slice`,
/// `partial_chocolate_cake`).
const PARTIAL_CAKE_ID: i32 = 1893;
const CAKE_SLICE_ID: i32 = 1895;
const PARTIAL_CHOCOLATE_CAKE_ID: i32 = 1899;
/// Every Baker's stall food the card counts as carried food (frozen
/// `CAKE_ITEMS` substrings `cake`, `bread`, `chocolate slice`).
const ARDY_FIGHTER_STALL_FOOD: &[i32] = &[
    CAKE_ID,
    PARTIAL_CAKE_ID,
    CAKE_SLICE_ID,
    CHOCOLATE_CAKE_ID,
    PARTIAL_CHOCOLATE_CAKE_ID,
    CHOCOLATE_SLICE_ID,
    BREAD_ID,
];
/// Seeded bank trip arithmetic uses the frozen Fight's possible pre-bank
/// combat hold: `90 s / 0.6 s = 150` game ticks, plus the measured
/// market→East Ardougne approach (~23 tiles) and the existing 20-game-tick
/// open/waitBankReady component. At the old 1.25 dirty snapshots per game
/// tick, `(150 + 23 + 20) * 1.25 = 241.25` old dirty increments; the former
/// 320-dirty cap is now 320 game ticks (192s). The 20-tile return similarly
/// estimated 25 old dirties, rounded to the current 40-game-tick cap.
const ARDY_FIGHTER_BANK_SEEDED_DEPOSIT_WATCH_TICKS: u32 = 320;
const ARDY_FIGHTER_BANK_SEEDED_RETURN_WATCH_TICKS: u32 = 40;
/// ArdyFighter's loadout setting only selects gear (`weaponOf` falls back to
/// the worn weapon); its food is ignored because the card eats what it
/// steals from the Baker's stall. The fixture exists so `selectedLoadout`
/// resolves instead of failing closed with no loadouts posted, and its
/// Cake carry mirrors the injected `foodTarget` of 1.
pub(super) const ARDY_FIGHTER_FIXTURE_LOADOUTS: &[FixtureLoadout] = &[FixtureLoadout {
    name: "Scenario Ardy Fighter food",
    carry: &[("Cake", 1)],
}];
const ARDY_FIGHTER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Ardy Fighter food"),
    },
    ScriptSettingInject {
        id: "target",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "bankStrategy",
        value: ScriptInjectValue::Str("Off"),
    },
    ScriptSettingInject {
        id: "foodTarget",
        value: ScriptInjectValue::Num(1.0),
    },
];
const ARDY_FIGHTER_LOOT_EMPTY: &[i32] = &[CAKE_ID, BREAD_ID, CHOCOLATE_SLICE_ID, CHOCOLATE_CAKE_ID];
/// Earned bank cell: stall food plus the deposit class, the Guard's Bones
/// and their noted form. `bankEveryItems=1` would fire on pre-Start Bones,
/// so the class stays empty until a kill feeds it.
const ARDY_FIGHTER_BANK_LOOT_EMPTY: &[i32] = &[
    CAKE_ID,
    BREAD_ID,
    CHOCOLATE_SLICE_ID,
    CHOCOLATE_CAKE_ID,
    BONES_ID,
    NOTED_BONES_ID,
];
const ARDY_FIGHTER_BANK_SEEDED_LOOT_EMPTY: &[i32] = &[
    BREAD_ID,
    CHOCOLATE_SLICE_ID,
    CHOCOLATE_CAKE_ID,
    STEEL_ARROW_ID,
    BODY_TALISMAN_ID,
    BLOOD_RUNE_ID,
    CHAOS_RUNE_ID,
    NATURE_RUNE_ID,
];
/// ArdyFighter default Guard/strength. No fabricated cakes; the script
/// must steal cake/bread/chocolate slice after Start. PeriodicBank Off.
pub(crate) fn ardy_fighter_scenario() -> Scenario {
    let mut scenario = combat_core_scenario(CombatCorePlan {
        name: "ardy_fighter",
        card: "ArdyFighter",
        tele: ARDOUGNE_GUARD,
        radius: 12,
        food_alias: "cake",
        food_id: CAKE_ID,
        food_count: 0,
        weapon_alias: "adamant_scimitar",
        weapon_id: COMBAT_SCIMITAR_ID,
        extra_give: &[],
        wear: Some((
            "wield and acknowledge the prepared Adamant scimitar before the hostile-field teleport",
            COMBAT_SCIMITAR_ID,
        )),
        loot_empty: ARDY_FIGHTER_LOOT_EMPTY,
        inject: ARDY_FIGHTER_INJECT,
        complete_quest: None,
        thieving: 5,
        agility: 0,
    });
    let style_watch = scenario
        .steps
        .iter_mut()
        .find(|step| step.name == "watch Strength XP from the selected melee style after Start")
        .expect("ardy fighter has a post-Start Strength watch");
    style_watch.wait.budget_ticks = ARDY_FIGHTER_STYLE_WATCH_TICKS;
    scenario
}
/// ArdyFighter's `bankStrategy=Loot count` PeriodicBank after it has looted a
/// Guard drop. `foodTarget=1` keeps the stall restock short so the cell has
/// room for the loot the bank trip deposits.
const ARDY_FIGHTER_BANK_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "target",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "bankStrategy",
        value: ScriptInjectValue::Str("Loot count"),
    },
    ScriptSettingInject {
        id: "bankEveryItems",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "foodTarget",
        value: ScriptInjectValue::Num(1.0),
    },
];
/// Earned-kill variant: the loot list is the Guard's guaranteed death drop
/// alone. Guard death unconditionally adds `npc_param(death_drop)` (Bones)
/// before its random table (content `guard.rs2`), so every kill feeds the
/// `bankEveryItems=1` trip end; the six DEFAULT_LOOT items are random-table
/// outcomes (1-14/128 each) and a cell must not gate on them. This mirrors
/// AutoFighter's Custom Bones bank inject. The seeded sibling keeps the
/// default loot list because its seeded Iron ore is one of those six.
const ARDY_FIGHTER_BANK_EARNED_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "target",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "bankStrategy",
        value: ScriptInjectValue::Str("Loot count"),
    },
    ScriptSettingInject {
        id: "bankEveryItems",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "foodTarget",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "loot",
        value: ScriptInjectValue::StrList(&["Bones"]),
    },
];
/// ArdyFighter's `bankStrategy=Loot count` trip on an earned kill: the loot
/// list is pinned to the Guard's guaranteed Bones death drop (see
/// `ARDY_FIGHTER_BANK_EARNED_INJECT`), so the first kill feeds the
/// `bankEveryItems=1` trip end deterministically. The PeriodicBank walks to
/// the East Ardougne booth, deposits the Bones and returns to the market
/// anchor for further work. Nothing in the deposit class is prepared:
/// `bankEveryItems=1` would otherwise treat a pre-Start listed item as the
/// trip end. The looted Bones are watched in the pack before the deposit,
/// so the kill and the trip each have their own ordinary watch. The seeded
/// sibling below remains the default-loot proof.
pub(crate) fn ardy_fighter_bank_scenario() -> Scenario {
    let mut scenario = combat_bank_scenario(
        "ardy_fighter_bank",
        "ArdyFighter",
        ARDOUGNE_GUARD,
        12,
        "cake",
        CAKE_ID,
        0,
        "adamant_scimitar",
        COMBAT_SCIMITAR_ID,
        &[],
        ARDY_FIGHTER_BANK_LOOT_EMPTY,
        ARDY_FIGHTER_BANK_EARNED_INJECT,
        5,
        "",
        0,
        &[
            (
                "watch the Guard's looted Bones in pack before the bank trip",
                Proof::ItemId {
                    id: BONES_ID,
                    count: 1,
                },
            ),
            (
                "watch a Guard drop enter a fresh East Ardougne bank",
                Proof::BankItemId {
                    id: BONES_ID,
                    count: 1,
                },
            ),
            ("watch the periodic bank close", Proof::BankClosed),
            (
                "watch return to the market anchor after banking",
                Proof::ArrivedNear {
                    x: ARDOUGNE_GUARD.x,
                    z: ARDOUGNE_GUARD.z,
                    level: ARDOUGNE_GUARD.level,
                    radius: 6,
                },
            ),
            (
                "watch fresh Strength XP after the bank return",
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1,
                },
            ),
        ],
    );
    let style_xp = scenario
        .steps
        .iter()
        .position(|step| step.name == "watch Strength XP from the selected melee style after Start")
        .expect("ardy fighter bank has a post-Start Strength watch");
    scenario.steps[style_xp].wait.budget_ticks = ARDY_FIGHTER_BANK_STYLE_FIGHT_TICKS;
    scenario.steps.insert(
        style_xp,
        Step {
            name: "watch the card select its Strength style across pre-style Loot-count trips",
            kind: StepKind::ObserveStyleAcrossLootTrips {
                deposit_id: BONES_ID,
                food: ARDY_FIGHTER_STALL_FOOD,
                progress_stat: THIEVING_STAT,
            },
            wait: Wait {
                arm: ARDY_FIGHTER_STRENGTH_STYLE,
                budget_ticks: ARDY_FIGHTER_BANK_TRIP_SESSION_TICKS,
            },
        },
    );
    scenario.settings.deadline = ARDY_FIGHTER_BANK_DEADLINE;
    scenario
}

/// Whole-cell wall deadline for the earned-kill bank cell. Measured live on
/// Engine A at the 289 engine's 600 ms game tick: thirteen passes in fifteen
/// lifted runs, at 95.702 s (142 game ticks), 97.110 s (144), 99.891 s (149),
/// 101.798 s (152), 102.924 s (155), 133.471 s (205), 138.713 s (214),
/// 139.431 s (216), 142.506 s (221), 149.772 s (232), 155.613 s (242),
/// 158.134 s (246) and 239.154 s (381). The largest is 239.154 s; the deadline
/// is that plus 50% headroom, rounded up to a whole second (358.73 s ->
/// 359 s). The other two runs failed the pre-existing 320-game-tick style
/// watch, which this deadline does not replace. `FIX-ARDY-STYLE-025`
/// replaced that watch with the trip-aware style watch, so runs with up to
/// three pre-style Loot-count trips now pass, and they take longer. It
/// measured 32 more passes on Engine A (`runs/base-d*`, `final-r*`,
/// `head2-r*`). The largest is `head2-r6`: 365.610 s (593 game ticks, three
/// pre-style trips, style at +367). The same rule gives 548.42 s -> 549 s.
/// The seeded sibling keeps the short gold deadline and every post-Start
/// watch keeps its game-tick budget.
pub(crate) const ARDY_FIGHTER_BANK_DEADLINE: Duration = Duration::from_secs(549);

/// ArdyFighter startup PeriodicBank witness. One DEFAULT_LOOT Guard drop is
/// seeded with one Cake so only PeriodicBank owns the first trip instead of
/// `RestockCakes`; the seeded Iron ore leaves the pack, is banked in East
/// Ardougne, and the script returns to the market before fresh Strength XP.
pub(crate) fn ardy_fighter_bank_seeded_scenario() -> Scenario {
    let mut scenario = combat_bank_scenario(
        "ardy_fighter_bank_seeded",
        "ArdyFighter",
        ARDOUGNE_GUARD,
        12,
        "cake",
        CAKE_ID,
        1,
        "adamant_scimitar",
        COMBAT_SCIMITAR_ID,
        &[("iron_ore", IRON_ORE_ID, 1)],
        ARDY_FIGHTER_BANK_SEEDED_LOOT_EMPTY,
        ARDY_FIGHTER_BANK_INJECT,
        5,
        "",
        0,
        &[
            (
                "watch seeded Iron ore leave the pack after PeriodicBank deposit",
                Proof::ItemIdAtMost {
                    id: IRON_ORE_ID,
                    count: 0,
                },
            ),
            (
                "watch seeded Iron ore enter a fresh East Ardougne bank",
                Proof::BankItemId {
                    id: IRON_ORE_ID,
                    count: 1,
                },
            ),
            ("watch seeded PeriodicBank close", Proof::BankClosed),
            (
                "watch return to the market anchor after seeded banking",
                Proof::ArrivedNear {
                    x: ARDOUGNE_GUARD.x,
                    z: ARDOUGNE_GUARD.z,
                    level: ARDOUGNE_GUARD.level,
                    radius: 6,
                },
            ),
            (
                "watch fresh Strength XP after the seeded bank return",
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1,
                },
            ),
        ],
    );
    for step in &mut scenario.steps {
        match step.name {
            "watch seeded Iron ore leave the pack after PeriodicBank deposit" => {
                step.wait.budget_ticks = ARDY_FIGHTER_BANK_SEEDED_DEPOSIT_WATCH_TICKS;
            }
            "watch return to the market anchor after seeded banking" => {
                step.wait.budget_ticks = ARDY_FIGHTER_BANK_SEEDED_RETURN_WATCH_TICKS;
            }
            "watch fresh Strength XP after the seeded bank return" => {
                step.wait.budget_ticks = ARDY_FIGHTER_STYLE_WATCH_TICKS;
            }
            _ => {}
        }
    }
    scenario
}
