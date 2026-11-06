//! Finite selected-377 combat, Survival and individual clue witnesses.
//! Account-only fixtures end before the native machine starts.

use super::*;
use host_play::evidence_writer::{
    EvidenceJob, EvidenceRequest, EvidenceSidecar, EvidenceWriter, PngColor,
};
use script::combat::{
    Allowances, Combat, CombatEnd, CombatReport, CombatRequest, CombatTables, CompiledKit,
    MeleeMode, Pick, Target,
};
use script::native::walk::Walk;
use script::native::{
    ActionHandle, NativeTick, Script, ScriptFailure, ScriptFlow, WalkReceipt, WalkRequest,
};
use script::quester::families::reach::{Reach, ReachArgs, ReachKind};
use std::sync::Mutex;
use std::task::Poll;
use vault::{Profile, ProfileSettings};

#[derive(Clone, Copy, Debug)]
enum Case {
    Cow,
    Eat,
    Cure,
    Admission,
    #[cfg(feature = "js")]
    HansClue,
    #[cfg(feature = "js")]
    HansHardClue,
}

impl Case {
    fn name(self) -> &'static str {
        match self {
            Self::Cow => "cow-food-loot",
            Self::Eat => "survival-eat",
            Self::Cure => "survival-cure",
            Self::Admission => "route-admission",
            #[cfg(feature = "js")]
            Self::HansClue => "clue-hans",
            #[cfg(feature = "js")]
            Self::HansHardClue => "clue-hans-hard",
        }
    }

    fn start(self) -> WorldTile {
        let (x, z) = match self {
            Self::Cow => (3255, 3278),
            Self::Eat | Self::Cure => (3190, 3245),
            Self::Admission => (3190, 3245),
            #[cfg(feature = "js")]
            Self::HansClue => (3222, 3222),
            #[cfg(feature = "js")]
            Self::HansHardClue => (3222, 3222),
        };
        WorldTile { x, z, level: 0 }
    }

    fn clue_alias(self) -> Option<&'static str> {
        match self {
            #[cfg(feature = "js")]
            Self::HansClue => Some("trail_clue_easy_simple005"),
            #[cfg(feature = "js")]
            Self::HansHardClue => Some("trail_clue_hard_riddle012"),
            _ => None,
        }
    }

    fn commands(self) -> Vec<String> {
        let tile = self.start();
        let mut commands = vec![
            "~clearinv".into(),
            "~clearinv worn".into(),
            "~clearbank".into(),
        ];
        if let Some(alias) = self.clue_alias() {
            commands.push(format!("give {alias} 1"));
        } else {
            commands.extend([
                "setstat attack 40".into(),
                "setstat strength 40".into(),
                "setstat defence 40".into(),
                format!(
                    "setstat hitpoints {}",
                    if matches!(self, Self::Eat) { 15 } else { 40 }
                ),
            ]);
            if matches!(self, Self::Cow) {
                commands.push("give bronze_sword 1".into());
            }
            commands.push(format!(
                "give {} 5",
                if matches!(self, Self::Admission) {
                    "cooked_chompy"
                } else {
                    "lobster"
                }
            ));
            if matches!(self, Self::Cure) {
                commands.push("give 1doseantipoison 1".into());
            }
        }
        commands.push(interact::tele_args(tile.level, tile.x, tile.z));
        if matches!(self, Self::Eat) {
            commands.push("~hit 12".into());
        }
        if matches!(self, Self::Eat | Self::Cure) {
            // _test/scripts/cheats/cheat_other.rs2:4-6 queues the real poison proc.
            // poison.rs2:68-71 gives a 30-tick first-fire window.
            commands.push("~poison 50".into());
        }
        commands
    }
}

struct Facts {
    food: i32,
    cure: i32,
    vial: i32,
    weapon: i32,
    cow: i32,
    loot: i32,
    loot_name: Arc<str>,
    clue: Option<i32>,
}

impl Facts {
    fn selected(data: &api::game_data::SelectedGameData, case: Case) -> Result<Self, String> {
        if data.revision() != 377 {
            return Err("machine qualification requires selected 377 data".into());
        }
        let item = |alias: &str| {
            data.item_by_alias(alias)
                .map(|row| row.id)
                .ok_or_else(|| format!("377 item unavailable: {alias}"))
        };
        let loot = data
            .item_by_alias("cow_hide")
            .ok_or("377 cowhide unavailable")?;
        let clue = case
            .clue_alias()
            .map(|alias| {
                data.trails()
                    .and_then(|facts| facts.rows.iter().find(|row| row.alias == alias))
                    .map(|row| row.id)
                    .ok_or_else(|| format!("377 trail unavailable: {alias}"))
            })
            .transpose()?;
        Ok(Self {
            food: item(if matches!(case, Case::Admission) {
                "cooked_chompy"
            } else {
                "lobster"
            })?,
            cure: item("1doseantipoison")?,
            vial: item("vial_empty")?,
            weapon: item("bronze_sword")?,
            cow: data.npc_by_config("cow").ok_or("377 cow unavailable")?.id,
            loot: loot.id,
            loot_name: Arc::from(loot.name.as_deref().ok_or("cowhide name unavailable")?),
            clue,
        })
    }
}

