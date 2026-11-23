//! Real-content regressions for clue arrival, including the Port Sarim boat.
#![cfg(all(feature = "live-harness", feature = "test-support"))]

#[path = "common/quester_live.rs"]
mod quester_live;

use api::snapshot::{GameSnapshot, WorldTile};
use quester_live::{Cell, Mode};
use scenario::{Proof, Step, StepKind, Wait};
use serde_json::json;
use std::sync::{Arc, OnceLock};

const ENTRANA_CLUE: i32 = 3579;
const SEARCH_CLUE: i32 = 2679;
const TRAIL_COMPLETE: &str = "Well done you've completed the Treasure Trail!";
const ANOTHER_CLUE: &str = "You've found another clue scroll!";
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
        && trail_item_outcome(snapshot, seeded).is_some()
}

fn trail_item_outcome(snapshot: &GameSnapshot, seeded: i32) -> Option<&'static str> {
    snapshot.inventory().iter().find_map(|item| {
        if item.count <= 0 || item.def.id == seeded {
            return None;
        }
        match item.def.name.as_deref() {
            Some("Clue scroll") => Some("next_clue"),
            Some("Casket") => Some("casket"),
            _ => None,
        }
    })
}

fn trail_completion_chat(snapshot: &GameSnapshot) -> bool {
    snapshot
        .chat_lines()
        .iter()
        .any(|line| line.type_ == 0 && line.username.is_none() && line.text == TRAIL_COMPLETE)
}

fn next_clue_dialog(snapshot: &GameSnapshot) -> bool {
    snapshot
        .chat_modal_texts()
        .iter()
        .any(|text| text == ANOTHER_CLUE)
}

fn ordinary_search_outcome(snapshot: &GameSnapshot) -> Option<&'static str> {
    if !snapshot.ingame() || snapshot.scene_state() != 2 || !snapshot.inventory_ready() {
        return None;
    }
    // The easy-clue draw includes the seeded ID. Only the engine's result
    // dialog can distinguish that draw from a search that did not progress.
    if held(snapshot, SEARCH_CLUE) != 0 {
        return next_clue_dialog(snapshot).then_some("same_clue_redrawn");
    }
    trail_item_outcome(snapshot, SEARCH_CLUE).or_else(|| {
        // The pre-Start fixture requires a pack with only the seeded clue.
        // Completion can post in chat before the engine transfers the loot.
        (trail_completion_chat(snapshot) && snapshot.inv().iter().any(|&(_, count)| count > 0))
            .then_some("trail_completed")
    })
}

#[cfg(test)]
mod ordinary_search_tests {
    use super::*;
    use api::obj_names::ItemDefView;
    use api::snapshot::{ChatLineView, ItemActionFamily, ItemContainer, ItemView};

    fn frame(pack: &[(i32, &str)]) -> GameSnapshot {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_transmission_ready(true, false);
        snapshot.seed_inventory(
            pack.iter()
                .enumerate()
                .map(|(slot, &(id, name))| ItemView {
                    def: ItemDefView {
                        id,
                        name: Some(name.into()),
                        stackable: false,
                        members: true,
                        base_value: 0,
                        noted: false,
                        certificate_link: -1,
                        certificate_template: -1,
                    },
                    container: ItemContainer::Inventory,
                    action_family: ItemActionFamily::Held,
                    slot: slot as i32,
                    count: 1,
                    actions: Vec::new(),
                    component_id: -1,
                })
                .collect(),
            28,
        );
        snapshot
    }

    fn set_completion_chat(snapshot: &mut GameSnapshot) {
        snapshot.seed_chat_lines(vec![ChatLineView {
            type_: 0,
            username: None,
            text: TRAIL_COMPLETE.into(),
            sequence: 1,
        }]);
    }

    #[test]
    fn accepts_a_new_scroll_or_casket() {
        for (id, name, outcome) in [
            (2681, "Clue scroll", "next_clue"),
            (2824, "Casket", "casket"),
        ] {
            assert_eq!(
                ordinary_search_outcome(&frame(&[(id, name)])),
                Some(outcome)
            );
        }
    }

