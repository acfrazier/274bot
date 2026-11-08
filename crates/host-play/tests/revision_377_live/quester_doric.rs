//! Native Quester owns every quest action. The witness only prepares the own-account
//! mainland fixture, reads server variables, and captures the real client UI.
use super::*;
use host_play::evidence_writer::{
    EvidenceJob, EvidenceRequest, EvidenceSidecar, EvidenceWriter, PngColor,
};
use script::native::NativePhase;
use script::{ScriptLifecycleReceipt, ScriptTerminalState};
use std::io::Write;
use std::sync::Mutex;
use vault::{Profile, ProfileSettings};

#[derive(Clone, Debug, serde::Serialize)]
struct Sample {
    tick: u32,
    tile: Option<(i32, i32, i32)>,
    mining_xp: i32,
    mining_level: i32,
    quest_points: i32,
    doric_colour: Option<i32>,
    inventory: Vec<(i32, i32)>,
    main_root: i32,
    main_text: Vec<String>,
    dialogue: Vec<String>,
    chat: Vec<(i32, String)>,
}

impl Sample {
    fn read(client: &Client, snapshot: &GameSnapshot) -> Self {
        let mining = snapshot.stats().iter().find(|row| row.index == 14);
        Self {
            tick: snapshot.tick(),
            tile: snapshot.tile(),
            mining_xp: mining.map_or(-1, |row| row.xp),
            mining_level: mining.map_or(-1, |row| row.base),
            quest_points: client.var.get(101).copied().unwrap_or(-1),
            doric_colour: snapshot
                .quest_statuses()
                .iter()
                .find(|row| row.name == "Doric's Quest")
                .map(|row| row.colour),
            inventory: snapshot
                .inventory()
                .iter()
                .map(|row| (row.def.id, row.count))
                .collect(),
            main_root: snapshot.modals().main,
            main_text: snapshot.main_modal_texts().to_vec(),
            dialogue: snapshot.chat_modal_texts().to_vec(),
            chat: snapshot
                .chat_lines()
                .iter()
                .map(|row| (row.sequence, row.text.clone()))
                .collect(),
        }
    }

    fn fresh(&self) -> bool {
        self.mining_xp == 0 && self.quest_points == 0 && self.doric_colour == Some(0xF80000)
    }

    fn product_complete(&self) -> bool {
        // Six clay (5 XP each) and four copper ore (17.5 XP each) are
        // mined by this Path before Doric grants the 1,300 XP reward.
        self.mining_xp == 1400
            && self.mining_level == 11
            && self.quest_points == 1
            && self.doric_colour == Some(0x00F800)
            && self.main_text.iter().any(|text| text == "1 Quest Point")
            && self.main_text.iter().any(|text| text == "1,300 Mining XP")
    }
}

// Native Complete status is destroyed with the compiled instance in the same
// poll. The lifecycle receipt survives that teardown and fences this Start.
fn native_run_completed(receipt: Option<&ScriptLifecycleReceipt>, generation: Option<u64>) -> bool {
    receipt.is_some_and(|receipt| {
        Some(receipt.runtime_generation) == generation
            && receipt.state == ScriptTerminalState::Completed
    })
}

#[test]
fn native_completion_requires_matching_completed_lifecycle_receipt() {
    assert!(!native_run_completed(None, Some(7)));
    for terminal in [
        ScriptTerminalState::Stopped,
        ScriptTerminalState::Completed,
        ScriptTerminalState::Failed,
        ScriptTerminalState::Cancelled,
    ] {
        let receipt = ScriptLifecycleReceipt {
            runtime_generation: 7,
            state: terminal,
            tick: 803,
            reason: "native terminal".into(),
        };
        assert_eq!(
            native_run_completed(Some(&receipt), Some(7)),
            terminal == ScriptTerminalState::Completed
        );
        assert!(!native_run_completed(Some(&receipt), Some(8)));
        assert!(!native_run_completed(Some(&receipt), None));
    }
}

