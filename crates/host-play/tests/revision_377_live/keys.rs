//! One owned account, normal native arrow input, existing camera snapshots.
use super::*;
use api::native_input::{GestureKey, KeyPhase};
use host::{KeyOwner, SlotInput};
use host_play::evidence_writer::{
    record_scenario_sources, EvidenceRequest, EvidenceWriter, PngColor,
};
use scenario::{
    Proof, RunnerStatus, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait,
};

#[test]
#[ignore = "requires LIVE=1, canonical 377 resources, owned prefix and PARITY_KEYS_EVIDENCE_DIR"]
fn native_key_duration_377() -> Result<(), String> {
    let bound = load_bound_377()?;
    resource_record(&bound);
    let directory = required_path("PARITY_KEYS_EVIDENCE_DIR")?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let name = host_play::mint_live_names(1).remove(0);
    let mut client = bound.template.prepare_client(fresh_uid(310), false)?;
    client.set_draw(false);
    client.maininit();
    assert_377_client(&client, false)?;
    if client.error_loading {
        return Err(format!(
            "key fixture assets: {}",
            client.last_progress_message
        ));
    }
    let mut snapshot = GameSnapshot::new();
    let mut pump = Pump::new();
    let input = SlotInput::new();
    let result = (|| {
        if !interact::login(
            &mut client,
            &name,
            &host_play::profile_password(&name),
            false,
        ) {
            return Err("key fixture login refused".into());
        }
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "key fixture scene",
            Duration::from_secs(90),
            |state| state.ingame() && state.attached() && state.scene_state() == 2,
        )?;
        let mut runner = ScenarioRunner::with_world(
            Scenario {
                name: "native_key_duration_377",
                seed: Seed {
                    profiles: vec![("keys", "unused")],
                    mainland: false,
                },
                steps: vec![Step {
                    name: "native left arrow for 400 milliseconds",
                    kind: StepKind::KeyGesture {
                        key: GestureKey::ArrowLeft,
                        duration_ms: 400,
                    },
                    wait: Wait {
                        arm: Proof::CameraDelta,
                        budget_ticks: 12,
                    },
                }],
                proof: Proof::CameraDelta,
                companions: vec![],
                settings: ScenarioSettings {
                    require_mainland_base: false,
                    ..ScenarioSettings::default()
                },
            },
            None,
        );
        runner.set_live_names(std::slice::from_ref(&name));
        runner.set_scene_settle(Duration::ZERO);
        runner.set_key_input(input.authority());
        record_scenario_sources(&mut runner, Some(client.revision().as_i32()));
        let writer = EvidenceWriter::new(2);
        let mut renderer = client::render::Renderer::new_prefer(false, false);
        let mut jobs = Vec::new();
        let ch = GestureKey::ArrowLeft.shell_char();
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut held_frames = 0;
        for phase in ["before", "released"] {
            if phase == "released" {
                loop {
                    let started = Instant::now();
                    input.consume_native_frame(&mut client.shell);
                    held_frames += u32::from(client.shell.key_held[ch as usize] != 0);
                    client.mainloop();
                    host::publish_snapshot(&mut snapshot, &client, pump.drain_client(&client));
                    runner.tick(&mut client);
                    match runner.status() {
                        RunnerStatus::Passed => break,
                        RunnerStatus::Failed(reason) => {
                            return Err(format!("key scenario: {reason}"))
                        }
                        _ => {}
                    }
                    if !client.ingame || Instant::now() >= deadline {
                        return Err("key scenario disconnected or exceeded thirty seconds".into());
                    }
                    if let Some(rest) = Duration::from_millis(20).checked_sub(started.elapsed()) {
                        std::thread::sleep(rest);
                    }
                }
            }
            client.set_draw(true);
            let client::render::backend::FrameOutput::PixMap(pixels) =
                renderer.mainredraw(&mut client)
            else {
                return Err("key frame was not a CPU PixMap".into());
            };
            client.set_draw(false);
            jobs.push(writer.submit(EvidenceRequest {
                png_path: directory.join(format!("{phase}.png")),
                width: pixels.width as u32,
                height: pixels.height as u32,
                pixels: pixels.pixels,
                color: PngColor::Rgba,
                sidecar: None,
            }));
        }
        let key = runner
            .evidence()
            .and_then(|evidence| evidence.keys.first())
            .ok_or("missing native key receipt")?;
        if key.input.phase != KeyPhase::Released
            || key.input.held_ms < 400
            || key.camera_before == key.camera_after
            || held_frames == 0
        {
            return Err(format!("native key effect or duration not proven: {key:?}"));
        }
        for _ in 0..30 {
            input.consume_native_frame(&mut client.shell);
            if client.shell.key_held[ch as usize] != 0 || input.key_owner(ch) != KeyOwner::None {
                return Err("native key leaked after release".into());
            }
            client.mainloop();
            host::publish_snapshot(&mut snapshot, &client, pump.drain_client(&client));
            std::thread::sleep(Duration::from_millis(20));
        }
        let manifest = runner
            .receipt_manifest_mut()
            .ok_or("key receipt manifest missing")?;
        writer.record_artifacts(&jobs, &directory, manifest)?;
        let evidence = runner.evidence().ok_or("key receipt missing")?;
        fs::write(directory.join("runner.json"), evidence.to_json())
            .map_err(|error| error.to_string())?;
        fs::write(directory.join("release.json"), json!({"outcome":"PASS","account":name,"held_frames":held_frames,"post_release_frames":30,"shell_key_held":client.shell.key_held[ch as usize],"owner":"None","camera":snapshot.camera()}).to_string()).map_err(|error| error.to_string())?;
        println!("SCENARIO_RECEIPT {}", evidence.to_json());
        Ok(())
    })();
    input.cancel_native_keys(&mut client.shell);
    let logout = (|| {
        if client.ingame {
            let interfaces = Arc::clone(&client.ifaces);
            if !interact::logout(&mut client, interfaces.as_slice()) {
                return Err("key clean logout unavailable".to_string());
            }
            wait_for(
                &mut client,
                &mut snapshot,
                &mut pump,
                "key clean logout",
                Duration::from_secs(30),
                |state| !state.ingame(),
            )?;
        }
        fs::write(directory.join("logout.json"), json!({"account":name,"ingame":client.ingame,"shell_key_held":client.shell.key_held[GestureKey::ArrowLeft.shell_char() as usize]}).to_string()).map_err(|error| error.to_string())?;
        Ok(())
    })();
    match (result, logout) {
        (Err(error), Err(cleanup)) => Err(format!("{error}; cleanup: {cleanup}")),
        (Err(error), _) | (_, Err(error)) => Err(error),
        _ => Ok(()),
    }
}
