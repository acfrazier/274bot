//! The live scenario's catalog Start transaction, shared by the headed panel
//! and headless `tui-play`: the stashed isolate Starts fired once on the
//! runner's StartScript step, with the configured catalog or pair witness
//! armed immediately before each actual isolate Start. Start returns before
//! V8 setup, so a started card stays stashed until its setup settles and a
//! setup failure still fails the witness that armed it.

use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use crate::catalog_core::CoreWatch;
use crate::paired_core::{pair_settings, PairCase, PairWatch, StartBarrier};
use crate::ScriptStartHandle;

/// One isolate Start the live prepare stashes for the StartScript pump.
/// Fleet runs stash one entry per started slot.
#[derive(Clone)]
pub struct PendingCatalogStart {
    pub slot: String,
    pub js: String,
    pub shape: script::LoadShape,
    pub bag: Option<Map<String, Value>>,
    pub siblings: Vec<(String, String)>,
    /// Scenario-owned loadouts for harness Start; empty uses operator store.
    pub loadouts: Vec<script::Loadout>,
    /// Compiled registry card; when set, Start uses `start_compiled`.
    pub compiled: Option<script::CompiledId>,
    /// Start was accepted; kept until its isolate setup settles so a setup
    /// failure still fails the core watch with its reason.
    pub started: bool,
    /// Fleet-only hold after the earlier stashed members started (the
    /// JiveKQ leader lets its peers prove their missing-peer bank hold).
    delay_after_peers: Option<Duration>,
    delay_started: Option<Instant>,
}

