use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use api::host_log;
use api::hostlog::{Category, Level};

use crate::play_status::lock_statuses;
use crate::script_runtime;
use crate::script_runtime::{
    pause_script, poll_start, reset_script_nav, script_slot, script_slot_or_insert, NavBot,
    ScriptWall,
};
use crate::Play;

#[cfg(feature = "test-support")]
use std::sync::LazyLock;

#[cfg(feature = "test-support")]
use api::snapshot::GameSnapshot;
#[cfg(feature = "test-support")]
use script::combat::{tables::PotionKind, GuardOp};

#[cfg(feature = "test-support")]
type GuardActionEvents = Arc<parking_lot::Mutex<Vec<GuardDrinkAction>>>;

#[cfg(feature = "test-support")]
static GUARD_ACTION_OBSERVERS: LazyLock<parking_lot::Mutex<HashMap<String, GuardActionEvents>>> =
    LazyLock::new(Default::default);

#[cfg(feature = "test-support")]
fn guard_action_observers() -> &'static parking_lot::Mutex<HashMap<String, GuardActionEvents>> {
    &GUARD_ACTION_OBSERVERS
}

/// One accepted antipoison Drink emitted by the native host walk guard.
#[cfg(feature = "test-support")]
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct GuardDrinkAction {
    /// Snapshot tick for the accepted action.
    pub snapshot_tick: u32,
    /// Potion name that the guard sent.
    pub item_name: String,
}

/// Scoped reader for accepted host guard antipoison Drink actions.
#[cfg(feature = "test-support")]
pub struct GuardActionObserver {
    account: String,
    actions: GuardActionEvents,
}

#[cfg(feature = "test-support")]
impl GuardActionObserver {
    /// Read the actions captured for this observer's account.
    pub fn actions(&self) -> Vec<GuardDrinkAction> {
        self.actions.lock().clone()
    }
}

#[cfg(feature = "test-support")]
impl Drop for GuardActionObserver {
    fn drop(&mut self) {
        let mut observers = guard_action_observers().lock();
        if observers
            .get(&self.account)
            .is_some_and(|actions| Arc::ptr_eq(actions, &self.actions))
        {
            observers.remove(&self.account);
        }
    }
}

/// Observe accepted host guard antipoison Drinks for one account in tests.
#[cfg(feature = "test-support")]
pub fn register_guard_action_observer_for_test(account: &str) -> GuardActionObserver {
    let actions = Arc::new(parking_lot::Mutex::new(Vec::new()));
    guard_action_observers()
        .lock()
        .insert(account.to_owned(), Arc::clone(&actions));
    GuardActionObserver {
        account: account.to_owned(),
        actions,
    }
}

/// Record an accepted antipoison Drink from the host walk guard in tests.
#[cfg(feature = "test-support")]
pub(crate) fn notify_guard_action_for_test(account: &str, snapshot: &GameSnapshot, op: &GuardOp) {
    let GuardOp::Drink {
        name,
        kind: PotionKind::Antipoison | PotionKind::SuperAntipoison,
    } = op
    else {
        return;
    };
    let observer = guard_action_observers().lock().get(account).cloned();
    if let Some(observer) = observer {
        observer.lock().push(GuardDrinkAction {
            snapshot_tick: snapshot.tick(),
            item_name: name.to_string(),
        });
    }
}

fn invalidate_bank_pick(navs: &Arc<Mutex<HashMap<String, NavBot>>>, name: &str) {
    if let Some(bot) = navs.lock().unwrap().get_mut(name) {
        bot.bank_pick.reset();
    }
}

/// End a script run's published rows on `name`'s status row (paint and walk
/// destination). Takes only the statuses lock, so a caller that holds a script
/// slot may call it (order: script, navigation, statuses).
pub(crate) fn retire_script_run_status(statuses: &Arc<Mutex<Vec<crate::SlotStatus>>>, name: &str) {
    if let Some(status) = lock_statuses(statuses)
        .iter_mut()
        .find(|row| row.username == name)
    {
        status.retire_script_run();
    }
}

/// Cloneable overlay source for a catalog Traveller (`InteractReq::Walk`).
/// The panel projects the borrowed route only while painting it, so compact
/// plans are never expanded into legacy `Route` allocations.
#[derive(Clone)]
pub struct ScriptNavPaint {
    pub(crate) navs: Arc<Mutex<HashMap<String, NavBot>>>,
}

/// One prepared native-card configuration; an optional Quester registry is
/// published only after the WorkAdmission result is consumed by its owner.
pub struct PreparedScriptConfig {
    config: Arc<script::native::PreparedConfig>,
    registry: Option<script::quester::registry::PreparedRegistry>,
}

impl PreparedScriptConfig {
    /// Commit a current Quester snapshot after the caller's operation and run
    /// identity fences pass. A stale or cancelled result is dropped instead.
    pub fn commit(
        self,
        identity: api::identity::WorkIdentity,
    ) -> Result<Arc<script::native::PreparedConfig>, script::native::StartError> {
        if let Some(registry) = self.registry {
            script::quester::registry::commit_snapshot(registry, identity, identity).map_err(
                |error| {
                    script::native::StartError::Unavailable(
                        format!(
                            "{}: {}",
                            error.code,
                            error.detail.as_deref().unwrap_or("Path reload failed")
                        )
                        .into(),
                    )
                },
            )?;
        }
        Ok(self.config)
    }
}

