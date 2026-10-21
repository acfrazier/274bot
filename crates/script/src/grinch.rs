//! Anchored collection policy over the shared ground, bank and walk machines.

use crate::acquire::{
    self, AcquireOutcome, AcquireRequest, AcquireVerdict, BankChoice, BankStage, CloseBank,
    CoinPolicy, DepositSpec, OpenGate, VendorChoice, WorldFacts,
};
use crate::gatherer::WorkArea;
use crate::ground_take::{FailureLedger, FailureStatus};
use crate::native::{
    walk::Walk, ActionError, ActionHandle, CompiledCard, ConfigError, Interrupt, NativePhase,
    NativeTick, PrepareContext, PreparedConfig, RetainedMemory, Script, ScriptFailure, ScriptFlow,
    ScriptStatus, SettingsApply, SettingsBag, StartError, StatusField, StatusValue, StopReason,
    WalkOptions, WalkRequest,
};
use crate::quester::families::reach::{
    choose_ground, Avoid, GroundIds, GroundTake, ReachArgs, ReachKind,
};
use crate::{CompiledId, SettingDef};
use api::ground_take::{GroundKey, GroundRefusal, TakeOutcome};
use api::selected::RunKey;
use api::snapshot::SnapshotView;
use api::WorldTile;
use std::fmt::Write as _;
use std::sync::{Arc, LazyLock};
use std::task::Poll;

const ITEMS: &[(i32, &str)] = &[
    (1961, "Easter egg (1961)"),
    (981, "Disk of returning (981)"),
    (1989, "Half full wine jug (1989)"),
    (1959, "Pumpkin (1959)"),
    (1053, "Green Halloween mask (1053)"),
    (1055, "Blue Halloween mask (1055)"),
    (1057, "Red Halloween mask (1057)"),
    (1050, "Santa hat (1050)"),
    (962, "Christmas cracker (962)"),
    (1038, "Red partyhat (1038)"),
    (1040, "Yellow partyhat (1040)"),
    (1042, "Blue partyhat (1042)"),
    (1044, "Green partyhat (1044)"),
    (1046, "Purple partyhat (1046)"),
    (1048, "White partyhat (1048)"),
    (1419, "Scythe (1419)"),
    (1037, "Bunny ears (1037)"),
    (4079, "Yo-yo (4079)"),
];

pub const CARD: CompiledCard = CompiledCard {
    id: CompiledId("Grinch"),
    name: "Grinch",
    description:
        "Collect selected visible holiday items around the Start tile, bank them and return.",
    category: "Gathering",
    schema_version: 1,
    schema,
    per_account_settings: &[],
    prepare,
    create,
};

/// Immutable choices prepared once for one settings revision.
pub struct Prepared {
    pub ids: Arc<[i32]>,
    pub radius: u16,
    /// `None` selects the nearest routable bank, never an air-distance fallback.
    pub bank: Option<Arc<str>>,
    pub banks: Arc<api::named_banks::NamedBankFacts>,
    pub selected: Arc<api::game_data::SelectedGameData>,
}

fn schema() -> &'static [SettingDef] {
    SCHEMA.as_slice()
}

static SCHEMA: LazyLock<Vec<SettingDef>> = LazyLock::new(|| {
    let mut items = setting("items", "string[]", None);
    items.label = Some("Items".into());
    items.options = ITEMS.iter().map(|(id, _)| id.to_string()).collect();
    items.option_labels = ITEMS.iter().map(|(_, label)| (*label).into()).collect();
    items.default =
        Some(serde_json::to_string(&items.options).expect("decimal item IDs serialize"));
    let mut radius = setting("radius", "number", Some("12"));
    radius.min = Some("1".into());
    radius.max = Some("16".into());
    radius.step = Some("1".into());
    let mut bank = setting("bank", "string", Some("Nearest"));
    bank.label = Some("Bank: Nearest reachable, or the bank you pick".into());
    bank.options.push("Nearest".into());
    bank.option_labels.push("Nearest reachable".into());
    bank.options_from = Some("named-banks".into());
    vec![
        items,
        radius,
        bank,
        setting("allow_wilderness", "boolean", Some("false")),
        setting("allow_danger_zones", "boolean", Some("false")),
        setting("allow_teleports", "boolean", Some("false")),
    ]
});

