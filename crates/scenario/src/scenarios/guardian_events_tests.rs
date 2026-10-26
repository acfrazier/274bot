use super::*;
use api::selected::{EntityId, Knowledge};
use client::io::ClientRevision;
use std::sync::Arc;

fn data() -> Arc<api::game_data::SelectedGameData> {
    api::game_data::for_revision(ClientRevision::R289).expect("selected 289 data")
}

#[test]
fn every_cell_is_registered_once_with_one_authentic_trigger() {
    let mut names = Vec::new();
    for (name, activity, _, trigger, oracle) in GUARDIAN_EVENT_CELLS {
        assert!(!names.contains(name), "{name} is listed twice");
        names.push(*name);
        let scenario = crate::get(name).unwrap_or_else(|| panic!("{name} is not registered"));
        assert_eq!(scenario.name, *name);
        assert_eq!(
            scenario.settings.start_script,
            Some(activity.start_script())
        );
        let observe: Vec<_> = scenario
            .steps
            .iter()
            .filter_map(|step| match step.kind {
                StepKind::ObserveRandomEvent { cell } => Some(cell),
                _ => None,
            })
            .collect();
        assert_eq!(observe.len(), 1, "{name}: one observed event");
        assert_eq!(observe[0].trigger, *trigger);
        assert_eq!(observe[0].oracle, *oracle);
        assert_eq!(observe[0].skill, activity.stat());
        // The observation is the last step: the run first proves the activity
        // works, and only the trigger is sent after that.
        let last = scenario.steps.last().unwrap();
        assert!(matches!(last.kind, StepKind::ObserveRandomEvent { .. }));
        let before = &scenario.steps[scenario.steps.len() - 2];
        assert!(matches!(
            before.wait.arm,
            Proof::FreshStatXpGain { id, .. } if id == activity.stat()
        ));
    }
    assert_eq!(names.len(), 20, "8 general and 12 skill events");
    let general = GUARDIAN_EVENT_CELLS
        .iter()
        .filter(|cell| matches!(cell.3, Trigger::General(_)))
        .map(|cell| match cell.3 {
            Trigger::General(id) => id,
            Trigger::Skill(_) | Trigger::Seeded => unreachable!(),
        })
        .collect::<Vec<_>>();
    assert_eq!(general, (1..=8).collect::<Vec<_>>());
    let skill = GUARDIAN_EVENT_CELLS
        .iter()
        .filter_map(|cell| match cell.3 {
            Trigger::Skill(id) => Some(id),
            Trigger::General(_) | Trigger::Seeded => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(skill, (1..=12).collect::<Vec<_>>());
}

#[test]
fn same_kind_head_variant_drops_the_distractor_before_triggering() {
    const RUNE_AXE_HEAD_ID: i32 = 520;
    let scenario = crate::get("guardian_skill_lost_axe_same_kind_head")
        .expect("the exact-tier live cell is registered");
    let seed = scenario
        .steps
        .iter()
        .position(|step| step.name == "seed an unrelated Rune axe head")
        .expect("seed step");
    let drop = scenario
        .steps
        .iter()
        .position(|step| step.name == "drop the Rune axe head beside the player")
        .expect("real drop step");
    let observe = scenario
        .steps
        .iter()
        .position(|step| step.name == "summon the event and observe the Guardian handle it")
        .expect("event observation step");
    assert!(seed < drop && drop < observe);
    assert!(matches!(
        &scenario.steps[seed].kind,
        StepKind::Perform { .. }
    ));
    assert_eq!(
        scenario.steps[seed].wait.arm,
        Proof::ItemId {
            id: RUNE_AXE_HEAD_ID,
            count: 1
        }
    );
    assert!(matches!(
        &scenario.steps[drop].kind,
        StepKind::Perform { .. }
    ));
    assert_eq!(
        scenario.steps[drop].wait.arm,
        Proof::ItemIdAtMost {
            id: RUNE_AXE_HEAD_ID,
            count: 0
        }
    );
    let StepKind::ObserveRandomEvent { cell } = &scenario.steps[observe].kind else {
        panic!("last step is the event observation");
    };
    assert_eq!(cell.trigger, Trigger::Skill(9));
    assert_eq!(
        cell.oracle,
        Oracle::LostTool {
            tool_ids: &[BRONZE_AXE_ID]
        }
    );
}

#[test]
fn the_fixtures_seed_only_the_accounts_own_state() {
    for (name, ..) in GUARDIAN_EVENT_CELLS {
        let scenario = crate::get(name).unwrap();
        for step in &scenario.steps {
            assert!(
                !step.name.contains("npcadd") && !step.name.contains("locadd"),
                "{name}: {}",
                step.name
            );
        }
    }
    // The seeded ids are the items the seed cheats name.
    let data = data();
    for (alias, id) in [
        ("harpoon", HARPOON_ID),
        ("bronze_pickaxe", BRONZE_PICKAXE_ID),
        ("bronze_axe", BRONZE_AXE_ID),
        ("bones", BONES_ID),
        ("lobster", Activity::Thieving.seeded_item()),
        ("kebab", KEBAB_ID),
        ("beer", BEER_ID),
        ("macro_cube", STRANGE_BOX_ID),
    ] {
        assert_eq!(
            data.item_by_alias(alias).map(|item| item.id),
            Some(id),
            "{alias}"
        );
    }
}

#[test]
fn strange_box_load_claim_seeds_inventory_and_bounds_the_handoff() {
    let scenario = guardian_event_strange_box_load_claim_scenario();
    assert_eq!(scenario.name, "guardian_event_strange_box_load_claim");
    assert_eq!(
        scenario.settings.start_file,
        Some("guardian_claim_toggle.js")
    );
    assert!(scenario.settings.start_script.is_none());
    assert!(scenario
        .steps
        .iter()
        .all(|step| { !step.name.contains("npcadd") && !step.name.contains("locadd") }));

    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("the Load Start step");
    let armed = scenario
        .steps
        .iter()
        .position(|step| {
            step.wait.arm
                == Proof::ScriptReceipt {
                    prefix: STRANGE_BOX_ARMED_RECEIPT,
                }
        })
        .expect("the observed initial Load policy");
    let seed_index = scenario
        .steps
        .iter()
        .position(|step| step.name == "seed one account-owned Strange box while Load ignores it")
        .expect("the real inventory seed step");
    assert!(
        start < armed && armed < seed_index,
        "Load must own the box before it arrives"
    );
    let seed = &scenario.steps[seed_index];
    assert_eq!(
        seed.wait.arm,
        Proof::ItemId {
            id: STRANGE_BOX_ID,
            count: 1
        }
    );

    let observations: Vec<_> = scenario
        .steps
        .iter()
        .filter_map(|step| match step.kind {
            StepKind::ObserveRandomEvent { cell } => Some((step, cell)),
            _ => None,
        })
        .collect();
    assert_eq!(observations.len(), 1);
    let (observe, cell) = observations[0];
    assert_eq!(cell.trigger, Trigger::Seeded);
    assert_eq!(cell.oracle, Oracle::StrangeBoxHandoff);
    assert_eq!(observe.wait.budget_ticks, 120);
    let resumed = scenario
        .steps
        .iter()
        .position(|step| {
            step.wait.arm
                == Proof::ScriptReceipt {
                    prefix: STRANGE_BOX_RESUMED_RECEIPT,
                }
        })
        .expect("the original handoff still requires script resumption");
    let restart = scenario
        .steps
        .iter()
        .rposition(|step| matches!(step.kind, StepKind::StartScript))
        .expect("the second Load Start");
    let owned = scenario
        .steps
        .iter()
        .rposition(|step| {
            step.wait.arm
                == Proof::ScriptReceipt {
                    prefix: STRANGE_BOX_OWNED_RECEIPT,
                }
        })
        .expect("the second box is script-owned");
    let stop = scenario
        .steps
        .iter()
        .rposition(|step| matches!(step.kind, StepKind::StopScript))
        .expect("Stop while the second box is still ignored");
    assert!(resumed < restart && restart < owned && owned < stop);
    assert_eq!(scenario.steps[stop].wait.arm, Proof::ScriptIdle);
    assert_eq!(
        scenario.steps[stop + 1].wait.arm,
        Proof::ItemIdAtMost {
            id: STRANGE_BOX_ID,
            count: 0,
        }
    );
    assert_eq!(scenario.proof, scenario.steps[stop + 1].wait.arm);
}

#[test]
fn every_evade_npc_has_a_content_chase_range() {
    let data = data();
    let rows = &data.npc_names().expect("npc names").rows;
    for (name, _, _, _, oracle) in GUARDIAN_EVENT_CELLS {
        let Oracle::Evade { npc } = oracle else {
            continue;
        };
        let ranges: Vec<i32> = rows
            .iter()
            .filter(|row| {
                row.display
                    .as_deref()
                    .is_some_and(|display| display.eq_ignore_ascii_case(npc))
            })
            .map(|row| row.maxrange)
            .collect();
        assert!(!ranges.is_empty(), "{name}: no {npc} in the content");
        assert!(ranges.iter().all(|range| *range > 0), "{name}: {ranges:?}");
    }
}

/// The fishing and mining stands are inside their published sites, with a
/// real placement of the worked resource in reach (no spawned resource).
#[test]
fn the_activity_stands_are_beside_real_resources() {
    let selected = data();
    let data = Arc::clone(&selected);
    let input = script::preparation::PreparationInput::new(
        289,
        std::mem::size_of::<Arc<api::game_data::SelectedGameData>>(),
        2 * 1024 * 1024,
        2 * 1024 * 1024,
    )
    .unwrap()
    .with_gathering_family_decode();
    let catalog = script::preparation::submit(
        script::preparation::PreparationKind::Profile,
        api::identity::Scope::Process,
        input,
        move |worker| data.prepare_gathering(worker),
    )
    .expect("admit the real gathering family")
    .join()
    .expect("worker settles")
    .expect("the gathering family prepares");
    for (method, start, radius) in [
        ("fishing.rarefish.op3", CATHERBY_HARPOON_START, 8),
        ("mining.copper", VARROCK_EAST_MINE_START, 4),
        ("mining.tin", VARROCK_EAST_MINE_START, 4),
        ("woodcutting.oak", DRAYNOR_OAK_START, 8),
        ("woodcutting.normal", LUMBRIDGE_TREES_START, 3),
    ] {
        let method_row = catalog
            .method(method)
            .unwrap_or_else(|error| panic!("{method} is not published: {error:?}"));
        let Knowledge::Known(spots) = &method_row.spots else {
            panic!("{method} placements are not complete");
        };
        let near: Vec<_> = spots
            .iter()
            .filter(|spot| spot.origin.level == start.level)
            .map(|spot| {
                (
                    (spot.origin.x - start.x)
                        .abs()
                        .max((spot.origin.z - start.z).abs()),
                    spot.origin.x,
                    spot.origin.z,
                    spot.entity,
                )
            })
            .filter(|(distance, ..)| *distance <= 12)
            .collect();
        assert!(
            near.iter().any(|(distance, ..)| *distance <= radius),
            "{method}: no placement within r{radius} of {start:?}; nearby {near:?}"
        );
        assert!(near
            .iter()
            .all(|(.., entity)| matches!(entity, EntityId::Loc(_) | EntityId::Npc(_))));
    }
}

#[test]
fn skill_cells_name_the_engine_patch_they_need() {
    assert!(crate::random_episode::SKILL_EVENT_PATCH.starts_with("tools/engine-patches/289/"));
    let patch = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(crate::random_episode::SKILL_EVENT_PATCH);
    let text = std::fs::read_to_string(&patch).expect("the patch ships in the repo");
    assert!(text.contains("[debugproc,skill_macro_event](int $event)"));
    for (_, _, _, trigger, _) in GUARDIAN_EVENT_CELLS {
        if let Trigger::Skill(id) = trigger {
            assert!(
                text.contains(&format!("mes(\"skill_macro_event {id}: ")),
                "the patch answers event {id}"
            );
        }
    }
}

/// Every registered scenario's step sends, run once against a synthetic
/// ingame client with an empty snapshot: the names whose sends emit
/// `needle`. A send that depends on live state the empty snapshot lacks is
/// not seen here.
fn scenarios_sending(needle: &str) -> Vec<&'static str> {
    use client::client::ClientConfig;
    use client::dash3d::ClientPlayer;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    let mut users = Vec::new();
    for name in crate::names() {
        let Ok(Some(scenario)) = crate::try_get(name) else {
            continue;
        };
        let mut client = Client::new(ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: crate::NO_CACHE_DIR.into(),
            members: true,
            lowmem: false,
        });
        client.ingame = true;
        client.scene_state = 2;
        client.set_cheat_admission(client::CheatAdmission::Granted);
        client.map_build_base_x = 3200;
        client.map_build_base_z = 3200;
        client.local_player = Some(ClientPlayer::at(20, 20));
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(&client);
        let mut found = false;
        for step in &scenario.steps {
            let send = match &step.kind {
                StepKind::Perform { send } | StepKind::Repeat { send } => send,
                StepKind::ObserveRandomEvent { cell } => {
                    found |= cell.trigger.command().contains(needle);
                    continue;
                }
                _ => continue,
            };
            client.out.pos = 0;
            let _ = catch_unwind(AssertUnwindSafe(|| send(&mut client, &snapshot)));
            let bytes = &client.out.data()[..client.out.pos];
            found |= bytes
                .windows(needle.len())
                .any(|window| window == needle.as_bytes());
        }
        if found {
            users.push(name);
        }
    }
    users
}