struct Witness {
    snapshot: GameSnapshot,
    pump: Pump,
    log: fs::File,
    account: String,
    directory: PathBuf,
    writer: Arc<EvidenceWriter>,
    jobs: Vec<EvidenceJob>,
    fixture_index: usize,
    fixture_tick: Option<u32>,
    relog_requested: bool,
    relog_detached: bool,
    relogged: bool,
    read_index: usize,
    pending_read: Option<i32>,
    last_read_tick: Option<u32>,
    last_tick: Option<u32>,
    baseline: Option<Sample>,
    latest: Option<Sample>,
    product_captured: bool,
    native_complete: bool,
    error: Option<String>,
}

impl Witness {
    fn record(&mut self, record: Value) -> Result<(), String> {
        writeln!(self.log, "{record}").map_err(|error| error.to_string())?;
        self.log.flush().map_err(|error| error.to_string())
    }

    fn capture(&mut self, client: &mut Client, phase: &str, sample: &Sample) -> Result<(), String> {
        let mut renderer = client::render::Renderer::new_prefer(client.config.lowmem, false);
        let was_draw = client.draw;
        client.set_draw(true);
        let output = renderer.mainredraw(client);
        client.set_draw(was_draw);
        let client::render::backend::FrameOutput::PixMap(pixels) = output else {
            return Err("377 Quester capture did not return the client CPU framebuffer".into());
        };
        let receipt = json!({
            "account": self.account, "phase": phase, "revision": 377,
            "sample": sample, "baseline": self.baseline,
            "driver": "production Play.script_start(Quester), bundled 377 Doric's Quest Path",
            "renderer": "real Client CpuPix3D framebuffer",
            "seeds": ["own account tutorial bypass and mainland position, then clean relog", "own account combat stats 40, bronze pickaxe and 6 cooked meat; see labelled commands"],
            "quest_state_seeds": 0, "food_seed": 6, "tool_seed": "bronze_pickaxe", "quest_item_seeds": 0, "reward_xp_seeds": 0, "quest_point_seeds": 0,
        });
        self.record(receipt.clone())?;
        self.jobs.push(self.writer.submit(EvidenceRequest {
            png_path: self.directory.join(format!("{phase}.png")),
            width: pixels.width as u32,
            height: pixels.height as u32,
            pixels: pixels.pixels,
            color: PngColor::Rgba,
            sidecar: Some(EvidenceSidecar {
                path: self.directory.join(format!("{phase}.json")),
                receipt,
                patch_error: None,
            }),
        }));
        Ok(())
    }

