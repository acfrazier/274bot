//! Route A checks Fairy Ring content, not historical accuracy.
//! Commute teleports affect this account only. Every ingredient is world-sourced.
use crate::*;
use api::interact;
use client::io::ClientRevision;

const COOK: usize = 278;
const EGG: i32 = 1944;
const BUCKET: i32 = 1925;
const MILK: i32 = 1927;
const POT: i32 = 1931;
const FLOUR: i32 = 1933;
const GRAIN: i32 = 1947;

fn step(
    name: &'static str,
    proof: Proof,
    send: impl Fn(&mut Client, &GameSnapshot) -> bool + Send + Sync + 'static,
) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(send),
        },
        wait: Wait {
            arm: proof,
            budget_ticks: 60,
        },
    }
}

fn commute(name: &'static str, x: i32, z: i32) -> Step {
    step(
        name,
        Proof::ArrivedNear {
            x,
            z,
            level: 0,
            radius: 0,
        },
        move |client, _| interact::cheat(client, &interact::tele_args(0, x, z)).is_sent(),
    )
}

fn take(name: &'static str, id: i32) -> Step {
    step(
        name,
        Proof::ItemId { id, count: 1 },
        move |client, snapshot| {
            let Some(item) = snapshot
                .ground_items()
                .iter()
                .filter(|row| row.def.id == id)
                .min_by_key(|row| row.distance)
            else {
                return false;
            };
            matches!(
                Interactions::new(snapshot, client)
                    .interact(OpTarget::GroundItem(item), ActionSpec::Label("Take".into())),
                SendResult::Sent { .. }
            )
        },
    )
}

fn loc(
    name: &'static str,
    label: &'static str,
    action: &'static str,
    item: Option<i32>,
    proof: Proof,
) -> Step {
    step(name, proof, move |client, snapshot| {
        let Some(target) = snapshot
            .locs()
            .iter()
            .filter(|row| row.name.as_deref() == Some(label))
            // 377 wheat_tall (5583) advertises Pick but has no script trigger.
            .filter(|row| label != "Wheat" || row.id != 5583)
            .min_by_key(|row| row.distance)
        else {
            return false;
        };
        let mut interaction = Interactions::new(snapshot, client);
        let result = if let Some(id) = item {
            let Some(held) = snapshot.inventory().iter().find(|row| row.def.id == id) else {
                return false;
            };
            interaction.use_item_on(held, OpTarget::Loc(target))
        } else {
            interaction.interact(OpTarget::Loc(target), ActionSpec::Label(action.into()))
        };
        matches!(result, SendResult::Sent { .. })
    })
}

fn talk() -> Step {
    step(
        "talk to the live Cook",
        Proof::ActiveContinue,
        |client, snapshot| {
            let Some(cook) = snapshot.npcs().iter().find(|row| row.r#type == Some(COOK)) else {
                return false;
            };
            matches!(
                Interactions::new(snapshot, client)
                    .interact(OpTarget::Npc(cook), ActionSpec::Label("Talk-to".into())),
                SendResult::Sent { .. }
            )
        },
    )
}

fn dialogue(name: &'static str, proof: Proof, readback: Option<&'static str>) -> Step {
    Step {
        name,
        kind: StepKind::Repeat {
            send: Box::new(move |client, snapshot| {
                // Completion can arrive before the old chat widget is retired.
                // Once the requested product proof holds, do not send a stale continue.
                if proof.check(snapshot, None) {
                    return true;
                }
                if let Some(command) = readback {
                    if !interact::cheat(client, command).is_sent() {
                        return false;
                    }
                }
                let mut interaction = Interactions::new(snapshot, client);
                if !snapshot.chat_options().is_empty() {
                    // quest_cook.rs2:2,29: both positive choices are option 1.
                    matches!(interaction.answer_choice(1), SendResult::Sent { .. })
                } else if snapshot.chat_continue_component_id() != -1 {
                    matches!(interaction.continue_dialog(None), SendResult::Sent { .. })
                } else {
                    true
                }
            }),
        },
        wait: Wait {
            arm: proof,
            budget_ticks: 120,
        },
    }
}