impl PendingCatalogStart {
    /// A loaded JS card (catalog, File or fixture) not yet started.
    pub fn load(
        slot: impl Into<String>,
        js: String,
        shape: script::LoadShape,
        bag: Option<Map<String, Value>>,
        siblings: Vec<(String, String)>,
        loadouts: Vec<script::Loadout>,
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
    pub fn compiled(slot: impl Into<String>, id: script::CompiledId) -> Self {
        Self {
            slot: slot.into(),
            js: String::new(),
            shape: script::LoadShape::Reject,
            bag: None,
            siblings: Vec::new(),
            loadouts: Vec::new(),
            compiled: Some(id),
            started: false,
            delay_after_peers: None,
            delay_started: None,
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

/// The same-folder sibling modules a card's isolate resolves at Start.
pub fn card_siblings(
    library: &script::JsLibrary,
    card: &script::JsCard,
) -> Result<Vec<(String, String)>, String> {
    script::resolve_sibling_modules(
        &card.path,
        &card.origin,
        library.cache(),
        script::CacheMeta {
            kind: card.kind,
            source: card.source,
            shape: None,
            api_family: Some(card.api_family.as_str().into()),
        },
    )
}

/// A live fleet launch, as panel-play and tui-play both stash it: each
/// member's catalog card transpiled, its harness bag (schema defaults,
/// `settings` overrides, `inject`) with the member's own settings merged
/// last, its sibling modules and the scenario's fixture `loadouts`, in the
/// plan's Start order. The live harness Starts a card past its catalog dim.
#[cfg(feature = "live-harness")]
pub fn fleet_catalog_starts(
    fleet: scenario::FleetStart,
    names: &[String],
    library: &mut script::JsLibrary,
    settings: &script::ScriptSettingsStore,
    inject: Option<&Map<String, Value>>,
    loadouts: &[script::Loadout],
) -> Result<Vec<PendingCatalogStart>, String> {
    let catalog = script::ScriptSource::Catalog;
    fleet
        .members(names)?
        .into_iter()
        .map(|member| {
            let name = member.card;
            library
                .ensure_js(catalog, name)
                .map_err(|e| format!("transpile {name}: {e}"))?;
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

/// Freeze the already-published prepared observation immediately before the
/// actual isolate Start call. A successful Start cannot overtake its baseline.
pub fn start_catalog_with_core<F>(watch: &CoreWatch, slot: &str, start: F) -> Result<(), String>
where
    F: FnOnce() -> Result<(), String>,
{
    watch.begin_start(slot)?;
    let result = start();
    if let Err(error) = &result {
        watch.fail_start(slot, error.clone());
    }
    result
}

fn start_stashed_catalog_card(
    handle: &ScriptStartHandle,
    card: &PendingCatalogStart,
) -> Result<(), String> {
    if let Some(id) = card.compiled {
        return handle.start_compiled(&card.slot, id);
    }
    let result = if card.loadouts.is_empty() {
        handle.start_load(
            &card.slot,
            card.js.clone(),
            card.shape,
            card.bag.clone(),
            card.siblings.clone(),
        )
    } else {
        handle.start_load_with_loadouts(
            &card.slot,
            card.js.clone(),
            card.shape,
            card.bag.clone(),
            card.siblings.clone(),
            &card.loadouts,
        )
    };
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

/// Pump the stashed Starts once per driven-slot frame. When the runner is
/// on its StartScript step (`on_start_script`), start every stashed isolate
/// with its witness armed; earlier, only settle already-started setups.
/// Returns false when Start was attempted and failed (or the pair is not
/// ready yet), so the pump must not consume the one-tick wait.
pub fn fire_pending_catalog_start(
    pending: &mut Vec<PendingCatalogStart>,
    on_start_script: bool,
    arming: impl FnOnce() -> StartArming,
) -> bool {
    let settling = pending.iter().any(|card| card.started);
    let starting = on_start_script && pending.iter().any(|card| !card.started);
    if !settling && !starting {
        return true;
    }
    let arming = arming();
    if settling {
        if let Some(handle) = arming.handle.as_ref() {
            settle_started_catalog_cards(pending, handle, &arming);
        }
    }
    if !starting {
        return true;
    }
    let Some(handle) = arming.handle.as_ref() else {
        return false;
    };
    if let Some(pair) = arming.pair.as_ref().filter(|pair| pair.configured()) {
        if pending.len() != 2 {
            pair.fail_start("pair core requires actual scripts on both visible slots");
            return false;
        }
        match pair.barrier() {
            StartBarrier::Wait => return false,
            StartBarrier::RejectStartedWhileUnready => {
                pair.fail_start("pair core Start while the counterpart is unready");
                return false;
            }
            StartBarrier::StartBoth => {}
        }
        if pair
            .begin_shared_start(&pending[0].slot, &pending[1].slot)
            .is_err()
        {
            return false;
        }
        for card in pending.iter_mut() {
            if let Err(error) = start_stashed_catalog_card(handle, card) {
                pair.fail_start(error);
                return false;
            }
            card.started = true;
        }
        return true;
    }
    let watch = arming.catalog.clone().unwrap_or_default();
    for card in pending.iter_mut().filter(|card| !card.started) {
        if card.delaying(Instant::now()) {
            return false;
        }
        if start_catalog_with_core(&watch, &card.slot, || {
            start_stashed_catalog_card(handle, card)
        })
        .is_err()
        {
            return false;
        }
        card.started = true;
    }
    true
}

/// Drop started cards whose setup settled; a failed setup fails the watch
/// that armed its Start (the refusal path `fail_start` already covers).
fn settle_started_catalog_cards(
    pending: &mut Vec<PendingCatalogStart>,
    handle: &ScriptStartHandle,
    arming: &StartArming,
) {
    let mut failed = Vec::new();
    pending.retain(|card| {
        if !card.started {
            return true;
        }
        match handle.poll_start(&card.slot) {
            script::StartPoll::Pending => true,
            script::StartPoll::Settled(script::StartOutcome::Failed(error)) => {
                failed.push((card.slot.clone(), error));
                false
            }
            script::StartPoll::Settled(
                script::StartOutcome::Ready | script::StartOutcome::Cancelled,
            )
            | script::StartPoll::NotOwed => false,
        }
    });
    for (slot, error) in failed {
        if let Some(pair) = arming.pair.as_ref().filter(|pair| pair.configured()) {
            pair.fail_start(error);
        } else if let Some(watch) = arming.catalog.as_ref() {
            watch.fail_start(&slot, error);
        }
    }
}

/// Both slots of a paired proof with complementary role bags. The same bags
/// go through the pair watch so its receipt reports the settings the
/// isolates actually started with.
pub fn pair_starts(
    names: &[String],
    case: PairCase,
    js: String,
    shape: script::LoadShape,
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
