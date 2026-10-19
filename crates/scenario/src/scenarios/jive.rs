//! First live coverage for the Jive* catalog cards, ported from rs2b0t's
//! Playwright e2e tests at `2de6bb23`. Each cell keeps the upstream fixture
//! (account state seeded before Start: stats, pack, bank, start tile) and
//! checks the same behaviour with harness proofs instead of script-log
//! milestones: log-line gates become item/XP/bank proofs, upstream's
//! post-Start starve/`~hit` harness cheats are not ported (nothing cheats
//! after Start; pack pressure is seeded instead), and per-milestone ms
//! budgets become one fixed deadline justified per cell.

use super::combat::{
    combat_fixture_loadouts, wear_combat_item_step, DRAGONFIRE_SHIELD_ID, DRAGON_BONES_ID,
};
use super::production::*;
use crate::*;

const RANGED_STAT: i32 = 4;
const PRAYER_STAT_ID: i32 = 5;
const MAGIC_SHORTBOW_ID: i32 = 861;
const RUNE_ARROW_ID: i32 = 892;
const SAPPHIRE_RING_ID: i32 = 1637;
const RING_OF_RECOIL_ID: i32 = 2550;
const BLACK_DRAGONHIDE_ID: i32 = 1747;
const NOTED_LOBSTER_ID: i32 = 380;
const BRONZE_SWORD_ID: i32 = 1277;
const SUPER_ATTACK_1_ID: i32 = 149;
const SUPER_ATTACK_3_ID: i32 = 145;
const SUPER_STRENGTH_2_ID: i32 = 159;
const SUPER_STRENGTH_3_ID: i32 = 157;
const SUPER_DEFENCE_3_ID: i32 = 163;
/// `scenario` food loadout the other combat cells pin (`combat.rs`
/// `GREEN_DRAGON_FIXTURE_LOADOUTS`): posting it at Start keeps the Jive
/// combat cards' `loadout` setting off the operator settings parser and
/// gives `scriptFood` a defined first eatable (Lobster) under the
/// throwaway HOME, where no operator `loadouts.json` exists.
const JIVE_FOOD_LOADOUT: &str = "Scenario Green Dragon food";

const VARROCK_WEST_STAND: WorldTile = WorldTile {
    x: 3185,
    z: 3440,
    level: 0,
};
const EDGEVILLE_BANK: WorldTile = WorldTile {
    x: 3094,
    z: 3493,
    level: 0,
};
const CHEST_STAND: WorldTile = WorldTile {
    x: 2914,
    z: 3451,
    level: 0,
};
const BLUE_DRAGON_SAFESPOT: WorldTile = WorldTile {
    x: 2901,
    z: 9809,
    level: 0,
};
const BLACK_DRAGON_STAND: WorldTile = WorldTile {
    x: 2836,
    z: 9817,
    level: 0,
};
const BLACK_DEMON_SAFESPOT: WorldTile = WorldTile {
    x: 2856,
    z: 9786,
    level: 0,
};
const KBD_ALCOVE: WorldTile = WorldTile {
    x: 2717,
    z: 9801,
    level: 0,
};

const fn inj_str(id: &'static str, value: &'static str) -> ScriptSettingInject {
    ScriptSettingInject {
        id,
        value: ScriptInjectValue::Str(value),
    }
}
const fn inj_num(id: &'static str, value: f64) -> ScriptSettingInject {
    ScriptSettingInject {
        id,
        value: ScriptInjectValue::Num(value),
    }
}
const fn inj_bool(id: &'static str, value: bool) -> ScriptSettingInject {
    ScriptSettingInject {
        id,
        value: ScriptInjectValue::Bool(value),
    }
}
const fn inj_list(id: &'static str, value: &'static [&'static str]) -> ScriptSettingInject {
    ScriptSettingInject {
        id,
        value: ScriptInjectValue::StrList(value),
    }
}

/// One post-Start watch with an explicit dirty-snapshot budget (runner
/// increments, ~2.5/s at full rate; not engine ticks or wall seconds).
fn jive_watch(name: &'static str, arm: Proof, budget_ticks: u32) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait { arm, budget_ticks },
    }
}

/// Pre-Start seed: run `cmds` once (stats, inv/bank stock, optional tele),
/// then wait for `arm`. Everything a Jive cell cheats happens here or in the
/// acknowledgement watches that follow; after Start the cell only observes.
fn jive_seed(name: &'static str, cmds: Vec<String>, arm: Proof) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                for cmd in &cmds {
                    cheat(c, cmd);
                }
                true
            }),
        },
        wait: Wait {
            arm,
            budget_ticks: 200,
        },
    }
}

fn jive_tele(name: &'static str, dest: WorldTile, radius: i32) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &tele_args(dest.level, dest.x, dest.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: dest.x,
                z: dest.z,
                level: dest.level,
                radius,
            },
            budget_ticks: 200,
        },
    }
}

fn stat_cmds(levels: &[(&str, i32)]) -> Vec<String> {
    let mut cmds: Vec<String> = levels
        .iter()
        .map(|(skill, level)| format!("setstat {skill} {level}"))
        .collect();
    cmds.push("~clearinv".to_string());
    cmds.push("~clearbank".to_string());
    cmds
}