fn setting(id: &str, ty: &str, default: Option<&str>) -> SettingDef {
    SettingDef {
        id: id.into(),
        ty: ty.into(),
        default: default.map(str::to_string),
        label: None,
        min: None,
        max: None,
        step: None,
        options: Vec::new(),
        option_labels: Vec::new(),
        group: None,
        show_if: None,
        options_from: None,
        csv_toggle: None,
        help: None,
        item_option_spec: None,
    }
}

fn config_error(field: &str, code: &str, message: &str) -> StartError {
    StartError::Config(ConfigError::new(field, code, message))
}

fn prepare(
    cx: &mut PrepareContext<'_>,
    revision: u64,
    bag: Arc<SettingsBag>,
) -> Result<Arc<PreparedConfig>, StartError> {
    if cx
        .selected
        .selected_pin()
        .map_err(StartError::Facts)?
        .as_ref()
        != cx.pin.as_ref()
    {
        return Err(StartError::Facts(api::selected::FactError::PinMismatch));
    }
    let mut ids = Vec::with_capacity(ITEMS.len());
    match bag.get("items") {
        None => ids.extend(ITEMS.iter().map(|(id, _)| *id)),
        Some(serde_json::Value::Array(values)) => {
            if values.len() > u32::BITS as usize {
                return Err(config_error(
                    "items",
                    "items-too-many",
                    "Select at most 32 item IDs",
                ));
            }
            for value in values {
                let id = value
                    .as_str()
                    .and_then(|value| value.parse::<i32>().ok())
                    .filter(|id| ITEMS.iter().any(|(known, _)| known == id))
                    .ok_or_else(|| {
                        config_error(
                            "items",
                            "unknown-item",
                            "choose base item IDs from the Grinch list",
                        )
                    })?;
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        Some(_) => {
            return Err(config_error(
                "items",
                "invalid-items",
                "items must be an array of decimal ID strings",
            ))
        }
    }
    if ids.is_empty() {
        return Err(config_error(
            "items",
            "empty-items",
            "select at least one item",
        ));
    }
    if ids.iter().any(|id| cx.selected.item_by_id(*id).is_none()) {
        return Err(config_error(
            "items",
            "unavailable-item",
            "a selected item is unavailable in this revision",
        ));
    }
    let radius = match bag.get("radius") {
        None => 12,
        Some(value) => value
            .as_i64()
            .filter(|value| (1..=16).contains(value))
            .ok_or_else(|| {
                config_error(
                    "radius",
                    "invalid-radius",
                    "radius must be an integer from 1 through 16",
                )
            })? as u16,
    };
    let bank = match bag.get("bank") {
        None => None,
        Some(value) if value.as_str() == Some("Nearest") => None,
        Some(value) => {
            let name = value.as_str().ok_or_else(|| {
                config_error("bank", "invalid-bank", "choose a named bank or Nearest")
            })?;
            let bank = cx
                .banks
                .banks()
                .iter()
                .find(|bank| bank.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| {
                    config_error("bank", "unknown-bank", "the named bank is unavailable")
                })?;
            Some(Arc::from(bank.name))
        }
    };
    for key in ["allow_wilderness", "allow_danger_zones", "allow_teleports"] {
        if bag.get(key).is_some_and(|value| !value.is_boolean()) {
            return Err(config_error(
                key,
                "invalid-permission",
                "walk permissions must be boolean",
            ));
        }
    }
    let prepared = Arc::new(Prepared {
        ids: ids.into(),
        radius,
        bank,
        banks: Arc::clone(&cx.banks),
        selected: Arc::clone(&cx.selected),
    });
    Ok(PreparedConfig::new(
        CARD.id,
        CARD.schema_version,
        revision,
        bag,
        prepared,
    ))
}

fn create(
    run: RunKey,
    config: Arc<PreparedConfig>,
    _retained: &mut RetainedMemory,
) -> Result<Box<dyn Script>, StartError> {
    let prepared = config
        .get::<Arc<Prepared>>()
        .cloned()
        .ok_or_else(|| config_error("", "config-identity", "not a prepared Grinch card"))?;
    Ok(Box::new(Grinch {
        run,
        config,
        prepared,
        anchor: None,
        phase: Phase::AwaitReady,
        failures: FailureLedger::default(),
        excluded: 0,
        picked: 0,
        banked: 0,
        bank_required: false,
        return_required: false,
        paused: false,
        held: false,
        death: Default::default(),
        status_phase: NativePhase::Preparing,
        status: Arc::from("waiting for ready scene"),
        target: Arc::from(""),
        exclusions: Arc::from(""),
        dirty: true,
        debug_foreground: None,
        failure: None,
    }))
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Watch {
    ground_collision: (u64, u64, u64),
    inventory: Option<u64>,
    free_slots: i32,
    here: WorldTile,
    map_base: (i32, i32),
    reach_ready: bool,
}

impl Watch {
    fn observe(snapshot: SnapshotView<'_>) -> Option<Self> {
        let player = snapshot.local_player()?.value;
        snapshot.inventory()?;
        snapshot.ground_items()?;
        let here = snapshot.here()?.value;
        let world = snapshot.world()?.value;
        Some(Self {
            ground_collision: snapshot.ground_collision_generations()?,
            inventory: snapshot.inventory_packet_generation(),
            free_slots: snapshot.stock().free_slots()?,
            here,
            map_base: (world.map_base_x, world.map_base_z),
            reach_ready: crate::reach_route::live(
                snapshot.reach().map(|view| view.value),
                Some(player.player.network),
            )
            .is_some(),
        })
    }
}

#[expect(
    clippy::large_enum_variant,
    reason = "keep GroundTake inline to avoid a heap allocation on every pickup; Grinch measures 896 B per running bot"
)]
enum Phase {
    AwaitReady,
    Scan,
    Take {
        key: GroundKey,
        handle: ActionHandle<GroundTake>,
    },
    // Bank work is allocated only for the trip, not reserved in every empty wait.
    Bank(Box<acquire::run::AcquireRun>),
    Return(Option<ActionHandle<Walk>>),
    Idle(Watch, Option<u64>),
    Stopped,
}

struct Grinch {
    run: RunKey,
    config: Arc<PreparedConfig>,
    prepared: Arc<Prepared>,
    anchor: Option<WorldTile>,
    phase: Phase,
    failures: FailureLedger,
    excluded: u32,
    picked: u64,
    banked: u64,
    bank_required: bool,
    return_required: bool,
    paused: bool,
    held: bool,
    death: crate::native::death::DeathLatch,
    status_phase: NativePhase,
    status: Arc<str>,
    target: Arc<str>,
    exclusions: Arc<str>,
    dirty: bool,
    debug_foreground: Option<bool>,
    failure: Option<ScriptFailure>,
}

impl Grinch {
    fn set_status(&mut self, phase: NativePhase, status: &str) {
        if self.status_phase != phase || self.status.as_ref() != status {
            self.status_phase = phase;
            self.status = Arc::from(status);
            self.dirty = true;
        }
    }

    fn clear_target(&mut self) {
        if !self.target.is_empty() {
            self.target = Arc::from("");
            self.dirty = true;
        }
    }

    fn invalidate(&mut self) {
        // Dropping native handles revokes their queued requests before reuse.
        self.phase = Phase::AwaitReady;
        self.clear_target();
        self.set_status(NativePhase::Preparing, "waiting for ready scene");
    }

    fn publish(&mut self, tick: &mut NativeTick<'_>) {
        if api::hostlog::debug_enabled() {
            let foreground = tick
                .cx
                .ledger
                .as_ref()
                .and_then(|ledger| ledger.owner.as_ref())
                .is_some_and(|owner| owner.live());
            if self.dirty || self.debug_foreground != Some(foreground) {
                self.trace_state(tick, foreground);
                self.debug_foreground = Some(foreground);
            }
        }
        if !self.dirty {
            return;
        }
        tick.output.status(ScriptStatus {
            run: self.run,
            card: CARD.id,
            phase: self.status_phase,
            active_settings: self.config.revision(),
            pending_settings: None,
            fields: Arc::from([
                StatusField {
                    key: "status",
                    label: "Status",
                    value: StatusValue::Text(Arc::clone(&self.status)),
                },
                StatusField {
                    key: "picked",
                    label: "Picked up",
                    value: StatusValue::Integer(i64::try_from(self.picked).unwrap_or(i64::MAX)),
                },
                StatusField {
                    key: "banked",
                    label: "Banked",
                    value: StatusValue::Integer(i64::try_from(self.banked).unwrap_or(i64::MAX)),
                },
                StatusField {
                    key: "target",
                    label: "Target",
                    value: StatusValue::Text(Arc::clone(&self.target)),
                },
                StatusField {
                    key: "excluded",
                    label: "Run exclusions",
                    value: StatusValue::Text(Arc::clone(&self.exclusions)),
                },
            ]),
            failure: self.failure.clone(),
        });
        self.dirty = false;
    }

    fn trace_state(&self, tick: &mut NativeTick<'_>, foreground: bool) {
        let (phase, blocked) = if self
            .failure
            .as_ref()
            .is_some_and(|failure| failure.code.as_ref() == "inventory-blocked")
        {
            (
                "inventory_blocked",
                if self.bank_required {
                    "\"deposit_did_not_free_space\""
                } else {
                    "\"no_selected_item_to_deposit\""
                },
            )
        } else {
            (
                match self.phase {
                    Phase::AwaitReady => "preparing",
                    Phase::Scan => "scanning",
                    Phase::Take { .. } => "taking",
                    Phase::Bank(_) => "banking",
                    Phase::Return(_) => "returning",
                    Phase::Idle(..) => "waiting",
                    Phase::Stopped => "stopped",
                },
                "null",
            )
        };
        let mut message = String::with_capacity(512 + self.failures.records().len() * 160);
        write!(
            message,
            "GRINCH_STATE {{\"run\":{{\"slot\":{},\"run\":{},\"session\":{}}},\"tick\":{},\"anchor\":",
            self.run.slot,
            self.run.run,
            self.run.session,
            tick.cx.evidence().tick,
        )
        .expect("writing into a String");
        if let Some(anchor) = self.anchor {
            write!(
                message,
                "{{\"x\":{},\"z\":{},\"level\":{}}}",
                anchor.x, anchor.z, anchor.level
            )
            .expect("writing into a String");
        } else {
            message.push_str("null");
        }
        write!(
            message,
            ",\"phase\":\"{phase}\",\"blocked_reason\":{blocked},\"picked\":{},\"banked\":{},\"foreground_children\":{},\"id_exclusions\":[",
            self.picked,
            self.banked,
            u8::from(foreground),
        )
        .expect("writing into a String");
        let mut separator = "";
        for (index, id) in self.prepared.ids.iter().enumerate() {
            if self.excluded & (1_u32 << index) != 0 {
                write!(message, "{separator}{id}").expect("writing into a String");
                separator = ",";
            }
        }
        message.push_str("],\"failed_keys\":[");
        for (index, record) in self.failures.records().iter().enumerate() {
            write!(
                message,
                "{}{{\"item_id\":{},\"tile\":{{\"x\":{},\"z\":{},\"level\":{}}},\"failures\":{},\"last_failure_tick\":{},\"excluded\":{}}}",
                if index == 0 { "" } else { "," },
                record.key.id,
                record.key.tile.x,
                record.key.tile.z,
                record.key.tile.level,
                record.failures,
                record.last_failure_tick,
                record.excluded,
            )
            .expect("writing into a String");
        }
        message.push_str("]}");
        tick.output.log(api::hostlog::Level::Debug, &message);
    }

    fn trace_take(
        &self,
        tick: &mut NativeTick<'_>,
        key: GroundKey,
        result: &Result<TakeOutcome, ActionError>,
    ) {
        if !api::hostlog::debug_enabled() {
            return;
        }
        let (outcome, units) = match result {
            Ok(TakeOutcome::Picked { units }) => ("picked", *units),
            Ok(TakeOutcome::LostRace) => ("lost_race", 0),
            Ok(TakeOutcome::FailedKey) => ("failed_key", 0),
            Ok(TakeOutcome::Cancelled) | Err(ActionError::Cancelled) => ("cancelled", 0),
            Err(ActionError::GroundTake(GroundRefusal::Capacity)) => ("capacity", 0),
            Err(ActionError::GroundTake(GroundRefusal::AlreadyOwned { .. })) => {
                ("already_owned", 0)
            }
            Err(ActionError::Stale) => ("stale", 0),
            Err(ActionError::Held) => ("held", 0),
            Err(_) => ("action_error", 0),
        };
        tick.output.log(
            api::hostlog::Level::Debug,
            &format!(
                "GRINCH_TAKE {{\"run\":{{\"slot\":{},\"run\":{},\"session\":{}}},\"tick\":{},\"item_id\":{},\"tile\":{{\"x\":{},\"z\":{},\"level\":{}}},\"outcome\":\"{outcome}\",\"units\":{units}}}",
                self.run.slot,
                self.run.run,
                self.run.session,
                tick.cx.evidence().tick,
                key.id,
                key.tile.x,
                key.tile.z,
                key.tile.level,
            ),
        );
    }

    fn fail(&mut self, code: &str, message: impl Into<Arc<str>>) -> ScriptFailure {
        let failure = ScriptFailure {
            code: Arc::from(code),
            message: message.into(),
        };
        self.phase = Phase::Stopped;
        self.clear_target();
        self.set_status(NativePhase::Blocked, &failure.message);
        self.failure = Some(failure.clone());
        self.dirty = true;
        failure
    }

    fn action_failure(&mut self, error: ActionError) -> ScriptFailure {
        self.fail("action-failed", format!("native action failed: {error:?}"))
    }

    fn selected_held(&self, snapshot: SnapshotView<'_>) -> i64 {
        let stock = snapshot.stock();
        self.prepared
            .ids
            .iter()
            .map(|id| i64::from(stock.held(*id).unwrap_or(0).max(0)))
            .sum()
    }

    fn start_bank(&mut self, tick: &mut NativeTick<'_>) -> Result<(), ScriptFailure> {
        if !self.bank_required && self.selected_held(tick.cx.snapshot()) == 0 {
            return Err(self.fail(
                "inventory-blocked",
                "inventory is full and contains no selected item to deposit",
            ));
        }
        self.bank_required = true;
        self.return_required = true;
        self.clear_target();
        let request = AcquireRequest {
            needs: Arc::from([]),
            deposit: Some(DepositSpec::Ids {
                products: Arc::clone(&self.prepared.ids),
                keep: Arc::from([]),
            }),
            bank: BankChoice {
                bank: None,
                explicit: self.prepared.bank.clone(),
                required: self.prepared.bank.is_some(),
                preferences: Default::default(),
            },
            stage: BankStage::Planned,
            partial_ok: false,
            close: CloseBank::Close,
            vendor: VendorChoice::None,
            coins: CoinPolicy::None,
            walk: WalkOptions::default(),
            required_after: tick.cx.evidence(),
        };
        let world = WorldFacts {
            data: Some(&self.prepared.selected),
            members: tick
                .cx
                .snapshot()
                .world()
                .is_some_and(|world| world.value.members),
        };
        let plan = acquire::plan::plan(&request, &tick.cx.snapshot().stock(), world)
            .map_err(|error| self.action_failure(error))?;
        let AcquireVerdict::Plan(plan) = plan else {
            return Err(self.fail(
                "bank-incomplete",
                "selected-only deposit did not produce a bank plan",
            ));
        };
        self.phase = Phase::Bank(Box::new(acquire::run::AcquireRun::new(
            request,
            plan,
            &self.prepared.banks,
            &tick.cx,
        )));
        self.set_status(NativePhase::Working, "banking selected items");
        Ok(())
    }

    fn exclude_id(&mut self, id: i32, tick: &mut NativeTick<'_>) {
        let Some(index) = self
            .prepared
            .ids
            .iter()
            .position(|selected| *selected == id)
        else {
            return;
        };
        let bit = 1_u32 << index;
        if self.excluded & bit != 0 {
            return;
        }
        self.excluded |= bit;
        let text = format!(
            "excluded {}: already owned, until next Start",
            item_label(id)
        );
        tick.output.log(api::hostlog::Level::Info, &text);
        self.append_exclusion(&text);
    }

    fn append_exclusion(&mut self, text: &str) {
        self.exclusions = if self.exclusions.is_empty() {
            Arc::from(text)
        } else {
            Arc::from(format!("{}; {text}", self.exclusions))
        };
        self.dirty = true;
    }

    fn scan(&mut self, tick: &mut NativeTick<'_>, watch: Watch) -> Result<(), ScriptFailure> {
        if self.bank_required || watch.free_slots == 0 {
            return self.start_bank(tick);
        }
        if self.return_required {
            self.phase = Phase::Return(None);
            self.set_status(NativePhase::Working, "returning to start");
            return Ok(());
        }
        let anchor = self.anchor.expect("scan follows ready anchor capture");
        let selected = choose_ground(
            &tick.cx,
            GroundIds::Selected {
                ids: &self.prepared.ids,
                excluded: self.excluded,
                failures: Some(&self.failures),
            },
            None,
            WorkArea::at_start(anchor, self.prepared.radius),
            &Avoid::default(),
        )
        .reachable()
        .map(|choice| {
            (
                GroundKey {
                    id: choice.def.id,
                    tile: choice.tile,
                },
                Arc::<str>::from(
                    choice
                        .def
                        .name
                        .as_deref()
                        .unwrap_or(item_label(choice.def.id)),
                ),
            )
        });
        if let Some((key, name)) = selected {
            if !tick.cx.may_continue_this_tick() {
                return Ok(());
            }
            let args = ReachArgs {
                kind: ReachKind::Ground {
                    id: key.id,
                    obj: name,
                },
                op: Arc::from("Take"),
                anchor: Some(anchor),
                radius: i32::from(self.prepared.radius),
                wait_if_missing: false,
                target_tile: Some(key.tile),
                reachable_only: true,
            };
            let handle = match tick.actions.begin::<GroundTake>(args, &mut tick.cx) {
                Ok(handle) => handle,
                Err(ActionError::BudgetExhausted) => return Ok(()),
                Err(error) => return Err(self.action_failure(error)),
            };
            self.target = Arc::from(item_label(key.id));
            self.dirty = true;
            self.phase = Phase::Take { key, handle };
            self.set_status(
                NativePhase::Working,
                &format!("collecting {}", item_label(key.id)),
            );
        } else if watch.here != anchor {
            self.return_required = true;
            self.phase = Phase::Return(None);
            self.set_status(NativePhase::Working, "returning to start");
        } else {
            self.clear_target();
            self.phase = Phase::Idle(watch, self.failures.next_expiry(tick.cx.evidence().tick));
            self.set_status(
                NativePhase::Waiting,
                &format!("waiting for items, r={}", self.prepared.radius),
            );
        }
        Ok(())
    }

    fn advance(&mut self, tick: &mut NativeTick<'_>, watch: Watch) -> Result<(), ScriptFailure> {
        match std::mem::replace(&mut self.phase, Phase::Scan) {
            Phase::AwaitReady | Phase::Scan => self.scan(tick, watch),
            Phase::Idle(previous, expiry) => {
                if previous == watch && expiry.is_none_or(|at| at > tick.cx.evidence().tick) {
                    self.phase = Phase::Idle(previous, expiry);
                    return Ok(());
                }
                self.set_status(
                    NativePhase::Working,
                    &format!("checking items, r={}", self.prepared.radius),
                );
                self.scan(tick, watch)
            }
            Phase::Take { key, handle } => {
                let result = tick.actions.poll(&handle, &mut tick.cx);
                if let Poll::Ready(result) = &result {
                    self.trace_take(tick, key, result);
                }
                match result {
                    Poll::Pending => self.phase = Phase::Take { key, handle },
                    Poll::Ready(Ok(TakeOutcome::Picked { units })) => {
                        self.picked = self
                            .picked
                            .saturating_add(u64::try_from(units).unwrap_or(0));
                        self.failures.record_success(key);
                        self.dirty = true;
                        tick.note_progress();
                    }
                    Poll::Ready(Ok(TakeOutcome::FailedKey)) => {
                        if matches!(
                            self.failures.record_failure(key, tick.cx.evidence().tick),
                            FailureStatus::Excluded { .. }
                        ) {
                            self.append_exclusion(&format!(
                                "excluded {} at {},{},{}: three failed Takes, until next Start",
                                item_label(key.id),
                                key.tile.x,
                                key.tile.z,
                                key.tile.level
                            ));
                        }
                    }
                    Poll::Ready(Ok(TakeOutcome::LostRace | TakeOutcome::Cancelled)) => {}
                    Poll::Ready(Err(ActionError::GroundTake(GroundRefusal::Capacity))) => {
                        self.start_bank(tick)?;
                    }
                    Poll::Ready(Err(ActionError::GroundTake(GroundRefusal::AlreadyOwned {
                        id,
                    }))) => {
                        self.exclude_id(id, tick);
                    }
                    Poll::Ready(Err(
                        ActionError::Stale | ActionError::Cancelled | ActionError::Held,
                    )) => {
                        self.invalidate();
                    }
                    Poll::Ready(Err(error)) => return Err(self.action_failure(error)),
                }
                if matches!(self.phase, Phase::Scan) {
                    self.clear_target();
                    self.set_status(
                        NativePhase::Working,
                        &format!("checking items, r={}", self.prepared.radius),
                    );
                }
                Ok(())
            }
            Phase::Bank(mut bank) => {
                match bank.poll(tick, OpenGate::Open) {
                    Poll::Pending => self.phase = Phase::Bank(bank),
                    Poll::Ready(Ok(AcquireOutcome::Done(receipt))) => {
                        if !receipt.complete
                            || receipt.bank_open
                            || receipt.withdrew_any
                            || receipt.bought != 0
                            || receipt.kept_unbankable
                            || self.selected_held(tick.cx.snapshot()) != 0
                        {
                            return Err(self.fail(
                                "bank-incomplete",
                                "selected-only bank transfer or closure was incomplete",
                            ));
                        }
                        if watch.free_slots <= 0 {
                            return Err(self.fail(
                                "inventory-blocked",
                                "bank trip did not free an inventory slot",
                            ));
                        }
                        self.banked = self.banked.saturating_add(u64::from(receipt.deposited));
                        self.dirty = true;
                        self.bank_required = false;
                        self.phase = Phase::Return(None);
                        self.set_status(NativePhase::Working, "returning to start");
                    }
                    Poll::Ready(Ok(AcquireOutcome::ChildDue { .. })) => {
                        return Err(self.fail(
                            "bank-incomplete",
                            "deposit-only bank trip requested acquisition work",
                        ))
                    }
                    Poll::Ready(Err(
                        ActionError::Stale | ActionError::Cancelled | ActionError::Held,
                    )) => self.invalidate(),
                    Poll::Ready(Err(error)) => return Err(self.action_failure(error)),
                }
                Ok(())
            }
            Phase::Return(handle) => {
                let anchor = self.anchor.expect("return follows anchor capture");
                if let Some(handle) = handle {
                    match tick.actions.poll(&handle, &mut tick.cx) {
                        Poll::Pending => self.phase = Phase::Return(Some(handle)),
                        Poll::Ready(Ok(receipt)) => {
                            receipt
                                .into_arrival()
                                .map_err(|error| self.action_failure(error))?;
                            if watch.here != anchor {
                                return Err(self.fail(
                                    "return-incomplete",
                                    "arrival receipt did not place the player at the Start tile",
                                ));
                            }
                            self.return_required = false;
                        }
                        Poll::Ready(Err(
                            ActionError::Stale | ActionError::Cancelled | ActionError::Held,
                        )) => self.invalidate(),
                        Poll::Ready(Err(error)) => return Err(self.action_failure(error)),
                    }
                } else if tick.cx.may_continue_this_tick() {
                    let request = WalkRequest {
                        target: anchor,
                        loc_id: None,
                        radius: 0,
                        arrival: nav::contract::ArrivalKind::Reach,
                        options: WalkOptions::default(),
                        required_after: tick.cx.evidence(),
                        evidence: None,
                        cross: Box::default(),
                        protect: false,
                        food_guard: false,
                        allow: Default::default(),
                    };
                    match tick.actions.begin::<Walk>(request, &mut tick.cx) {
                        Ok(handle) => self.phase = Phase::Return(Some(handle)),
                        Err(ActionError::BudgetExhausted) => self.phase = Phase::Return(None),
                        Err(error) => return Err(self.action_failure(error)),
                    }
                } else {
                    self.phase = Phase::Return(None);
                }
                Ok(())
            }
            Phase::Stopped => {
                self.phase = Phase::Stopped;
                Ok(())
            }
        }
    }
}

impl Script for Grinch {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        if matches!(self.phase, Phase::Stopped) {
            return self.failure.clone().map_or(Ok(ScriptFlow::Complete), Err);
        }
        let current = tick.cx.run();
        if current.slot != self.run.slot || current.run != self.run.run {
            let failure = self.fail("stale-run", "the Grinch instance belongs to a retired run");
            self.publish(tick);
            return Err(failure);
        }
        if current.session != self.run.session {
            self.run = current;
            self.invalidate();
            self.dirty = true;
        }
        if self.paused || self.held || !tick.cx.eligible {
            self.invalidate();
            self.publish(tick);
            return Ok(ScriptFlow::Continue);
        }
        if self.death.observe(tick.cx.snapshot()) {
            let failure = self.fail(
                "death",
                "Grinch stops on death; Start again to capture a new anchor",
            );
            self.publish(tick);
            return Err(failure);
        }
        let Some(watch) = Watch::observe(tick.cx.snapshot()) else {
            self.invalidate();
            self.publish(tick);
            return Ok(ScriptFlow::Continue);
        };
        if self.anchor.is_none() {
            self.anchor = Some(watch.here);
        }
        let modal = tick
            .cx
            .snapshot()
            .main_modal()
            .is_some_and(|modal| modal.value.root >= 0)
            || tick
                .cx
                .snapshot()
                .chat_modal()
                .is_some_and(|modal| modal.value.root >= 0)
            || tick
                .cx
                .snapshot()
                .count_dialog_open()
                .is_some_and(|open| open.value)
            || tick
                .cx
                .snapshot()
                .bank_session()
                .is_some_and(|bank| bank.value.open);
        if modal
            && matches!(
                self.phase,
                Phase::AwaitReady | Phase::Scan | Phase::Idle(..)
            )
            && !self.bank_required
        {
            self.invalidate();
            self.publish(tick);
            return Ok(ScriptFlow::Continue);
        }
        let result: Result<(), ScriptFailure> = (|| {
            loop {
                if !tick.cx.charge_transition() {
                    break;
                }
                let previous = std::mem::discriminant(&self.phase);
                let starting_return = matches!(self.phase, Phase::Return(None));
                self.advance(tick, watch)?;
                if !tick.cx.may_continue_this_tick() {
                    break;
                }
                let started_child = previous != std::mem::discriminant(&self.phase)
                    && matches!(self.phase, Phase::Take { .. } | Phase::Bank(_))
                    || starting_return && matches!(self.phase, Phase::Return(Some(_)));
                if !started_child && !matches!(self.phase, Phase::Scan | Phase::Return(None)) {
                    break;
                }
            }
            Ok(())
        })();
        self.publish(tick);
        result.map(|()| ScriptFlow::Continue)
    }

    fn configure(&mut self, next: Arc<PreparedConfig>) -> Result<SettingsApply, ConfigError> {
        if next.card() != CARD.id || next.get::<Arc<Prepared>>().is_none() {
            return Err(ConfigError::new(
                "",
                "config-identity",
                "not a prepared Grinch card",
            ));
        }
        Ok(SettingsApply::RestartRequired)
    }

    fn interrupt(&mut self, event: Interrupt) {
        match event {
            Interrupt::Pause => self.paused = true,
            Interrupt::Resume => self.paused = false,
            Interrupt::Hold(held) => self.held = held,
            Interrupt::SessionEnded | Interrupt::SessionReady => {}
        }
        if !matches!(self.phase, Phase::Stopped) {
            self.invalidate();
        }
    }

    fn on_stop(&mut self, _reason: StopReason) {
        self.phase = Phase::Stopped;
        self.clear_target();
    }

    fn waits_for_evidence(&self) -> bool {
        matches!(self.phase, Phase::Idle(..))
    }

    fn recovery_anchor(&self) -> Option<WorldTile> {
        (!matches!(
            self.phase,
            Phase::Take { .. } | Phase::Bank(_) | Phase::Return(Some(_)) | Phase::Stopped
        ))
        .then_some(self.anchor)
        .flatten()
    }
}

fn item_label(id: i32) -> &'static str {
    ITEMS
        .iter()
        .find(|(known, _)| *known == id)
        .map_or("Selected item", |(_, label)| *label)
}

#[cfg(test)]
#[path = "grinch_tests.rs"]
mod tests;
