use crate::*;
use client::io::ClientRevision;

const COURTYARD_SOUTH: WorldTile = WorldTile {
    x: 3220,
    z: 3216,
    level: 0,
};
const AT_COURTYARD_SOUTH: Proof = Proof::ArrivedNear {
    x: COURTYARD_SOUTH.x,
    z: COURTYARD_SOUTH.z,
    level: COURTYARD_SOUTH.level,
    radius: 0,
};
const HANS_ID: usize = 0;
// Match shop_buyout_open_npc_bank: let the server's NPC approach run before
// retrying Talk-to against the NPC's current observation.
const HANS_TALK_APPROACH_TICKS: u32 = 24;

/// Qualify the selected 377 client without a whole-world route or game-data
/// operation: observe the mainland landing, walk four to six courtyard tiles,
/// then talk to the live NPC whose selected-cache type is Hans (0).
///
/// The existing mainland seed owns the teleport. Its landing is not counted as
/// the walk, and an already-open dialogue cannot satisfy the fresh NPC step.
pub(crate) fn client_377_scenario() -> Scenario {
    let talked = std::sync::Mutex::new(None::<u32>);
    Scenario {
        name: "client_377",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "observe the 377 mainland landing with no dialogue",
                kind: StepKind::Perform {
                    send: Box::new(|client, snapshot| {
                        client.revision() == ClientRevision::R377
                            && matches!(snapshot.tile(), Some((3220, 3220 | 3222, 0)))
                            && Proof::NoActiveContinue.check(snapshot, None)
                    }),
                },
                wait: Wait {
                    arm: Proof::NoActiveContinue,
                    budget_ticks: 5,
                },
            },
            Step {
                name: "capture the selected 377 scene",
                kind: StepKind::Shot {
                    label: "scene2_377",
                },
                wait: Wait {
                    arm: Proof::IngameScene2,
                    budget_ticks: 5,
                },
            },
            Step {
                name: "walk directly south inside the loaded courtyard",
                kind: StepKind::Perform {
                    send: Box::new(|client, snapshot| {
                        matches!(
                            Interactions::new(snapshot, client).walk(COURTYARD_SOUTH),
                            SendResult::Sent { .. }
                        )
                    }),
                },
                wait: Wait {
                    arm: AT_COURTYARD_SOUTH,
                    budget_ticks: 30,
                },
            },
            Step {
                name: "capture the observed direct-walk destination",
                kind: StepKind::Shot {
                    label: "direct_walk_377",
                },
                wait: Wait {
                    arm: AT_COURTYARD_SOUTH,
                    budget_ticks: 5,
                },
            },
            Step {
                name: "talk to the selected-cache Hans NPC by numeric type",
                kind: StepKind::Repeat {
                    send: Box::new(move |client, snapshot| {
                        let Ok(mut talked) = talked.lock() else {
                            return false;
                        };
                        if !Proof::NoActiveContinue.check(snapshot, None) {
                            return talked.is_some();
                        }
                        if talked.is_some_and(|tick| {
                            snapshot.tick().saturating_sub(tick) <= HANS_TALK_APPROACH_TICKS
                        }) {
                            return true;
                        }
                        let Some((x, z, level)) = snapshot.tile() else {
                            return true;
                        };
                        let here = WorldTile { x, z, level };
                        let Some(npc) = snapshot
                            .npcs()
                            .iter()
                            .filter_map(|npc| {
                                let distance = api::query::chebyshev_to(here, npc.network);
                                (npc.r#type == Some(HANS_ID)
                                    && npc.network.level == COURTYARD_SOUTH.level
                                    && distance <= 16)
                                    .then_some((npc, distance))
                            })
                            .min_by_key(|(_, distance)| *distance)
                            .map(|(npc, _)| npc)
                        else {
                            return true;
                        };
                        match Interactions::new(snapshot, client)
                            .interact(OpTarget::Npc(npc), ActionSpec::Label("Talk-to".into()))
                        {
                            SendResult::Sent { .. } => {
                                *talked = Some(snapshot.tick());
                                true
                            }
                            SendResult::Refused {
                                reason:
                                    SendReason::SceneUnavailable
                                    | SendReason::OffScene
                                    | SendReason::StaleTarget,
                                ..
                            } => true,
                            SendResult::Refused { .. } => false,
                        }
                    }),
                },
                wait: Wait {
                    arm: Proof::ActiveContinue,
                    budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
                },
            },
            Step {
                name: "capture the fresh Hans dialogue",
                kind: StepKind::Shot {
                    label: "hans_dialogue_377",
                },
                wait: Wait {
                    arm: Proof::ActiveContinue,
                    budget_ticks: 5,
                },
            },
        ],
        proof: Proof::ActiveContinue,
        companions: vec![],
        settings: ScenarioSettings {
            capture: true,
            full_rate: true,
            require_mainland_base: true,
            terminal_shot: Some("client_377"),
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::client::{ClientConfig, ClientNpc};
    use client::config::runtime_377::{CacheFields377, InterfaceFields377, NpcFields377};
    use client::config::{Cache, CacheRevision377, IfType, IfTypeMut, NpcType};
    use client::dash3d::ClientPlayer;
    use client::io::ClientStream;
    use std::sync::Arc;

    fn scene_client(revision: ClientRevision) -> Client {
        let mut client = Client::from_shared_with_revision(
            ClientConfig {
                host: "127.0.0.1".into(),
                port: 43595,
                cache_dir: crate::NO_CACHE_DIR.into(),
                members: true,
                lowmem: false,
            },
            Arc::new(Cache::default()),
            Arc::new(Vec::<Option<Box<IfType>>>::new()),
            Arc::new(Vec::<Option<Arc<IfTypeMut>>>::new()),
            revision,
        );
        client.ingame = true;
        client.scene_state = 2;
        client.map_build_base_x = 3200;
        client.map_build_base_z = 3200;
        client.local_player = Some(ClientPlayer::at(20, 20));
        for column in &mut client.collision[0].flags {
            column.fill(0);
        }
        // Fixture publication is independent of the R274-only wire helper.
        client.gens.player = 1;
        client.gens.scene = 1;
        client.gens.iface = 1;
        client
    }

    fn perform(scenario: &Scenario, index: usize, client: &mut Client) -> bool {
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(client);
        let send = match &scenario.steps[index].kind {
            StepKind::Perform { send } | StepKind::Repeat { send } => send,
            _ => panic!("expected a direct client action"),
        };
        send(client, &snapshot)
    }

    fn talk_runner() -> ScenarioRunner {
        let mut scenario = client_377_scenario();
        let talk = scenario.steps.remove(4);
        scenario.steps = vec![talk];
        scenario.seed.mainland = false;
        scenario.settings = ScenarioSettings::default();
        let mut runner = ScenarioRunner::with_world(scenario, None);
        runner.set_scene_settle(std::time::Duration::ZERO);
        runner
    }

    fn prepare_hans(client: &mut Client) {
        let cache = Arc::get_mut(&mut client.cache).expect("sole cache owner");
        if cache.npcs.is_empty() {
            cache.npcs.push(NpcType::default());
        }
        cache.npcs[0].name = "Hans".into();
        cache.npcs[0].op = vec![Some("Talk-to".into()), None, None, None, None];
        // A 377 menu exists only when its selected current form is interactive.
        cache.revision_377 = Some(Box::new(CacheRevision377 {
            config: CacheFields377 {
                npcs: vec![NpcFields377 {
                    field_1451: -1,
                    field_1430: -1,
                    field_1440: -1,
                    field_1434: true,
                    morph: None,
                }],
                ..Default::default()
            },
            interfaces: InterfaceFields377 {
                client_code_layers: Default::default(),
                type1_fields: Vec::new(),
                null_string_masks: Vec::new(),
                initial_overlays: Vec::new(),
            },
        }));
    }

    fn publish_hans(client: &mut Client, position: Option<(i32, i32)>) {
        if let Some((local_x, local_z)) = position {
            let mut npc = ClientNpc {
                r#type: Some(HANS_ID),
                ..Default::default()
            };
            npc.entity.x = local_x * 128;
            npc.entity.z = local_z * 128;
            npc.entity.route_x[0] = local_x;
            npc.entity.route_z[0] = local_z;
            client.npc[1] = Some(Box::new(npc));
            client.npc_ids[0] = 1;
            client.npc_count = 1;
        } else {
            client.npc_count = 0;
            client.npc_ids[0] = 0;
        }
        client.gens.npc = client.gens.npc.wrapping_add(1);
        client.gens.npc_info = client.gens.npc_info.wrapping_add(1);
    }

    fn next_game_tick(runner: &mut ScenarioRunner, client: &mut Client) {
        client.gens.player = client.gens.player.wrapping_add(1);
        client.gens.player_info = client.gens.player_info.wrapping_add(1);
        runner.tick(client);
    }

    #[test]
    fn qualification_refuses_wrong_revision_landing_and_stale_dialogue() {
        let scenario = get("client_377").unwrap();
        let mut selected = scene_client(ClientRevision::R377);
        assert!(perform(&scenario, 0, &mut selected));
        assert_eq!(selected.out.pos, 0);

        for revision in [ClientRevision::R274, ClientRevision::R289] {
            let mut legacy = scene_client(revision);
            assert!(!perform(&scenario, 0, &mut legacy));
            assert_eq!(legacy.out.pos, 0);
        }

        selected.local_player = Some(ClientPlayer::at(20, 16));
        selected.gens.player = selected.gens.player.wrapping_add(1);
        assert!(!perform(&scenario, 0, &mut selected));
        selected.local_player = Some(ClientPlayer::at(20, 20));
        selected.chat_modal_id = 100;
        selected.gens.player = selected.gens.player.wrapping_add(1);
        selected.gens.iface = selected.gens.iface.wrapping_add(1);
        assert!(!perform(&scenario, 0, &mut selected));
        assert_eq!(selected.out.pos, 0);
        assert!(
            !perform(&scenario, 4, &mut selected),
            "an existing dialogue must block a new Hans Talk-to"
        );
        assert_eq!(selected.out.pos, 0);
    }

    #[test]
    fn direct_walk_sends_without_a_nav_world_and_waits_for_exact_arrival() {
        let scenario = get("client_377").unwrap();
        let mut client = scene_client(ClientRevision::R377);
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        client.stream = Some(ClientStream::connect("127.0.0.1", port).unwrap());
        let _peer = listener.accept().unwrap();

        assert!(perform(&scenario, 2, &mut client));
        assert!(
            client.out.pos > 0,
            "the real 377 walk must reach its packet sink"
        );
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(&client);
        assert!(!scenario.steps[2].wait.arm.check(&snapshot, None));

        client.local_player = Some(ClientPlayer::at(20, 17));
        client.gens.player = client.gens.player.wrapping_add(1);
        snapshot.rebuild(&client);
        assert!(!scenario.steps[2].wait.arm.check(&snapshot, None));

        client.local_player = Some(ClientPlayer::at(20, 16));
        client.gens.player = client.gens.player.wrapping_add(1);
        snapshot.rebuild(&client);
        assert!(scenario.steps[2].wait.arm.check(&snapshot, None));
    }

    #[test]
    fn hans_repeat_waits_for_visibility_and_retries_after_the_approach_window() {
        let mut runner = talk_runner();
        let mut client = scene_client(ClientRevision::R377);
        client.local_player = Some(ClientPlayer::at(20, 16));
        prepare_hans(&mut client);
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        client.stream = Some(ClientStream::connect("127.0.0.1", port).unwrap());
        let _peer = listener.accept().unwrap();

        runner.tick(&mut client);
        assert_eq!(
            runner.status(),
            RunnerStatus::Running { step: 0, total: 1 },
            "an absent Hans keeps the retry armed"
        );
        assert_eq!(client.out.pos, 0);

        for position in [(40, 16), (38, 18)] {
            publish_hans(&mut client, Some(position));
            next_game_tick(&mut runner, &mut client);
            assert_eq!(
                runner.status(),
                RunnerStatus::Running { step: 0, total: 1 },
                "a distant or wandering Hans keeps the retry armed"
            );
            assert_eq!(
                client.out.pos, 0,
                "Hans outside 16 tiles must not be targeted"
            );
        }

        publish_hans(&mut client, Some((36, 16)));
        next_game_tick(&mut runner, &mut client);
        assert_eq!(
            runner.status(),
            RunnerStatus::Running { step: 0, total: 1 },
            "Talk-to must not satisfy the fresh-dialogue arm"
        );
        let after_talk = client.out.pos;
        assert!(after_talk > 0, "visible Hans must receive Talk-to");

        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(&client);
        assert!(
            !Proof::ActiveContinue.check(&snapshot, None),
            "the interaction alone must not fabricate a conversation"
        );

        for _ in 0..HANS_TALK_APPROACH_TICKS {
            next_game_tick(&mut runner, &mut client);
            assert_eq!(
                client.out.pos, after_talk,
                "allow the pending server approach instead of re-talking every tick"
            );
        }
        publish_hans(&mut client, Some((21, 17)));
        next_game_tick(&mut runner, &mut client);
        assert!(
            client.out.pos > after_talk,
            "a missing dialogue must retry the wandering NPC after the approach window"
        );
        assert_eq!(runner.status(), RunnerStatus::Running { step: 0, total: 1 });

        // A received fresh continue is terminal; do not send another Talk-to
        // on the poll that observes it, even after the approach window expires.
        let after_retry = client.out.pos;
        client.chat_modal_id = 100;
        client.gens.iface = client.gens.iface.wrapping_add(1);
        next_game_tick(&mut runner, &mut client);
        assert_eq!(client.out.pos, after_retry);
        assert_eq!(runner.status(), RunnerStatus::Passed);
    }

    #[test]
    fn hans_repeat_ignores_a_rendered_only_nearby_npc() {
        let mut runner = talk_runner();
        let mut client = scene_client(ClientRevision::R377);
        client.local_player = Some(ClientPlayer::at(20, 16));
        prepare_hans(&mut client);
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        client.stream = Some(ClientStream::connect("127.0.0.1", port).unwrap());
        let _peer = listener.accept().unwrap();

        publish_hans(&mut client, Some((36, 16)));
        client.npc[1].as_mut().unwrap().entity.route_x[0] = 40;
        next_game_tick(&mut runner, &mut client);

        assert_eq!(
            client.out.pos, 0,
            "a rendered-near Hans outside network range must not receive Talk-to"
        );
        assert_eq!(runner.status(), RunnerStatus::Running { step: 0, total: 1 });
    }
}