    #[test]
    fn accepts_completion_with_rewards_and_no_seeded_clue() {
        let mut snapshot = frame(&[(1125, "Black platebody"), (556, "Air rune")]);
        set_completion_chat(&mut snapshot);
        assert_eq!(ordinary_search_outcome(&snapshot), Some("trail_completed"));
    }

    #[test]
    fn accepts_a_same_id_redraw_with_the_search_dialog() {
        let mut snapshot = frame(&[(SEARCH_CLUE, "Clue scroll")]);
        snapshot.seed_chat_modal(1, vec![ANOTHER_CLUE.into()]);
        assert_eq!(
            ordinary_search_outcome(&snapshot),
            Some("same_clue_redrawn")
        );
    }

    #[test]
    fn rejects_a_held_seed_or_incomplete_completion_evidence() {
        assert_eq!(
            ordinary_search_outcome(&frame(&[(SEARCH_CLUE, "Clue scroll"), (2824, "Casket")])),
            None
        );
        let mut held = frame(&[(SEARCH_CLUE, "Clue scroll"), (1125, "Black platebody")]);
        set_completion_chat(&mut held);
        assert_eq!(ordinary_search_outcome(&held), None);
        assert_eq!(
            ordinary_search_outcome(&frame(&[(1125, "Black platebody")])),
            None
        );
        let mut empty = frame(&[]);
        assert_eq!(ordinary_search_outcome(&empty), None);
        set_completion_chat(&mut empty);
        assert_eq!(ordinary_search_outcome(&empty), None);
    }

    #[test]
    fn rejects_an_unready_snapshot() {
        let mut snapshot = frame(&[(2681, "Clue scroll")]);
        snapshot.seed_transmission_ready(false, false);
        assert_eq!(ordinary_search_outcome(&snapshot), None);
        snapshot.seed_transmission_ready(true, false);
        snapshot.seed_ingame(1);
        assert_eq!(ordinary_search_outcome(&snapshot), None);
    }
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
                // Post-relog: only a reply newer than this step's baseline
                // proves the new session. A pre-relog `get tutorial: 1000`
                // line retained in the ring must not pass it (the
                // `FreshTutorial` rule in `scenario/src/tutorial.rs`).
                arm: Proof::FreshTutorial,
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
    let search_receipt = Arc::new(OnceLock::new());
    let watch_receipt = Arc::clone(&search_receipt);
    let watch = fixture.steps.last_mut().unwrap();
    assert!(matches!(
        watch.wait.arm,
        Proof::ClueReplaced {
            seeded: SEARCH_CLUE
        }
    ));
    watch.name = "watch Sherlock advance the seeded search clue";
    watch.kind = StepKind::Await {
        evidence: "clue_search_progress(2679)",
        ready: Box::new(move |snapshot| {
            if let Some(outcome) = ordinary_search_outcome(snapshot) {
                watch_receipt.get_or_init(|| json!({"outcome": outcome, "tile": snapshot.tile()}));
            }
            watch_receipt.get().is_some()
        }),
        summary: None,
    };
    // Await records the outcome on the game frame that proves it. A modal or
    // trail item can disappear before the family observer reads that receipt.
    watch.wait.arm = Proof::IngameScene2;
    fixture.proof = Proof::IngameScene2;
    quester_live::run_family(
        Cell {
            quest: "clue-walk-arrive",
            display: "Sherlock ordinary search",
            label: "ordinary".into(),
            scenario: fixture,
            start_settings: Default::default(),
            mode: Mode::Clean,
            observe_start: Some(Box::new(|snapshot| {
                if snapshot.inv() != [(SEARCH_CLUE, 1)]
                    || snapshot.tile() != Some((3256, 3236, 0))
                    || trail_completion_chat(snapshot)
                    || next_clue_dialog(snapshot)
                {
                    return Err("Start requires only the seeded search clue ten tiles from the mainland target, with no prior search outcome".into());
                }
                Ok(json!({"clue": SEARCH_CLUE, "tile": snapshot.tile()}))
            })),
        },
        Box::new(|handle, account, _| {
            handle.start_compiled(account, script::CompiledId("Sherlock"), Default::default())
        }),
        Box::new(move |_, _, _| Ok(search_receipt.get().cloned())),
    )
    .expect("Sherlock must advance the ordinary search clue within the fixed deadline");
}
