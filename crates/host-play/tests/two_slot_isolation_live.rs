//! Isolated two-slot lifecycle proof through shared production Play.
//!
//! Two minted actors start the same frozen Alcher catalog card with distinct
//! supported settings. Pause/Resume/Stop go through Play controls. This is
//! component proof preparation, not catalog or frontend acceptance. Root owns
//! LIVE launches.

#[path = "support/two_slot_isolation.rs"]
mod two_slot_isolation;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact::{self, Interactions, SendResult};
use api::snapshot::GameSnapshot;
use host::Pump;
use host_play::{ProfileOptions, ScriptStartHandle, SharedClientTemplate};
use serde_json::json;
use vault::{Profile, ProfileSettings};

use two_slot_isolation::{
    alcher_row, catalog_ledger, hash_file, near, prepare_alcher, slot_settings,
    verify_registry_identity, IsolationWitness, Observation, PreparedCard, RecordedState, SlotKind,
    SlotRecord, FODDER_COUNT, MAGIC_LEVEL, NATURE_COUNT, VARROCK_WEST,
};

const PREP_DEADLINE: Duration = Duration::from_secs(180);
const PROGRESS_DEADLINE: Duration = Duration::from_secs(90);
const DRAIN_DEADLINE: Duration = Duration::from_secs(12);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prep {
    WaitIngame,
    TutSkip,
    WaitTutorial,
    Relog,
    WaitLoggedOut,
    WaitRelog,
    Seed,
    WaitSeed,
    DrainDialogs,
    AckBank,
    WaitAck,
    CloseBank,
    WaitClosed,
    Ready,
}

struct SlotLive {
    kind: SlotKind,
    account: String,
    expected_player: String,
    settings: serde_json::Map<String, serde_json::Value>,
    snapshot: GameSnapshot,
    pump: Pump,
    start_handle: Option<ScriptStartHandle>,
    js: String,
    shape: script::LoadShape,
    siblings: Vec<(String, String)>,
    prep: Prep,
    started: bool,
    baseline: Option<Observation>,
    latest: Option<Observation>,
    record: Option<SlotRecord>,
    start_error: Option<String>,
    last_action: Instant,
}