fn jive_scenario(
    name: &'static str,
    card: &'static str,
    steps: Vec<Step>,
    proof: Proof,
    deadline: Duration,
    inject: &'static [ScriptSettingInject],
    fixture_loadouts: Option<&'static [FixtureLoadout]>,
) -> Scenario {
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline,
            start_script: Some(card),
            script_settings_inject: Some(inject),
            fixture_loadouts,
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Ported from rs2b0t `e2e/jivechests-live.ts` @2de6bb23.
/// Changed: upstream's `[chests] opened/banked` log milestones and exact
/// 7-keys-a-trip log count become pack/bank item proofs; the chest
/// always adds an Uncut dragonstone before its random extra
/// (`content/scripts/areas/area_taverly/scripts/crystal_chest.rs2:24`),
/// so 7 banked dragonstones prove 7 opens banked. The junk-came-home
/// log gate is not ported (a negative pack state at an arbitrary
/// terminal instant is not a function proof).
/// One banked trip is upstream's pass bar (`opens >= 7`, `trips >= 1`);
/// the second withdrawal of 7 keys proves the loop closed.
pub(crate) fn jive_chests_scenario() -> Scenario {
    let bank = FALADOR_WEST_BANK;
    let mut cmds = stat_cmds(&[("magic", 55)]);
    cmds.extend([
        "givebank crystal_key 14".to_string(),
        "givebank lawrune 50".to_string(),
        "givebank airrune 150".to_string(),
        "givebank waterrune 50".to_string(),
        "give lawrune 20".to_string(),
        "give airrune 60".to_string(),
        "give waterrune 20".to_string(),
        tele_args(bank.level, bank.x, bank.z),
    ]);
    let mut steps = script_live_seed_steps();
    steps.push(jive_seed(
        "seed Magic 55, fourteen banked crystal keys and teleport runes, stand at Falador West",
        cmds,
        Proof::ArrivedNear {
            x: bank.x,
            z: bank.z,
            level: bank.level,
            radius: 6,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge Magic 55 before Start",
        Proof::Stat {
            id: MAGIC_STAT,
            min: 55,
        },
    ));
    steps.push(bank_fletcher_watch(
        "confirm no crystal key in pack before Start",
        Proof::ItemAtMost {
            name: "Crystal key",
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "confirm no dragonstone in pack before Start",
        Proof::ItemAtMost {
            name: "Uncut dragonstone",
            count: 0,
        },
    ));
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the fourteen-key seed bank",
        Proof::BankItem {
            name: "Crystal key",
            count: 14,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded dragonstone in bank",
        Proof::BankItemAtMost {
            name: "Uncut dragonstone",
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    steps.extend([
        jive_watch(
            "watch a full trip of seven keys withdrawn",
            Proof::Item {
                name: "Crystal key",
                count: 7,
            },
            300,
        ),
        jive_watch(
            "watch the walk end on the Taverley chest stand",
            Proof::ArrivedNear {
                x: CHEST_STAND.x,
                z: CHEST_STAND.z,
                level: CHEST_STAND.level,
                radius: 2,
            },
            900,
        ),
        jive_watch(
            "watch the first chest open pay its dragonstone",
            Proof::Item {
                name: "Uncut dragonstone",
                count: 1,
            },
            300,
        ),
        jive_watch(
            "watch seven opens' dragonstones reach the bank",
            Proof::BankItem {
                name: "Uncut dragonstone",
                count: 7,
            },
            1500,
        ),
        jive_watch(
            "watch the next trip's seven keys withdrawn",
            Proof::Item {
                name: "Crystal key",
                count: 7,
            },
            300,
        ),
    ]);
    // One withdraw + Falador->Taverley walk + 7 opens + teleport home +
    // bank + rewithdraw, at human walk speed with route margin.
    jive_scenario(
        "jive_chests",
        "JiveChests",
        steps,
        Proof::Item {
            name: "Crystal key",
            count: 7,
        },
        Duration::from_secs(900),
        JIVE_CHESTS_INJECT,
        None,
    )
}

const JIVE_CHESTS_INJECT: &[ScriptSettingInject] = &[inj_bool("teleportHome", true)];

/// Ported from rs2b0t `e2e/jivecrafting-live.ts` @2de6bb23 (default
/// product Sapphire ring, Crafting 99, 30 seeded sets).
/// Changed: upstream's `[crafting] banked N` trip counting becomes exact
/// pack/bank ring counts (a full gem load is 13: mould + 13 bars + 13 gems
/// in a 28 pack, `JiveCrafting/logic.ts` `setsPerTrip`), and its
/// xp-resumed-after-trip gate becomes a cumulative XP gate past one full
/// load (14 rings x 40 Crafting XP, `logic.ts` `JEWELLERY`).
pub(crate) fn jive_crafting_scenario() -> Scenario {
    let bank = AL_KHARID_BANK;
    let mut cmds = stat_cmds(&[("crafting", 99)]);
    cmds.extend([
        "givebank ring_mould 1".to_string(),
        "givebank gold_bar 30".to_string(),
        "givebank sapphire 30".to_string(),
        tele_args(bank.level, bank.x, bank.z),
    ]);
    let mut steps = script_live_seed_steps();
    steps.push(jive_seed(
        "seed Crafting 99 and thirty gold/sapphire sets in the Al Kharid bank",
        cmds,
        Proof::ArrivedNear {
            x: bank.x,
            z: bank.z,
            level: bank.level,
            radius: 6,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge Crafting 99 before Start",
        Proof::Stat {
            id: CRAFTING_STAT,
            min: 99,
        },
    ));
    steps.push(bank_fletcher_watch(
        "confirm no gold bar in pack before Start",
        Proof::ItemAtMost {
            name: "Gold bar",
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "confirm no sapphire ring in pack before Start",
        Proof::ItemIdAtMost {
            id: SAPPHIRE_RING_ID,
            count: 0,
        },
    ));
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the thirty gold bar seed bank",
        Proof::BankItem {
            name: "Gold bar",
            count: 30,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the thirty sapphire seed bank",
        Proof::BankItem {
            name: "Sapphire",
            count: 30,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the ring mould seed bank",
        Proof::BankItem {
            name: "Ring mould",
            count: 1,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded sapphire ring in bank",
        Proof::BankItemIdAtMost {
            id: SAPPHIRE_RING_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    steps.extend([
        jive_watch(
            "watch Crafting XP from the first furnace ring",
            Proof::StatXpGain {
                id: CRAFTING_STAT,
                min: 40,
            },
            300,
        ),
        jive_watch(
            "watch a full load of thirteen sapphire rings in pack",
            Proof::ItemId {
                id: SAPPHIRE_RING_ID,
                count: 13,
            },
            600,
        ),
        jive_watch(
            "watch the full load enter the bank",
            Proof::BankItemId {
                id: SAPPHIRE_RING_ID,
                count: 13,
            },
            600,
        ),
        jive_watch(
            "watch the second load's first ring after restock",
            Proof::ItemId {
                id: SAPPHIRE_RING_ID,
                count: 1,
            },
            600,
        ),
    ]);
    // Two bank<->furnace loops of 13 makes each at Al Kharid, with margin.
    jive_scenario(
        "jive_crafting",
        "JiveCrafting",
        steps,
        Proof::StatXpGain {
            id: CRAFTING_STAT,
            min: 560,
        },
        Duration::from_secs(600),
        JIVE_CRAFTING_INJECT,
        None,
    )
}

const JIVE_CRAFTING_INJECT: &[ScriptSettingInject] = &[inj_str("product", "Sapphire ring")];

/// Ported from rs2b0t `e2e/jiveenchanter-live.ts` @2de6bb23 (default jewel
/// Sapphire ring, staff banked, Magic 70, 60 seeded rings).
/// Changed: upstream's `[enchanter] wielding/banked` log milestones become
/// the worn staff, exact pack/bank Ring of recoil counts (a staffed full
/// load is 27: 28 slots minus the cosmic stack, `JiveEnchanter/logic.ts`),
/// and a cumulative Magic XP gate past one full load (28 casts x 17.5 XP,
/// Lvl-1 Enchant). The cast-pacing (1 cast/3 ticks) stays descriptive, as
/// upstream: no timing oracle.
pub(crate) fn jive_enchanter_scenario() -> Scenario {
    let bank = VARROCK_WEST_STAND;
    let mut cmds = stat_cmds(&[("magic", 70)]);
    cmds.extend([
        "givebank sapphire_ring 60".to_string(),
        "givebank cosmicrune 80".to_string(),
        "givebank waterrune 80".to_string(),
        "givebank staff_of_water 1".to_string(),
        tele_args(bank.level, bank.x, bank.z),
    ]);
    let mut steps = script_live_seed_steps();
    steps.push(jive_seed(
        "seed Magic 70, sixty banked sapphire rings, runes and a staff of water at Varrock West",
        cmds,
        Proof::ArrivedNear {
            x: bank.x,
            z: bank.z,
            level: bank.level,
            radius: 6,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge Magic 70 before Start",
        Proof::Stat {
            id: MAGIC_STAT,
            min: 70,
        },
    ));
    steps.push(bank_fletcher_watch(
        "confirm no sapphire ring in pack before Start",
        Proof::ItemIdAtMost {
            id: SAPPHIRE_RING_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "confirm no ring of recoil in pack before Start",
        Proof::ItemIdAtMost {
            id: RING_OF_RECOIL_ID,
            count: 0,
        },
    ));
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the sixty sapphire ring seed bank",
        Proof::BankItemId {
            id: SAPPHIRE_RING_ID,
            count: 60,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the banked staff of water",
        Proof::BankItemId {
            id: STAFF_OF_WATER_ID,
            count: 1,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded ring of recoil in bank",
        Proof::BankItemIdAtMost {
            id: RING_OF_RECOIL_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    steps.extend([
        jive_watch(
            "watch the banked staff of water get wielded",
            Proof::EquipmentId {
                id: STAFF_OF_WATER_ID,
            },
            300,
        ),
        jive_watch(
            "watch Magic XP from the first enchant cast",
            Proof::StatXpGain {
                id: MAGIC_STAT,
                min: 17,
            },
            300,
        ),
        jive_watch(
            "watch a full load of twenty-seven rings of recoil in pack",
            Proof::ItemId {
                id: RING_OF_RECOIL_ID,
                count: 27,
            },
            900,
        ),
        jive_watch(
            "watch the full load enter the bank",
            Proof::BankItemId {
                id: RING_OF_RECOIL_ID,
                count: 27,
            },
            300,
        ),
        jive_watch(
            "watch the second load's first ring of recoil after restock",
            Proof::ItemId {
                id: RING_OF_RECOIL_ID,
                count: 1,
            },
            600,
        ),
    ]);
    // 28 casts at the script's own 3-tick pacing plus two bank visits.
    jive_scenario(
        "jive_enchanter",
        "JiveEnchanter",
        steps,
        Proof::StatXpGain {
            id: MAGIC_STAT,
            min: 490,
        },
        Duration::from_secs(600),
        JIVE_ENCHANTER_INJECT,
        None,
    )
}

const JIVE_ENCHANTER_INJECT: &[ScriptSettingInject] = &[inj_str("jewel", "Sapphire ring")];

/// Upstream Jive combat stat line (`jivedragons-live.ts` `LEVELS`, also the
/// black fixture's): 85 Ranged / 80 Magic / 99 Hitpoints puts combat above
/// 97, so Taverley baby blues never aggress on the safespot.
const DRAGON_LEVELS: &[(&str, i32)] = &[
    ("attack", 75),
    ("strength", 75),
    ("defence", 75),
    ("hitpoints", 99),
    ("prayer", 70),
    ("ranged", 85),
    ("magic", 80),
    ("agility", 40),
    ("woodcutting", 45),
];

/// The dragon-table loot list upstream's ranged kits inject (`loot` for
/// blue, `lootBlack` for black: same names, different setting keys).
const DRAGON_LOOT: &[&str] = &[
    "Dragon bones",
    "Dragonhide",
    "Uncut diamond",
    "Uncut ruby",
    "Uncut emerald",
    "Uncut sapphire",
];

/// Ported from rs2b0t `e2e/jivedragons-live.ts` @2de6bb23 at its defaults
/// (site blue, style range, leave teleport) with `--dusty` (key banked).
/// Changed: started on the site's own safespot after acknowledged seeding
/// (the black-range fixture's start shape; the Falador walk-in and cold
/// Jailer key leg are the `acquire_key_v2_ts` cell's behaviour, not this
/// card's combat proof). Upstream's log/counter milestones (wielded, kill,
/// loot, banktrip) become worn-bow, Ranged XP (a blue dragon's 105 HP at
/// the content 0.4 XP/damage ranged rate is 42 XP, `skill_combat/scripts/
/// combat.rs2` `give_combat_experience`), pack then banked Dragon bones
/// (the guaranteed drop). Bank timing: with `combatStyle` range at a site
/// without a ranged threat, the script keeps no food back from slot-freeing
/// (`JiveDragons.ts:431-433` `slotFoodReserve()` is 0 unless the bot is too
/// hurt to loot, so upstream's `foodReserve` 4 is kept) and banks only once
/// the pack is full and the food is spent (`:1074-1086`). The pack's ~22
/// non-kit slots fill at two drops a kill, about 11 blue kills before the
/// trip; the live receipt `REFREEZE-MERGE-023/live` measured ~138 server
/// ticks a kill. The
/// bankCommonJunk/solveClues/buryBones/leaveVia settings are upstream's.
pub(crate) fn jive_dragons_scenario() -> Scenario {
    let mut cmds = stat_cmds(DRAGON_LEVELS);
    cmds.extend([
        "givebank lobster 400".to_string(),
        "givebank lawrune 200".to_string(),
        "givebank airrune 5000".to_string(),
        "givebank waterrune 200".to_string(),
        "givebank earthrune 200".to_string(),
        "givebank magic_shortbow 1".to_string(),
        "givebank rune_arrow 5000".to_string(),
        "givebank 4doserangerspotion 5".to_string(),
        "givebank dusty_key 1".to_string(),
        "give magic_shortbow 1".to_string(),
        "give rune_arrow 500".to_string(),
        "give 4doserangerspotion 1".to_string(),
        "give lobster 20".to_string(),
        "give dusty_key 1".to_string(),
        "give airrune 30".to_string(),
        "give waterrune 10".to_string(),
        "give lawrune 10".to_string(),
    ]);
    let mut steps = script_live_seed_steps();
    steps.push(jive_seed(
        "seed the ranged dragon stat line, stocked pack and bank on the safe tile",
        cmds,
        Proof::Stat {
            id: RANGED_STAT,
            min: 85,
        },
    ));
    steps.extend([
        bank_fletcher_watch(
            "acknowledge Hitpoints 99 before Start",
            Proof::Stat {
                id: HITPOINTS_STAT,
                min: 99,
            },
        ),
        bank_fletcher_watch(
            "acknowledge the carried magic shortbow before Start",
            Proof::ItemId {
                id: MAGIC_SHORTBOW_ID,
                count: 1,
            },
        ),
        bank_fletcher_watch(
            "acknowledge five hundred carried rune arrows before Start",
            Proof::ItemId {
                id: RUNE_ARROW_ID,
                count: 500,
            },
        ),
        bank_fletcher_watch(
            "acknowledge twenty carried lobsters before Start",
            Proof::ItemId {
                id: LOBSTER_ID,
                count: 20,
            },
        ),
        bank_fletcher_watch(
            "confirm no seeded dragon bones in pack before Start",
            Proof::ItemIdAtMost {
                id: DRAGON_BONES_ID,
                count: 0,
            },
        ),
    ]);
    steps.push(jive_tele(
        "teleport to the Taverley blue dragon safespot after preparation is acknowledged",
        BLUE_DRAGON_SAFESPOT,
        3,
    ));
    steps.push(start_catalog_step());
    steps.extend([
        jive_watch(
            "watch the script wield the magic shortbow",
            Proof::EquipmentId {
                id: MAGIC_SHORTBOW_ID,
            },
            300,
        ),
        jive_watch(
            "watch a blue dragon kill's worth of Ranged XP",
            Proof::StatXpGain {
                id: RANGED_STAT,
                min: 42,
            },
            900,
        ),
        jive_watch(
            "watch looted dragon bones reach the pack",
            Proof::ItemId {
                id: DRAGON_BONES_ID,
                count: 1,
            },
            600,
        ),
        jive_watch(
            "watch the bones reach the Falador bank",
            Proof::BankItemId {
                id: DRAGON_BONES_ID,
                count: 1,
            },
            3000,
        ),
    ]);
    // About 11 kills at ~138 server ticks each (~15 min), the Falador
    // teleport and the deposit, plus seeding, the walk-in and margin.
    jive_scenario(
        "jive_dragons",
        "JiveDragons",
        steps,
        Proof::StatXpGain {
            id: RANGED_STAT,
            min: 42,
        },
        Duration::from_secs(1800),
        JIVE_DRAGONS_BLUE_INJECT,
        combat_fixture_loadouts("GreenDragon"),
    )
}

const JIVE_DRAGONS_BLUE_INJECT: &[ScriptSettingInject] = &[
    inj_str("loadout", JIVE_FOOD_LOADOUT),
    inj_str("site", "taverley-blue"),
    inj_str("combatStyle", "range"),
    inj_str("bow", "Magic shortbow"),
    inj_str("ammo", "Rune arrow"),
    inj_str("rangeStyle", "rapid"),
    inj_num("ammoWithdraw", 500.0),
    inj_bool("useSpecial", true),
    inj_bool("rangingPotion", true),
    inj_num("foodWithdraw", 20.0),
    inj_num("panicHp", 30.0),
    inj_num("foodReserve", 4.0),
    inj_num("healTo", 90.0),
    inj_num("teleStock", 2.0),
    inj_bool("buryBones", false),
    inj_bool("solveClues", false),
    inj_bool("bankCommonJunk", false),
    inj_str("leaveVia", "teleport"),
    inj_list("loot", DRAGON_LOOT),
    inj_str("logDetail", "Verbose"),
];

/// Ported from rs2b0t `e2e/jivedragons-black-range-live.ts` @2de6bb23
/// (fixture/settings in `e2e/jivedragons-black-fixture.ts`).
/// Changed: upstream runs on a private world at a 200 ms tick collecting
/// an event trace; neither exists here (shared Engine A stays at its
/// 600 ms default), so the same fixture is started on the same corridor
/// stand and the trace contract's kill/loot outcome becomes game-state
/// proofs. The private 'Private Sharks' loadout becomes the existing
/// GreenDragon scenario food loadout (Lobster), so bank/pack food is
/// Lobster where upstream seeds Shark; the full-28-slot food pack (a
/// slot-freeing-policy probe) becomes upstream's black ranged kit pack.
/// `buryBones` is on in `blackSettings`, so one kill proves itself as
/// 72 Prayer XP from the buried bones plus the hide in pack, not banked
/// bones.
pub(crate) fn jive_dragons_black_range_scenario() -> Scenario {
    let mut cmds = stat_cmds(DRAGON_LEVELS);
    cmds.extend([
        "givebank lobster 400".to_string(),
        "givebank rune_arrow 5000".to_string(),
        "givebank magic_shortbow 1".to_string(),
        "givebank 4doserangerspotion 5".to_string(),
        "givebank 4dose2antipoison 20".to_string(),
        "givebank airrune 5000".to_string(),
        "givebank waterrune 200".to_string(),
        "givebank lawrune 200".to_string(),
        "give magic_shortbow 1".to_string(),
        "give rune_arrow 500".to_string(),
        "give 4doserangerspotion 1".to_string(),
        "give lobster 20".to_string(),
        "give 4dose2antipoison 1".to_string(),
        "give airrune 30".to_string(),
        "give waterrune 10".to_string(),
        "give lawrune 10".to_string(),
        "give dusty_key 1".to_string(),
    ]);
    let mut steps = script_live_seed_steps();
    steps.push(jive_seed(
        "seed the black ranged stat line, stocked pack and bank on the safe tile",
        cmds,
        Proof::Stat {
            id: RANGED_STAT,
            min: 85,
        },
    ));
    steps.extend([
        bank_fletcher_watch(
            "acknowledge the carried magic shortbow before Start",
            Proof::ItemId {
                id: MAGIC_SHORTBOW_ID,
                count: 1,
            },
        ),
        bank_fletcher_watch(
            "acknowledge twenty carried lobsters before Start",
            Proof::ItemId {
                id: LOBSTER_ID,
                count: 20,
            },
        ),
        bank_fletcher_watch(
            "confirm no seeded black dragonhide in pack before Start",
            Proof::ItemIdAtMost {
                id: BLACK_DRAGONHIDE_ID,
                count: 0,
            },
        ),
    ]);
    steps.push(jive_tele(
        "teleport to the Taverley black dragon corridor stand after preparation is acknowledged",
        BLACK_DRAGON_STAND,
        3,
    ));
    steps.push(start_catalog_step());
    steps.extend([
        jive_watch(
            "watch the script wield the magic shortbow",
            Proof::EquipmentId {
                id: MAGIC_SHORTBOW_ID,
            },
            300,
        ),
        jive_watch(
            "watch Ranged XP from the black dragon fight",
            Proof::StatXpGain {
                id: RANGED_STAT,
                min: 1,
            },
            900,
        ),
        jive_watch(
            "watch the kill's dragon bones buried for Prayer XP",
            Proof::StatXpGain {
                id: PRAYER_STAT_ID,
                min: 72,
            },
            1800,
        ),
        jive_watch(
            "watch the black dragonhide reach the pack",
            Proof::ItemId {
                id: BLACK_DRAGONHIDE_ID,
                count: 1,
            },
            300,
        ),
    ]);
    // One 190-HP black dragon at 85 Ranged from the corridor stand.
    jive_scenario(
        "jive_dragons_black_range",
        "JiveDragons",
        steps,
        Proof::StatXpGain {
            id: PRAYER_STAT_ID,
            min: 72,
        },
        Duration::from_secs(1200),
        JIVE_DRAGONS_BLACK_INJECT,
        combat_fixture_loadouts("GreenDragon"),
    )
}

const JIVE_DRAGONS_BLACK_INJECT: &[ScriptSettingInject] = &[
    inj_str("loadout", JIVE_FOOD_LOADOUT),
    inj_str("site", "taverley-black"),
    inj_num("stand", 1.0),
    inj_str("combatStyle", "range"),
    inj_str("bow", "Magic shortbow"),
    inj_str("ammo", "Rune arrow"),
    inj_str("rangeStyle", "rapid"),
    inj_num("ammoWithdraw", 500.0),
    inj_bool("useSpecial", true),
    inj_bool("rangingPotion", true),
    inj_num("foodWithdraw", 20.0),
    inj_num("panicHp", 30.0),
    inj_num("foodReserve", 4.0),
    inj_num("healTo", 90.0),
    inj_num("teleStock", 2.0),
    inj_bool("buryBones", true),
    inj_bool("solveClues", false),
    inj_bool("bankCommonJunk", true),
    inj_bool("usePotions", false),
    inj_str("leaveVia", "teleport"),
    inj_list("lootBlack", DRAGON_LOOT),
    inj_str("logDetail", "Verbose"),
];

/// Ported from rs2b0t `e2e/jivedragons-potion-restock-live.ts` @2de6bb23.
/// Changed: upstream stops the script after its first bank trip and reads
/// pack+bank through the page API; here the same end state is watched
/// while the script runs (fresh three-dose supers held, partials gone
/// from pack, exact 35 banked lobsters: 20 seeded plus 20 noted deposited
/// minus 5 withdrawn, upstream's `assert.equal(result.foodBanked, 35)` at
/// `jivedragons-potion-restock-live.ts:59`). The `loadout` is the
/// existing GreenDragon scenario food loadout: its carry names no
/// potions, so the script's own fallback (one three-dose flask of each
/// super) is the behaviour under test, exactly as upstream's blank
/// loadout. The `>= 35` bank watch is armed first so it observes the
/// script's own open bank session; the closing `<= 35` watch re-reads
/// that same open bank after the withdraw watches, so the pair pins the
/// end count at exactly 35 (the `>=` watch alone already passes at the
/// 40 the noted deposit makes, before the 5 are withdrawn).
pub(crate) fn jive_dragons_potion_restock_scenario() -> Scenario {
    let bank = FALADOR_WEST_BANK;
    let mut cmds: Vec<String> = vec!["~clearinv".to_string(), "~clearbank".to_string()];
    cmds.extend([
        "givebank lobster 20".to_string(),
        "givebank dusty_key 1".to_string(),
        "givebank 3dose2attack 2".to_string(),
        "givebank 3dose2strength 2".to_string(),
        "givebank 3dose2defense 2".to_string(),
        "give bronze_sword 1".to_string(),
        "give antidragonbreathshield 1".to_string(),
        "give 1dose2attack 1".to_string(),
        "give 2dose2strength 1".to_string(),
        "give 3dose2defense 1".to_string(),
        "give cert_lobster 20".to_string(),
        tele_args(bank.level, bank.x, bank.z),
    ]);
    let mut steps = script_live_seed_steps();
    steps.push(jive_seed(
        "seed partial supers and noted lobsters in pack, fresh supers in the Falador West bank",
        cmds,
        Proof::ArrivedNear {
            x: bank.x,
            z: bank.z,
            level: bank.level,
            radius: 6,
        },
    ));
    steps.extend([
        bank_fletcher_watch(
            "acknowledge the partial super attack(1) in pack before Start",
            Proof::ItemId {
                id: SUPER_ATTACK_1_ID,
                count: 1,
            },
        ),
        bank_fletcher_watch(
            "acknowledge the partial super strength(2) in pack before Start",
            Proof::ItemId {
                id: SUPER_STRENGTH_2_ID,
                count: 1,
            },
        ),
        bank_fletcher_watch(
            "acknowledge twenty noted lobsters in pack before Start",
            Proof::ItemId {
                id: NOTED_LOBSTER_ID,
                count: 20,
            },
        ),
        bank_fletcher_watch(
            "confirm no unnoted lobster food in pack before Start",
            Proof::ItemIdAtMost {
                id: LOBSTER_ID,
                count: 0,
            },
        ),
    ]);
    steps.push(wear_combat_item_step(
        "wear and acknowledge the bronze sword before Start",
        BRONZE_SWORD_ID,
    ));
    steps.push(wear_combat_item_step(
        "wear and acknowledge the dragonfire shield before Start",
        DRAGONFIRE_SHIELD_ID,
    ));
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the two fresh super attack(3) seed bank",
        Proof::BankItemId {
            id: SUPER_ATTACK_3_ID,
            count: 2,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the twenty lobster seed bank",
        Proof::BankItemId {
            id: LOBSTER_ID,
            count: 20,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    steps.extend([
        jive_watch(
            "watch the restock leave exactly thirty-five lobsters in the open bank",
            Proof::BankItemId {
                id: LOBSTER_ID,
                count: 35,
            },
            450,
        ),
        jive_watch(
            "watch a fresh super attack(3) held",
            Proof::ItemId {
                id: SUPER_ATTACK_3_ID,
                count: 1,
            },
            300,
        ),
        jive_watch(
            "watch a fresh super strength(3) held",
            Proof::ItemId {
                id: SUPER_STRENGTH_3_ID,
                count: 1,
            },
            150,
        ),
        jive_watch(
            "watch the full super defence(3) retained",
            Proof::ItemId {
                id: SUPER_DEFENCE_3_ID,
                count: 1,
            },
            150,
        ),
        jive_watch(
            "watch the partial super attack(1) leave the pack",
            Proof::ItemIdAtMost {
                id: SUPER_ATTACK_1_ID,
                count: 0,
            },
            150,
        ),
        jive_watch(
            "watch the partial super strength(2) leave the pack",
            Proof::ItemIdAtMost {
                id: SUPER_STRENGTH_2_ID,
                count: 0,
            },
            150,
        ),
        jive_watch(
            "watch five restocked lobsters held",
            Proof::ItemId {
                id: LOBSTER_ID,
                count: 5,
            },
            150,
        ),
        jive_watch(
            "watch the noted lobsters leave the pack",
            Proof::ItemIdAtMost {
                id: NOTED_LOBSTER_ID,
                count: 0,
            },
            150,
        ),
        jive_watch(
            "watch the open bank hold at most thirty-five lobsters after the withdraw",
            Proof::BankItemIdAtMost {
                id: LOBSTER_ID,
                count: 35,
            },
            150,
        ),
    ]);
    // One at-booth restock: upstream allows its bank trip 60 s.
    jive_scenario(
        "jive_dragons_potion_restock",
        "JiveDragons",
        steps,
        Proof::ItemId {
            id: SUPER_ATTACK_3_ID,
            count: 1,
        },
        Duration::from_secs(300),
        JIVE_DRAGONS_POTION_INJECT,
        combat_fixture_loadouts("GreenDragon"),
    )
}

/// The potion-restock bank trip with edible Lobsters beside the noted ones
/// (`FIX-024-COMBAT-BANK-R3`): three edible Lobsters are given before the
/// twenty notes, so the edible row comes first on the bank side. The trip
/// must deposit only the noted row (bank 20 + 20 = 40), keep the three
/// edible Lobsters and withdraw two more to `foodWithdraw` 5 (bank 38). A
/// name-only deposit takes the edible row instead (bank 23, then 18 after a
/// five-Lobster withdraw) and keeps the notes. The edible food means the
/// card's food check does not start the trip: the Bronze sword waits in the
/// bank, and the melee card banks for its missing weapon (frozen
/// `JiveDragons.ts` `needStyleSupplies`, `BankRun.validate`).
pub(crate) fn jive_dragons_mixed_food_restock_scenario() -> Scenario {
    let bank = FALADOR_WEST_BANK;
    let mut cmds: Vec<String> = vec!["~clearinv".to_string(), "~clearbank".to_string()];
    cmds.extend([
        "givebank lobster 20".to_string(),
        "givebank dusty_key 1".to_string(),
        "givebank bronze_sword 1".to_string(),
        "givebank 3dose2attack 2".to_string(),
        "givebank 3dose2strength 2".to_string(),
        "givebank 3dose2defense 2".to_string(),
        "give antidragonbreathshield 1".to_string(),
        "give 1dose2attack 1".to_string(),
        "give 2dose2strength 1".to_string(),
        "give 3dose2defense 1".to_string(),
        "give lobster 3".to_string(),
        "give cert_lobster 20".to_string(),
        tele_args(bank.level, bank.x, bank.z),
    ]);
    let mut steps = script_live_seed_steps();
    steps.push(jive_seed(
        "seed edible and noted lobsters in pack, the sword and fresh supers in the Falador West bank",
        cmds,
        Proof::ArrivedNear {
            x: bank.x,
            z: bank.z,
            level: bank.level,
            radius: 6,
        },
    ));
    steps.extend([
        bank_fletcher_watch(
            "acknowledge twenty noted lobsters in pack before Start",
            Proof::ItemId {
                id: NOTED_LOBSTER_ID,
                count: 20,
            },
        ),
        bank_fletcher_watch(
            "acknowledge three edible lobsters in pack before Start",
            Proof::ItemId {
                id: LOBSTER_ID,
                count: 3,
            },
        ),
        bank_fletcher_watch(
            "confirm no more than three edible lobsters in pack before Start",
            Proof::ItemIdAtMost {
                id: LOBSTER_ID,
                count: 3,
            },
        ),
    ]);
    steps.push(wear_combat_item_step(
        "wear and acknowledge the dragonfire shield before Start",
        DRAGONFIRE_SHIELD_ID,
    ));
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the twenty lobster seed bank",
        Proof::BankItemId {
            id: LOBSTER_ID,
            count: 20,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the bronze sword in the seed bank",
        Proof::BankItemId {
            id: BRONZE_SWORD_ID,
            count: 1,
        },
    ));
    steps.push(bank_fletcher_watch(
        "confirm no more than twenty lobsters in the seed bank",
        Proof::BankItemIdAtMost {
            id: LOBSTER_ID,
            count: 20,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    steps.extend([
        jive_watch(
            "watch the noted deposit leave at least thirty-eight lobsters in the open bank",
            Proof::BankItemId {
                id: LOBSTER_ID,
                count: 38,
            },
            450,
        ),
        jive_watch(
            "watch the noted lobsters leave the pack",
            Proof::ItemIdAtMost {
                id: NOTED_LOBSTER_ID,
                count: 0,
            },
            150,
        ),
        jive_watch(
            "watch five edible lobsters held",
            Proof::ItemId {
                id: LOBSTER_ID,
                count: 5,
            },
            150,
        ),
        jive_watch(
            "watch the open bank hold at most thirty-eight lobsters after the withdraw",
            Proof::BankItemIdAtMost {
                id: LOBSTER_ID,
                count: 38,
            },
            150,
        ),
        jive_watch(
            "watch the bronze sword worn after the trip",
            Proof::EquipmentId {
                id: BRONZE_SWORD_ID,
            },
            300,
        ),
    ]);
    // The same at-booth restock as `jive_dragons_potion_restock`.
    jive_scenario(
        "jive_dragons_mixed_food_restock",
        "JiveDragons",
        steps,
        Proof::ItemId {
            id: LOBSTER_ID,
            count: 5,
        },
        Duration::from_secs(300),
        JIVE_DRAGONS_POTION_INJECT,
        combat_fixture_loadouts("GreenDragon"),
    )
}

const JIVE_DRAGONS_POTION_INJECT: &[ScriptSettingInject] = &[
    inj_str("loadout", JIVE_FOOD_LOADOUT),
    inj_str("site", "taverley-blue"),
    inj_str("combatStyle", "melee"),
    inj_str("weapon", "Bronze sword"),
    inj_bool("usePotions", true),
    inj_bool("useSpecial", false),
    inj_bool("solveClues", false),
    inj_num("foodWithdraw", 5.0),
    inj_str("leaveVia", "walk"),
];

/// Ported from rs2b0t `e2e/jivedemons-live.ts` @2de6bb23 (default style
/// range, no banked key).
/// Changed: started on the pocket safespot with the Dusty key and the
/// Falador escape cast carried. The upstream cold-key leg (bank read,
/// Jailer fight, Jail key) is the `acquire_key_v2_ts` cell's behaviour;
/// carrying the key is what upstream's `--dusty` banked-key variant
/// proves, and without a carried key or cast a cell started inside the
/// pocket could never leave it. Upstream's kill/loot/banktrip counter
/// milestones become Ranged XP only: a black demon's only guaranteed
/// drop is Ashes (`all.npc` `[black_demon]` `param=death_drop,ashes`;
/// `black_demon.rs2` rolls the rest of the table), and upstream's loot
/// list deliberately excludes Ashes, so no banked-loot watch can be a
/// non-chance oracle for this cell. Its safespot-soak and starve
/// gates are timing/post-Start-cheat oracles and are not ported.
/// As in `jive_dragons`, `foodReserve` is pinned to the seeded 20 so the
/// bank trip fires on loot fill within the deadline.
pub(crate) fn jive_demons_scenario() -> Scenario {
    let mut cmds = stat_cmds(&[
        ("attack", 75),
        ("strength", 75),
        ("defence", 75),
        ("hitpoints", 99),
        ("ranged", 85),
        ("magic", 80),
    ]);
    cmds.extend([
        "givebank lobster 400".to_string(),
        "givebank lawrune 200".to_string(),
        "givebank airrune 5000".to_string(),
        "givebank waterrune 200".to_string(),
        "givebank magic_shortbow 1".to_string(),
        "givebank rune_arrow 5000".to_string(),
        "give magic_shortbow 1".to_string(),
        "give rune_arrow 500".to_string(),
        "give lobster 20".to_string(),
        "give dusty_key 1".to_string(),
        "give airrune 30".to_string(),
        "give waterrune 10".to_string(),
        "give lawrune 10".to_string(),
    ]);
    let mut steps = script_live_seed_steps();
    steps.push(jive_seed(
        "seed the ranged demon stat line, stocked pack and bank on the safe tile",
        cmds,
        Proof::Stat {
            id: RANGED_STAT,
            min: 85,
        },
    ));
    steps.extend([
        bank_fletcher_watch(
            "acknowledge the carried magic shortbow before Start",
            Proof::ItemId {
                id: MAGIC_SHORTBOW_ID,
                count: 1,
            },
        ),
        bank_fletcher_watch(
            "acknowledge twenty carried lobsters before Start",
            Proof::ItemId {
                id: LOBSTER_ID,
                count: 20,
            },
        ),
        bank_fletcher_watch(
            "confirm no seeded coins in pack before Start",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
    ]);
    steps.push(jive_tele(
        "teleport to the Taverley black demon pocket safespot after preparation is acknowledged",
        BLACK_DEMON_SAFESPOT,
        3,
    ));
    steps.push(start_catalog_step());
    steps.extend([
        jive_watch(
            "watch the script wield the magic shortbow",
            Proof::EquipmentId {
                id: MAGIC_SHORTBOW_ID,
            },
            300,
        ),
        jive_watch(
            "watch Ranged XP from the demon fight",
            Proof::StatXpGain {
                id: RANGED_STAT,
                min: 1,
            },
            900,
        ),
    ]);
    // The port's 1500 s deadline, kept: seed, tele and the first demon
    // fight fit well inside it, and no bank leg is gated (see above).
    jive_scenario(
        "jive_demons",
        "JiveDemons",
        steps,
        Proof::StatXpGain {
            id: RANGED_STAT,
            min: 1,
        },
        Duration::from_secs(1500),
        JIVE_DEMONS_INJECT,
        combat_fixture_loadouts("GreenDragon"),
    )
}

const JIVE_DEMONS_INJECT: &[ScriptSettingInject] = &[
    inj_str("loadout", JIVE_FOOD_LOADOUT),
    inj_str("site", "taverley-black-demon"),
    inj_str("combatStyle", "range"),
    inj_str("bow", "Magic shortbow"),
    inj_str("ammo", "Rune arrow"),
    inj_str("rangeStyle", "rapid"),
    inj_num("ammoWithdraw", 500.0),
    inj_num("foodWithdraw", 20.0),
    inj_num("panicHp", 30.0),
    inj_num("foodReserve", 20.0),
    inj_num("healTo", 90.0),
    inj_num("teleStock", 2.0),
    inj_bool("bankCommonJunk", false),
    inj_str("leaveVia", "teleport"),
    inj_list(
        "loot",
        &[
            "Coins",
            "Rune chainbody",
            "Rune med helm",
            "Blood rune",
            "Law rune",
            "Chaos rune",
            "Herb",
            "Uncut diamond",
            "Uncut ruby",
            "Uncut emerald",
            "Uncut sapphire",
        ],
    ),
    inj_str("logDetail", "Verbose"),
];

/// Ported from rs2b0t `e2e/jivekbd-live.ts` @2de6bb23 (defaults: Fire
/// Blast, teleport out, start at Edgeville bank).
/// Changed: upstream's dose-trace/alcove-soak/starve milestones are
/// log-and-trace oracles; the same legs are proven here by the worn
/// banked Dragonfire shield (the script must withdraw it at Edgeville:
/// the pack does not carry it), arrival on the lair alcove tile (the
/// Wilderness ladder/lever entry upstream's `dose`/`dungeon`/`lair`
/// milestones sequence), Magic XP from casting, and the King Black
/// Dragon's guaranteed Dragon bones reaching the pack (always on its
/// table, rs2b0t `src/bot/data/dropdb.ts:52` "King black dragon").
/// The run keeps upstream's full entry leg: no teleport into the lair.
pub(crate) fn jive_kbd_scenario() -> Scenario {
    let bank = EDGEVILLE_BANK;
    let mut cmds = stat_cmds(&[
        ("attack", 75),
        ("strength", 75),
        ("defence", 75),
        ("hitpoints", 99),
        ("magic", 80),
    ]);
    cmds.extend([
        "givebank lobster 400".to_string(),
        "givebank airrune 5000".to_string(),
        "givebank deathrune 5000".to_string(),
        "givebank chaosrune 5000".to_string(),
        "givebank firerune 500".to_string(),
        "givebank 4dose2antipoison 20".to_string(),
        "givebank staff_of_fire 1".to_string(),
        "givebank antidragonbreathshield 1".to_string(),
        "givebank lawrune 200".to_string(),
        "give staff_of_fire 1".to_string(),
        "give airrune 1200".to_string(),
        "give deathrune 300".to_string(),
        "give chaosrune 300".to_string(),
        "give firerune 20".to_string(),
        "give 4dose2antipoison 1".to_string(),
        "give lobster 20".to_string(),
        "give lawrune 6".to_string(),
        tele_args(bank.level, bank.x, bank.z),
    ]);
    let mut steps = script_live_seed_steps();
    steps.push(jive_seed(
        "seed the KBD mage stat line, stocked pack and Edgeville bank, stand at the bank",
        cmds,
        Proof::ArrivedNear {
            x: bank.x,
            z: bank.z,
            level: bank.level,
            radius: 6,
        },
    ));
    steps.extend([
        bank_fletcher_watch(
            "acknowledge Magic 80 before Start",
            Proof::Stat {
                id: MAGIC_STAT,
                min: 80,
            },
        ),
        bank_fletcher_watch(
            "acknowledge the carried staff of fire before Start",
            Proof::ItemId {
                id: STAFF_OF_FIRE_ID,
                count: 1,
            },
        ),
        bank_fletcher_watch(
            "acknowledge twenty carried lobsters before Start",
            Proof::ItemId {
                id: LOBSTER_ID,
                count: 20,
            },
        ),
        bank_fletcher_watch(
            "confirm the dragonfire shield is bank-only, not carried, before Start",
            Proof::ItemIdAtMost {
                id: DRAGONFIRE_SHIELD_ID,
                count: 0,
            },
        ),
        bank_fletcher_watch(
            "confirm no seeded dragon bones in pack before Start",
            Proof::ItemIdAtMost {
                id: DRAGON_BONES_ID,
                count: 0,
            },
        ),
    ]);
    steps.push(start_catalog_step());
    steps.extend([
        jive_watch(
            "watch the banked dragonfire shield get withdrawn and worn",
            Proof::EquipmentId {
                id: DRAGONFIRE_SHIELD_ID,
            },
            600,
        ),
        jive_watch(
            "watch the Wilderness entry end on the lair alcove",
            Proof::ArrivedNear {
                x: KBD_ALCOVE.x,
                z: KBD_ALCOVE.z,
                level: KBD_ALCOVE.level,
                radius: 3,
            },
            1800,
        ),
        jive_watch(
            "watch Magic XP from Fire Blast casts at the dragon",
            Proof::StatXpGain {
                id: MAGIC_STAT,
                min: 34,
            },
            900,
        ),
        jive_watch(
            "watch the kill's dragon bones reach the pack",
            Proof::ItemId {
                id: DRAGON_BONES_ID,
                count: 1,
            },
            3600,
        ),
    ]);
    // Upstream's own default budget is 45 minutes: Edgeville restock, the
    // Wilderness walk/ladder/lever entry, and a 240-HP dragon whose magic
    // defence makes a kill a few hundred Fire Blast casts, possibly across
    // a rune restock trip.
    jive_scenario(
        "jive_kbd",
        "JiveKBD",
        steps,
        Proof::StatXpGain {
            id: MAGIC_STAT,
            min: 34,
        },
        Duration::from_secs(2700),
        JIVE_KBD_INJECT,
        combat_fixture_loadouts("GreenDragon"),
    )
}

const JIVE_KBD_INJECT: &[ScriptSettingInject] = &[
    inj_str("loadout", JIVE_FOOD_LOADOUT),
    inj_str("staff", "Staff of fire"),
    inj_str("spell", "Fire Blast"),
    inj_num("runesWithdraw", 150.0),
    inj_num("runeBuffer", 300.0),
    inj_num("foodWithdraw", 20.0),
    inj_num("dosesWithdraw", 1.0),
    inj_num("panicHp", 30.0),
    inj_num("foodReserve", 4.0),
    inj_num("healTo", 90.0),
    inj_str("site", "kbd-lair"),
    inj_num("teleStock", 2.0),
    inj_bool("bankCommonJunk", false),
    inj_list(
        "loot",
        &[
            "Dragon bones",
            "Dragonhide",
            "Coins",
            "Rune longsword",
            "Adamant platebody",
            "Dragon med helm",
            "Adamant axe",
            "Rune axe",
            "Mithril battleaxe",
            "Amulet of strength",
            "Bronze arrow",
            "Iron arrow",
            "Air rune",
            "Fire rune",
            "Blood rune",
            "Law rune",
            "Death rune",
            "Adamantite bar",
            "Runite bar",
            "Coal",
            "Iron ore",
            "Yew logs",
            "Shark",
            "Lobster",
            "Oyster pearls",
            "Uncut diamond",
            "Uncut ruby",
            "Uncut emerald",
            "Uncut sapphire",
        ],
    ),
    inj_str("logDetail", "Verbose"),
];
