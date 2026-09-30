//! Real-Play qualification cells for the stage-1 Gatherer card.
//!
//! The cells use the operator Start path: a loopback profile is bound, a
//! slot is spawned, `Play::script_start` prepares and commits the compiled
//! card, and the worker observes a real Client. Selected logs/ores and dispatch
//! receipts are never injected. Fixture placement is supplied by the engine via
//! `GATHERER_WC_TILE` and `GATHERER_MINE_TILE` (`x,z,level`); the selected
//! catalogue and nav pack are supplied by `GATHERER_CATALOG_ROOT` and
//! `GATHERER_NAV_PACK`.
//!
//! Set `LIVE=1` and run an ignored test with the local 289 engine.  Debug
//! host logging is enabled so the production Driver's `native-packet` lines
//! (including a `CloseModal` + four-drop batch when the fixture presents a
//! modal) are part of the live witness.  Power cells check observed empty
//! slots, not merely dispatch receipts.
//! The mining power fixture optionally documents the incidental-gem keep policy
//! with `GATHERER_MINE_KEEP_GEM=1`: it seeds one uncut sapphire, treats that
//! occupied slot as unavailable to ore, and proves the gem remains present
//! through both full-pack ore-disposal cycles and subsequent gathers.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact::{self, Interactions, SendResult};
use api::snapshot::{GameSnapshot, WorldTile};
use host::Pump;
use host_play::{ProfileOptions, ScriptStartHandle, SharedClientTemplate};
use serde_json::{json, Map, Value};
use vault::{Profile, ProfileSettings};

const PREP_DEADLINE: Duration = Duration::from_secs(180);
const POWER_DEADLINE: Duration = Duration::from_secs(1800);
const WEDGE_DEADLINE: Duration = Duration::from_secs(900);
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const REQUIRED_CYCLES: u32 = 2;
const REQUIRED_POST_DROP_GATHERS: u32 = 2;
const LOG_ID: i32 = 1511;
const COPPER_ID: i32 = 436;
const TIN_ID: i32 = 438;
const IRON_ID: i32 = 440;
const COAL_ID: i32 = 453;
const UNCUT_SAPPHIRE_ID: i32 = 1623;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cell {
    Woodcutting,
    Mining,
}