impl SlotLive {
    fn publish(&mut self, client: &client::client::Client) -> Observation {
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);
        Observation::from_snapshot(&self.snapshot, self.kind)
    }

    fn chat_has(&self, needle: &str) -> bool {
        self.snapshot
            .chat_lines()
            .iter()
            .any(|line| line.text.to_ascii_lowercase().contains(needle))
            || self
                .snapshot
                .chat_modal_texts()
                .iter()
                .any(|text| text.to_ascii_lowercase().contains(needle))
    }

    fn capture_baseline(&mut self, observation: &Observation) -> Result<(), String> {
        if !observation.ingame || observation.scene_state != 2 {
            return Err(format!(
                "Start baseline is not attached ingame scene2: {observation:?}"
            ));
        }
        let player = observation
            .player
            .as_deref()
            .ok_or_else(|| "Start baseline has no local player".to_string())?;
        if !player.eq_ignore_ascii_case(&self.expected_player) {
            return Err(format!(
                "Start baseline player {player:?} is not fresh account {:?}",
                self.account
            ));
        }
        if observation.magic < MAGIC_LEVEL {
            return Err(format!(
                "Start baseline magic {} is below required {MAGIC_LEVEL}",
                observation.magic
            ));
        }
        if !near(observation.tile, VARROCK_WEST, 6) {
            return Err(format!(
                "Start baseline is not at Varrock West: {:?}",
                observation.tile
            ));
        }
        if observation.coins > 0
            || observation.natures > 0
            || observation.own_inv() > 0
            || observation.foreign_inv() > 0
        {
            return Err(
                "Start baseline already has alch outcome or selected fodder in inventory".into(),
            );
        }
        self.baseline = Some(observation.clone());
        self.record = Some(SlotRecord::new(
            self.kind,
            self.account.clone(),
            self.settings.clone(),
            observation.clone(),
        ));
        println!(
            "{}",
            json!({
                "phase": "baseline-after-preparation",
                "account": self.account,
                "kind": self.kind,
                "settings": self.settings,
                "observation": observation,
            })
        );
        Ok(())
    }

    fn start_script(&mut self) -> Result<(), String> {
        let Some(handle) = self.start_handle.as_ref() else {
            return Err("Start reached before ScriptStartHandle install".into());
        };
        handle.start_load(
            &self.account,
            self.js.clone(),
            self.shape,
            Some(self.settings.clone()),
            self.siblings.clone(),
        )?;
        self.started = true;
        println!(
            "{}",
            json!({
                "phase": "start",
                "account": self.account,
                "kind": self.kind,
                "settings": self.settings,
            })
        );
        Ok(())
    }

    fn frame(&mut self, client: &mut client::client::Client, hold: bool) {
        let observation = self.publish(client);
        self.latest = Some(observation.clone());
        if self.started {
            if let Some(record) = self.record.as_mut() {
                record.observe(observation, &self.expected_player);
            }
            return;
        }
        if let Err(error) = self.advance_prep(client, &observation, hold) {
            self.start_error = Some(error);
        }
    }

    fn send_ok(hold: bool) -> bool {
        !hold
    }

    fn advance_prep(
        &mut self,
        client: &mut client::client::Client,
        observation: &Observation,
        hold: bool,
    ) -> Result<(), String> {
        let now = Instant::now();
        match self.prep {
            Prep::WaitIngame => {
                if observation.ingame && observation.scene_state == 2 {
                    self.prep = Prep::TutSkip;
                }
            }
            Prep::TutSkip => {
                if !Self::send_ok(hold) {
                    return Ok(());
                }
                interact::cheat(client, "setvar tutorial 1000");
                interact::cheat(client, "getvar tutorial");
                self.last_action = now;
                self.prep = Prep::WaitTutorial;
            }
            Prep::WaitTutorial => {
                if self.chat_has("get tutorial: 1000") {
                    self.prep = Prep::Relog;
                }
            }
            Prep::Relog => {
                if !Self::send_ok(hold) {
                    return Ok(());
                }
                let ifaces = Arc::clone(&client.ifaces);
                if !interact::logout(client, &ifaces) {
                    return Err("logout iface missing (side icons still tutorial-locked?)".into());
                }
                self.last_action = now;
                self.prep = Prep::WaitLoggedOut;
            }
            Prep::WaitLoggedOut => {
                if !observation.ingame {
                    self.prep = Prep::WaitRelog;
                }
            }
            Prep::WaitRelog => {
                if observation.ingame && observation.scene_state == 2 {
                    self.prep = Prep::Seed;
                }
            }
            Prep::Seed => {
                if !Self::send_ok(hold) {
                    return Ok(());
                }
                interact::cheat(client, &format!("setstat magic {MAGIC_LEVEL}"));
                interact::cheat(
                    client,
                    &format!("givebank {} {FODDER_COUNT}", self.kind.seed_obj()),
                );
                interact::cheat(client, &format!("givebank naturerune {NATURE_COUNT}"));
                interact::cheat(client, "givebank staff_of_fire 1");
                interact::cheat(
                    client,
                    &interact::tele_args(VARROCK_WEST.2, VARROCK_WEST.0, VARROCK_WEST.1),
                );
                self.last_action = now;
                self.prep = Prep::WaitSeed;
            }
            Prep::WaitSeed => {
                if observation.ingame
                    && observation.scene_state == 2
                    && observation.magic >= MAGIC_LEVEL
                    && near(observation.tile, VARROCK_WEST, 6)
                {
                    self.prep = Prep::DrainDialogs;
                }
            }
            Prep::DrainDialogs => {
                let modals = self.snapshot.modals();
                if modals.main == -1 && modals.chat == -1 {
                    self.prep = Prep::AckBank;
                } else if Self::send_ok(hold) {
                    let _ = interact::close_modal(client);
                }
            }
            Prep::AckBank => {
                if observation.bank_open && observation.bank_loaded {
                    self.prep = Prep::WaitAck;
                    return Ok(());
                }
                if !Self::send_ok(hold) {
                    return Ok(());
                }
                if now.duration_since(self.last_action) >= Duration::from_millis(400) {
                    match Interactions::new(&self.snapshot, client).open_nearest_booth() {
                        SendResult::Sent { .. } | SendResult::Refused { .. } => {
                            self.last_action = now;
                            self.prep = Prep::WaitAck;
                        }
                    }
                }
            }
            Prep::WaitAck => {
                if observation.bank_open
                    && observation.bank_loaded
                    && observation.bank_own >= 1
                    && observation.bank_natures >= 1
                    && observation.bank_staff >= 1
                    && observation.bank_foreign == 0
                {
                    println!(
                        "{}",
                        json!({
                            "phase": "acknowledged-bank",
                            "account": self.account,
                            "kind": self.kind,
                            "observation": observation,
                        })
                    );
                    self.prep = Prep::CloseBank;
                } else if !observation.bank_open
                    && Self::send_ok(hold)
                    && now.duration_since(self.last_action) >= Duration::from_millis(400)
                {
                    let _ = Interactions::new(&self.snapshot, client).open_nearest_booth();
                    self.last_action = now;
                }
            }
            Prep::CloseBank => {
                if !Self::send_ok(hold) {
                    return Ok(());
                }
                let _ = interact::close_modal(client);
                self.last_action = now;
                self.prep = Prep::WaitClosed;
            }
            Prep::WaitClosed => {
                if !observation.bank_open {
                    self.prep = Prep::Ready;
                } else if Self::send_ok(hold)
                    && now.duration_since(self.last_action) > Duration::from_millis(400)
                {
                    let _ = interact::close_modal(client);
                    self.last_action = now;
                }
            }
            Prep::Ready => {
                if self.baseline.is_none() {
                    self.capture_baseline(observation)?;
                }
                self.start_script()?;
            }
        }
        Ok(())
    }
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Result<Self, String> {
        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("clock: {error}"))?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "274bot-two-slot-isolation-{}-{serial}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path)
            .map_err(|error| format!("create {}: {error}", path.display()))?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required"))
}

