//! Shared catalog Start/Stop transaction for the panel per-frame live pump
//! and headless `tui-play`. Stashed compiled cards and isolates Start once on
//! the runner's StartScript step, with catalog/pair witnesses armed first.
//! Start returns before preparation/setup: retain each card until `poll_start`
//! settles so failure belongs to the witness that armed that lifetime.

use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use crate::catalog_core::CoreWatch;
use crate::paired_core::{PairWatch, StartBarrier};
#[cfg(feature = "js")]
use crate::paired_core::{pair_settings, PairCase};
use crate::ScriptStartHandle;

/// One stashed script Start for the live prepare/StartScript pump.
/// Compiled cards and JS builds' loaded cards share the same lifecycle.
#[derive(Clone)]
pub struct PendingCatalogStart {
    pub slot: String,
    #[cfg(feature = "js")]
    pub js: String,
    #[cfg(feature = "js")]
    pub shape: script_js::LoadShape,
    pub bag: Option<Map<String, Value>>,
    #[cfg(feature = "js")]
    pub siblings: Vec<(String, String)>,
    #[cfg(feature = "js")]
    /// Scenario-owned loadouts for harness Start; empty uses operator store.
    pub loadouts: Vec<api::loadout::Loadout>,
    /// Compiled registry card; when set, Start uses `start_compiled`.
    pub compiled: Option<script::CompiledId>,
    /// Start was accepted; retain the card so a later Stop/Start can reuse it.
    pub started: bool,
    pub settled: bool,
    /// A temporary dispatch refusal, retained separately from setup failure.
    pub waiting_reason: Option<String>,
    /// Fleet-only hold after the earlier stashed members started (the
    /// JiveKQ leader lets its peers prove their missing-peer bank hold).
    delay_after_peers: Option<Duration>,
    delay_started: Option<Instant>,
}

impl PendingCatalogStart {
    #[cfg(feature = "js")]
    /// A loaded JS card (catalog, File or fixture) not yet started.
    pub fn load(
        slot: impl Into<String>,
        js: String,
        shape: script_js::LoadShape,
        bag: Option<Map<String, Value>>,
        siblings: Vec<(String, String)>,
        loadouts: Vec<api::loadout::Loadout>,
    ) -> Self {
        Self {
            slot: slot.into(),
            js,
            shape,
            bag,
            siblings,
            loadouts,
            compiled: None,
            started: false,
            settled: false,
            waiting_reason: None,
            delay_after_peers: None,
            delay_started: None,
        }
    }

    /// Hold this Start until `delay` has passed since the StartScript pump
    /// first reached it, i.e. after every member stashed before it started.
    pub fn with_delay_after_peers(mut self, delay: Option<Duration>) -> Self {
        self.delay_after_peers = delay;
        self
    }

    /// A compiled registry card not yet started.
    pub fn compiled(
        slot: impl Into<String>,
        id: script::CompiledId,
        bag: Map<String, Value>,
    ) -> Self {
        Self {
            slot: slot.into(),
            #[cfg(feature = "js")]
            js: String::new(),
            #[cfg(feature = "js")]
            shape: script_js::LoadShape::Reject,
            bag: Some(bag),
            #[cfg(feature = "js")]
            siblings: Vec::new(),
            #[cfg(feature = "js")]
            loadouts: Vec::new(),
            compiled: Some(id),
            started: false,
            settled: false,
            waiting_reason: None,
            delay_after_peers: None,
            delay_started: None,
        }
    }

    fn waiting(&mut self, reason: String) {
        if self.waiting_reason.as_ref() != Some(&reason) {
            eprintln!("[live] Start held: {reason}");
            self.waiting_reason = Some(reason);
        }
    }

    /// Whether this Start still waits out its fleet delay: the first call
    /// starts the clock, and the Start proceeds once the delay has passed.
    fn delaying(&mut self, now: Instant) -> bool {
        let Some(delay) = self.delay_after_peers else {
            return false;
        };
        let began = *self.delay_started.get_or_insert_with(|| {
            eprintln!(
                "[live] delaying fleet Start for {} by {}s",
                self.slot,
                delay.as_secs()
            );
            now
        });
        if now.duration_since(began) < delay {
            return true;
        }
        eprintln!("[live] releasing delayed fleet Start for {}", self.slot);
        self.delay_after_peers = None;
        false
    }
}