fn server_readback(name: &'static str, command: &'static str, reply: &'static str) -> Step {
    Step {
        name,
        kind: StepKind::Repeat {
            send: Box::new(move |client, _| interact::cheat(client, command).is_sent()),
        },
        wait: Wait {
            arm: Proof::FreshChat { needle: reply },
            budget_ticks: 60,
        },
    }
}

fn quest_colour(name: &'static str, colour: i32) -> Step {
    step(name, Proof::NoActiveContinue, move |_, snapshot| {
        snapshot
            .quest_statuses()
            .iter()
            .any(|row| row.name == "Cook's Assistant" && row.colour == colour)
    })
}

pub(crate) fn cook_377_scenario() -> Scenario {
    Scenario {
        name: "cook_377",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            step(
                "require fresh 377 Cooking XP and inventory",
                Proof::NoActiveContinue,
                |client, snapshot| {
                    client.revision() == ClientRevision::R377
                        && snapshot
                            .stats()
                            .iter()
                            .any(|row| row.index == 7 && row.xp == 0)
                        && [EGG, MILK, FLOUR, GRAIN, BUCKET, POT].iter().all(|id| {
                            !snapshot
                                .inventory()
                                .iter()
                                .any(|row| row.def.id == *id && row.count > 0)
                        })
                },
            ),
            server_readback(
                "SERVER READBACK fresh cookquest=0",
                "getvar cookquest",
                "get cookquest: 0",
            ),
            server_readback("SERVER READBACK fresh qp=0", "getvar qp", "get qp: 0"),
            quest_colour("observe unstarted red Cook quest-list row", 0xF80000),
            commute("SEED position: Cook kitchen commute", 3208, 3215),
            talk(),
            dialogue(
                "accept Cook request; SERVER READBACK cookquest=1",
                Proof::FreshChat {
                    needle: "get cookquest: 1",
                },
                Some("getvar cookquest"),
            ),
            dialogue(
                "finish Cook ingredient instructions",
                Proof::NoActiveContinue,
                None,
            ),
            server_readback(
                "SERVER READBACK product cookquest=1",
                "getvar cookquest",
                "get cookquest: 1",
            ),
            quest_colour("observe started yellow Cook quest-list row", 0xF8F800),
            take("take the kitchen's world-spawned empty pot", POT),
            commute("SEED position: chicken pen commute", 3227, 3300),
            take("take a world-spawned egg", EGG),
            commute("SEED position: farm bucket commute", 3225, 3294),
            take("take the farm's world-spawned bucket", BUCKET),
            commute("SEED position: dairy field commute", 3253, 3275),
            // cow_milking.rs2:35-38 dispatches the real Milk script.
            loc(
                "use the collected bucket on a dairy cow",
                "Dairy Cow",
                "",
                Some(BUCKET),
                Proof::ItemId { id: MILK, count: 1 },
            ),
            commute("SEED position: wheat field commute", 3158, 3300),
            loc(
                "pick scripted wheat naturally",
                "Wheat",
                "Pick",
                None,
                Proof::ItemId {
                    id: GRAIN,
                    count: 1,
                },
            ),
            commute("SEED position: Lumbridge mill entrance commute", 3166, 3304),
            loc(
                "climb the mill's ground-floor ladder",
                "Ladder",
                "Climb-up",
                None,
                Proof::ArrivedNear {
                    x: 3166,
                    z: 3304,
                    level: 1,
                    radius: 8,
                },
            ),
            loc(
                "climb the mill's middle-floor ladder",
                "Ladder",
                "Climb-up",
                None,
                Proof::ArrivedNear {
                    x: 3166,
                    z: 3304,
                    level: 2,
                    radius: 8,
                },
            ),
            // windmills.rs2:85-91: grain removal and hopper_full are product writes.
            loc(
                "put collected grain in the hopper",
                "Hopper",
                "",
                Some(GRAIN),
                Proof::FreshChat {
                    needle: "You put the grain in the hopper.",
                },
            ),
            server_readback(
                "SERVER READBACK product hopper_full=1",
                "getvar hopper_full",
                "get hopper_full: 1",
            ),
            loc(
                "operate the filled hopper controls",
                "Hopper controls",
                "Operate",
                None,
                Proof::FreshChat {
                    needle: "You operate the hopper.",
                },
            ),
            server_readback(
                "SERVER READBACK product flour=1",
                "getvar flour",
                "get flour: 1",
            ),
            loc(
                "descend the top-floor ladder",
                "Ladder",
                "Climb-down",
                None,
                Proof::ArrivedNear {
                    x: 3166,
                    z: 3304,
                    level: 1,
                    radius: 8,
                },
            ),
            loc(
                "descend the middle-floor ladder",
                "Ladder",
                "Climb-down",
                None,
                Proof::ArrivedNear {
                    x: 3166,
                    z: 3304,
                    level: 0,
                    radius: 8,
                },
            ),
            loc(
                "empty the flour bin into the collected pot",
                "Flour bin",
                "Empty",
                None,
                Proof::ItemId {
                    id: FLOUR,
                    count: 1,
                },
            ),
            step(
                "require all three naturally gathered ingredients",
                Proof::ItemId { id: EGG, count: 1 },
                |_, snapshot| {
                    [EGG, MILK, FLOUR]
                        .iter()
                        .all(|id| Proof::ItemId { id: *id, count: 1 }.check(snapshot, None))
                        && Proof::ItemIdAtMost {
                            id: GRAIN,
                            count: 0,
                        }
                        .check(snapshot, None)
                },
            ),
            commute("SEED position: Cook hand-in commute", 3208, 3215),
            talk(),
            // quest_cook.rs2:77-91: inventory deletion then queued completion.
            dialogue(
                "hand in through Cook and observe green completion row",
                Proof::QuestDone {
                    name: "Cook's Assistant",
                },
                None,
            ),
            server_readback(
                "SERVER READBACK product cookquest=2",
                "getvar cookquest",
                "get cookquest: 2",
            ),
            server_readback("SERVER READBACK product qp=1", "getvar qp", "get qp: 1"),
            quest_colour("observe completed green Cook quest-list row", 0x00F800),
            step(
                "observe client-visible quest points and completion interface",
                Proof::NoActiveContinue,
                |_, snapshot| {
                    Proof::VarpExact { id: 101, value: 1 }.check(snapshot, None)
                        && snapshot.modals().main >= 0
                        && snapshot
                            .main_modal_texts()
                            .iter()
                            .any(|text| text == "1 Quest Point")
                        && snapshot
                            .main_modal_texts()
                            .iter()
                            .any(|text| text == "You have completed the Cook's Assistant Quest!")
                        && snapshot
                            .main_modal_texts()
                            .iter()
                            .any(|text| text == "300 Cooking XP")
                },
            ),
            step(
                "observe Cooking level four after the 300 XP reward",
                Proof::Stat { id: 7, min: 4 },
                |_, _| true,
            ),
            step(
                "require exact client XP gain and consumed ingredients",
                Proof::Stat { id: 7, min: 4 },
                |_, snapshot| {
                    snapshot
                        .stats()
                        .iter()
                        .any(|row| row.index == 7 && row.xp == 300)
                        && [EGG, MILK, FLOUR].iter().all(|id| {
                            Proof::ItemIdAtMost { id: *id, count: 0 }.check(snapshot, None)
                        })
                },
            ),
        ],
        proof: Proof::QuestDone {
            name: "Cook's Assistant",
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: std::time::Duration::from_secs(900),
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_a_seeds_only_commute_positions_and_requires_product_rewards() {
        let cell = cook_377_scenario();
        assert!(cell.seed.mainland);
        assert_eq!(
            cell.steps
                .iter()
                .filter(|step| step.name.starts_with("SEED position:"))
                .count(),
            7
        );
        assert!(cell.steps.iter().any(|step| matches!(
            step.wait.arm,
            Proof::FreshChat {
                needle: "get cookquest: 1"
            }
        )));
        assert!(cell.steps.iter().any(|step| matches!(
            step.wait.arm,
            Proof::FreshChat {
                needle: "get qp: 1"
            }
        )));
        assert!(cell
            .steps
            .iter()
            .any(|step| matches!(step.wait.arm, Proof::Stat { id: 7, min: 4 })));
        assert_eq!(
            cell.steps
                .iter()
                .filter(|step| step.name.contains("climb the mill"))
                .count(),
            2
        );
    }

    fn client(revision: ClientRevision) -> Client {
        use client::client::ClientConfig;
        use client::config::{Cache, VarpType};
        use std::sync::Arc;
        let mut client = Client::from_shared_with_revision(
            ClientConfig {
                host: "127.0.0.1".into(),
                port: 43595,
                cache_dir: crate::NO_CACHE_DIR.into(),
                members: true,
                lowmem: false,
            },
            Arc::new(Cache {
                varps: (0..102).map(|_| VarpType::default()).collect(),
                ..Default::default()
            }),
            Arc::new(Vec::new()),
            Arc::new(Vec::new()),
            revision,
        );
        client.ingame = true;
        client.scene_state = 2;
        client.var = vec![0; 102];
        client.gens.stat = 1;
        client
    }

    fn perform(step: &Step, client: &mut Client) -> bool {
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(client);
        match &step.kind {
            StepKind::Perform { send } => send(client, &snapshot),
            _ => panic!("expected one-shot guard"),
        }
    }

    #[test]
    fn fresh_guard_refuses_other_revisions_and_seeded_xp() {
        let cell = cook_377_scenario();
        let first = &cell.steps[0];
        for revision in [ClientRevision::R274, ClientRevision::R289] {
            assert!(!perform(first, &mut client(revision)));
        }
        let mut selected = client(ClientRevision::R377);
        assert!(perform(first, &mut selected));
        selected.stat_xp[7] = 300;
        assert!(!perform(first, &mut selected));
    }

    #[test]
    fn terminal_guard_requires_exact_client_xp() {
        let cell = cook_377_scenario();
        let last = cell.steps.last().unwrap();
        let mut selected = client(ClientRevision::R377);
        assert!(!perform(last, &mut selected));
        selected.stat_xp[7] = 300;
        assert!(perform(last, &mut selected));
        selected.stat_xp[7] = 301;
        assert!(!perform(last, &mut selected));
    }

    #[test]
    fn dialogue_waits_for_a_real_continue_and_stops_on_product_completion() {
        let step = dialogue(
            "completion",
            Proof::QuestDone {
                name: "Cook's Assistant",
            },
            None,
        );
        let StepKind::Repeat { send } = step.kind else {
            panic!("dialogue repeats")
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_chat_modal(356, vec!["Click here to continue".into()]);
        let mut client = client(ClientRevision::R377);
        assert!(
            send(&mut client, &snapshot),
            "a modal without a continue component waits"
        );
        assert!(
            !step.wait.arm.check(&snapshot, None),
            "waiting does not prove completion"
        );
        snapshot.seed_quest_statuses(
            vec![api::snapshot::QuestStatusView {
                component_id: 7333,
                name: "Cook's Assistant".into(),
                colour: 0x00F800,
            }],
            true,
        );
        assert!(
            send(&mut client, &snapshot),
            "the product completion stops further continues"
        );
    }
}