fn selected_profile(
    revision: u16,
    nav_pack: PathBuf,
    catalog_root: PathBuf,
    temp: &Path,
) -> Result<(Arc<host_play::ServerProfile>, Arc<SharedClientTemplate>), String> {
    let options = ProfileOptions {
        profile: Some(format!("local-{revision}")),
        revision: Some(revision.to_string()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(if revision == 289 { 44594 } else { 43594 }),
        http_port: Some(if revision == 289 { 1080 } else { 80 }),
        nav_pack: Some(nav_pack),
        nav_flags: std::env::var_os("TWO_SLOT_NAV_FLAGS").map(PathBuf::from),
        engine_dir: std::env::var_os("TWO_SLOT_ENGINE_DIR").map(PathBuf::from),
        vault_path: Some(temp.join("vault")),
        catalog_root: Some(catalog_root),
        ..ProfileOptions::default()
    };
    let profile = options.resolve(None)?.bind()?;
    if profile.target() != client::BotTarget::Local || profile.client().game_host() != "127.0.0.1" {
        return Err("two-slot isolation proof requires a loopback-only local profile".into());
    }
    let template = SharedClientTemplate::load(Arc::clone(&profile))?;
    if template.world().is_none() {
        return Err("selected template has no navigation world".into());
    }
    Ok((profile, template))
}

fn mint_profile(account: &str, password: &str, offset: u32) -> Result<Profile, String> {
    Ok(Profile {
        username: account.to_string(),
        password: password.to_string(),
        uid: (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("clock: {error}"))?
            .as_millis()
            % i32::MAX as u128) as i32
            + offset as i32,
        settings: ProfileSettings::default(),
    })
}

