use super::{FeatureCase, Observation};
use crate::Play;
use api::game_data::SelectedGameData;
use api::selected::{ClientRevision, RunKey};
use api::snapshot::{GameSnapshot, ItemContainer, ItemView};
use client::util::JString;
use scenario::quester::{quester_role_stage, QuesterSeed, QuesterStage, TestProfile};
use scenario::{Proof, Scenario, StepKind};
use script::quester::pair::Gang;
use script::native::NativePhase;
use script::{RunState, ScriptTerminalState, StartOutcome, StartPoll};
use serde_json::{json, Map, Value};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

const QUEST: &str = "blackarmgang";
const DISPLAY: &str = "Shield of Arrav";
const KEY_ALIAS: &str = "phoenixkey2";
const KEY_SOURCE: &str = "GameSnapshot.inventory and GameSnapshot.trade.confirm";
const CONFIRM_SOURCE: &str = "GameSnapshot.trade.confirm.mine/theirs";
const CLOCK_SOURCE: &str = "GameSnapshot.tick";
const NATIVE_SOURCE: &str = "Play::script_start_typed/script_poll_start/script_native_status/script_lifecycle_receipt";
const CONTINUING_FRESHNESS: Duration = Duration::from_secs(60);

/// The guide's rendezvous for `p-key` and `b-key` (`maps/m50_52.jm2:8228`).
const STAND: api::snapshot::WorldTile = api::snapshot::WorldTile {
    x: 3208,
    z: 3391,
    level: 0,
};

/// R289 content varps recorded by the live path proofs (`phoenixgang_146`,
/// `blackarmgang_145`); the account stages and their values are the authored
/// p:9 / b:2 rows in `quester_partner_trade_live.rs`.
const PHOENIX_STAGE_VARP: i32 = 146;
const BLACK_ARM_STAGE_VARP: i32 = 145;
const PHOENIX_STAGE: i32 = 9;
const BLACK_ARM_STAGE: i32 = 2;
const PHOENIX_ITEMS: &[(&str, i32)] = &[(KEY_ALIAS, 1)];
const BLACK_ARM_ITEMS: &[(&str, i32)] = &[];
const PHOENIX_SEED: &[(&str, i32)] = &[("phoenixgang", PHOENIX_STAGE)];
const BLACK_ARM_SEED: &[(&str, i32)] = &[("blackarmgang", BLACK_ARM_STAGE)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StartProgress {
    NotRequested,
    Waiting,
    Submitted,
    Admitted,
}

impl Default for StartProgress {
    fn default() -> Self {
        Self::NotRequested
    }
}

#[derive(Default)]
struct SlotCapture {
    seed_scenario_built: bool,
    seed_ready: bool,
    seed_tick: Option<u32>,
    stage_value: Option<i32>,
    initial_key_count: Option<i32>,
    last_tick: Option<u32>,
    active_tick_anchor: Option<u32>,
    active_tick_advances: u64,
    latest_key_count: Option<i32>,
    offer_ticks: u64,
    confirm_events: u64,
    confirm_ticks: u64,
    last_confirm_tick: Option<u32>,
    confirm_open_last: bool,
    confirm_partner_matches: bool,
    confirm_matches: bool,
    confirm_mine_key_count: Option<i32>,
    confirm_theirs_key_count: Option<i32>,
    confirm_tick: Option<u32>,
    handoff: bool,
    handoff_tick: Option<u32>,
    stop_tick: Option<u32>,
    stop_sent: bool,
    post_stop_tick: Option<u32>,
    post_stop_key_count: Option<i32>,
    last_run_state: Option<RunState>,
    native_run: Option<RunKey>,
    native_card: bool,
    native_phase: Option<NativePhase>,
    progress_idle_ms: Option<u64>,
    lifecycle_seen: bool,
    lifecycle_state: Option<ScriptTerminalState>,
    lifecycle_generation: Option<u64>,
    lifecycle_tick: Option<u64>,
    lifecycle_reason: Option<String>,
    stop_confirmed: bool,
    start: StartProgress,
}

#[derive(Default)]
struct SharedCapture {
    slots: Vec<SlotCapture>,
    profile: Option<TestProfile>,
    control_prepared: bool,
    activation_attempted: bool,
    activation_complete: bool,
    stop_requested: bool,
    stopped: bool,
    roster_restored: bool,
}

struct NativeTradeCase {
    names: Vec<String>,
    screen_names: Vec<String>,
    selected: Arc<SelectedGameData>,
    path: Arc<script::quester::path::PathDocument>,
    path_source: &'static str,
    key_id: i32,
    capture: Arc<Mutex<SharedCapture>>,
    previous_accounts: Option<Vec<String>>,
}

fn lock_capture(capture: &Mutex<SharedCapture>) -> MutexGuard<'_, SharedCapture> {
    capture.lock().unwrap_or_else(|error| error.into_inner())
}

