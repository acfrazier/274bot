//! Native Quester ownership checks for the 377 quest journal paths.
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
    xp: i32,
    level: i32,
    qp: i32,
    quest_colour: Option<i32>,
    inventory: Vec<(i32, i32)>,
    main_text: Vec<String>,
    main_root: i32,
    dialogue: Vec<String>,
    chat: Vec<(i32, String)>,
}

#[derive(Clone, Copy)]
struct Quest {
    key: &'static str,
    label: &'static str,
    var: &'static str,
    row_label: &'static str,
    terminal: &'static str,
    skill: i32,
    xp_delta: i32,
    qp_delta: i32,
    qp_reward: &'static str,
}

// quest_sheep.rs2:5 gives 150 XP. The fresh account also spins 20 balls:
// spinning.rs2:60-64 and spinning.struct:8-12 give 2.5 XP per ball.
const SHEEP: Quest = Quest {
    key: "sheep",
    label: "Sheep Shearer",
    row_label: "Sheep Shearer",
    var: "sheep",
    terminal: "22",
    skill: 12,
    xp_delta: 200,
    qp_delta: 1,
    qp_reward: "1 Quest Point",
};
const RUNE: Quest = Quest {
    key: "runemysteries",
    label: "Rune Mysteries",
    row_label: "Rune Mysteries Quest",
    var: "runemysteries",
    terminal: "6",
    skill: 20,
    xp_delta: 0,
    qp_delta: 1,
    qp_reward: "1 Quest Point",
};
const ROMEO: Quest = Quest {
    key: "romeojuliet",
    label: "Romeo & Juliet",
    row_label: "Romeo & Juliet",
    var: "rjquest",
    terminal: "100",
    skill: 0,
    xp_delta: 0,
    qp_delta: 5,
    qp_reward: "5 Quest Points",
};

