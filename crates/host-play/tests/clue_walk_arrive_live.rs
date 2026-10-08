//! Real-content regressions for clue arrival, including the Port Sarim boat.
#![cfg(all(feature = "live-harness", feature = "test-support"))]

#[path = "common/quester_live.rs"]
mod quester_live;

use api::snapshot::{GameSnapshot, WorldTile};
use quester_live::{Cell, Mode};
use scenario::{Proof, Step, StepKind, Wait};
use serde_json::json;

const ENTRANA_CLUE: i32 = 3579;
const SEARCH_CLUE: i32 = 2679;
const SWORD: i32 = 1277;
const DRAYNOR: WorldTile = WorldTile {
    x: 3092,
    z: 3242,
    level: 0,
};

fn held(snapshot: &GameSnapshot, id: i32) -> i32 {
    snapshot
        .inv()
        .iter()
        .filter(|(item, _)| *item == id)
        .map(|(_, count)| count)
        .sum()
}

fn clue_replaced(snapshot: &GameSnapshot, seeded: i32) -> bool {
    snapshot.ingame()
        && snapshot.scene_state() == 2
        && held(snapshot, seeded) == 0
        && snapshot.inventory().iter().any(|item| {
            item.count > 0
                && item.def.id != seeded
                && matches!(
                    item.def.name.as_deref(),
                    Some("Clue scroll") | Some("Casket")
                )
        })
}

fn reseed_after_relog(scenario: &mut scenario::Scenario) {
    let after = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::Relog))
        .unwrap()
        + 1;
    scenario.steps.insert(
        after,
        Step {
            name: "restore tutorial completion after fresh-account relog",
            kind: StepKind::Perform {
                send: Box::new(|client, _| {
                    ["setvar tutorial 1000", "getvar tutorial"]
                        .into_iter()
                        .all(|command| {
                            api::interact::cheat(client, command) == client::CheatSend::Sent
                        })
                }),
            },
            wait: Wait {
                arm: Proof::Chat {
                    needle: "get tutorial: 1000",
                },
                budget_ticks: 200,
            },
        },
    );
}

fn entrana_fixture() -> scenario::Scenario {
    let mut fixture = scenario::get("sherlock_talk").unwrap();
    fixture.name = "sherlock_entrana_walk_arrive";
    reseed_after_relog(&mut fixture);
    let seed = fixture
        .steps
        .iter_mut()
        .find(|step| step.name == "seed the Sherlock talk pack")
        .unwrap();
    seed.kind = StepKind::Perform {
        send: Box::new(|client, _| {
            [
                "~clearinv",
                "give trail_clue_hard_riddle027 1",
                "give bronze_sword 1",
                "give spade 1",
            ]
            .into_iter()
            .all(|command| api::interact::cheat(client, command) == client::CheatSend::Sent)
        }),
    };
    seed.wait.arm = Proof::ItemId {
        id: ENTRANA_CLUE,
        count: 1,
    };
    let stand = fixture
        .steps
        .iter_mut()
        .find(|step| step.name == "teleport next to the first clue step")
        .unwrap();
    stand.kind = StepKind::Perform {
        send: Box::new(|client, _| {
            api::interact::seed_at(client, DRAYNOR.level, DRAYNOR.x, DRAYNOR.z);
            true
        }),
    };
    stand.wait.arm = Proof::Arrived {
        x: DRAYNOR.x,
        z: DRAYNOR.z,
        level: DRAYNOR.level,
    };
    fixture.steps.last_mut().unwrap().wait.arm = Proof::ClueReplaced {
        seeded: ENTRANA_CLUE,
    };
    fixture.proof = Proof::ClueReplaced {
        seeded: ENTRANA_CLUE,
    };
    fixture
}

fn entrana_proved(stripped: bool, boarded: bool, north_search: bool, replaced: bool) -> bool {
    stripped && boarded && north_search && replaced
}

#[test]
fn entrana_oracle_requires_every_real_route_and_search_witness() {
    assert!(entrana_proved(true, true, true, true));
    for missing in 0..4 {
        let mut facts = [true; 4];
        facts[missing] = false;
        assert!(!entrana_proved(facts[0], facts[1], facts[2], facts[3]));
    }
}