#[cfg(feature = "js")]
/// The same-folder sibling modules a card's isolate resolves at Start.
pub fn card_siblings(
    library: &script_js::JsLibrary,
    card: &script_js::JsCard,
) -> Result<Vec<(String, String)>, String> {
    script_js::resolve_sibling_modules(
        &card.path,
        &card.origin,
        library.cache(),
        script_js::CacheMeta {
            kind: card.kind,
            source: card.source,
            shape: None,
            api_family: Some(card.api_family.as_str().into()),
        },
    )
}

/// A live fleet launch, as panel-play and tui-play both stash it: each
/// member's catalog card has already been prepared on WorkAdmission before
/// calling this helper. It merges each member's harness bag (schema defaults,
/// `settings` overrides, `inject`) with the member's own settings last,
/// plus sibling modules and fixture `loadouts`, in the plan's Start order.
/// The live harness Starts a card past its catalog dim.
#[cfg(all(feature = "js", feature = "live-harness"))]
pub fn fleet_catalog_starts(
    fleet: scenario::FleetStart,
    names: &[String],
    library: &mut script_js::JsLibrary,
    settings: &script::ScriptSettingsStore,
    inject: Option<&Map<String, Value>>,
    loadouts: &[api::loadout::Loadout],
) -> Result<Vec<PendingCatalogStart>, String> {
    let catalog = script::ScriptSource::Catalog;
    fleet
        .members(names)?
        .into_iter()
        .map(|member| {
            let name = member.card;
            if !library.js_is_ready(catalog, name) {
                return Err(format!("catalog card {name} is not prepared"));
            }
            let card = library
                .get(catalog, name)
                .cloned()
                .ok_or_else(|| format!("$RS2B0T catalog has no {name} card"))?;
            let mut bag = settings.merged_bag(catalog, name, &card.settings_schema, inject);
            bag.extend(
                member
                    .settings
                    .into_iter()
                    .map(|(key, value)| (key.to_string(), Value::String(value))),
            );
            let siblings = card_siblings(library, &card)?;
            Ok(PendingCatalogStart::load(
                names[member.slot].clone(),
                card.js,
                card.shape,
                Some(bag),
                siblings,
                loadouts.to_vec(),
            )
            .with_delay_after_peers(member.delay_after_peers))
        })
        .collect()
}

/// The Play-owned handles a Start or setup settlement needs. The pump asks
/// for them only on those paths, not on every frame.
#[derive(Clone, Default)]
pub struct StartArming {
    pub handle: Option<ScriptStartHandle>,
    pub catalog: Option<CoreWatch>,
    pub pair: Option<PairWatch>,
}

fn start_stashed_catalog_card(
    handle: &ScriptStartHandle,
    card: &PendingCatalogStart,
    before_start: impl FnOnce() -> Result<(), String>,
) -> Result<(), script::StartLoadError> {
    use crate::play_scripts::StartRequest;
    #[cfg(feature = "js")]
    let request = || {
        if let Some(id) = card.compiled {
            StartRequest::Compiled {
                id,
                bag: card.bag.clone().unwrap_or_default(),
            }
        } else {
            StartRequest::Load {
                source: card.js.clone(),
                shape: card.shape,
                bag: card.bag.clone(),
                siblings: card.siblings.clone(),
                loadouts: (!card.loadouts.is_empty()).then_some(card.loadouts.as_slice()),
            }
        }
    };
    #[cfg(not(feature = "js"))]
    let Some(id) = card.compiled else {
        return Err(script::surface::SurfaceError::NoFactory(
            script::surface::SurfaceKind::Js,
        )
        .into());
    };
    #[cfg(not(feature = "js"))]
    let request = || StartRequest::Compiled {
        id,
        bag: card.bag.clone().unwrap_or_default(),
        _lifetime: std::marker::PhantomData,
    };
    let result = handle.dispatch_start(&card.slot, request, before_start);
    #[cfg(feature = "js")]
    if result.is_ok() && crate::debug_enabled() {
        let fixture_names = card
            .loadouts
            .iter()
            .map(|loadout| loadout.name.as_str())
            .collect::<Vec<_>>();
        let selected_loadout = card
            .bag
            .as_ref()
            .and_then(|bag| bag.get("loadout"))
            .and_then(Value::as_str)
            .unwrap_or("<unset>");
        eprintln!(
            "[live] catalog start slot={} fixtures={} names={fixture_names:?} loadout={selected_loadout:?}",
            card.slot,
            card.loadouts.len()
        );
    }
    result
}

