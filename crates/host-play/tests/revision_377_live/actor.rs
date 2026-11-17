//! Two native clients: normal public-chat keys and a remote actor lifetime receipt.
use super::menu::{wait_pair, Peer};
use super::*;
use api::snapshot::{actor_observation, ActorSelector};
use client::render::backend::FrameOutput;
use scenario::{
    Proof, RunnerStatus, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait,
};
use std::io::Write;

const TEXT: &str = "Actor parity hello";
const INPUT: &str = "red:wave:Actor parity hello";

fn capture(peer: &mut Peer, directory: &Path, label: &str) -> Result<Value, String> {
    peer.client.set_draw(true);
    let frame = peer.renderer.mainredraw(&mut peer.client);
    peer.client.set_draw(false);
    let FrameOutput::PixMap(map) = frame else {
        return Err("CPU frame required".into());
    };
    let pixels: Vec<u8> = map
        .pixels
        .iter()
        .flat_map(|&pixel| [(pixel >> 16) as u8, (pixel >> 8) as u8, pixel as u8])
        .collect();
    let path = directory.join(format!("{}-{label}.png", peer.name));
    let mut encoder = png::Encoder::new(
        fs::File::create(&path).map_err(|e| e.to_string())?,
        map.width as u32,
        map.height as u32,
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .map_err(|e| e.to_string())?
        .write_image_data(&pixels)
        .map_err(|e| e.to_string())?;
    Ok(json!({"path":path,"width":map.width,"height":map.height,"cycle":peer.client.loop_cycle}))
}

fn event(log: &mut fs::File, phase: &str, value: Value) -> Result<(), String> {
    writeln!(log, "{}", json!({"phase":phase,"detail":value}))
        .and_then(|()| log.flush())
        .map_err(|e| e.to_string())
}

#[test]
#[ignore = "two fresh 377 accounts, LIVE=1, canonical resources, PARITY_2A_EVIDENCE_DIR and PARITY_2A_SERVED_IDENTITY"]
fn public_chat_style_and_lifetime_377() -> Result<(), String> {
    let bound = load_bound_377()?;
    resource_record(&bound);
    let directory = required_path("PARITY_2A_EVIDENCE_DIR")?;
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let served: Value = serde_json::from_slice(
        &fs::read(required_path("PARITY_2A_SERVED_IDENTITY")?).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let identity = json!({"host":build_stamp::HOST,"client":build_stamp::CLIENT,"served":served,
        "game_data":api::build_info::game_data_pack(377),
        "runtime_cache":bound.profile.prepared_cache().map(|cache|cache.identity.content_id_hex())});
    let names = host_play::mint_live_names(2);
    let mut log = fs::File::create(directory.join(format!("{}-actor.jsonl", names[0])))
        .map_err(|e| e.to_string())?;
    event(&mut log, "identity", identity.clone())?;
    let mut peers = [
        Peer::new(&bound, names[0].clone(), 210)?,
        Peer::new(&bound, names[1].clone(), 211)?,
    ];
    let mut frames = Vec::new();
    let mut observations = Vec::new();
    let mut runner_receipt = Value::Null;
    let mut result = (|| {
        for peer in &mut peers {
            peer.login()?;
        }
        wait_pair(&mut peers, "fresh actor accounts", |peers| {
            peers.iter().all(|peer| {
                peer.snapshot.ingame()
                    && peer.snapshot.attached()
                    && peer.snapshot.scene_state() == 2
            })
        })?;
        event(
            &mut log,
            "unseeded",
            json!([peers[0].state(), peers[1].state()]),
        )?;
        for peer in &mut peers {
            interact::mainland_hop(&mut peer.client);
        }
        event(
            &mut log,
            "fixture",
            json!({"helper":"api::interact::mainland_hop",
            "scope":"only own two accounts' tutorial progress and position"}),
        )?;
        wait_pair(&mut peers, "actor accounts mainland", |peers| {
            peers.iter().all(|peer| {
                peer.snapshot.tile() == Some((3220, 3220, 0)) && peer.snapshot.scene_state() == 2
            })
        })?;
        for peer in &mut peers {
            let ifaces = Arc::clone(&peer.client.ifaces);
            if !interact::logout(&mut peer.client, ifaces.as_slice()) {
                return Err("fixture logout unavailable".into());
            }
        }
        wait_pair(&mut peers, "fixture logout", |peers| {
            peers.iter().all(|peer| !peer.client.ingame)
        })?;
        for peer in &mut peers {
            peer.login()?;
        }
        wait_pair(&mut peers, "nearby named actor", |peers| {
            peers.iter().all(|peer| {
                peer.snapshot.ingame()
                    && peer.snapshot.attached()
                    && peer.snapshot.scene_state() == 2
            }) && peers[0].player_slot(&names[1]).is_some()
                && peers[1].player_slot(&names[0]).is_some()
        })?;
        for peer in &peers {
            if peer.client.main_modal_id != -1
                || peer.client.chat_modal_id != -1
                || peer.client.dialog_input_open
                || peer.client.social_input_open
                || peer.client.chat_public_mode != 0
            {
                return Err("public chat needs idle interfaces and public-chat-on".into());
            }
        }
        let index = peers[1]
            .player_slot(&names[0])
            .ok_or("remote named actor missing")?;
        let actor = ActorSelector::Player(index);
        let style = Proof::ActorChatStyle {
            actor,
            colour: 1,
            effect: 1,
        };
        let text = Proof::OverheadText { actor, text: TEXT };
        let lifetime = Proof::OverheadTextLifetime { actor, text: TEXT };
        let mut runner = ScenarioRunner::with_world(
            Scenario {
                name: "public-chat-style-lifetime-377",
                seed: Seed {
                    profiles: vec![("test", "test")],
                    mainland: false,
                },
                steps: [("style", style), ("text", text), ("lifetime", lifetime)]
                    .into_iter()
                    .map(|(name, proof)| Step {
                        name,
                        kind: StepKind::ObserveActor { actor },
                        wait: Wait {
                            arm: proof,
                            budget_ticks: 30,
                        },
                    })
                    .collect(),
                proof: lifetime,
                companions: vec![],
                settings: ScenarioSettings::default(),
            },
            None,
        );
        runner.set_scene_settle(Duration::ZERO);
        for _ in 0..4 {
            runner.tick(&mut peers[1].client);
        }
        if !matches!(runner.status(), RunnerStatus::Running { .. }) {
            return Err("actor runner did not arm".into());
        }
        if actor_observation(&peers[1].client, Some(actor))
            .and_then(|view| view.overhead_text)
            .is_some()
        {
            return Err("fresh actor already has overhead text".into());
        }
        frames.push(capture(&mut peers[1], &directory, "before-chat")?);
        for character in INPUT.bytes() {
            peers[0]
                .client
                .shell
                .apply_key(true, 0, i32::from(character));
        }
        wait_pair(&mut peers, "normal chat typing", |peers| {
            peers[0].client.chat_input == INPUT
        })?;
        frames.push(capture(&mut peers[0], &directory, "typed-chat")?);
        let before_generation = peers[1].client.gens.player_info;
        peers[0].client.shell.apply_key(true, 0, 13);
        let mut seen = false;
        let mut observer_sent = false;
        wait_pair(
            &mut peers,
            "remote style appearance and natural expiry",
            |peers| {
                let before = peers[1].client.out.pos;
                runner.tick(&mut peers[1].client);
                observer_sent |= before != peers[1].client.out.pos;
                if let Some(view) = actor_observation(&peers[1].client, Some(actor)) {
                    if !seen
                        && view.overhead_text == Some(TEXT)
                        && view.remaining_cycles > 0
                        && peers[1].client.gens.player_info > before_generation
                    {
                        observations.push(json!({"view":view,"player_info_generation":peers[1].client.gens.player_info}));
                        seen = true;
                    }
                }
                if seen && frames.len() == 2 {
                    match capture(&mut peers[1], &directory, "remote-bubble") {
                        Ok(frame) => frames.push(frame),
                        Err(error) => runner.fail_start(&error),
                    }
                }
                matches!(
                    runner.status(),
                    RunnerStatus::Passed | RunnerStatus::Failed(_)
                )
            },
        )?;
        runner_receipt = serde_json::to_value(runner.evidence()).map_err(|e| e.to_string())?;
        if runner.status() != RunnerStatus::Passed || !seen || observer_sent {
            return Err(format!(
                "actor proof: {:?}; seen={seen}; observer_sent={observer_sent}",
                runner.status()
            ));
        }
        let ended =
            actor_observation(&peers[1].client, Some(actor)).ok_or("actor missing at expiry")?;
        if ended.overhead_text.is_some() || ended.remaining_cycles != 0 {
            return Err("native bubble did not expire".into());
        }
        observations
            .push(json!({"view":ended,"player_info_generation":peers[1].client.gens.player_info}));
        frames.push(capture(&mut peers[1], &directory, "expired-bubble")?);
        event(&mut log, "runner", runner_receipt.clone())?;
        Ok(())
    })();
    for peer in &mut peers {
        if peer.client.ingame {
            let ifaces = Arc::clone(&peer.client.ifaces);
            if !interact::logout(&mut peer.client, ifaces.as_slice()) && result.is_ok() {
                result = Err("clean logout unavailable".into());
            }
        }
    }
    if let Err(error) = wait_pair(&mut peers, "clean actor logouts", |peers| {
        peers.iter().all(|peer| !peer.client.ingame)
    }) {
        if result.is_ok() {
            result = Err(error);
        }
    }
    let receipt = json!({"cell":"host.actor-377-public-chat-lifetime-live",
        "outcome":if result.is_ok(){"PASS"}else{"FAIL"},"error":result.as_ref().err(),
        "identity":identity,"accounts":names,"input":INPUT,"text":TEXT,"observations":observations,
        "runner":runner_receipt,"frames":frames,"normal_gameshell_input":true,
        "synthetic_actor_or_chat_rows":false,"end_states":[peers[0].state(),peers[1].state()]});
    let path = directory.join(format!("{}-actor.json", names[0]));
    fs::write(
        &path,
        serde_json::to_vec_pretty(&receipt).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    event(&mut log, "verdict", receipt)?;
    println!(
        "{}",
        json!({"cell":"host.actor-377-public-chat-lifetime-live","outcome":if result.is_ok(){"PASS"}else{"FAIL"},"receipt":path})
    );
    result
}
