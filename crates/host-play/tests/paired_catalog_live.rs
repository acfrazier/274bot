//! Paired NatureCrafter Air, MuleCrafter Air, and Duel Arena fixtures through
//! shared Play.
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
use api::snapshot::{GameSnapshot, WorldTile};
use client::io::ClientRevision;
use host::Pump;
use host_play::{ProfileOptions, ScriptStartHandle, SharedClientTemplate};
use serde_json::json;
use vault::{Profile, ProfileSettings};

use paired_catalog::{
    air_operation_gates, air_prepared_current, air_settings, bank_ack_target_absence,
    bank_seed_acknowledged, card_row, catalog_ledger, duel_operation_gates, duel_prepared_current,
    duel_settings, flax_operation_gates, flax_prepared_current, frozen_card_hashes_match,
    hash_file, mule_mode_requires_partner, mule_operation_gates, mule_prepared_current,
    mule_settings, near, prepare_card, relog_admission, shared_start_barrier,
    verify_generated_duel_controls, verify_registry_identity, AirClaim, AirObservation,
    AirPairWitness, AirRole, AirSlotRecord, DuelClaim, DuelObservation, DuelPairWitness,
    DuelSlotRecord, FlaxClaim, FlaxObservation, FlaxPairWitness, FlaxRole, FlaxSlotRecord,
    GateKind, MuleClaim, MulePairWitness, MuleRole, MuleSlotRecord, PairCase, PreparedCard,
    RelogAdmission, StartBarrier, StartBarrierInput, AIR_RUINS, BANK_SEED_ESSENCE,
    CATALOG_COMMIT_A, CATALOG_COMMIT_B, DUEL_ARENA, DUEL_ARENA_LOGIC_SHA256, DUEL_ARENA_SHA256,
    DUEL_CHALLENGE_ANCHOR, DUEL_INTERFACE_SHA256, FALADOR_EAST, FLAXRUNNER, FLAXRUNNER_SHA256,
    FLAX_BANK, FLAX_FIELD, FLAX_MEET, MULECRAFTER, MULECRAFTER_LOGIC_SHA256, MULECRAFTER_SHA256,
    MULE_TRADE_CAP, NATURECRAFTER, NATURECRAFTER_SHA256, NATURE_RUNNER_LOGIC_SHA256,
    PREP_DEADLINE_SECS, SCRIPT_GOLD_DEADLINE_SECS, SCRIPT_GOLD_WATCH_TICKS, TRADE_CAP,
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
    MuleCrafter,
    MuleMule,
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
    mule: Option<MuleSlotRecord>,
    duel: Option<DuelSlotRecord>,
    latest_air: Option<AirObservation>,
    latest_duel: Option<DuelObservation>,
    weapon_id: i32,
    saw_logout: bool,
    bank_ack_done: bool,
    bank_ack_generation: Option<u64>,
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

    fn air_role(&self) -> AirRole {
        match self.kind {
            SlotKind::AirMaster => AirRole::Master,
            _ => AirRole::Runner,
        }
    }

    fn mule_role(&self) -> MuleRole {
        match self.kind {
            SlotKind::MuleCrafter => MuleRole::Crafter,
            _ => MuleRole::Mule,
        }
    }

    fn has_baseline(&self) -> bool {
        match self.kind {
            SlotKind::AirMaster | SlotKind::AirRunner => self.air.is_some(),
            SlotKind::MuleCrafter | SlotKind::MuleMule => self.mule.is_some(),
            SlotKind::Duel => self.duel.is_some(),
        }
    }

    fn prepared_unstarted(&self) -> bool {
        self.prep == Prep::Ready && self.has_baseline() && !self.started
    }

    fn current_ok(&self) -> bool {
        match self.kind {
            SlotKind::AirMaster | SlotKind::AirRunner => {
                self.latest_air.as_ref().is_some_and(|observation| {
                    air_prepared_current(self.air_role(), &self.expected_player, observation)
                        .is_ok()
                })
            }
            SlotKind::MuleCrafter | SlotKind::MuleMule => {
                self.latest_air.as_ref().is_some_and(|observation| {
                    mule_prepared_current(self.mule_role(), &self.expected_player, observation)
                        .is_ok()
                })
            }
            SlotKind::Duel => self.latest_duel.as_ref().is_some_and(|observation| {
                duel_prepared_current(&self.expected_player, observation).is_ok()
            }),
        }
    }

    fn falador_east_booth(snapshot: &GameSnapshot) -> Option<(WorldTile, i32)> {
        snapshot
            .locs()
            .iter()
            .filter(|loc| {
                loc.tile.level == FALADOR_EAST.2
                    && loc.actions.iter().any(|action| {
                        action
                            .as_deref()
                            .is_some_and(|name| name.eq_ignore_ascii_case("Use-quickly"))
                    })
                    && (loc.tile.x - FALADOR_EAST.0)
                        .abs()
                        .max((loc.tile.z - FALADOR_EAST.1).abs())
                        <= 2
            })
            .min_by_key(|loc| {
                (loc.tile.x - FALADOR_EAST.0)
                    .abs()
                    .max((loc.tile.z - FALADOR_EAST.1).abs())
            })
            .map(|loc| (loc.tile, loc.id))
    }

    fn start_script(&mut self) -> Result<(), String> {
        if self.started {
            return Err("Start invoked more than once".into());
        }
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
                    SlotKind::MuleCrafter => "mule_crafter",
                    SlotKind::MuleMule => "mule_mule",
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
                record.observe(observation.clone());
            }
            if let Some(record) = self.mule.as_mut() {
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
        air_prepared_current(self.air_role(), &self.expected_player, observation)?;
        let modals = self.snapshot.modals();
        if modals.main != -1 || modals.chat != -1 {
            return Err("Start baseline still has an open modal".into());
        }
        self.air = Some(AirSlotRecord::new(
            self.air_role(),
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

    fn capture_mule_baseline(&mut self, observation: &AirObservation) -> Result<(), String> {
        mule_mode_requires_partner(self.mule_role(), &self.partner)?;
        mule_prepared_current(self.mule_role(), &self.expected_player, observation)?;
        let modals = self.snapshot.modals();
        if modals.main != -1 || modals.chat != -1 {
            return Err("Start baseline still has an open modal".into());
        }
        self.mule = Some(MuleSlotRecord::new(
            self.mule_role(),
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
                    SlotKind::MuleCrafter => "crafter",
                    _ => "mule",
                },
                "counterpart": self.partner,
                "seeded_first_supplies": matches!(
                    self.kind,
                    SlotKind::MuleCrafter | SlotKind::MuleMule
                ),
                "seeded_raw_unnoted_essence": observation.essence_unnoted,
                "seeded_air_runes": observation.air_runes,
                "seeded_bank_unnoted_essence": if matches!(self.kind, SlotKind::MuleMule) {
                    BANK_SEED_ESSENCE
                } else {
                    0
                },
                "bank_seed_acknowledged": self.bank_ack_done,
                "observation": observation,
            })
        );
        Ok(())
    }

    fn capture_duel_baseline(&mut self, observation: &DuelObservation) -> Result<(), String> {
        duel_prepared_current(&self.expected_player, observation)?;
        let modals = self.snapshot.modals();
        if modals.main != -1 || modals.chat != -1 {
            return Err("Start baseline still has an open modal".into());
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
                let (ingame, scene_state, inventory_tab) = if let Some(observation) = air {
                    (
                        observation.ingame,
                        observation.scene_state,
                        observation.inventory_tab_available,
                    )
                } else if let Some(observation) = duel {
                    (
                        observation.ingame,
                        observation.scene_state,
                        observation.inventory_tab_available,
                    )
                } else {
                    return Ok(());
                };
                match relog_admission(self.saw_logout, ingame, scene_state, inventory_tab) {
                    RelogAdmission::WaitLogout | RelogAdmission::WaitLogin => {}
                    RelogAdmission::LoggedOut => {
                        self.saw_logout = true;
                        println!(
                            "{}",
                            json!({
                                "phase": "relog-observed-logout",
                                "account": self.account,
                            })
                        );
                    }
                    RelogAdmission::Ready => {
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
            }
            Prep::Seed => {
                if !Self::send_ok(hold) {
                    return Ok(());
                }
                let kind = match self.kind {
                    SlotKind::AirMaster => {
                        interact::cheat(client, "give air_talisman 1");
                        interact::cheat(
                            client,
                            &interact::tele_args(AIR_RUINS.2, AIR_RUINS.0, AIR_RUINS.1),
                        );
                        "air_master_talisman_noessence_at_ruins"
                    }
                    SlotKind::MuleCrafter => {
                        interact::cheat(client, "give air_talisman 1");
                        interact::cheat(client, &format!("give blankrune {MULE_TRADE_CAP}"));
                        interact::cheat(
                            client,
                            &interact::tele_args(AIR_RUINS.2, AIR_RUINS.0, AIR_RUINS.1),
                        );
                        "mule_crafter_talisman_raw_firstload27_at_ruins"
                    }
                    SlotKind::AirRunner | SlotKind::MuleMule if !self.bank_ack_done => {
                        interact::cheat(client, &format!("givebank blankrune {BANK_SEED_ESSENCE}"));
                        interact::cheat(
                            client,
                            &interact::tele_args(FALADOR_EAST.2, FALADOR_EAST.0, FALADOR_EAST.1),
                        );
                        if matches!(self.kind, SlotKind::MuleMule) {
                            "mule_mule_bank200_at_falador_east"
                        } else {
                            "air_runner_bank200_at_falador_east"
                        }
                    }
                    SlotKind::AirRunner => {
                        interact::cheat(client, &format!("give blankrune {TRADE_CAP}"));
                        interact::cheat(
                            client,
                            &interact::tele_args(AIR_RUINS.2, AIR_RUINS.0, AIR_RUINS.1),
                        );
                        "air_runner_firstload25_at_ruins"
                    }
                    SlotKind::MuleMule => {
                        interact::cheat(client, &format!("give blankrune {MULE_TRADE_CAP}"));
                        interact::cheat(
                            client,
                            &interact::tele_args(AIR_RUINS.2, AIR_RUINS.0, AIR_RUINS.1),
                        );
                        "mule_mule_firstload27_at_ruins"
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
                        "duel_bronze_scimitar_at_challenge_anchor"
                    }
                };
                println!(
                    "{}",
                    json!({
                        "phase": "seed-queued",
                        "account": self.account,
                        "kind": kind,
                        "note": "queued seed is not bank acknowledgement or script restock",
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
                            && o.essence_unnoted == 0
                    }),
                    SlotKind::MuleCrafter => air.is_some_and(|o| {
                        o.ingame
                            && o.scene_state == 2
                            && near(o.tile, AIR_RUINS, 8)
                            && o.air_talisman >= 1
                            && o.essence_unnoted == MULE_TRADE_CAP
                    }),
                    SlotKind::AirRunner | SlotKind::MuleMule if !self.bank_ack_done => air
                        .is_some_and(|o| {
                            o.ingame && o.scene_state == 2 && near(o.tile, FALADOR_EAST, 8)
                        }),
                    SlotKind::AirRunner => air.is_some_and(|o| {
                        o.ingame
                            && o.scene_state == 2
                            && near(o.tile, AIR_RUINS, 8)
                            && o.essence_unnoted >= TRADE_CAP
                    }),
                    SlotKind::MuleMule => air.is_some_and(|o| {
                        o.ingame
                            && o.scene_state == 2
                            && near(o.tile, AIR_RUINS, 8)
                            && o.essence_unnoted >= MULE_TRADE_CAP
                    }),
                    SlotKind::Duel => duel.is_some_and(|o| {
                        o.ingame && o.scene_state == 2 && near(o.tile, DUEL_CHALLENGE_ANCHOR, 8)
                    }),
                };
                if ready {
                    println!(
                        "{}",
                        json!({
                            "phase": "seed-observed",
                            "account": self.account,
                            "bank_ack_done": self.bank_ack_done,
                        })
                    );
                    self.prep = Prep::DrainDialogs;
                }
            }
            Prep::DrainDialogs => {
                let modals = self.snapshot.modals();
                if modals.main == -1 && modals.chat == -1 {
                    self.prep = match self.kind {
                        SlotKind::AirRunner | SlotKind::MuleMule if !self.bank_ack_done => {
                            Prep::AckBank
                        }
                        SlotKind::Duel => Prep::Wear,
                        SlotKind::AirMaster
                        | SlotKind::AirRunner
                        | SlotKind::MuleCrafter
                        | SlotKind::MuleMule => Prep::Ready,
                    };
                } else if Self::send_ok(hold) {
                    let _ = interact::close_modal(client);
                }
            }
            Prep::AckBank => {
                let Some(observation) = air else {
                    return Ok(());
                };
                if !near(observation.tile, FALADOR_EAST, 8) {
                    return Err(bank_ack_target_absence(observation.tile, false).unwrap_or_else(
                        || {
                            format!(
                                "AckBank refused: runner stand {:?} is not Falador East {FALADOR_EAST:?}",
                                observation.tile
                            )
                        },
                    ));
                }
                if bank_seed_acknowledged(observation, BANK_SEED_ESSENCE) {
                    self.prep = Prep::WaitAck;
                    return Ok(());
                }
                if !Self::send_ok(hold) {
                    return Ok(());
                }
                if now.duration_since(self.last_action) < Duration::from_millis(400) {
                    return Ok(());
                }
                let booth = Self::falador_east_booth(&self.snapshot);
                if let Some(reason) = bank_ack_target_absence(observation.tile, booth.is_some()) {
                    println!(
                        "{}",
                        json!({
                            "phase": "ack-bank-target-absent",
                            "account": self.account,
                            "detail": reason,
                            "stand": FALADOR_EAST,
                            "tile": observation.tile,
                        })
                    );
                    self.last_action = now;
                    return Ok(());
                }
                let Some((tile, id)) = booth else {
                    return Ok(());
                };
                match Interactions::new(&self.snapshot, client).open_booth_at(tile, id) {
                    SendResult::Sent { .. } => {
                        self.last_action = now;
                        self.prep = Prep::WaitAck;
                    }
                    SendResult::Refused { reason, .. } => {
                        println!(
                            "{}",
                            json!({
                                "phase": "ack-bank-refused",
                                "account": self.account,
                                "reason": format!("{reason:?}"),
                                "target_absence": bank_ack_target_absence(
                                    observation.tile,
                                    Self::falador_east_booth(&self.snapshot).is_some(),
                                ),
                                "stand": FALADOR_EAST,
                                "tile": observation.tile,
                                "booth": [tile.x, tile.z, tile.level],
                            })
                        );
                        self.last_action = now;
                    }
                }
            }
            Prep::WaitAck => {
                let Some(observation) = air else {
                    return Ok(());
                };
                if !near(observation.tile, FALADOR_EAST, 8) {
                    return Err(bank_ack_target_absence(observation.tile, false).unwrap_or_else(
                        || {
                            format!(
                                "WaitAck refused: runner stand {:?} is not Falador East {FALADOR_EAST:?}",
                                observation.tile
                            )
                        },
                    ));
                }
                if bank_seed_acknowledged(observation, BANK_SEED_ESSENCE) {
                    self.bank_ack_generation = Some(observation.bank_session_generation);
                    println!(
                        "{}",
                        json!({
                            "phase": "acknowledged-bank",
                            "account": self.account,
                            "bank_essence_unnoted": observation.bank_essence_unnoted,
                            "bank_session_generation": observation.bank_session_generation,
                            "stand": FALADOR_EAST,
                            "tile": observation.tile,
                            "note": "givebank blankrune 200 observed open+loaded at Falador East; not the script restock",
                        })
                    );
                    self.prep = Prep::CloseBank;
                } else if !observation.bank_open
                    && Self::send_ok(hold)
                    && now.duration_since(self.last_action) >= Duration::from_millis(400)
                {
                    let booth = Self::falador_east_booth(&self.snapshot);
                    if let Some(reason) = bank_ack_target_absence(observation.tile, booth.is_some())
                    {
                        println!(
                            "{}",
                            json!({
                                "phase": "ack-bank-target-absent",
                                "account": self.account,
                                "detail": reason,
                                "stand": FALADOR_EAST,
                                "tile": observation.tile,
                            })
                        );
                        self.last_action = now;
                    } else if let Some((tile, id)) = booth {
                        match Interactions::new(&self.snapshot, client).open_booth_at(tile, id) {
                            SendResult::Sent { .. } => {
                                self.last_action = now;
                            }
                            SendResult::Refused { reason, .. } => {
                                println!(
                                    "{}",
                                    json!({
                                        "phase": "ack-bank-refused",
                                        "account": self.account,
                                        "reason": format!("{reason:?}"),
                                        "stand": FALADOR_EAST,
                                        "tile": observation.tile,
                                    })
                                );
                                self.last_action = now;
                            }
                        }
                    }
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
                    println!(
                        "{}",
                        json!({
                            "phase": "closed-bank",
                            "account": self.account,
                            "bank_session_generation": observation.bank_session_generation,
                            "ack_generation": self.bank_ack_generation,
                            "generation_moved": self.bank_ack_generation
                                .map(|generation| generation != observation.bank_session_generation),
                            "stand": FALADOR_EAST,
                        })
                    );
                    self.bank_ack_done = true;
                    self.prep = Prep::Seed;
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
            Prep::Ready => match self.kind {
                SlotKind::AirMaster | SlotKind::AirRunner => {
                    if self.air.is_none() {
                        let Some(observation) = air else {
                            return Ok(());
                        };
                        self.capture_air_baseline(observation)?;
                        println!(
                            "{}",
                            json!({
                                "phase": "prepared",
                                "account": self.account,
                                "role": match self.air_role() {
                                    AirRole::Master => "master",
                                    AirRole::Runner => "runner",
                                },
                                "note": "waiting for shared start barrier; not Start",
                            })
                        );
                    }
                }
                SlotKind::MuleCrafter | SlotKind::MuleMule => {
                    if self.mule.is_none() {
                        let Some(observation) = air else {
                            return Ok(());
                        };
                        self.capture_mule_baseline(observation)?;
                        println!(
                            "{}",
                            json!({
                                "phase": "prepared",
                                "account": self.account,
                                "role": match self.mule_role() {
                                    MuleRole::Crafter => "crafter",
                                    MuleRole::Mule => "mule",
                                },
                                "note": "waiting for shared start barrier; not Start",
                            })
                        );
                    }
                }
                SlotKind::Duel => {
                    if self.duel.is_none() {
                        let Some(observation) = duel else {
                            return Ok(());
                        };
                        self.capture_duel_baseline(observation)?;
                        println!(
                            "{}",
                            json!({
                                "phase": "prepared",
                                "account": self.account,
                                "note": "waiting for shared start barrier; not Start",
                            })
                        );
                    }
                }
            },
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
        SlotKind::MuleCrafter => mule_settings(&card.schema, MuleRole::Crafter, &partner),
        SlotKind::MuleMule => mule_settings(&card.schema, MuleRole::Mule, &partner),
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
        mule: None,
        duel: None,
        latest_air: None,
        latest_duel: None,
        weapon_id,
        saw_logout: false,
        bank_ack_done: false,
        bank_ack_generation: None,
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
        PairCase::Mule => new_slot(
            SlotKind::MuleCrafter,
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
        PairCase::Flax => {
            return Err(
                "flax LIVE prep remains root-owned; offline FlaxPairWitness is the 157 fixture"
                    .into(),
            );
        }
    };
    let slot_b = match case {
        PairCase::Air => new_slot(
            SlotKind::AirRunner,
            names[1].clone(),
            screen_a.clone(),
            &card,
            weapon,
        )?,
        PairCase::Mule => new_slot(
            SlotKind::MuleMule,
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
        PairCase::Flax => {
            return Err(
                "flax LIVE prep remains root-owned; offline FlaxPairWitness is the 157 fixture"
                    .into(),
            );
        }
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
                    PairCase::Air | PairCase::Mule => pair.0.frame_air(client, hold),
                    PairCase::Duel => pair.0.frame_duel(client, hold),
                    PairCase::Flax => {}
                }
            } else if username == pair.1.account {
                match case {
                    PairCase::Air | PairCase::Mule => pair.1.frame_air(client, hold),
                    PairCase::Duel => pair.1.frame_duel(client, hold),
                    PairCase::Flax => {}
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
            let decision = shared_start_barrier(StartBarrierInput {
                a_prepared: pair.0.prepared_unstarted(),
                b_prepared: pair.1.prepared_unstarted(),
                a_started: pair.0.started,
                b_started: pair.1.started,
                a_wait_ack: pair.0.prep == Prep::WaitAck,
                b_wait_ack: pair.1.prep == Prep::WaitAck,
                a_current_ok: pair.0.current_ok(),
                b_current_ok: pair.1.current_ok(),
            });
            if decision == StartBarrier::StartBoth {
                pair.0.start_script()?;
                pair.1.start_script()?;
            }
            (
                decision,
                pair.0.started,
                pair.1.started,
                pair.0.prep,
                pair.1.prep,
            )
        };
        let timed_out = Instant::now() >= deadline;
        if phase == 0 {
            match snapshot.0 {
                StartBarrier::StartBoth => {
                    println!(
                        "{}",
                        json!({
                            "phase": "both-started",
                            "gold_clock": "begin",
                            "note": "gold clock starts only after both Start; preparation is not counted",
                        })
                    );
                    phase = 1;
                    deadline = Instant::now() + Duration::from_secs(SCRIPT_GOLD_DEADLINE_SECS);
                }
                StartBarrier::RejectStartedWhileUnready => {
                    break Err(format!(
                        "start while the other actor was not prepared; a_prep={:?} b_prep={:?} a_started={} b_started={}",
                        snapshot.3, snapshot.4, snapshot.1, snapshot.2
                    ));
                }
                StartBarrier::Wait => {
                    if timed_out {
                        break Err(format!(
                            "preparation timeout; a_prep={:?} b_prep={:?} a_started={} b_started={}",
                            snapshot.3, snapshot.4, snapshot.1, snapshot.2
                        ));
                    }
                }
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
                PairCase::Mule => {
                    if let (Some(crafter), Some(mule)) = (pair.0.mule.clone(), pair.1.mule.clone())
                    {
                        let witness = MulePairWitness { crafter, mule };
                        if witness.qualify_full_cycle().is_ok() {
                            break Ok(json!({
                                "claim": MuleClaim::MuleBankReturnSecondCycle,
                                "witness": witness,
                            }));
                        }
                        if timed_out {
                            match witness.qualify_supported() {
                                Ok(claim) => {
                                    break Ok(json!({
                                        "claim": claim,
                                        "full_cycle": false,
                                        "limit": "mule bank deposit/restock/second transfer/craft did not finish inside unchanged SCRIPT_GOLD_DEADLINE 180s",
                                        "witness": witness,
                                    }));
                                }
                                Err(error) => break Err(error),
                            }
                        }
                    } else if timed_out {
                        break Err("idle/non-progress: missing Mule records after Start".into());
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
                PairCase::Flax => {
                    break Err(
                        "flax LIVE prep remains root-owned; offline FlaxPairWitness is the 157 fixture"
                            .into(),
                    );
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
                "mule_a": pair.0.mule,
                "mule_b": pair.1.mule,
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

#[test]
#[ignore = "requires LIVE=1, PAIRED_CATALOG_REVISION/NAV_PACK/ROOT/COMMIT, and local engine"]
fn paired_catalog_mule_live() {
    fail_live("paired_catalog_mule_live", PairCase::Mule);
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
            bank_session_generation: 0,
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

    fn mule_pair() -> MulePairWitness {
        let crafter_base = air_obs("alice", MULE_TRADE_CAP, 0, 0);
        let mule_base = air_obs("bob", 27, 0, 0);
        let mut crafter = MuleSlotRecord::new(
            MuleRole::Crafter,
            "alice".into(),
            "alice".into(),
            "bob".into(),
            json!({"mode":"Crafter","partner":"bob","rune":"Air rune","bankFill":true})
                .as_object()
                .cloned()
                .unwrap(),
            crafter_base.clone(),
        );
        let mut mule = MuleSlotRecord::new(
            MuleRole::Mule,
            "bob".into(),
            "bob".into(),
            "alice".into(),
            json!({"mode":"Mule","partner":"alice","rune":"Air rune","bankFill":true})
                .as_object()
                .cloned()
                .unwrap(),
            mule_base.clone(),
        );
        crafter.post_start = 8;
        mule.post_start = 8;
        crafter.saw_offer_with_partner = true;
        crafter.saw_confirm_with_partner = true;
        mule.saw_offer_with_partner = true;
        mule.saw_confirm_with_partner = true;
        crafter.transferred_in = 27;
        mule.transferred_out = 27;
        crafter.air_transferred_out = 27;
        mule.air_transferred_in = 27;
        crafter.partner_transfer_events = 1;
        mule.partner_transfer_events = 1;
        crafter.post_exchange_craft_events = 1;
        crafter.air_from_script = 27;
        crafter.xp_from_script = 270;
        crafter.craft_events = 2;
        let mut crafter_now = crafter_base;
        crafter_now.air_runes = 27;
        crafter_now.runecraft_xp = 270;
        crafter_now.tick = 80;
        crafter.latest = Some(crafter_now);
        let mut mule_now = mule_base;
        mule_now.essence_unnoted = 0;
        mule_now.air_runes = 27;
        mule_now.tick = 80;
        mule.latest = Some(mule_now);
        mule.min_essence_after_start = 0;
        MulePairWitness { crafter, mule }
    }

    fn mule_fresh_pair() -> MulePairWitness {
        let crafter_base = air_obs("alice", MULE_TRADE_CAP, 0, 0);
        let mule_base = air_obs("bob", 27, 0, 0);
        MulePairWitness {
            crafter: MuleSlotRecord::new(
                MuleRole::Crafter,
                "alice".into(),
                "alice".into(),
                "bob".into(),
                json!({"mode":"Crafter","partner":"bob","rune":"Air rune","bankFill":true})
                    .as_object()
                    .cloned()
                    .unwrap(),
                crafter_base,
            ),
            mule: MuleSlotRecord::new(
                MuleRole::Mule,
                "bob".into(),
                "bob".into(),
                "alice".into(),
                json!({"mode":"Mule","partner":"alice","rune":"Air rune","bankFill":true})
                    .as_object()
                    .cloned()
                    .unwrap(),
                mule_base,
            ),
        }
    }

    fn with_trade(
        mut observation: AirObservation,
        partner: &str,
        offer: bool,
        confirm: bool,
    ) -> AirObservation {
        observation.trade_offer_open = offer;
        observation.trade_confirm_open = confirm;
        observation.trade_partner = Some(partner.into());
        observation
    }

    fn without_trade_partner(mut observation: AirObservation) -> AirObservation {
        observation.trade_partner = None;
        observation
    }

    fn observe_first_exchange(pair: &mut MulePairWitness) {
        let mut seed_craft = air_obs("alice", 0, MULE_TRADE_CAP, 135);
        seed_craft.tile = Some((AIR_RUINS.0, 4800, AIR_RUINS.2));
        seed_craft.in_temple = true;
        pair.crafter.observe(seed_craft);
        pair.crafter
            .observe(with_trade(air_obs("alice", 0, 27, 135), "bob", true, false));
        pair.crafter
            .observe(with_trade(air_obs("alice", 0, 27, 135), "bob", false, true));
        pair.mule
            .observe(with_trade(air_obs("bob", 27, 0, 0), "alice", true, false));
        pair.mule
            .observe(with_trade(air_obs("bob", 27, 0, 0), "alice", false, true));
        pair.mule.observe(air_obs("bob", 0, 27, 0));
        pair.crafter.observe(air_obs("alice", 27, 0, 135));
        let mut post_exchange_craft = air_obs("alice", 0, 27, 270);
        post_exchange_craft.tile = Some((AIR_RUINS.0, 4800, AIR_RUINS.2));
        post_exchange_craft.in_temple = true;
        pair.crafter.observe(post_exchange_craft);
    }

    fn observe_second_exchange(pair: &mut MulePairWitness, finish_craft: bool) {
        let mut deposited = air_obs("bob", 0, 0, 0);
        deposited.tile = Some(FALADOR_EAST);
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        pair.mule.observe(deposited);

        let mut restocked = air_obs("bob", MULE_TRADE_CAP, 0, 0);
        restocked.tile = Some(FALADOR_EAST);
        restocked.bank_open = true;
        restocked.bank_loaded = true;
        pair.mule.observe(restocked);
        pair.mule.observe(air_obs("bob", MULE_TRADE_CAP, 0, 0));

        pair.crafter
            .observe(with_trade(air_obs("alice", 0, 27, 270), "bob", true, false));
        pair.crafter
            .observe(with_trade(air_obs("alice", 0, 27, 270), "bob", false, true));
        pair.mule
            .observe(with_trade(air_obs("bob", 27, 0, 0), "alice", true, false));
        pair.mule
            .observe(with_trade(air_obs("bob", 27, 0, 0), "alice", false, true));
        pair.mule.observe(air_obs("bob", 0, 27, 0));
        pair.crafter.observe(air_obs("alice", 27, 0, 270));

        if finish_craft {
            let mut post_exchange_craft = air_obs("alice", 0, 27, 405);
            post_exchange_craft.tile = Some((AIR_RUINS.0, 4800, AIR_RUINS.2));
            post_exchange_craft.in_temple = true;
            pair.crafter.observe(post_exchange_craft);
        }
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
        frozen_card_hashes_match(PairCase::Mule).unwrap();
        frozen_card_hashes_match(PairCase::Flax).unwrap();
        frozen_card_hashes_match(PairCase::Duel).unwrap();
        assert_eq!(NATURECRAFTER, "NatureCrafter");
        assert_eq!(MULECRAFTER, "MuleCrafter");
        assert_eq!(FLAXRUNNER, "FlaxRunner");
        assert_eq!(DUEL_ARENA, "Duel Arena Combat Trainer");
        assert_eq!(
            NATURECRAFTER_SHA256,
            "025ac395b25d64ef818cc0321478f0a2c84a051b79f99decbbfec5a9a2f0812a"
        );
        assert_eq!(
            MULECRAFTER_SHA256,
            "bf745db4c0a3df22406b49c8a8b716b10e0594302853d80ca6f38862d05dc1f9"
        );
        assert_eq!(
            DUEL_ARENA_SHA256,
            "5656dabb30a47aac590fa1afadba19e689dd792d70da8dc4851e18d62e52d090"
        );
        assert_eq!(
            FLAXRUNNER_SHA256,
            "6edae2ae773b73b907b5a3d4c020052075f7b32f9c2872ca78246eead9dfff34"
        );
        assert_eq!(
            NATURE_RUNNER_LOGIC_SHA256,
            "7a81b75a4cc4fde41d12565f5fe999de7931d88f2529da6d2d5e6a31f82d82f0"
        );
        assert_eq!(
            MULECRAFTER_LOGIC_SHA256,
            "d9cc408c1857a02332e3338e1ae1e956dea51c7c191d4a66af81be6e5108251a"
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
            card_row(commit, 274, MULECRAFTER).unwrap();
            card_row(commit, 274, FLAXRUNNER).unwrap();
            card_row(commit, 289, DUEL_ARENA).unwrap();
            card_row(commit, 289, MULECRAFTER).unwrap();
            card_row(commit, 289, FLAXRUNNER).unwrap();
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
        assert!(mule_operation_gates().iter().any(
            |gate| gate.kind == GateKind::ArityLimited && gate.call.contains("Trade.offerAll")
        ));
        assert!(mule_operation_gates()
            .iter()
            .any(|gate| gate.kind == GateKind::UnusedByCase
                && gate.source.contains("bankFill=false")));
        assert!(flax_operation_gates().iter().any(|gate| {
            gate.kind == GateKind::Mapped && gate.call.contains("driveActivePartnerTrade")
        }));
        assert!(flax_operation_gates()
            .iter()
            .any(|gate| gate.kind == GateKind::UnusedByCase && gate.source.contains("FlaxAIO")));
        assert!(duel_operation_gates().iter().any(|gate| {
            gate.kind == GateKind::CatalogLiteralMatchesGenerated && gate.call.contains("ifButton")
        }));
        assert!(duel_operation_gates()
            .iter()
            .any(|gate| gate.kind == GateKind::UnusedByCase && gate.owner.contains("t_68de6f48")));
        assert_eq!(SCRIPT_GOLD_DEADLINE_SECS, 180);
        assert_eq!(SCRIPT_GOLD_WATCH_TICKS, 150);
        assert_eq!(PREP_DEADLINE_SECS, 180);
        assert_eq!(BANK_SEED_ESSENCE, 200);
        assert_eq!(TRADE_CAP, 25);
        assert_eq!(MULE_TRADE_CAP, 27);
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
    fn mule_rejects_wrong_partner() {
        let mut pair = mule_pair();
        pair.crafter.saw_wrong_partner = true;
        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("wrong partner"), "{error}");
    }

    #[test]
    fn mule_rejects_one_sided_confirmation() {
        let mut pair = mule_pair();
        pair.mule.saw_confirm_with_partner = false;
        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("one-sided confirmation"), "{error}");
    }

    #[test]
    fn mule_rejects_seed_only_inventory() {
        let mut pair = mule_pair();
        pair.crafter.air_from_script = 0;
        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("seed-only"), "{error}");
    }

    #[test]
    fn mule_rejects_seeded_rune_products() {
        let mut pair = mule_pair();
        pair.crafter.baseline.air_runes = MULE_TRADE_CAP;
        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("baseline already held Air 556"), "{error}");
    }

    #[test]
    fn mule_rejects_essence_only_exchange_without_rune_transfer() {
        let mut pair = mule_pair();
        pair.mule.air_transferred_in = 0;
        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("crafted Air 556"), "{error}");
    }

    #[test]
    fn mule_rejects_missing_conservation() {
        let mut pair = mule_pair();
        pair.mule.transferred_out = 0;
        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("missing conservation"), "{error}");
    }

    #[test]
    fn mule_rejects_stale_trade() {
        let mut pair = mule_pair();
        pair.crafter.latest.as_mut().unwrap().trade_offer_open = true;
        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("stale trade"), "{error}");
    }

    #[test]
    fn mule_rejects_empty_partner() {
        assert!(mule_mode_requires_partner(MuleRole::Mule, "").is_err());
        assert!(mule_mode_requires_partner(MuleRole::Crafter, "").is_ok());
        let mut pair = mule_pair();
        pair.mule.partner.clear();
        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("fixture miss"), "{error}");
    }

    #[test]
    fn mule_rejects_both_crafter() {
        let mut pair = mule_pair();
        pair.mule.role = MuleRole::Crafter;
        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("both Crafter"), "{error}");
    }

    #[test]
    fn mule_rejects_full_cycle_without_mule_bank_deposit() {
        let pair = mule_pair();
        pair.qualify_supported().unwrap();
        let error = pair.qualify_full_cycle().unwrap_err();
        assert!(error.contains("no mule bank deposit"), "{error}");
    }

    #[test]
    fn mule_rejects_full_cycle_without_restock() {
        let mut pair = mule_pair();
        pair.mule.air_from_script = 27;
        pair.mule.deposited_received_air = true;
        pair.mule.saw_bank_open_loaded = true;
        pair.mule.saw_bank_at_falador = true;
        let error = pair.qualify_full_cycle().unwrap_err();
        assert!(error.contains("no actual bank restock"), "{error}");
    }

    #[test]
    fn mule_rejects_full_cycle_without_second_cycle() {
        let mut pair = mule_pair();
        pair.mule.air_from_script = 27;
        pair.mule.deposited_received_air = true;
        pair.mule.saw_bank_open_loaded = true;
        pair.mule.saw_bank_at_falador = true;
        pair.mule.restock_withdraw = true;
        pair.mule.returned_to_ruins = true;
        let error = pair.qualify_full_cycle().unwrap_err();
        assert!(error.contains("no further work"), "{error}");
    }

    #[test]
    fn mule_accepts_first_exchange_craft() {
        let pair = mule_pair();
        assert_eq!(
            pair.qualify_supported().unwrap(),
            MuleClaim::FirstExchangeCraft
        );
    }

    #[test]
    fn mule_rejects_full_cycle_without_post_second_exchange_craft() {
        let mut pair = mule_fresh_pair();
        observe_first_exchange(&mut pair);
        observe_second_exchange(&mut pair, false);
        pair.qualify_supported().unwrap();
        let error = pair.qualify_full_cycle().unwrap_err();
        assert!(error.contains("fresh crafter craft"), "{error}");
    }

    #[test]
    fn mule_accepts_full_cycle_after_second_transfer_and_fresh_craft() {
        let mut pair = mule_fresh_pair();
        observe_first_exchange(&mut pair);
        observe_second_exchange(&mut pair, true);
        assert_eq!(
            pair.qualify_full_cycle().unwrap(),
            MuleClaim::MuleBankReturnSecondCycle
        );
    }

    #[test]
    fn mule_rejects_trade_without_published_counterpart() {
        let mut pair = mule_fresh_pair();
        let mut seed_craft = air_obs("alice", 0, MULE_TRADE_CAP, 135);
        seed_craft.tile = Some((AIR_RUINS.0, 4800, AIR_RUINS.2));
        seed_craft.in_temple = true;
        pair.crafter.observe(seed_craft);
        pair.crafter.observe(without_trade_partner(with_trade(
            air_obs("alice", 0, 27, 135),
            "bob",
            true,
            false,
        )));
        pair.crafter.observe(without_trade_partner(with_trade(
            air_obs("alice", 0, 27, 135),
            "bob",
            false,
            true,
        )));
        pair.mule.observe(without_trade_partner(with_trade(
            air_obs("bob", 27, 0, 0),
            "alice",
            true,
            false,
        )));
        pair.mule.observe(without_trade_partner(with_trade(
            air_obs("bob", 27, 0, 0),
            "alice",
            false,
            true,
        )));
        pair.mule.observe(air_obs("bob", 0, 27, 0));
        pair.crafter.observe(air_obs("alice", 27, 0, 135));
        let mut post_exchange_craft = air_obs("alice", 0, 27, 270);
        post_exchange_craft.tile = Some((AIR_RUINS.0, 4800, AIR_RUINS.2));
        post_exchange_craft.in_temple = true;
        pair.crafter.observe(post_exchange_craft);

        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("one-sided confirmation"), "{error}");
    }

    #[test]
    fn mule_rejects_transfer_not_adjacent_to_confirmed_trade() {
        let mut pair = mule_fresh_pair();
        let mut seed_craft = air_obs("alice", 0, MULE_TRADE_CAP, 135);
        seed_craft.tile = Some((AIR_RUINS.0, 4800, AIR_RUINS.2));
        seed_craft.in_temple = true;
        pair.crafter.observe(seed_craft);
        pair.crafter
            .observe(with_trade(air_obs("alice", 0, 27, 135), "bob", true, false));
        pair.crafter
            .observe(with_trade(air_obs("alice", 0, 27, 135), "bob", false, true));
        pair.mule
            .observe(with_trade(air_obs("bob", 27, 0, 0), "alice", true, false));
        pair.mule
            .observe(with_trade(air_obs("bob", 27, 0, 0), "alice", false, true));

        pair.crafter.observe(air_obs("alice", 0, 27, 135));
        pair.mule.observe(air_obs("bob", 27, 0, 0));
        pair.mule.observe(air_obs("bob", 0, 27, 0));
        pair.crafter.observe(air_obs("alice", 27, 0, 135));
        let mut post_exchange_craft = air_obs("alice", 0, 27, 270);
        post_exchange_craft.tile = Some((AIR_RUINS.0, 4800, AIR_RUINS.2));
        post_exchange_craft.in_temple = true;
        pair.crafter.observe(post_exchange_craft);

        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("missing conservation"), "{error}");
    }

    #[test]
    fn mule_rejects_second_transfer_without_second_counterpart_handshake() {
        let mut pair = mule_fresh_pair();
        observe_first_exchange(&mut pair);

        let mut deposited = air_obs("bob", 0, 0, 0);
        deposited.tile = Some(FALADOR_EAST);
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        pair.mule.observe(deposited);
        let mut restocked = air_obs("bob", MULE_TRADE_CAP, 0, 0);
        restocked.tile = Some(FALADOR_EAST);
        restocked.bank_open = true;
        restocked.bank_loaded = true;
        pair.mule.observe(restocked);
        pair.mule.observe(air_obs("bob", MULE_TRADE_CAP, 0, 0));

        pair.mule.observe(air_obs("bob", 0, 27, 0));
        pair.crafter.observe(air_obs("alice", 27, 0, 270));
        let mut post_exchange_craft = air_obs("alice", 0, 27, 405);
        post_exchange_craft.tile = Some((AIR_RUINS.0, 4800, AIR_RUINS.2));
        post_exchange_craft.in_temple = true;
        pair.crafter.observe(post_exchange_craft);

        let error = pair.qualify_full_cycle().unwrap_err();
        assert!(error.contains("second transfer"), "{error}");
    }

    #[test]
    fn mule_rejects_bank_cycle_observed_after_second_transfer() {
        let mut pair = mule_fresh_pair();
        observe_first_exchange(&mut pair);

        let mut unobserved_restock = air_obs("bob", MULE_TRADE_CAP, 0, 0);
        unobserved_restock.tile = Some(FALADOR_EAST);
        pair.mule.observe(unobserved_restock);
        pair.mule.observe(air_obs("bob", MULE_TRADE_CAP, 0, 0));
        pair.crafter
            .observe(with_trade(air_obs("alice", 0, 27, 270), "bob", true, false));
        pair.crafter
            .observe(with_trade(air_obs("alice", 0, 27, 270), "bob", false, true));
        pair.mule
            .observe(with_trade(air_obs("bob", 27, 0, 0), "alice", true, false));
        pair.mule
            .observe(with_trade(air_obs("bob", 27, 0, 0), "alice", false, true));
        pair.mule.observe(air_obs("bob", 0, 27, 0));
        pair.crafter.observe(air_obs("alice", 27, 0, 270));
        let mut post_exchange_craft = air_obs("alice", 0, 27, 405);
        post_exchange_craft.tile = Some((AIR_RUINS.0, 4800, AIR_RUINS.2));
        post_exchange_craft.in_temple = true;
        pair.crafter.observe(post_exchange_craft);

        let mut deposited = air_obs("bob", 0, 0, 0);
        deposited.tile = Some(FALADOR_EAST);
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        pair.mule.observe(deposited);
        let mut restocked = air_obs("bob", MULE_TRADE_CAP, 0, 0);
        restocked.tile = Some(FALADOR_EAST);
        restocked.bank_open = true;
        restocked.bank_loaded = true;
        pair.mule.observe(restocked);
        pair.mule.observe(air_obs("bob", MULE_TRADE_CAP, 0, 0));

        let error = pair.qualify_full_cycle().unwrap_err();
        assert!(error.contains("second transfer"), "{error}");
    }

    #[test]
    fn mule_observe_crafter_bank_deposit_keeps_first_exchange_craft() {
        let mut pair = mule_fresh_pair();
        observe_first_exchange(&mut pair);
        let mut deposited = air_obs("alice", 0, 0, 270);
        deposited.tile = Some(FALADOR_EAST);
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        pair.crafter.observe(deposited);
        assert!(
            pair.crafter.air_from_script > 0,
            "sticky peak must survive crafter bank deposit of 556"
        );
        assert_eq!(pair.crafter.xp_from_script, 270);
        assert_eq!(pair.crafter.latest.as_ref().unwrap().air_runes, 0);
        assert_eq!(
            pair.qualify_supported().unwrap(),
            MuleClaim::FirstExchangeCraft
        );
    }

    #[test]
    fn mule_observe_deposit_keeps_held_air_for_full_cycle_gate() {
        let mut pair = mule_fresh_pair();
        observe_first_exchange(&mut pair);
        pair.mule.observe(air_obs("bob", 0, 27, 0));
        let mut deposited = air_obs("bob", 0, 0, 0);
        deposited.tile = Some(FALADOR_EAST);
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        pair.mule.observe(deposited);
        assert!(
            pair.mule.air_from_script > 0,
            "sticky peak must survive mule Falador East deposit of received 556"
        );
        assert!(pair.mule.deposited_received_air);
        pair.qualify_supported().unwrap();
        let error = pair.qualify_full_cycle().unwrap_err();
        assert!(
            error.contains("no actual bank restock"),
            "held-air/deposit must pass so restock can fail honestly: {error}"
        );
        assert!(!error.contains("never held script Air 556"), "{error}");
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
    fn serializes_complete_pair_witnesses() {
        let air = serde_json::to_value(air_pair()).unwrap();
        for key in ["baseline", "account", "settings", "partner"] {
            assert!(air["master"].get(key).is_some(), "missing master.{key}");
            assert!(air["runner"].get(key).is_some(), "missing runner.{key}");
        }
        let mule = serde_json::to_value(mule_pair()).unwrap();
        for key in ["baseline", "account", "settings", "partner"] {
            assert!(mule["crafter"].get(key).is_some(), "missing crafter.{key}");
            assert!(mule["mule"].get(key).is_some(), "missing mule.{key}");
        }
        for key in [
            "air_transferred_out",
            "partner_transfer_events",
            "post_exchange_craft_events",
        ] {
            assert!(mule["crafter"].get(key).is_some(), "missing crafter.{key}");
        }
        assert!(mule["mule"].get("air_transferred_in").is_some());
        let duel = serde_json::to_value(duel_pair()).unwrap();
        for slot in ["a", "b"] {
            assert!(duel[slot].get("baseline").is_some());
            assert!(duel[slot].get("account").is_some());
        }
    }

    #[test]
    fn stale_pre_logout_scene2_is_not_relog_admission() {
        assert_eq!(
            relog_admission(false, true, 2, true),
            RelogAdmission::WaitLogout
        );
        assert_eq!(
            relog_admission(false, false, 0, false),
            RelogAdmission::LoggedOut
        );
        assert_eq!(
            relog_admission(true, false, 0, false),
            RelogAdmission::WaitLogin
        );
        assert_eq!(relog_admission(true, true, 2, true), RelogAdmission::Ready);
        assert_ne!(relog_admission(false, true, 2, true), RelogAdmission::Ready);
    }

    #[test]
    fn bank_seed_is_not_acknowledgement_without_open_loaded_count_at_falador() {
        let mut ruins = air_obs("bob", 25, 0, 0);
        ruins.tile = Some(AIR_RUINS);
        ruins.bank_open = false;
        ruins.bank_loaded = false;
        ruins.bank_essence_unnoted = 0;
        assert!(!bank_seed_acknowledged(&ruins, BANK_SEED_ESSENCE));

        let mut queued = air_obs("bob", 0, 0, 0);
        queued.tile = Some(FALADOR_EAST);
        queued.bank_open = false;
        queued.bank_loaded = false;
        queued.bank_essence_unnoted = 0;
        assert!(!bank_seed_acknowledged(&queued, BANK_SEED_ESSENCE));

        let mut short = queued.clone();
        short.bank_open = true;
        short.bank_loaded = true;
        short.bank_essence_unnoted = 1;
        assert!(!bank_seed_acknowledged(&short, BANK_SEED_ESSENCE));

        let mut ruins_open = ruins.clone();
        ruins_open.bank_open = true;
        ruins_open.bank_loaded = true;
        ruins_open.bank_essence_unnoted = BANK_SEED_ESSENCE;
        assert!(!bank_seed_acknowledged(&ruins_open, BANK_SEED_ESSENCE));

        let mut ack = queued;
        ack.bank_open = true;
        ack.bank_loaded = true;
        ack.bank_essence_unnoted = BANK_SEED_ESSENCE;
        assert!(bank_seed_acknowledged(&ack, BANK_SEED_ESSENCE));
    }

    #[test]
    fn missing_booth_at_ruins_reports_target_absence() {
        let absence = bank_ack_target_absence(Some(AIR_RUINS), false).unwrap();
        assert!(absence.contains("Falador East"), "{absence}");
        assert!(absence.contains("2983"), "{absence}");
        assert!(bank_ack_target_absence(Some(FALADOR_EAST), false)
            .unwrap()
            .contains("Use-quickly"));
        assert!(bank_ack_target_absence(Some(FALADOR_EAST), true).is_none());
    }

    #[test]
    fn mismatched_readiness_does_not_start() {
        assert_eq!(
            shared_start_barrier(StartBarrierInput {
                a_prepared: true,
                b_prepared: false,
                a_started: false,
                b_started: false,
                a_wait_ack: false,
                b_wait_ack: true,
                a_current_ok: true,
                b_current_ok: false,
            }),
            StartBarrier::Wait
        );
        assert_eq!(
            shared_start_barrier(StartBarrierInput {
                a_prepared: true,
                b_prepared: true,
                a_started: false,
                b_started: false,
                a_wait_ack: false,
                b_wait_ack: true,
                a_current_ok: true,
                b_current_ok: true,
            }),
            StartBarrier::Wait
        );
        assert_eq!(
            shared_start_barrier(StartBarrierInput {
                a_prepared: true,
                b_prepared: true,
                a_started: false,
                b_started: false,
                a_wait_ack: false,
                b_wait_ack: false,
                a_current_ok: true,
                b_current_ok: false,
            }),
            StartBarrier::Wait
        );
        assert_eq!(
            shared_start_barrier(StartBarrierInput {
                a_prepared: true,
                b_prepared: false,
                a_started: false,
                b_started: false,
                a_wait_ack: false,
                b_wait_ack: false,
                a_current_ok: true,
                b_current_ok: false,
            }),
            StartBarrier::Wait
        );
    }

    #[test]
    fn one_barrier_starts_both_only_when_prepared() {
        assert_eq!(
            shared_start_barrier(StartBarrierInput {
                a_prepared: true,
                b_prepared: true,
                a_started: false,
                b_started: false,
                a_wait_ack: false,
                b_wait_ack: false,
                a_current_ok: true,
                b_current_ok: true,
            }),
            StartBarrier::StartBoth
        );
        assert_eq!(
            shared_start_barrier(StartBarrierInput {
                a_prepared: true,
                b_prepared: true,
                a_started: true,
                b_started: false,
                a_wait_ack: false,
                b_wait_ack: false,
                a_current_ok: true,
                b_current_ok: true,
            }),
            StartBarrier::RejectStartedWhileUnready
        );
        assert_eq!(
            shared_start_barrier(StartBarrierInput {
                a_prepared: true,
                b_prepared: false,
                a_started: true,
                b_started: false,
                a_wait_ack: false,
                b_wait_ack: true,
                a_current_ok: true,
                b_current_ok: false,
            }),
            StartBarrier::RejectStartedWhileUnready
        );
        assert_eq!(
            shared_start_barrier(StartBarrierInput {
                a_prepared: true,
                b_prepared: true,
                a_started: true,
                b_started: true,
                a_wait_ack: false,
                b_wait_ack: false,
                a_current_ok: true,
                b_current_ok: true,
            }),
            StartBarrier::Wait
        );
    }

    #[test]
    fn air_prepared_current_rejects_master_essence_and_runner_short_load() {
        let master = air_obs("alice", 0, 0, 0);
        air_prepared_current(AirRole::Master, "alice", &master).unwrap();
        let mut with_essence = master.clone();
        with_essence.essence_unnoted = 1;
        assert!(air_prepared_current(AirRole::Master, "alice", &with_essence).is_err());

        let runner = air_obs("bob", 25, 0, 0);
        air_prepared_current(AirRole::Runner, "bob", &runner).unwrap();
        let mut short = runner.clone();
        short.essence_unnoted = 1;
        assert!(air_prepared_current(AirRole::Runner, "bob", &short).is_err());
        let mut at_bank = runner;
        at_bank.tile = Some(FALADOR_EAST);
        assert!(air_prepared_current(AirRole::Runner, "bob", &at_bank).is_err());
    }

    #[test]
    fn mule_prepared_current_requires_exact_raw_loads_and_crafter_talisman() {
        let crafter = air_obs("alice", MULE_TRADE_CAP, 0, 0);
        mule_prepared_current(MuleRole::Crafter, "alice", &crafter).unwrap();
        let mut empty = crafter.clone();
        empty.essence_unnoted = 0;
        assert!(mule_prepared_current(MuleRole::Crafter, "alice", &empty).is_err());
        let mut surplus = crafter.clone();
        surplus.essence_unnoted = MULE_TRADE_CAP + 1;
        assert!(mule_prepared_current(MuleRole::Crafter, "alice", &surplus).is_err());

        let mule = air_obs("bob", 27, 0, 0);
        mule_prepared_current(MuleRole::Mule, "bob", &mule).unwrap();
        let mut short = mule.clone();
        short.essence_unnoted = 1;
        assert!(mule_prepared_current(MuleRole::Mule, "bob", &short).is_err());
        let mut with_talisman = mule.clone();
        with_talisman.air_talisman = 1;
        assert!(mule_prepared_current(MuleRole::Mule, "bob", &with_talisman).is_err());
        let mut mule_surplus = mule.clone();
        mule_surplus.essence_unnoted = MULE_TRADE_CAP + 1;
        assert!(mule_prepared_current(MuleRole::Mule, "bob", &mule_surplus).is_err());
        let mut at_bank = mule;
        at_bank.tile = Some(FALADOR_EAST);
        assert!(mule_prepared_current(MuleRole::Mule, "bob", &at_bank).is_err());
    }

    #[test]
    fn mule_rejects_seed_craft_and_empty_exchange() {
        let mut pair = mule_fresh_pair();
        pair.crafter
            .observe(air_obs("alice", 0, MULE_TRADE_CAP, 135));
        pair.crafter.observe(with_trade(
            air_obs("alice", 0, MULE_TRADE_CAP, 135),
            "bob",
            true,
            false,
        ));
        pair.crafter.observe(with_trade(
            air_obs("alice", 0, MULE_TRADE_CAP, 135),
            "bob",
            false,
            true,
        ));
        pair.mule.observe(with_trade(
            air_obs("bob", MULE_TRADE_CAP, 0, 0),
            "alice",
            true,
            false,
        ));
        pair.mule.observe(with_trade(
            air_obs("bob", MULE_TRADE_CAP, 0, 0),
            "alice",
            false,
            true,
        ));
        pair.mule.observe(air_obs("bob", 0, 0, 0));
        pair.crafter
            .observe(air_obs("alice", 0, MULE_TRADE_CAP, 135));

        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("missing conservation"), "{error}");
    }

    #[test]
    fn mule_rejects_exchange_without_fresh_post_exchange_craft() {
        let mut pair = mule_fresh_pair();
        pair.crafter
            .observe(air_obs("alice", 0, MULE_TRADE_CAP, 135));
        pair.crafter.observe(with_trade(
            air_obs("alice", 0, MULE_TRADE_CAP, 135),
            "bob",
            true,
            false,
        ));
        pair.crafter.observe(with_trade(
            air_obs("alice", 0, MULE_TRADE_CAP, 135),
            "bob",
            false,
            true,
        ));
        pair.mule.observe(with_trade(
            air_obs("bob", MULE_TRADE_CAP, 0, 0),
            "alice",
            true,
            false,
        ));
        pair.mule.observe(with_trade(
            air_obs("bob", MULE_TRADE_CAP, 0, 0),
            "alice",
            false,
            true,
        ));
        pair.crafter
            .observe(air_obs("alice", MULE_TRADE_CAP, 0, 135));
        pair.mule.observe(air_obs("bob", 0, MULE_TRADE_CAP, 0));

        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("post-exchange"), "{error}");
    }

    fn flax_obs(player: &str, flax: i32, string: i32, xp: i32) -> FlaxObservation {
        FlaxObservation {
            ingame: true,
            scene_state: 2,
            inventory_tab_available: true,
            player: Some(player.into()),
            tile: Some(FLAX_MEET),
            crafting: 1,
            crafting_xp: xp,
            flax,
            bow_string: string,
            ..FlaxObservation::default()
        }
    }

    fn flax_pair() -> FlaxPairWitness {
        let runner_base = FlaxObservation {
            tile: Some(FLAX_FIELD),
            ..flax_obs("runner", 0, 0, 0)
        };
        let spinner_base = flax_obs("spinner", 0, 0, 0);
        flax_prepared_current(FlaxRole::Runner, "runner", &runner_base).unwrap();
        flax_prepared_current(FlaxRole::Spinner, "spinner", &spinner_base).unwrap();
        let mut pair = FlaxPairWitness {
            runner: FlaxSlotRecord::new(
                FlaxRole::Runner,
                "runner".into(),
                "runner".into(),
                "spinner".into(),
                Default::default(),
                runner_base,
            ),
            spinner: FlaxSlotRecord::new(
                FlaxRole::Spinner,
                "spinner".into(),
                "spinner".into(),
                "runner".into(),
                Default::default(),
                spinner_base,
            ),
        };
        pair.runner.observe(with_flax_trade(
            flax_obs("runner", 24, 0, 0),
            "spinner",
            true,
            false,
        ));
        pair.runner.observe(with_flax_trade(
            flax_obs("runner", 24, 0, 0),
            "spinner",
            false,
            true,
        ));
        pair.spinner.observe(with_flax_trade(
            flax_obs("spinner", 0, 0, 0),
            "runner",
            true,
            false,
        ));
        pair.spinner.observe(with_flax_trade(
            flax_obs("spinner", 0, 0, 0),
            "runner",
            false,
            true,
        ));
        pair.runner.observe(flax_obs("runner", 0, 0, 0));
        pair.spinner.observe(flax_obs("spinner", 24, 0, 0));
        pair
    }

    fn with_flax_trade(
        mut observation: FlaxObservation,
        partner: &str,
        offer: bool,
        confirm: bool,
    ) -> FlaxObservation {
        observation.trade_offer_open = offer;
        observation.trade_confirm_open = confirm;
        observation.trade_partner = Some(partner.into());
        observation
    }

    #[test]
    fn flax_rejects_wrong_partner() {
        let mut pair = flax_pair();
        pair.runner.saw_wrong_partner = true;
        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("wrong partner"), "{error}");
    }

    #[test]
    fn flax_rejects_one_sided_confirmation() {
        let mut pair = flax_pair();
        pair.spinner.saw_confirm_with_partner = false;
        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("one-sided"), "{error}");
    }

    #[test]
    fn flax_rejects_seed_only_strings() {
        let mut pair = flax_pair();
        pair.spinner.baseline.bow_string = 24;
        let error = pair.qualify_supported().unwrap_err();
        assert!(error.contains("seed-only"), "{error}");
    }

    #[test]
    fn flax_rejects_first_transfer_as_full_cycle() {
        let pair = flax_pair();
        assert_eq!(
            pair.qualify_supported().unwrap(),
            FlaxClaim::FirstFlaxTransfer
        );
        let error = pair.qualify_full_cycle().unwrap_err();
        assert!(
            error.contains("no further work") || error.contains("seed-only"),
            "{error}"
        );
    }

    #[test]
    fn flax_rejects_missing_second_production() {
        let mut pair = flax_pair();
        pair.spinner.observe(FlaxObservation {
            flax: 0,
            bow_string: 24,
            crafting_xp: 15,
            ..flax_obs("spinner", 0, 24, 15)
        });
        let mut bank = flax_obs("spinner", 0, 0, 15);
        bank.tile = Some(FLAX_BANK);
        bank.bank_open = true;
        bank.bank_loaded = true;
        pair.spinner.observe(bank);
        pair.spinner.observe(flax_obs("spinner", 0, 0, 15));
        let error = pair.qualify_full_cycle().unwrap_err();
        assert!(
            error.contains("no further work") || error.contains("second"),
            "{error}"
        );
    }
}