/// Cloneable isolate-start handle for slot-thread live pumps that cannot
/// hold `&Play` (the per-frame hook is built before `Play` is stored).
#[derive(Clone)]
pub struct ScriptStartHandle {
    scripts: ScriptWall,
    hold: Arc<Mutex<std::sync::Weak<String>>>,
    navs: Arc<Mutex<HashMap<String, NavBot>>>,
    statuses: Arc<Mutex<Vec<crate::SlotStatus>>>,
    game_data: Option<Arc<api::game_data::SelectedGameData>>,
    named_banks: Arc<api::named_banks::NamedBankFacts>,
    /// The Play's state composition: a Start without caller-owned loadouts
    /// leases the operator's rows from it. `None` for the legacy direct
    /// connection, whose Starts post no operator rows.
    state: Option<Arc<super::local_state::StateComposition>>,
}

pub(crate) enum StartRequest<'a> {
    #[cfg(feature = "js")]
    Load {
        source: String,
        shape: script_js::LoadShape,
        bag: Option<serde_json::Map<String, serde_json::Value>>,
        siblings: Vec<(String, String)>,
        loadouts: Option<&'a [api::loadout::Loadout]>,
    },
    Compiled {
        id: script::CompiledId,
        bag: serde_json::Map<String, serde_json::Value>,
        #[cfg(not(feature = "js"))]
        _lifetime: std::marker::PhantomData<&'a ()>,
    },
}
fn validate_script_loadout_setting(
    rows: &[api::loadout::Loadout],
    bag: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Result<(), script::loadouts_store::LoadoutResolutionError> {
    let wanted = bag.and_then(|bag| bag.get("loadout").map(|value| value.as_str().unwrap_or("")));
    script::loadouts_store::resolve_script_loadout_setting(rows, wanted).map(|_| ())
}

impl ScriptStartHandle {
    /// The operator's loadout rows for a Start on `name`'s slot: the state
    /// composition's current lease (a readback, so a row the frontend's
    /// form just saved is seen), or no rows for a composition without a
    /// state service.
    fn operator_loadouts(&self, name: &str) -> Result<Vec<api::loadout::Loadout>, String> {
        match &self.state {
            Some(state) => state.operator_loadouts(name),
            None => Ok(Vec::new()),
        }
    }

    #[cfg(feature = "js")]
    /// Start a loaded JS bot on `name`'s slot. Same isolate spawn as
    /// [`Play::script_start_load`], without the control-thread wake
    /// (the slot thread is already pumping). Operator loadouts come from
    /// the process state service; harness scenario Start uses
    /// [`Self::start_load_with_loadouts`] when the scenario owns fixture
    /// loadouts.
    pub fn start_load(
        &self,
        name: &str,
        source: String,
        shape: script_js::LoadShape,
        settings_bag: Option<serde_json::Map<String, serde_json::Value>>,
        siblings: Vec<(String, String)>,
    ) -> Result<(), String> {
        self.dispatch_start(
            name,
            || StartRequest::Load {
                source,
                shape,
                bag: settings_bag,
                siblings,
                loadouts: None,
            },
            || Ok(()),
        )
        .map_err(|error| error.to_string())
    }

    #[cfg(feature = "js")]
    /// Harness catalog Start with caller-owned loadouts. It validates the
    /// saved setting against those rows before starting, then posts the bag.
    /// Does not affect [`Play::script_start_load_typed`].
    pub fn start_load_with_loadouts(
        &self,
        name: &str,
        source: String,
        shape: script_js::LoadShape,
        settings_bag: Option<serde_json::Map<String, serde_json::Value>>,
        siblings: Vec<(String, String)>,
        loadouts: &[api::loadout::Loadout],
    ) -> Result<(), String> {
        self.dispatch_start(
            name,
            || StartRequest::Load {
                source,
                shape,
                bag: settings_bag,
                siblings,
                loadouts: Some(loadouts),
            },
            || Ok(()),
        )
        .map_err(|error| error.to_string())
    }

    /// Prepare a compiled card off-pump. Ready/failure settles through
    /// `poll_start`; Stop or removal can revoke the captured slot meanwhile.
    pub fn start_compiled(
        &self,
        name: &str,
        id: script::CompiledId,
        bag: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String> {
        self.dispatch_start(
            name,
            || StartRequest::Compiled {
                id,
                bag,
                #[cfg(not(feature = "js"))]
                _lifetime: std::marker::PhantomData,
            },
            || Ok(()),
        )
        .map_err(|error| error.to_string())
    }

    /// The single admission point for operator, coordinator and scenario Starts.
    /// Keep the hold lock through admission so publishing a confirmation cannot
    /// race a slot-thread Start between its decision and the actual slot call.
    pub(crate) fn dispatch_start<'a>(
        &self,
        name: &str,
        request: impl FnOnce() -> StartRequest<'a>,
        before_start: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), script::StartLoadError> {
        use script::StartLoadError::{NoLoadouts, Refused, Waiting};
        let hold = self
            .hold
            .lock()
            .map_err(|_| Refused("script start hold unavailable".into()))?;
        if let Some(reason) = hold.upgrade() {
            return Err(Waiting(format!("{name}: blocked, {reason}")));
        }
        before_start().map_err(Refused)?;
        let request = request();
        host_log!(
            Category::ScriptLifecycle,
            Level::Info,
            slot = name,
            "start script"
        );
        let slot = match &request {
            #[cfg(feature = "js")]
            StartRequest::Load { .. } => script_slot_or_insert(&self.scripts, name),
            StartRequest::Compiled { .. } => script_slot(&self.scripts, name)
                .ok_or_else(|| Refused(format!("no slot: {name}")))?,
        };
        let mut slot = slot
            .lock()
            .map_err(|_| Refused(format!("script slot retiring: {name}")))?;
        let result = match request {
            StartRequest::Compiled { id, bag, .. } => {
                // Every compiled Start reads the operator's loadouts file
                // again, so the run's later leases (a quester compiling a
                // Path, the progress page) see the rows the frontend's form
                // saved, whether or not the bag names a loadout. Only a bag
                // that names one needs the rows now: without one, a failed
                // read is logged and the card's own lease reports it.
                let refreshed = self.operator_loadouts(name);
                let validated = if bag.contains_key("loadout") {
                    refreshed.map_err(Refused).and_then(|rows| {
                        validate_script_loadout_setting(&rows, Some(&bag)).map_err(|error| {
                            match error {
                                error @ script::loadouts_store::LoadoutResolutionError::NoLoadouts {
                                    ..
                                } => NoLoadouts(error.to_string()),
                                other => Refused(other.to_string()),
                            }
                        })
                    })
                } else {
                    if let Err(error) = &refreshed {
                        host_log!(
                            Category::ScriptLifecycle,
                            Level::Warn,
                            slot = name,
                            "{error}; the compiled Start goes on without fresh loadouts"
                        );
                    }
                    Ok(())
                };
                if let Err(error) = validated {
                    Err(error)
                } else if script::compiled_card(id).is_none() {
                    Err(Refused(format!("not ported: {}", id.0)))
                } else if let Some(selected) = self.game_data.clone() {
                    slot.start_compiled(
                        name,
                        id,
                        Arc::new(bag),
                        selected,
                        Arc::clone(&self.named_banks),
                    )
                    .map_err(|error| Refused(error.to_string()))
                } else {
                    Err(Refused("selected game data unavailable".into()))
                }
            }
            #[cfg(feature = "js")]
            StartRequest::Load {
                source,
                shape,
                bag,
                siblings,
                loadouts,
            } => {
                // Caller-owned rows (a harness scenario's fixtures) or the
                // operator's current rows from the state service.
                let loadouts: Result<Arc<[api::loadout::Loadout]>, String> = match loadouts {
                    Some(loadouts) => Ok(Arc::from(loadouts.to_vec())),
                    None => self.operator_loadouts(name).map(Arc::from),
                };
                match loadouts {
                    Ok(loadouts) => {
                        slot.start_surface_with_settings(script_runtime::js_surface_start(
                            script_runtime::js_start_card(source, shape, siblings),
                            bag.map(Arc::new),
                            self.game_data.clone(),
                            Arc::clone(&self.named_banks),
                            loadouts,
                        ))
                    }
                    Err(error) => Err(Refused(error)),
                }
            }
        };
        if result.is_ok() {
            invalidate_bank_pick(&self.navs, name);
        }
        if let Err(error) = &result {
            host_log!(stderr; Category::ScriptLifecycle, Level::Error, slot = name, "start failed: {error}");
        }
        result
    }
    /// Install a native card fixture through the same SlotScript lifecycle as
    /// a compiled Start. This seam is test-only: it does not add a registry
    /// card or another production start path.
    // Test seam for fixture-only native starts; no production registry path.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn start_test_script(
        &self,
        name: &str,
        script: Box<dyn script::native::Script>,
        selected: Option<Arc<api::game_data::SelectedGameData>>,
    ) -> Result<api::selected::RunKey, String> {
        self.start_test_script_with_card(name, script::CompiledId("test"), script, selected)
    }

    /// Install a fixture under an explicit compiled card identity.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn start_test_script_with_card(
        &self,
        name: &str,
        card: script::CompiledId,
        script: Box<dyn script::native::Script>,
        selected: Option<Arc<api::game_data::SelectedGameData>>,
    ) -> Result<api::selected::RunKey, String> {
        let slot = script_slot_or_insert(&self.scripts, name);
        let mut slot = slot
            .lock()
            .map_err(|_| format!("script slot retiring: {name}"))?;
        slot.start_test_script_with_card(card, script, selected)?;
        slot.native_run()
            .ok_or_else(|| format!("test script did not publish a run: {name}"))
    }

    /// `name`'s latest Start, read atomically (see [`Play::script_poll_start`]).
    pub fn poll_start(&self, name: &str) -> script::StartPoll {
        poll_start(&self.scripts, name)
    }

    /// `name`'s script lifecycle, `Idle` when the slot is missing or retiring.
    pub fn run_state(&self, name: &str) -> script::RunState {
        script_slot(&self.scripts, name)
            .and_then(|slot| slot.lock().ok().map(|slot| slot.state()))
            .unwrap_or(script::RunState::Idle)
    }

    /// `name`'s script `last_error`; `None` when the slot has none.
    pub fn last_error(&self, name: &str) -> Option<String> {
        script_slot(&self.scripts, name).and_then(|slot| {
            slot.lock()
                .ok()
                .and_then(|slot| slot.last_error().map(str::to_string))
        })
    }

    /// Stop from a slot-thread pump, revoking both script and navigation work.
    pub fn stop(&self, name: &str) -> Result<(), String> {
        let slot = script_slot(&self.scripts, name).ok_or_else(|| format!("no slot: {name}"))?;
        let mut slot = slot
            .lock()
            .map_err(|_| format!("script slot retiring: {name}"))?;
        slot.stop();
        reset_script_nav(&self.navs, name, None);
        drop(slot);
        retire_script_run_status(&self.statuses, name);
        Ok(())
    }

    pub fn read_journal(&self, name: &str, target: api::selected::RunKey) -> Result<(), String> {
        let slot = script_slot(&self.scripts, name).ok_or("stale native run")?;
        let mut slot = slot.lock().map_err(|_| "script slot retiring")?;
        slot.read_journal_compiled(target)
            .map_err(|failure| failure.message.to_string())
    }

    /// Fail closed unless the slot is observed Idle with no native run.
    pub fn idle(&self, name: &str) -> bool {
        script_slot(&self.scripts, name).is_some_and(|slot| {
            slot.lock().ok().is_some_and(|slot| {
                slot.state() == script::RunState::Idle && slot.native_run().is_none()
            })
        })
    }

    /// Observe one slot's generation and native run under the same slot lock.
    /// Live pumps use this pair at the actual PLAYER_INFO callback; separate
    /// generation and run reads could straddle a runtime replacement.
    #[cfg(any(test, feature = "test-support"))]
    pub fn observe_native_runtime(
        &self,
        name: &str,
    ) -> Option<(u64, Option<api::selected::RunKey>)> {
        let slot = script_slot(&self.scripts, name)?;
        let slot = slot.lock().ok()?;
        Some((slot.runtime_generation(), slot.native_run()))
    }

    /// `name`'s latest recorded paint frame (the frame its status row
    /// publishes), for the live pump's File-card receipt watches.
    pub fn paint(&self, name: &str) -> Option<Arc<script::shim::ScriptPaint>> {
        script_runtime::script_paint_of(&self.scripts, name)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Play {
    /// Offline configuration fixtures use real generated facts without loading
    /// a client cache. Production only receives facts through ServerProfile.
    pub fn bind_script_test_data(&mut self, data: Arc<api::game_data::SelectedGameData>) {
        assert!(self.scripts.lock().unwrap().is_empty());
        self.game_data = Some(data);
    }
}

#[cfg(any(test, feature = "test-support"))]
impl ScriptNavPaint {
    /// Read the per-frame host policy only while a native guarded route is live.
    pub fn native_walk_cure_poison_for_test(
        &self,
        name: &str,
    ) -> Option<script::survival::CurePoison> {
        let navs = self.navs.lock().ok()?;
        let bot = navs.get(name)?;
        bot.native_walk.as_ref().filter(|owner| owner.live())?;
        bot.walk_guard.as_ref()?;
        bot.has_route().then_some(bot.cure_poison)
    }
}
impl Play {
    /// Install a read-only harness observer before spawning slots. It runs once
    /// after native execution and same-frame walk completion for each received
    /// PLAYER_INFO edge. Its storage and call are absent from product builds.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_post_native_player_info_observer(
        &mut self,
        observer: impl Fn(&client::client::Client, &str) + Send + Sync + 'static,
    ) {
        self.post_native_player_info = Some(Arc::new(observer));
    }

    /// Install a read-only harness observer before spawning slots. It runs each
    /// client frame after the real host policy copy and native observation.
    /// Its storage and call exist only with the test-support feature.
    #[cfg(feature = "test-support")]
    pub fn set_post_host_frame_observer(
        &mut self,
        observer: impl Fn(&client::client::Client, &str) + Send + Sync + 'static,
    ) {
        self.post_host_frame = Some(Arc::new(observer));
    }

    /// Overlay handle for catalog `walk` / `ctx.walk` Traveller (Play's
    /// per-uid NavBot). WalkTo's [`WalkArm`] map is a different latch.
    pub fn script_nav_paint(&self) -> ScriptNavPaint {
        ScriptNavPaint {
            navs: Arc::clone(&self.navs),
        }
    }
    /// Whether `name` is a slot this play controls (spawned or armed), so
    /// script control can never create an entry no thread drives.
    fn slot_active(&self, name: &str) -> bool {
        self.spawned.contains(name) || self.arms.contains_key(name)
    }

    #[cfg(test)]
    pub(crate) fn mark_script_running_for_walk(&self, name: &str) {
        struct Dummy;
        impl script::native::Script for Dummy {
            fn tick(
                &mut self,
                _ctx: &mut script::native::NativeTick<'_>,
            ) -> Result<script::native::ScriptFlow, script::native::ScriptFailure> {
                Ok(script::native::ScriptFlow::Continue)
            }
        }
        let slot = crate::script_runtime::script_slot_or_insert(&self.scripts, name);
        slot.lock()
            .unwrap()
            .start_test_script(Box::new(Dummy), None)
            .unwrap();
    }

    /// A stopped script run's published rows end with it: its paint and its
    /// script-walk destination. Lifecycle commands retire them directly
    /// because a disconnected or idle slot has no observe pass and no nav
    /// step, and a front-end refresh must not need another route step to
    /// learn that the walk ended. Call only after releasing the script-slot
    /// guard: script -> status is the sole permitted nesting order, and this
    /// path does not need to nest them.
    fn retire_script_run_status(&self, name: &str) {
        retire_script_run_status(&self.statuses, name);
    }

    /// Start the same off-pump compiled preparation transaction as live/scenario
    /// handles. Assignment owners commit only after `script_poll_start` is Ready.
    pub fn script_start(
        &self,
        name: &str,
        id: script::CompiledId,
        bag: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String> {
        self.script_start_typed(name, id, bag)
            .map_err(|error| error.to_string())
    }

    pub fn script_start_typed(
        &self,
        name: &str,
        id: script::CompiledId,
        bag: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), script::StartLoadError> {
        if !self.slot_active(name) {
            return Err(script::StartLoadError::Refused(format!("no slot: {name}")));
        }
        script_slot_or_insert(&self.scripts, name);
        self.script_start_handle().dispatch_start(
            name,
            || StartRequest::Compiled {
                id,
                bag,
                #[cfg(not(feature = "js"))]
                _lifetime: std::marker::PhantomData,
            },
            || Ok(()),
        )?;
        self.wake(name);
        Ok(())
    }

    /// Hold all new Starts for the lifetime of the returned token. Overlapping
    /// confirmations and accepted native copies share it, so replacing a
    /// prompt cannot bypass a copy whose durable write is still in flight.
    pub fn hold_script_starts(&self, reason: String) -> Arc<String> {
        let mut hold = self.script_start_hold.lock().unwrap();
        if let Some(token) = hold.upgrade() {
            return token;
        }
        let token = Arc::new(reason);
        *hold = Arc::downgrade(&token);
        token
    }

    /// Current copy/confirmation hold for control-thread witnesses. This is
    /// only an observation; actual Start admission still takes the hold lock.
    pub fn script_start_wait_reason(&self) -> Option<Arc<String>> {
        self.script_start_hold.lock().unwrap().upgrade()
    }

    /// A profile preparer has only shared pinned resources, no slot or game
    /// action handle. Frontend persistence follows successful typed preparation.
    /// The accepted job is charged to the one process WorkAdmission worker.
    pub fn script_prepare_config(
        &self,
        id: script::CompiledId,
        revision: u64,
        bag: Arc<script::native::SettingsBag>,
    ) -> Result<
        script::preparation::PreparedTask<Result<PreparedScriptConfig, script::native::StartError>>,
        script::preparation::PreparationError,
    > {
        let selected = self.game_data.clone().ok_or_else(|| {
            script::preparation::PreparationError::Unavailable(
                "selected game data unavailable".into(),
            )
        })?;
        let banks = Arc::clone(&self.named_banks);
        let quester = id == script::quester::card::CARD.id;
        let cold_gathering =
            id == script::CompiledId("Gatherer") && selected.try_gathering().is_none();
        let settings_input = script::preparation::PreparationInput::for_settings(revision, &bag)?;
        let mut folder_source = quester.then(script::quester::registry::source_for_preparation);
        let input = if let Some(source) = folder_source.as_ref() {
            script::preparation::PreparationInput::for_quester_folder(
                revision,
                settings_input,
                source,
            )?
        } else if cold_gathering {
            settings_input.with_gathering_family_decode()
        } else {
            settings_input
        };
        enum Stage {
            Registry(Arc<script::native::SettingsBag>),
            Decode(Arc<script::native::SettingsBag>),
            Configure(Arc<script::native::SettingsBag>),
            Done,
        }
        let mut stage = if quester {
            Stage::Registry(bag)
        } else if cold_gathering {
            Stage::Decode(bag)
        } else {
            Stage::Configure(bag)
        };
        let mut registry_preparation = None;
        let mut registry_quests = None;
        let mut prepared_registry = None;
        let mut registry_actions = 0_u32;
        script::preparation::submit_cooperative(
            script::preparation::PreparationKind::Profile,
            api::identity::Scope::Process,
            input,
            move |worker| match std::mem::replace(&mut stage, Stage::Done) {
                Stage::Registry(bag) => {
                    registry_actions = registry_actions
                        .checked_add(1)
                        .expect("bounded Quester registry action count");
                    let result = (|| -> Result<
                        Option<script::quester::registry::PreparedRegistry>,
                        script::native::StartError,
                    > {
                        if registry_preparation.is_none() {
                            let quests = Arc::new(
                                api::quest_facts::QuestCatalog::from_identity(
                                    selected.quest_identity(),
                                )
                                .map_err(script::native::StartError::Facts)?,
                            );
                            registry_preparation = Some(
                                script::quester::registry::RegistryPreparation::from_source(
                                    folder_source.take().expect("Quester folder source captured"),
                                    Arc::clone(&selected),
                                    Arc::clone(&quests),
                                )
                                .map_err(|error| {
                                    script::native::StartError::Unavailable(
                                        format!(
                                            "{}: {}",
                                            error.code,
                                            error.detail.as_deref().unwrap_or("Path reload failed")
                                        )
                                        .into(),
                                    )
                                })?,
                            );
                            registry_quests = Some(quests);
                        }
                        registry_preparation
                            .as_mut()
                            .expect("registry preparation initialized")
                            .step(worker)
                            .map_err(|error| {
                                script::native::StartError::Unavailable(
                                    format!(
                                        "{}: {}",
                                        error.code,
                                        error.detail.as_deref().unwrap_or("Path reload failed")
                                    )
                                    .into(),
                                )
                            })
                    })();
                    match result {
                        Ok(Some(candidate)) => {
                            registry_preparation = None;
                            prepared_registry = Some(candidate);
                            stage = Stage::Configure(bag);
                            script::preparation::CooperativePreparationStep::Continue {
                                completed: registry_actions,
                                total: Some(registry_actions),
                            }
                        }
                        Ok(None) => {
                            stage = Stage::Registry(bag);
                            script::preparation::CooperativePreparationStep::Continue {
                                completed: registry_actions,
                                total: None,
                            }
                        }
                        Err(error) => script::preparation::CooperativePreparationStep::Complete {
                            output: Err(error),
                            completed: registry_actions,
                            total: Some(registry_actions),
                        },
                    }
                }
                Stage::Decode(bag) => match selected.prepare_gathering(worker) {
                    Ok(_) => {
                        stage = Stage::Configure(bag);
                        script::preparation::CooperativePreparationStep::Continue {
                            completed: 1,
                            total: Some(2),
                        }
                    }
                    Err(error) => script::preparation::CooperativePreparationStep::Complete {
                        output: Err(script::native::StartError::Facts(error)),
                        completed: 1,
                        total: Some(2),
                    },
                },
                Stage::Configure(bag) => {
                    let ready = if quester {
                        Some(script::quester::card::QuesterRegistryReady::new(
                            prepared_registry
                                .as_ref()
                                .expect("Quester registry prepared")
                                .registry()
                                .clone(),
                            registry_quests
                                .take()
                                .expect("Quester quest facts prepared"),
                        ))
                    } else {
                        None
                    };
                    script::preparation::CooperativePreparationStep::Complete {
                        output: script::slot::prepare_config(
                            worker,
                            id,
                            revision,
                            bag,
                            Arc::clone(&selected),
                            Arc::clone(&banks),
                            ready,
                        )
                        .map(|config| PreparedScriptConfig {
                            config,
                            registry: prepared_registry.take(),
                        }),
                        completed: if quester {
                            registry_actions + 1
                        } else if cold_gathering {
                            2
                        } else {
                            1
                        },
                        total: Some(if quester {
                            registry_actions + 1
                        } else if cold_gathering {
                            2
                        } else {
                            1
                        }),
                    }
                }
                Stage::Done => unreachable!("completed preparation is not dispatched"),
            },
        )
    }

    pub fn script_configure_compiled(
        &self,
        name: &str,
        config: Arc<script::native::PreparedConfig>,
        target: api::selected::RunKey,
    ) -> script::CompiledDelivery {
        let Some(slot) = script_slot(&self.scripts, name) else {
            return script::CompiledDelivery::Stale;
        };
        let Ok(mut slot) = slot.lock() else {
            return script::CompiledDelivery::Stale;
        };
        slot.configure_compiled(config, target)
    }

    pub fn script_native_run(&self, name: &str) -> Option<api::selected::RunKey> {
        script_slot(&self.scripts, name)
            .and_then(|slot| slot.lock().ok().and_then(|slot| slot.native_run()))
    }

    pub fn script_native_status(&self, name: &str) -> Option<Arc<script::native::ScriptStatus>> {
        script_slot(&self.scripts, name)
            .and_then(|slot| slot.lock().ok().and_then(|slot| slot.native_status()))
    }

    /// Read-only per-slot seat counters for status and diagnostics consumers.
    /// Sampling does not enter the JS isolate or allocate a per-frame record.
    pub fn script_slot_metrics(&self, name: &str) -> Option<script::slot::SlotMetrics> {
        script_slot(&self.scripts, name)
            .and_then(|slot| slot.lock().ok().map(|slot| slot.slot_metrics()))
    }

    /// Current or last native/v2 walk assessment, retained after a refusal.
    /// rs2b0t-compatible scripts receive assessment text only in the host log.
    pub fn script_walk_risk(&self, name: &str) -> Option<Arc<str>> {
        let navs = self.navs.lock().ok()?;
        let bot = navs.get(name)?;
        if bot.compat_v1 {
            return None;
        }
        bot.assessment
            .as_ref()
            .or(bot.last_assessment.as_ref())
            .map(|assessment| Arc::clone(&assessment.reason))
    }

    pub fn script_native_settings_revision(&self, name: &str) -> Option<u64> {
        script_slot(&self.scripts, name).and_then(|slot| {
            slot.lock()
                .ok()
                .and_then(|slot| slot.native_settings_revision())
        })
    }

    pub fn script_paint(&self, name: &str) -> Option<Arc<script::shim::ScriptPaint>> {
        script_runtime::script_paint_of(&self.scripts, name)
    }

    pub fn script_control_key(&self, name: &str) -> Option<(u64, u64)> {
        script_slot(&self.scripts, name)
            .and_then(|slot| slot.lock().ok().map(|slot| slot.control_key()))
    }

    /// Native controls validate the worker/run/session under the mutation lock.
    pub fn script_native_pause(
        &self,
        name: &str,
        target: api::selected::RunKey,
        paused: bool,
    ) -> bool {
        let Some(slot) = script_slot(&self.scripts, name) else {
            return false;
        };
        let Ok(mut slot) = slot.lock() else {
            return false;
        };
        if slot.native_run() != Some(target) {
            return false;
        }
        if paused {
            pause_script(&mut slot, &self.navs, name);
        } else {
            slot.resume();
        }
        drop(slot);
        self.wake(name);
        true
    }

    pub fn script_native_stop(&self, name: &str, target: api::selected::RunKey) -> bool {
        let Some(slot) = script_slot(&self.scripts, name) else {
            return false;
        };
        let Ok(mut slot) = slot.lock() else {
            return false;
        };
        if slot.native_run() != Some(target) {
            return false;
        }
        slot.stop();
        reset_script_nav(&self.navs, name, None);
        drop(slot);
        self.retire_script_run_status(name);
        self.wake(name);
        true
    }

    pub fn script_native_read_journal(
        &self,
        name: &str,
        target: api::selected::RunKey,
    ) -> Result<(), String> {
        self.script_start_handle().read_journal(name, target)?;
        self.wake(name);
        Ok(())
    }

    #[cfg(feature = "test-support")]
    /// Skip the active row of `name`'s running compiled Quester queue.
    ///
    /// `target` binds the request to the run identity shown by native status;
    /// a missing or stale identity is refused before the queue is changed.
    /// This is a synchronous control transaction on the slot lock. An accepted
    /// request wakes the target slot so its next eligible tick can cancel and
    /// park the active quest.
    pub fn script_skip_quest(
        &self,
        name: &str,
        target: Option<api::selected::RunKey>,
    ) -> Result<script::native::SkippedQuest, script::native::ScriptFailure> {
        let failure = |code: &str, message: &str| script::native::ScriptFailure {
            code: code.into(),
            message: message.into(),
        };
        let Some(slot) = script_slot(&self.scripts, name) else {
            return Err(failure(
                "not-quester",
                "bot is not running the Quester queue",
            ));
        };
        let mut slot = slot
            .lock()
            .map_err(|_| failure("skip-quest-refused", "script slot retiring"))?;
        let skipped = slot.skip_quest_compiled(target)?;
        drop(slot);
        host_log!(
            Category::Lifecycle,
            Level::Info,
            slot = name,
            "command accepted: SkipQuest on {name}: {name}: quest_id={} step_id={}",
            skipped.quest_id,
            skipped.step_id
        );
        self.wake(name);
        Ok(skipped)
    }

    #[cfg(feature = "js")]
    /// Start a loaded JS bot on `name`'s slot: the isolate is spawned here,
    /// on Start (never at Load). Same slot gating as
    /// [`Play::script_start`].
    pub fn script_start_load(
        &self,
        name: &str,
        source: String,
        shape: script_js::LoadShape,
        settings_bag: Option<serde_json::Map<String, serde_json::Value>>,
        siblings: Vec<(String, String)>,
    ) -> Result<(), String> {
        self.script_start_load_typed(name, source, shape, settings_bag, siblings)
            .map_err(|e| e.to_string())
    }

    #[cfg(feature = "js")]
    /// Initial load result with operational refusals separate from loader errors.
    pub fn script_start_load_typed(
        &self,
        name: &str,
        source: String,
        shape: script_js::LoadShape,
        settings_bag: Option<serde_json::Map<String, serde_json::Value>>,
        siblings: Vec<(String, String)>,
    ) -> Result<(), script::StartLoadError> {
        if !self.slot_active(name) {
            return Err(script::StartLoadError::Refused(format!("no slot: {name}")));
        }
        self.script_start_handle().dispatch_start(
            name,
            || StartRequest::Load {
                source,
                shape,
                bag: settings_bag,
                siblings,
                loadouts: None,
            },
            || Ok(()),
        )?;
        self.wake(name);
        Ok(())
    }

    /// Cloneable isolate-start handle for slot-thread live pumps that
    /// cannot hold `&Play`.
    pub fn script_start_handle(&self) -> ScriptStartHandle {
        ScriptStartHandle {
            scripts: Arc::clone(&self.scripts),
            hold: Arc::clone(&self.script_start_hold),
            navs: Arc::clone(&self.navs),
            statuses: Arc::clone(&self.statuses),
            game_data: self.game_data.clone(),
            named_banks: Arc::clone(&self.named_banks),
            state: self.local_state.clone(),
        }
    }

    /// Pause `name`'s script (operator Pause; survives login until
    /// Resume re-arms it). No-op when the slot has no script.
    pub fn script_pause(&self, name: &str) {
        if let Some(slot) = script_slot(&self.scripts, name) {
            let Ok(mut slot) = slot.lock() else {
                return;
            };
            pause_script(&mut slot, &self.navs, name);
        }
        self.wake(name);
    }

    /// Resume `name`'s script; the next `on_is_up` re-gates it.
    pub fn script_resume(&self, name: &str) {
        if let Some(slot) = script_slot(&self.scripts, name) {
            let Ok(mut slot) = slot.lock() else {
                return;
            };
            slot.resume();
        }
        self.wake(name);
    }

    /// Stop `name`'s script: teardown hook, instance dropped, Idle. Retire its
    /// published paint and script-walk destination even while the client is
    /// offline.
    pub fn script_stop(&self, name: &str) {
        let slot = script_slot(&self.scripts, name);
        let mut guard = if let Some(slot) = slot.as_ref() {
            let Ok(guard) = slot.lock() else {
                return;
            };
            Some(guard)
        } else {
            None
        };
        if let Some(slot) = guard.as_mut() {
            slot.stop();
        }
        // A stopped script's route and reconnect carry belong to that run.
        reset_script_nav(&self.navs, name, None);
        drop(guard);
        self.retire_script_run_status(name);
        self.wake(name);
    }

    /// Stop only the exact active script lifetime inspected by a confirmation.
    /// Identity, generation, state, and Stop are one slot-lock transaction, so
    /// a completed run or a newer script can never be stopped by an old prompt.
    pub fn script_stop_if_identity_generation(
        &self,
        name: &str,
        identity: &str,
        generation: u64,
    ) -> bool {
        let Some(slot) = script_slot(&self.scripts, name) else {
            return false;
        };
        {
            let Ok(mut slot) = slot.lock() else {
                return false;
            };
            if slot.source_identity() != Some(identity)
                || slot.runtime_generation() != generation
                || !matches!(
                    slot.state(),
                    script::RunState::Starting
                        | script::RunState::Running
                        | script::RunState::Paused
                )
            {
                return false;
            }
            slot.stop();
            reset_script_nav(&self.navs, name, None);
        }
        self.retire_script_run_status(name);
        self.wake(name);
        true
    }

    pub fn script_attach_identity(&self, name: &str, identity: impl Into<String>) {
        if let Some(slot) = script_slot(&self.scripts, name) {
            if let Ok(mut slot) = slot.lock() {
                slot.attach_source_identity(identity);
            }
        }
    }

    pub fn script_runtime_generation(&self, name: &str) -> Option<u64> {
        script_slot(&self.scripts, name)
            .and_then(|slot| slot.lock().ok().map(|slot| slot.runtime_generation()))
    }

    /// `name`'s script run, read under one slot lock: the slot's lifetime
    /// incarnation, the run generation (it moves at every Start and Stop,
    /// never at Pause) and the lifecycle state. `None` without a slot or
    /// when a retiring worker poisoned the slot lock. A paint control is
    /// bound to the first two.
    pub fn script_run_key(&self, name: &str) -> Option<(u64, u64, script::RunState)> {
        script_slot(&self.scripts, name).and_then(|slot| {
            slot.lock().ok().map(|slot| {
                (
                    slot.control_key().0,
                    slot.runtime_generation(),
                    slot.state(),
                )
            })
        })
    }

    pub fn script_source_identity(&self, name: &str) -> Option<String> {
        script_slot(&self.scripts, name).and_then(|slot| {
            slot.lock()
                .ok()
                .and_then(|slot| slot.source_identity().map(str::to_string))
        })
    }

    pub fn script_post_settings_fenced(
        &self,
        name: &str,
        bag: &serde_json::Map<String, serde_json::Value>,
        identity: &str,
        generation: u64,
    ) -> bool {
        let Some(slot) = script_slot(&self.scripts, name) else {
            return false;
        };
        slot.lock()
            .ok()
            .is_some_and(|mut slot| slot.post_settings_bag_fenced(bag, identity, generation))
    }

    /// One-shot script-local paint button for `name`. No-op when there is
    /// no slot, the slot is not Running, `id` is empty, `generation` does
    /// not match the last forwarded frame, or that frame does not advertise
    /// `id`. Never walks, pauses, or stops.
    pub fn script_paint_click(&self, name: &str, id: &str, generation: u64) {
        if id.is_empty() {
            return;
        }
        let Some(slot) = script_slot(&self.scripts, name) else {
            return;
        };
        let Ok(slot) = slot.lock() else {
            return;
        };
        if slot.state() != script::RunState::Running {
            return;
        }
        let Some(paint) = slot.paint() else {
            return;
        };
        if paint.generation != generation {
            return;
        }
        if !paint.buttons.iter().any(|b| b.id == id) {
            return;
        }
        slot.paint_click(id);
    }

    /// Persistent strip/rail/tabs selection for `name`. No-op when there is
    /// no slot, the slot is not Running, `key` or `name` is empty,
    /// `generation` does not match the last forwarded frame, or that frame
    /// does not advertise `name` under `key`. Never walks, pauses, or stops.
    pub fn script_paint_select(&self, name: &str, key: &str, select_name: &str, generation: u64) {
        if key.is_empty() || select_name.is_empty() {
            return;
        }
        let Some(slot) = script_slot(&self.scripts, name) else {
            return;
        };
        let Ok(slot) = slot.lock() else {
            return;
        };
        if slot.state() != script::RunState::Running {
            return;
        }
        let Some(paint) = slot.paint() else {
            return;
        };
        if paint.generation != generation {
            return;
        }
        if !script_runtime::script_paint_select_advertised(&paint, key, select_name) {
            return;
        }
        slot.paint_select(key, select_name);
    }

    /// `name`'s script lifecycle state; `Idle` when the slot has none or its
    /// retiring worker poisoned the slot lock.
    pub fn script_state(&self, name: &str) -> script::RunState {
        script_slot(&self.scripts, name)
            .and_then(|slot| slot.lock().ok().map(|slot| slot.state()))
            .unwrap_or(script::RunState::Idle)
    }

    /// `name`'s armed run: runtime, time since gameplay last progressed and
    /// levels gained since Start. `None` when no run is armed (idle slot) or
    /// the slot lock is poisoned by a retiring worker. Reads only; the
    /// Fleet window calls it for its visible rows while it is open.
    pub fn script_progress(&self, name: &str) -> Option<script::ScriptProgress> {
        script_slot(&self.scripts, name).and_then(|slot| {
            slot.lock()
                .ok()
                .and_then(|slot| slot.progress(std::time::Instant::now()))
        })
    }

    /// Resolve `name`'s non-blocking isolate Start/Stop. The slot thread
    /// does this every observe; a slot that is offline or queued for login
    /// has no observe, so the UI pumps it (see [`Self::pump_script_lifecycles`]).
    pub fn pump_script_lifecycle(&self, name: &str) {
        if let Some(slot) = script_slot(&self.scripts, name) {
            if let Ok(mut slot) = slot.lock() {
                slot.observe_lifecycle();
            }
        }
    }

    /// Resolve every slot's pending Start/Stop once. Called per UI frame by
    /// the panel and the TUI. A slot its own thread holds is skipped: that
    /// thread is observing it right now.
    pub fn pump_script_lifecycles(&self) {
        let slots: Vec<_> = self.scripts.lock().unwrap().values().cloned().collect();
        for slot in slots {
            if let Ok(mut slot) = slot.try_lock() {
                slot.observe_lifecycle();
            }
        }
    }

    /// `name`'s latest operator Load Start: Start returns before V8 setup,
    /// so the assignment and the load diagnostic are committed from the
    /// settled outcome, not from Start's `Ok`. The lifecycle observe, the
    /// outcome take and the in-flight check happen under one slot lock, so
    /// the slot thread's own observe can never settle it unseen between
    /// them. A removed slot owes nothing.
    pub fn script_poll_start(&self, name: &str) -> script::StartPoll {
        poll_start(&self.scripts, name)
    }

    #[cfg(feature = "memory-profile")]
    pub fn memory_script_metrics(&self, name: &str) -> Option<serde_json::Value> {
        script_slot(&self.scripts, name)
            .and_then(|slot| slot.lock().ok().and_then(|slot| slot.memory_metrics()))
    }

    #[cfg(feature = "memory-profile")]
    pub fn memory_script_progress(&self, name: &str) -> serde_json::Value {
        script_slot(&self.scripts, name)
            .and_then(|slot| slot.lock().ok().map(|slot| slot.memory_progress()))
            .unwrap_or(serde_json::Value::Null)
    }

    /// `name`'s script `last_error`; `None` when the slot has none or is
    /// retiring after poisoning its lock.
    pub fn script_last_error(&self, name: &str) -> Option<String> {
        script_slot(&self.scripts, name).and_then(|slot| {
            slot.lock()
                .ok()
                .and_then(|slot| slot.last_error().map(str::to_string))
        })
    }

    /// Latest bounded ScriptRunner.stop receipt. This is non-consuming and
    /// independent of [`Self::script_take_pending_logs`].
    pub fn script_lifecycle_receipt(&self, name: &str) -> Option<script::ScriptLifecycleReceipt> {
        script_slot(&self.scripts, name)
            .and_then(|slot| slot.lock().ok().and_then(|slot| slot.lifecycle_receipt()))
    }

    /// Isolate log lines staged since the last take (panel log pane).
    pub fn script_take_pending_logs(&self, name: &str) -> Vec<String> {
        let Some(slot) = script_slot(&self.scripts, name) else {
            return Vec::new();
        };
        // A slot its own thread holds keeps its lines for the next frame.
        // A poisoned lock belongs to a retiring worker; the reaper owns
        // removal, while UI readers treat it as absent.
        let mut slot = match slot.try_lock() {
            Ok(slot) => slot,
            Err(std::sync::TryLockError::WouldBlock | std::sync::TryLockError::Poisoned(_)) => {
                return Vec::new();
            }
        };
        slot.take_pending_logs()
    }
}