fn new_slot(kind: SlotKind, account: String, card: &PreparedCard) -> Result<SlotLive, String> {
    let expected_player = client::util::jstring::JString::to_screen_name(&account);
    let settings = slot_settings(&card.schema, kind);
    if settings
        == slot_settings(
            &card.schema,
            match kind {
                SlotKind::Chainbody => SlotKind::Scimitar,
                SlotKind::Scimitar => SlotKind::Chainbody,
            },
        )
    {
        return Err("slot settings collapsed to the peer bag".into());
    }
    Ok(SlotLive {
        kind,
        account,
        expected_player,
        settings,
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        start_handle: None,
        js: card.js.clone(),
        shape: card.shape,
        siblings: card.siblings.clone(),
        prep: Prep::WaitIngame,
        started: false,
        baseline: None,
        latest: None,
        record: None,
        start_error: None,
        last_action: Instant::now(),
    })
}

fn store_latest(slot: &mut SlotLive, assign: impl FnOnce(&mut SlotRecord, Observation)) -> bool {
    let Some(observation) = slot.latest.clone() else {
        return false;
    };
    let Some(record) = slot.record.as_mut() else {
        return false;
    };
    assign(record, observation);
    true
}

#[derive(Debug, Clone, Copy)]
enum Phase {
    Prep,
    FirstProgress,
    Drain,
    PauseProve,
    Further,
    AfterStop,
}

struct SlotSnapshot {
    a_started: bool,
    b_started: bool,
    a_prep: Prep,
    b_prep: Prep,
    a_first: Option<Observation>,
    b_first: Option<Observation>,
    a_drain: Option<Observation>,
    b_drain: Option<Observation>,
    a_pause_end: Option<Observation>,
    b_pause_end: Option<Observation>,
    a_further: Option<Observation>,
    a_latest: Option<Observation>,
    b_latest: Option<Observation>,
}

