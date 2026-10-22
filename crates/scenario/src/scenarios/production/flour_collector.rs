use super::*;

/// Empty pot (`pot_empty`, rs2b0t `274.json` id 1931).
const POT_EMPTY_ID: i32 = 1931;
/// Pot of flour (`pot_flour`, rs2b0t `274.json` id 1933).
const POT_FLOUR_ID: i32 = 1933;
/// Empty pots banked at Seers: one full pack plus the restock.
const FLOUR_SEED_POTS: i32 = 30;
const FLOUR_PACK_POTS: i32 = 28;
const FLOUR_RESTOCK_POTS: i32 = FLOUR_SEED_POTS - FLOUR_PACK_POTS;
/// Per-leg cap in observed game ticks (one per `PLAYER_INFO`). A leg that
/// walks the Seers bank to the Sinclair flour barrel (about 90 tiles,
/// `FlourCollector.ts` `MURDER_LOC`) plus up to 28 barrel fills needs about
/// 90 game ticks of walking at one tile per game tick; 400 leaves room for
/// the fill loop and return walk.
const FLOUR_LEG_WATCH_TICKS: u32 = 400;
/// Whole-scenario cap: upstream's default run length (`minutes: 8`). It covers
/// three walks of about 90 tiles (bank to barrel, barrel to bank, bank to
/// barrel) at 600ms per game tick, plus 30 barrel fills.
pub(crate) const FLOUR_COLLECTOR_DEADLINE: Duration = Duration::from_secs(480);

fn flour_watch(name: &'static str, arm: Proof) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm,
            budget_ticks: FLOUR_LEG_WATCH_TICKS,
        },
    }
}

/// Ported from rs2b0t `e2e/flour-collector-310-live.ts @2de6bb23`.
/// - Quest gate: upstream's `setvar`, server `getvar`, relog, then
///   `Quests.status('Murder Mystery') === 'inProgress'`. Ours is the same
///   `setvar`/`getvar` pair with a `FreshChat` receipt before and after relog.
///   No in-progress quest proof exists (`QuestDone` is completion-only).
/// - Seed: `~clearinv`, `givebank pot_empty 30`, then tele to the stand (2724,3493),
///   two tiles in z from the Seers bank tile (2725,3491). Upstream's `seedItemsToBank`
///   teleports onto the bank tile itself (`tutorial/harness.ts:439`); ours opens the
///   nearest booth with `open_nearest_booth` (`api/src/interact.rs:688`), which refuses
///   a booth it cannot operate from (`api/tests/interact.rs:1885`).
/// - Oracle: upstream's `page.evaluate` counts become `ItemId`/`BankItemId`
///   proofs at the same counts. Its pause/resume snapshots become pre-Start
///   checks, and the restock gate becomes `ItemId pot_empty 2` after the flour
///   is banked. Upstream's screenshots and `pageerror` gate are dropped; the
///   runner writes its own terminal shot.
/// - Deadline: upstream's 8-minute default, not a timing oracle.
pub(crate) fn flour_collector_scenario() -> Scenario {
    let bank = SEERS_BANK;
    let bank_stand = WorldTile {
        x: 2724,
        z: 3493,
        level: 0,
    };
    let murderquest_in_progress = Proof::FreshChat {
        needle: "get murderquest: 1",
    };
    let mut steps: Vec<Step> = Vec::new();
    steps.extend(script_live_seed_steps());
    steps.push(Step {
        name: "set Murder Mystery in progress by the authentic setvar and read it back",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "setvar murderquest 1");
                cheat(c, "getvar murderquest");
                true
            }),
        },
        wait: Wait {
            arm: murderquest_in_progress,
            budget_ticks: 200,
        },
    });
    steps.push(Step {
        name: "relog so the quest varp must persist server-side",
        kind: StepKind::Relog,
        wait: Wait {
            arm: Proof::SideTabAvailable { index: 3 },
            budget_ticks: 600,
        },
    });
    steps.push(Step {
        name: "read Murder Mystery back after relog",
        kind: StepKind::Repeat {
            send: Box::new(|c, _| {
                cheat(c, "getvar murderquest");
                true
            }),
        },
        wait: Wait {
            arm: murderquest_in_progress,
            budget_ticks: 200,
        },
    });
    steps.push(Step {
        name: "seed 30 empty pots at Seers bank: clear pack, bank pots, tele to bank",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("givebank pot_empty {FLOUR_SEED_POTS}"));
                cheat(c, &tele_args(bank_stand.level, bank_stand.x, bank_stand.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank_stand.x,
                z: bank_stand.z,
                level: bank_stand.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm no seeded empty pots in pack before Start",
            Proof::ItemIdAtMost {
                id: POT_EMPTY_ID,
                count: 0,
            },
        ),
        (
            "confirm no flour in pack before Start",
            Proof::ItemIdAtMost {
                id: POT_FLOUR_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the exact empty-pot seed bank",
        Proof::BankItemId {
            id: POT_EMPTY_ID,
            count: FLOUR_SEED_POTS,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no flour in the seed bank before Start",
        Proof::BankItemIdAtMost {
            id: POT_FLOUR_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    let second_load = Proof::ItemId {
        id: POT_FLOUR_ID,
        count: 1,
    };
    for (step_name, arm) in [
        (
            "watch the script withdraw a full pack of 28 empty pots",
            Proof::ItemId {
                id: POT_EMPTY_ID,
                count: FLOUR_PACK_POTS,
            },
        ),
        (
            "watch all 28 pots filled with flour at the Sinclair barrel",
            Proof::ItemId {
                id: POT_FLOUR_ID,
                count: FLOUR_PACK_POTS,
            },
        ),
        (
            "watch the player walk back to the Seers bank",
            Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 6,
            },
        ),
        (
            "watch the 28 flour pots banked at Seers",
            Proof::BankItemId {
                id: POT_FLOUR_ID,
                count: FLOUR_PACK_POTS,
            },
        ),
        (
            "watch the pack hold no flour after banking",
            Proof::ItemIdAtMost {
                id: POT_FLOUR_ID,
                count: 0,
            },
        ),
        (
            "watch the script restock the 2 remaining empty pots",
            Proof::ItemId {
                id: POT_EMPTY_ID,
                count: FLOUR_RESTOCK_POTS,
            },
        ),
        ("watch the second load begin: flour in pack", second_load),
    ] {
        steps.push(flour_watch(step_name, arm));
    }
    Scenario {
        name: "flour_collector",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: second_load,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: FLOUR_COLLECTOR_DEADLINE,
            start_script: Some("FlourCollector"),
            terminal_shot: Some("flour_collector"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