#[test]
#[ignore = "requires LIVE=1, Engine A paths, CPU rendering and isolated HOME"]
fn live_sherlock_entrana_walk_arrive() {
    assert!(std::env::var_os("QUESTER_TICK_MS").is_none());
    let mut stripped = false;
    let mut boarded = false;
    let mut north_search = false;
    quester_live::run_family(
        Cell {
            quest: "clue-walk-arrive", display: "Sherlock Entrana search", label: "entrana".into(),
            scenario: entrana_fixture(), start_settings: Default::default(), mode: Mode::Clean,
            observe_start: Some(Box::new(|snapshot| {
                if snapshot.tile() != Some((DRAYNOR.x, DRAYNOR.z, DRAYNOR.level)) || held(snapshot, ENTRANA_CLUE) != 1 || held(snapshot, SWORD) != 1 || held(snapshot, 952) != 1 {
                    return Err("Start requires the clue, sword and spade on the Draynor operable bank stand".into());
                }
                Ok(json!({"origin": DRAYNOR, "clue": ENTRANA_CLUE, "sword": SWORD, "spade": 952}))
            })),
        },
        Box::new(|handle, account, _| handle.start_compiled(account, script::CompiledId("Sherlock"), Default::default())),
        Box::new(move |snapshot, _, _| {
            stripped |= snapshot.tile().is_some_and(|(x, z, level)| (3088..=3095).contains(&x) && (3240..=3245).contains(&z) && level == 0) && held(snapshot, SWORD) == 0 && held(snapshot, ENTRANA_CLUE) == 1;
            boarded |= snapshot.tile().is_some_and(|(x, z, level)| (2802..=2878).contains(&x) && (3329..=3393).contains(&z) && (0..=1).contains(&level));
            // drawers2's east-only approach rotates north at placement angle 3.
            let replaced = clue_replaced(snapshot, ENTRANA_CLUE);
            north_search |= replaced && snapshot.tile() == Some((2818, 3352, 0));
            if entrana_proved(stripped, boarded, north_search, replaced) {
                return Ok(Some(json!({"stripped_at_draynor": stripped, "real_boat_arrival": boarded, "drawers_north_search": north_search, "clue_replaced": replaced, "tile": snapshot.tile()})));
            }
            Ok(None)
        }),
    ).expect("Sherlock must strip, take the real boat, search the drawers from the north stand and replace the clue within the fixed deadline");
}

#[test]
#[ignore = "requires LIVE=1, Engine A paths, CPU rendering and isolated HOME"]
fn live_sherlock_ordinary_search_walk_arrive() {
    assert!(std::env::var_os("QUESTER_TICK_MS").is_none());
    let mut fixture = scenario::get("sherlock_search").unwrap();
    reseed_after_relog(&mut fixture);
    let stand = fixture
        .steps
        .iter_mut()
        .find(|step| step.name == "teleport next to the first clue step")
        .unwrap();
    stand.kind = StepKind::Perform {
        send: Box::new(|client, _| {
            api::interact::seed_at(client, 0, 3256, 3236);
            true
        }),
    };
    stand.wait.arm = Proof::Arrived {
        x: 3256,
        z: 3236,
        level: 0,
    };
    quester_live::run_family(
        Cell {
            quest: "clue-walk-arrive",
            display: "Sherlock ordinary search",
            label: "ordinary".into(),
            scenario: fixture,
            start_settings: Default::default(),
            mode: Mode::Clean,
            observe_start: Some(Box::new(|snapshot| {
                if held(snapshot, SEARCH_CLUE) != 1 || snapshot.tile() != Some((3256, 3236, 0)) {
                    return Err(
                        "Start requires the ordinary search clue ten tiles from the mainland target"
                            .into(),
                    );
                }
                Ok(json!({"clue": SEARCH_CLUE, "tile": snapshot.tile()}))
            })),
        },
        Box::new(|handle, account, _| {
            handle.start_compiled(account, script::CompiledId("Sherlock"), Default::default())
        }),
        Box::new(|snapshot, _, _| {
            Ok(clue_replaced(snapshot, SEARCH_CLUE)
                .then(|| json!({"clue_replaced": true, "tile": snapshot.tile()})))
        }),
    )
    .expect("Sherlock must replace the ordinary search clue within the fixed deadline");
}
