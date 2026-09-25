//! Shared full-core catalog witness used by headless and headed proof paths.
//!
//! This module intentionally contains the exact compact observations, Start
//! predicates, cycle observers and qualification logic from the original
//! `catalog_boundary_live` harness. It is proof infrastructure, not gameplay.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use api::line_of_sight::{line_of_sight_v2, CollisionQuery};
use api::obj_names::ObjNames;
use api::snapshot::{ActorKind, GameSnapshot, LocView, NpcView, SceneView, WorldTile};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[path = "catalog_core_ranging.rs"]
mod ranging;
pub use ranging::{
    ranging_guild_bank_baseline_ready, ranging_guild_full_baseline_ready,
    ranging_guild_redeem_baseline_ready, ranging_guild_round_baseline_ready, RangingGuildBankCycle,
    RangingGuildFullCycle, RangingGuildRedeemCycle, RangingGuildRoundCycle, ARCHERY_TICKET_ID,
    COINS_PER_TRIP, ENTRY_FEE, RANGED_LIVE, RANGING_GUILD_FULL_MINUTES,
    RANGING_GUILD_FULL_STOP_NEEDLE, RANGING_GUILD_MERCHANT_STAND, RANGING_GUILD_SEERS_BANK,
    RANGING_GUILD_STAND, RUNE_ARROWS_PER_TRADE, SEED_KEEP_TICKETS, SEERS_BANK_RADIUS,
    TARGET_RESULT_MODAL, TICKETS_PER_TRADE, VARP_TARGET_COUNT, VARP_TARGET_HIT, VARP_TARGET_SCORE,
};
#[path = "catalog_core_hunt.rs"]
mod hunt;
pub use hunt::{
    hunt_baseline_ready, leave_lair_box, parse_hunt_receipt_line, HuntBox, HuntCell,
    HuntDeliveryCycle, HuntObservation, HuntOutcome, HuntReceipt, ScriptAct, ScriptActLedger,
    ScriptActRow, ScriptActsPublished, ACQUIRE_KEY_DEST, ACQUIRE_KEY_RADIUS,
    ACQUIRE_KEY_RECEIPT_PREFIX, ACQUIRE_KEY_V2_STOP, BANK_V2_DEST, BANK_V2_RADIUS,
    BANK_V2_RECEIPT_PREFIX, BANK_V2_STOP, CELL_V2_DOOR, CELL_V2_RADIUS, CELL_V2_RECEIPT_PREFIX,
    CELL_V2_STOP, DUSTY_KEY_ID, ENTER_LAIR_MAX_CHEB, ENTER_LAIR_MIN_CHEB,
    ENTER_LAIR_RECEIPT_PREFIX, ENTER_LAIR_V2_STOP, HOLD_SPOT_MAX_CHEB, HOLD_SPOT_MIN_CHEB,
    HOLD_SPOT_RECEIPT_PREFIX, HOLD_SPOT_V2_STOP, HUNT_ACT_ROWS, HUNT_LAIR_FIXTURE, JAILER,
    JAIL_CELL, JAIL_DOOR, JAIL_DOOR_LOC, JAIL_KEY_ID, LEAVE_LAIR_BOX_PAD, LEAVE_LAIR_MAX_CHEB,
    LEAVE_LAIR_MIN_CHEB, LEAVE_LAIR_RADIUS, LEAVE_LAIR_RECEIPT_PREFIX, LEAVE_LAIR_V2_STOP,
    RETREAT_SPOT_MAX_CHEB, RETREAT_SPOT_MIN_CHEB, RETREAT_SPOT_RECEIPT_PREFIX,
    RETREAT_SPOT_V2_STOP, VELRAK, WALK_SPOT_MAX_CHEB, WALK_SPOT_MIN_CHEB, WALK_SPOT_RECEIPT_PREFIX,
    WALK_SPOT_V2_STOP,
};

#[path = "catalog_ids.rs"]
mod ids;
pub use ids::*;
#[path = "catalog_case.rs"]
mod case;
pub use case::*;