#[derive(Clone, Debug, Default, serde::Serialize)]
struct Sample {
    tick: u32,
    tile: Option<(i32, i32, i32)>,
    hp: i32,
    hp_max: i32,
    combat_xp: i32,
    food: i32,
    cure: i32,
    vial: i32,
    weapon: i32,
    loot: i32,
    ground_loot: usize,
    clue: i32,
    replacement: bool,
    poison_line: bool,
    drink_line: bool,
    chat: Vec<(i32, String)>,
}

fn count(snapshot: &GameSnapshot, id: i32) -> i32 {
    snapshot
        .inventory()
        .iter()
        .filter(|row| row.def.id == id)
        .map(|row| row.count)
        .sum()
}

impl Sample {
    fn read(snapshot: &GameSnapshot, facts: &Facts) -> Self {
        let hp = snapshot.stats().iter().find(|row| row.index == 3);
        Self {
            tick: snapshot.tick(),
            tile: snapshot.tile(),
            hp: hp.map_or(0, |row| row.effective),
            hp_max: hp.map_or(0, |row| row.base),
            combat_xp: snapshot
                .stats()
                .iter()
                .filter(|row| (0..=3).contains(&row.index))
                .map(|row| row.xp)
                .sum(),
            food: count(snapshot, facts.food),
            cure: count(snapshot, facts.cure),
            vial: count(snapshot, facts.vial),
            weapon: count(snapshot, facts.weapon)
                + snapshot
                    .equipment()
                    .iter()
                    .filter(|row| row.def.id == facts.weapon)
                    .map(|row| row.count)
                    .sum::<i32>(),
            loot: count(snapshot, facts.loot),
            ground_loot: snapshot
                .ground_items()
                .iter()
                .filter(|row| row.def.id == facts.loot)
                .count(),
            clue: facts.clue.map_or(0, |id| count(snapshot, id)),
            replacement: facts.clue.is_some_and(|seeded| {
                snapshot.inventory().iter().any(|row| {
                    row.def.id != seeded
                        && row.count > 0
                        && matches!(row.def.name.as_deref(), Some("Clue scroll" | "Casket"))
                })
            }),
            poison_line: snapshot
                .chat_lines()
                .iter()
                .any(|row| row.text == "You have been poisoned!"),
            drink_line: snapshot
                .chat_lines()
                .iter()
                .any(|row| row.text == "You drink some of your antipoison potion."),
            chat: snapshot
                .chat_lines()
                .iter()
                .map(|row| (row.sequence, row.text.clone()))
                .collect(),
        }
    }

    fn prepared(&self, case: Case) -> bool {
        let at = case.start();
        if self.tile != Some((at.x, at.z, at.level)) || self.loot != 0 {
            return false;
        }
        match case {
            Case::Cow => {
                self.food == 5
                    && self.weapon == 1
                    && self.hp == 40
                    && self.hp_max == 40
                    && self.ground_loot == 0
            }
            Case::Eat => self.food == 5 && self.hp == 3 && self.hp_max == 15 && self.poison_line,
            Case::Admission => self.food == 5 && self.hp == 40 && self.hp_max == 40,
            Case::Cure => {
                self.food == 5
                    && self.cure == 1
                    && self.vial == 0
                    && self.hp == 40
                    && self.hp_max == 40
                    && self.poison_line
                    && !self.drink_line
            }
            #[cfg(feature = "js")]
            Case::HansClue | Case::HansHardClue => self.clue == 1 && !self.replacement,
        }
    }
}

#[derive(Default, Debug)]
struct Outcome {
    combat: Option<CombatReport>,
    loot_taken: bool,
    walk: Option<WalkReceipt>,
    assessment: Option<Arc<script::combat::risk::RouteAssessment>>,
    error: Option<String>,
}

struct FightScript {
    request: Option<CombatRequest>,
    tables: Arc<CombatTables>,
    fight: Option<ActionHandle<Combat>>,
    loot: Option<ActionHandle<Reach>>,
    loot_args: Option<ReachArgs>,
    outcome: Arc<Mutex<Outcome>>,
}

