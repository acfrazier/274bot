//! Real-resource Gatherer bank trips through the production 377 template.
//! Fixtures change account state only, before Start. No scene or yield seeding.

use super::*;
use host_play::evidence_writer::{
    EvidenceJob, EvidenceRequest, EvidenceSidecar, EvidenceWriter, PngColor,
};
use script::native::{NativePhase, ScriptStatus, StatusValue};
use std::sync::Mutex;
use vault::{Profile, ProfileSettings};

#[derive(Clone, Copy)]
enum Case {
    Trees,
    Shrimp,
}

impl Case {
    fn name(self) -> &'static str {
        match self {
            Self::Trees => "lumbridge-normal-trees",
            Self::Shrimp => "draynor-net-shrimp",
        }
    }

    fn site(self) -> &'static str {
        match self {
            Self::Trees => "woodcutting.lumbridge",
            Self::Shrimp => "fishing.draynor_village",
        }
    }

    fn skill(self) -> &'static str {
        match self {
            Self::Trees => "woodcutting",
            Self::Shrimp => "fishing",
        }
    }

    fn tool(self) -> (i32, &'static str) {
        match self {
            Self::Trees => (1351, "bronze_axe"),
            Self::Shrimp => (303, "net"),
        }
    }

    fn products(self) -> &'static [i32] {
        match self {
            Self::Trees => &[1511],
            Self::Shrimp => &[317, 321],
        }
    }

    fn start(self) -> WorldTile {
        match self {
            Self::Trees => WorldTile {
                x: 3190,
                z: 3245,
                level: 0,
            },
            Self::Shrimp => WorldTile {
                x: 3087,
                z: 3228,
                level: 0,
            },
        }
    }

    fn settings(self) -> serde_json::Map<String, Value> {
        json!({
            "skill": match self { Self::Trees => "Woodcutting", Self::Shrimp => "Fishing" },
            "woodcuttingResources": ["normal"],
            "fishingMethod": "fishing.net.tool_303.products_317_321",
            "location": "Site",
            "site": self.site(),
            "disposition": "Bank",
            "bank": "Draynor",
            "allowTeleports": false,
            "allowWilderness": false,
            "allowDangerZones": false,
        })
        .as_object()
        .unwrap()
        .clone()
    }

    fn commands(self) -> Vec<String> {
        let tile = self.start();
        let mut commands = vec![
            "~clearinv".into(),
            "~clearinv worn".into(),
            "~clearbank".into(),
            format!("setstat {} 40", self.skill()),
        ];
        if matches!(self, Self::Shrimp) {
            // Fresh HP 10 and Defence 60 produce combat 18, above the
            // selected Draynor level-7 wizard zone's combat cap of 14.
            commands.push("setstat defence 60".into());
        }
        commands.extend([
            format!("give {} 1", self.tool().1),
            format!(
                "tele {},{},{},{},{}",
                tile.level,
                tile.x >> 6,
                tile.z >> 6,
                tile.x & 63,
                tile.z & 63
            ),
        ]);
        commands
    }

    fn combat_prepared(self, combat_level: i32) -> bool {
        !matches!(self, Self::Shrimp) || combat_level == 18
    }
}

#[derive(Clone, Debug, serde::Serialize)]
struct Sample {
    tick: u32,
    tile: Option<(i32, i32, i32)>,
    xp: i32,
    level: i32,
    combat_level: i32,
    tool_in_inventory: i32,
    tool_equipped: bool,
    inventory_slots: usize,
    held_products: i32,
    bank_loaded: bool,
    bank_products: i32,
    bank_primary_product: i32,
}

impl Sample {
    fn read(case: Case, snapshot: &GameSnapshot) -> Self {
        let count = |items: &[api::snapshot::ItemView], ids: &[i32]| {
            items
                .iter()
                .filter(|item| ids.contains(&item.def.id))
                .map(|item| item.count)
                .sum()
        };
        let stat = snapshot
            .stats()
            .iter()
            .find(|stat| stat.name.eq_ignore_ascii_case(case.skill()));
        Self {
            tick: snapshot.tick(),
            tile: snapshot.tile(),
            xp: stat.map_or(0, |stat| stat.xp),
            level: stat.map_or(0, |stat| stat.base),
            combat_level: snapshot
                .local_player()
                .map_or(0, |player| player.player.combat_level),
            tool_in_inventory: count(snapshot.inventory(), &[case.tool().0]),
            tool_equipped: snapshot
                .equipment()
                .iter()
                .any(|item| item.def.id == case.tool().0),
            inventory_slots: snapshot.inventory().len(),
            held_products: count(snapshot.inventory(), case.products()),
            bank_loaded: snapshot.bank_loaded(),
            bank_products: count(snapshot.bank(), case.products()),
            bank_primary_product: count(snapshot.bank(), &case.products()[..1]),
        }
    }