fn validate_names(names: &[String]) -> Result<Vec<String>, String> {
    if names.len() < 2 || names.len() % 2 != 0 {
        return Err("native_partner_trade requires an even fleet of at least two accounts".into());
    }
    let mut screen_names = Vec::with_capacity(names.len());
    for (index, name) in names.iter().enumerate() {
        if name.trim().is_empty() {
            return Err(format!("native_partner_trade account {index} has an empty name"));
        }
        let screen_name = JString::to_screen_name(name);
        if screen_name.trim().is_empty()
            || screen_names
                .iter()
                .any(|known: &String| known.eq_ignore_ascii_case(&screen_name))
        {
            return Err(format!(
                "native_partner_trade account {index} has an empty or duplicate screen identity"
            ));
        }
        screen_names.push(screen_name);
    }
    Ok(screen_names)
}

fn partner_slot(slot: usize) -> usize {
    slot ^ 1
}

fn gang_for(slot: usize) -> Gang {
    if slot % 2 == 0 {
        Gang::Phoenix
    } else {
        Gang::BlackArm
    }
}

fn gang_name(gang: Gang) -> &'static str {
    match gang {
        Gang::Phoenix => "phoenix",
        Gang::BlackArm => "blackarm",
    }
}

fn stage_for(gang: Gang) -> (&'static str, i32, i32, &'static [(&'static str, i32)]) {
    match gang {
        Gang::Phoenix => (
            "p:9",
            PHOENIX_STAGE_VARP,
            PHOENIX_STAGE,
            PHOENIX_ITEMS,
        ),
        Gang::BlackArm => (
            "b:2",
            BLACK_ARM_STAGE_VARP,
            BLACK_ARM_STAGE,
            BLACK_ARM_ITEMS,
        ),
    }
}

fn key_count(items: &[ItemView], key_id: i32, container: Option<ItemContainer>) -> i32 {
    items
        .iter()
        .filter(|item| {
            item.def.id == key_id
                && !item.def.noted
                && container.map_or(true, |expected| item.container == expected)
        })
        .fold(0, |total, item| total.saturating_add(item.count.max(0)))
}

fn observed_varp(snapshot: &GameSnapshot, id: i32) -> Option<i32> {
    snapshot
        .varps()
        .iter()
        .find(|varp| varp.index == id)
        .map(|varp| varp.value)
}

fn pair_proved(slots: &[SlotCapture], slot: usize) -> bool {
    let phoenix = &slots[slot & !1];
    let black_arm = &slots[(slot & !1) + 1];
    phoenix.confirm_matches
        && black_arm.confirm_matches
        && phoenix.confirm_partner_matches
        && black_arm.confirm_partner_matches
        && phoenix.handoff
        && black_arm.handoff
}

fn pair_post_stop_owned(slots: &[SlotCapture], slot: usize) -> bool {
    let phoenix = &slots[slot & !1];
    let black_arm = &slots[(slot & !1) + 1];
    phoenix.stop_confirmed
        && black_arm.stop_confirmed
        && phoenix.handoff
        && black_arm.handoff
        && phoenix.post_stop_key_count == Some(0)
        && black_arm.post_stop_key_count == Some(1)
}

/// Consume one authentic client snapshot in the seed runner's client callback.
/// No Play/product method is called while the case lock is held.
fn capture_snapshot(
    capture: &Mutex<SharedCapture>,
    slot: usize,
    seed: &QuesterSeed,
    stage_varp: i32,
    stage_value: i32,
    key_id: i32,
    partner_screen_name: &str,
    snapshot: &GameSnapshot,
) -> bool {
    let need_seed_readiness = {
        let state = lock_capture(capture);
        !state.slots[slot].seed_ready
    };
    let observed_stage = if need_seed_readiness {
        observed_varp(snapshot, stage_varp)
    } else {
        None
    };
    let inventory_key_count = snapshot
        .ingame()
        .then(|| key_count(snapshot.inventory(), key_id, Some(ItemContainer::Inventory)));
    let expected_initial_key_count = if slot % 2 == 0 { 1 } else { 0 };
    let seed_ready = need_seed_readiness
        && snapshot.ingame()
        && snapshot.scene_state() == 2
        && snapshot.tile() == Some((STAND.x, STAND.z, STAND.level))
        && observed_stage == Some(stage_value)
        && inventory_key_count == Some(expected_initial_key_count)
        && seed.observe_start(snapshot).is_ok();
    let trade = snapshot.trade();
    let confirm = if trade.confirm_open {
        trade.confirm.as_deref().map(|confirm| {
            let mine = key_count(
                &confirm.mine,
                key_id,
                Some(ItemContainer::TradeMyOffer),
            );
            let theirs = key_count(
                &confirm.theirs,
                key_id,
                Some(ItemContainer::TradeTheirOffer),
            );
            let partner_matches = trade.partner.as_deref().is_some_and(|partner| {
                partner.eq_ignore_ascii_case(partner_screen_name)
            });
            (mine, theirs, partner_matches)
        })
    } else {
        None
    };
    let tick = snapshot.tick();
    let ingame_tick = snapshot.ingame().then_some(tick);
    let mut state = lock_capture(capture);
    let activated = state.activation_complete;
    let stop_requested = state.stop_requested;
    let slot_state = &mut state.slots[slot];
    if seed_ready {
        slot_state.seed_ready = true;
        slot_state.seed_tick = Some(tick);
        slot_state.stage_value = observed_stage;
        slot_state.initial_key_count = inventory_key_count;
    }
    if let Some(tick) = ingame_tick {
        slot_state.last_tick = Some(tick);
        if activated {
            if let Some(anchor) = slot_state.active_tick_anchor {
                slot_state.active_tick_advances =
                    slot_state.active_tick_advances.max(u64::from(tick.saturating_sub(anchor)));
            }
        }
        if inventory_key_count.is_some() {
            slot_state.latest_key_count = inventory_key_count;
        }
        if trade.offer_open {
            slot_state.offer_ticks = slot_state.offer_ticks.saturating_add(1);
        }
        if let Some((mine, theirs, partner_matches)) = confirm {
            if !slot_state.confirm_open_last {
                slot_state.confirm_events = slot_state.confirm_events.saturating_add(1);
            }
            if slot_state.last_confirm_tick != Some(tick) {
                slot_state.confirm_ticks = slot_state.confirm_ticks.saturating_add(1);
                slot_state.last_confirm_tick = Some(tick);
            }
            slot_state.confirm_mine_key_count = Some(mine);
            slot_state.confirm_theirs_key_count = Some(theirs);
            slot_state.confirm_partner_matches |= partner_matches;
            let list_matches = match slot % 2 {
                0 => mine == 1 && theirs == 0,
                _ => mine == 0 && theirs == 1,
            };
            if partner_matches && list_matches {
                slot_state.confirm_matches = true;
                slot_state.confirm_tick.get_or_insert(tick);
            }
            slot_state.confirm_open_last = true;
        } else {
            slot_state.confirm_open_last = false;
        }
        if slot_state.confirm_matches
            && slot_state
                .confirm_tick
                .is_some_and(|confirmed_at| tick > confirmed_at)
            && inventory_key_count
                == Some(if slot % 2 == 0 { 0 } else { 1 })
        {
            slot_state.handoff = true;
            slot_state.handoff_tick.get_or_insert(tick);
        }
        if stop_requested
            && slot_state
                .stop_tick
                .is_some_and(|stopped_at| tick > stopped_at)
        {
            slot_state.post_stop_tick = Some(tick);
            slot_state.post_stop_key_count = inventory_key_count;
        }
    }
    stop_requested
        && slot_state
            .stop_tick
            .is_some_and(|stopped_at| tick > stopped_at)
}

impl NativeTradeCase {
    fn check_names(&self, names: &[String]) -> Result<(), String> {
        if names != self.names {
            return Err("native_partner_trade received a different account roster".into());
        }
        Ok(())
    }

    fn start_settings(&self, slot: usize) -> Map<String, Value> {
        let gang = gang_for(slot);
        let mut settings = Map::new();
        settings.insert("quests".into(), json!([QUEST]));
        settings.insert("include_draft_guides".into(), Value::Bool(true));
        settings.insert(
            "partner_account".into(),
            Value::String(self.screen_names[partner_slot(slot)].clone()),
        );
        settings.insert("gang".into(), Value::String(gang_name(gang).into()));
        settings
    }

    fn start_outcome_error(name: &str, outcome: StartOutcome) -> String {
        match outcome {
            StartOutcome::Failed(reason) => {
                format!("native_partner_trade typed Start failed for {name}: {reason}")
            }
            StartOutcome::Cancelled => {
                format!("native_partner_trade typed Start was cancelled for {name}")
            }
            StartOutcome::Rejected(error) => {
                format!("native_partner_trade typed Start was rejected for {name}: {error}")
            }
            StartOutcome::Ready => unreachable!("Ready is not an error"),
        }
    }

}

pub(super) fn build(names: &[String]) -> Result<Box<dyn FeatureCase>, String> {
    let screen_names = validate_names(names)?;
    let selected = api::game_data::for_revision(ClientRevision::R289)
        .map_err(|error| format!("native_partner_trade selected R289 content unavailable: {error}"))?;
    let (path, path_source) = scenario::quester::load_quester_path_document(QUEST)?;
    if path.id.0.as_ref() != QUEST {
        return Err(format!(
            "native_partner_trade path registry returned {:?}, expected {QUEST}",
            path.id.0
        ));
    }
    let identity = selected
        .quest_identity()
        .and_then(|facts| facts.rows.iter().find(|row| row.id == QUEST))
        .ok_or("native_partner_trade selected R289 content has no Shield of Arrav identity")?;
    if identity.display != DISPLAY {
        return Err(format!(
            "native_partner_trade selected quest identity is {:?}, expected {DISPLAY:?}",
            identity.display
        ));
    }
    let key_id = selected
        .item_by_alias(KEY_ALIAS)
        .ok_or("native_partner_trade selected R289 content has no Phoenix key")?
        .id;
    let slots = (0..names.len()).map(|_| SlotCapture::default()).collect();
    Ok(Box::new(NativeTradeCase {
        names: names.to_vec(),
        screen_names,
        selected,
        path: Arc::new(path),
        path_source,
        key_id,
        capture: Arc::new(Mutex::new(SharedCapture {
            slots,
            ..SharedCapture::default()
        })),
        previous_accounts: None,
    }))
}

impl FeatureCase for NativeTradeCase {
    fn seed(&self, slot: usize, names: &[String]) -> Result<Scenario, String> {
        self.check_names(names)?;
        if slot >= self.names.len() {
            return Err(format!("native_partner_trade has no account slot {slot}"));
        }
        {
            let state = lock_capture(&self.capture);
            if state.slots[slot].seed_scenario_built {
                return Err(format!(
                    "native_partner_trade seed scenario for slot {slot} was requested twice"
                ));
            }
        }
        let gang = gang_for(slot);
        let (stage, stage_varp, stage_value, items) = stage_for(gang);
        let (seed_varp, seed_value) = if gang == Gang::Phoenix {
            PHOENIX_SEED[0]
        } else {
            BLACK_ARM_SEED[0]
        };
        let identity = self
            .selected
            .quest_identity()
            .and_then(|facts| facts.rows.iter().find(|row| row.id == QUEST))
            .ok_or("native_partner_trade selected R289 content has no Shield of Arrav identity")?;
        let fixture = quester_role_stage(
            QuesterStage {
                name: "native partner trade account seed",
                quest_display: DISPLAY,
                path: &self.path,
                identity,
                selected: &self.selected,
                stage,
                loadout: None,
                extra_items: items,
                stand: STAND,
            },
            gang,
            seed_varp,
            &[(seed_varp, seed_value)],
        )?;
        let profile = fixture.seed.profile;
        let mut scenario = fixture.scenario;
        scenario.settings.start_script = None;
        let start_index = scenario
            .steps
            .iter()
            .position(|step| matches!(&step.kind, StepKind::StartScript))
            .ok_or("native_partner_trade fixture has no typed-Start boundary")?;
        if start_index + 2 != scenario.steps.len() {
            return Err("native_partner_trade fixture Start boundary moved from its final watch".into());
        }
        scenario.steps.remove(start_index);
        let final_step = scenario
            .steps
            .last_mut()
            .ok_or("native_partner_trade fixture has no terminal observation step")?;
        final_step.name = "observe native trade state through Stop";
        final_step.kind = StepKind::Await {
            evidence: "real partner trade and post-Stop account snapshot",
            ready: Box::new({
                let capture = Arc::clone(&self.capture);
                let seed = fixture.seed;
                let slot = slot;
                let key_id = self.key_id;
                let partner_screen_name = self.screen_names[partner_slot(slot)].clone();
                move |snapshot| {
                    capture_snapshot(
                        &capture,
                        slot,
                        &seed,
                        stage_varp,
                        stage_value,
                        key_id,
                        &partner_screen_name,
                        snapshot,
                    )
                }
            }),
            summary: None,
        };
        final_step.wait.arm = Proof::IngameScene2;
        scenario.proof = Proof::IngameScene2;
        {
            let mut state = lock_capture(&self.capture);
            if state.slots[slot].seed_scenario_built {
                return Err(format!(
                    "native_partner_trade seed scenario for slot {slot} was requested twice"
                ));
            }
            if state.profile.is_some_and(|known| known != profile) {
                return Err("native_partner_trade account seed profiles differ".into());
            }
            state.slots[slot].seed_scenario_built = true;
            state.profile = Some(profile);
        }
        Ok(scenario)
    }

    fn prepare_unused(&mut self, play: &Play, names: &[String]) -> Result<bool, String> {
        self.check_names(names)?;
        {
            let state = lock_capture(&self.capture);
            if state.stopped {
                return Err("native_partner_trade cannot prepare control after Stop".into());
            }
            if state.activation_attempted {
                return Err("native_partner_trade control was requested after feature activation".into());
            }
            if state.control_prepared {
                return Ok(true);
            }
            if state.slots.iter().any(|slot| !slot.seed_ready) {
                return Err("native_partner_trade control requires every real role seed to be observed".into());
            }
        }
        for (slot, name) in names.iter().enumerate() {
            if play.script_state(name) != RunState::Idle {
                return Err(format!(
                    "native_partner_trade control refuses to adopt non-idle slot {slot} ({name})"
                ));
            }
        }
        let previous_accounts = play.quest_accounts();
        self.previous_accounts = Some(
            previous_accounts
                .iter()
                .map(|account| account.0.to_string())
                .collect(),
        );
        play.set_quest_accounts(names.iter().map(String::as_str));
        let roster = play.quest_accounts();
        let roster_matches = roster.len() == names.len()
            && names
                .iter()
                .all(|name| roster.iter().any(|account| account.0.as_ref() == name));
        if !roster_matches {
            let previous = self.previous_accounts.as_deref().unwrap_or_default();
            play.set_quest_accounts(previous.iter().map(String::as_str));
            self.previous_accounts = None;
            return Err("native_partner_trade partner account roster did not admit the exact benchmark names".into());
        }
        for (slot, name) in names.iter().enumerate() {
            if play.script_state(name) != RunState::Idle {
                let previous = self.previous_accounts.as_deref().unwrap_or_default();
                play.set_quest_accounts(previous.iter().map(String::as_str));
                self.previous_accounts = None;
                return Err(format!(
                    "native_partner_trade control slot {slot} changed from Idle during roster setup"
                ));
            }
        }
        lock_capture(&self.capture).control_prepared = true;
        Ok(true)
    }

    fn activate(&mut self, play: &Play, names: &[String]) -> Result<bool, String> {
        self.check_names(names)?;
        {
            let mut state = lock_capture(&self.capture);
            if !state.control_prepared {
                return Err("native_partner_trade cannot activate before its unused control setup".into());
            }
            if state.stopped {
                return Err("native_partner_trade cannot activate after Stop".into());
            }
            state.activation_attempted = true;
            if state.activation_complete {
                return Ok(true);
            }
        }
        for (slot, name) in names.iter().enumerate() {
            let start = lock_capture(&self.capture).slots[slot].start;
            if matches!(start, StartProgress::NotRequested | StartProgress::Waiting) {
                if play.script_state(name) != RunState::Idle {
                    return Err(format!(
                        "native_partner_trade refuses typed Start for non-idle slot {slot} ({name})"
                    ));
                }
                match play.script_start_typed(
                    name,
                    script::quester::CARD.id,
                    self.start_settings(slot),
                ) {
                    Ok(()) => lock_capture(&self.capture).slots[slot].start = StartProgress::Submitted,
                    Err(script::StartLoadError::Waiting(reason)) => {
                        let _ = reason;
                        lock_capture(&self.capture).slots[slot].start = StartProgress::Waiting;
                        continue;
                    }
                    Err(error) => {
                        return Err(format!(
                            "native_partner_trade typed Start unavailable for slot {slot} ({name}): {error}"
                        ));
                    }
                }
            }
            let start = lock_capture(&self.capture).slots[slot].start;
            if start == StartProgress::Submitted {
                match play.script_poll_start(name) {
                    StartPoll::Pending => continue,
                    StartPoll::Settled(StartOutcome::Ready) => {
                        lock_capture(&self.capture).slots[slot].start = StartProgress::Admitted;
                    }
                    StartPoll::Settled(outcome) => {
                        return Err(Self::start_outcome_error(name, outcome));
                    }
                    StartPoll::NotOwed => {
                        let state = play.script_state(name);
                        let status = play.script_native_status(name);
                        if matches!(state, RunState::Running | RunState::Paused)
                            && status
                                .as_ref()
                                .is_some_and(|status| status.card == script::quester::CARD.id)
                        {
                            lock_capture(&self.capture).slots[slot].start = StartProgress::Admitted;
                        } else if state == RunState::Starting
                            || (matches!(state, RunState::Running | RunState::Paused)
                                && status.is_none())
                        {
                            continue;
                        } else {
                            return Err(format!(
                                "native_partner_trade typed Start outcome was lost for slot {slot} ({name}); run state is {state:?}"
                            ));
                        }
                    }
                }
            }
        }
        let mut state = lock_capture(&self.capture);
        let admitted = state
            .slots
            .iter()
            .all(|slot| slot.start == StartProgress::Admitted);
        if admitted && !state.activation_complete {
            state.activation_complete = true;
            for slot in &mut state.slots {
                slot.active_tick_anchor = slot.last_tick;
            }
        }
        Ok(admitted)
    }

    fn observe(&mut self, play: &Play, names: &[String]) -> Result<Observation, String> {
        self.check_names(names)?;
        let mut active_count = 0;
        let mut continuing_count = 0;
        for (slot, name) in names.iter().enumerate() {
            let (start, stop_requested, lifecycle_needed) = {
                let state = lock_capture(&self.capture);
                let captured = &state.slots[slot];
                (
                    captured.start,
                    state.stop_requested,
                    state.stop_requested
                        && captured.stop_sent
                        && matches!(
                            captured.start,
                            StartProgress::Submitted | StartProgress::Admitted
                        )
                        && !captured.lifecycle_seen,
                )
            };
            let run_state = play.script_state(name);
            let native_status = play.script_native_status(name);
            let progress = play.script_progress(name);
            let lifecycle = lifecycle_needed
                .then(|| play.script_lifecycle_receipt(name))
                .flatten();
            let current_card = native_status
                .as_ref()
                .is_some_and(|status| status.card == script::quester::CARD.id);
            let phase = native_status.as_ref().map(|status| status.phase);
            let current_run = native_status.as_ref().map(|status| status.run);
            let idle_ms = progress
                .and_then(|progress| progress.idle_for)
                .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX));
            if start == StartProgress::Admitted && !stop_requested {
                if run_state == RunState::Error {
                    return Err(format!(
                        "native_partner_trade native Quester entered Error for slot {slot} ({name})"
                    ));
                }
                if run_state == RunState::Idle {
                    return Err(format!(
                        "native_partner_trade native Quester ended before normal Stop for slot {slot} ({name})"
                    ));
                }
                if matches!(run_state, RunState::Running | RunState::Paused)
                    && native_status.is_some()
                    && !current_card
                {
                    return Err(format!(
                        "native_partner_trade slot {slot} ({name}) is running a card other than Quester"
                    ));
                }
                if phase == Some(NativePhase::Blocked) {
                    let reason = native_status
                        .as_ref()
                        .and_then(|status| status.failure.as_ref())
                        .map(|failure| format!("{}: {}", failure.code, failure.message))
                        .unwrap_or_else(|| "native status has no failure detail".into());
                    return Err(format!(
                        "native_partner_trade native Quester is Blocked for slot {slot} ({name}): {reason}"
                    ));
                }
            }
            let active = start == StartProgress::Admitted
                && run_state == RunState::Running
                && current_card
                && matches!(phase, Some(NativePhase::Working | NativePhase::Waiting));
            let active_ticks = lock_capture(&self.capture).slots[slot].active_tick_advances;
            let continuing = active
                && idle_ms.is_some_and(|idle| {
                    idle <= u64::try_from(CONTINUING_FRESHNESS.as_millis()).unwrap_or(u64::MAX)
                })
                && active_ticks > 0;
            active_count += usize::from(active);
            continuing_count += usize::from(continuing);
            let mut state = lock_capture(&self.capture);
            let captured = &mut state.slots[slot];
            captured.last_run_state = Some(run_state);
            if current_card {
                captured.native_card = true;
                captured.native_phase = phase;
                captured.native_run = current_run;
            }
            captured.progress_idle_ms = idle_ms;
            if let Some(receipt) = lifecycle {
                captured.lifecycle_seen = true;
                captured.lifecycle_state = Some(receipt.state);
                captured.lifecycle_generation = Some(receipt.runtime_generation);
                captured.lifecycle_tick = Some(receipt.tick);
                captured.lifecycle_reason = Some(receipt.reason);
            }
            captured.stop_confirmed = captured.stop_sent
                && state.stop_requested
                && run_state == RunState::Idle
                && matches!(
                    captured.lifecycle_state,
                    Some(ScriptTerminalState::Stopped | ScriptTerminalState::Cancelled)
                );
        }
        let state = lock_capture(&self.capture);
        let mut observation = Observation {
            ready: 0,
            active: active_count,
            proved: 0,
            continuing: continuing_count,
        };
        for (slot, captured) in state.slots.iter().enumerate() {
            observation.ready += usize::from(captured.seed_ready);
            let proved = if state.stop_requested {
                pair_post_stop_owned(&state.slots, slot)
            } else {
                pair_proved(&state.slots, slot)
            };
            observation.proved += usize::from(proved);
        }
        Ok(observation)
    }

    fn stop(&mut self, play: &Play, names: &[String]) -> Result<(), String> {
        self.check_names(names)?;
        {
            let mut state = lock_capture(&self.capture);
            if !state.control_prepared {
                return Err("native_partner_trade cannot Stop before control setup".into());
            }
            if state.stopped {
                return Ok(());
            }
            if !state.stop_requested {
                state.stop_requested = true;
                for slot in &mut state.slots {
                    slot.stop_tick = None;
                    slot.post_stop_tick = None;
                    slot.post_stop_key_count = None;
                }
            }
        }
        let mut failures = Vec::new();
        for (slot, name) in names.iter().enumerate() {
            let (start, stop_sent) = {
                let state = lock_capture(&self.capture);
                (state.slots[slot].start, state.slots[slot].stop_sent)
            };
            if !matches!(start, StartProgress::Submitted | StartProgress::Admitted) {
                if play.script_state(name) != RunState::Idle {
                    failures.push(format!(
                        "slot {slot} ({name}) is active although this case did not submit its Start"
                    ));
                } else {
                    let mut state = lock_capture(&self.capture);
                    let captured = &mut state.slots[slot];
                    if captured.stop_tick.is_none() {
                        captured.stop_tick = captured.last_tick;
                    }
                }
                continue;
            }
            let before = play.script_state(name);
            if before == RunState::Idle && !stop_sent {
                failures.push(format!(
                    "slot {slot} ({name}) ended before this case could issue normal Stop"
                ));
                continue;
            }
            if before != RunState::Idle && !stop_sent {
                play.script_stop(name);
                play.pump_script_lifecycle(name);
                let mut state = lock_capture(&self.capture);
                let captured = &mut state.slots[slot];
                captured.stop_sent = true;
                if captured.stop_tick.is_none() {
                    captured.stop_tick = captured.last_tick;
                }
            }
            let after = play.script_state(name);
            if after != RunState::Idle {
                failures.push(format!(
                    "slot {slot} ({name}) did not reach Idle after normal Stop ({after:?})"
                ));
                continue;
            }
            match play.script_lifecycle_receipt(name) {
                Some(receipt)
                    if matches!(
                        receipt.state,
                        ScriptTerminalState::Stopped | ScriptTerminalState::Cancelled
                    ) =>
                {
                    let mut state = lock_capture(&self.capture);
                    let captured = &mut state.slots[slot];
                    captured.lifecycle_seen = true;
                    captured.lifecycle_state = Some(receipt.state);
                    captured.lifecycle_generation = Some(receipt.runtime_generation);
                    captured.lifecycle_tick = Some(receipt.tick);
                    captured.lifecycle_reason = Some(receipt.reason);
                    captured.stop_confirmed = captured.stop_sent && state.stop_requested;
                }
                Some(receipt) => failures.push(format!(
                    "slot {slot} ({name}) ended with {:?}, not normal Stop/Cancelled",
                    receipt.state
                )),
                None => failures.push(format!(
                    "slot {slot} ({name}) reached Idle without a terminal lifecycle receipt"
                )),
            }
        }
        let all_idle = names
            .iter()
            .all(|name| play.script_state(name) == RunState::Idle);
        if !all_idle {
            failures.push("one or more benchmark slots remain non-idle after Stop".into());
        }
        if all_idle {
            if let Some(previous) = self.previous_accounts.as_deref() {
                play.set_quest_accounts(previous.iter().map(String::as_str));
                let roster = play.quest_accounts();
                if roster.len() != previous.len()
                    || previous.iter().any(|name| {
                        !roster.iter().any(|account| account.0.as_ref() == name)
                    })
                {
                    failures.push("native_partner_trade could not restore the prior partner roster".into());
                } else {
                    lock_capture(&self.capture).roster_restored = true;
                }
            }
        }
        if failures.is_empty() {
            lock_capture(&self.capture).stopped = true;
            Ok(())
        } else {
            Err(format!("native_partner_trade Stop incomplete: {}", failures.join("; ")))
        }
    }

    fn evidence(&self, names: &[String]) -> Value {
        let state = lock_capture(&self.capture);
        let slots: Vec<_> = state
            .slots
            .iter()
            .enumerate()
            .map(|(slot, captured)| {
                let post_stop_owned = pair_post_stop_owned(&state.slots, slot);
                json!({
                    "slot": slot,
                    "account": names.get(slot),
                    "pair": slot / 2,
                    "role": gang_name(gang_for(slot)),
                    "partner_slot": partner_slot(slot),
                    "seed": {
                        "ready": captured.seed_ready,
                        "seed_tick": captured.seed_tick,
                        "stage_varp": if slot % 2 == 0 { "phoenixgang" } else { "blackarmgang" },
                        "stage_value": captured.stage_value,
                        "initial_key_inventory": captured.initial_key_count,
                    },
                    "native": {
                        "typed_start": format!("{:?}", captured.start),
                        "run_state": captured.last_run_state.map(|value| format!("{value:?}")),
                        "card_is_quester": captured.native_card,
                        "phase": captured.native_phase.map(|value| format!("{value:?}")),
                        "run": captured.native_run.map(|run| {
                            json!({"slot": run.slot, "run": run.run, "session": run.session})
                        }),
                        "progress_idle_ms": captured.progress_idle_ms,
                        "active_tick_advances": captured.active_tick_advances,
                    },
                    "trade": {
                        "confirm_source": CONFIRM_SOURCE,
                        "offer_open_tick_observations": captured.offer_ticks,
                        "confirm_open_events": captured.confirm_events,
                        "confirm_observed_ticks": captured.confirm_ticks,
                        "confirm_tick": captured.confirm_tick,
                        "confirm_partner_matches_configured_slot": captured.confirm_partner_matches,
                        "confirm_mine_phoenix_key_count": captured.confirm_mine_key_count,
                        "confirm_theirs_phoenix_key_count": captured.confirm_theirs_key_count,
                        "decoded_confirm_matches_role": captured.confirm_matches,
                        "inventory_key_count": captured.latest_key_count,
                        "handoff_observed": captured.handoff,
                        "handoff_tick": captured.handoff_tick,
                    },
                    "stop": {
                        "requested": state.stop_requested,
                        "stop_tick": captured.stop_tick,
                        "stop_called": captured.stop_sent,
                        "idle_after_stop": captured.stop_confirmed,
                        "lifecycle_state": captured.lifecycle_state.map(|value| format!("{value:?}")),
                        "lifecycle_generation": captured.lifecycle_generation,
                        "lifecycle_tick": captured.lifecycle_tick,
                        "lifecycle_reason": captured.lifecycle_reason,
                        "post_stop_snapshot_tick": captured.post_stop_tick,
                        "post_stop_key_inventory": captured.post_stop_key_count,
                        "pair_post_stop_ownership_proved": post_stop_owned,
                    },
                    "source_ownership": {
                        "clock": CLOCK_SOURCE,
                        "key_and_handoff": KEY_SOURCE,
                        "native_owner": NATIVE_SOURCE,
                    }
                })
            })
            .collect();
        json!({
            "scenario": "native_partner_trade",
            "account_roster_matches": names == self.names,
            "pairing": "stable adjacent pairs: slot 0/1, 2/3, ...; even slots Phoenix, odd slots Black Arm",
            "client_revision": "R289",
            "native_card": "Quester",
            "path": {"id": QUEST, "source": self.path_source, "trade": "authored blackarmgang p-key/b-key partner path"},
            "seed_profile": state.profile.map(|profile| match profile {
                TestProfile::Base40 => "Base40",
                TestProfile::Base60 => "Base60",
            }),
            "control_prepared": state.control_prepared,
            "feature_use_requested": state.activation_attempted,
            "all_typed_starts_admitted": state.activation_complete,
            "stop_requested": state.stop_requested,
            "partner_roster_restored": state.roster_restored,
            "continuing_freshness_seconds": CONTINUING_FRESHNESS.as_secs(),
            "post_stop_ownership_source": KEY_SOURCE,
            "slots": slots,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_trade_requires_even_distinct_account_rosters() {
        assert!(validate_names(&["alpha".into(), "beta".into()]).is_ok());
        assert!(validate_names(&["a".into(), "b".into(), "c".into(), "d".into()]).is_ok());
        assert!(validate_names(&["alpha".into()]).is_err());
        assert!(validate_names(&["a".into(), "b".into(), "c".into()]).is_err());
        assert!(validate_names(&["same".into(), "same".into()]).is_err());
        assert!(validate_names(&["".into(), "beta".into()]).is_err());
    }

    #[test]
    fn native_trade_pairs_remain_adjacent_for_pilot_and_measured_fleets() {
        for count in [2, 10] {
            let partners: Vec<_> = (0..count).map(partner_slot).collect();
            assert!(partners.iter().enumerate().all(|(slot, partner)| {
                *partner == if slot % 2 == 0 { slot + 1 } else { slot - 1 }
            }));
            assert!(partners.iter().all(|partner| *partner < count));
        }
    }

    #[test]
    fn native_trade_pair_requires_both_reciprocal_confirmations_and_handoffs() {
        let mut slots = vec![SlotCapture::default(), SlotCapture::default()];
        assert!(!pair_proved(&slots, 0));

        for slot in &mut slots {
            slot.confirm_matches = true;
            slot.confirm_partner_matches = true;
            slot.handoff = true;
        }
        assert!(pair_proved(&slots, 0));
        assert!(pair_proved(&slots, 1));

        slots[1].confirm_partner_matches = false;
        assert!(!pair_proved(&slots, 0));
    }

    #[test]
    fn native_trade_post_stop_proof_requires_idle_receipts_and_real_ownership() {
        let mut slots = vec![SlotCapture::default(), SlotCapture::default()];
        for slot in &mut slots {
            slot.stop_confirmed = true;
            slot.handoff = true;
        }
        slots[0].post_stop_key_count = Some(0);
        slots[1].post_stop_key_count = Some(1);

        assert!(pair_post_stop_owned(&slots, 0));
        assert!(pair_post_stop_owned(&slots, 1));
        slots[1].stop_confirmed = false;
        assert!(!pair_post_stop_owned(&slots, 0));
        slots[1].stop_confirmed = true;
        slots[1].post_stop_key_count = Some(0);
        assert!(!pair_post_stop_owned(&slots, 0));
    }
}