impl Sample {
    fn read(client: &Client, snapshot: &GameSnapshot, quest: Quest) -> Self {
        let stat = snapshot.stats().iter().find(|row| row.index == quest.skill);
        Self {
            tick: snapshot.tick(),
            tile: snapshot.tile(),
            xp: stat.map_or(0, |row| row.xp),
            level: stat.map_or(1, |row| row.base),
            qp: client.var.get(101).copied().unwrap_or(-1),
            quest_colour: snapshot
                .quest_statuses()
                .iter()
                .find(|row| row.name == quest.row_label)
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
        self.xp == 0 && self.qp == 0 && self.quest_colour == Some(0xF80000)
    }
    fn complete(&self, q: Quest, base_xp: i32) -> bool {
        let completion = if q.key == "romeojuliet" {
            self.main_text
                .iter()
                .any(|s| s.contains("You have completed the"))
                && self
                    .main_text
                    .iter()
                    .any(|s| s.contains("Romeo & Juliet Quest!"))
        } else {
            self.main_text
                .iter()
                .any(|s| s.contains(&format!("You have completed the {} Quest!", q.label)))
        };
        let reward = match q.key {
            "sheep" => {
                self.main_text.iter().any(|s| s.contains("150 Crafting XP"))
                    && self.main_text.iter().any(|s| s == q.qp_reward)
            }
            "runemysteries" => {
                self.main_text.iter().any(|s| s == "Runecrafting skill")
                    && self.main_text.iter().any(|s| s == "Air talisman")
                    && self.main_text.iter().any(|s| s == q.qp_reward)
            }
            "romeojuliet" => self.main_text.iter().any(|s| s == q.qp_reward),
            _ => false,
        };
        self.xp - base_xp == q.xp_delta
            && self.qp == q.qp_delta
            && self.quest_colour == Some(0x00F800)
            && completion
            && reward
    }
}

fn native_run_completed(receipt: Option<&ScriptLifecycleReceipt>, generation: Option<u64>) -> bool {
    receipt.is_some_and(|r| {
        Some(r.runtime_generation) == generation && r.state == ScriptTerminalState::Completed
    })
}

fn witness_predicate(sample: &Sample, quest: Quest, base_xp: i32, baseline: bool) -> bool {
    if baseline {
        sample.fresh() && base_xp == 0
    } else {
        sample.complete(quest, base_xp)
    }
}

#[test]
fn journal_witness_rejects_stale_or_partial_product_evidence() {
    let fresh = Sample {
        tick: 1,
        tile: Some((3200, 3200, 0)),
        xp: 0,
        level: 1,
        qp: 0,
        quest_colour: Some(0xF80000),
        inventory: vec![],
        main_root: 0,
        main_text: vec![],
        dialogue: vec![],
        chat: vec![],
    };
    assert!(witness_predicate(&fresh, SHEEP, 0, true));
    for quest in [SHEEP, RUNE, ROMEO] {
        let mut completed = Sample {
            tick: 2,
            tile: fresh.tile,
            xp: quest.xp_delta,
            level: 1,
            qp: quest.qp_delta,
            quest_colour: Some(0x00F800),
            inventory: vec![],
            main_root: 0,
            main_text: vec![],
            dialogue: vec![],
            chat: vec![],
        };
        completed.main_text = match quest.key {
            "sheep" => vec![
                "You have completed the Sheep Shearer Quest!".into(),
                "150 Crafting XP".into(),
                "1 Quest Point".into(),
            ],
            "runemysteries" => vec![
                "You have completed the Rune Mysteries Quest!".into(),
                "Runecrafting skill".into(),
                "Air talisman".into(),
                "1 Quest Point".into(),
            ],
            _ => vec![
                "You have completed the".into(),
                "Romeo & Juliet Quest!".into(),
                "5 Quest Points".into(),
            ],
        };
        assert!(
            witness_predicate(&completed, quest, 0, false),
            "{} terminal witness",
            quest.key
        );
        completed.quest_colour = Some(0xF80000);
        assert!(
            !witness_predicate(&completed, quest, 0, false),
            "{} pending quest row",
            quest.key
        );
        completed.quest_colour = Some(0x00F800);
        assert!(
            !witness_predicate(&completed, quest, quest.xp_delta + 1, false),
            "{} incorrect XP delta",
            quest.key
        );
    }
    assert!(!native_run_completed(None, Some(1)));
    for state in [
        ScriptTerminalState::Stopped,
        ScriptTerminalState::Failed,
        ScriptTerminalState::Cancelled,
    ] {
        let receipt = ScriptLifecycleReceipt {
            runtime_generation: 4,
            state,
            tick: 1,
            reason: "fixture".into(),
        };
        assert!(!native_run_completed(Some(&receipt), Some(4)));
    }
    let completed = ScriptLifecycleReceipt {
        runtime_generation: 4,
        state: ScriptTerminalState::Completed,
        tick: 2,
        reason: "complete".into(),
    };
    assert!(native_run_completed(Some(&completed), Some(4)));
    assert!(!native_run_completed(Some(&completed), Some(5)));
}

struct Witness {
    snapshot: GameSnapshot,
    pump: Pump,
    log: fs::File,
    account: String,
    directory: PathBuf,
    writer: Arc<EvidenceWriter>,
    jobs: Vec<EvidenceJob>,
    quest: Quest,
    relog_requested: bool,
    relog_detached: bool,
    relogged: bool,
    baseline: Option<Sample>,
    latest: Option<Sample>,
    base_xp: Option<i32>,
    reads: usize,
    pending: Option<i32>,
    read_tick: Option<u32>,
    last_tick: Option<u32>,
    product_captured: bool,
    native_complete: bool,
    error: Option<String>,
}
impl Witness {
    fn record(&mut self, value: Value) -> Result<(), String> {
        writeln!(self.log, "{value}").map_err(|e| e.to_string())?;
        self.log.flush().map_err(|e| e.to_string())
    }
    fn capture(&mut self, client: &mut Client, phase: &str, sample: &Sample) -> Result<(), String> {
        let mut renderer = client::render::Renderer::new_prefer(client.config.lowmem, false);
        let draw = client.draw;
        client.set_draw(true);
        let out = renderer.mainredraw(client);
        client.set_draw(draw);
        let client::render::backend::FrameOutput::PixMap(pixels) = out else {
            return Err("377 Quester capture did not return CPU framebuffer".into());
        };
        let receipt = json!({"account":self.account,"phase":phase,"revision":377,"quest":self.quest.key,"sample":sample,"baseline":self.baseline,"driver":"production Play.script_start(Quester), bundled 377 quest path","renderer":"real Client CpuPix3D framebuffer","seeds":["own-account tutorial bypass and mainland position, then clean relog"],"quest_state_seeds":0,"quest_item_seeds":0,"reward_xp_seeds":0,"quest_point_seeds":0});
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
        host::publish_snapshot(&mut self.snapshot, client, self.pump.drain_client(client));
        if self.relog_requested && !self.snapshot.ingame() {
            self.relog_detached = true
        }
        if hold
            || !self.snapshot.ingame()
            || !self.snapshot.attached()
            || self.snapshot.scene_state() != 2
        {
            return Ok(());
        }
        let s = Sample::read(client, &self.snapshot, self.quest);
        self.latest = Some(s.clone());
        if self.last_tick != Some(s.tick) {
            self.record(json!({"phase":"observation","sample":s}))?;
            self.last_tick = Some(s.tick);
        }
        if !self.relogged {
            if self.relog_detached
                && self
                    .snapshot
                    .side_tabs()
                    .iter()
                    .any(|t| t.index == 3 && t.available)
            {
                self.relogged = true;
                self.record(json!({"phase":"fixture-relogged","sample":s}))?;
            } else {
                if !self.relog_requested
                    && s.tick >= 3
                    && s.tile
                        .is_some_and(|(x, z, l)| lumbridge(WorldTile { x, z, level: l }))
                {
                    let ifaces = Arc::clone(&client.ifaces);
                    if !interact::logout(client, &ifaces) {
                        return Err("could not cleanly logout mainland fixture".into());
                    }
                    self.relog_requested = true;
                    self.record(json!({"phase":"SEED own-account tutorial/mainland preparation; clean relog","sample":s}))?;
                }
                return Ok(());
            }
        }
        if self.baseline.is_none() {
            if !s.fresh() {
                return Err(format!("fresh-account quest conditions failed: {s:?}"));
            }
            self.base_xp = Some(s.xp);
            self.capture(client, "fresh", &s)?;
            self.baseline = Some(s.clone());
            return Ok(());
        }
        if s.complete(self.quest, self.base_xp.unwrap_or(0)) && !self.product_captured {
            self.capture(client, "product-completion", &s)?;
            self.product_captured = true;
        }
        let expected = [
            (
                format!("getvar {}", self.quest.var),
                format!("get {}: 0", self.quest.var),
            ),
            ("getvar qp".into(), "get qp: 0".into()),
            (
                format!("getvar {}", self.quest.var),
                format!("get {}: {}", self.quest.var, self.quest.terminal),
            ),
            (
                "getvar qp".into(),
                format!("get qp: {}", self.quest.qp_delta),
            ),
        ];
        if self.reads < 4 && (self.reads < 2 || (self.product_captured && self.native_complete)) {
            if let Some(seq) = self.pending {
                if s.chat
                    .iter()
                    .any(|(n, text)| *n > seq && text == &expected[self.reads].1)
                {
                    self.record(json!({"phase":"SERVER READBACK observed","command":expected[self.reads].0,"reply":expected[self.reads].1,"sample":s}))?;
                    self.reads += 1;
                    self.pending = None;
                }
            } else if self.read_tick != Some(s.tick) {
                let before = s.chat.iter().map(|(n, _)| *n).max().unwrap_or(-1);
                if !interact::cheat(client, &expected[self.reads].0).is_sent() {
                    return Err("own-account getvar read refused".into());
                }
                self.record(json!({"phase":"SERVER READBACK request, own-account read-only","command":expected[self.reads].0,"after_sequence":before}))?;
                self.pending = Some(before);
                self.read_tick = Some(s.tick);
            }
        }
        Ok(())
    }
}

fn run_quest(q: Quest) -> Result<(), String> {
    let bound = load_bound_377()?;
    identity_record(&bound);
    let account = host_play::mint_live_names(1).remove(0);
    let directory = required_path("QUESTER_377_EVIDENCE_DIR")?.join(&account);
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let log = fs::File::create(directory.join("observations.jsonl")).map_err(|e| e.to_string())?;
    let writer = Arc::new(EvidenceWriter::new(4));
    let state = Arc::new(Mutex::new(Witness {
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        log,
        account: account.clone(),
        directory,
        writer: writer.clone(),
        jobs: Vec::new(),
        quest: q,
        relog_requested: false,
        relog_detached: false,
        relogged: false,
        baseline: None,
        latest: None,
        base_xp: None,
        reads: 0,
        pending: None,
        read_tick: None,
        last_tick: None,
        product_captured: false,
        native_complete: false,
        error: None,
    }));
    let frame = state.clone();
    let name = account.clone();
    let mut play = host_play::run_with_template(
        Arc::clone(&bound.template),
        true,
        vec![],
        |_| (None, None),
        move |c, u, f| {
            if u == name {
                let mut s = frame.lock().unwrap();
                if s.error.is_none() {
                    if let Err(e) = s.frame(c, f.hold) {
                        s.error = Some(e)
                    }
                }
            }
        },
    )?;
    play.try_spawn_slot(
        Profile {
            username: account.clone(),
            password: host_play::profile_password(&account).into(),
            uid: fresh_uid(300),
            settings: ProfileSettings::default(),
        },
        None,
        None,
        None,
    )?;
    let deadline = Instant::now() + Duration::from_secs(1500);
    let prep = Instant::now() + Duration::from_secs(180);
    let mut started = false;
    let mut complete = false;
    let mut generation = None;
    let mut lifecycle = None;
    let mut last_status = String::new();
    let result = (|| loop {
        if Path::new("/Volumes/dev-scratch/274bot-evidence/MAC-QUIET.lock").exists() {
            return Err("MAC-QUIET.lock appeared".into());
        }
        if let Some(error) = play.script_last_error(&account) {
            return Err(format!("native Quester lifecycle: {error}"));
        }
        let status = play.script_native_status(&account);
        let receipt = play.script_lifecycle_receipt(&account);
        let mut s = state.lock().map_err(|_| "witness poisoned")?;
        if let Some(error) = &s.error {
            return Err(error.clone());
        }
        // Initial readbacks must precede Start, not race the first quest action.
        if !started && s.baseline.is_some() && s.reads == 2 {
            s.record(json!({"phase":"native Quester start", "quest":q.key}))?;
            drop(s);
            play.script_start(
                &account,
                script::CompiledId("Quester"),
                serde_json::Map::from_iter([("quests".into(), json!([q.key]))]),
            )?;
            started = true;
            continue;
        }
        if let Some(status) = status {
            let detail = format!("{status:?}");
            if detail != last_status {
                let event = json!({"phase":"native status", "status":detail, "latest":s.latest});
                s.record(event)?;
                last_status = detail;
            }
            if status.phase == NativePhase::Blocked || status.failure.is_some() {
                return Err(format!("native Quester failed: {status:?}"));
            }
            generation.get_or_insert(status.run.run);
        }
        if !complete && native_run_completed(receipt.as_ref(), generation) {
            s.record(json!({"phase":"native lifecycle", "receipt":receipt}))?;
            lifecycle = receipt;
            complete = true;
            s.native_complete = true;
        }
        if complete && s.product_captured && s.reads == 4 {
            let event = json!({"outcome":"PASS", "account":account, "native_lifecycle":lifecycle, "baseline":s.baseline, "latest":s.latest});
            s.record(event)?;
            return Ok(());
        }
        if Instant::now() >= deadline || (!started && Instant::now() >= prep) {
            return Err(format!(
                "quest witness deadline started={started} complete={complete} reads={} latest={:?}",
                s.reads, s.latest
            ));
        }
        drop(s);
        std::thread::sleep(Duration::from_millis(20));
    })();
    play.script_stop(&account);
    play.stop_slot(&account);
    drop(play);
    let mut s = state.lock().map_err(|_| "witness poisoned")?;
    let event = json!({"phase":"final","outcome":if result.is_ok(){"PASS"}else{"FAIL"},"error":result.as_ref().err(),"quest":q.key,"latest":s.latest});
    s.record(event)?;
    let jobs = s.jobs.clone();
    drop(s);
    for job in jobs {
        let outcome = writer
            .wait(&job, Duration::from_secs(30))
            .ok_or("screenshot write timed out")?;
        if let Some(e) = outcome.error {
            return Err(e);
        }
    }
    result
}

#[test]
#[ignore = "requires LIVE=1, owned qj3 prefix, canonical 377 inputs and QUESTER_377_EVIDENCE_DIR"]
fn quester_377_sheep_native() -> Result<(), String> {
    run_quest(SHEEP)
}
#[test]
#[ignore = "requires LIVE=1, owned qj3 prefix, canonical 377 inputs and QUESTER_377_EVIDENCE_DIR"]
fn quester_377_runemysteries_native() -> Result<(), String> {
    run_quest(RUNE)
}
#[test]
#[ignore = "requires LIVE=1, owned qj3 prefix, canonical 377 inputs and QUESTER_377_EVIDENCE_DIR"]
fn quester_377_romeojuliet_native() -> Result<(), String> {
    run_quest(ROMEO)
}