#[path = "catalog_observation.rs"]
mod observation;
pub use observation::*;
use observation::empty_worn;
#[path = "catalog_inspect.rs"]
mod inspect;
pub use inspect::*;
#[path = "catalog_prayer.rs"]
mod prayer;
pub use prayer::*;
#[path = "catalog_los.rs"]
mod los;
pub use los::*;
#[path = "catalog_actor.rs"]
mod actor;
pub use actor::*;
#[path = "catalog_fight_field.rs"]
mod fight_field;
pub use fight_field::*;
#[path = "catalog_bone.rs"]
mod bone;
pub use bone::*;
#[path = "catalog_fletching.rs"]
mod fletching;
pub use fletching::*;
#[path = "catalog_alchemy.rs"]
mod alchemy;
pub use alchemy::*;
#[path = "catalog_herbs.rs"]
mod herbs;
pub use herbs::*;
#[path = "catalog_gems.rs"]
mod gems;
pub use gems::*;
#[path = "catalog_doors.rs"]
mod doors;
pub use doors::*;
#[path = "catalog_agility.rs"]
mod agility;
pub use agility::*;
#[path = "catalog_flax.rs"]
mod flax;
pub use flax::*;
#[path = "catalog_combat.rs"]
mod combat;
pub use combat::*;
#[path = "catalog_superheat.rs"]
mod superheat;
pub use superheat::*;
#[path = "catalog_potions.rs"]
mod potions;
pub use potions::*;
#[path = "catalog_tanning.rs"]
mod tanning;
pub use tanning::*;
#[path = "catalog_runecraft.rs"]
mod runecraft;
pub use runecraft::*;
#[path = "catalog_thieving.rs"]
mod thieving;
pub use thieving::*;
#[path = "catalog_gnome.rs"]
mod gnome;
pub use gnome::*;
#[path = "catalog_coal.rs"]
mod coal;
pub use coal::*;
#[path = "catalog_station.rs"]
mod station;
pub use station::*;
#[path = "catalog_teleport.rs"]
mod teleport;
pub use teleport::*;
#[path = "catalog_shop.rs"]
mod shop;
pub use shop::*;
#[path = "catalog_boots.rs"]
mod boots;
pub use boots::*;
#[path = "catalog_smithing.rs"]
mod smithing;
pub use smithing::*;
#[path = "catalog_leather.rs"]
mod leather;
pub use leather::*;
#[path = "catalog_firemaker.rs"]
mod firemaker;
pub use firemaker::*;
#[path = "catalog_baseline.rs"]
mod baseline;
pub use baseline::*;
#[path = "catalog_witness.rs"]
mod witness;
pub use witness::*;










































/// Opt-in bridge from the production slot snapshot publisher to the catalog
/// core witness. Disabled by default; ordinary Play/panel runs pay one mutex
/// check and never build an [`Observation`].
#[derive(Clone, Default)]
pub struct CoreWatch {
    active: Arc<AtomicBool>,
    inner: Arc<Mutex<CoreWatchState>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreWatchStatus {
    Disabled,
    Ready,
    Running,
    Qualified,
    Failed,
}

#[derive(Debug, Default)]
enum CoreWatchState {
    #[default]
    Disabled,
    Ready {
        case: CoreCase,
        account: String,
        latest: Option<Observation>,
        start_preparation: HerbCleanerStartPreparation,
    },
    Running {
        account: String,
        witness: Box<CoreWitness>,
    },
    /// Immutable bounded receipt cached at the first terminal qualification.
    Qualified {
        account: String,
        evidence: Arc<Value>,
    },
    Failed {
        case: CoreCase,
        account: String,
        error: String,
        latest: Option<Observation>,
        witness: Option<Box<CoreWitness>>,
    },
}

impl CoreWatch {
    /// Replace any previous run with one ready to collect its pre-Start baseline.
    pub fn configure(&self, case: CoreCase, account: impl Into<String>) {
        *self.inner.lock().unwrap() = CoreWatchState::Ready {
            case,
            account: account.into(),
            latest: None,
            start_preparation: HerbCleanerStartPreparation::default(),
        };
        self.active.store(true, Ordering::Release);
    }