    fn frame(&mut self, client: &mut Client, hold: bool) -> Result<(), String> {
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);
        if self.relog_requested && !self.snapshot.ingame() {
            self.relog_detached = true;
        }
        if hold
            || !self.snapshot.ingame()
            || !self.snapshot.attached()
            || self.snapshot.scene_state() != 2
        {
            return Ok(());
        }
        let sample = Sample::read(client, &self.snapshot);
        self.latest = Some(sample.clone());
        if !self.relogged {
            if self.relog_detached
                && self
                    .snapshot
                    .side_tabs()
                    .iter()
                    .any(|tab| tab.index == 3 && tab.available)
            {
                self.relogged = true;
                self.record(json!({"phase":"fixture-relogged", "sample":sample}))?;
            } else {
                if !self.relog_requested
                    && sample.tick >= 3
                    && sample
                        .tile
                        .is_some_and(|(x, z, level)| lumbridge(WorldTile { x, z, level }))
                {
                    let ifaces = Arc::clone(&client.ifaces);
                    if !interact::logout(client, &ifaces) {
                        return Err("377 Quester mainland fixture could not cleanly log out".into());
                    }
                    self.relog_requested = true;
                    self.record(json!({"phase":"SEED own-account tutorial/mainland preparation complete; clean relog", "sample":sample}))?;
                }
                return Ok(());
            }
        }
        let fixture = [
            "setstat attack 40",
            "setstat strength 40",
            "setstat defence 40",
            "setstat hitpoints 40",
            "give bronze_pickaxe 1",
            "give cooked_meat 6",
        ];
        if self.fixture_index < fixture.len() {
            if self.fixture_tick != Some(sample.tick) {
                let command = fixture[self.fixture_index];
                if !interact::cheat(client, command).is_sent() {
                    return Err(format!("combat fixture command refused: {command}"));
                }
                self.record(json!({"phase":"SEED own-account combat stats, pickaxe and food", "command":command, "tick":sample.tick}))?;
                self.fixture_index += 1;
                self.fixture_tick = Some(sample.tick);
            }
            return Ok(());
        }
        if self.fixture_tick == Some(sample.tick) {
            return Ok(());
        }
        if self.last_tick != Some(sample.tick) {
            self.record(json!({"phase":"observation", "sample":sample}))?;
            self.last_tick = Some(sample.tick);
        }
        if sample.product_complete() && !self.product_captured {
            self.capture(client, "product-completion", &sample)?;
            self.product_captured = true;
        }
        let readbacks = [
            ("getvar doricquest", "get doricquest: 0"),
            ("getvar qp", "get qp: 0"),
            ("getvar doricquest", "get doricquest: 100"),
            ("getvar qp", "get qp: 1"),
        ];
        if self.read_index < readbacks.len()
            && (self.read_index < 2 || (self.product_captured && self.native_complete))
        {
            let (command, expected) = readbacks[self.read_index];
            if let Some(before) = self.pending_read {
                if sample
                    .chat
                    .iter()
                    .any(|(sequence, text)| *sequence > before && text == expected)
                {
                    self.record(json!({"phase":"SERVER READBACK observed", "command":command, "reply":expected, "sample":sample}))?;
                    self.read_index += 1;
                    self.pending_read = None;
                }
            } else if self.last_read_tick != Some(sample.tick) {
                let before = sample
                    .chat
                    .iter()
                    .map(|(sequence, _)| *sequence)
                    .max()
                    .unwrap_or(-1);
                if !interact::cheat(client, command).is_sent() {
                    return Err(format!("SERVER READBACK command refused: {command}"));
                }
                self.record(json!({"phase":"SERVER READBACK request, read-only own account", "command":command, "after_sequence":before, "tick":sample.tick}))?;
                self.pending_read = Some(before);
                self.last_read_tick = Some(sample.tick);
            }
        }
        if self.read_index == 2 && self.baseline.is_none() {
            if !sample.fresh() {
                return Err(format!("fresh-account conditions failed: {sample:?}"));
            }
            self.capture(client, "fresh", &sample)?;
            self.baseline = Some(sample);
        }
        Ok(())
    }
}

