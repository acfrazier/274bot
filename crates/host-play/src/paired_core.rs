//! Shared paired full-cycle witness used by the existing paired harness and
//! the opt-in headed pair-watch bridge.
//!
//! This is proof infrastructure, not a second gameplay engine. Headed pair
//! cells are Air, Mule, Flax, and Duel. Duel counterpart identity is native
//! witness-owned; Duel settings carry schema target stats and have no partner
//! key.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use api::game_data::{DuelControls, SelectedGameData};
use api::snapshot::{GameSnapshot, ItemView, WidgetView};
use client::util::JString;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
#[path = "pair_case.rs"]
mod case;
pub use case::*;
#[path = "pair_runecraft.rs"]
mod runecraft;
pub use runecraft::*;
#[path = "pair_air.rs"]
mod air;
pub use air::*;
#[path = "pair_mule.rs"]
mod mule;
pub use mule::*;
#[path = "pair_flax.rs"]
mod flax;
pub use flax::*;
#[path = "pair_duel.rs"]
mod duel;
pub use duel::*;






































pub fn pair_settings(
    case: PairCase,
    schema: &[script::SettingDef],
    slot_index: usize,
    _self_name: &str,
    partner: &str,
) -> Result<Map<String, Value>, String> {
    let partner = JString::to_screen_name(partner);
    let partner = partner.as_str();
    match (case, slot_index) {
        (PairCase::Air, 0) => Ok(air_settings(schema, AirRole::Master, partner)),
        (PairCase::Air, 1) => Ok(air_settings(schema, AirRole::Runner, partner)),
        (PairCase::Mule, 0) => Ok(mule_settings(schema, MuleRole::Crafter, partner)),
        (PairCase::Mule, 1) => Ok(mule_settings(schema, MuleRole::Mule, partner)),
        (PairCase::Flax, 0) => Ok(flax_settings(schema, FlaxRole::Runner, partner)),
        (PairCase::Flax, 1) => Ok(flax_settings(schema, FlaxRole::Spinner, partner)),
        (PairCase::Duel, _) => Ok(duel_settings(schema)),
        _ => Err(format!(
            "pair settings require two headed slots for {case:?}, got index {slot_index}"
        )),
    }
}






#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairWatchStatus {
    Disabled,
    Ready,
    Running,
    Qualified,
    Failed,
}

#[derive(Clone, Default)]
pub struct PairWatch {
    active: Arc<AtomicBool>,
    inner: Arc<Mutex<PairWatchState>>,
}

struct SlotReady {
    account: String,
    peer: String,
    settings: Map<String, Value>,
    latest_air: Option<AirObservation>,
    latest_flax: Option<FlaxObservation>,
    latest_duel: Option<DuelObservation>,
    weapon_id: i32,
}

enum PairRuntimeWitness {
    Air(AirPairWitness),
    Mule(MulePairWitness),
    Flax(FlaxPairWitness),
    Duel(DuelPairWitness),
}

#[derive(Default)]
enum PairWatchState {
    #[default]
    Disabled,
    Ready {
        case: PairCase,
        a: Box<SlotReady>,
        b: Box<SlotReady>,
    },
    Running {
        a_account: String,
        b_account: String,
        witness: Box<PairRuntimeWitness>,
        duel_weapon_id: i32,
    },
    Qualified {
        evidence: Arc<Value>,
    },
    Failed {
        error: String,
        evidence: Option<Value>,
    },
}

impl PairWatch {
    pub fn configure(
        &self,
        case: PairCase,
        account_a: impl Into<String>,
        account_b: impl Into<String>,
    ) {
        let a = account_a.into();
        let b = account_b.into();
        *self.inner.lock().unwrap() = PairWatchState::Ready {
            case,
            a: Box::new(SlotReady {
                account: a.clone(),
                peer: b.clone(),
                settings: Map::new(),
                latest_air: None,
                latest_flax: None,
                latest_duel: None,
                weapon_id: 0,
            }),
            b: Box::new(SlotReady {
                account: b,
                peer: a,
                settings: Map::new(),
                latest_air: None,
                latest_flax: None,
                latest_duel: None,
                weapon_id: 0,
            }),
        };
        self.active.store(true, Ordering::Release);
    }