    /// Disable and drop all bounded witness state.
    pub fn clear(&self) {
        self.active.store(false, Ordering::Release);
        *self.inner.lock().unwrap() = CoreWatchState::Disabled;
    }

    /// Mark the exact instant immediately before the catalog isolate is started.
    /// The most recent observation from the production publisher becomes the
    /// immutable Start baseline; a later snapshot may already contain script work.
    pub fn begin_start(&self, account: &str) -> Result<(), String> {
        let mut state = self.inner.lock().unwrap();
        let current = std::mem::take(&mut *state);
        match current {
            CoreWatchState::Disabled => {
                *state = CoreWatchState::Disabled;
                Ok(())
            }
            CoreWatchState::Ready {
                case,
                account: expected,
                latest,
                start_preparation,
            } if expected == account => match latest {
                None => {
                    *state = CoreWatchState::Ready {
                        case,
                        account: expected,
                        latest: None,
                        start_preparation,
                    };
                    Err("catalog core has no published pre-Start observation".into())
                }
                Some(observation) => {
                    let preparation = if case == CoreCase::HerbCleanerEmptyBank {
                        start_preparation.receipt_for(&expected, &observation)
                    } else {
                        None
                    };
                    match validate_start_baseline(
                        case,
                        &expected,
                        &observation,
                        preparation.as_ref(),
                    ) {
                        Ok(()) => {
                            *state = CoreWatchState::Running {
                                account: expected,
                                witness: Box::new(CoreWitness::new_with_start_preparation(
                                    case,
                                    observation,
                                    preparation,
                                )),
                            };
                            Ok(())
                        }
                        Err(error) => {
                            *state = CoreWatchState::Failed {
                                case,
                                account: expected,
                                error: error.clone(),
                                latest: Some(observation),
                                witness: None,
                            };
                            Err(error)
                        }
                    }
                }
            },
            CoreWatchState::Ready {
                case,
                account: expected,
                latest,
                start_preparation,
            } => {
                let error = format!(
                    "catalog core Start slot {account:?} is not configured account {expected:?}"
                );
                *state = CoreWatchState::Ready {
                    case,
                    account: expected,
                    latest,
                    start_preparation,
                };
                Err(error)
            }
            other => {
                *state = other;
                Err("catalog core Start was armed more than once".into())
            }
        }
    }

    /// Preserve an isolate-start refusal as a terminal core failure.
    pub fn fail_start(&self, account: &str, error: impl Into<String>) {
        let mut state = self.inner.lock().unwrap();
        let current = std::mem::take(&mut *state);
        *state = match current {
            CoreWatchState::Running {
                account: expected,
                witness,
            } if expected == account => CoreWatchState::Failed {
                case: witness.case,
                account: expected,
                error: error.into(),
                latest: None,
                witness: Some(witness),
            },
            other => other,
        };
    }

    /// Observe one compact record made from the already-published slot snapshot.
    /// A session boundary after Start is terminal: relogged/stale rows cannot be
    /// combined with the original baseline to manufacture progress.
    pub fn observe(&self, account: &str, observation: Observation, session_boundary: bool) {
        let mut state = self.inner.lock().unwrap();
        Self::observe_locked(&mut state, account, observation, session_boundary);
    }