#[test]
#[ignore = "requires LIVE=1, owned qd3 prefix, canonical 377 inputs and QUESTER_377_EVIDENCE_DIR"]
fn quester_377_doric_native() -> Result<(), String> {
    let bound = load_bound_377()?;
    resource_record(&bound);
    let account = host_play::mint_live_names(1).remove(0);
    let directory = required_path("QUESTER_377_EVIDENCE_DIR")?.join(&account);
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let log = fs::File::create(directory.join("observations.jsonl"))
        .map_err(|error| error.to_string())?;
    let writer = Arc::new(EvidenceWriter::new(4));
    let state = Arc::new(Mutex::new(Witness {
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        log,
        account: account.clone(),
        directory,
        writer: Arc::clone(&writer),
        jobs: Vec::new(),
        fixture_index: 0,
        fixture_tick: None,
        relog_requested: false,
        relog_detached: false,
        relogged: false,
        read_index: 0,
        pending_read: None,
        last_read_tick: None,
        last_tick: None,
        baseline: None,
        latest: None,
        product_captured: false,
        native_complete: false,
        error: None,
    }));
    let frame_state = Arc::clone(&state);
    let frame_account = account.clone();
    let mut play = host_play::run_with_template(
        Arc::clone(&bound.template),
        true,
        vec![],
        |_| (None, None),
        move |client, username, frame| {
            if username == frame_account {
                let mut state = frame_state.lock().unwrap();
                if state.error.is_none() {
                    if let Err(error) = state.frame(client, frame.hold) {
                        state.error = Some(error);
                    }
                }
            }
        },
    )?;
    play.try_spawn_slot(
        Profile {
            username: account.clone(),
            password: host_play::profile_password(&account).into(),
            uid: fresh_uid(90),
            settings: ProfileSettings::default(),
        },
        None,
        None,
        None,
    )?;
    let deadline = Instant::now() + Duration::from_secs(3600);
    let preparation_deadline = Instant::now() + Duration::from_secs(180);
    let mut started = false;
    let mut relogin_requested = false;
    let mut complete = false;
    let mut last_status = String::new();
    let mut run_generation = None;
    let mut completion = None;
    let result = (|| loop {
        if Path::new("/Volumes/dev-scratch/274bot-evidence/MAC-QUIET.lock").exists() {
            return Err("MAC-QUIET.lock appeared".into());
        }
        if let Some(error) = play.script_last_error(&account) {
            return Err(format!("native Quester lifecycle: {error}"));
        }
        let status = play.script_native_status(&account);
        let lifecycle = play.script_lifecycle_receipt(&account);
        let mut state = state.lock().map_err(|_| "377 Quester witness poisoned")?;
        if let Some(error) = &state.error {
            return Err(error.clone());
        }
        if !relogin_requested && state.relog_detached {
            drop(state);
            if let Some(arm) = play.arm(&account) {
                arm.arm_explicit_login();
                relogin_requested = true;
            }
            continue;
        }
        if !started && state.baseline.is_some() {
            state.record(
                json!({"phase":"native Quester start", "account":account, "quests":["doric"]}),
            )?;
            drop(state);
            play.script_start(
                &account,
                script::CompiledId("Quester"),
                serde_json::Map::from_iter([("quests".into(), json!(["doric"]))]),
            )?;
            started = true;
            continue;
        }
        if let Some(status) = status {
            let detail = format!("{status:?}");
            if detail != last_status {
                let event =
                    json!({"phase":"native status", "status":detail, "latest":state.latest});
                state.record(event)?;
                println!(
                    "{}",
                    json!({"account":account, "phase":"native status", "status":detail, "latest":state.latest})
                );
                last_status = detail;
            }
            if status.phase == NativePhase::Blocked || status.failure.is_some() {
                return Err(format!("native Quester failed: {status:?}"));
            }
            run_generation.get_or_insert(status.run.run);
        }
        if !complete && native_run_completed(lifecycle.as_ref(), run_generation) {
            let receipt = lifecycle.expect("matching native completion receipt");
            state.record(json!({"phase":"native lifecycle", "receipt":receipt}))?;
            println!(
                "{}",
                json!({"account":account, "phase":"native lifecycle", "receipt":receipt})
            );
            completion = Some(receipt);
            complete = true;
            state.native_complete = true;
        }
        if complete && state.product_captured && state.read_index == 4 {
            let event = json!({"outcome":"PASS", "account":account, "native_complete":complete, "native_lifecycle":completion, "baseline":state.baseline, "latest":state.latest});
            state.record(event)?;
            println!(
                "{}",
                json!({"outcome":"PASS", "test":"quester_377_doric_native", "account":account, "native_complete":complete, "native_lifecycle":completion, "latest":state.latest})
            );
            return Ok(());
        }
        if Instant::now() >= deadline || (!started && Instant::now() >= preparation_deadline) {
            return Err(format!(
                "377 Quester deadline: started={started} complete={complete} reads={} latest={:?}",
                state.read_index, state.latest
            ));
        }
        drop(state);
        std::thread::sleep(Duration::from_millis(20));
    })();
    play.script_stop(&account);
    play.stop_slot(&account);
    drop(play);
    let mut state = state.lock().map_err(|_| "377 Quester witness poisoned")?;
    let event = json!({"phase":"final", "outcome":if result.is_ok(){"PASS"}else{"FAIL"}, "error":result.as_ref().err(), "account":account, "latest":state.latest});
    state.record(event)?;
    let jobs = state.jobs.clone();
    drop(state);
    for job in jobs {
        let outcome = writer
            .wait(&job, Duration::from_secs(30))
            .ok_or("377 Quester screenshot write timed out")?;
        if let Some(error) = outcome.error {
            return Err(error);
        }
    }
    result
}