    /// Carry the actual per-slot bags constructed for Start through the
    /// owned watch so freeze/receipt do not invent empty settings.
    pub fn install_prepared_settings(
        &self,
        account_a: &str,
        bag_a: Map<String, Value>,
        account_b: &str,
        bag_b: Map<String, Value>,
    ) -> Result<(), String> {
        let mut state = self.inner.lock().unwrap();
        match &mut *state {
            PairWatchState::Ready { case, a, b }
                if a.account == account_a && b.account == account_b =>
            {
                prepared_settings_match(*case, account_a, account_b, &bag_a)?;
                prepared_settings_match(*case, account_b, account_a, &bag_b)?;
                a.settings = bag_a;
                b.settings = bag_b;
                Ok(())
            }
            PairWatchState::Ready { a, b, .. } => Err(format!(
                "pair settings slots {account_a:?}/{account_b:?} are not configured accounts {:?}/{:?}",
                a.account, b.account
            )),
            _ => Err("pair settings can only be installed on a configured ready pair".into()),
        }
    }

    /// Bind the selected-revision bronze scimitar id so snapshot
    /// observations can check the actual equipped weapon. Counterpart
    /// identity is already the other configured account.
    pub fn install_duel_weapon(&self, weapon_id: i32) -> Result<(), String> {
        if weapon_id <= 0 {
            return Err("Duel weapon id must come from the selected cache".into());
        }
        let mut state = self.inner.lock().unwrap();
        match &mut *state {
            PairWatchState::Ready {
                case: PairCase::Duel,
                a,
                b,
            } => {
                a.weapon_id = weapon_id;
                b.weapon_id = weapon_id;
                Ok(())
            }
            PairWatchState::Ready { case, .. } => {
                Err(format!("Duel weapon is not used for headed {case:?} cells"))
            }
            _ => Err("Duel weapon can only be installed on a configured ready pair".into()),
        }
    }

    pub fn clear(&self) {
        self.active.store(false, Ordering::Release);
        *self.inner.lock().unwrap() = PairWatchState::Disabled;
    }