impl Cell {
    const fn name(self) -> &'static str {
        match self {
            Self::Woodcutting => "gatherer_wc_power",
            Self::Mining => "gatherer_mine_tier_power",
        }
    }

    const fn tile_env(self) -> &'static str {
        match self {
            Self::Woodcutting => "GATHERER_WC_TILE",
            Self::Mining => "GATHERER_MINE_TILE",
        }
    }

    const fn skill_name(self) -> &'static str {
        match self {
            Self::Woodcutting => "woodcutting",
            Self::Mining => "mining",
        }
    }

    const fn setting_skill(self) -> &'static str {
        match self {
            Self::Woodcutting => "Woodcutting",
            Self::Mining => "Mining",
        }
    }

    const fn default_tool_alias(self) -> &'static str {
        match self {
            Self::Woodcutting => "bronze_axe",
            Self::Mining => "steel_pickaxe",
        }
    }

    const fn default_tool_id(self) -> i32 {
        match self {
            Self::Woodcutting => 1351,
            Self::Mining => 1269,
        }
    }

    const fn default_level(self) -> i32 {
        match self {
            Self::Woodcutting => 1,
            Self::Mining => 30,
        }
    }

    const fn products(self) -> &'static [i32] {
        match self {
            Self::Woodcutting => &[LOG_ID],
            Self::Mining => &[COPPER_ID, TIN_ID, IRON_ID, COAL_ID],
        }
    }

    const fn target_preference(self) -> &'static str {
        match self {
            Self::Woodcutting => "Nearest",
            Self::Mining => "Best tier",
        }
    }

    fn resources(self) -> Vec<String> {
        match self {
            Self::Woodcutting => vec!["normal".into()],
            Self::Mining => std::env::var("GATHERER_MINE_RESOURCES")
                .unwrap_or_else(|_| "iron,coal".into())
                .split(',')
                .map(str::trim)
                .filter(|resource| !resource.is_empty())
                .map(ToOwned::to_owned)
                .collect(),
        }
    }

    fn level(self) -> i32 {
        std::env::var(match self {
            Self::Woodcutting => "GATHERER_WC_LEVEL",
            Self::Mining => "GATHERER_MINE_LEVEL",
        })
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| self.default_level())
    }

    fn tool_alias(self) -> String {
        std::env::var(match self {
            Self::Woodcutting => "GATHERER_WC_TOOL",
            Self::Mining => "GATHERER_MINE_TOOL",
        })
        .unwrap_or_else(|_| self.default_tool_alias().into())
    }

    fn tool_id(self) -> i32 {
        std::env::var(match self {
            Self::Woodcutting => "GATHERER_WC_TOOL_ID",
            Self::Mining => "GATHERER_MINE_TOOL_ID",
        })
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| self.default_tool_id())
    }

    fn settings(self) -> Map<String, Value> {
        let mut bag = Map::new();
        bag.insert("skill".into(), json!(self.setting_skill()));
        bag.insert(
            "woodcuttingResources".into(),
            json!(if self == Self::Woodcutting {
                vec!["normal"]
            } else {
                Vec::<&str>::new()
            }),
        );
        bag.insert(
            "miningResources".into(),
            json!(if self == Self::Mining {
                self.resources()
            } else {
                Vec::<String>::new()
            }),
        );
        bag.insert("targetPreference".into(), json!(self.target_preference()));
        bag.insert("location".into(), json!("Start"));
        bag.insert("radius".into(), json!(12));
        bag.insert("disposition".into(), json!("Power"));
        bag.insert("allowTeleports".into(), json!(false));
        bag.insert("allowWilderness".into(), json!(false));
        bag.insert("deathPolicy".into(), json!("Stop"));
        bag.insert("maxDeaths".into(), json!(2));
        bag
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveCase {
    Power,
    CancelBeforeDrain,
    DeathDuringDrop,
    RunKeyChangeDuringDrop,
}

impl LiveCase {
    const fn name(self) -> &'static str {
        match self {
            Self::Power => "power",
            Self::CancelBeforeDrain => "cancel-before-drain",
            Self::DeathDuringDrop => "death-during-drop",
            Self::RunKeyChangeDuringDrop => "run-key-change-during-drop",
        }
    }

    const fn is_power(self) -> bool {
        matches!(self, Self::Power)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prep {
    WaitIngame,
    TutSkip,
    WaitTutorial,
    Relog,
    WaitRelog,
    Seed,
    WaitSeed,
    Equip,
    WaitEquip,
    OpenBank,
    WaitBank,
    WaitBankClose,
    ReturnFromBank,
    Ready,
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InvRow {
    id: i32,
    count: i32,
}

#[derive(Debug, Clone)]
struct Observation {
    inventory: BTreeMap<i32, InvRow>,
    product_count: i32,
    xp: i32,
}

#[derive(Debug, Default, Clone)]
struct Witness {
    cycles: u32,
    post_drop_gathers: u32,
    confirmed_drops: u32,
    full_pack_seen: u32,
    max_product_count: i32,
    last_xp: i32,
    last_status_yielded: i64,
    last_status_dropped: i64,
    awaiting_drop: bool,
    awaiting_post_drop_gather: bool,
    cycle_drop_start: u32,
    cycle_confirmed_start: u32,
    cycle_xp_start: i32,
    gem_observed: bool,
    gem_retained: bool,
    gem_count: i32,
    methods: BTreeSet<String>,
    targets: BTreeSet<String>,
    wedge_triggered: bool,
    stopped_before_drain: bool,
    death_command_sent: bool,
    death_blocked: bool,
    death_restart_tile: Option<(i32, i32, i32)>,
    death_restart_xp: i32,
    death_restart_gathered: bool,
    modal_command_sent: bool,
    modal_observed: bool,
    run_key_changed: bool,
    banked_unusable_tool: Option<i32>,
    failure: Option<String>,
}

struct GatherSlot {
    cell: Cell,
    case: LiveCase,
    target: WorldTile,
    tool_id: i32,
    keep_gem: bool,
    tool_alias: String,
    requested_level: i32,
    phase: Prep,
    snapshot: GameSnapshot,
    pump: Pump,
    last_action: Instant,
    started: bool,
    start_requested: bool,
    baseline: Option<Observation>,
    latest: Option<Observation>,
    unsettled_items: BTreeMap<(i32, i32), Instant>,
    witness: Witness,
    error: Option<String>,
}

impl GatherSlot {
    fn new(cell: Cell, case: LiveCase, target: WorldTile) -> Self {
        Self {
            cell,
            case,
            target,
            tool_id: cell.tool_id(),
            tool_alias: cell.tool_alias(),
            keep_gem: cell == Cell::Mining
                && std::env::var("GATHERER_MINE_KEEP_GEM").as_deref() == Ok("1"),
            requested_level: cell.level(),
            phase: Prep::WaitIngame,
            snapshot: GameSnapshot::new(),
            pump: Pump::new(),
            last_action: Instant::now(),
            started: false,
            start_requested: false,
            baseline: None,
            latest: None,
            unsettled_items: BTreeMap::new(),
            witness: Witness::default(),
            error: None,
        }
    }

    fn publish(&mut self, client: &client::client::Client) -> Observation {
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);
        self.capture()
    }

    fn capture(&self) -> Observation {
        let inventory = self
            .snapshot
            .inventory()
            .iter()
            .filter(|item| item.slot >= 0 && item.def.id >= 0 && item.count > 0)
            .map(|item| {
                (
                    item.slot,
                    InvRow {
                        id: item.def.id,
                        count: item.count,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let product_count = inventory
            .values()
            .filter(|row| self.cell.products().contains(&row.id))
            .map(|row| row.count.max(0))
            .sum();
        let xp = self
            .snapshot
            .stats()
            .iter()
            .find(|stat| stat.name.eq_ignore_ascii_case(self.cell.skill_name()))
            .map_or(0, |stat| stat.xp);
        Observation {
            inventory,
            product_count,
            xp,
        }
    }

    fn frame(&mut self, client: &mut client::client::Client, hold: bool) {
        let observation = self.publish(client);
        if self.started && self.witness.modal_command_sent && self.snapshot.modals().main >= 0 {
            self.witness.modal_observed = true;
        }
        let previous = self.latest.replace(observation.clone());
        if hold {
            return;
        }
        if self.started {
            if let Err(error) = self.observe_running(previous, observation) {
                self.error = Some(error);
            }
        } else if let Err(error) = self.advance_prep(client) {
            self.error = Some(error);
        }
    }

    fn pre_drain_wedge(&self) -> bool {
        self.started
            && self.witness.awaiting_drop
            && !self.witness.wedge_triggered
            && !self.case.is_power()
    }

    fn mark_stop_wedge(&mut self) {
        self.witness.wedge_triggered = true;
        self.witness.stopped_before_drain = true;
    }

    fn mark_death_wedge(&mut self) {
        self.witness.wedge_triggered = true;
        self.witness.death_command_sent = true;
    }

    fn reset_after_stop_for_restart(&mut self) {
        self.started = true;
        self.phase = Prep::Running;
        self.start_requested = false;
        self.witness.awaiting_drop = false;
    }

    fn observe_running(
        &mut self,
        previous: Option<Observation>,
        observation: Observation,
    ) -> Result<(), String> {
        if self
            .witness
            .banked_unusable_tool
            .is_some_and(|id| observation.inventory.values().any(|row| row.id == id))
        {
            return Err("Gatherer fetched the unusable banked pickaxe".into());
        }
        if self.keep_gem {
            let gem_count = observation
                .inventory
                .values()
                .filter(|row| row.id == UNCUT_SAPPHIRE_ID)
                .map(|row| row.count.max(0))
                .sum();
            self.witness.gem_observed |= gem_count > 0;
            self.witness.gem_count = gem_count;
            self.witness.gem_retained = gem_count > 0;
        }
        // Inventory and equipment arrive in separate packets. A removed tool
        // must reappear in an observed container within the settlement bound.
        self.unsettled_items.retain(|&(id, count), _| {
            !observation
                .inventory
                .values()
                .any(|item| item.id == id && item.count == count)
                && !self
                    .snapshot
                    .equipment()
                    .iter()
                    .any(|item| item.def.id == id && item.count == count)
        });
        if let Some((&(id, count), _)) = self
            .unsettled_items
            .iter()
            .find(|(_, since)| since.elapsed() >= Duration::from_secs(2))
        {
            return Err(format!(
                "{}: non-product id {id} count {count} was not conserved",
                self.cell.name()
            ));
        }
        if let Some(previous) = previous {
            for (slot, old) in &previous.inventory {
                if self.cell.products().contains(&old.id) {
                    if !observation.inventory.contains_key(slot) {
                        self.witness.confirmed_drops =
                            self.witness.confirmed_drops.saturating_add(1);
                    }
                } else if !observation.inventory.contains_key(slot)
                    && !self
                        .snapshot
                        .equipment()
                        .iter()
                        .any(|item| item.def.id == old.id && item.count == old.count)
                {
                    self.unsettled_items
                        .entry((old.id, old.count))
                        .or_insert_with(Instant::now);
                }
            }
        }
        self.witness.max_product_count = self
            .witness
            .max_product_count
            .max(observation.product_count);
        self.witness.last_xp = observation.xp;
        if observation.product_count >= self.cycle_product_capacity() as i32
            && !self.witness.awaiting_drop
        {
            self.witness.full_pack_seen = self.witness.full_pack_seen.saturating_add(1);
            self.witness.awaiting_drop = true;
            self.witness.cycle_drop_start = self.witness.last_status_dropped.max(0) as u32;
            self.witness.cycle_confirmed_start = self.witness.confirmed_drops;
            self.witness.cycle_xp_start = observation.xp;
        }
        self.latest = Some(observation);
        Ok(())
    }

    fn cycle_product_capacity(&self) -> u32 {
        28 - self.baseline.as_ref().map_or(0, |baseline| {
            baseline
                .inventory
                .values()
                .filter(|row| !self.cell.products().contains(&row.id))
                .count() as u32
        })
    }

    fn chat_has(&self, needle: &str) -> bool {
        let needle = needle.to_ascii_lowercase();
        self.snapshot
            .chat_lines()
            .iter()
            .any(|line| line.text.to_ascii_lowercase().contains(&needle))
            || self
                .snapshot
                .chat_modal_texts()
                .iter()
                .any(|text| text.to_ascii_lowercase().contains(&needle))
    }

    fn inventory_tab_available(&self) -> bool {
        self.snapshot
            .side_tabs()
            .iter()
            .any(|tab| tab.index == 3 && tab.available)
    }

    fn item_in_inventory(&self) -> bool {
        self.snapshot
            .inventory()
            .iter()
            .any(|item| item.def.id == self.tool_id && item.count > 0)
    }

    fn item_equipped(&self) -> bool {
        self.snapshot
            .equipment()
            .iter()
            .any(|item| item.def.id == self.tool_id && item.count > 0)
    }

    fn near_target(&self) -> bool {
        self.snapshot.tile().is_some_and(|tile| {
            tile.2 == self.target.level
                && (tile.0 - self.target.x)
                    .abs()
                    .max((tile.1 - self.target.z).abs())
                    <= 6
        })
    }

    fn stat_level(&self) -> i32 {
        self.snapshot
            .stats()
            .iter()
            .find(|stat| stat.name.eq_ignore_ascii_case(self.cell.skill_name()))
            .map_or(0, |stat| stat.base.max(stat.effective))
    }

    fn advance_prep(&mut self, client: &mut client::client::Client) -> Result<(), String> {
        match self.phase {
            Prep::WaitIngame => {
                if self.snapshot.ingame() && self.snapshot.scene_state() == 2 {
                    self.phase = Prep::TutSkip;
                }
            }
            Prep::TutSkip => {
                send_cheat(client, "setvar tutorial 1000")?;
                send_cheat(client, "getvar tutorial")?;
                self.phase = Prep::WaitTutorial;
            }
            Prep::WaitTutorial => {
                if self.chat_has("get tutorial: 1000") {
                    self.phase = Prep::Relog;
                }
            }
            Prep::Relog => {
                let ifaces = Arc::clone(&client.ifaces);
                if !interact::logout(client, &ifaces) {
                    return Err("logout interface unavailable during fixture preparation".into());
                }
                self.phase = Prep::WaitRelog;
            }
            Prep::WaitRelog => {
                if self.snapshot.ingame()
                    && self.snapshot.scene_state() == 2
                    && self.inventory_tab_available()
                {
                    self.phase = Prep::Seed;
                }
            }
            Prep::Seed => {
                send_cheat(
                    client,
                    &format!(
                        "setstat {} {}",
                        self.cell.skill_name(),
                        self.requested_level
                    ),
                )?;
                send_cheat(
                    client,
                    if self.cell == Cell::Mining {
                        "setstat attack 1"
                    } else {
                        "setstat attack 5"
                    },
                )?;
                // Isolate gathering from the mine's aggressive scorpions.
                send_cheat(client, "setstat defence 99")?;
                send_cheat(client, "setstat hitpoints 99")?;
                if self.keep_gem {
                    send_cheat(client, "give uncut_sapphire 1")?;
                }
                send_cheat(client, &format!("give {} 1", self.tool_alias))?;
                send_cheat(
                    client,
                    &interact::tele_args(self.target.level, self.target.x, self.target.z),
                )?;
                self.phase = Prep::WaitSeed;
            }
            Prep::WaitSeed => {
                if self.snapshot.ingame()
                    && self.snapshot.scene_state() == 2
                    && self.near_target()
                    && self.item_in_inventory()
                    && self.stat_level() >= self.requested_level
                {
                    if self.cell == Cell::Mining {
                        send_cheat(client, "givebank rune_pickaxe 1")?;
                        send_cheat(client, &interact::tele_args(0, 2809, 3441))?;
                        self.phase = Prep::OpenBank;
                    } else {
                        self.phase = Prep::Equip;
                    }
                }
            }
            Prep::Equip => {
                if self.item_equipped() {
                    self.phase = Prep::Ready;
                } else if self.last_action.elapsed() >= Duration::from_millis(400) {
                    match Interactions::new(&self.snapshot, client).wear(self.tool_id) {
                        SendResult::Sent { .. } | SendResult::Refused { .. } => {
                            self.last_action = Instant::now();
                        }
                    }
                    self.phase = Prep::WaitEquip;
                }
            }
            Prep::WaitEquip => {
                if self.item_equipped() {
                    self.phase = Prep::Ready;
                } else if self.last_action.elapsed() >= Duration::from_millis(800) {
                    self.phase = Prep::Equip;
                }
            }
            Prep::OpenBank => {
                if self.snapshot.tile() == Some((2809, 3441, 0))
                    && self.last_action.elapsed() >= Duration::from_millis(800)
                {
                    if let Some(loc) = self.snapshot.locs().iter().find(|loc| {
                        loc.tile.x == 2809
                            && loc.tile.z == 3442
                            && loc.tile.level == 0
                            && loc.name.as_deref() == Some("Bank booth")
                    }) {
                        if matches!(
                            Interactions::new(&self.snapshot, client)
                                .open_booth_at(loc.tile, loc.id),
                            SendResult::Sent { .. }
                        ) {
                            self.phase = Prep::WaitBank;
                            self.last_action = Instant::now();
                        }
                    }
                }
            }
            Prep::WaitBank => {
                if self.snapshot.bank_loaded() {
                    if let Some(row) = self.snapshot.bank().iter().find(|row| {
                        row.def.name.as_deref() == Some("Rune pickaxe") && row.count == 1
                    }) {
                        self.witness.banked_unusable_tool = Some(row.def.id);
                        if matches!(
                            Interactions::new(&self.snapshot, client).close_modal(),
                            SendResult::Sent { .. }
                        ) {
                            self.phase = Prep::WaitBankClose;
                        }
                    }
                }
            }
            Prep::WaitBankClose => {
                if !self.snapshot.bank_loaded() && self.snapshot.bank_component_id() < 0 {
                    send_cheat(
                        client,
                        &interact::tele_args(self.target.level, self.target.x, self.target.z),
                    )?;
                    self.phase = Prep::ReturnFromBank;
                }
            }
            Prep::ReturnFromBank => {
                if self.near_target() && self.snapshot.scene_state() == 2 {
                    self.phase = Prep::Ready;
                }
            }
            Prep::Ready | Prep::Running => {}
        }
        Ok(())
    }

    fn mark_started(&mut self) -> Result<Map<String, Value>, String> {
        if self.phase != Prep::Ready {
            return Err(format!(
                "{} Start reached {:?}, not Ready",
                self.cell.name(),
                self.phase
            ));
        }
        let baseline = self.latest.clone().ok_or("missing Start baseline")?;
        if baseline.product_count != 0 {
            return Err(format!(
                "{} Start baseline already has {} products",
                self.cell.name(),
                baseline.product_count
            ));
        }
        let tool_ready = match self.cell {
            Cell::Woodcutting => self.item_equipped(),
            Cell::Mining => self.item_in_inventory() && self.witness.banked_unusable_tool.is_some(),
        };
        if !tool_ready {
            return Err(format!(
                "{} Start baseline lacks its required held/banked tool fixture",
                self.cell.name()
            ));
        }
        if self.keep_gem {
            let gem_count = baseline
                .inventory
                .values()
                .filter(|row| row.id == UNCUT_SAPPHIRE_ID)
                .map(|row| row.count.max(0))
                .sum();
            if gem_count == 0 {
                return Err(
                    "Mining Start baseline lacks the requested retained uncut sapphire fixture"
                        .into(),
                );
            }
            self.witness.gem_observed = true;
            self.witness.gem_retained = true;
            self.witness.gem_count = gem_count;
        }
        self.baseline = Some(baseline);
        self.phase = Prep::Running;
        self.started = true;
        self.start_requested = false;
        Ok(self.cell.settings())
    }

    fn apply_status(&mut self, status: &script::native::ScriptStatus) -> Result<(), String> {
        if status.card != script::CompiledId("Gatherer") {
            return Err(format!(
                "unexpected card in {} status: {:?}",
                self.cell.name(),
                status.card
            ));
        }
        if self.cell == Cell::Mining {
            for field in status
                .fields
                .iter()
                .filter(|field| field.key == "method" || field.key == "target")
            {
                if let script::native::StatusValue::Text(value) = &field.value {
                    if field.key == "method" {
                        self.witness.methods.insert(value.to_string());
                    } else {
                        self.witness.targets.insert(value.to_string());
                    }
                }
            }
        }
        self.witness.last_status_yielded =
            integer_field(status, "yielded").unwrap_or(self.witness.last_status_yielded);
        self.witness.last_status_dropped =
            integer_field(status, "dropped").unwrap_or(self.witness.last_status_dropped);
        if let Some(failure) = &status.failure {
            self.witness.failure = Some(format!("{}: {}", failure.code, failure.message));
            if self.case == LiveCase::DeathDuringDrop && failure.code.as_ref() == "died" {
                self.witness.death_blocked = true;
                return Ok(());
            }
            return Err(format!("{} script failure: {failure:?}", self.cell.name()));
        }
        if self.witness.death_restart_tile.is_some()
            && self.witness.last_status_yielded > 0
            && self.witness.last_xp > self.witness.death_restart_xp
        {
            self.witness.death_restart_gathered = true;
        }
        if self.witness.awaiting_drop {
            let empty = self
                .latest
                .as_ref()
                .is_some_and(|observation| observation.product_count == 0);
            let dropped = self.witness.last_status_dropped.max(0) as u32;
            if empty
                && dropped > self.witness.cycle_drop_start
                && self
                    .witness
                    .confirmed_drops
                    .saturating_sub(self.witness.cycle_confirmed_start)
                    >= self.cycle_product_capacity()
            {
                self.witness.cycles = self.witness.cycles.saturating_add(1);
                self.witness.awaiting_drop = false;
                self.witness.awaiting_post_drop_gather = true;
                self.witness.cycle_xp_start = self
                    .latest
                    .as_ref()
                    .map_or(self.witness.cycle_xp_start, |observation| observation.xp);
            }
        }
        if self.witness.awaiting_post_drop_gather {
            if let Some(observation) = self.latest.as_ref() {
                if observation.product_count > 0 && observation.xp > self.witness.cycle_xp_start {
                    self.witness.post_drop_gathers =
                        self.witness.post_drop_gathers.saturating_add(1);
                    self.witness.awaiting_post_drop_gather = false;
                    self.witness.cycle_xp_start = observation.xp;
                }
            }
        }
        Ok(())
    }

    fn qualifies(&self) -> Result<(), String> {
        match self.case {
            LiveCase::Power => {
                if !self.witness.modal_observed {
                    return Err("power fixture did not observe its disposal modal".into());
                }
                if self.witness.cycles < REQUIRED_CYCLES {
                    return Err(format!(
                        "{} only completed {} full cycles (full packs {}, confirmed drops {})",
                        self.cell.name(),
                        self.witness.cycles,
                        self.witness.full_pack_seen,
                        self.witness.confirmed_drops
                    ));
                }
                if self.witness.post_drop_gathers < REQUIRED_POST_DROP_GATHERS {
                    return Err(format!(
                        "{} only observed {} post-disposal gathers",
                        self.cell.name(),
                        self.witness.post_drop_gathers
                    ));
                }
                if self.witness.confirmed_drops < REQUIRED_CYCLES * self.cycle_product_capacity() {
                    return Err(format!(
                        "{} confirmed only {} empty product slots",
                        self.cell.name(),
                        self.witness.confirmed_drops
                    ));
                }
                if self.keep_gem && (!self.witness.gem_observed || !self.witness.gem_retained) {
                    return Err(format!(
                        "{} retained-gem proof incomplete: observed={} retained={} count={}",
                        self.cell.name(),
                        self.witness.gem_observed,
                        self.witness.gem_retained,
                        self.witness.gem_count
                    ));
                }
                if self.witness.last_status_yielded <= 0
                    || self.witness.last_xp <= self.baseline_xp()
                {
                    return Err(format!(
                        "{} had no observed yield/XP: yielded={} xp={}",
                        self.cell.name(),
                        self.witness.last_status_yielded,
                        self.witness.last_xp
                    ));
                }
                if self.cell == Cell::Mining
                    && self.witness.methods.len() < 2
                    && self.cell.resources().len() >= 2
                {
                    return Err(format!(
                        "{} did not show tier preference/depletion fallback; methods={:?} targets={:?}",
                        self.cell.name(),
                        self.witness.methods,
                        self.witness.targets
                    ));
                }
                Ok(())
            }
            LiveCase::CancelBeforeDrain => {
                if !self.witness.stopped_before_drain || self.witness.confirmed_drops != 0 {
                    return Err(format!(
                        "{} cancellation did not clear the pre-drain drop outbox: {:?}",
                        self.cell.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::DeathDuringDrop => {
                if !self.witness.death_command_sent
                    || !self.witness.death_blocked
                    || !self.witness.death_restart_gathered
                {
                    return Err(format!(
                        "{} death-during-drop did not settle Blocked(died): {:?}",
                        self.cell.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::RunKeyChangeDuringDrop => {
                if !self.witness.run_key_changed
                    || !self.witness.stopped_before_drain
                    || self.witness.confirmed_drops < 28
                {
                    return Err(format!(
                        "{} run-key wedge did not stop before drain and restart: {:?}",
                        self.cell.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
        }
    }

    fn baseline_xp(&self) -> i32 {
        self.baseline
            .as_ref()
            .map_or(0, |observation| observation.xp)
    }
}

fn integer_field(status: &script::native::ScriptStatus, key: &str) -> Option<i64> {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match &field.value {
            script::native::StatusValue::Integer(value) => Some(*value),
            _ => None,
        })
}

fn send_cheat(client: &mut client::client::Client, command: &str) -> Result<(), String> {
    if interact::cheat(client, command).is_sent() {
        Ok(())
    } else {
        Err(format!("fixture command refused: {command}"))
    }
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Result<Self, String> {
        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("clock: {error}"))?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("274bot-gatherer-{label}-{serial}"));
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
    std::env::var(name).map_err(|_| format!("{name} is required for Gatherer live qualification"))
}

fn parse_tile(name: &str, raw: &str) -> Result<WorldTile, String> {
    let mut parts = raw.split(',').map(str::trim);
    let x = parts
        .next()
        .ok_or_else(|| format!("{name} must be x,z,level"))?
        .parse()
        .map_err(|_| format!("{name} has invalid x: {raw}"))?;
    let z = parts
        .next()
        .ok_or_else(|| format!("{name} must be x,z,level"))?
        .parse()
        .map_err(|_| format!("{name} has invalid z: {raw}"))?;
    let level = parts
        .next()
        .ok_or_else(|| format!("{name} must be x,z,level"))?
        .parse()
        .map_err(|_| format!("{name} has invalid level: {raw}"))?;
    if parts.next().is_some() {
        return Err(format!("{name} must be x,z,level: {raw}"));
    }
    Ok(WorldTile { x, z, level })
}

fn mint_profile(account: &str, password: &str, offset: i32) -> Result<Profile, String> {
    let uid = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("clock: {error}"))?
        .as_millis()
        .checked_rem(i32::MAX as u128)
        .ok_or("uid clock overflow")? as i32;
    Ok(Profile {
        username: account.to_owned(),
        password: password.to_owned().into(),
        uid: uid.saturating_add(offset),
        settings: ProfileSettings::default(),
    })
}

fn selected_profile(
    nav_pack: PathBuf,
    catalog_root: PathBuf,
    temp: &Path,
) -> Result<(Arc<host_play::ServerProfile>, Arc<SharedClientTemplate>), String> {
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(
            std::env::var("GATHERER_GAME_PORT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(45594),
        ),
        http_port: Some(
            std::env::var("GATHERER_HTTP_PORT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(2080),
        ),
        nav_pack: Some(nav_pack),
        nav_flags: std::env::var_os("GATHERER_NAV_FLAGS").map(PathBuf::from),
        engine_dir: std::env::var_os("GATHERER_ENGINE_DIR").map(PathBuf::from),
        vault_path: Some(temp.join("vault")),
        catalog_root: Some(catalog_root),
        ..ProfileOptions::default()
    };
    let profile = options.resolve(None)?.bind()?;
    if profile.profile_class() != host_play::ProfileClass::Local
        || profile.client().game_host() != "127.0.0.1"
    {
        return Err("Gatherer live qualification requires a loopback local profile".into());
    }
    let template = SharedClientTemplate::load(Arc::clone(&profile))?;
    if template.world().is_none() {
        return Err("selected template has no navigation world".into());
    }
    Ok((profile, template))
}

fn run_cell(cell: Cell, case: LiveCase) -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Ok(());
    }
    let _home = script::IsolatedEnv::enter(cell.name());
    api::hostlog::set_debug(true);
    let nav_pack = PathBuf::from(required("GATHERER_NAV_PACK")?);
    let catalog_root = PathBuf::from(required("GATHERER_CATALOG_ROOT")?);
    if !nav_pack.is_file() {
        return Err(format!(
            "GATHERER_NAV_PACK is not a file: {}",
            nav_pack.display()
        ));
    }
    if !catalog_root.is_dir() {
        return Err(format!(
            "GATHERER_CATALOG_ROOT is not a directory: {}",
            catalog_root.display()
        ));
    }
    let target = parse_tile(cell.tile_env(), &required(cell.tile_env())?)?;
    let temp = TempRoot::new(cell.name())?;
    let (profile, template) = selected_profile(nav_pack, catalog_root, temp.path())?;
    let names = host_play::mint_live_names(1);
    let credentials = host_play::mint_live_entries(&names);
    let account = names.first().cloned().ok_or("failed to mint account")?;
    let password = credentials
        .first()
        .map(|entry| entry.1.clone())
        .ok_or("failed to mint password")?;
    let state = Arc::new(Mutex::new(GatherSlot::new(cell, case, target)));
    let start_handle: Arc<Mutex<Option<ScriptStartHandle>>> = Arc::new(Mutex::new(None));
    let frame_state = Arc::clone(&state);
    let frame_handle = Arc::clone(&start_handle);
    let frame_account = account.clone();
    let mut play = host_play::run_with_template(
        Arc::clone(&template),
        true,
        vec![],
        |_| (None, None),
        move |client, username, hold| {
            if username != frame_account {
                return;
            }
            let (do_wedge, open_modal) = frame_state
                .lock()
                .ok()
                .map(|slot| {
                    (
                        !hold && slot.pre_drain_wedge(),
                        !hold
                            && slot.case == LiveCase::Power
                            && slot.witness.awaiting_drop
                            && !slot.witness.modal_command_sent,
                    )
                })
                .unwrap_or((false, false));
            if open_modal && interact::cheat(client, "openmain book4").is_sent() {
                if let Ok(mut slot) = frame_state.lock() {
                    slot.witness.modal_command_sent = true;
                }
            }
            if do_wedge {
                let mode = frame_state.lock().ok().map(|slot| slot.case);
                match mode {
                    Some(LiveCase::CancelBeforeDrain | LiveCase::RunKeyChangeDuringDrop) => {
                        if let Some(handle) =
                            frame_handle.lock().ok().and_then(|guard| guard.clone())
                        {
                            if handle.stop(&frame_account).is_ok() {
                                if let Ok(mut slot) = frame_state.lock() {
                                    slot.mark_stop_wedge();
                                }
                            }
                        }
                    }
                    Some(LiveCase::DeathDuringDrop)
                        if interact::cheat(client, "~death").is_sent() =>
                    {
                        if let Ok(mut slot) = frame_state.lock() {
                            slot.mark_death_wedge();
                        }
                    }
                    _ => {}
                }
            }
            if let Ok(mut slot) = frame_state.lock() {
                slot.frame(client, hold);
            }
        },
    )?;
    if let Ok(mut handle) = start_handle.lock() {
        *handle = Some(play.script_start_handle());
    }
    play.try_spawn_slot(mint_profile(&account, &password, 1)?, None, None, None)?;
    let settings = cell.settings();
    println!(
        "{}",
        json!({
            "phase": "identity",
            "cell": cell.name(),
            "live_case": case.name(),
            "profile": profile.label(),
            "game_port": profile.client().game_port(),
            "nav_pack": profile.nav_pack(),
            "account": account,
            "target": {"x": target.x, "z": target.z, "level": target.level},
            "settings": settings,
            "driver_trace": "enabled; native-packet account/tick/count lines are the packet witness",
        })
    );
    let started_at = Instant::now();
    let mut restart_requested = false;
    let mut death_stop_requested = false;
    let mut prior_run = None;
    let mut reported = (0, 0, 0);
    let result = loop {
        let lifecycle_error = play.script_last_error(&account);
        let (phase, start_requested, slot_error, witness) = {
            let slot = state.lock().map_err(|_| "live state poisoned")?;
            (
                slot.phase,
                slot.start_requested,
                slot.error.clone(),
                slot.witness.clone(),
            )
        };
        if let Some(error) = lifecycle_error {
            let death_receipt = case == LiveCase::DeathDuringDrop && witness.death_command_sent;
            if !death_receipt {
                break Err(format!("{} lifecycle error: {error}", cell.name()));
            }
        }
        if let Some(error) = slot_error {
            break Err(error);
        }
        if phase == Prep::Ready && !start_requested {
            let start_bag = {
                let mut slot = state.lock().map_err(|_| "live state poisoned")?;
                slot.start_requested = true;
                match slot.mark_started() {
                    Ok(settings) => settings,
                    Err(error) => break Err(error),
                }
            };
            if let Err(error) =
                play.script_start(&account, script::CompiledId("Gatherer"), start_bag)
            {
                break Err(format!("{} Start failed: {error}", cell.name()));
            }
            println!("{}", json!({"phase": "start", "cell": cell.name()}));
        }
        let current_run = play.script_native_run(&account);
        if case == LiveCase::DeathDuringDrop && witness.death_blocked {
            if !death_stop_requested {
                play.script_stop(&account);
                death_stop_requested = true;
            } else if !restart_requested && current_run.is_none() {
                let mut slot = state.lock().map_err(|_| "live state poisoned")?;
                let respawned = slot.snapshot.ingame()
                    && slot.snapshot.scene_state() == 2
                    && slot.snapshot.tile().is_some_and(|(x, z, level)| {
                        level == 0 && (x - 3222).abs().max((z - 3218).abs()) < 12
                    })
                    && slot.snapshot.stats().iter().any(|stat| {
                        stat.name.eq_ignore_ascii_case("Hitpoints") && stat.effective > 0
                    });
                if respawned {
                    slot.witness.death_restart_tile = slot.snapshot.tile();
                    slot.witness.death_restart_xp = slot.latest.as_ref().map_or(0, |row| row.xp);
                    slot.witness.last_status_yielded = 0;
                    slot.reset_after_stop_for_restart();
                    // The castle respawn courtyard has no admitted normal tree
                    // within twelve tiles; this fresh Start selects a wider area.
                    let mut restart_settings = settings.clone();
                    restart_settings.insert("radius".into(), json!(32));
                    println!(
                        "{}",
                        json!({
                            "phase": "death-restart",
                            "tile": slot.witness.death_restart_tile,
                            "xp": slot.witness.death_restart_xp,
                            "radius": 32,
                        })
                    );
                    drop(slot);
                    if let Err(error) = play.script_start(
                        &account,
                        script::CompiledId("Gatherer"),
                        restart_settings,
                    ) {
                        break Err(format!("{} death restart failed: {error}", cell.name()));
                    }
                    restart_requested = true;
                }
            }
        }
        if case == LiveCase::RunKeyChangeDuringDrop && !witness.wedge_triggered {
            if let Some(run) = current_run {
                prior_run = Some(run);
            }
        }
        if case == LiveCase::RunKeyChangeDuringDrop
            && witness.stopped_before_drain
            && !restart_requested
            && current_run.is_none()
        {
            if let Err(error) =
                play.script_start(&account, script::CompiledId("Gatherer"), settings.clone())
            {
                break Err(format!("{} restart failed: {error}", cell.name()));
            }
            restart_requested = true;
            if let Ok(mut slot) = state.lock() {
                slot.reset_after_stop_for_restart();
            }
        }
        if case == LiveCase::RunKeyChangeDuringDrop
            && witness.stopped_before_drain
            && restart_requested
        {
            if let (Some(old), Some(current)) = (prior_run, current_run) {
                if old != current {
                    if let Ok(mut slot) = state.lock() {
                        slot.witness.run_key_changed = true;
                    }
                }
            }
        }
        if let Some(status) = play.script_native_status(&account) {
            let mut slot = state.lock().map_err(|_| "live state poisoned")?;
            if let Err(error) = slot.apply_status(&status) {
                slot.error = Some(error);
            }
        }
        let (witness, latest, product_capacity) = {
            let slot = state.lock().map_err(|_| "live state poisoned")?;
            (
                slot.witness.clone(),
                slot.latest.clone(),
                slot.cycle_product_capacity(),
            )
        };
        if reported
            != (
                witness.cycles,
                witness.post_drop_gathers,
                witness.confirmed_drops,
            )
        {
            reported = (
                witness.cycles,
                witness.post_drop_gathers,
                witness.confirmed_drops,
            );
            println!(
                "{}",
                json!({
                    "phase": "progress",
                    "cell": cell.name(),
                    "live_case": case.name(),
                    "cycles": witness.cycles,
                    "post_drop_gathers": witness.post_drop_gathers,
                    "confirmed_drops": witness.confirmed_drops,
                    "product_capacity": product_capacity,
                    "gem_observed": witness.gem_observed,
                    "gem_retained": witness.gem_retained,
                    "gem_count": witness.gem_count,
                    "yielded": witness.last_status_yielded,
                    "dropped": witness.last_status_dropped,
                    "xp": witness.last_xp,
                    "methods": witness.methods,
                })
            );
        }
        let done = match case {
            LiveCase::Power => {
                witness.cycles >= REQUIRED_CYCLES
                    && witness.post_drop_gathers >= REQUIRED_POST_DROP_GATHERS
            }
            LiveCase::CancelBeforeDrain => witness.stopped_before_drain,
            LiveCase::DeathDuringDrop => witness.death_restart_gathered,
            LiveCase::RunKeyChangeDuringDrop => {
                witness.run_key_changed && witness.confirmed_drops >= 28
            }
        };
        if done
            && state
                .lock()
                .map_err(|_| "live state poisoned")?
                .unsettled_items
                .is_empty()
        {
            break Ok(());
        }
        let deadline = if phase == Prep::Running {
            if case.is_power() {
                started_at + PREP_DEADLINE + POWER_DEADLINE
            } else {
                started_at + PREP_DEADLINE + WEDGE_DEADLINE
            }
        } else {
            started_at + PREP_DEADLINE
        };
        if Instant::now() >= deadline {
            break Err(format!(
                "{} timeout: phase={phase:?} witness={witness:?} latest={latest:?}",
                cell.name()
            ));
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    let (witness, product_capacity) = {
        let slot = state.lock().map_err(|_| "live state poisoned")?;
        (slot.witness.clone(), slot.cycle_product_capacity())
    };
    println!(
        "{}",
        json!({
            "phase": "witness",
            "cell": cell.name(),
            "live_case": case.name(),
            "cycles": witness.cycles,
            "post_drop_gathers": witness.post_drop_gathers,
            "confirmed_drops": witness.confirmed_drops,
            "full_pack_seen": witness.full_pack_seen,
            "product_capacity": product_capacity,
            "gem_observed": witness.gem_observed,
            "gem_retained": witness.gem_retained,
            "gem_count": witness.gem_count,
            "stopped_before_drain": witness.stopped_before_drain,
            "death_command_sent": witness.death_command_sent,
            "death_blocked": witness.death_blocked,
            "death_restart_tile": witness.death_restart_tile,
            "death_restart_gathered": witness.death_restart_gathered,
            "run_key_changed": witness.run_key_changed,
            "modal_command_sent": witness.modal_command_sent,
            "banked_unusable_tool": witness.banked_unusable_tool,
            "yielded": witness.last_status_yielded,
            "dropped": witness.last_status_dropped,
            "xp": witness.last_xp,
            "methods": witness.methods,
            "targets": witness.targets,
            "failure": witness.failure,
            "driver_trace": "host debug enabled; inspect native-packet account/tick/count lines",
        })
    );
    play.script_stop(&account);
    play.stop_slot(&account);
    result?;
    let qualification = state.lock().map_err(|_| "live state poisoned")?.qualifies();
    qualification
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_* fixture settings and local 289 engine"]
fn gatherer_wc_power() {
    run_cell(Cell::Woodcutting, LiveCase::Power).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_* fixture settings and local 289 engine"]
fn gatherer_mine_tier_power() {
    run_cell(Cell::Mining, LiveCase::Power).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_WC_TILE and local 289 engine"]
fn gatherer_cancel_before_drain_live() {
    run_cell(Cell::Woodcutting, LiveCase::CancelBeforeDrain).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_WC_TILE and local 289 engine"]
fn gatherer_death_during_drop_live() {
    run_cell(Cell::Woodcutting, LiveCase::DeathDuringDrop).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_WC_TILE and local 289 engine"]
fn gatherer_run_key_change_during_drop_live() {
    run_cell(Cell::Woodcutting, LiveCase::RunKeyChangeDuringDrop).unwrap();
}