/// The cells `docs/dev/harness.md` lists for one patch, between
/// `<!-- engine-patch-cells:<patch> -->` and `<!-- /engine-patch-cells -->`.
fn documented_cells(patch: &str) -> Vec<String> {
    let doc = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/dev/harness.md"),
    )
    .expect("docs/dev/harness.md");
    let open = format!("<!-- engine-patch-cells:{patch} -->");
    let start = doc
        .find(&open)
        .unwrap_or_else(|| panic!("docs/dev/harness.md has no {open}"));
    let rest = &doc[start + open.len()..];
    let end = rest
        .find("<!-- /engine-patch-cells -->")
        .expect("closed cell list");
    rest[..end]
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

/// `docs/dev/harness.md` names every scenario cell that needs an engine test
/// patch, so a contributor on an unmodified engine knows which cells need
/// which patch.
#[test]
fn every_engine_patch_user_is_documented() {
    for (patch, needle) in [("0001", "givebank"), ("0002", "skill_macro_event")] {
        let users = scenarios_sending(needle);
        assert!(!users.is_empty(), "no scenario sends {needle}");
        let documented = documented_cells(patch);
        let missing: Vec<_> = users
            .iter()
            .filter(|name| !documented.iter().any(|doc| doc == *name))
            .collect();
        let stale: Vec<_> = documented
            .iter()
            .filter(|doc| !users.contains(&doc.as_str()))
            .collect();
        assert!(
            missing.is_empty() && stale.is_empty(),
            "patch {patch}: undocumented {missing:?}, documented but unused {stale:?}"
        );
    }
}