    pub fn configured(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    pub fn barrier(&self) -> StartBarrier {
        let state = self.inner.lock().unwrap();
        match &*state {
            PairWatchState::Disabled => StartBarrier::Wait,
            PairWatchState::Ready { case, a, b } => {
                let a_ok = slot_prepared(*case, true, a);
                let b_ok = slot_prepared(*case, false, b);
                shared_start_barrier(StartBarrierInput {
                    a_prepared: a_ok,
                    b_prepared: b_ok,
                    a_started: false,
                    b_started: false,
                    a_wait_ack: false,
                    b_wait_ack: false,
                    a_current_ok: a_ok,
                    b_current_ok: b_ok,
                })
            }
            PairWatchState::Running { .. }
            | PairWatchState::Qualified { .. }
            | PairWatchState::Failed { .. } => StartBarrier::Wait,
        }
    }

    pub fn begin_shared_start(&self, account_a: &str, account_b: &str) -> Result<(), String> {
        let mut state = self.inner.lock().unwrap();
        let current = std::mem::take(&mut *state);
        match current {
            PairWatchState::Disabled => {
                *state = PairWatchState::Disabled;
                Ok(())
            }
            PairWatchState::Ready { case, a, b }
                if a.account == account_a && b.account == account_b =>
            {
                match freeze_pair(case, &a, &b) {
                    Ok(witness) => {
                        *state = PairWatchState::Running {
                            a_account: account_a.to_string(),
                            b_account: account_b.to_string(),
                            witness: Box::new(witness),
                            duel_weapon_id: a.weapon_id,
                        };
                        Ok(())
                    }
                    Err(error) => {
                        *state = PairWatchState::Ready { case, a, b };
                        Err(error)
                    }
                }
            }
            PairWatchState::Ready { case, a, b } => {
                let error = format!(
                    "pair core Start slots {account_a:?}/{account_b:?} are not configured accounts {:?}/{:?}",
                    a.account, b.account
                );
                *state = PairWatchState::Ready { case, a, b };
                Err(error)
            }
            other => {
                *state = other;
                Err("pair core Start was armed more than once".into())
            }
        }
    }

    pub fn fail_start(&self, error: impl Into<String>) {
        let mut state = self.inner.lock().unwrap();
        let current = std::mem::take(&mut *state);
        *state = match current {
            PairWatchState::Running { witness, .. } => PairWatchState::Failed {
                error: error.into(),
                evidence: Some(runtime_evidence(&witness)),
            },
            other => other,
        };
    }

    pub fn observe_air(&self, account: &str, observation: AirObservation, session_boundary: bool) {
        if !self.active.load(Ordering::Acquire) {
            return;
        }
        let mut state = self.inner.lock().unwrap();
        observe_air_locked(&mut state, account, observation, session_boundary);
    }

    pub fn observe_flax(
        &self,
        account: &str,
        observation: FlaxObservation,
        session_boundary: bool,
    ) {
        if !self.active.load(Ordering::Acquire) {
            return;
        }
        let mut state = self.inner.lock().unwrap();
        observe_flax_locked(&mut state, account, observation, session_boundary);
    }

    pub fn observe_duel(
        &self,
        account: &str,
        observation: DuelObservation,
        session_boundary: bool,
    ) {
        if !self.active.load(Ordering::Acquire) {
            return;
        }
        let mut state = self.inner.lock().unwrap();
        observe_duel_locked(&mut state, account, observation, session_boundary);
    }

    pub fn observe_snapshot(&self, account: &str, snapshot: &GameSnapshot, session_boundary: bool) {
        if !self.active.load(Ordering::Acquire) {
            return;
        }
        let mut state = self.inner.lock().unwrap();
        let dispatch = match &*state {
            PairWatchState::Ready { case, a, b }
                if a.account == account || b.account == account =>
            {
                let (peer, weapon_id) = if a.account == account {
                    (a.peer.clone(), a.weapon_id)
                } else {
                    (b.peer.clone(), b.weapon_id)
                };
                Some((*case, peer, weapon_id))
            }
            PairWatchState::Running {
                a_account,
                b_account,
                witness,
                duel_weapon_id,
            } if a_account == account || b_account == account => {
                let case = match &**witness {
                    PairRuntimeWitness::Air(_) => PairCase::Air,
                    PairRuntimeWitness::Mule(_) => PairCase::Mule,
                    PairRuntimeWitness::Flax(_) => PairCase::Flax,
                    PairRuntimeWitness::Duel(_) => PairCase::Duel,
                };
                let peer = match &**witness {
                    PairRuntimeWitness::Duel(pair) if a_account == account => {
                        pair.a.partner.clone()
                    }
                    PairRuntimeWitness::Duel(pair) => pair.b.partner.clone(),
                    _ => String::new(),
                };
                Some((case, peer, *duel_weapon_id))
            }
            _ => None,
        };
        let Some((case, peer, weapon_id)) = dispatch else {
            return;
        };
        match case {
            PairCase::Flax => {
                observe_flax_locked(
                    &mut state,
                    account,
                    FlaxObservation::from_snapshot(snapshot),
                    session_boundary,
                );
            }
            PairCase::Air | PairCase::Mule => {
                observe_air_locked(
                    &mut state,
                    account,
                    AirObservation::from_snapshot(snapshot),
                    session_boundary,
                );
            }
            PairCase::Duel => {
                observe_duel_locked(
                    &mut state,
                    account,
                    DuelObservation::from_snapshot(snapshot, &peer, weapon_id),
                    session_boundary,
                );
            }
        }
    }

    pub fn status(&self) -> PairWatchStatus {
        if !self.active.load(Ordering::Acquire) {
            return PairWatchStatus::Disabled;
        }
        let mut state = self.inner.lock().unwrap();
        let should_cache = match &*state {
            PairWatchState::Running { witness, .. } => runtime_full_cycle(witness).is_ok(),
            _ => false,
        };
        if should_cache {
            let current = std::mem::take(&mut *state);
            if let PairWatchState::Running { witness, .. } = current {
                *state = PairWatchState::Qualified {
                    evidence: Arc::new(runtime_evidence(&witness)),
                };
            }
        }
        match &*state {
            PairWatchState::Disabled => PairWatchStatus::Disabled,
            PairWatchState::Ready { .. } => PairWatchStatus::Ready,
            PairWatchState::Running { .. } => PairWatchStatus::Running,
            PairWatchState::Qualified { .. } => PairWatchStatus::Qualified,
            PairWatchState::Failed { .. } => PairWatchStatus::Failed,
        }
    }

    pub fn failure(&self) -> Option<String> {
        match &*self.inner.lock().unwrap() {
            PairWatchState::Failed { error, .. } => Some(error.clone()),
            _ => None,
        }
    }

    pub fn qualify(&self) -> Result<Arc<Value>, String> {
        let mut state = self.inner.lock().unwrap();
        let current = std::mem::take(&mut *state);
        match current {
            PairWatchState::Running {
                a_account,
                b_account,
                witness,
                duel_weapon_id,
            } => match runtime_terminal_claim(&witness) {
                Ok(()) => {
                    let evidence = Arc::new(runtime_evidence(&witness));
                    *state = PairWatchState::Qualified {
                        evidence: Arc::clone(&evidence),
                    };
                    Ok(evidence)
                }
                Err(error) => {
                    *state = PairWatchState::Running {
                        a_account,
                        b_account,
                        witness,
                        duel_weapon_id,
                    };
                    Err(error)
                }
            },
            PairWatchState::Qualified { evidence } => {
                let receipt = Arc::clone(&evidence);
                *state = PairWatchState::Qualified { evidence };
                Ok(receipt)
            }
            failed @ PairWatchState::Failed { .. } => {
                let error = match &failed {
                    PairWatchState::Failed { error, .. } => error.clone(),
                    _ => unreachable!(),
                };
                *state = failed;
                Err(error)
            }
            ready @ PairWatchState::Ready { .. } => {
                *state = ready;
                Err("pair core Start not observed".into())
            }
            PairWatchState::Disabled => {
                *state = PairWatchState::Disabled;
                Err("pair core watch disabled".into())
            }
        }
    }

    pub fn evidence(&self) -> Value {
        match &*self.inner.lock().unwrap() {
            PairWatchState::Disabled => json!({"phase": "disabled"}),
            PairWatchState::Ready { case, a, b } => json!({
                "phase": "ready",
                "case": case,
                "a": a.account,
                "b": b.account,
            }),
            PairWatchState::Running { witness, .. } => json!({
                "phase": "running",
                "witness": runtime_evidence(witness),
                "qualification": runtime_terminal_claim(witness).err(),
            }),
            PairWatchState::Qualified { evidence } => json!({
                "phase": "qualified",
                "receipt": evidence,
            }),
            PairWatchState::Failed { error, evidence } => json!({
                "phase": "failed",
                "error": error,
                "witness": evidence,
            }),
        }
    }
}

fn slot_prepared(case: PairCase, first: bool, slot: &SlotReady) -> bool {
    match case {
        PairCase::Air => slot.latest_air.as_ref().is_some_and(|obs| {
            air_prepared_current(
                if first {
                    AirRole::Master
                } else {
                    AirRole::Runner
                },
                &slot.account,
                obs,
            )
            .is_ok()
        }),
        PairCase::Mule => slot.latest_air.as_ref().is_some_and(|obs| {
            mule_prepared_current(
                if first {
                    MuleRole::Crafter
                } else {
                    MuleRole::Mule
                },
                &slot.account,
                obs,
            )
            .is_ok()
        }),
        PairCase::Flax => slot.latest_flax.as_ref().is_some_and(|obs| {
            flax_prepared_current(
                if first {
                    FlaxRole::Runner
                } else {
                    FlaxRole::Spinner
                },
                &slot.account,
                obs,
            )
            .is_ok()
        }),
        PairCase::Duel => slot
            .latest_duel
            .as_ref()
            .is_some_and(|obs| duel_prepared_current(&slot.account, obs).is_ok()),
    }
}

fn freeze_pair(case: PairCase, a: &SlotReady, b: &SlotReady) -> Result<PairRuntimeWitness, String> {
    if a.settings.is_empty() != b.settings.is_empty() {
        return Err("pair settings must be prepared for both slots or neither".into());
    }
    let a_settings = freeze_slot_settings(case, a, &b.account)?;
    let b_settings = freeze_slot_settings(case, b, &a.account)?;
    match case {
        PairCase::Air => {
            let Some(a_obs) = a.latest_air.clone() else {
                return Err("pair core has no published pre-Start observation".into());
            };
            let Some(b_obs) = b.latest_air.clone() else {
                return Err("pair core has no published pre-Start observation".into());
            };
            air_prepared_current(AirRole::Master, &a.account, &a_obs)?;
            air_prepared_current(AirRole::Runner, &b.account, &b_obs)?;
            Ok(PairRuntimeWitness::Air(AirPairWitness {
                master: AirSlotRecord::new(
                    AirRole::Master,
                    a.account.clone(),
                    a.account.clone(),
                    b.account.clone(),
                    a_settings,
                    a_obs,
                ),
                runner: AirSlotRecord::new(
                    AirRole::Runner,
                    b.account.clone(),
                    b.account.clone(),
                    a.account.clone(),
                    b_settings,
                    b_obs,
                ),
            }))
        }
        PairCase::Mule => {
            let Some(a_obs) = a.latest_air.clone() else {
                return Err("pair core has no published pre-Start observation".into());
            };
            let Some(b_obs) = b.latest_air.clone() else {
                return Err("pair core has no published pre-Start observation".into());
            };
            mule_prepared_current(MuleRole::Crafter, &a.account, &a_obs)?;
            mule_prepared_current(MuleRole::Mule, &b.account, &b_obs)?;
            Ok(PairRuntimeWitness::Mule(MulePairWitness {
                crafter: MuleSlotRecord::new(
                    MuleRole::Crafter,
                    a.account.clone(),
                    a.account.clone(),
                    b.account.clone(),
                    a_settings,
                    a_obs,
                ),
                mule: MuleSlotRecord::new(
                    MuleRole::Mule,
                    b.account.clone(),
                    b.account.clone(),
                    a.account.clone(),
                    b_settings,
                    b_obs,
                ),
            }))
        }
        PairCase::Flax => {
            let Some(a_obs) = a.latest_flax.clone() else {
                return Err("pair core has no published pre-Start observation".into());
            };
            let Some(b_obs) = b.latest_flax.clone() else {
                return Err("pair core has no published pre-Start observation".into());
            };
            flax_prepared_current(FlaxRole::Runner, &a.account, &a_obs)?;
            flax_prepared_current(FlaxRole::Spinner, &b.account, &b_obs)?;
            Ok(PairRuntimeWitness::Flax(FlaxPairWitness {
                runner: FlaxSlotRecord::new(
                    FlaxRole::Runner,
                    a.account.clone(),
                    a.account.clone(),
                    b.account.clone(),
                    a_settings,
                    a_obs,
                ),
                spinner: FlaxSlotRecord::new(
                    FlaxRole::Spinner,
                    b.account.clone(),
                    b.account.clone(),
                    a.account.clone(),
                    b_settings,
                    b_obs,
                ),
            }))
        }
        PairCase::Duel => {
            let Some(a_obs) = a.latest_duel.clone() else {
                return Err("pair core has no published pre-Start observation".into());
            };
            let Some(b_obs) = b.latest_duel.clone() else {
                return Err("pair core has no published pre-Start observation".into());
            };
            duel_prepared_current(&a.account, &a_obs)?;
            duel_prepared_current(&b.account, &b_obs)?;
            Ok(PairRuntimeWitness::Duel(DuelPairWitness {
                a: DuelSlotRecord::new(
                    a.account.clone(),
                    a.account.clone(),
                    b.account.clone(),
                    a_settings,
                    a_obs,
                ),
                b: DuelSlotRecord::new(
                    b.account.clone(),
                    b.account.clone(),
                    a.account.clone(),
                    b_settings,
                    b_obs,
                ),
            }))
        }
    }
}

fn freeze_slot_settings(
    case: PairCase,
    slot: &SlotReady,
    partner: &str,
) -> Result<Map<String, Value>, String> {
    prepared_settings_match(case, &slot.account, partner, &slot.settings)?;
    Ok(slot.settings.clone())
}

fn prepared_settings_match(
    case: PairCase,
    account: &str,
    partner: &str,
    bag: &Map<String, Value>,
) -> Result<(), String> {
    if bag.is_empty() {
        return Ok(());
    }
    match case {
        PairCase::Duel => match bag.get("partner") {
            Some(value) => Err(format!(
                "Duel settings for {account} must not carry partner {value}; counterpart identity is native witness-owned"
            )),
            None => Ok(()),
        },
        PairCase::Air | PairCase::Mule | PairCase::Flax => {
            match bag.get("partner").and_then(Value::as_str) {
                Some(name) if account_identity_eq(name, partner) => Ok(()),
                Some(name) => Err(format!(
                    "pair settings partner {name:?} is not the reciprocal account {partner:?}"
                )),
                None => Err(format!("pair settings for {account} are missing partner")),
            }
        }
    }
}

fn observe_air_locked(
    state: &mut PairWatchState,
    account: &str,
    observation: AirObservation,
    session_boundary: bool,
) {
    let current = std::mem::take(state);
    *state = match current {
        PairWatchState::Ready { case, mut a, mut b }
            if a.account == account || b.account == account =>
        {
            if session_boundary {
                if a.account == account {
                    a.latest_air = None;
                } else {
                    b.latest_air = None;
                }
            } else if a.account == account {
                a.latest_air = Some(observation);
            } else {
                b.latest_air = Some(observation);
            }
            PairWatchState::Ready { case, a, b }
        }
        PairWatchState::Running {
            a_account,
            b_account,
            mut witness,
            duel_weapon_id,
        } if a_account == account || b_account == account => {
            if session_boundary {
                PairWatchState::Failed {
                    error: "pair core session boundary after Start".into(),
                    evidence: Some(runtime_evidence(&witness)),
                }
            } else {
                match &mut *witness {
                    PairRuntimeWitness::Air(pair) if a_account == account => {
                        pair.master.observe(observation);
                    }
                    PairRuntimeWitness::Air(pair) => pair.runner.observe(observation),
                    PairRuntimeWitness::Mule(pair) if a_account == account => {
                        pair.crafter.observe(observation);
                    }
                    PairRuntimeWitness::Mule(pair) => pair.mule.observe(observation),
                    PairRuntimeWitness::Flax(_) | PairRuntimeWitness::Duel(_) => {}
                }
                PairWatchState::Running {
                    a_account,
                    b_account,
                    witness,
                    duel_weapon_id,
                }
            }
        }
        other => other,
    };
}

fn observe_flax_locked(
    state: &mut PairWatchState,
    account: &str,
    observation: FlaxObservation,
    session_boundary: bool,
) {
    let current = std::mem::take(state);
    *state = match current {
        PairWatchState::Ready { case, mut a, mut b }
            if a.account == account || b.account == account =>
        {
            if session_boundary {
                if a.account == account {
                    a.latest_flax = None;
                } else {
                    b.latest_flax = None;
                }
            } else if a.account == account {
                a.latest_flax = Some(observation);
            } else {
                b.latest_flax = Some(observation);
            }
            PairWatchState::Ready { case, a, b }
        }
        PairWatchState::Running {
            a_account,
            b_account,
            mut witness,
            duel_weapon_id,
        } if a_account == account || b_account == account => {
            if session_boundary {
                PairWatchState::Failed {
                    error: "pair core session boundary after Start".into(),
                    evidence: Some(runtime_evidence(&witness)),
                }
            } else {
                if let PairRuntimeWitness::Flax(pair) = &mut *witness {
                    if a_account == account {
                        pair.runner.observe(observation);
                    } else {
                        pair.spinner.observe(observation);
                    }
                }
                PairWatchState::Running {
                    a_account,
                    b_account,
                    witness,
                    duel_weapon_id,
                }
            }
        }
        other => other,
    };
}

fn observe_duel_locked(
    state: &mut PairWatchState,
    account: &str,
    observation: DuelObservation,
    session_boundary: bool,
) {
    let current = std::mem::take(state);
    *state = match current {
        PairWatchState::Ready { case, mut a, mut b }
            if a.account == account || b.account == account =>
        {
            if session_boundary {
                if a.account == account {
                    a.latest_duel = None;
                } else {
                    b.latest_duel = None;
                }
            } else if a.account == account {
                a.latest_duel = Some(observation);
            } else {
                b.latest_duel = Some(observation);
            }
            PairWatchState::Ready { case, a, b }
        }
        PairWatchState::Running {
            a_account,
            b_account,
            mut witness,
            duel_weapon_id,
        } if a_account == account || b_account == account => {
            if session_boundary {
                PairWatchState::Failed {
                    error: "pair core session boundary after Start".into(),
                    evidence: Some(runtime_evidence(&witness)),
                }
            } else {
                if let PairRuntimeWitness::Duel(pair) = &mut *witness {
                    if a_account == account {
                        pair.a.observe(observation);
                    } else {
                        pair.b.observe(observation);
                    }
                }
                PairWatchState::Running {
                    a_account,
                    b_account,
                    witness,
                    duel_weapon_id,
                }
            }
        }
        other => other,
    };
}

fn runtime_full_cycle(witness: &PairRuntimeWitness) -> Result<(), String> {
    match witness {
        PairRuntimeWitness::Air(pair) => pair.qualify_full_cycle().map(|_| ()),
        PairRuntimeWitness::Mule(pair) => pair.qualify_full_cycle().map(|_| ()),
        PairRuntimeWitness::Flax(pair) => pair.qualify_full_cycle().map(|_| ()),
        PairRuntimeWitness::Duel(pair) => pair.qualify_full_cycle().map(|_| ()),
    }
}

fn runtime_terminal_claim(witness: &PairRuntimeWitness) -> Result<(), String> {
    match witness {
        PairRuntimeWitness::Air(_) | PairRuntimeWitness::Mule(_) | PairRuntimeWitness::Flax(_) => {
            runtime_full_cycle(witness)
        }
        PairRuntimeWitness::Duel(pair) => pair
            .qualify_full_cycle()
            .map(|_| ())
            .or_else(|_| pair.qualify_supported().map(|_| ())),
    }
}

fn runtime_evidence(witness: &PairRuntimeWitness) -> Value {
    match witness {
        PairRuntimeWitness::Air(pair) => json!({
            "case": PairCase::Air,
            "supported": pair.qualify_supported().ok(),
            "full": pair.qualify_full_cycle().ok(),
            "master": pair.master,
            "runner": pair.runner,
        }),
        PairRuntimeWitness::Mule(pair) => json!({
            "case": PairCase::Mule,
            "supported": pair.qualify_supported().ok(),
            "full": pair.qualify_full_cycle().ok(),
            "crafter": pair.crafter,
            "mule": pair.mule,
        }),
        PairRuntimeWitness::Flax(pair) => json!({
            "case": PairCase::Flax,
            "supported": pair.qualify_supported().ok(),
            "full": pair.qualify_full_cycle().ok(),
            "runner": pair.runner,
            "spinner": pair.spinner,
        }),
        PairRuntimeWitness::Duel(pair) => json!({
            "case": PairCase::Duel,
            "claim": pair.qualify_full_cycle().ok().or_else(|| pair.qualify_supported().ok()),
            "supported": pair.qualify_supported().ok(),
            "full": pair.qualify_full_cycle().ok(),
            "a": pair.a,
            "b": pair.b,
        }),
    }
}