fn run_cell() -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Ok(());
    }
    let _home = script::IsolatedEnv::enter("two-slot-isolation-live-home");
    let revision = required("TWO_SLOT_REVISION")?
        .parse::<u16>()
        .map_err(|_| "TWO_SLOT_REVISION must be 274 or 289".to_string())?;
    if !matches!(revision, 274 | 289) {
        return Err("TWO_SLOT_REVISION must be 274 or 289".into());
    }
    let nav_pack = PathBuf::from(required("TWO_SLOT_NAV_PACK")?);
    let root = PathBuf::from(required("TWO_SLOT_CATALOG_ROOT")?);
    if !root.is_dir() {
        return Err(format!(
            "TWO_SLOT_CATALOG_ROOT is not a directory: {}",
            root.display()
        ));
    }
    let commit = required("TWO_SLOT_CATALOG_COMMIT")?;
    let catalog = catalog_ledger(&commit)?;
    let row = alcher_row(&commit, revision)?;
    let registry_path = verify_registry_identity(&root, &catalog)?;
    let temp = TempRoot::new()?;
    let card = prepare_alcher(&root, temp.path(), &row)?;
    let (profile, template) = selected_profile(revision, nav_pack, root.clone(), temp.path())?;
    let names = host_play::mint_live_names(2);
    if names.len() != 2 || names[0].eq_ignore_ascii_case(&names[1]) {
        return Err("failed to mint two distinct live accounts".into());
    }
    let credentials = host_play::mint_live_entries_for_target(&names, profile.target());
    if credentials.len() != 2 {
        return Err("failed to mint two local credentials".into());
    }

    let slot_a = new_slot(SlotKind::Chainbody, names[0].clone(), &card)?;
    let slot_b = new_slot(SlotKind::Scimitar, names[1].clone(), &card)?;
    let state = Arc::new(Mutex::new((slot_a, slot_b)));
    let frame_state = Arc::clone(&state);
    let mut play = host_play::run_with_template(
        Arc::clone(&template),
        true,
        vec![],
        |_| (None, None),
        move |client, username, hold| {
            let mut pair = frame_state.lock().unwrap();
            if username == pair.0.account {
                pair.0.frame(client, hold);
            } else if username == pair.1.account {
                pair.1.frame(client, hold);
            }
        },
    )?;
    {
        let handle = play.script_start_handle();
        let mut pair = state.lock().unwrap();
        pair.0.start_handle = Some(handle.clone());
        pair.1.start_handle = Some(handle);
    }
    let nav_sha256 = hash_file(profile.nav_pack())?;
    let (settings_a, settings_b) = {
        let pair = state.lock().unwrap();
        (pair.0.settings.clone(), pair.1.settings.clone())
    };
    println!(
        "{}",
        json!({
            "phase": "identity",
            "revision": profile.revision().as_i32(),
            "profile": profile.label(),
            "cache_id": profile.cache_id(),
            "nav_pack": profile.nav_pack(),
            "nav_sha256": nav_sha256,
            "catalog_commit": commit,
            "catalog_identity": catalog.identity,
            "catalog_read_only_path": catalog.read_only_path,
            "catalog_root": root,
            "registry_path": registry_path,
            "registry_sha256": catalog.registry_sha256,
            "card": card.identity,
            "accounts": names,
            "settings_a": settings_a,
            "settings_b": settings_b,
        })
    );

    play.try_spawn_slot(
        mint_profile(&names[0], &credentials[0].1, 0)?,
        None,
        None,
        None,
    )?;
    play.try_spawn_slot(
        mint_profile(&names[1], &credentials[1].1, 1)?,
        None,
        None,
        None,
    )?;

    let mut phase = Phase::Prep;
    let mut deadline = Instant::now() + PREP_DEADLINE;
    let mut pause_a = None;
    let mut pause_b = None;
    let mut resume_a = None;
    let outcome = loop {
        if let Some(error) = play.script_last_error(&names[0]) {
            break Err(format!("script error on {}: {error}", names[0]));
        }
        if let Some(error) = play.script_last_error(&names[1]) {
            break Err(format!("script error on {}: {error}", names[1]));
        }

        let snapshot = {
            let mut pair = state.lock().unwrap();
            if let Some(error) = pair.0.start_error.take() {
                break Err(format!("slot {}: {error}", pair.0.account));
            }
            if let Some(error) = pair.1.start_error.take() {
                break Err(format!("slot {}: {error}", pair.1.account));
            }
            SlotSnapshot {
                a_started: pair.0.started,
                b_started: pair.1.started,
                a_prep: pair.0.prep,
                b_prep: pair.1.prep,
                a_first: pair
                    .0
                    .record
                    .as_ref()
                    .and_then(|record| record.first.clone()),
                b_first: pair
                    .1
                    .record
                    .as_ref()
                    .and_then(|record| record.first.clone()),
                a_drain: pair
                    .0
                    .record
                    .as_ref()
                    .and_then(|record| record.drain.clone()),
                b_drain: pair
                    .1
                    .record
                    .as_ref()
                    .and_then(|record| record.drain.clone()),
                a_pause_end: pair
                    .0
                    .record
                    .as_ref()
                    .and_then(|record| record.pause_end.clone()),
                b_pause_end: pair
                    .1
                    .record
                    .as_ref()
                    .and_then(|record| record.pause_end.clone()),
                a_further: pair
                    .0
                    .record
                    .as_ref()
                    .and_then(|record| record.further.clone()),
                a_latest: pair.0.latest.clone(),
                b_latest: pair.1.latest.clone(),
            }
        };
        let timed_out = Instant::now() >= deadline;
        match phase {
            Phase::Prep => {
                if snapshot.a_started && snapshot.b_started {
                    println!("{}", json!({"phase": "both-started"}));
                    phase = Phase::FirstProgress;
                    deadline = Instant::now() + PROGRESS_DEADLINE;
                } else if timed_out {
                    break Err(format!(
                        "preparation timeout; a_prep={:?} b_prep={:?} a_started={} b_started={}",
                        snapshot.a_prep, snapshot.b_prep, snapshot.a_started, snapshot.b_started
                    ));
                }
            }
            Phase::FirstProgress => {
                if snapshot.a_first.is_some() && snapshot.b_first.is_some() {
                    println!("{}", json!({"phase": "first-progress"}));
                    play.script_pause(&names[0]);
                    pause_a = Some(RecordedState::from(play.script_state(&names[0])));
                    pause_b = Some(RecordedState::from(play.script_state(&names[1])));
                    println!("{}", json!({"phase": "pause", "a": pause_a, "b": pause_b}));
                    phase = Phase::Drain;
                    deadline = Instant::now() + DRAIN_DEADLINE;
                } else if timed_out {
                    break Err(
                        "idle/non-progress: both scripts did not produce item/coin/XP deltas"
                            .into(),
                    );
                }
            }
            Phase::Drain => {
                if timed_out {
                    let mut pair = state.lock().unwrap();
                    let stored = store_latest(&mut pair.0, |record, observation| {
                        record.drain = Some(observation);
                    }) && store_latest(&mut pair.1, |record, observation| {
                        record.drain = Some(observation);
                    });
                    if !stored {
                        break Err("drain finished without Start baselines".into());
                    }
                    phase = Phase::PauseProve;
                    deadline = Instant::now() + PROGRESS_DEADLINE;
                }
            }
            Phase::PauseProve => {
                let state_a = RecordedState::from(play.script_state(&names[0]));
                let state_b = RecordedState::from(play.script_state(&names[1]));
                let isolated = match (
                    snapshot.a_drain.as_ref(),
                    snapshot.b_drain.as_ref(),
                    snapshot.a_latest.as_ref(),
                    snapshot.b_latest.as_ref(),
                ) {
                    (Some(a_drain), Some(b_drain), Some(a_now), Some(b_now)) => {
                        a_now.magic_xp <= a_drain.magic_xp
                            && a_now.coins <= a_drain.coins
                            && b_now.has_progress_over(b_drain)
                            && state_a == RecordedState::Paused
                            && state_b == RecordedState::Running
                    }
                    _ => false,
                };
                if isolated {
                    let mut pair = state.lock().unwrap();
                    let _ = store_latest(&mut pair.0, |record, observation| {
                        record.pause_end = Some(observation);
                    });
                    let _ = store_latest(&mut pair.1, |record, observation| {
                        record.pause_end = Some(observation);
                    });
                    drop(pair);
                    play.script_resume(&names[0]);
                    resume_a = Some(RecordedState::from(play.script_state(&names[0])));
                    pause_a = Some(state_a);
                    pause_b = Some(state_b);
                    println!("{}", json!({"phase": "resume", "a": resume_a}));
                    phase = Phase::Further;
                    deadline = Instant::now() + PROGRESS_DEADLINE;
                } else if timed_out {
                    break Err(format!(
                        "isolation failure after pause; a_state={state_a:?} b_state={state_b:?}"
                    ));
                }
            }
            Phase::Further => {
                let state_a = RecordedState::from(play.script_state(&names[0]));
                let progressed = match (snapshot.a_pause_end.as_ref(), snapshot.a_latest.as_ref()) {
                    (Some(paused), Some(now)) => {
                        now.has_progress_over(paused) && state_a == RecordedState::Running
                    }
                    _ => false,
                };
                if progressed {
                    let mut pair = state.lock().unwrap();
                    let _ = store_latest(&mut pair.0, |record, observation| {
                        record.further = Some(observation);
                    });
                    drop(pair);
                    play.script_stop(&names[0]);
                    resume_a = Some(state_a);
                    println!(
                        "{}",
                        json!({
                            "phase": "stop",
                            "a": RecordedState::from(play.script_state(&names[0])),
                            "b": RecordedState::from(play.script_state(&names[1])),
                        })
                    );
                    phase = Phase::AfterStop;
                    deadline = Instant::now() + PROGRESS_DEADLINE;
                } else if timed_out {
                    break Err("no-progress control: resumed slot made no further progress".into());
                }
            }
            Phase::AfterStop => {
                let state_a = RecordedState::from(play.script_state(&names[0]));
                let state_b = RecordedState::from(play.script_state(&names[1]));
                let done = match (
                    snapshot.b_pause_end.as_ref(),
                    snapshot.b_latest.as_ref(),
                    snapshot.a_further.as_ref(),
                    snapshot.a_latest.as_ref(),
                ) {
                    (Some(b_paused), Some(b_now), Some(a_further), Some(a_now)) => {
                        b_now.has_progress_over(b_paused)
                            && a_now.magic_xp <= a_further.magic_xp
                            && a_now.coins <= a_further.coins
                            && state_a == RecordedState::Idle
                            && state_b == RecordedState::Running
                    }
                    _ => false,
                };
                if done {
                    let mut pair = state.lock().unwrap();
                    let _ = store_latest(&mut pair.0, |record, observation| {
                        record.after_stop = Some(observation);
                    });
                    let _ = store_latest(&mut pair.1, |record, observation| {
                        record.after_stop = Some(observation);
                    });
                    let Some(record_a) = pair.0.record.clone() else {
                        break Err("missing slot A witness".into());
                    };
                    let Some(record_b) = pair.1.record.clone() else {
                        break Err("missing slot B witness".into());
                    };
                    drop(pair);
                    let mut witness = IsolationWitness::new(record_a, record_b);
                    witness.pause_a = pause_a;
                    witness.pause_b = pause_b;
                    witness.resume_a = resume_a;
                    witness.stop_a = Some(state_a);
                    witness.stop_b = Some(state_b);
                    break witness.qualify();
                } else if timed_out {
                    break Err(format!(
                        "isolation failure after stop; a_state={state_a:?} b_state={state_b:?}"
                    ));
                }
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    play.script_stop(&names[0]);
    play.script_stop(&names[1]);
    play.stop_slot(&names[0]);
    play.stop_slot(&names[1]);
    let outcome = outcome?;
    println!("PASS: two_slot_isolation_live: {outcome}");
    Ok(())
}

#[test]
#[ignore = "requires LIVE=1, TWO_SLOT_REVISION/NAV_PACK/CATALOG_ROOT/COMMIT, and local engine"]
fn two_slot_isolation_live() {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run_cell));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            eprintln!("FAIL: two_slot_isolation_live: {error}");
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("FAIL: two_slot_isolation_live: {error:?}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use two_slot_isolation::{
        IsolationWitness, Observation, RecordedState, SlotKind, SlotRecord,
        ADAMANT_SCIMITAR_ALCH_COINS, HIGH_ALCH_MAGIC_XP, RUNE_CHAINBODY_ALCH_COINS,
    };

    fn observation(_kind: SlotKind, player: &str) -> Observation {
        Observation {
            ingame: true,
            scene_state: 2,
            player: Some(player.into()),
            tile: Some(VARROCK_WEST),
            tick: 10,
            magic: MAGIC_LEVEL,
            magic_xp: 100,
            coins: 0,
            natures: 0,
            own_unnoted: 0,
            own_noted: 0,
            foreign_unnoted: 0,
            foreign_noted: 0,
            bank_open: false,
            bank_loaded: false,
            bank_own: 0,
            bank_foreign: 0,
            bank_natures: 0,
            bank_staff: 0,
        }
    }

    fn progressed(baseline: &Observation, kind: SlotKind) -> Observation {
        let mut obs = baseline.clone();
        obs.tick += 20;
        obs.magic_xp = baseline.magic_xp + HIGH_ALCH_MAGIC_XP;
        obs.coins = kind.alch_coins();
        obs.natures = 0;
        obs.own_noted = 0;
        obs
    }

    fn with_peaks(record: &mut SlotRecord, natures: i32, own: i32) {
        record.peak_natures = natures;
        record.peak_own = own;
    }

    fn seeded_pair() -> (SlotRecord, SlotRecord) {
        let a_base = observation(SlotKind::Chainbody, "alice");
        let b_base = observation(SlotKind::Scimitar, "bob");
        let mut a = SlotRecord::new(
            SlotKind::Chainbody,
            "alice".into(),
            SlotKind::Chainbody.settings(),
            a_base.clone(),
        );
        let mut b = SlotRecord::new(
            SlotKind::Scimitar,
            "bob".into(),
            SlotKind::Scimitar.settings(),
            b_base.clone(),
        );
        with_peaks(&mut a, 27, 27);
        with_peaks(&mut b, 27, 27);
        let mut a_first = progressed(&a_base, SlotKind::Chainbody);
        a_first.natures = 0;
        a.first = Some(a_first.clone());
        a.post_start = 4;
        let b_first = progressed(&b_base, SlotKind::Scimitar);
        b.first = Some(b_first.clone());
        b.post_start = 4;
        a.drain = Some(a_first.clone());
        b.drain = Some(b_first.clone());
        a.pause_end = Some(a_first);
        let mut b_pause = b_first.clone();
        b_pause.magic_xp += HIGH_ALCH_MAGIC_XP;
        b_pause.coins += ADAMANT_SCIMITAR_ALCH_COINS;
        b.pause_end = Some(b_pause.clone());
        let mut a_further = a.drain.clone().unwrap();
        a_further.magic_xp += HIGH_ALCH_MAGIC_XP;
        a_further.coins += RUNE_CHAINBODY_ALCH_COINS;
        a.further = Some(a_further.clone());
        a.after_stop = Some(a_further);
        let mut b_after = b_pause;
        b_after.magic_xp += HIGH_ALCH_MAGIC_XP;
        b_after.coins += ADAMANT_SCIMITAR_ALCH_COINS;
        b.after_stop = Some(b_after);
        (a, b)
    }

    fn complete(a: SlotRecord, b: SlotRecord) -> IsolationWitness {
        IsolationWitness {
            a,
            b,
            pause_a: Some(RecordedState::Paused),
            pause_b: Some(RecordedState::Running),
            resume_a: Some(RecordedState::Running),
            stop_a: Some(RecordedState::Idle),
            stop_b: Some(RecordedState::Running),
        }
    }

    #[test]
    fn rejects_mixed_identities() {
        let (mut a, b) = seeded_pair();
        a.account = "bob".into();
        let error = complete(a, b).qualify().unwrap_err();
        assert!(error.contains("mixed identities"), "{error}");
    }

    #[test]
    fn rejects_queued_only_success() {
        let (mut a, b) = seeded_pair();
        a.first = None;
        let error = complete(a, b).qualify().unwrap_err();
        assert!(error.contains("queued-only"), "{error}");
    }

    #[test]
    fn rejects_no_progress_while_peer_paused() {
        let (a, mut b) = seeded_pair();
        b.pause_end = b.drain.clone();
        let error = complete(a, b).qualify().unwrap_err();
        assert!(error.contains("no-progress"), "{error}");
    }

    #[test]
    fn rejects_paused_slot_that_keeps_progressing() {
        let (mut a, b) = seeded_pair();
        let mut pause = a.drain.clone().unwrap();
        pause.coins += RUNE_CHAINBODY_ALCH_COINS;
        pause.magic_xp += HIGH_ALCH_MAGIC_XP;
        a.pause_end = Some(pause);
        let error = complete(a, b).qualify().unwrap_err();
        assert!(error.contains("pause isolation"), "{error}");
    }

    #[test]
    fn rejects_cross_slot_item_contamination() {
        let (mut a, b) = seeded_pair();
        a.foreign_seen = true;
        let error = complete(a, b).qualify().unwrap_err();
        assert!(error.contains("contamination"), "{error}");
    }

    #[test]
    fn accepts_pause_resume_stop_isolation() {
        let (a, b) = seeded_pair();
        let core = complete(a, b).qualify().expect("isolated pair should pass");
        assert_eq!(core["a"]["kind"], json!("chainbody"));
        assert_eq!(core["b"]["kind"], json!("scimitar"));
        assert_eq!(core["stop_a"], json!("idle"));
        assert_eq!(core["stop_b"], json!("running"));
    }
}