    fn observe_locked(
        state: &mut CoreWatchState,
        account: &str,
        observation: Observation,
        session_boundary: bool,
    ) {
        let current = std::mem::take(&mut *state);
        *state = match current {
            CoreWatchState::Ready {
                case,
                account: expected,
                latest: _,
                mut start_preparation,
            } if expected == account => {
                if session_boundary {
                    CoreWatchState::Ready {
                        case,
                        account: expected,
                        latest: None,
                        start_preparation: HerbCleanerStartPreparation::default(),
                    }
                } else {
                    if case == CoreCase::HerbCleanerEmptyBank {
                        start_preparation.observe(&expected, &observation);
                    }
                    CoreWatchState::Ready {
                        case,
                        account: expected,
                        latest: Some(observation),
                        start_preparation,
                    }
                }
            }
            CoreWatchState::Running {
                account: expected,
                mut witness,
            } if expected == account => {
                if session_boundary {
                    CoreWatchState::Failed {
                        case: witness.case,
                        account: expected,
                        error: "catalog core session boundary after Start".into(),
                        latest: Some(observation),
                        witness: Some(witness),
                    }
                } else {
                    witness.observe(&observation);
                    if let Some(error) = witness.swarm_stopped_without_recovery(&observation) {
                        CoreWatchState::Failed {
                            case: witness.case,
                            account: expected,
                            error,
                            latest: Some(observation),
                            witness: Some(witness),
                        }
                    } else {
                        CoreWatchState::Running {
                            account: expected,
                            witness,
                        }
                    }
                }
            }
            other => other,
        };
    }

    /// Convert only for the configured slot and only after Start has been armed.
    pub fn observe_snapshot(
        &self,
        account: &str,
        snapshot: &GameSnapshot,
        names: &ObjNames,
        session_boundary: bool,
    ) {
        self.observe_snapshot_with_lifecycle(
            account,
            snapshot,
            names,
            None,
            BoundedGuardian::default(),
            session_boundary,
            None,
            None,
            None,
        );
    }

