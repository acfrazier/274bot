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

/// Qualify the selected 377 client without a whole-world route or game-data
/// operation: observe the mainland landing, walk four to six courtyard tiles,
/// then talk to the live NPC whose selected-cache type is Hans (0).
///
/// The existing mainland seed owns the teleport. Its landing is not counted as
/// the walk, and an already-open dialogue cannot satisfy the fresh NPC step.
pub(crate) fn client_377_scenario() -> Scenario {
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
                        Interactions::new(snapshot, client)
                            .walk(COURTYARD_SOUTH)
                            .is_sent()
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
                kind: StepKind::Perform {
                    send: Box::new(|client, snapshot| {
                        if !Proof::NoActiveContinue.check(snapshot, None) {
                            return false;
                        }
                        let Some(npc) = snapshot
                            .npcs()
                            .iter()
                            .filter(|npc| {
                                npc.r#type == Some(HANS_ID)
                                    && npc.tile.level == COURTYARD_SOUTH.level
                                    && npc.distance <= 16
                            })
                            .min_by_key(|npc| npc.distance)
                        else {
                            return false;
                        };
                        Interactions::new(snapshot, client)
                            .interact(OpTarget::Npc(npc), ActionSpec::Label("Talk-to".into()))
                            .is_sent()
                    }),
                },
                wait: Wait {
                    arm: Proof::ActiveContinue,
                    budget_ticks: 30,
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
    use client::client::ClientConfig;
    use client::config::{Cache, IfType, IfTypeMut};
    use client::dash3d::ClientPlayer;
    use client::io::{ClientStream, ServerProt};
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
        client.bump_gens(ServerProt::PLAYER_INFO);
        client.bump_gens(ServerProt::REBUILD_NORMAL);
        client
    }

    fn perform(scenario: &Scenario, index: usize, client: &mut Client) -> bool {
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(client);
        let StepKind::Perform { send } = &scenario.steps[index].kind else {
            panic!("expected a direct client action");
        };
        send(client, &snapshot)
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
        selected.bump_gens(ServerProt::PLAYER_INFO);
        assert!(!perform(&scenario, 0, &mut selected));
        selected.local_player = Some(ClientPlayer::at(20, 20));
        selected.chat_modal_id = 100;
        selected.bump_gens(ServerProt::PLAYER_INFO);
        selected.bump_gens(ServerProt::IF_OPENCHAT);
        assert!(!perform(&scenario, 0, &mut selected));
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
        client.bump_gens(ServerProt::PLAYER_INFO);
        snapshot.rebuild(&client);
        assert!(!scenario.steps[2].wait.arm.check(&snapshot, None));

        client.local_player = Some(ClientPlayer::at(20, 16));
        client.bump_gens(ServerProt::PLAYER_INFO);
        snapshot.rebuild(&client);
        assert!(scenario.steps[2].wait.arm.check(&snapshot, None));
    }
}