    fn at_resource(&self, case: Case) -> bool {
        self.tile.is_some_and(|(x, z, level)| {
            let start = case.start();
            level == start.level && (x - start.x).abs().max((z - start.z).abs()) <= 12
        })
    }

    fn in_region(&self, region: &api::gather_methods::SceneRegionInput) -> bool {
        self.tile.is_some_and(|(x, z, level)| {
            level == region.level
                && (region.min_x - 1..=region.max_x + 1).contains(&x)
                && (region.min_z - 1..=region.max_z + 1).contains(&z)
        })
    }
}

#[derive(Default)]
struct Witness {
    saw_empty_bank: bool,
    banked: i32,
    primary_banked: i32,
    deposited_xp: Option<i32>,
}

impl Witness {
    fn observe(&mut self, sample: &Sample) {
        if !sample.bank_loaded {
            return;
        }
        if sample.bank_products == 0 {
            self.saw_empty_bank = true;
        }
        if self.saw_empty_bank && sample.bank_products > self.banked {
            self.banked = sample.bank_products;
            self.primary_banked = sample.bank_primary_product;
            self.deposited_xp = Some(sample.xp);
        }
    }

    fn returned(
        &self,
        region: &api::gather_methods::SceneRegionInput,
        baseline: &Sample,
        latest: &Sample,
        trips: i64,
    ) -> bool {
        trips > 0
            && self.saw_empty_bank
            && self.banked >= 27
            && self.primary_banked > 0
            && self
                .deposited_xp
                .is_some_and(|xp| latest.xp > xp && xp > baseline.xp)
            && latest.in_region(region)
            && latest.held_products > 0
            && (latest.tool_in_inventory == 1 || latest.tool_equipped)
    }
}

struct LiveState {
    case: Case,
    snapshot: GameSnapshot,
    pump: Pump,
    commands: Vec<String>,
    next_command: usize,
    last_command_tick: Option<u32>,
    relog_requested: bool,
    relog_detached: bool,
    relogged: bool,
    preparation_observed: bool,
    baseline: Option<Sample>,
    latest: Option<Sample>,
    witness: Witness,
    directory: PathBuf,
    writer: Arc<EvidenceWriter>,
    jobs: Vec<EvidenceJob>,
    bank_capture: bool,
    capture_return: bool,
    return_captured: bool,
    error: Option<String>,
}