/// What the live pump should do after one catalog Start frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartScriptPump {
    /// Tick the runner this frame (seed, File StartScript, later steps).
    Continue,
    /// Do not tick (missing handle, pair barrier, delayed fleet Start, or
    /// a Start held by an operator hold it must wait out).
    Hold,
    /// A compiled card settled Ready; latch ScriptRunning then tick.
    CompiledRunning,
    /// A Start was refused or failed; fail the StartScript step with this
    /// reason instead of holding the runner (which would starve its
    /// deadline and hang the cell).
    Failed(String),
}

/// Pump stashed catalog lifetimes once per driven-slot frame. Start all pending
/// cards on StartScript; otherwise settle already-started preparation/setup.
/// StopScript revokes each slot and resets its pending Start for the restart.
/// Hold the runner while Start/Stop cannot advance, including preparation or
/// witness readiness, and report Running/Failed outcomes explicitly. A
/// refused or failed Start is never a Hold: the runner must fail its
/// StartScript step with the reason, not wait out a deadline it cannot
/// reach while held.
pub fn fire_pending_catalog_start(
    pending: &mut [PendingCatalogStart],
    on_start_script: bool,
    on_stop_script: bool,
    arming: impl FnOnce() -> StartArming,
) -> StartScriptPump {
    if on_stop_script {
        let Some(handle) = arming().handle else {
            return StartScriptPump::Hold;
        };
        for card in pending.iter_mut() {
            if handle.stop(&card.slot).is_err() {
                return StartScriptPump::Hold;
            }
            card.started = false;
            card.settled = false;
            card.delay_started = None;
            card.waiting_reason = None;
        }
        return StartScriptPump::Continue;
    }
    let settling = pending.iter().any(|card| card.started && !card.settled);
    let starting = on_start_script && pending.iter().any(|card| !card.started);
    if !settling && !starting {
        return StartScriptPump::Continue;
    }
    let arming = arming();
    if settling {
        if let Some(handle) = arming.handle.as_ref() {
            if let Some(pump) = settle_started_catalog_cards(pending, handle, &arming) {
                return pump;
            }
        }
    }
    if !starting {
        return StartScriptPump::Continue;
    }
    let Some(handle) = arming.handle.as_ref() else {
        return StartScriptPump::Hold;
    };
    if let Some(pair) = arming.pair.as_ref().filter(|pair| pair.configured()) {
        if pending.len() != 2 {
            pair.fail_start("pair core requires actual scripts on both visible slots");
            return StartScriptPump::Hold;
        }
        if pending.iter().all(|card| !card.started) {
            match pair.barrier() {
                StartBarrier::Wait => return StartScriptPump::Hold,
                StartBarrier::RejectStartedWhileUnready => {
                    pair.fail_start("pair core Start while the counterpart is unready");
                    return StartScriptPump::Hold;
                }
                StartBarrier::StartBoth => {}
            }
        }
        for index in 0..pending.len() {
            if pending[index].started {
                continue;
            }
            if let Err(error) = start_stashed_catalog_card(handle, &pending[index], || {
                if index == 0 {
                    pair.begin_shared_start(&pending[0].slot, &pending[1].slot)
                } else {
                    Ok(())
                }
            }) {
                match error {
                    script::StartLoadError::Waiting(reason) => pending[index].waiting(reason),
                    script::StartLoadError::NoLoadouts(reason) => {
                        pair.fail_start(reason.clone());
                        return StartScriptPump::Failed(format!("start failed: {reason}"));
                    }
                    other => {
                        if pending.iter().all(|card| !card.started) {
                            pair.retry_start();
                        }
                        pending[index].waiting(other.to_string());
                    }
                }
                return StartScriptPump::Hold;
            }
            pending[index].started = true;
            pending[index].waiting_reason = None;
            if index == 0 {
                pair.accept_start();
            }
        }
        return StartScriptPump::Continue;
    }
    let watch = arming.catalog.clone().unwrap_or_default();
    for card in pending.iter_mut().filter(|card| !card.started) {
        if card.delaying(Instant::now()) {
            return StartScriptPump::Hold;
        }
        if let Err(error) =
            start_stashed_catalog_card(handle, card, || watch.begin_start(&card.slot))
        {
            match error {
                script::StartLoadError::Waiting(reason) => {
                    card.waiting(reason);
                    return StartScriptPump::Hold;
                }
                script::StartLoadError::NoLoadouts(reason) => {
                    watch.fail_start(&card.slot, reason.clone());
                    return StartScriptPump::Failed(format!("start failed: {reason}"));
                }
                other => {
                    watch.retry_start(&card.slot);
                    card.waiting(other.to_string());
                    return StartScriptPump::Hold;
                }
            }
        }
        card.started = true;
        card.waiting_reason = None;
    }
    StartScriptPump::Continue
}