impl Script for FightScript {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        let result = (|| -> Result<(), script::native::ActionError> {
            if let Some(request) = self.request.take() {
                self.fight = Some(tick.actions.begin::<Combat>(
                    (Arc::new(request), Arc::clone(&self.tables)),
                    &mut tick.cx,
                )?);
            }
            if let Some(handle) = &self.fight {
                if let Poll::Ready(report) = tick.actions.poll(handle, &mut tick.cx) {
                    let report = report?;
                    self.outcome.lock().unwrap().combat = Some(report);
                    self.fight = None;
                    if matches!(report.end, CombatEnd::Killed) {
                        self.loot = Some(
                            tick.actions
                                .begin::<Reach>(self.loot_args.take().unwrap(), &mut tick.cx)?,
                        );
                    }
                }
            }
            if let Some(handle) = &self.loot {
                if let Poll::Ready(taken) = tick.actions.poll(handle, &mut tick.cx) {
                    self.outcome.lock().unwrap().loot_taken = taken?;
                    self.loot = None;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.outcome.lock().unwrap().error = Some(format!("native combat/loot: {error:?}"));
        }
        Ok(ScriptFlow::Continue)
    }
}

struct SurvivalScript {
    started: bool,
    advisory: bool,
    advisory_id: Option<u64>,
    walk: Option<ActionHandle<Walk>>,
    outcome: Arc<Mutex<Outcome>>,
}

impl Script for SurvivalScript {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        let result = (|| -> Result<(), script::native::ActionError> {
            if !self.started {
                let request = WalkRequest {
                    target: WorldTile {
                        x: 3218,
                        z: 3245,
                        level: 0,
                    },
                    candidate_goals: None,
                    loc_id: None,
                    radius: 0,
                    arrival: nav_planner::arrival::ArrivalKind::Area,
                    options: Default::default(),
                    required_after: tick.cx.evidence(),
                    evidence: None,
                    cross: Box::default(),
                    protect: false,
                    food_guard: true,
                    allow: script::native::WalkAllow {
                        prayer: false,
                        food: true,
                        escape: false,
                    },
                };
                if self.advisory {
                    let Some(id) = self.advisory_id else {
                        self.advisory_id = Some(tick.actions.assess_walk(request, &mut tick.cx)?);
                        return Ok(());
                    };
                    let Some(receipt) = tick.actions.take_assess_receipt(&mut tick.cx, id) else {
                        return Ok(());
                    };
                    if receipt.refusal.is_some() {
                        self.outcome.lock().unwrap().error =
                            Some(format!("route admission refused: {receipt:?}"));
                        return Ok(());
                    }
                    self.outcome.lock().unwrap().assessment = receipt.assessment;
                }
                self.started = true;
                self.walk = Some(tick.actions.begin::<Walk>(request, &mut tick.cx)?);
            }
            if let Some(handle) = &self.walk {
                if let Poll::Ready(receipt) = tick.actions.poll(handle, &mut tick.cx) {
                    self.outcome.lock().unwrap().walk = Some(receipt?);
                    self.walk = None;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.outcome.lock().unwrap().error = Some(format!("native Survival walk: {error:?}"));
        }
        Ok(ScriptFlow::Continue)
    }
}

struct LiveState {
    case: Case,
    facts: Facts,
    snapshot: GameSnapshot,
    pump: Pump,
    commands: Vec<String>,
    next_command: usize,
    last_command_tick: Option<u32>,
    relog_requested: bool,
    detached: bool,
    relogged: bool,
    ready_tick: Option<u32>,
    baseline: Option<Sample>,
    latest: Option<Sample>,
    last_sample_tick: Option<u32>,
    capture: Option<&'static str>,
    captured: bool,
    witness: Option<Value>,
    directory: PathBuf,
    writer: Arc<EvidenceWriter>,
    jobs: Vec<EvidenceJob>,
    error: Option<String>,
}

impl LiveState {
    fn save_frame(
        &mut self,
        client: &mut Client,
        phase: &str,
        sample: &Sample,
    ) -> Result<(), String> {
        let mut renderer = client::render::Renderer::new_prefer(client.config.lowmem, false);
        let was_draw = client.draw;
        client.set_draw(true);
        let output = renderer.mainredraw(client);
        client.set_draw(was_draw);
        let client::render::backend::FrameOutput::PixMap(pixels) = output else {
            return Err("377 machine frame was not a CPU PixMap".into());
        };
        let mut receipt = json!({"case":self.case.name(), "phase":phase, "revision":377,
            "sample":sample, "baseline":self.baseline, "witness":self.witness,
            "renderer":"real Client CpuPix3D framebuffer", "fixture_commands":self.commands,
            "fixture_commands_after_start":0, "scene_entities_seeded":0, "proof_products_seeded":0});
        host_play::evidence_writer::record_source_identity(&mut receipt);
        println!("{receipt}");
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
            self.detached = true;
        }
        if hold
            || !self.snapshot.ingame()
            || !self.snapshot.attached()
            || self.snapshot.scene_state() != 2
        {
            return Ok(());
        }
        let sample = Sample::read(&self.snapshot, &self.facts);
        self.latest = Some(sample.clone());
        if !self.relogged {
            if self.detached
                && self
                    .snapshot
                    .side_tabs()
                    .iter()
                    .any(|tab| tab.index == 3 && tab.available)
            {
                self.relogged = true;
                self.ready_tick = Some(sample.tick);
            } else if !self.relog_requested
                && sample.tick >= 3
                && sample
                    .tile
                    .is_some_and(|(x, z, level)| lumbridge(WorldTile { x, z, level }))
            {
                let ifaces = Arc::clone(&client.ifaces);
                if !interact::logout(client, &ifaces) {
                    return Err("377 fixture logout unavailable".into());
                }
                self.relog_requested = true;
            }
            return Ok(());
        }
        // Establish the poison owner's clean-prefix observation before poisoning.
        if matches!(self.case, Case::Eat | Case::Cure)
            && self
                .ready_tick
                .is_none_or(|tick| sample.tick.saturating_sub(tick) < 35)
        {
            return Ok(());
        }
        if let Some(phase) = self.capture.take() {
            self.save_frame(client, phase, &sample)?;
            self.captured = true;
            return Ok(());
        }
        if self.baseline.is_none() {
            if let Some(command) = self.commands.get(self.next_command) {
                if self.last_command_tick != Some(sample.tick) {
                    if !interact::cheat(client, command).is_sent() {
                        return Err(format!("fixture refused: {command}"));
                    }
                    println!(
                        "{}",
                        json!({"phase":"fixture", "case":self.case.name(), "tick":sample.tick, "command":command})
                    );
                    self.last_command_tick = Some(sample.tick);
                    self.next_command += 1;
                }
                return Ok(());
            }
            if self.last_command_tick != Some(sample.tick) && sample.prepared(self.case) {
                self.baseline = Some(sample.clone());
                self.save_frame(client, "prepared", &sample)?;
            }
        }
        if self.baseline.is_some() && self.last_sample_tick != Some(sample.tick) {
            println!(
                "{}",
                json!({"phase":"machine-sample", "case":self.case.name(), "sample":sample})
            );
            self.last_sample_tick = Some(sample.tick);
        }
        Ok(())
    }
}

fn proved(case: Case, baseline: &Sample, sample: &Sample, outcome: &Outcome, immune: bool) -> bool {
    if !baseline.prepared(case) || sample.tick <= baseline.tick {
        return false;
    }
    match case {
        Case::Cow => {
            outcome.combat.is_some_and(|report| {
                matches!(report.end, CombatEnd::Killed)
                    && report
                        .engaged
                        .is_some_and(|actor| actor.kind == script::combat::ActorKind::Npc)
                    && report.engaged_npc_type > 0
                    && report.swings > 0
            }) && sample.combat_xp > baseline.combat_xp
                && outcome.loot_taken
                && sample.loot > baseline.loot
        }
        Case::Eat => {
            sample.food < baseline.food
                && sample.hp > baseline.hp
                && sample.hp <= sample.hp_max
                && sample.tile != baseline.tile
        }
        Case::Cure => {
            sample.cure == 0
                && sample.vial == 1
                && sample.drink_line
                && !baseline.drink_line
                && immune
                && sample.tile != baseline.tile
        }
        Case::Admission => {
            outcome.walk.as_ref().is_some_and(|receipt| {
                receipt.refusal.is_none() && receipt.end == script::native::WalkEnd::Arrived
            }) && outcome.assessment.as_ref().is_some_and(|assessment| {
                !assessment.input.missing_facts
                    && assessment.input.food_iter().any(|(_, count)| count == 5)
            }) && sample.tile == Some((3218, 3245, 0))
                && sample.food == baseline.food
        }
        #[cfg(feature = "js")]
        Case::HansClue | Case::HansHardClue => {
            sample.clue == 0 && sample.replacement && !baseline.replacement
        }
    }
}

fn run(case: Case) -> Result<(), String> {
    if std::env::var("BOT_LIVE_NAME_PREFIX").as_deref() != Ok("f377") {
        return Err("machine qualification requires BOT_LIVE_NAME_PREFIX=f377".into());
    }
    #[cfg(feature = "js")]
    script_js::install_cards().map_err(|error| error.to_string())?;
    let bound = load_bound_377()?;
    identity_record(&bound);
    let data = bound
        .profile
        .game_data()
        .ok_or("377 selected facts unavailable")?;
    let facts = Facts::selected(&data, case)?;
    let directory = required_path("MACHINES_377_EVIDENCE_DIR")?.join(case.name());
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let account = host_play::mint_live_names(1).remove(0);
    let writer = Arc::new(EvidenceWriter::new(4));
    let outcome = Arc::new(Mutex::new(Outcome::default()));
    let tables = CombatTables::build(Arc::clone(&data))
        .map_err(|error| format!("377 combat tables: {error:?}"))?;
    let request = CombatRequest {
        target: Target::Npc {
            types: Arc::from([facts.cow]),
            pick: Pick::Nearest,
            not_targeting_others: true,
        },
        melee_mode: Some(MeleeMode::Accurate),
        kit: Some(Arc::new(CompiledKit {
            worn: Arc::from([(3, facts.weapon)]),
            carry: Arc::from([]),
        })),
        budget_ticks: 200,
        allow: Allowances {
            prayer: false,
            potions: false,
            ..Default::default()
        },
        ..Default::default()
    };
    let loot_args = ReachArgs {
        kind: ReachKind::Ground {
            id: facts.loot,
            obj: Arc::clone(&facts.loot_name),
        },
        op: Arc::from("Take"),
        anchor: None,
        radius: 4,
        wait_if_missing: true,
        target_tile: None,
        reachable_only: true,
    };
    let state = Arc::new(Mutex::new(LiveState {
        case,
        facts,
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        commands: case.commands(),
        next_command: 0,
        last_command_tick: None,
        relog_requested: false,
        detached: false,
        relogged: false,
        ready_tick: None,
        baseline: None,
        latest: None,
        last_sample_tick: None,
        capture: None,
        captured: false,
        witness: None,
        directory,
        writer: Arc::clone(&writer),
        jobs: vec![],
        error: None,
    }));
    let frame_state = Arc::clone(&state);
    let frame_account = account.clone();
    let mut play = host_play::run_with_template(
        Arc::clone(&bound.template),
        true,
        vec![],
        |_| (None, None),
        move |client, name, frame| {
            if name == frame_account {
                let mut state = frame_state.lock().unwrap();
                if state.error.is_none() {
                    if let Err(error) = state.frame(client, frame.hold) {
                        state.error = Some(error);
                    }
                }
            }
        },
    )?;
    let mut settings = ProfileSettings::default();
    if matches!(case, Case::Eat) {
        settings.survival.cure_poison = vault::CurePoison::Never;
    }
    play.try_spawn_slot(
        Profile {
            username: account.clone(),
            password: host_play::profile_password(&account).into(),
            uid: fresh_uid(30),
            settings,
        },
        None,
        None,
        None,
    )?;
    println!(
        "{}",
        json!({"phase":"machine-identity", "account":account, "case":case.name()})
    );
    let preparation_deadline = Instant::now() + Duration::from_secs(180);
    let mut machine_deadline = None;
    let mut started = false;
    let mut script: Option<Box<dyn Script>> = match case {
        Case::Cow => Some(Box::new(FightScript {
            request: Some(request),
            tables,
            fight: None,
            loot: None,
            loot_args: Some(loot_args),
            outcome: Arc::clone(&outcome),
        })),
        Case::Eat | Case::Cure | Case::Admission => Some(Box::new(SurvivalScript {
            started: false,
            advisory: matches!(case, Case::Admission),
            advisory_id: None,
            walk: None,
            outcome: Arc::clone(&outcome),
        })),
        #[cfg(feature = "js")]
        Case::HansClue | Case::HansHardClue => None,
    };
    let result = (|| loop {
        if Path::new("/Volumes/dev-scratch/274bot-evidence/MAC-QUIET.lock").exists() {
            return Err("MAC-QUIET.lock appeared".into());
        }
        if let Some(error) = play.script_last_error(&account) {
            return Err(format!("machine lifecycle: {error}"));
        }
        let condition = play
            .statuses()
            .into_iter()
            .find(|row| row.username == account)
            .map(|row| row.condition);
        let mut state = state.lock().map_err(|_| "machine state poisoned")?;
        if let Some(error) = &state.error {
            return Err(error.clone());
        }
        if !started && state.baseline.is_some() {
            if matches!(case, Case::Admission) {
                let world = play.world().ok_or("377 admission world unavailable")?;
                let input = host_play::admission::capture(
                    &state.snapshot,
                    true,
                    condition.as_ref(),
                    false,
                    Some(&world),
                );
                if input.missing_facts
                    || !input
                        .food_iter()
                        .any(|(id, count)| id == state.facts.food && count == 5)
                {
                    return Err(format!(
                        "377 route admission did not retain selected food: {input:?}"
                    ));
                }
                println!(
                    "{}",
                    json!({"phase":"route-admission-input", "account":account,
                    "food":input.food_iter().collect::<Vec<_>>(), "missing_facts":input.missing_facts,
                    "hp":input.hp})
                );
            }
            drop(state);
            if let Some(script) = script.take() {
                play.script_start_handle().start_test_script(
                    &account,
                    script,
                    Some(Arc::clone(&data)),
                )?;
                play.wake(&account);
            } else {
                play.script_start(&account, script::CompiledId("Sherlock"), Default::default())?;
            }
            println!(
                "{}",
                json!({"phase":"machine-start", "case":case.name(), "account":account})
            );
            machine_deadline = Some(Instant::now() + Duration::from_secs(240));
            started = true;
            continue;
        }
        let outcome = outcome.lock().map_err(|_| "machine outcome poisoned")?;
        if let Some(error) = &outcome.error {
            return Err(error.clone());
        }
        if let (Some(baseline), Some(sample)) = (&state.baseline, &state.latest) {
            let immune = condition.is_some_and(|row| {
                row.tick == sample.tick
                    && matches!(row.poison, api::condition::PoisonState::Immune { .. })
            });
            if started && proved(case, baseline, sample, &outcome, immune) && !state.captured {
                let witness = json!({"phase":"machine-witness", "case":case.name(), "account":account,
                    "baseline":baseline, "latest":sample, "combat":outcome.combat,
                    "advisory":outcome.assessment.as_ref().map(|assessment| json!({
                        "food":assessment.input.food_iter().collect::<Vec<_>>(),
                        "missing_facts":assessment.input.missing_facts,
                        "verdict":format!("{:?}",assessment.verdict)})),
                    "loot_taken":outcome.loot_taken, "walk":format!("{:?}",outcome.walk), "condition":format!("{condition:?}")});
                println!("{witness}");
                state.witness = Some(witness);
                state.capture = Some("result");
            }
        }
        if state.captured {
            println!(
                "{}",
                json!({"phase":"machine-pass", "case":case.name(), "account":account})
            );
            return Ok(());
        }
        if (!started && Instant::now() >= preparation_deadline)
            || machine_deadline.is_some_and(|deadline| Instant::now() >= deadline)
        {
            state.capture = Some("failed");
            return Err(format!("377 {} deadline: started={started}, commands={}, baseline={:?}, latest={:?}, outcome={outcome:?}, condition={condition:?}", case.name(),state.next_command,state.baseline,state.latest));
        }
        drop(outcome);
        drop(state);
        std::thread::sleep(Duration::from_millis(20));
    })();
    if let Err(error) = &result {
        {
            let mut state = state.lock().map_err(|_| "machine state poisoned")?;
            state.capture = Some("failed");
            state.captured = false;
            state.witness = Some(json!({"outcome":"FAIL", "error":error}));
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline
            && !state.lock().map_err(|_| "machine state poisoned")?.captured
        {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    play.script_stop(&account);
    play.stop_slot(&account);
    drop(play);
    let jobs = state
        .lock()
        .map_err(|_| "machine state poisoned")?
        .jobs
        .clone();
    for job in jobs {
        let result = writer
            .wait(&job, Duration::from_secs(30))
            .ok_or("machine capture deadline")?;
        if let Some(error) = result.error {
            return Err(error);
        }
    }
    result
}

#[test]
#[ignore = "requires canonical 377 resources, LIVE=1, f377 prefix and MACHINES_377_EVIDENCE_DIR"]
fn machines_377_cow_with_food_and_loot() -> Result<(), String> {
    run(Case::Cow)
}
#[test]
#[ignore = "requires canonical 377 resources, LIVE=1, f377 prefix and MACHINES_377_EVIDENCE_DIR"]
fn machines_377_survival_eats_on_poison_line() -> Result<(), String> {
    run(Case::Eat)
}
#[test]
#[ignore = "requires canonical 377 resources, LIVE=1, f377 prefix and MACHINES_377_EVIDENCE_DIR"]
fn machines_377_route_admission_with_selected_food() -> Result<(), String> {
    run(Case::Admission)
}
#[test]
#[ignore = "requires canonical 377 resources, LIVE=1, f377 prefix and MACHINES_377_EVIDENCE_DIR"]
fn machines_377_survival_cures_poison() -> Result<(), String> {
    run(Case::Cure)
}
#[cfg(feature = "js")]
#[test]
#[ignore = "requires canonical 377 resources, JS, LIVE=1, f377 prefix and MACHINES_377_EVIDENCE_DIR"]
fn machines_377_clue_hans() -> Result<(), String> {
    run(Case::HansClue)
}
#[cfg(feature = "js")]
#[test]
#[ignore = "requires canonical 377 resources, JS, LIVE=1, f377 prefix and MACHINES_377_EVIDENCE_DIR"]
fn machines_377_clue_hans_hard() -> Result<(), String> {
    run(Case::HansHardClue)
}

#[test]
fn selected_377_operands_and_machine_oracles_do_not_accept_fixture_only_state() {
    let data = api::game_data::for_revision(ClientRevision::R377).unwrap();
    let facts = Facts::selected(&data, Case::Cow).unwrap();
    assert_eq!(facts.cow, 81);
    assert_eq!(facts.food, 379);
    assert_eq!(facts.cure, 179);
    assert_eq!(facts.loot, 1739);
    let commands = Case::Eat.commands();
    assert_eq!(&commands[commands.len() - 2..], ["~hit 12", "~poison 50"]);
    let tables = CombatTables::build(Arc::clone(&data)).unwrap();
    assert_eq!(tables.selected().revision(), 377);
    assert_eq!(tables.food(facts.food).unwrap().heal, 12);
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    let sample = Sample::read(&snapshot, &facts);
    for case in [Case::Cow, Case::Eat, Case::Cure] {
        assert!(!sample.prepared(case));
        assert!(!proved(case, &sample, &sample, &Outcome::default(), false));
    }
    assert!(Facts::selected(
        &api::game_data::for_revision(ClientRevision::R289).unwrap(),
        Case::Cow
    )
    .is_err());
}

fn oracle_baseline(case: Case) -> Sample {
    let at = case.start();
    Sample {
        tick: 1,
        tile: Some((at.x, at.z, at.level)),
        hp: 40,
        hp_max: 40,
        food: 5,
        weapon: 1,
        ..Default::default()
    }
}

#[test]
fn admission_witness_requires_advisory_food_and_successful_native_arrival() {
    let baseline = oracle_baseline(Case::Admission);
    let mut sample = baseline.clone();
    sample.tick += 1;
    sample.tile = Some((3218, 3245, 0));
    let mut input = host_play::admission::RiskInput {
        missing_facts: false,
        food_len: 1,
        ..Default::default()
    };
    input.food_ids[0] = Facts::selected(
        &api::game_data::for_revision(ClientRevision::R377).unwrap(),
        Case::Admission,
    )
    .unwrap()
    .food;
    input.food_counts[0] = 5;
    let mut outcome = Outcome {
        walk: Some(WalkReceipt {
            request_id: 1,
            evidence: api::quest_progress::EvidenceStamp {
                run: api::selected::RunKey {
                    slot: 1,
                    run: 1,
                    session: 1,
                },
                tick: 2,
                sequence: 2,
            },
            goal: None,
            end: script::native::WalkEnd::Arrived,
            blocked: None,
            detail: None,
            refusal: None,
            assessment: None,
            escape: None,
        }),
        ..Default::default()
    };
    assert!(!proved(
        Case::Admission,
        &baseline,
        &sample,
        &outcome,
        false
    ));
    outcome.assessment = Some(Arc::new(script::combat::risk::RouteAssessment {
        verdict: script::combat::risk::Verdict::Survivable,
        plan: Default::default(),
        crossings: Box::default(),
        more: 0,
        supplies: Box::default(),
        hp_after: 40,
        volley: 0,
        input,
        generation: 1,
        reason: Arc::from("known advisory input"),
    }));
    assert!(proved(Case::Admission, &baseline, &sample, &outcome, false));
    Arc::get_mut(outcome.assessment.as_mut().unwrap())
        .unwrap()
        .input
        .missing_facts = true;
    assert!(!proved(
        Case::Admission,
        &baseline,
        &sample,
        &outcome,
        false
    ));
    Arc::get_mut(outcome.assessment.as_mut().unwrap())
        .unwrap()
        .input
        .missing_facts = false;
    outcome.walk.as_mut().unwrap().end = script::native::WalkEnd::NoPath;
    assert!(!proved(
        Case::Admission,
        &baseline,
        &sample,
        &outcome,
        false
    ));
}

#[test]
fn cow_witness_requires_kill_swing_xp_and_native_loot() {
    let baseline = oracle_baseline(Case::Cow);
    assert!(baseline.prepared(Case::Cow));
    let mut sample = baseline.clone();
    sample.tick += 1;
    sample.combat_xp += 32;
    sample.loot = 1;
    let report = CombatReport {
        end: CombatEnd::Killed,
        evidence: api::quest_progress::EvidenceStamp {
            run: api::selected::RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 2,
            sequence: 2,
        },
        engaged: Some(script::combat::ActorRef {
            kind: script::combat::ActorKind::Npc,
            index: 1,
        }),
        engaged_npc_type: 81,
        ticks: 8,
        swings: 2,
        casts: 0,
        damage_taken: 0,
        food: 0,
        prayer_doses: 0,
        boost_doses: 0,
        antifire_doses: 0,
        hits_while_protected: 0,
        protect_switches: 0,
        intruders: 0,
        ammo_pickups: 0,
        restorations: 0,
        locked_ticks: 0,
        multi_op_plans: 0,
        melee_mode_fallback: None,
        flick_resets: 0,
        flick_misses: 0,
        flick_fallback: false,
    };
    let mut outcome = Outcome {
        combat: Some(report),
        loot_taken: true,
        ..Default::default()
    };
    assert!(proved(Case::Cow, &baseline, &sample, &outcome, false));
    outcome.combat.as_mut().unwrap().end = CombatEnd::TargetGone;
    assert!(!proved(Case::Cow, &baseline, &sample, &outcome, false));
    outcome.combat = Some(report);
    outcome.combat.as_mut().unwrap().swings = 0;
    assert!(!proved(Case::Cow, &baseline, &sample, &outcome, false));
    outcome.combat = Some(report);
    outcome.loot_taken = false;
    assert!(!proved(Case::Cow, &baseline, &sample, &outcome, false));
    outcome.loot_taken = true;
    sample.combat_xp = baseline.combat_xp;
    assert!(!proved(Case::Cow, &baseline, &sample, &outcome, false));
    sample.combat_xp += 32;
    sample.loot = 0;
    assert!(!proved(Case::Cow, &baseline, &sample, &outcome, false));
}

#[test]
fn survival_food_witness_requires_movement_consumption_and_healing_after_start() {
    let mut baseline = oracle_baseline(Case::Eat);
    baseline.hp = 3;
    baseline.hp_max = 15;
    baseline.poison_line = true;
    assert!(baseline.prepared(Case::Eat));
    let mut sample = baseline.clone();
    sample.tick += 1;
    sample.tile.as_mut().unwrap().0 += 1;
    sample.food = 4;
    sample.hp = 15;
    assert!(proved(
        Case::Eat,
        &baseline,
        &sample,
        &Outcome::default(),
        false
    ));
    sample.food = 5;
    assert!(!proved(
        Case::Eat,
        &baseline,
        &sample,
        &Outcome::default(),
        false
    ));
    sample.food = 4;
    sample.hp = 3;
    assert!(!proved(
        Case::Eat,
        &baseline,
        &sample,
        &Outcome::default(),
        false
    ));
    sample.hp = 15;
    sample.tile = baseline.tile;
    assert!(!proved(
        Case::Eat,
        &baseline,
        &sample,
        &Outcome::default(),
        false
    ));
    assert!(!proved(
        Case::Eat,
        &sample,
        &sample,
        &Outcome::default(),
        false
    ));
}

#[test]
fn survival_cure_witness_requires_chat_inventory_and_owner_immunity() {
    let mut baseline = oracle_baseline(Case::Cure);
    baseline.cure = 1;
    baseline.poison_line = true;
    assert!(baseline.prepared(Case::Cure));
    let mut sample = baseline.clone();
    sample.tick += 1;
    sample.tile.as_mut().unwrap().0 += 1;
    sample.cure = 0;
    sample.vial = 1;
    sample.drink_line = true;
    assert!(proved(
        Case::Cure,
        &baseline,
        &sample,
        &Outcome::default(),
        true
    ));
    assert!(!proved(
        Case::Cure,
        &baseline,
        &sample,
        &Outcome::default(),
        false
    ));
    sample.drink_line = false;
    assert!(!proved(
        Case::Cure,
        &baseline,
        &sample,
        &Outcome::default(),
        true
    ));
    sample.drink_line = true;
    sample.cure = 1;
    assert!(!proved(
        Case::Cure,
        &baseline,
        &sample,
        &Outcome::default(),
        true
    ));
    sample.cure = 0;
    sample.vial = 0;
    assert!(!proved(
        Case::Cure,
        &baseline,
        &sample,
        &Outcome::default(),
        true
    ));
}

#[cfg(feature = "js")]
#[test]
fn selected_hans_clue_witnesses_require_real_handoffs() {
    script_js::install_cards().expect("install the production Sherlock card");
    assert!(script::compiled_card(script::CompiledId("Sherlock")).is_some());
    let data = api::game_data::for_revision(ClientRevision::R377).unwrap();
    for case in [Case::HansClue, Case::HansHardClue] {
        let facts = Facts::selected(&data, case).unwrap();
        let row = data
            .talk_key()
            .unwrap()
            .talk
            .iter()
            .find(|row| Some(row.id) == facts.clue)
            .unwrap();
        assert_eq!(row.npc.alias, "hans");
        assert_eq!(row.npc.id, 0);
        let mut baseline = oracle_baseline(case);
        baseline.clue = 1;
        assert!(baseline.prepared(case));
        let mut sample = baseline.clone();
        sample.tick += 1;
        sample.clue = 0;
        sample.replacement = true;
        assert!(proved(case, &baseline, &sample, &Outcome::default(), false));
        sample.clue = 1;
        assert!(!proved(
            case,
            &baseline,
            &sample,
            &Outcome::default(),
            false
        ));
        sample.clue = 0;
        sample.replacement = false;
        assert!(!proved(
            case,
            &baseline,
            &sample,
            &Outcome::default(),
            false
        ));
    }
}