impl LiveState {
    fn capture(&mut self, client: &mut Client, phase: &str, sample: &Sample) -> Result<(), String> {
        let mut renderer = client::render::Renderer::new_prefer(client.config.lowmem, false);
        let was_draw = client.draw;
        client.set_draw(true);
        let output = renderer.mainredraw(client);
        client.set_draw(was_draw);
        let client::render::backend::FrameOutput::PixMap(pixels) = output else {
            return Err("377 Gatherer capture did not return a CPU PixMap".into());
        };
        let receipt = json!({
            "case": self.case.name(), "phase": phase, "revision": 377,
            "sample": sample, "baseline": self.baseline,
            "renderer": "real Client CpuPix3D framebuffer",
            "fixture_commands": self.commands,
            "fixture_commands_after_start": 0,
            "scene_entities_seeded": 0, "proof_products_seeded": 0,
        });
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
        let sample = Sample::read(self.case, &self.snapshot);
        self.latest = Some(sample.clone());
        if !self.relogged {
            let inventory_ready = self
                .snapshot
                .side_tabs()
                .iter()
                .any(|tab| tab.index == 3 && tab.available);
            if self.relog_detached && inventory_ready {
                self.relogged = true;
                println!("{}", json!({"phase":"fixture-relogged", "sample":sample}));
            } else {
                // mainland_hop deliberately leaves tutorial side icons locked.
                // The normal logout/login is required before tools are observable.
                if !self.relog_requested
                    && sample.tick >= 3
                    && sample
                        .tile
                        .is_some_and(|(x, z, level)| lumbridge(WorldTile { x, z, level }))
                {
                    let ifaces = Arc::clone(&client.ifaces);
                    if !interact::logout(client, &ifaces) {
                        return Err("377 fixture logout interface unavailable".into());
                    }
                    self.relog_requested = true;
                    println!(
                        "{}",
                        json!({"phase":"fixture-relog-request", "sample":sample})
                    );
                }
                return Ok(());
            }
        }
        if self.baseline.is_none() {
            if self.next_command < self.commands.len() {
                if self.last_command_tick != Some(sample.tick) {
                    let command = &self.commands[self.next_command];
                    if !interact::cheat(client, command).is_sent() {
                        return Err(format!("fixture command refused: {command}"));
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
            if !self.preparation_observed && self.last_command_tick != Some(sample.tick) {
                println!("{}", json!({"phase":"fixture-observed", "sample":sample}));
                self.preparation_observed = true;
            }
            if self.last_command_tick == Some(sample.tick)
                || sample.level != 40
                || !self.case.combat_prepared(sample.combat_level)
                || !sample.at_resource(self.case)
                || sample.tool_in_inventory != 1
                || sample.inventory_slots != 1
                || sample.held_products != 0
                || sample.tool_equipped
            {
                return Ok(());
            }
            self.baseline = Some(sample.clone());
            self.capture(client, "prepared", &sample)?;
        } else {
            self.witness.observe(&sample);
            if self.witness.banked >= 27 && !self.bank_capture {
                self.capture(client, "banked", &sample)?;
                self.bank_capture = true;
            }
            if self.capture_return && !self.return_captured {
                self.capture(client, "returned", &sample)?;
                self.return_captured = true;
            }
        }
        Ok(())
    }
}

fn integer(status: &ScriptStatus, key: &str) -> i64 {
    status
        .fields
        .iter()
        .find_map(|field| match &field.value {
            StatusValue::Integer(value) if field.key == key => Some(*value),
            _ => None,
        })
        .unwrap_or(0)
}

fn run(case: Case) -> Result<(), String> {
    let bound = load_bound_377()?;
    resource_record(&bound);
    let region = bound
        .profile
        .game_data()
        .and_then(|data| {
            data.gather_site(case.skill(), case.site())
                .map(|site| site.region)
        })
        .ok_or("selected 377 gathering site unavailable")?;
    let directory = required_path("GATHERER_377_EVIDENCE_DIR")?.join(case.name());
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let account = host_play::mint_live_names(1).remove(0);
    let writer = Arc::new(EvidenceWriter::new(4));
    let state = Arc::new(Mutex::new(LiveState {
        case,
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        commands: case.commands(),
        next_command: 0,
        last_command_tick: None,
        relog_requested: false,
        relog_detached: false,
        relogged: false,
        preparation_observed: false,
        baseline: None,
        latest: None,
        witness: Witness::default(),
        directory,
        writer: Arc::clone(&writer),
        jobs: Vec::new(),
        bank_capture: false,
        capture_return: false,
        return_captured: false,
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
    let profile = Profile {
        username: account.clone(),
        password: host_play::profile_password(&account).into(),
        uid: fresh_uid(match case {
            Case::Trees => 20,
            Case::Shrimp => 21,
        }),
        settings: ProfileSettings::default(),
    };
    play.try_spawn_slot(profile, None, None, None)?;
    println!(
        "{}",
        json!({"phase":"gatherer-identity", "account":account, "case":case.name(), "settings":case.settings()})
    );
    let deadline = Instant::now() + Duration::from_secs(900);
    let preparation_deadline = Instant::now() + Duration::from_secs(180);
    let mut started = false;
    let mut relogin_requested = false;
    let mut last_status = None;
    let result = (|| loop {
        if Path::new("/Volumes/dev-scratch/274bot-evidence/MAC-QUIET.lock").exists() {
            return Err("MAC-QUIET.lock appeared".into());
        }
        if let Some(error) = play.script_last_error(&account) {
            return Err(format!("377 Gatherer lifecycle: {error}"));
        }
        let status = play.script_native_status(&account);
        let mut state = state.lock().map_err(|_| "377 Gatherer state poisoned")?;
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
            drop(state);
            play.script_start(&account, script::CompiledId("Gatherer"), case.settings())?;
            started = true;
            continue;
        }
        if let Some(status) = status {
            let progress = (
                integer(&status, "yielded"),
                integer(&status, "deposited"),
                integer(&status, "trips"),
                status.phase,
            );
            if last_status != Some(progress) {
                println!(
                    "{}",
                    json!({"phase":"gatherer-progress", "account":account,
                        "yielded":progress.0, "deposited":progress.1, "trips":progress.2,
                        "native_phase":format!("{:?}",status.phase), "fields":format!("{:?}",status.fields), "sample":state.latest})
                );
                last_status = Some(progress);
            }
            if status.phase == NativePhase::Blocked || status.failure.is_some() {
                return Err(format!("377 Gatherer refused: {status:?}"));
            }
            if let (Some(baseline), Some(latest)) = (&state.baseline, &state.latest) {
                if state
                    .witness
                    .returned(&region, baseline, latest, progress.2)
                    && progress.1 >= 27
                {
                    state.capture_return = true;
                }
            }
            if state.return_captured {
                println!(
                    "{}",
                    json!({"phase":"gatherer-pass", "account":account, "case":case.name(),
                        "banked":state.witness.banked, "primary_banked":state.witness.primary_banked,
                        "trips":progress.2, "baseline":state.baseline, "latest":state.latest,
                        "saw_empty_bank_before_deposit":state.witness.saw_empty_bank})
                );
                return Ok(());
            }
        }
        if Instant::now() >= deadline || (!started && Instant::now() >= preparation_deadline) {
            return Err(format!(
                "377 Gatherer timed out: started={started} commands={} baseline={:?} latest={:?}",
                state.next_command, state.baseline, state.latest
            ));
        }
        drop(state);
        std::thread::sleep(Duration::from_millis(20));
    })();
    play.script_stop(&account);
    play.stop_slot(&account);
    drop(play);
    let jobs = state
        .lock()
        .map_err(|_| "377 Gatherer state poisoned")?
        .jobs
        .clone();
    for job in jobs {
        let outcome = writer
            .wait(&job, Duration::from_secs(30))
            .ok_or("377 capture timed out")?;
        if let Some(error) = outcome.error {
            return Err(error);
        }
    }
    result
}

#[test]
#[ignore = "requires LIVE=1, owned account prefix, canonical 377 inputs and GATHERER_377_EVIDENCE_DIR"]
fn gatherer_377_lumbridge_normal_trees_bank_and_return() -> Result<(), String> {
    run(Case::Trees)
}

#[test]
#[ignore = "requires LIVE=1, owned account prefix, canonical 377 inputs and GATHERER_377_EVIDENCE_DIR"]
fn gatherer_377_draynor_shrimp_bank_and_return() -> Result<(), String> {
    run(Case::Shrimp)
}

#[test]
fn bank_witness_requires_an_empty_live_page_and_fresh_yield_after_return() {
    let baseline = Sample {
        tick: 1,
        tile: Some((3087, 3228, 0)),
        xp: 1000,
        level: 40,
        combat_level: 18,
        tool_in_inventory: 1,
        tool_equipped: false,
        inventory_slots: 1,
        held_products: 0,
        bank_loaded: false,
        bank_products: 0,
        bank_primary_product: 0,
    };
    let region = api::gather_methods::SceneRegionInput {
        min_x: 3085,
        min_z: 3227,
        max_x: 3086,
        max_z: 3230,
        level: 0,
    };
    let mut bank = Sample {
        tick: 2,
        xp: 2000,
        bank_loaded: true,
        bank_products: 27,
        bank_primary_product: 20,
        ..baseline.clone()
    };
    let mut witness = Witness::default();
    witness.observe(&bank);
    assert_eq!(
        witness.banked, 0,
        "an existing bank stack is not a deposit proof"
    );
    bank.bank_products = 0;
    bank.bank_primary_product = 0;
    witness.observe(&bank);
    bank.bank_products = 27;
    bank.bank_primary_product = 20;
    witness.observe(&bank);
    let mut returned = Sample {
        tick: 3,
        xp: 2100,
        held_products: 1,
        ..baseline.clone()
    };
    assert!(!witness.returned(&region, &baseline, &returned, 0));
    assert!(witness.returned(&region, &baseline, &returned, 1));
    returned.xp = bank.xp;
    assert!(!witness.returned(&region, &baseline, &returned, 1));
    returned.xp = 2100;
    returned.tile = Some((3093, 3243, 0));
    assert!(!witness.returned(&region, &baseline, &returned, 1));
    let distant_site = api::gather_methods::SceneRegionInput {
        min_x: 3210,
        min_z: 3240,
        max_x: 3230,
        max_z: 3250,
        level: 0,
    };
    returned.tile = Some((3220, 3245, 0));
    assert!(witness.returned(&distant_site, &baseline, &returned, 1));
}

#[test]
fn fishing_fixture_requires_the_combat_appearance_acknowledgement() {
    for combat_level in [0, 3, 14, 17] {
        assert!(!Case::Shrimp.combat_prepared(combat_level));
    }
    assert!(Case::Shrimp.combat_prepared(18));
    assert!(Case::Trees.combat_prepared(3));
}