    /// Convert the published snapshot and attach the slot's bounded native
    /// lifecycle receipt, prior-frame guardian fact and (hunt cards only)
    /// the slot's act ledger without touching the panel's pending-log
    /// consumer.
    #[allow(clippy::too_many_arguments)] // watch observe packs account/snapshot/lifecycle/paint fields
    pub fn observe_snapshot_with_lifecycle(
        &self,
        account: &str,
        snapshot: &GameSnapshot,
        names: &ObjNames,
        lifecycle: Option<script::ScriptLifecycleReceipt>,
        guardian: BoundedGuardian,
        session_boundary: bool,
        inspect: Option<RouteInspectPublished>,
        paint: Option<&script::shim::ScriptPaint>,
        acts: Option<ScriptActsPublished>,
    ) {
        if !self.active.load(Ordering::Acquire) {
            return;
        }
        let mut state = self.inner.lock().unwrap();
        if matches!(
            &*state,
            CoreWatchState::Ready { account: expected, .. }
                | CoreWatchState::Running { account: expected, .. }
                if expected == account
        ) {
            // Convert while the lifecycle lock is held so a reconfiguration
            // cannot attach this snapshot to a later run of the same account.
            let copies_prayer = match &*state {
                CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                    case.copies_prayer_varps()
                }
                CoreWatchState::Running { witness, .. } => witness.case.copies_prayer_varps(),
                CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
            };
            let copies_los = match &*state {
                CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                    case.copies_line_of_sight()
                }
                CoreWatchState::Running { witness, .. } => witness.case.copies_line_of_sight(),
                CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
            };
            let copies_actor = match &*state {
                CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                    case.copies_actor_observation()
                }
                CoreWatchState::Running { witness, .. } => witness.case.copies_actor_observation(),
                CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
            };
            let copies_fight = match &*state {
                CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                    case.copies_fight_field()
                }
                CoreWatchState::Running { witness, .. } => witness.case.copies_fight_field(),
                CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
            };
            let hunt = match &*state {
                CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                    case.hunt_cell()
                }
                CoreWatchState::Running { witness, .. } => witness.case.hunt_cell(),
                CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => None,
            };
            let mut observation = Observation::from_snapshot(snapshot, names);
            observation.script_lifecycle = lifecycle;
            observation.guardian = guardian;
            if let Some(published) = inspect {
                observation.attach_route_inspect(published);
            }
            if copies_prayer {
                observation.attach_prayer_varps(snapshot);
            }
            if copies_los {
                observation.attach_line_of_sight(snapshot, paint);
            }
            if copies_actor {
                observation.attach_actor_observation(snapshot, paint);
            }
            if copies_fight {
                observation.attach_fight_field(snapshot, paint);
            }
            if let Some(cell) = hunt {
                observation.attach_hunt(cell, snapshot, paint, acts);
            }
            Self::observe_locked(&mut state, account, observation, session_boundary);
        }
    }

    pub fn configured(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    /// True only for the active inspect cards. Callers copy published hops
    /// only then; other cases keep the empty default.
    pub fn copies_route_inspect(&self) -> bool {
        if !self.active.load(Ordering::Acquire) {
            return false;
        }
        match &*self.inner.lock().unwrap() {
            CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                case.copies_route_inspect()
            }
            CoreWatchState::Running { witness, .. } => witness.case.copies_route_inspect(),
            CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
        }
    }

    /// True only for the line-of-sight File card. Callers attach paint and
    /// compact collision identity only then.
    pub fn copies_line_of_sight(&self) -> bool {
        if !self.active.load(Ordering::Acquire) {
            return false;
        }
        match &*self.inner.lock().unwrap() {
            CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                case.copies_line_of_sight()
            }
            CoreWatchState::Running { witness, .. } => witness.case.copies_line_of_sight(),
            CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
        }
    }

    /// True only for the actor-observation File card. Callers attach paint and
    /// one packed NPC row only then.
    pub fn copies_actor_observation(&self) -> bool {
        if !self.active.load(Ordering::Acquire) {
            return false;
        }
        match &*self.inner.lock().unwrap() {
            CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                case.copies_actor_observation()
            }
            CoreWatchState::Running { witness, .. } => witness.case.copies_actor_observation(),
            CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
        }
    }

    /// True only for the fight-field File card. Callers attach paint and
    /// one packed NPC row only then.
    pub fn copies_fight_field(&self) -> bool {
        if !self.active.load(Ordering::Acquire) {
            return false;
        }
        match &*self.inner.lock().unwrap() {
            CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                case.copies_fight_field()
            }
            CoreWatchState::Running { witness, .. } => witness.case.copies_fight_field(),
            CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
        }
    }

    /// True only for the v2 hunt File cards. Callers attach paint and the
    /// slot's act ledger only then.
    pub fn copies_hunt(&self) -> bool {
        if !self.active.load(Ordering::Acquire) {
            return false;
        }
        match &*self.inner.lock().unwrap() {
            CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                case.hunt_cell().is_some()
            }
            CoreWatchState::Running { witness, .. } => witness.case.hunt_cell().is_some(),
            CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
        }
    }

    /// Allocation-free terminal polling for the headed UI. The full receipt
    /// remains behind [`CoreWatch::qualify`] and is built once on terminal I/O.
    pub fn status(&self) -> CoreWatchStatus {
        if !self.active.load(Ordering::Acquire) {
            return CoreWatchStatus::Disabled;
        }
        let mut state = self.inner.lock().unwrap();
        let should_cache = matches!(
            &*state,
            CoreWatchState::Running { witness, .. } if witness.qualified()
        );
        if should_cache {
            let current = std::mem::take(&mut *state);
            if let CoreWatchState::Running { account, witness } = current {
                let evidence = Arc::new(
                    witness
                        .qualify()
                        .expect("qualified predicate must produce a receipt"),
                );
                *state = CoreWatchState::Qualified { account, evidence };
            }
        }
        match &*state {
            CoreWatchState::Disabled => CoreWatchStatus::Disabled,
            CoreWatchState::Ready { .. } => CoreWatchStatus::Ready,
            CoreWatchState::Running { .. } => CoreWatchStatus::Running,
            CoreWatchState::Qualified { .. } => CoreWatchStatus::Qualified,
            CoreWatchState::Failed { .. } => CoreWatchStatus::Failed,
        }
    }

    pub fn failure(&self) -> Option<String> {
        match &*self.inner.lock().unwrap() {
            CoreWatchState::Failed { error, .. } => Some(error.clone()),
            _ => None,
        }
    }

    /// Return full core evidence only after the unchanged witness qualifies.
    pub fn qualify(&self) -> Result<Arc<Value>, String> {
        let mut state = self.inner.lock().unwrap();
        let current = std::mem::take(&mut *state);
        match current {
            CoreWatchState::Running { account, witness } => match witness.qualify() {
                Ok(value) => {
                    let evidence = Arc::new(value);
                    *state = CoreWatchState::Qualified {
                        account,
                        evidence: Arc::clone(&evidence),
                    };
                    Ok(evidence)
                }
                Err(error) => {
                    *state = CoreWatchState::Running { account, witness };
                    Err(error)
                }
            },
            CoreWatchState::Qualified { account, evidence } => {
                let receipt = Arc::clone(&evidence);
                *state = CoreWatchState::Qualified { account, evidence };
                Ok(receipt)
            }
            failed @ CoreWatchState::Failed { .. } => {
                let error = match &failed {
                    CoreWatchState::Failed { error, .. } => error.clone(),
                    _ => unreachable!(),
                };
                *state = failed;
                Err(error)
            }
            ready @ CoreWatchState::Ready { .. } => {
                *state = ready;
                Err("catalog core Start not observed".into())
            }
            CoreWatchState::Disabled => {
                *state = CoreWatchState::Disabled;
                Err("catalog core watch disabled".into())
            }
        }
    }

    /// Compact failure/timeout receipt; never serializes the world snapshot.
    pub fn evidence(&self) -> Value {
        match &*self.inner.lock().unwrap() {
            CoreWatchState::Disabled => json!({"phase": "disabled"}),
            CoreWatchState::Ready {
                case,
                account,
                latest,
                start_preparation,
            } => json!({
                "phase": "ready",
                "case": case,
                "account": account,
                "latest": latest,
                "start_preparation": start_preparation,
            }),
            CoreWatchState::Running { account, witness } => json!({
                "phase": "running",
                "account": account,
                "witness": witness,
                "qualification": witness.qualify().err(),
            }),
            CoreWatchState::Qualified { account, evidence } => json!({
                "phase": "qualified",
                "account": account,
                "receipt": evidence,
            }),
            CoreWatchState::Failed {
                case,
                account,
                error,
                latest,
                witness,
            } => json!({
                "phase": "failed",
                "case": case,
                "account": account,
                "error": error,
                "latest": latest,
                "witness": witness,
            }),
        }
    }
}

