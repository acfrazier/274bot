//! Sherlock must pass the real monk's category check, not a copied name filter.
#![cfg(all(feature = "live-harness", feature = "test-support"))]

#[path = "common/quester_live.rs"]
mod quester_live;

use api::snapshot::{GameSnapshot, WorldTile};
use quester_live::{Cell, Mode};
use scenario::{Proof, StepKind, Wait};
use serde_json::json;

const CLUE: i32 = 3579;
// content/scripts/skill_crafting/configs/jewellery/silver.obj:29-44:
// silver_sickle (2961) has category weapon_slash; the old regex missed it.
const BANNED: i32 = 2961;
// content/scripts/skill_fletching/configs/stringing/bows.obj:1-15:
// unstrung_longbow (48) is named Longbow but has category unstrung_bow,
// absent from monk_of_entrana.rs2:26-50; the old regex wrongly banked it.
const PERMITTED: i32 = 48;
const ORIGIN: WorldTile = WorldTile {
    x: 3092,
    z: 3243,
    level: 0,
};

fn inventory(snapshot: &GameSnapshot, id: i32) -> i32 {
    snapshot
        .inv()
        .iter()
        .filter(|(item, _)| *item == id)
        .map(|(_, count)| count)
        .sum()
}

fn boarding_proved(tile: Option<(i32, i32, i32)>, banned: i32, permitted: i32, clue: i32) -> bool {
    // monk_of_entrana.rs2:15-23 checks the gear then sails to
    // 1_44_52_18_3 (2834,3331,1). Also accept the immediately disembarked island.
    tile.is_some_and(|(x, z, level)| {
        (2802..=2878).contains(&x) && (3329..=3393).contains(&z) && (0..=1).contains(&level)
    }) && banned == 0
        && permitted == 1
        && clue == 1
}

fn fixture() -> scenario::Scenario {
    // Reuse the released Sherlock fixture's login/mainland preparation and
    // native Start. Replace only its account-local pack, origin and observer.
    let mut scenario = scenario::get("sherlock_talk").unwrap();
    scenario.name = "sherlock_entrana_content";
    let seed = scenario
        .steps
        .iter_mut()
        .find(|step| step.name == "seed the Sherlock talk pack")
        .unwrap();
    seed.name = "seed Entrana category counterexamples";
    seed.kind = StepKind::Perform {
        send: Box::new(|client, _| {
            [
                "~clearinv",
                "give trail_clue_hard_riddle027 1",
                "give silver_sickle 1",
                "give unstrung_longbow 1",
                "give spade 1",
            ]
            .into_iter()
            .all(|command| api::interact::cheat(client, command) == client::CheatSend::Sent)
        }),
    };
    seed.wait = Wait {
        arm: Proof::ItemId { id: CLUE, count: 1 },
        budget_ticks: 200,
    };
    let stand = scenario
        .steps
        .iter_mut()
        .find(|step| step.name == "teleport next to the first clue step")
        .unwrap();
    stand.kind = StepKind::Perform {
        send: Box::new(|client, _| {
            api::interact::seed_at(client, ORIGIN.level, ORIGIN.x, ORIGIN.z);
            true
        }),
    };
    stand.wait = Wait {
        arm: Proof::Arrived {
            x: ORIGIN.x,
            z: ORIGIN.z,
            level: ORIGIN.level,
        },
        budget_ticks: 200,
    };
    // The run_family observer below owns the actual success predicate. Keep
    // the existing post-Start watcher waiting for this clue, not the talk seed.
    scenario.steps.last_mut().unwrap().wait.arm = Proof::ClueReplaced { seeded: CLUE };
    scenario.proof = Proof::ClueReplaced { seeded: CLUE };
    scenario
}

#[test]
fn boarding_oracle_rejects_bad_gear_and_seed_only_evidence() {
    assert!(boarding_proved(Some((2834, 3331, 1)), 0, 1, 1));
    assert!(boarding_proved(Some((2834, 3331, 0)), 0, 1, 1));
    for input in [
        (Some((3092, 3243, 0)), 0, 1, 1),
        (Some((2834, 3331, 1)), 1, 1, 1),
        (Some((2834, 3331, 1)), 0, 0, 1),
        (Some((2834, 3331, 1)), 0, 1, 0),
        (None, 0, 1, 1),
    ] {
        assert!(!boarding_proved(input.0, input.1, input.2, input.3));
    }
}

#[test]
#[ignore = "requires LIVE=1, explicit Engine A paths, CPU rendering and isolated HOME"]
fn live_sherlock_entrana_content_categories() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert!(
        std::env::var_os("QUESTER_TICK_MS").is_none(),
        "final proof keeps Engine A at 600 ms"
    );
    quester_live::run_family(
        Cell {
            quest: "entrana-content",
            display: "Sherlock Entrana boarding",
            label: "content-categories".into(),
            scenario: fixture(),
            start_settings: Default::default(),
            mode: Mode::Clean,
            observe_start: Some(Box::new(|snapshot| {
                if !snapshot.ingame() || snapshot.scene_state() != 2 || snapshot.tile() != Some((ORIGIN.x, ORIGIN.z, ORIGIN.level))
                    || inventory(snapshot, CLUE) != 1 || inventory(snapshot, BANNED) != 1 || inventory(snapshot, PERMITTED) != 1 {
                    return Err("Entrana fixture must hold both category counterexamples and the clue at Draynor bank before Start".into());
                }
                Ok(json!({"origin":ORIGIN,"clue":CLUE,"banned_sickle":BANNED,"permitted_unstrung_bow":PERMITTED,"seeded_before_start":true}))
            })),
        },
        Box::new(|handle, account, _| handle.start_compiled(account, script::CompiledId("Sherlock"), Default::default())),
        Box::new(|snapshot, _, _| {
            let banned = inventory(snapshot, BANNED) + snapshot.equipment().iter().filter(|item| item.def.id == BANNED).map(|item| item.count).sum::<i32>();
            if boarding_proved(snapshot.tile(), banned, inventory(snapshot, PERMITTED), inventory(snapshot, CLUE)) {
                return Ok(Some(json!({"boarded_after_content_gear_check":true,"tile":snapshot.tile(),"banned_sickle_held":banned,"permitted_unstrung_bow_held":inventory(snapshot, PERMITTED),"clue_held":inventory(snapshot, CLUE)})));
            }
            Ok(None)
        }),
    ).expect("Sherlock must bank the banned sickle, retain the permitted bow and board through the real monk");
}
