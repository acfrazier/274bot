//! Paired NatureCrafter Air and Duel Arena fixtures through shared Play.
//!
//! Actual frozen catalog scripts. Root owns LIVE launches. This crate only
//! prepares the harness and offline witnesses.

#[path = "support/paired_catalog.rs"]
mod paired_catalog;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::game_data::{self, WEARPOS_RIGHTHAND};
use api::interact::{self, Interactions, SendResult};
use api::snapshot::GameSnapshot;
use client::io::ClientRevision;
use host::Pump;
use host_play::{ProfileOptions, ScriptStartHandle, SharedClientTemplate};
use serde_json::json;
use vault::{Profile, ProfileSettings};

use paired_catalog::{
    air_operation_gates, air_settings, card_row, catalog_ledger, duel_operation_gates,
    duel_settings, frozen_card_hashes_match, hash_file, near, prepare_card,
    verify_generated_duel_controls, verify_registry_identity, AirClaim, AirObservation,
    AirPairWitness, AirRole, AirSlotRecord, DuelClaim, DuelObservation, DuelPairWitness,
    DuelSlotRecord, GateKind, PairCase, PreparedCard, AIR_RUINS, CATALOG_COMMIT_A,
    CATALOG_COMMIT_B, DUEL_ARENA, DUEL_ARENA_LOGIC_SHA256, DUEL_ARENA_SHA256,
    DUEL_CHALLENGE_ANCHOR, DUEL_INTERFACE_SHA256, FALADOR_EAST, NATURECRAFTER,
    NATURECRAFTER_SHA256, NATURE_RUNNER_LOGIC_SHA256, PREP_DEADLINE_SECS,
    SCRIPT_GOLD_DEADLINE_SECS, SCRIPT_GOLD_WATCH_TICKS, TRADE_CAP,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prep {
    WaitIngame,
    TutSkip,
    WaitTutorial,
    Relog,
    WaitRelog,
    Seed,
    WaitSeed,
    DrainDialogs,
    AckBank,
    WaitAck,
    CloseBank,
    WaitClosed,
    Wear,
    WaitWear,
    Ready,
}

enum SlotKind {
    AirMaster,
    AirRunner,
    Duel,
}

struct SlotLive {
    kind: SlotKind,
    account: String,
    expected_player: String,
    partner: String,
    settings: serde_json::Map<String, serde_json::Value>,
    snapshot: GameSnapshot,
    pump: Pump,
    start_handle: Option<ScriptStartHandle>,
    js: String,
    shape: script::LoadShape,
    siblings: Vec<(String, String)>,
    prep: Prep,
    started: bool,
    start_error: Option<String>,
    last_action: Instant,
    air: Option<AirSlotRecord>,
    duel: Option<DuelSlotRecord>,
    latest_air: Option<AirObservation>,
    latest_duel: Option<DuelObservation>,
    weapon_id: i32,
}

impl SlotLive {
    fn publish_air(&mut self, client: &client::client::Client) -> AirObservation {
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);
        AirObservation::from_snapshot(&self.snapshot)
    }

    fn publish_duel(&mut self, client: &client::client::Client) -> DuelObservation {
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);
        DuelObservation::from_snapshot(&self.snapshot, &self.partner, self.weapon_id)
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
                "kind": match self.kind {
                    SlotKind::AirMaster => "air_master",
                    SlotKind::AirRunner => "air_runner",
                    SlotKind::Duel => "duel",
                },
                "settings": self.settings,
            })
        );
        Ok(())
    }

    fn frame_air(&mut self, client: &mut client::client::Client, hold: bool) {
        let observation = self.publish_air(client);
        self.latest_air = Some(observation.clone());
        if self.started {
            if let Some(record) = self.air.as_mut() {
                record.observe(observation);
            }
            return;
        }
        if let Err(error) = self.advance_prep(client, Some(&observation), None, hold) {
            self.start_error = Some(error);
        }
    }

    fn frame_duel(&mut self, client: &mut client::client::Client, hold: bool) {
        let observation = self.publish_duel(client);
        self.latest_duel = Some(observation.clone());
        if self.started {
            if let Some(record) = self.duel.as_mut() {
                record.observe(observation);
            }
            return;
        }
        if let Err(error) = self.advance_prep(client, None, Some(&observation), hold) {
            self.start_error = Some(error);
        }
    }

    fn send_ok(hold: bool) -> bool {
        !hold
    }

    fn capture_air_baseline(&mut self, observation: &AirObservation) -> Result<(), String> {
        if !observation.ingame || observation.scene_state != 2 {
            return Err(format!(
                "Start baseline is not attached ingame scene2: {observation:?}"
            ));
        }
        if !observation.inventory_tab_available {
            return Err("Start baseline inventory tab is not bound after relog".into());
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
        if !near(observation.tile, AIR_RUINS, 8) {
            return Err(format!(
                "Start baseline is not at Air ruins: {:?}",
                observation.tile
            ));
        }
        if observation.air_runes > 0 {
            return Err("Start baseline already has Air 556".into());
        }
        if observation.essence_noted > 0 {
            return Err("Start baseline has noted essence 1437; Air does not accept noting".into());
        }
        if observation.trade_active() {
            return Err("Start baseline already has a trade window".into());
        }
        match self.kind {
            SlotKind::AirMaster => {
                if observation.air_talisman <= 0 {
                    return Err("master baseline has no Air talisman".into());
                }
                if observation.essence_unnoted > 0 {
                    return Err("master baseline already holds unnoted essence".into());
                }
            }
            SlotKind::AirRunner => {
                if observation.essence_unnoted <= 0 {
                    return Err(
                        "runner baseline has no seeded unnoted 1436 for the first load".into(),
                    );
                }
            }
            SlotKind::Duel => {}
        }
        self.air = Some(AirSlotRecord::new(
            match self.kind {
                SlotKind::AirMaster => AirRole::Master,
                _ => AirRole::Runner,
            },
            self.account.clone(),
            self.expected_player.clone(),
            self.partner.clone(),
            self.settings.clone(),
            observation.clone(),
        ));
        println!(
            "{}",
            json!({
                "phase": "baseline-after-preparation",
                "account": self.account,
                "role": match self.kind {
                    SlotKind::AirMaster => "master",
                    _ => "runner",
                },
                "seeded_first_supplies": matches!(self.kind, SlotKind::AirRunner),
                "observation": observation,
            })
        );
        Ok(())
    }

    fn capture_duel_baseline(&mut self, observation: &DuelObservation) -> Result<(), String> {
        if !observation.ingame || observation.scene_state != 2 {
            return Err(format!(
                "Start baseline is not attached ingame scene2: {observation:?}"
            ));
        }
        if !observation.inventory_tab_available {
            return Err("Start baseline inventory tab is not bound after relog".into());
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
        if !observation.in_challenge_area {
            return Err(format!(
                "Start baseline is not in the Duel Arena challenge area: {:?}",
                observation.tile
            ));
        }
        if observation.duel_active() || observation.duel_win_open {
            return Err("seeded modal is not a duel: baseline already has a duel interface".into());
        }
        if !observation.weapon_equipped {
            return Err("Start baseline has no 1-handed melee weapon equipped".into());
        }
        self.duel = Some(DuelSlotRecord::new(
            self.account.clone(),
            self.expected_player.clone(),
            self.partner.clone(),
            self.settings.clone(),
            observation.clone(),
        ));
        println!(
            "{}",
            json!({
                "phase": "baseline-after-preparation",
                "account": self.account,
                "observation": observation,
            })
        );
        Ok(())
    }

    fn advance_prep(
        &mut self,
        client: &mut client::client::Client,
        air: Option<&AirObservation>,
        duel: Option<&DuelObservation>,
        hold: bool,
    ) -> Result<(), String> {
        let now = Instant::now();
        match self.prep {
            Prep::WaitIngame => {
                let ready = air.is_some_and(|o| o.ingame && o.scene_state == 2)
                    || duel.is_some_and(|o| o.ingame && o.scene_state == 2);
                if ready {
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
                println!(
                    "{}",
                    json!({
                        "phase": "before-relog",
                        "account": self.account,
                    })
                );
                let ifaces = Arc::clone(&client.ifaces);
                if !interact::logout(client, &ifaces) {
                    return Err("logout iface missing (side icons still tutorial-locked?)".into());
                }
                self.last_action = now;
                self.prep = Prep::WaitRelog;
            }
            Prep::WaitRelog => {
                let ready = air
                    .is_some_and(|o| o.ingame && o.scene_state == 2 && o.inventory_tab_available)
                    || duel.is_some_and(|o| {
                        o.ingame && o.scene_state == 2 && o.inventory_tab_available
                    });
                if ready {
                    println!(
                        "{}",
                        json!({
                            "phase": "after-relog",
                            "account": self.account,
                        })
                    );
                    self.prep = Prep::Seed;
                }
            }
            Prep::Seed => {
                if !Self::send_ok(hold) {
                    return Ok(());
                }
                match self.kind {
                    SlotKind::AirMaster => {
                        interact::cheat(client, "give air_talisman 1");
                        interact::cheat(
                            client,
                            &interact::tele_args(AIR_RUINS.2, AIR_RUINS.0, AIR_RUINS.1),
                        );
                    }
                    SlotKind::AirRunner => {
                        interact::cheat(client, "givebank blankrune 200");
                        interact::cheat(client, &format!("give blankrune {TRADE_CAP}"));
                        interact::cheat(
                            client,
                            &interact::tele_args(AIR_RUINS.2, AIR_RUINS.0, AIR_RUINS.1),
                        );
                    }
                    SlotKind::Duel => {
                        interact::cheat(client, "give bronze_scimitar 1");
                        interact::cheat(
                            client,
                            &interact::tele_args(
                                DUEL_CHALLENGE_ANCHOR.2,
                                DUEL_CHALLENGE_ANCHOR.0,
                                DUEL_CHALLENGE_ANCHOR.1,
                            ),
                        );
                    }
                }
                println!(
                    "{}",
                    json!({
                        "phase": "seed-mutations-acknowledged",
                        "account": self.account,
                        "kind": match self.kind {
                            SlotKind::AirMaster => "air_master_talisman_at_ruins",
                            SlotKind::AirRunner => "air_runner_seeded_first_load_and_bank_at_ruins",
                            SlotKind::Duel => "duel_bronze_scimitar_at_challenge_anchor",
                        },
                        "note": "seeded first supplies are distinct from later script-caused bank withdrawal/return/transfer",
                    })
                );
                self.last_action = now;
                self.prep = Prep::WaitSeed;
            }
            Prep::WaitSeed => {
                let ready = match self.kind {
                    SlotKind::AirMaster => air.is_some_and(|o| {
                        o.ingame
                            && o.scene_state == 2
                            && near(o.tile, AIR_RUINS, 8)
                            && o.air_talisman >= 1
                    }),
                    SlotKind::AirRunner => air.is_some_and(|o| {
                        o.ingame
                            && o.scene_state == 2
                            && near(o.tile, AIR_RUINS, 8)
                            && o.essence_unnoted >= 1
                    }),
                    SlotKind::Duel => duel.is_some_and(|o| {
                        o.ingame && o.scene_state == 2 && near(o.tile, DUEL_CHALLENGE_ANCHOR, 8)
                    }),
                };
                if ready {
                    self.prep = Prep::DrainDialogs;
                }
            }
            Prep::DrainDialogs => {
                let modals = self.snapshot.modals();
                if modals.main == -1 && modals.chat == -1 {
                    self.prep = match self.kind {
                        SlotKind::AirRunner => Prep::AckBank,
                        SlotKind::Duel => Prep::Wear,
                        SlotKind::AirMaster => Prep::Ready,
                    };
                } else if Self::send_ok(hold) {
                    let _ = interact::close_modal(client);
                }
            }
            Prep::AckBank => {
                let Some(observation) = air else {
                    return Ok(());
                };
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
                let Some(observation) = air else {
                    return Ok(());
                };
                if observation.bank_open
                    && observation.bank_loaded
                    && observation.bank_essence_unnoted >= 1
                {
                    println!(
                        "{}",
                        json!({
                            "phase": "acknowledged-bank",
                            "account": self.account,
                            "bank_essence_unnoted": observation.bank_essence_unnoted,
                            "stand": FALADOR_EAST,
                            "note": "givebank blankrune is seed, not the script restock",
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
                let Some(observation) = air else {
                    return Ok(());
                };
                if !observation.bank_open {
                    self.prep = Prep::Ready;
                } else if Self::send_ok(hold)
                    && now.duration_since(self.last_action) > Duration::from_millis(400)
                {
                    let _ = interact::close_modal(client);
                    self.last_action = now;
                }
            }
            Prep::Wear => {
                if !Self::send_ok(hold) {
                    return Ok(());
                }
                match Interactions::new(&self.snapshot, client).wear(self.weapon_id) {
                    SendResult::Sent { .. } | SendResult::Refused { .. } => {
                        self.last_action = now;
                        self.prep = Prep::WaitWear;
                    }
                }
            }
            Prep::WaitWear => {
                let Some(observation) = duel else {
                    return Ok(());
                };
                if observation.weapon_equipped {
                    self.prep = Prep::Ready;
                } else if Self::send_ok(hold)
                    && now.duration_since(self.last_action) >= Duration::from_millis(400)
                {
                    let _ = Interactions::new(&self.snapshot, client).wear(self.weapon_id);
                    self.last_action = now;
                }
            }
            Prep::Ready => {
                match self.kind {
                    SlotKind::AirMaster | SlotKind::AirRunner => {
                        if self.air.is_none() {
                            let Some(observation) = air else {
                                return Ok(());
                            };
                            self.capture_air_baseline(observation)?;
                        }
                    }
                    SlotKind::Duel => {
                        if self.duel.is_none() {
                            let Some(observation) = duel else {
                                return Ok(());
                            };
                            self.capture_duel_baseline(observation)?;
                        }
                    }
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
            "274bot-paired-catalog-{}-{serial}",
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
        nav_flags: std::env::var_os("PAIRED_CATALOG_NAV_FLAGS").map(PathBuf::from),
        engine_dir: std::env::var_os("PAIRED_CATALOG_ENGINE_DIR").map(PathBuf::from),
        vault_path: Some(temp.join("vault")),
        catalog_root: Some(catalog_root),
        ..ProfileOptions::default()
    };
    let profile = options.resolve(None)?.bind()?;
    if profile.target() != client::BotTarget::Local || profile.client().game_host() != "127.0.0.1" {
        return Err("paired catalog fixtures require a loopback-only local profile".into());
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

fn weapon_id(revision: u16) -> Result<i32, String> {
    let data = game_data::for_revision(match revision {
        289 => ClientRevision::R289,
        _ => ClientRevision::R274,
    })?;
    let item = data
        .item_by_alias("bronze_scimitar")
        .ok_or_else(|| "selected cache has no bronze_scimitar".to_string())?;
    if item.wear_position != WEARPOS_RIGHTHAND {
        return Err(format!(
            "bronze_scimitar wear_position {} is not right-hand {WEARPOS_RIGHTHAND}",
            item.wear_position
        ));
    }
    Ok(item.id)
}

fn new_slot(
    kind: SlotKind,
    account: String,
    partner: String,
    card: &PreparedCard,
    weapon_id: i32,
) -> Result<SlotLive, String> {
    let expected_player = client::util::jstring::JString::to_screen_name(&account);
    let settings = match kind {
        SlotKind::AirMaster => air_settings(&card.schema, AirRole::Master, &partner),
        SlotKind::AirRunner => air_settings(&card.schema, AirRole::Runner, &partner),
        SlotKind::Duel => duel_settings(&card.schema),
    };
    Ok(SlotLive {
        kind,
        account,
        expected_player,
        partner,
        settings,
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        start_handle: None,
        js: card.js.clone(),
        shape: card.shape,
        siblings: card.siblings.clone(),
        prep: Prep::WaitIngame,
        started: false,
        start_error: None,
        last_action: Instant::now(),
        air: None,
        duel: None,
        latest_air: None,
        latest_duel: None,
        weapon_id,
    })
}

fn run_cell(case: PairCase) -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Ok(());
    }
    let _home = script::IsolatedEnv::enter("paired-catalog-live-home");
    let revision = required("PAIRED_CATALOG_REVISION")?
        .parse::<u16>()
        .map_err(|_| "PAIRED_CATALOG_REVISION must be 274 or 289".to_string())?;
    if !matches!(revision, 274 | 289) {
        return Err("PAIRED_CATALOG_REVISION must be 274 or 289".into());
    }
    let nav_pack = PathBuf::from(required("PAIRED_CATALOG_NAV_PACK")?);
    let root = PathBuf::from(required("PAIRED_CATALOG_ROOT")?);
    if !root.is_dir() {
        return Err(format!(
            "PAIRED_CATALOG_ROOT is not a directory: {}",
            root.display()
        ));
    }
    let commit = required("PAIRED_CATALOG_COMMIT")?;
    let catalog = catalog_ledger(&commit)?;
    let row = card_row(&commit, revision, case.card_name())?;
    let registry_path = verify_registry_identity(&root, &catalog)?;
    let temp = TempRoot::new()?;
    let card = prepare_card(&root, temp.path(), &row, case.card_name())?;
    let weapon = weapon_id(revision)?;
    let data = game_data::for_revision(match revision {
        289 => ClientRevision::R289,
        _ => ClientRevision::R274,
    })?;
    if case == PairCase::Duel {
        verify_generated_duel_controls(&data)?;
    }
    let (profile, template) = selected_profile(revision, nav_pack, root.clone(), temp.path())?;
    let names = host_play::mint_live_names(2);
    if names.len() != 2 || names[0].eq_ignore_ascii_case(&names[1]) {
        return Err("failed to mint two distinct live accounts".into());
    }
    let credentials = host_play::mint_live_entries_for_target(&names, profile.target());
    if credentials.len() != 2 {
        return Err("failed to mint two local credentials".into());
    }
    let screen_a = client::util::jstring::JString::to_screen_name(&names[0]);
    let screen_b = client::util::jstring::JString::to_screen_name(&names[1]);
    let slot_a = match case {
        PairCase::Air => new_slot(
            SlotKind::AirMaster,
            names[0].clone(),
            screen_b.clone(),
            &card,
            weapon,
        )?,
        PairCase::Duel => new_slot(
            SlotKind::Duel,
            names[0].clone(),
            screen_b.clone(),
            &card,
            weapon,
        )?,
    };
    let slot_b = match case {
        PairCase::Air => new_slot(
            SlotKind::AirRunner,
            names[1].clone(),
            screen_a.clone(),
            &card,
            weapon,
        )?,
        PairCase::Duel => new_slot(
            SlotKind::Duel,
            names[1].clone(),
            screen_a.clone(),
            &card,
            weapon,
        )?,
    };
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
                match case {
                    PairCase::Air => pair.0.frame_air(client, hold),
                    PairCase::Duel => pair.0.frame_duel(client, hold),
                }
            } else if username == pair.1.account {
                match case {
                    PairCase::Air => pair.1.frame_air(client, hold),
                    PairCase::Duel => pair.1.frame_duel(client, hold),
                }
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
    println!(
        "{}",
        json!({
            "phase": "identity",
            "case": case,
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
            "gold_deadline_secs": SCRIPT_GOLD_DEADLINE_SECS,
            "gold_watch_ticks": SCRIPT_GOLD_WATCH_TICKS,
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

    let mut phase = 0_u8;
    let mut deadline = Instant::now() + Duration::from_secs(PREP_DEADLINE_SECS);
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
            (pair.0.started, pair.1.started, pair.0.prep, pair.1.prep)
        };
        let timed_out = Instant::now() >= deadline;
        if phase == 0 {
            if snapshot.0 && snapshot.1 {
                println!("{}", json!({"phase": "both-started"}));
                phase = 1;
                deadline = Instant::now() + Duration::from_secs(SCRIPT_GOLD_DEADLINE_SECS);
            } else if timed_out {
                break Err(format!(
                    "preparation timeout; a_prep={:?} b_prep={:?} a_started={} b_started={}",
                    snapshot.2, snapshot.3, snapshot.0, snapshot.1
                ));
            }
        } else {
            let pair = state.lock().unwrap();
            match case {
                PairCase::Air => {
                    if let (Some(master), Some(runner)) = (pair.0.air.clone(), pair.1.air.clone()) {
                        let witness = AirPairWitness { master, runner };
                        if witness.qualify_full_cycle().is_ok() {
                            break Ok(json!({
                                "claim": AirClaim::BankReturnSecondCycle,
                                "witness": witness,
                            }));
                        }
                        if timed_out {
                            match witness.qualify_supported() {
                                Ok(claim) => {
                                    break Ok(json!({
                                        "claim": claim,
                                        "full_cycle": false,
                                        "limit": "bank-return/second transfer/craft did not finish inside unchanged SCRIPT_GOLD_DEADLINE 180s",
                                        "witness": witness,
                                    }));
                                }
                                Err(error) => break Err(error),
                            }
                        }
                    } else if timed_out {
                        break Err("idle/non-progress: missing Air records after Start".into());
                    }
                }
                PairCase::Duel => {
                    if let (Some(a), Some(b)) = (pair.0.duel.clone(), pair.1.duel.clone()) {
                        let witness = DuelPairWitness { a, b };
                        if witness.qualify_full_cycle().is_ok() {
                            break Ok(json!({
                                "claim": DuelClaim::ResetAndFurther,
                                "witness": witness,
                            }));
                        }
                        if timed_out {
                            match witness.qualify_supported() {
                                Ok(claim) => {
                                    break Ok(json!({
                                        "claim": claim,
                                        "full_cycle": false,
                                        "limit": "duel end/reset and further combat did not finish inside unchanged SCRIPT_GOLD_DEADLINE 180s; challenge interval 5s and hit XP are honest randomness",
                                        "witness": witness,
                                    }));
                                }
                                Err(error) => break Err(error),
                            }
                        }
                    } else if timed_out {
                        break Err("idle/non-progress: missing Duel records after Start".into());
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    {
        let pair = state.lock().unwrap();
        println!(
            "{}",
            json!({
                "phase": "witness",
                "air_a": pair.0.air,
                "air_b": pair.1.air,
                "duel_a": pair.0.duel,
                "duel_b": pair.1.duel,
            })
        );
    }
    play.script_stop(&names[0]);
    play.script_stop(&names[1]);
    play.stop_slot(&names[0]);
    play.stop_slot(&names[1]);
    let outcome = outcome?;
    println!("PASS: paired_catalog_{:?}: {outcome}", case);
    Ok(())
}

fn fail_live(name: &str, case: PairCase) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_cell(case)));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            eprintln!("FAIL: {name}: {error}");
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("FAIL: {name}: {error:?}");
            std::process::exit(1);
        }
    }
}

#[test]
#[ignore = "requires LIVE=1, PAIRED_CATALOG_REVISION/NAV_PACK/ROOT/COMMIT, and local engine"]
fn paired_catalog_air_live() {
    fail_live("paired_catalog_air_live", PairCase::Air);
}

#[test]
#[ignore = "requires LIVE=1, PAIRED_CATALOG_REVISION/NAV_PACK/ROOT/COMMIT, and local engine"]
fn paired_catalog_duel_live() {
    fail_live("paired_catalog_duel_live", PairCase::Duel);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn air_obs(player: &str, essence: i32, air: i32, xp: i32) -> AirObservation {
        AirObservation {
            ingame: true,
            scene_state: 2,
            inventory_tab_available: true,
            player: Some(player.into()),
            tile: Some(AIR_RUINS),
            tick: 10,
            runecraft: 1,
            runecraft_xp: xp,
            essence_unnoted: essence,
            essence_noted: 0,
            air_runes: air,
            air_talisman: if player == "alice" { 1 } else { 0 },
            bank_open: false,
            bank_loaded: false,
            bank_essence_unnoted: 0,
            trade_offer_open: false,
            trade_confirm_open: false,
            trade_partner: None,
            trade_accept_id: -1,
            trade_mine_essence: 0,
            trade_theirs_essence: 0,
            in_temple: false,
        }
    }

    fn air_pair() -> AirPairWitness {
        let master_base = air_obs("alice", 0, 0, 0);
        let runner_base = air_obs("bob", 25, 0, 0);
        let mut master = AirSlotRecord::new(
            AirRole::Master,
            "alice".into(),
            "alice".into(),
            "bob".into(),
            json!({"mode":"Master","partner":"bob","rune":"Air runes"})
                .as_object()
                .cloned()
                .unwrap(),
            master_base.clone(),
        );
        let mut runner = AirSlotRecord::new(
            AirRole::Runner,
            "bob".into(),
            "bob".into(),
            "alice".into(),
            json!({"mode":"Runner","partner":"alice","rune":"Air runes"})
                .as_object()
                .cloned()
                .unwrap(),
            runner_base.clone(),
        );
        master.post_start = 8;
        runner.post_start = 8;
        master.saw_offer_with_partner = true;
        master.saw_confirm_with_partner = true;
        runner.saw_offer_with_partner = true;
        runner.saw_confirm_with_partner = true;
        master.transferred_in = 25;
        runner.transferred_out = 25;
        master.air_from_script = 25;
        master.xp_from_script = 125;
        let mut master_now = master_base;
        master_now.air_runes = 25;
        master_now.runecraft_xp = 125;
        master_now.tick = 80;
        master.latest = Some(master_now);
        let mut runner_now = runner_base;
        runner_now.essence_unnoted = 0;
        runner_now.tick = 80;
        runner.latest = Some(runner_now);
        runner.min_essence_after_start = 0;
        AirPairWitness { master, runner }
    }

    fn duel_obs(player: &str, xp: i32) -> DuelObservation {
        DuelObservation {
            ingame: true,
            scene_state: 2,
            inventory_tab_available: true,
            player: Some(player.into()),
            tile: Some(DUEL_CHALLENGE_ANCHOR),
            tick: 10,
            attack_xp: xp,
            strength_xp: 0,
            defence_xp: 0,
            hitpoints_xp: 0,
            in_combat: false,
            in_challenge_area: true,
            in_fight_pen: false,
            main_modal: -1,
            duel_offer_open: false,
            duel_confirm_open: false,
            duel_win_open: false,
            duel_partner: None,
            waiting_for_other: false,
            weapon_equipped: true,
            peer_visible: true,
        }
    }

    fn duel_pair() -> DuelPairWitness {
        let a_base = duel_obs("alice", 0);
        let b_base = duel_obs("bob", 0);
        let mut a = DuelSlotRecord::new(
            "alice".into(),
            "alice".into(),
            "bob".into(),
            serde_json::Map::new(),
            a_base,
        );
        let mut b = DuelSlotRecord::new(
            "bob".into(),
            "bob".into(),
            "alice".into(),
            serde_json::Map::new(),
            b_base,
        );
        a.post_start = 8;
        b.post_start = 8;
        a.saw_offer = true;
        a.saw_confirm = true;
        b.saw_offer = true;
        b.saw_confirm = true;
        a.saw_pen = true;
        b.saw_pen = true;
        a.saw_combat = true;
        b.saw_combat = true;
        a.melee_xp_from_script = 12;
        b.melee_xp_from_script = 8;
        DuelPairWitness { a, b }
    }

    #[test]
    fn frozen_catalog_hashes_match_both_revisions() {
        frozen_card_hashes_match(PairCase::Air).unwrap();
        frozen_card_hashes_match(PairCase::Duel).unwrap();
        assert_eq!(NATURECRAFTER, "NatureCrafter");
        assert_eq!(DUEL_ARENA, "Duel Arena Combat Trainer");
        assert_eq!(
            NATURECRAFTER_SHA256,
            "025ac395b25d64ef818cc0321478f0a2c84a051b79f99decbbfec5a9a2f0812a"
        );
        assert_eq!(
            DUEL_ARENA_SHA256,
            "5656dabb30a47aac590fa1afadba19e689dd792d70da8dc4851e18d62e52d090"
        );
        assert_eq!(
            NATURE_RUNNER_LOGIC_SHA256,
            "7a81b75a4cc4fde41d12565f5fe999de7931d88f2529da6d2d5e6a31f82d82f0"
        );
        assert_eq!(
            DUEL_ARENA_LOGIC_SHA256,
            "325ce631a7a3f246ab0bc51e9b09945aaa018d7c8971b334994384f62cd1d8f2"
        );
        assert_eq!(
            DUEL_INTERFACE_SHA256,
            "658e20f50117f0d23b8524e0ca389399d6fd52de2de1585313081324434634f9"
        );
        for commit in [CATALOG_COMMIT_A, CATALOG_COMMIT_B] {
            catalog_ledger(commit).unwrap();
            card_row(commit, 274, NATURECRAFTER).unwrap();
            card_row(commit, 289, DUEL_ARENA).unwrap();
        }
    }

    #[test]
    fn generated_duel_controls_match_both_caches() {
        for revision in [ClientRevision::R274, ClientRevision::R289] {
            let data = game_data::for_revision(revision).unwrap();
            verify_generated_duel_controls(&data).unwrap();
            let scim = data.item_by_alias("bronze_scimitar").unwrap();
            assert_eq!(scim.wear_position, WEARPOS_RIGHTHAND);
            assert_eq!(data.item_by_alias("blankrune").unwrap().id, 1436);
            assert_eq!(data.item_by_alias("airrune").unwrap().id, 556);
            assert_eq!(data.item_by_alias("air_talisman").unwrap().id, 1438);
        }
    }

    #[test]
    fn operation_gates_name_foreign_owners_and_do_not_claim_shop_named_bank() {
        assert!(air_operation_gates().iter().any(
            |gate| gate.kind == GateKind::ArityLimited && gate.call.contains("Trade.offerAll")
        ));
        assert!(air_operation_gates()
            .iter()
            .any(|gate| gate.kind == GateKind::UnusedByCase && gate.owner.contains("t_1591d140")));
        assert!(air_operation_gates()
            .iter()
            .any(|gate| gate.kind == GateKind::UnusedByCase && gate.owner.contains("t_bced5c76")));
        assert!(duel_operation_gates().iter().any(|gate| {
            gate.kind == GateKind::CatalogLiteralMatchesGenerated && gate.call.contains("ifButton")
        }));
        assert!(duel_operation_gates()
            .iter()
            .any(|gate| gate.kind == GateKind::UnusedByCase && gate.owner.contains("t_68de6f48")));
        assert_eq!(SCRIPT_GOLD_DEADLINE_SECS, 180);
        assert_eq!(SCRIPT_GOLD_WATCH_TICKS, 150);
    }

    #[test]
    fn air_rejects_wrong_partner() {
        let mut pair = air_pair();
        pair.master.saw_wrong_partner = true;
        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("wrong partner"), "{error}");
    }

    #[test]
    fn air_rejects_one_sided_confirmation() {
        let mut pair = air_pair();
        pair.runner.saw_confirm_with_partner = false;
        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("one-sided confirmation"), "{error}");
    }

    #[test]
    fn air_rejects_seed_only_inventory() {
        let mut pair = air_pair();
        pair.master.air_from_script = 0;
        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("seed-only"), "{error}");
    }

    #[test]
    fn air_rejects_missing_conservation() {
        let mut pair = air_pair();
        pair.runner.transferred_out = 0;
        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("missing conservation"), "{error}");
    }

    #[test]
    fn air_rejects_stale_trade() {
        let mut pair = air_pair();
        pair.master.latest.as_mut().unwrap().trade_offer_open = true;
        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("stale trade"), "{error}");
    }

    #[test]
    fn air_rejects_full_cycle_without_bank_restock() {
        let pair = air_pair();
        pair.qualify_supported().unwrap();
        let error = pair.qualify_full_cycle().unwrap_err();
        assert!(error.contains("no actual bank restock"), "{error}");
    }

    #[test]
    fn air_rejects_full_cycle_without_second_transfer() {
        let mut pair = air_pair();
        pair.runner.saw_bank_open_loaded = true;
        pair.runner.saw_bank_at_falador = true;
        pair.runner.restock_withdraw = true;
        pair.runner.returned_to_ruins = true;
        let error = pair.qualify_full_cycle().unwrap_err();
        assert!(error.contains("no further work"), "{error}");
    }

    #[test]
    fn air_accepts_first_transfer_craft() {
        let pair = air_pair();
        assert_eq!(
            pair.qualify_supported().unwrap(),
            AirClaim::FirstTransferCraft
        );
    }

    #[test]
    fn duel_rejects_wrong_partner() {
        let mut pair = duel_pair();
        pair.a.saw_wrong_partner = true;
        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("wrong partner"), "{error}");
    }

    #[test]
    fn duel_rejects_one_sided_confirmation() {
        let mut pair = duel_pair();
        pair.b.saw_confirm = false;
        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("one-sided confirmation"), "{error}");
    }

    #[test]
    fn duel_rejects_seeded_modal() {
        let mut pair = duel_pair();
        pair.a.baseline.duel_offer_open = true;
        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("seeded modal"), "{error}");
    }

    #[test]
    fn duel_rejects_queued_challenge_without_combat() {
        let mut pair = duel_pair();
        pair.a.saw_combat = false;
        pair.b.saw_combat = false;
        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("no real duel combat"), "{error}");
    }

    #[test]
    fn duel_rejects_seed_only_xp() {
        let mut pair = duel_pair();
        pair.a.melee_xp_from_script = 0;
        pair.b.melee_xp_from_script = 0;
        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("seed-only"), "{error}");
    }

    #[test]
    fn duel_rejects_full_cycle_without_reset() {
        let pair = duel_pair();
        pair.qualify_supported().unwrap();
        let error = pair.qualify_full_cycle().unwrap_err();
        assert!(error.contains("no further work"), "{error}");
    }

    #[test]
    fn duel_accepts_first_combat() {
        let pair = duel_pair();
        assert_eq!(pair.qualify_supported().unwrap(), DuelClaim::FirstCombat);
    }

    #[test]
    fn serializes_complete_air_and_duel_witnesses() {
        let air = serde_json::to_value(air_pair()).unwrap();
        for key in ["baseline", "account", "settings", "partner"] {
            assert!(air["master"].get(key).is_some(), "missing master.{key}");
            assert!(air["runner"].get(key).is_some(), "missing runner.{key}");
        }
        let duel = serde_json::to_value(duel_pair()).unwrap();
        for slot in ["a", "b"] {
            assert!(duel[slot].get("baseline").is_some());
            assert!(duel[slot].get("account").is_some());
        }
    }
}