/// Settle setup once, keeping the card for a subsequent Stop/Start transaction.
/// A failed setup fails the watch that armed its Start and returns a pump
/// notice so the scenario fails its StartScript step with the reason.
/// Compiled cards also return a Running notice so the scenario can latch it.
fn settle_started_catalog_cards(
    pending: &mut [PendingCatalogStart],
    handle: &ScriptStartHandle,
    arming: &StartArming,
) -> Option<StartScriptPump> {
    let mut settle_pump = None;
    for card in pending
        .iter_mut()
        .filter(|card| card.started && !card.settled)
    {
        let compiled = card.compiled.is_some();
        let error = match handle.poll_start(&card.slot) {
            script::StartPoll::Pending => continue,
            script::StartPoll::Settled(script::StartOutcome::Failed(error)) => {
                settle_pump = Some(StartScriptPump::Failed(format!("start failed: {error}")));
                Some(error)
            }
            script::StartPoll::Settled(script::StartOutcome::Rejected(error)) => {
                settle_pump = Some(StartScriptPump::Failed(format!("start rejected: {error}")));
                Some(error.to_string())
            }
            script::StartPoll::Settled(script::StartOutcome::Ready) => {
                if compiled {
                    settle_pump = Some(StartScriptPump::CompiledRunning);
                }
                None
            }
            script::StartPoll::Settled(script::StartOutcome::Cancelled) => {
                settle_pump = Some(StartScriptPump::Failed("start cancelled".into()));
                None
            }
            script::StartPoll::NotOwed => {
                if compiled {
                    settle_pump = Some(match handle.run_state(&card.slot) {
                        script::RunState::Running | script::RunState::Paused => {
                            StartScriptPump::CompiledRunning
                        }
                        _ => StartScriptPump::Failed(handle.last_error(&card.slot).map_or_else(
                            || "start did not reach running".into(),
                            |error| format!("start rejected: {error}"),
                        )),
                    });
                }
                None
            }
        };
        card.settled = true;
        if let Some(error) = error {
            if let Some(pair) = arming.pair.as_ref().filter(|pair| pair.configured()) {
                pair.fail_start(error);
            } else if let Some(watch) = arming.catalog.as_ref() {
                watch.fail_start(&card.slot, error);
            }
        }
    }
    settle_pump
}

#[cfg(feature = "js")]
/// Both slots of a paired proof with complementary role bags. The same bags
/// go through the pair watch so its receipt reports the settings the
/// isolates actually started with.
pub fn pair_starts(
    names: &[String],
    case: PairCase,
    js: String,
    shape: script_js::LoadShape,
    schema: &[script::SettingDef],
    siblings: Vec<(String, String)>,
    watch: &PairWatch,
) -> Result<Vec<PendingCatalogStart>, String> {
    let [a, b] = names else {
        return Err("pair core watch supports exactly two driven slots".into());
    };
    let a_bag = pair_settings(case, schema, 0, a, b)?;
    let b_bag = pair_settings(case, schema, 1, b, a)?;
    watch.install_prepared_settings(a, a_bag.clone(), b, b_bag.clone())?;
    Ok(vec![
        PendingCatalogStart::load(
            a.clone(),
            js.clone(),
            shape,
            Some(a_bag),
            siblings.clone(),
            Vec::new(),
        ),
        PendingCatalogStart::load(b.clone(), js, shape, Some(b_bag), siblings, Vec::new()),
    ])
}

#[cfg(test)]
#[path = "live_start_tests.rs"]
mod tests;