fn validate_start_baseline(
    case: CoreCase,
    account: &str,
    observation: &Observation,
    start_preparation: Option<&HerbCleanerStartPreparationReceipt>,
) -> Result<(), String> {
    if !observation.ingame || observation.scene_state != 2 {
        return Err(format!(
            "Start baseline is not attached ingame scene2: {observation:?}"
        ));
    }
    let player = observation
        .player
        .as_deref()
        .ok_or_else(|| "Start baseline has no local player".to_string())?;
    let expected = client::util::jstring::JString::to_screen_name(account);
    if !player.eq_ignore_ascii_case(&expected) {
        return Err(format!(
            "Start baseline player {player:?} is not fresh account {account:?}"
        ));
    }
    if case == CoreCase::HerbCleanerEmptyBank && start_preparation.is_none() {
        return Err(format!(
            "{} Start baseline lacks this run's loaded-bank preparation receipt before closed-bank Start: {observation:?}",
            case.scenario_name()
        ));
    }
    validate_case_baseline_with_preparation(case, observation, start_preparation)
}

#[cfg(test)]
#[path = "catalog_actor_tests.rs"]
mod actor_observation_selection_tests;

#[cfg(test)]
#[path = "catalog_fight_field_tests.rs"]
mod fight_field_selection_tests;
