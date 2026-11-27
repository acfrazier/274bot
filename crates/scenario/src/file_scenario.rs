//! File-scenario admission and checks. This layer has no script runtime authority.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use crate::file_journal::{
    CheckContext, JournalData, JournalRecord, MarkData, RecordKind, RecordSource,
};
pub use crate::file_journal::{FileJournal, Mark};
use api::game_data::SelectedGameData;
use api::snapshot::{ActorKind, GameSnapshot, ItemView, WorldTile};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::named_predicates::{BankExactParams, StandingBankClosedParams};
use crate::{
    NamedPredicateGate, NamedPredicateParams, PredicateAuthority, PredicateContext, Proof,
    ScenarioSettings,
};

const MAX_ROLES: usize = 10;

/// Ordinary runs use only the immutable whole-run deadline until a profile cap is approved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QuiescePolicy {
    WholeDeadlineOnly,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FilePreamble {
    Tutorial,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileRole {
    pub fresh: Option<String>,
    pub mainland: bool,
    pub preamble: Option<FilePreamble>,
}

#[derive(Debug)]
pub struct FileScenario {
    pub name: String,
    pub case: String,
    pub deadline: Duration,
    pub idle_ticks: u32,
    pub roles: BTreeMap<String, FileRole>,
    pub settings: ScenarioSettings,
    pub terminal_shot: Option<String>,
    pub death_allowed: bool,
    pub confirmed_only: bool,
    pub limits: Vec<String>,
    pub proof: FileCheck,
    pub quiesce: QuiescePolicy,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    name: String,
    revision: Option<u16>,
    deadline_s: Option<u64>,
    idle_ticks: Option<u32>,
    cases: Option<Vec<String>>,
    accounts: Option<BTreeMap<String, FileRole>>,
    #[serde(default)]
    headed: bool,
    #[serde(default)]
    view: View,
    proof: Option<Value>,
    terminal_shot: Option<String>,
    #[serde(default)]
    death: Death,
    #[serde(default)]
    seeds: Seeds,
    #[serde(default)]
    limits: Vec<String>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct View {
    renderer: Option<bool>,
    full_rate: Option<bool>,
    capture: Option<bool>,
    nav_paint: Option<bool>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Death {
    #[default]
    Fail,
    Allowed,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Seeds {
    #[default]
    Any,
    ConfirmedOnly,
}

pub fn file_scenario(
    header: Value,
    requested_case: Option<&str>,
    revision: u16,
    headed: bool,
) -> Result<FileScenario, String> {
    let Header {
        name,
        revision: required,
        deadline_s,
        idle_ticks,
        cases,
        accounts,
        headed: requires_head,
        view,
        proof,
        terminal_shot,
        death,
        seeds,
        limits,
    } = serde_json::from_value(header).map_err(|e| format!("scenario_header: {e}"))?;
    bounded_text(&name, 64, "name")?;
    if let Some(required) = required {
        if !matches!(required, 274 | 289 | 377) || required != revision {
            return Err(format!(
                "scenario_header.revision: required {required}, selected {revision}"
            ));
        }
    }
    if requires_head && !headed {
        return Err("scenario_header.headed: a headed caller is required".into());
    }
    let deadline_s = deadline_s.unwrap_or(180);
    let idle_ticks = idle_ticks.unwrap_or(100);
    if !(1..=7200).contains(&deadline_s) || !(1..=12000).contains(&idle_ticks) {
        return Err("scenario_header: deadline_s must be 1–7200 and idle_ticks 1–12000".into());
    }
    let cases = cases.unwrap_or_else(|| vec!["default".into()]);
    let mut seen = BTreeSet::new();
    for case in &cases {
        bounded_text(case, 160, "case")?;
        if !seen.insert(case.as_str()) {
            return Err(format!("scenario_header.cases: duplicate {case:?}"));
        }
    }
    let case = requested_case
        .or_else(|| cases.first().map(String::as_str))
        .ok_or("scenario_header.cases: empty list")?;
    if !seen.contains(case) {
        return Err(format!("scenario_header.cases: unknown case {case:?}"));
    }
    let roles = accounts.unwrap_or_else(|| BTreeMap::from([("main".into(), FileRole::default())]));
    if !(1..=MAX_ROLES).contains(&roles.len()) {
        return Err("scenario_header.accounts: expected 1–10 roles".into());
    }
    for (role, options) in &roles {
        bounded_text(role, 160, "role")?;
        if let Some(fresh) = &options.fresh {
            if !(1..=4).contains(&fresh.len())
                || !fresh.bytes().all(|byte| byte.is_ascii_alphanumeric())
            {
                return Err(
                    "scenario_header.accounts.fresh: expected 1–4 ASCII letters or digits".into(),
                );
            }
        }
    }
    for limit in &limits {
        bounded_text(limit, 160, "limit")?;
    }
    if let Some(label) = &terminal_shot {
        bounded_text(label, 160, "terminal shot")?;
    }
    let proof = proof.map_or(
        Ok(FileCheck::Native(Proof::IngameScene2)),
        FileCheck::from_literal,
    )?;
    if !proof.terminal() {
        return Err("scenario_header.proof: expected a snapshot predicate".into());
    }
    let deadline = Duration::from_secs(deadline_s);
    let mut settings = ScenarioSettings {
        deadline,
        ..ScenarioSettings::default()
    };
    if let Some(value) = view.renderer {
        settings.renderer = value;
    }
    if let Some(value) = view.capture {
        settings.capture = value;
    }
    if let Some(value) = view.full_rate {
        settings.full_rate = value;
    }
    if let Some(value) = view.nav_paint {
        settings.nav.show_nav_path = value;
    }
    Ok(FileScenario {
        name,
        case: case.into(),
        deadline,
        idle_ticks,
        roles,
        settings,
        terminal_shot,
        death_allowed: matches!(death, Death::Allowed),
        confirmed_only: matches!(seeds, Seeds::ConfirmedOnly),
        limits,
        proof,
        quiesce: QuiescePolicy::WholeDeadlineOnly,
    })
}

fn bounded_text(text: &str, bound: usize, field: &str) -> Result<(), String> {
    if text.is_empty() || text.len() > bound || text.chars().any(char::is_control) {
        return Err(format!(
            "scenario_header.{field}: expected 1–{bound} bytes of printable text"
        ));
    }
    Ok(())
}

pub(super) type Slots = Vec<(i32, i32, i32)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container {
    Inventory,
    Equipment,
    Bank,
    Component(i32),
}

impl Container {
    pub(super) fn parse(value: Value) -> Result<Self, String> {
        match value.as_str() {
            Some("inventory") => Ok(Self::Inventory),
            Some("worn") => Ok(Self::Equipment),
            Some("bank") => Ok(Self::Bank),
            Some(_) => Err("unknown container".into()),
            None => integer(&value, 0, i32::MAX, "where").map(Self::Component),
        }
    }

    pub(super) fn slots(self, snapshot: &GameSnapshot) -> Option<Slots> {
        self.items(snapshot).map(slots_of)
    }

    fn items(self, snapshot: &GameSnapshot) -> Option<&[ItemView]> {
        match self {
            Self::Inventory => (snapshot.inventory_ready() && snapshot.inventory_size() > 0)
                .then(|| snapshot.inventory()),
            Self::Equipment => snapshot.equipment_posted().then(|| snapshot.equipment()),
            Self::Bank => Proof::BankOpen
                .check(snapshot, None)
                .then(|| snapshot.bank()),
            Self::Component(id) => {
                if snapshot.inventory_ready()
                    && snapshot
                        .inventory()
                        .iter()
                        .any(|row| row.component_id == id)
                {
                    Some(snapshot.inventory())
                } else if snapshot.equipment_posted()
                    && snapshot
                        .equipment()
                        .iter()
                        .any(|row| row.component_id == id)
                {
                    Some(snapshot.equipment())
                } else if Proof::BankOpen.check(snapshot, None)
                    && snapshot.bank().iter().any(|row| row.component_id == id)
                {
                    Some(snapshot.bank())
                } else {
                    None
                }
            }
        }
    }

    fn total(self, snapshot: &GameSnapshot, id: i32) -> Option<i64> {
        self.items(snapshot).map(|rows| {
            self.count(
                rows.iter()
                    .map(|row| (row.component_id, row.def.id, row.count)),
                id,
            )
        })
    }

    fn count(self, rows: impl Iterator<Item = (i32, i32, i32)>, id: i32) -> i64 {
        rows.filter(|(component, object, _)| {
            *object == id
                && (!matches!(self, Self::Component(_))
                    || matches!(self, Self::Component(expected) if expected == *component))
        })
        .map(|(_, _, count)| i64::from(count))
        .sum()
    }
}

pub(super) fn slots_of(items: &[ItemView]) -> Slots {
    items
        .iter()
        .map(|item| (item.slot, item.def.id, item.count))
        .collect()
}

#[derive(Clone, Debug)]
pub struct Amount {
    min: i64,
    max: i64,
}

impl Amount {
    fn matches(&self, n: i64) -> bool {
        (self.min..=self.max).contains(&n)
    }
}

#[derive(Debug)]
pub enum Item {
    Alias(String),
    Id(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exactness {
    Value,
    Packet,
}

#[derive(Clone, Debug)]
pub enum TextMatcher {
    Exact(String),
    Contains(String),
    Prefix(String),
}
impl TextMatcher {
    fn matches(&self, text: &str) -> bool {
        match self {
            Self::Exact(expected) => text == expected,
            Self::Contains(needle) => text.contains(needle),
            Self::Prefix(prefix) => text.starts_with(prefix),
        }
    }
}

#[derive(Clone, Debug)]
pub struct StatMatcher {
    skill: i32,
    base: Option<Amount>,
    effective: Option<Amount>,
    xp: Option<Amount>,
}

#[derive(Clone, Debug)]
pub struct InventoryMatcher {
    container: Option<Container>,
    component: Option<i32>,
    slot: Option<i32>,
    item: Option<i32>,
    delta: Option<Amount>,
}

#[derive(Clone, Debug)]
pub struct MessageMatcher {
    type_: Option<i32>,
    text: Option<TextMatcher>,
}

#[derive(Clone, Debug)]
pub struct VarMatcher {
    id: i32,
    value: Option<Amount>,
}

#[derive(Clone, Debug)]
pub enum RecordMatcher {
    Stat(StatMatcher),
    Inventory(InventoryMatcher),
    Message(MessageMatcher),
    Var(VarMatcher),
}

impl RecordMatcher {
    fn kind(&self) -> RecordKind {
        match self {
            Self::Stat(_) => RecordKind::Stat,
            Self::Inventory(_) => RecordKind::Inventory,
            Self::Message(_) => RecordKind::Message,
            Self::Var(_) => RecordKind::Varp,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SilentMessageFilter {
    type_: Option<i32>,
    allow: Vec<TextMatcher>,
}

#[derive(Clone, Debug)]
pub enum CountEnd {
    Mark(u64),
    Label(String),
}

#[derive(Debug)]
pub enum SnapshotProof {
    ItemIdAny {
        ids: Vec<i32>,
        count: i32,
    },
    BankItemIdAny {
        ids: Vec<i32>,
        count: i32,
    },
    QuestDone {
        name: String,
    },
    NpcIdsAbsentNear {
        ids: Vec<usize>,
        radius: i32,
    },
    GroundItemIdsAbsentNear {
        ids: Vec<i32>,
        x: i32,
        z: i32,
        level: i32,
        radius: i32,
    },
    Npc {
        type_id: usize,
        near: Option<WorldTile>,
        radius: Option<i32>,
        index: Option<usize>,
        form: Option<usize>,
        options: Vec<String>,
        no_option: Option<String>,
        sequence: Option<i32>,
        frame: Option<i32>,
        present: bool,
    },
    Loc {
        at: WorldTile,
        name: Option<String>,
        form: Option<i32>,
        options: Vec<String>,
        first_option: Option<String>,
        no_option: Option<String>,
    },
    NpcNameNear {
        name: String,
        x: i32,
        z: i32,
        level: i32,
        radius: i32,
    },
    NpcNameTargetingLocal {
        name: String,
    },
    LocalTargetingNpcName {
        name: String,
    },
    NpcNameUnengagedInSight {
        name: String,
        radius: i32,
    },
    LocActionNear {
        id: i32,
        x: i32,
        z: i32,
        level: i32,
        radius: i32,
        action: String,
        present: bool,
    },
    VisitedWithin {
        min_x: i32,
        max_x: i32,
        min_z: i32,
        max_z: i32,
        level: i32,
    },
    Chat {
        needle: String,
    },
    FreshTutorial {
        since: u64,
    },
    FreshChat {
        needle: String,
        since: u64,
    },
    FreshChatSincePreviousStep {
        needle: String,
        since: u64,
    },
    FreshQuestCompletion {
        chat_needle: String,
        main_root_component_id: i32,
        component_id: i32,
        text: String,
        since: u64,
    },
    StatXpGain {
        id: i32,
        min: i32,
        since: u64,
    },
    FreshStatXpGain {
        id: i32,
        min: i32,
        since: u64,
    },
    XpDelta {
        id: i32,
        min: i32,
        since: u64,
    },
    ProductDelta {
        id: i32,
        count: i32,
        since: u64,
    },
    InputConsumed {
        id: i32,
        count: i32,
        since: u64,
    },
    ClueReplaced {
        seeded: i32,
        since: u64,
    },
}

#[derive(Debug)]
pub enum FileCheck {
    Native(Proof),
    Named(NamedPredicateGate),
    All(Vec<FileCheck>),
    ActionSent(u64),
    ExactTile {
        x: i32,
        z: i32,
        level: i32,
        radius: i32,
    },
    TileUnchanged(u64),
    Stat {
        skill: i32,
        base: Option<Amount>,
        effective: Option<Amount>,
        xp: Option<Amount>,
    },
    XpSince {
        skill: i32,
        mark: u64,
        amount: Amount,
    },
    Item {
        item: Item,
        container: Container,
        since: Option<u64>,
        amount: Amount,
    },
    ContainerUnchanged {
        container: Container,
        since: u64,
    },
    Received {
        after: u64,
        exact: Exactness,
        stats: Vec<StatMatcher>,
        inventories: Vec<InventoryMatcher>,
        messages: Vec<MessageMatcher>,
        vars: Vec<VarMatcher>,
    },
    Silent {
        after: u64,
        ticks: u64,
        exact: Exactness,
        stats: Vec<StatMatcher>,
        inventories: Vec<InventoryMatcher>,
        vars: Vec<VarMatcher>,
        messages: Option<SilentMessageFilter>,
        dialogue: bool,
    },
    Count {
        record: RecordMatcher,
        from: u64,
        to: Option<CountEnd>,
        amount: Amount,
        exact: Exactness,
    },
    Order {
        first: String,
        then: String,
        gap: Option<Amount>,
    },
    Var {
        id: i32,
        value: Amount,
    },
    Dialogue {
        open: Option<bool>,
        root: Option<i32>,
        continue_component: Option<i32>,
        lines: Option<Vec<String>>,
        options: Option<Vec<String>>,
    },
    Hint {
        kind: String,
        x: i32,
        z: i32,
        level: i32,
        radius: i32,
    },
    Signal(String),
    Snapshot(SnapshotProof),
}

#[derive(Clone, Copy, Debug)]
pub struct CheckField {
    pub name: &'static str,
    pub ty: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct CheckSignature {
    pub name: &'static str,
    pub fields: &'static [CheckField],
}

macro_rules! check_signatures {
    ($($name:literal { $($field:literal : $ty:literal),* $(,)? }),* $(,)?) => {
        pub const FILE_CHECK_SIGNATURES: &[CheckSignature] = &[
            $(CheckSignature {
                name: $name,
                fields: &[$(CheckField { name: $field, ty: $ty }),*],
            }),*
        ];
        pub const FILE_CHECK_ENTRIES: &[(&str, &str, &str)] = &[
            $((concat!("expect.", $name), concat!("watch.", $name), concat!("probe.", $name))),*
        ];
    };
}

check_signatures!(
    "action_sent" { "after": "positive integer" },
    "active_continue" {},
    "all" { "of": "predicate list" },
    "amount_prompt" {},
    "arrived" { "x": "integer", "z": "integer", "level": "integer" },
    "arrived_near" { "x": "integer", "z": "integer", "level": "integer", "radius": "integer" },
    "arrived_ring" { "x": "integer", "z": "integer", "level": "integer", "min": "integer", "max": "integer" },
    "bank_closed" {},
    "bank_exact" { "expected": "expected item totals" },
    "bank_item" { "name": "string", "count": "integer" },
    "bank_item_id" { "id": "integer", "count": "integer" },
    "bank_item_id_any" { "ids": "integer list", "count": "integer" },
    "bank_item_id_at_most" { "id": "integer", "count": "integer" },
    "bank_item_at_most" { "name": "string", "count": "integer" },
    "bank_open" {},
    "chat" { "needle": "string" },
    "chat_closed" {},
    "chat_choice" {},
    "clue_replaced" { "seeded": "integer", "since": "positive integer" },
    "clue_protect_window" { "seeded": "integer" },
    "container_unchanged" { "where": "container", "since": "positive integer" },
    "count" { "record": "record matcher", "from": "positive integer", "to": "to mark or label", "amount": "amount", "exact": "exactness" },
    "dialogue" { "open": "boolean", "root": "integer", "continue": "integer", "lines": "dialogue lines", "options": "dialogue options" },
    "equipment_id" { "id": "integer" },
    "essence_mine" {},
    "fresh_chat" { "needle": "string", "since": "positive integer" },
    "fresh_chat_since_previous_step" { "needle": "string", "since": "positive integer" },
    "fresh_quest_completion" { "chat_needle": "string", "main_root_component_id": "integer", "component_id": "integer", "text": "string", "since": "positive integer" },
    "fresh_stat_xp_gain" { "id": "integer", "min": "integer", "since": "positive integer" },
    "fresh_tutorial" { "since": "positive integer" },
    "ground_item_ids_absent_near" { "ids": "integer list", "x": "integer", "z": "integer", "level": "integer", "radius": "integer" },
    "hint" { "kind": "hint kind", "at": "tile", "radius": "integer" },
    "ingame_scene_2" {},
    "input_consumed" { "id": "integer", "count": "integer", "since": "positive integer" },
    "item" { "name": "string", "count": "integer" },
    "item_at_most" { "name": "string", "count": "integer" },
    "item_count" { "item": "integer", "where": "container", "amount": "amount" },
    "item_delta" { "item": "integer", "where": "container", "since": "positive integer", "amount": "amount" },
    "item_id" { "id": "integer", "count": "integer" },
    "item_id_any" { "ids": "integer list", "count": "integer" },
    "item_id_at_most" { "id": "integer", "count": "integer" },
    "item_id_in_full_pack" { "id": "integer", "count": "integer" },
    "loc_action_near" { "id": "integer", "x": "integer", "z": "integer", "level": "integer", "radius": "integer", "action": "string", "present": "boolean" },
    "loc_id_near" { "id": "integer", "x": "integer", "z": "integer", "level": "integer", "radius": "integer" },
    "loc" { "at": "tile", "name": "string", "form": "integer", "options": "string list", "first_option": "string", "no_option": "string" },
    "logged_out" {},
    "login_packets" { "family": "packet family", "form": "packet form", "amount": "amount" },
    "local_targeting_npc_name" { "name": "string" },
    "npc" { "type": "integer", "near": "tile", "radius": "integer", "index": "integer", "form": "integer", "options": "string list", "no_option": "string", "sequence": "integer", "frame": "integer", "present": "boolean" },
    "npc_at" { "type": "integer", "x": "integer", "z": "integer" },
    "npc_ids_absent_near" { "ids": "integer list", "radius": "integer" },
    "npc_name_near" { "name": "string", "x": "integer", "z": "integer", "level": "integer", "radius": "integer" },
    "npc_name_targeting_local" { "name": "string" },
    "npc_name_unengaged_in_sight" { "name": "string", "radius": "integer" },
    "npc_near" { "type": "integer", "radius": "integer" },
    "no_active_continue" {},
    "order" { "first": "string", "then": "string", "gap": "amount" },
    "product_delta" { "id": "integer", "count": "integer", "since": "positive integer" },
    "quest_done" { "name": "string" },
    "received" { "after": "positive integer", "stats": "stat matchers", "inventories": "inventory matchers", "messages": "message matchers", "vars": "var matchers", "exact": "exactness" },
    "render_view_ready" { "x": "integer", "z": "integer", "level": "integer", "orbit_yaw": "integer", "orbit_pitch": "integer" },
    "side_tab_available" { "index": "integer" },
    "script_idle" {},
    "script_receipt" { "prefix": "string" },
    "script_running" {},
    "signal" { "name": "signal name" },
    "silent" { "after": "positive integer", "ticks": "positive integer", "stats": "stat matchers", "inventories": "inventory matchers", "vars": "var matchers", "messages": "message filter", "dialogue": "boolean", "exact": "exactness" },
    "standing_bank_closed" { "stand": "tile" },
    "stat" { "id": "integer", "min": "integer" },
    "stat_at_most" { "id": "integer", "max": "integer" },
    "stat_value" { "skill": "skill", "base": "amount", "effective": "amount", "xp": "amount" },
    "stat_xp_gain" { "id": "integer", "min": "integer", "since": "positive integer" },
    "tile" { "x": "integer", "z": "integer", "level": "integer", "radius": "integer" },
    "tutorial_closed" {},
    "var" { "id": "integer", "value": "amount" },
    "varp" { "id": "integer", "min": "integer" },
    "varp_exact" { "id": "integer", "value": "integer" },
    "visited_within" { "min_x": "integer", "max_x": "integer", "min_z": "integer", "max_z": "integer", "level": "integer" },
    "xp_delta" { "id": "integer", "min": "integer", "since": "positive integer" },
    "xp_since" { "skill": "skill", "since": "positive integer", "amount": "amount" },
    "world_speed_changed" { "ms": "integer" },
);

impl FileCheck {
    pub fn from_literal(value: Value) -> Result<Self, String> {
        if let Value::String(name) = value {
            return Self::parse(&name, Map::new());
        }
        let mut args = object(value, "predicate")?;
        let name = take_string(&mut args, "is")?;
        Self::parse(&name, args)
    }

    pub fn parse(name: &str, mut args: Map<String, Value>) -> Result<Self, String> {
        if !FILE_CHECK_SIGNATURES
            .iter()
            .any(|signature| signature.name == name)
        {
            return Err(format!(
                "scenario predicate {name:?}: unavailable or forbidden for a File run"
            ));
        }
        use Proof as P;
        let check = match name {
            "action_sent" => Self::ActionSent(handle(&mut args, "after")?),
            "all" => {
                let Value::Array(rows) = args.remove("of").ok_or("missing of")? else {
                    return Err("all.of: expected list".into());
                };
                if rows.is_empty() {
                    return Err("all.of: expected a nonempty list".into());
                }
                Self::All(
                    rows.into_iter()
                        .map(Self::from_literal)
                        .collect::<Result<_, _>>()?,
                )
            }
            "bank_closed" => bind_named(name, NamedPredicateParams::None)?,
            "standing_bank_closed" => {
                let mut stand = object(args.remove("stand").ok_or("missing stand")?, "stand")?;
                let (x, z, level) = coordinates(&mut stand)?;
                empty(&stand)?;
                bind_named(
                    name,
                    NamedPredicateParams::StandingBankClosed(StandingBankClosedParams {
                        stand: WorldTile { x, z, level },
                    }),
                )?
            }
            "bank_exact" => {
                let items = match args.remove("expected").ok_or("missing expected")? {
                    Value::Array(items) => items,
                    Value::Object(items) if items.is_empty() => Vec::new(),
                    _ => return Err("bank_exact.expected: expected a list of item totals".into()),
                };
                let mut expected = BTreeMap::new();
                for item in items {
                    let mut item = object(item, "bank_exact.expected")?;
                    let id = take_int(&mut item, "id", 0, i32::MAX)?;
                    let count = take_int(&mut item, "count", 1, i32::MAX)?;
                    empty(&item)?;
                    if expected.insert(id, count).is_some() {
                        return Err(format!("bank_exact.expected: duplicate item {id}"));
                    }
                }
                bind_named(
                    name,
                    NamedPredicateParams::BankExact(BankExactParams { expected }),
                )?
            }
            "bank_open" => Self::Native(P::BankOpen),
            "chat_closed" => Self::Native(P::ChatClosed),
            "no_active_continue" => Self::Native(P::NoActiveContinue),
            "chat_choice" => Self::Native(P::ChatChoice),
            "active_continue" => Self::Native(P::ActiveContinue),
            "tutorial_closed" => Self::Native(P::TutorialClosed),
            "essence_mine" => Self::Native(P::EssenceMine),
            "side_tab_available" => Self::Native(P::SideTabAvailable {
                index: take_int(&mut args, "index", 0, i32::MAX)?,
            }),
            "amount_prompt" => Self::Native(P::AmountPrompt),
            "ingame_scene_2" => Self::Native(P::IngameScene2),
            "equipment_id" => Self::Native(P::EquipmentId {
                id: take_int(&mut args, "id", 0, i32::MAX)?,
            }),
            "item_id"
            | "item_id_at_most"
            | "item_id_in_full_pack"
            | "bank_item_id"
            | "bank_item_id_at_most" => {
                let id = take_int(&mut args, "id", 0, i32::MAX)?;
                let count = take_int(&mut args, "count", 0, i32::MAX)?;
                Self::Native(match name {
                    "item_id" => P::ItemId { id, count },
                    "item_id_at_most" => P::ItemIdAtMost { id, count },
                    "item_id_in_full_pack" => P::ItemIdInFullPack { id, count },
                    "bank_item_id" => P::BankItemId { id, count },
                    _ => P::BankItemIdAtMost { id, count },
                })
            }
            "arrived" | "arrived_near" => {
                let (x, z, level) = coordinates(&mut args)?;
                Self::Native(if name == "arrived" {
                    P::Arrived { x, z, level }
                } else {
                    P::ArrivedNear {
                        x,
                        z,
                        level,
                        radius: take_int(&mut args, "radius", 0, 16383)?,
                    }
                })
            }
            "arrived_ring" => {
                let (x, z, level) = coordinates(&mut args)?;
                let min = take_int(&mut args, "min", 0, 16383)?;
                let max = take_int(&mut args, "max", min, 16383)?;
                Self::Native(P::ArrivedRing {
                    x,
                    z,
                    level,
                    min,
                    max,
                })
            }
            "loc_id_near" => {
                let (x, z, level) = coordinates(&mut args)?;
                Self::Native(P::LocIdNear {
                    id: take_int(&mut args, "id", 0, i32::MAX)?,
                    x,
                    z,
                    level,
                    radius: take_int(&mut args, "radius", 0, 16383)?,
                })
            }
            "npc_at" => Self::Native(P::NpcAt {
                r#type: take_int(&mut args, "type", 0, i32::MAX)? as usize,
                x: take_int(&mut args, "x", 0, 16383)?,
                z: take_int(&mut args, "z", 0, 16383)?,
            }),
            "npc" => {
                let type_id = take_int(&mut args, "type", 0, i32::MAX)? as usize;
                let near = args
                    .remove("near")
                    .map(|value| tile_value(value, "near"))
                    .transpose()?;
                let radius = optional_int(&mut args, "radius", 0, 16383)?;
                let index =
                    optional_int(&mut args, "index", 0, i32::MAX)?.map(|value| value as usize);
                let form =
                    optional_int(&mut args, "form", 0, i32::MAX)?.map(|value| value as usize);
                let options = take_string_list(&mut args, "options")?;
                let no_option = optional_text(&mut args, "no_option")?;
                let sequence = optional_int(&mut args, "sequence", -1, i32::MAX)?;
                let frame = optional_int(&mut args, "frame", -1, i32::MAX)?;
                let present = args
                    .remove("present")
                    .map(|value| bool_value(&value, "present"))
                    .transpose()?
                    .unwrap_or(true);
                Self::Snapshot(SnapshotProof::Npc {
                    type_id,
                    near,
                    radius,
                    index,
                    form,
                    options,
                    no_option,
                    sequence,
                    frame,
                    present,
                })
            }
            "npc_near" => Self::Native(P::NpcNear {
                r#type: take_int(&mut args, "type", 0, i32::MAX)? as usize,
                radius: take_int(&mut args, "radius", 0, 16383)?,
            }),
            "render_view_ready" => {
                let (x, z, level) = coordinates(&mut args)?;
                Self::Native(P::RenderViewReady {
                    x,
                    z,
                    level,
                    orbit_yaw: take_int(&mut args, "orbit_yaw", 0, 2047)?,
                    orbit_pitch: take_int(&mut args, "orbit_pitch", 0, 2047)?,
                })
            }
            "tile" if args.contains_key("unchanged_since") => {
                Self::TileUnchanged(handle(&mut args, "unchanged_since")?)
            }
            "tile" => {
                let radius = optional_int(&mut args, "radius", 0, 16383)?.unwrap_or(0);
                let (x, z, level) = if let Some(at) = args.remove("at") {
                    let mut at = object(at, "at")?;
                    let tile = coordinates(&mut at)?;
                    empty(&at)?;
                    tile
                } else {
                    coordinates(&mut args)?
                };
                Self::ExactTile {
                    x,
                    z,
                    level,
                    radius,
                }
            }
            "stat" => Self::Native(P::Stat {
                id: take_int(&mut args, "id", 0, 24)?,
                min: take_int(&mut args, "min", 0, i32::MAX)?,
            }),
            "stat_at_most" => Self::Native(P::StatAtMost {
                id: take_int(&mut args, "id", 0, 24)?,
                max: take_int(&mut args, "max", 0, i32::MAX)?,
            }),
            "stat_value" => {
                let skill = skill(&mut args)?;
                let base = args.remove("base").map(constraint).transpose()?;
                let effective = args.remove("effective").map(constraint).transpose()?;
                let xp = args.remove("xp").map(constraint).transpose()?;
                if base.is_none() && effective.is_none() && xp.is_none() {
                    return Err("stat_value: no constraint".into());
                }
                Self::Stat {
                    skill,
                    base,
                    effective,
                    xp,
                }
            }
            "xp_since" => Self::XpSince {
                skill: skill(&mut args)?,
                mark: handle(&mut args, "since")?,
                amount: amount_field_or_flat(&mut args)?,
            },
            "item" | "item_at_most" | "bank_item" | "bank_item_at_most" => {
                let count = i64::from(take_int(&mut args, "count", 0, i32::MAX)?);
                Self::Item {
                    item: Item::Alias(take_string(&mut args, "name")?),
                    container: if name.starts_with("bank_") {
                        Container::Bank
                    } else {
                        Container::Inventory
                    },
                    since: None,
                    amount: if name.ends_with("_at_most") {
                        Amount { min: 0, max: count }
                    } else {
                        Amount {
                            min: count,
                            max: i64::MAX,
                        }
                    },
                }
            }
            "item_count" | "item_delta" => {
                let item = args.remove("item").ok_or("missing item")?;
                let item = if let Value::String(alias) = item {
                    Item::Alias(alias)
                } else {
                    Item::Id(integer(&item, 0, i32::MAX, "item")?)
                };
                let container = Container::parse(
                    args.remove("where")
                        .unwrap_or_else(|| Value::from("inventory")),
                )?;
                let since = if name == "item_delta" {
                    Some(handle(&mut args, "since")?)
                } else {
                    None
                };
                Self::Item {
                    item,
                    container,
                    since,
                    amount: amount_field_or_flat(&mut args)?,
                }
            }
            "container_unchanged" => Self::ContainerUnchanged {
                container: Container::parse(args.remove("where").ok_or("missing where")?)?,
                since: handle(&mut args, "since")?,
            },
            "item_id_any" | "bank_item_id_any" => {
                let ids = take_i32_list(&mut args, "ids", 0, i32::MAX)?;
                let count = take_int(&mut args, "count", 0, i32::MAX)?;
                Self::Snapshot(if name == "item_id_any" {
                    SnapshotProof::ItemIdAny { ids, count }
                } else {
                    SnapshotProof::BankItemIdAny { ids, count }
                })
            }
            "quest_done" => Self::Snapshot(SnapshotProof::QuestDone {
                name: take_string(&mut args, "name")?,
            }),
            "npc_ids_absent_near" => Self::Snapshot(SnapshotProof::NpcIdsAbsentNear {
                ids: take_usize_list(&mut args, "ids", 0, i32::MAX)?,
                radius: take_int(&mut args, "radius", 0, 16383)?,
            }),
            "ground_item_ids_absent_near" => {
                let ids = take_i32_list(&mut args, "ids", 0, i32::MAX)?;
                let (x, z, level) = coordinates(&mut args)?;
                Self::Snapshot(SnapshotProof::GroundItemIdsAbsentNear {
                    ids,
                    x,
                    z,
                    level,
                    radius: take_int(&mut args, "radius", 0, 16383)?,
                })
            }
            "npc_name_near" => {
                let name = take_string(&mut args, "name")?;
                let (x, z, level) = coordinates(&mut args)?;
                Self::Snapshot(SnapshotProof::NpcNameNear {
                    name,
                    x,
                    z,
                    level,
                    radius: take_int(&mut args, "radius", 0, 16383)?,
                })
            }
            "npc_name_targeting_local" => Self::Snapshot(SnapshotProof::NpcNameTargetingLocal {
                name: take_string(&mut args, "name")?,
            }),
            "local_targeting_npc_name" => Self::Snapshot(SnapshotProof::LocalTargetingNpcName {
                name: take_string(&mut args, "name")?,
            }),
            "npc_name_unengaged_in_sight" => {
                Self::Snapshot(SnapshotProof::NpcNameUnengagedInSight {
                    name: take_string(&mut args, "name")?,
                    radius: take_int(&mut args, "radius", 0, 16383)?,
                })
            }
            "loc_action_near" => {
                let (x, z, level) = coordinates(&mut args)?;
                Self::Snapshot(SnapshotProof::LocActionNear {
                    id: take_int(&mut args, "id", 0, i32::MAX)?,
                    x,
                    z,
                    level,
                    radius: take_int(&mut args, "radius", 0, 16383)?,
                    action: take_string(&mut args, "action")?,
                    present: take_bool(&mut args, "present")?,
                })
            }
            "loc" => Self::Snapshot(SnapshotProof::Loc {
                at: take_tile(&mut args, "at")?,
                name: optional_text(&mut args, "name")?,
                form: optional_int(&mut args, "form", 0, i32::MAX)?,
                options: take_string_list(&mut args, "options")?,
                first_option: optional_text(&mut args, "first_option")?,
                no_option: optional_text(&mut args, "no_option")?,
            }),
            "visited_within" => {
                let min_x = take_int(&mut args, "min_x", 0, 16383)?;
                let max_x = take_int(&mut args, "max_x", min_x, 16383)?;
                let min_z = take_int(&mut args, "min_z", 0, 16383)?;
                let max_z = take_int(&mut args, "max_z", min_z, 16383)?;
                Self::Snapshot(SnapshotProof::VisitedWithin {
                    min_x,
                    max_x,
                    min_z,
                    max_z,
                    level: take_int(&mut args, "level", 0, 3)?,
                })
            }
            "chat" => Self::Snapshot(SnapshotProof::Chat {
                needle: take_string(&mut args, "needle")?,
            }),
            "fresh_tutorial" => Self::Snapshot(SnapshotProof::FreshTutorial {
                since: handle(&mut args, "since")?,
            }),
            "fresh_chat" | "fresh_chat_since_previous_step" => {
                let needle = take_string(&mut args, "needle")?;
                let since = handle(&mut args, "since")?;
                Self::Snapshot(if name == "fresh_chat" {
                    SnapshotProof::FreshChat { needle, since }
                } else {
                    SnapshotProof::FreshChatSincePreviousStep { needle, since }
                })
            }
            "fresh_quest_completion" => Self::Snapshot(SnapshotProof::FreshQuestCompletion {
                chat_needle: take_string(&mut args, "chat_needle")?,
                main_root_component_id: take_int(&mut args, "main_root_component_id", 0, i32::MAX)?,
                component_id: take_int(&mut args, "component_id", 0, i32::MAX)?,
                text: take_string(&mut args, "text")?,
                since: handle(&mut args, "since")?,
            }),
            "stat_xp_gain" | "fresh_stat_xp_gain" | "xp_delta" => {
                let id = take_int(&mut args, "id", 0, 24)?;
                let min = take_int(&mut args, "min", 0, i32::MAX)?;
                let since = handle(&mut args, "since")?;
                Self::Snapshot(match name {
                    "stat_xp_gain" => SnapshotProof::StatXpGain { id, min, since },
                    "fresh_stat_xp_gain" => SnapshotProof::FreshStatXpGain { id, min, since },
                    _ => SnapshotProof::XpDelta { id, min, since },
                })
            }
            "product_delta" | "input_consumed" => {
                let id = take_int(&mut args, "id", 0, i32::MAX)?;
                let count = take_int(&mut args, "count", 0, i32::MAX)?;
                let since = handle(&mut args, "since")?;
                Self::Snapshot(if name == "product_delta" {
                    SnapshotProof::ProductDelta { id, count, since }
                } else {
                    SnapshotProof::InputConsumed { id, count, since }
                })
            }
            "clue_replaced" => Self::Snapshot(SnapshotProof::ClueReplaced {
                seeded: take_int(&mut args, "seeded", 0, i32::MAX)?,
                since: handle(&mut args, "since")?,
            }),
            "received" => {
                let after = handle(&mut args, "after")?;
                let exact = optional_exactness(&mut args)?.unwrap_or(Exactness::Value);
                let stats = take_matcher_list(&mut args, "stats", parse_stat_matcher)?;
                let inventories =
                    take_matcher_list(&mut args, "inventories", parse_inventory_matcher)?;
                let messages = take_matcher_list(&mut args, "messages", parse_message_matcher)?;
                let vars = take_matcher_list(&mut args, "vars", parse_var_matcher)?;
                if stats.is_empty()
                    && inventories.is_empty()
                    && messages.is_empty()
                    && vars.is_empty()
                {
                    return Err("received: expected at least one record matcher".into());
                }
                Self::Received {
                    after,
                    exact,
                    stats,
                    inventories,
                    messages,
                    vars,
                }
            }
            "silent" => {
                let after = handle(&mut args, "after")?;
                let ticks = take_positive_u64(&mut args, "ticks")?;
                let exact = optional_exactness(&mut args)?.unwrap_or(Exactness::Value);
                let stats = take_matcher_list(&mut args, "stats", parse_stat_matcher)?;
                let inventories =
                    take_matcher_list(&mut args, "inventories", parse_inventory_matcher)?;
                let vars = take_matcher_list(&mut args, "vars", parse_var_matcher)?;
                let messages = args
                    .remove("messages")
                    .map(parse_silent_message_filter)
                    .transpose()?;
                let dialogue = args
                    .remove("dialogue")
                    .map(|value| bool_value(&value, "dialogue"))
                    .transpose()?
                    .unwrap_or(false);
                if stats.is_empty()
                    && inventories.is_empty()
                    && vars.is_empty()
                    && messages.is_none()
                    && !dialogue
                {
                    return Err("silent: expected at least one observation selector".into());
                }
                Self::Silent {
                    after,
                    ticks,
                    exact,
                    stats,
                    inventories,
                    vars,
                    messages,
                    dialogue,
                }
            }
            "count" => Self::Count {
                record: parse_record_matcher(args.remove("record").ok_or("missing record")?)?,
                from: handle(&mut args, "from")?,
                to: args.remove("to").map(parse_count_end).transpose()?,
                amount: take_amount(&mut args, "amount")?,
                exact: take_exactness(&mut args)?,
            },
            "order" => Self::Order {
                first: take_string(&mut args, "first")?,
                then: take_string(&mut args, "then")?,
                gap: args.remove("gap").map(parse_amount_value).transpose()?,
            },
            "var" => Self::Var {
                id: take_int(&mut args, "id", 0, i32::MAX)?,
                value: constraint(args.remove("value").ok_or("missing value")?)?,
            },
            "dialogue" => {
                let open = args
                    .remove("open")
                    .map(|value| bool_value(&value, "open"))
                    .transpose()?;
                let root = optional_int(&mut args, "root", -1, i32::MAX)?;
                let continue_component = optional_int(&mut args, "continue", -1, i32::MAX)?;
                let lines = optional_string_list(&mut args, "lines")?;
                let options = optional_string_list(&mut args, "options")?;
                if open.is_none()
                    && root.is_none()
                    && continue_component.is_none()
                    && lines.is_none()
                    && options.is_none()
                {
                    return Err("dialogue: expected at least one selector".into());
                }
                Self::Dialogue {
                    open,
                    root,
                    continue_component,
                    lines,
                    options,
                }
            }
            "hint" => {
                let kind = take_string(&mut args, "kind")?;
                if kind != "tile" {
                    return Err(
                        "hint: non-tile hint kinds are not observable in a File snapshot".into(),
                    );
                }
                let mut at = object(args.remove("at").ok_or("missing at")?, "at")?;
                let (x, z, level) = coordinates(&mut at)?;
                empty(&at)?;
                Self::Hint {
                    kind,
                    x,
                    z,
                    level,
                    radius: optional_int(&mut args, "radius", 0, 16383)?.unwrap_or(0),
                }
            }
            "signal" => Self::Signal(take_string(&mut args, "name")?),
            "login_packets" => {
                return Err(
                    "login_packets: packet family/form history is unavailable in a File check"
                        .into(),
                )
            }
            "varp" | "varp_exact" => {
                return Err(format!(
                    "{name}: no per-id receive witness in a File snapshot"
                ))
            }
            "logged_out" => {
                return Err("logged_out is runner-owned, not a File snapshot check".into())
            }
            "script_receipt"
            | "script_running"
            | "script_idle"
            | "clue_protect_window"
            | "world_speed_changed" => {
                return Err(format!(
                    "{name}: table-only state is unavailable in a File check"
                ))
            }
            _ => {
                return Err(format!(
                    "scenario predicate {name:?}: unavailable or forbidden for a File run"
                ))
            }
        };
        empty(&args)?;
        Ok(check)
    }

    pub fn terminal(&self) -> bool {
        match self {
            Self::All(rows) => rows.iter().all(Self::terminal),
            Self::ActionSent(_)
            | Self::XpSince { .. }
            | Self::TileUnchanged(_)
            | Self::ContainerUnchanged { .. }
            | Self::Item { since: Some(_), .. }
            | Self::Received { .. }
            | Self::Silent { .. }
            | Self::Count { .. }
            | Self::Order { .. }
            | Self::Var { .. } => false,
            _ => true,
        }
    }

    pub fn is_gameplay(&self) -> bool {
        match self {
            Self::ActionSent(_)
            | Self::Signal(_)
            | Self::Native(
                Proof::IngameScene2
                | Proof::TutorialClosed
                | Proof::RenderViewReady { .. }
                | Proof::LoggedOut,
            ) => false,
            Self::All(rows) => rows.iter().any(Self::is_gameplay),
            _ => true,
        }
    }

    pub fn admit(&self, journal: &FileJournal) -> Result<(), String> {
        match self {
            Self::All(rows) => {
                for row in rows {
                    row.admit(journal)?;
                }
            }
            Self::Received {
                exact,
                stats,
                inventories,
                messages,
                vars,
                ..
            } => {
                if *exact == Exactness::Packet {
                    if !messages.is_empty() {
                        return Err("received: message packet provider is unavailable".into());
                    }
                    require_packet_provider(journal, RecordKind::Stat, !stats.is_empty())?;
                    require_packet_provider(
                        journal,
                        RecordKind::Inventory,
                        !inventories.is_empty(),
                    )?;
                    require_packet_provider(journal, RecordKind::Varp, !vars.is_empty())?;
                } else if !vars.is_empty() {
                    return Err("received: var values are not observed in value mode".into());
                }
            }
            Self::Silent {
                exact,
                stats,
                inventories,
                vars,
                messages,
                dialogue,
                ..
            } => {
                if *exact == Exactness::Packet {
                    if messages.is_some() || *dialogue {
                        return Err("silent: messages and dialogue have no packet provider".into());
                    }
                    require_packet_provider(journal, RecordKind::Stat, !stats.is_empty())?;
                    require_packet_provider(
                        journal,
                        RecordKind::Inventory,
                        !inventories.is_empty(),
                    )?;
                    require_packet_provider(journal, RecordKind::Varp, !vars.is_empty())?;
                } else if !vars.is_empty() {
                    return Err("silent: var values are not observed in value mode".into());
                }
            }
            Self::Count { record, exact, .. } => {
                if *exact == Exactness::Packet {
                    if record.kind() == RecordKind::Message {
                        return Err("count: message packet provider is unavailable".into());
                    }
                    require_packet_provider(journal, record.kind(), true)?;
                } else if record.kind() == RecordKind::Varp {
                    return Err("count: var values are not observed in value mode".into());
                }
            }
            Self::Var { .. } => require_packet_provider(journal, RecordKind::Varp, true)?,
            _ => {}
        }
        Ok(())
    }

    pub fn limitation(&self) -> Option<&'static str> {
        match self {
            Self::All(rows) => rows.iter().find_map(Self::limitation),
            Self::Received {
                exact: Exactness::Value,
                ..
            }
            | Self::Silent {
                exact: Exactness::Value,
                ..
            }
            | Self::Count {
                exact: Exactness::Value,
                ..
            } => Some("exact=value compares sampled endpoints and can miss intermediate states"),
            _ => None,
        }
    }

    pub fn check(
        &self,
        snapshot: &GameSnapshot,
        selected: &SelectedGameData,
        journal: &FileJournal,
        session: u64,
    ) -> Result<bool, String> {
        Ok(self
            .check_matched(
                snapshot,
                selected,
                journal,
                session,
                &CheckContext::default(),
            )?
            .is_some())
    }

    pub fn check_matched(
        &self,
        snapshot: &GameSnapshot,
        selected: &SelectedGameData,
        journal: &FileJournal,
        session: u64,
        context: &CheckContext,
    ) -> Result<Option<Vec<u64>>, String> {
        match self {
            Self::All(rows) => {
                let mut serials = Vec::new();
                for row in rows {
                    let Some(matched) =
                        row.check_matched(snapshot, selected, journal, session, context)?
                    else {
                        return Ok(None);
                    };
                    serials.extend(matched);
                }
                serials.sort_unstable();
                serials.dedup();
                Ok(Some(serials))
            }
            Self::Received {
                after,
                exact,
                stats,
                inventories,
                messages,
                vars,
            } => received_check(
                *after,
                *exact,
                stats,
                inventories,
                messages,
                vars,
                snapshot,
                journal,
                session,
            ),
            Self::Silent {
                after,
                ticks,
                exact,
                stats,
                inventories,
                vars,
                messages,
                dialogue,
            } => silent_check(
                *after,
                *ticks,
                *exact,
                stats,
                inventories,
                vars,
                messages.as_ref(),
                *dialogue,
                snapshot,
                journal,
                session,
            ),
            Self::Count {
                record,
                from,
                to,
                amount,
                exact,
            } => count_check(
                record,
                *from,
                to.as_ref(),
                amount,
                *exact,
                snapshot,
                journal,
                session,
                context,
            ),
            Self::Order { first, then, gap } => {
                order_check(first, then, gap.as_ref(), journal, session, context)
            }
            Self::Var { id, value } => var_check(*id, value, journal, session),
            Self::Dialogue {
                open,
                root,
                continue_component,
                lines,
                options,
            } => Ok(dialogue_check(
                *open,
                *root,
                *continue_component,
                lines.as_deref(),
                options.as_deref(),
                snapshot,
            )
            .then(Vec::new)),
            Self::Hint {
                kind: _,
                x,
                z,
                level,
                radius,
            } => Ok(hint_check(*x, *z, *level, *radius, snapshot).then(Vec::new)),
            Self::Signal(name) => Ok(context.has_signal(name).then(Vec::new)),
            Self::Snapshot(proof) => Ok(snapshot_proof_check(
                proof, snapshot, selected, journal, session,
            )?
            .then(Vec::new)),
            _ => Ok(self
                .check_snapshot(snapshot, selected, journal, session)?
                .then(Vec::new)),
        }
    }

    fn check_snapshot(
        &self,
        snapshot: &GameSnapshot,
        selected: &SelectedGameData,
        journal: &FileJournal,
        session: u64,
    ) -> Result<bool, String> {
        let stat = |index| snapshot.stats().iter().find(|stat| stat.index == index);
        let mark = |id| journal.get_mark(id, session);
        let result = match self {
            Self::Native(proof) => proof.check(snapshot, None),
            Self::Named(gate) => gate.check(PredicateContext::snapshot_only(snapshot)),
            Self::All(rows) => {
                let mut result = true;
                for row in rows {
                    result &= row.check(snapshot, selected, journal, session)?;
                }
                result
            }
            Self::ActionSent(id) => journal.action_sent(*id, session)?,
            Self::ExactTile {
                x,
                z,
                level,
                radius,
            } => snapshot.tile().is_some_and(|(a, b, c)| {
                c == *level && (a - x).abs().max((b - z).abs()) <= *radius
            }),
            Self::TileUnchanged(id) => mark(*id)?
                .tile
                .is_some_and(|tile| snapshot.tile() == Some(tile)),
            Self::Stat {
                skill,
                base,
                effective,
                xp,
            } => stat(*skill).is_some_and(|stat| {
                base.as_ref()
                    .is_none_or(|amount| amount.matches(i64::from(stat.base)))
                    && effective
                        .as_ref()
                        .is_none_or(|amount| amount.matches(i64::from(stat.effective)))
                    && xp
                        .as_ref()
                        .is_none_or(|amount| amount.matches(i64::from(stat.xp)))
            }),
            Self::XpSince {
                skill,
                mark: id,
                amount,
            } => {
                let before = mark(*id)?
                    .xp
                    .iter()
                    .find(|(index, _)| index == skill)
                    .ok_or("mark has no observed skill")?
                    .1;
                stat(*skill)
                    .is_some_and(|stat| amount.matches(i64::from(stat.xp) - i64::from(before)))
            }
            Self::Item {
                item,
                container,
                since,
                amount,
            } => {
                let id = match item {
                    Item::Id(id) => *id,
                    Item::Alias(alias) => {
                        selected
                            .item_by_alias(alias)
                            .ok_or_else(|| format!("unknown item {alias:?}"))?
                            .id
                    }
                };
                let before = if let Some(handle) = since {
                    mark_slots(mark(*handle)?, *container)?
                        .iter()
                        .filter(|(_, object, _)| *object == id)
                        .map(|(_, _, count)| i64::from(*count))
                        .sum()
                } else {
                    0
                };
                container
                    .total(snapshot, id)
                    .is_some_and(|count| amount.matches(count - before))
            }
            Self::ContainerUnchanged { container, since } => {
                let before = mark_slots(mark(*since)?, *container)?;
                container.slots(snapshot).is_some_and(|now| now == *before)
            }
            _ => false,
        };
        Ok(result)
    }

    /// A walk root needs its own fresh observed arrival, not an unrelated true check.
    pub fn checks_tile(&self, target: (i32, i32, i32), max_radius: i32) -> bool {
        match self {
            Self::Native(Proof::Arrived { x, z, level }) => (*x, *z, *level) == target,
            Self::ExactTile {
                x,
                z,
                level,
                radius,
            }
            | Self::Native(Proof::ArrivedNear {
                x,
                z,
                level,
                radius,
            }) => (*x, *z, *level) == target && *radius <= max_radius,
            Self::All(rows) => rows.iter().any(|row| row.checks_tile(target, max_radius)),
            _ => false,
        }
    }
}

fn dialogue_check(
    open: Option<bool>,
    root: Option<i32>,
    continue_component: Option<i32>,
    lines: Option<&[String]>,
    options: Option<&[String]>,
    snapshot: &GameSnapshot,
) -> bool {
    let root_id = snapshot.modals().chat;
    let continue_id = snapshot.chat_continue_component_id();
    let observed_options = snapshot.chat_options();
    let observed_lines = snapshot.chat_modal_texts();
    let is_open = root_id != -1
        || continue_id != -1
        || !observed_options.is_empty()
        || !observed_lines.is_empty();

    open.is_none_or(|expected| expected == is_open)
        && root.is_none_or(|expected| expected == root_id)
        && continue_component.is_none_or(|expected| expected == continue_id)
        && lines.is_none_or(|expected| expected == observed_lines)
        && options.is_none_or(|expected| {
            expected.len() == observed_options.len()
                && expected
                    .iter()
                    .zip(observed_options)
                    .all(|(expected, observed)| expected == &observed.text)
        })
}

fn hint_matches(
    hint: Option<&api::snapshot::HintTileView>,
    x: i32,
    z: i32,
    level: i32,
    radius: i32,
) -> bool {
    hint.is_some_and(|hint| {
        hint.level == level && (hint.x - x).abs().max((hint.z - z).abs()) <= radius
    })
}

fn hint_check(x: i32, z: i32, level: i32, radius: i32, snapshot: &GameSnapshot) -> bool {
    hint_matches(snapshot.hint_tile(), x, z, level, radius)
}

fn action_matches(actual: &str, expected: &str) -> bool {
    let actual = actual.trim();
    let expected = expected.trim();
    actual
        .get(..expected.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(expected))
}

fn has_action<'a>(mut actions: impl Iterator<Item = &'a str>, expected: &str) -> bool {
    actions.any(|actual| action_matches(actual, expected))
}

fn snapshot_proof_check(
    proof: &SnapshotProof,
    snapshot: &GameSnapshot,
    selected: &SelectedGameData,
    journal: &FileJournal,
    session: u64,
) -> Result<bool, String> {
    let mark = |since| journal.get_mark(since, session);
    let current_xp = |id| {
        snapshot
            .stats()
            .iter()
            .find(|stat| stat.index == id)
            .map(|stat| stat.xp)
    };
    let xp_since = |id, since| -> Result<Option<i64>, String> {
        let baseline = mark(since)?
            .xp
            .iter()
            .find(|(skill, _)| *skill == id)
            .map(|(_, xp)| *xp)
            .ok_or("mark has no observed skill")?;
        Ok(current_xp(id).map(|current| i64::from(current) - i64::from(baseline)))
    };

    let result = match proof {
        SnapshotProof::ItemIdAny { ids, count } => {
            let Some(items) = Container::Inventory.items(snapshot) else {
                return Ok(false);
            };
            items
                .iter()
                .filter(|item| ids.contains(&item.def.id))
                .map(|item| i64::from(item.count))
                .sum::<i64>()
                >= i64::from(*count)
        }
        SnapshotProof::BankItemIdAny { ids, count } => {
            let Some(items) = Container::Bank.items(snapshot) else {
                return Ok(false);
            };
            items
                .iter()
                .filter(|item| ids.contains(&item.def.id))
                .map(|item| i64::from(item.count))
                .sum::<i64>()
                >= i64::from(*count)
        }
        SnapshotProof::QuestDone { name } => nav_planner::WorldState::from_snapshot(snapshot)
            .quests
            .contains(name.as_str()),
        SnapshotProof::NpcIdsAbsentNear { ids, radius } => {
            let Some((x, z, level)) = snapshot.tile() else {
                return Ok(false);
            };
            let here = WorldTile { x, z, level };
            !snapshot.npcs().iter().any(|npc| {
                npc.r#type.is_some_and(|kind| ids.contains(&kind))
                    && api::query::chebyshev_to(here, npc.network) <= *radius
            })
        }
        SnapshotProof::GroundItemIdsAbsentNear {
            ids,
            x,
            z,
            level,
            radius,
        } => {
            snapshot.ingame()
                && snapshot.scene_state() == 2
                && snapshot.tile().is_some_and(|(px, pz, pl)| {
                    pl == *level && (px - x).abs().max((pz - z).abs()) <= *radius
                })
                && !snapshot.ground_items().iter().any(|item| {
                    item.count > 0
                        && ids.contains(&item.def.id)
                        && item.tile.level == *level
                        && (item.tile.x - x).abs().max((item.tile.z - z).abs()) <= *radius
                })
        }
        SnapshotProof::Npc {
            type_id,
            near,
            radius,
            index,
            form,
            options,
            no_option,
            sequence,
            frame,
            present,
        } => {
            let player_tile = if near.is_none() && radius.is_some() {
                let Some((x, z, level)) = snapshot.tile() else {
                    return Ok(false);
                };
                Some(WorldTile { x, z, level })
            } else {
                None
            };
            let center = near.as_ref().or(player_tile.as_ref());
            let found = snapshot.npcs().iter().any(|npc| {
                npc.r#type == Some(*type_id)
                    && index.is_none_or(|expected| npc.index == expected)
                    && form.is_none_or(|expected| npc.r#type == Some(expected))
                    && center.is_none_or(|center| {
                        api::query::chebyshev_to(*center, npc.network)
                            <= radius.as_ref().copied().unwrap_or(0)
                    })
                    && options.iter().all(|expected| {
                        has_action(npc.actions.iter().flatten().map(String::as_str), expected)
                    })
                    && no_option.as_ref().is_none_or(|expected| {
                        !has_action(npc.actions.iter().flatten().map(String::as_str), expected)
                    })
                    && sequence.is_none_or(|expected| npc.animation == expected)
                    && frame.is_none_or(|expected| npc.animation_frame == expected)
            });
            found == *present
        }
        SnapshotProof::Loc {
            at,
            name,
            form,
            options,
            first_option,
            no_option,
        } => {
            if !snapshot.ingame() || snapshot.scene_state() != 2 {
                return Ok(false);
            }
            snapshot.locs().iter().any(|loc| {
                loc.tile == *at
                    && name
                        .as_ref()
                        .is_none_or(|expected| loc.name.as_ref() == Some(expected))
                    && form.is_none_or(|expected| loc.id == expected)
                    && options.iter().all(|expected| {
                        has_action(loc.actions.iter().flatten().map(String::as_str), expected)
                    })
                    && first_option.as_ref().is_none_or(|expected| {
                        loc.actions
                            .iter()
                            .flatten()
                            .next()
                            .is_some_and(|actual| action_matches(actual, expected))
                    })
                    && no_option.as_ref().is_none_or(|expected| {
                        !has_action(loc.actions.iter().flatten().map(String::as_str), expected)
                    })
            })
        }
        SnapshotProof::NpcNameNear {
            name,
            x,
            z,
            level,
            radius,
        } => snapshot.npcs().iter().any(|npc| {
            npc.name.as_deref() == Some(name.as_str())
                && api::query::chebyshev_to(
                    npc.network,
                    WorldTile {
                        x: *x,
                        z: *z,
                        level: *level,
                    },
                ) <= *radius
        }),
        SnapshotProof::NpcNameTargetingLocal { name } => {
            let slot = if snapshot.self_slot() >= 0 {
                Some(snapshot.self_slot() as usize)
            } else {
                snapshot.local_player().map(|player| player.player.index)
            };
            slot.is_some_and(|slot| {
                snapshot.npcs().iter().any(|npc| {
                    npc.name.as_deref() == Some(name.as_str())
                        && npc.target.is_some_and(|target| {
                            target.kind == ActorKind::Player && target.index == slot
                        })
                })
            })
        }
        SnapshotProof::LocalTargetingNpcName { name } => {
            let Some(target) = snapshot
                .local_player()
                .and_then(|player| player.player.actor.target)
            else {
                return Ok(false);
            };
            target.kind == ActorKind::Npc
                && snapshot.npcs().iter().any(|npc| {
                    npc.index == target.index && npc.name.as_deref() == Some(name.as_str())
                })
        }
        SnapshotProof::NpcNameUnengagedInSight { name, radius } => {
            npc_unengaged_in_sight(snapshot, name, *radius)
        }
        SnapshotProof::LocActionNear {
            id,
            x,
            z,
            level,
            radius,
            action,
            present,
        } => {
            if !snapshot.ingame() || snapshot.scene_state() != 2 {
                return Ok(false);
            }
            let found = snapshot.locs().iter().any(|loc| {
                loc.id == *id
                    && loc.tile.level == *level
                    && (loc.tile.x - x).abs().max((loc.tile.z - z).abs()) <= *radius
                    && has_action(loc.actions.iter().flatten().map(String::as_str), action)
            });
            found == *present
        }
        SnapshotProof::VisitedWithin {
            min_x,
            max_x,
            min_z,
            max_z,
            level,
        } => {
            crate::proof::visited_within_contains(snapshot, *min_x, *max_x, *min_z, *max_z, *level)
        }
        SnapshotProof::Chat { needle } => snapshot
            .chat_lines()
            .first()
            .is_some_and(|line| line.text.contains(needle.as_str())),
        SnapshotProof::FreshTutorial { since } => {
            let baseline = mark(*since)?.chat_sequence.unwrap_or(i32::MIN);
            crate::proof::fresh_chat_contains(
                snapshot,
                baseline,
                crate::tutorial::TUTORIAL_CHAT_NEEDLE,
            )
        }
        SnapshotProof::FreshChat { needle, since }
        | SnapshotProof::FreshChatSincePreviousStep { needle, since } => {
            let baseline = mark(*since)?.chat_sequence.unwrap_or(i32::MIN);
            crate::proof::fresh_chat_contains(snapshot, baseline, needle)
        }
        SnapshotProof::FreshQuestCompletion {
            chat_needle,
            main_root_component_id,
            component_id,
            text,
            since,
        } => {
            let baseline = mark(*since)?.chat_sequence.unwrap_or(i32::MIN);
            crate::proof::fresh_quest_completion_contains(
                snapshot.chat_lines(),
                snapshot.widgets(),
                baseline,
                chat_needle,
                *main_root_component_id,
                *component_id,
                text,
            )
        }
        SnapshotProof::StatXpGain { id, min, since }
        | SnapshotProof::FreshStatXpGain { id, min, since } => {
            xp_since(*id, *since)?.is_some_and(|delta| delta >= i64::from(*min))
        }
        SnapshotProof::XpDelta { id, min, since } => {
            snapshot.ingame()
                && snapshot.scene_state() == 2
                && xp_since(*id, *since)?.is_some_and(|delta| delta >= i64::from(*min))
        }
        SnapshotProof::ProductDelta { id, count, since } => {
            let before = mark_inventory_count(mark(*since)?, *id)?;
            snapshot.ingame()
                && snapshot.scene_state() == 2
                && snapshot
                    .inventory_ready()
                    .then(|| Container::Inventory.total(snapshot, *id))
                    .flatten()
                    .is_some_and(|after| after.saturating_sub(before) >= i64::from(*count))
        }
        SnapshotProof::InputConsumed { id, count, since } => {
            let before = mark_inventory_count(mark(*since)?, *id)?;
            snapshot.ingame()
                && snapshot.scene_state() == 2
                && snapshot
                    .inventory_ready()
                    .then(|| Container::Inventory.total(snapshot, *id))
                    .flatten()
                    .is_some_and(|after| before.saturating_sub(after) >= i64::from(*count))
        }
        SnapshotProof::ClueReplaced { seeded, since } => {
            let before = mark_inventory_count(mark(*since)?, *seeded)?;
            let Some(items) = Container::Inventory.items(snapshot) else {
                return Ok(false);
            };
            let after = items
                .iter()
                .filter(|item| item.def.id == *seeded)
                .map(|item| i64::from(item.count))
                .sum::<i64>();
            snapshot.ingame()
                && snapshot.scene_state() == 2
                && before > 0
                && after == 0
                && items.iter().any(|item| {
                    item.count > 0
                        && item.def.id != *seeded
                        && selected.items().iter().any(|definition| {
                            definition.id == item.def.id
                                && matches!(
                                    definition.name.as_deref(),
                                    Some("Clue scroll") | Some("Casket")
                                )
                        })
                })
        }
    };
    Ok(result)
}

fn mark_inventory_count(mark: &MarkData, id: i32) -> Result<i64, String> {
    Ok(mark_slots(mark, Container::Inventory)?
        .iter()
        .filter(|(_, object, _)| *object == id)
        .map(|(_, _, count)| i64::from(*count))
        .sum())
}

fn npc_unengaged_in_sight(snapshot: &GameSnapshot, name: &str, radius: i32) -> bool {
    use api::line_of_sight::{has_line_of_sight_local, Footprint};

    let Some((x, z, level)) = snapshot.tile() else {
        return false;
    };
    let scene = snapshot.scene();
    if !snapshot.ingame() || snapshot.scene_state() != 2 || !scene.available || scene.level != level
    {
        return false;
    }
    let flags = |lx: i32, lz: i32| {
        if lx < 0 || lz < 0 || lx >= scene.width || lz >= scene.height {
            return None;
        }
        scene
            .collision_flags
            .get((lx * scene.height + lz) as usize)
            .copied()
    };
    let player_tile = WorldTile { x, z, level };
    let player = Footprint {
        lx: x - scene.base_x,
        lz: z - scene.base_z,
        size: 1,
    };
    if flags(player.lx, player.lz).is_none() {
        return false;
    }
    snapshot.npcs().iter().any(|npc| {
        let tile = npc.network;
        let source = Footprint {
            lx: tile.x - scene.base_x,
            lz: tile.z - scene.base_z,
            size: npc.size,
        };
        npc.name.as_deref() == Some(name)
            && tile.level == level
            && api::query::chebyshev_to(tile, player_tile) <= radius
            && !npc.in_combat
            && npc.target.is_none()
            && (npc.total_health == 0 || npc.health > 0)
            && flags(source.lx, source.lz).is_some()
            && has_line_of_sight_local(&flags, source, player)
    })
}

fn require_packet_provider(
    journal: &FileJournal,
    kind: RecordKind,
    required: bool,
) -> Result<(), String> {
    if required && !journal.has_packet_provider(kind) {
        return Err(format!("{kind:?} packet provider is not armed"));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum MatcherRef<'a> {
    Stat(&'a StatMatcher),
    Inventory(&'a InventoryMatcher),
    Message(&'a MessageMatcher),
    Var(&'a VarMatcher),
}

impl MatcherRef<'_> {
    fn kind(self) -> RecordKind {
        match self {
            Self::Stat(_) => RecordKind::Stat,
            Self::Inventory(_) => RecordKind::Inventory,
            Self::Message(_) => RecordKind::Message,
            Self::Var(_) => RecordKind::Varp,
        }
    }
}

impl<'a> From<&'a RecordMatcher> for MatcherRef<'a> {
    fn from(matcher: &'a RecordMatcher) -> Self {
        match matcher {
            RecordMatcher::Stat(matcher) => Self::Stat(matcher),
            RecordMatcher::Inventory(matcher) => Self::Inventory(matcher),
            RecordMatcher::Message(matcher) => Self::Message(matcher),
            RecordMatcher::Var(matcher) => Self::Var(matcher),
        }
    }
}

#[allow(clippy::too_many_arguments)] // Selectors mirror the FileCheck schema.
fn received_check(
    after: u64,
    exact: Exactness,
    stats: &[StatMatcher],
    inventories: &[InventoryMatcher],
    messages: &[MessageMatcher],
    vars: &[VarMatcher],
    _snapshot: &GameSnapshot,
    journal: &FileJournal,
    session: u64,
) -> Result<Option<Vec<u64>>, String> {
    let mark = journal.get_mark(after, session)?;
    let matchers: Vec<_> = stats
        .iter()
        .map(MatcherRef::Stat)
        .chain(inventories.iter().map(MatcherRef::Inventory))
        .chain(messages.iter().map(MatcherRef::Message))
        .chain(vars.iter().map(MatcherRef::Var))
        .collect();
    for matcher in &matchers {
        if journal.history_evicted(matcher.kind(), mark.at.serial) {
            return Err(format!(
                "journal-evicted: received {:?} history",
                matcher.kind()
            ));
        }
        if exact == Exactness::Packet && journal.has_packet_gap_since(mark, matcher.kind()) {
            return Err(format!(
                "received: packet history gap for {:?} after mark {after}",
                matcher.kind()
            ));
        }
    }
    let mut records: Vec<_> = journal
        .records()
        .filter(|record| {
            record.serial > mark.at.serial
                && record.session == session
                && matchers
                    .iter()
                    .any(|matcher| matcher.kind() == record.kind())
        })
        .collect();
    records.sort_unstable_by_key(|record| record.serial);

    let mut assigned = vec![None; records.len()];
    for matcher_index in 0..matchers.len() {
        let mut visited = vec![false; records.len()];
        if !augment_record_match(
            matcher_index,
            &matchers,
            &records,
            exact,
            &mut visited,
            &mut assigned,
        ) {
            return Ok(None);
        }
    }
    let mut serials: Vec<_> = assigned
        .iter()
        .enumerate()
        .filter_map(|(index, matcher)| matcher.map(|_| records[index].serial))
        .collect();
    serials.sort_unstable();
    Ok(Some(serials))
}

fn augment_record_match(
    matcher_index: usize,
    matchers: &[MatcherRef<'_>],
    records: &[&JournalRecord],
    exact: Exactness,
    visited: &mut [bool],
    assigned: &mut [Option<usize>],
) -> bool {
    for (record_index, record) in records.iter().enumerate() {
        if visited[record_index] || !record_matches(record, matchers[matcher_index], exact) {
            continue;
        }
        visited[record_index] = true;
        if assigned[record_index].is_none()
            || augment_record_match(
                assigned[record_index].expect("record has a matching matcher"),
                matchers,
                records,
                exact,
                visited,
                assigned,
            )
        {
            assigned[record_index] = Some(matcher_index);
            return true;
        }
    }
    false
}

fn record_matches(record: &JournalRecord, matcher: MatcherRef<'_>, exact: Exactness) -> bool {
    if record.source
        != match exact {
            Exactness::Value => RecordSource::Value,
            Exactness::Packet => RecordSource::Packet,
        }
    {
        return false;
    }
    match (matcher, &record.data) {
        (MatcherRef::Stat(matcher), JournalData::Stat { skill, after, .. }) => {
            *skill == matcher.skill
                && matcher.base.as_ref().is_none_or(|amount| {
                    after.base.is_some_and(|value| amount.matches(value.into()))
                })
                && matcher.effective.as_ref().is_none_or(|amount| {
                    after
                        .effective
                        .is_some_and(|value| amount.matches(value.into()))
                })
                && matcher
                    .xp
                    .as_ref()
                    .is_none_or(|amount| after.xp.is_some_and(|value| amount.matches(value.into())))
        }
        (
            MatcherRef::Inventory(matcher),
            JournalData::Inventory {
                container,
                component,
                slot,
                before,
                after,
            },
        ) => {
            matcher.container.is_none_or(|value| value == *container)
                && matcher.component.is_none_or(|value| value == *component)
                && matcher.slot.is_none_or(|value| value == *slot)
                && matcher.item.is_none_or(|id| {
                    before.is_some_and(|item| item.id == id)
                        || after.is_some_and(|item| item.id == id)
                })
                && matcher.delta.as_ref().is_none_or(|amount| {
                    let before = before.map_or(0, |item| item.count);
                    let after = after.map_or(0, |item| item.count);
                    amount.matches(i64::from(after) - i64::from(before))
                })
        }
        (
            MatcherRef::Inventory(matcher),
            JournalData::InventoryReset {
                carried, component, ..
            },
        ) => {
            matcher.container.is_none_or(|container| {
                matches!(
                    (container, carried),
                    (Container::Inventory, true)
                        | (Container::Equipment, false)
                        | (Container::Component(_), _)
                )
            }) && matcher.component.is_none_or(|value| value == *component)
                && matcher.slot.is_none()
                && matcher.item.is_none()
                && matcher.delta.is_none()
        }
        (MatcherRef::Message(matcher), JournalData::Message { type_, text, .. }) => {
            matcher.type_.is_none_or(|value| value == *type_)
                && matcher
                    .text
                    .as_ref()
                    .is_none_or(|value| value.matches(text))
        }
        (MatcherRef::Var(matcher), JournalData::Varp { id, value }) => {
            *id == matcher.id
                && matcher
                    .value
                    .as_ref()
                    .is_none_or(|amount| amount.matches(i64::from(*value)))
        }
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)] // Selectors mirror the FileCheck schema.
fn silent_check(
    after: u64,
    ticks: u64,
    exact: Exactness,
    stats: &[StatMatcher],
    inventories: &[InventoryMatcher],
    vars: &[VarMatcher],
    messages: Option<&SilentMessageFilter>,
    dialogue: bool,
    snapshot: &GameSnapshot,
    journal: &FileJournal,
    session: u64,
) -> Result<Option<Vec<u64>>, String> {
    let mark = journal.mark_any(after)?;
    if mark.at.session != session
        || journal.has_session_change_after(mark.at.serial, mark.at.session)
        || journal
            .records()
            .any(|record| record.serial > mark.at.serial && record.session != mark.at.session)
    {
        return Err("silent: session changed inside the observation window".into());
    }
    let end_tick = mark
        .at
        .tick
        .checked_add(ticks)
        .ok_or("silent: tick window overflow")?;
    let current_tick = u64::from(snapshot.tick());
    if current_tick < mark.at.tick {
        return Err("silent: session tick moved backwards".into());
    }

    let matchers: Vec<_> = stats
        .iter()
        .map(MatcherRef::Stat)
        .chain(inventories.iter().map(MatcherRef::Inventory))
        .chain(vars.iter().map(MatcherRef::Var))
        .collect();
    for matcher in &matchers {
        if journal.history_evicted(matcher.kind(), mark.at.serial) {
            return Err(format!(
                "journal-evicted: silent {:?} history",
                matcher.kind()
            ));
        }
        if exact == Exactness::Packet && journal.has_packet_gap_since(mark, matcher.kind()) {
            return Err(format!(
                "silent: packet history gap for {:?} after mark {after}",
                matcher.kind()
            ));
        }
    }
    if dialogue && journal.history_evicted(RecordKind::Dialogue, mark.at.serial) {
        return Err("journal-evicted: silent dialogue history".into());
    }
    if messages.is_some() && journal.history_evicted(RecordKind::Message, mark.at.serial) {
        return Err("journal-evicted: silent message history".into());
    }

    for record in journal
        .records()
        .filter(|record| record.serial > mark.at.serial)
    {
        if record.session != session {
            return Err("silent: session changed inside the observation window".into());
        }
        if record.source
            != match exact {
                Exactness::Value => RecordSource::Value,
                Exactness::Packet => RecordSource::Packet,
            }
        {
            continue;
        }
        if let Some(tick) = record.tick {
            if tick < mark.at.tick || tick > end_tick {
                continue;
            }
        } else if exact != Exactness::Packet {
            continue;
        }
        let forbidden = match &record.data {
            JournalData::Dialogue { .. } => dialogue,
            JournalData::Message { type_, text, .. } => messages.is_some_and(|filter| {
                filter.type_.is_none_or(|wanted| wanted == *type_)
                    && !filter.allow.iter().any(|allow| allow.matches(text))
            }),
            _ => matchers.iter().any(|matcher| {
                matcher.kind() == record.kind() && record_matches(record, *matcher, exact)
            }),
        };
        if forbidden {
            let detail = match &record.data {
                JournalData::Message { type_, text, .. } => format!(": type {type_} {text:?}"),
                _ => String::new(),
            };
            return Err(format!(
                "silent: forbidden {:?} observation at record {}{detail}",
                record.kind(),
                record.serial
            ));
        }
    }

    if current_tick < end_tick {
        Ok(None)
    } else {
        Ok(Some(Vec::new()))
    }
}

#[allow(clippy::too_many_arguments)] // Selectors mirror the FileCheck schema.
fn count_check(
    matcher: &RecordMatcher,
    from: u64,
    to: Option<&CountEnd>,
    amount: &Amount,
    exact: Exactness,
    _snapshot: &GameSnapshot,
    journal: &FileJournal,
    session: u64,
    context: &CheckContext,
) -> Result<Option<Vec<u64>>, String> {
    let start = journal.get_mark(from, session)?;
    let (end_serial, complete) = match to {
        Some(CountEnd::Mark(handle)) => {
            let end = journal.get_mark(*handle, session)?;
            (end.at.serial, true)
        }
        Some(CountEnd::Label(label)) => {
            let Some(end) = context.label(label) else {
                return Ok(None);
            };
            if end.actor != journal.actor_id() || end.session != session {
                return Err("count: incomparable actor or session label".into());
            }
            let Some(serial) = end.serials.iter().copied().max() else {
                return Ok(None);
            };
            (serial, true)
        }
        None => (journal.last_serial(), false),
    };
    if end_serial < start.at.serial {
        return Err("count: end precedes start watermark".into());
    }
    if journal.history_evicted(matcher.kind(), start.at.serial) {
        return Err(format!(
            "journal-evicted: count {:?} history",
            matcher.kind()
        ));
    }
    if exact == Exactness::Packet && journal.has_packet_gap_since(start, matcher.kind()) {
        return Err(format!(
            "count: packet history gap for {:?} after mark {from}",
            matcher.kind()
        ));
    }
    if journal.has_session_change_after(start.at.serial, session) {
        return Err("count: session changed inside the interval".into());
    }
    let mut serials: Vec<_> = journal
        .records_after(matcher.kind(), start.at.serial)
        .filter(|record| {
            record.serial <= end_serial
                && record.session == session
                && record_matches(record, matcher.into(), exact)
        })
        .map(|record| record.serial)
        .collect();
    serials.sort_unstable();
    let count = serials.len() as i64;
    if amount.matches(count) {
        return Ok(Some(serials));
    }
    if count > amount.max || complete {
        return Err(format!(
            "count: observed {count} records outside {}..={}",
            amount.min, amount.max
        ));
    }
    Ok(None)
}

fn order_check(
    first_name: &str,
    then_name: &str,
    gap: Option<&Amount>,
    journal: &FileJournal,
    session: u64,
    context: &CheckContext,
) -> Result<Option<Vec<u64>>, String> {
    let (Some(first), Some(then)) = (context.label(first_name), context.label(then_name)) else {
        return Ok(None);
    };
    if first.actor != journal.actor_id()
        || then.actor != journal.actor_id()
        || first.session != session
        || then.session != session
        || first.actor != then.actor
        || first.session != then.session
    {
        return Err("order: incomparable actor or session labels".into());
    }
    let first_last = first.serials.iter().copied().max();
    let then_first = then.serials.iter().copied().min();
    if let (Some(first_last), Some(then_first)) = (first_last, then_first) {
        if first_last >= then_first {
            return Ok(None);
        }
    } else if first.tick >= then.tick {
        return Ok(None);
    }
    let tick_gap = then.tick.saturating_sub(first.tick);
    if gap.is_some_and(|amount| !amount.matches(tick_gap as i64)) {
        return Ok(None);
    }
    let mut serials = first.serials.clone();
    serials.extend_from_slice(&then.serials);
    serials.sort_unstable();
    serials.dedup();
    Ok(Some(serials))
}

fn var_check(
    id: i32,
    value: &Amount,
    journal: &FileJournal,
    session: u64,
) -> Result<Option<Vec<u64>>, String> {
    let latest = journal
        .records_after(RecordKind::Varp, 0)
        .filter(|record| record.session == session)
        .filter_map(|record| match record.data {
            JournalData::Varp {
                id: observed,
                value,
            } if observed == id => Some((record.serial, value)),
            _ => None,
        })
        .max_by_key(|(serial, _)| *serial);
    let Some((serial, observed)) = latest else {
        if journal.history_evicted(RecordKind::Varp, 0) {
            return Err("journal-evicted: varp history".into());
        }
        return Ok(None);
    };
    Ok(value.matches(i64::from(observed)).then(|| vec![serial]))
}

fn bind_named(name: &str, params: NamedPredicateParams) -> Result<FileCheck, String> {
    NamedPredicateGate::bind(name, params, PredicateAuthority::Script)
        .map(FileCheck::Named)
        .map_err(|error| error.to_string())
}

fn mark_slots(mark: &MarkData, container: Container) -> Result<&Slots, String> {
    match container {
        Container::Inventory => mark.inventory.as_ref(),
        Container::Equipment => mark.equipment.as_ref(),
        Container::Bank => mark.bank.as_ref(),
        Container::Component(_) => None,
    }
    .ok_or_else(|| "mark has no observed container".into())
}

pub fn object(value: Value, field: &str) -> Result<Map<String, Value>, String> {
    match value {
        Value::Object(args) => Ok(args),
        _ => Err(format!("{field}: expected table with named fields")),
    }
}

pub fn empty(args: &Map<String, Value>) -> Result<(), String> {
    args.keys()
        .next()
        .map_or(Ok(()), |key| Err(format!("unknown argument {key:?}")))
}

pub fn take_string(args: &mut Map<String, Value>, field: &str) -> Result<String, String> {
    match args.remove(field) {
        Some(Value::String(text)) if (1..=160).contains(&text.len()) => Ok(text),
        _ => Err(format!("{field}: expected 1–160 bytes of text")),
    }
}

fn optional_text(args: &mut Map<String, Value>, field: &str) -> Result<Option<String>, String> {
    args.remove(field)
        .map(|value| match value {
            Value::String(text) if (1..=160).contains(&text.len()) => Ok(text),
            _ => Err(format!("{field}: expected 1–160 bytes of text")),
        })
        .transpose()
}

fn integer(value: &Value, min: i32, max: i32, field: &str) -> Result<i32, String> {
    value
        .as_i64()
        .and_then(|n| i32::try_from(n).ok())
        .filter(|n| (min..=max).contains(n))
        .ok_or_else(|| format!("{field}: expected integer in {min}..={max}"))
}

fn optional_int(
    args: &mut Map<String, Value>,
    field: &str,
    min: i32,
    max: i32,
) -> Result<Option<i32>, String> {
    args.remove(field)
        .map(|value| integer(&value, min, max, field))
        .transpose()
}

fn take_int(args: &mut Map<String, Value>, field: &str, min: i32, max: i32) -> Result<i32, String> {
    optional_int(args, field, min, max)?.ok_or_else(|| format!("missing {field}"))
}

fn coordinates(args: &mut Map<String, Value>) -> Result<(i32, i32, i32), String> {
    Ok((
        take_int(args, "x", 0, 16383)?,
        take_int(args, "z", 0, 16383)?,
        take_int(args, "level", 0, 3)?,
    ))
}

fn take_tile(args: &mut Map<String, Value>, field: &str) -> Result<WorldTile, String> {
    let value = args
        .remove(field)
        .ok_or_else(|| format!("missing {field}"))?;
    tile_value(value, field)
}

fn tile_value(value: Value, field: &str) -> Result<WorldTile, String> {
    let mut fields = object(value, field)?;
    let (x, z, level) = coordinates(&mut fields)?;
    empty(&fields)?;
    Ok(WorldTile { x, z, level })
}
fn skill(args: &mut Map<String, Value>) -> Result<i32, String> {
    let value = args.remove("skill").ok_or("missing skill")?;
    let index = if let Some(name) = value.as_str() {
        (0..api::snapshot::stat_count())
            .find(|&index| api::snapshot::stat_name(index).eq_ignore_ascii_case(name))
            .ok_or("unknown skill")?
    } else {
        integer(&value, 0, 24, "skill")? as usize
    };
    if !api::snapshot::stat_used(index) {
        return Err("skill not enabled".into());
    }
    Ok(index as i32)
}

fn handle(args: &mut Map<String, Value>, field: &str) -> Result<u64, String> {
    args.remove(field)
        .and_then(|value| value.as_u64())
        .filter(|n| *n != 0)
        .ok_or_else(|| format!("{field}: expected a mark handle"))
}

fn constraint(value: Value) -> Result<Amount, String> {
    if let Some(exact) = value.as_i64() {
        return Ok(Amount {
            min: exact,
            max: exact,
        });
    }
    let mut args = object(value, "amount")?;
    let result = amount(&mut args)?;
    empty(&args)?;
    Ok(result)
}

fn amount(args: &mut Map<String, Value>) -> Result<Amount, String> {
    let get = |args: &mut Map<String, Value>, name| {
        args.remove(name)
            .map(|value| value.as_i64().ok_or("amount: expected integer"))
            .transpose()
    };
    match (get(args, "eq")?, get(args, "min")?, get(args, "max")?) {
        (Some(exact), None, None) => Ok(Amount {
            min: exact,
            max: exact,
        }),
        (None, low, high) if low.is_some() || high.is_some() => {
            let min = low.unwrap_or(i64::MIN);
            let max = high.unwrap_or(i64::MAX);
            if min <= max {
                Ok(Amount { min, max })
            } else {
                Err("amount: min exceeds max".into())
            }
        }
        _ => Err("amount: use eq or min/max".into()),
    }
}
fn take_bool(args: &mut Map<String, Value>, field: &str) -> Result<bool, String> {
    args.remove(field)
        .map(|value| bool_value(&value, field))
        .transpose()?
        .ok_or_else(|| format!("missing {field}"))
}

fn bool_value(value: &Value, field: &str) -> Result<bool, String> {
    value
        .as_bool()
        .ok_or_else(|| format!("{field}: expected boolean"))
}

fn take_positive_u64(args: &mut Map<String, Value>, field: &str) -> Result<u64, String> {
    args.remove(field)
        .and_then(|value| value.as_u64())
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("{field}: expected positive integer"))
}

fn parse_amount_value(value: Value) -> Result<Amount, String> {
    constraint(value)
}

fn take_amount(args: &mut Map<String, Value>, field: &str) -> Result<Amount, String> {
    args.remove(field)
        .ok_or_else(|| format!("missing {field}"))
        .and_then(constraint)
}

fn amount_field_or_flat(args: &mut Map<String, Value>) -> Result<Amount, String> {
    if let Some(value) = args.remove("amount") {
        constraint(value)
    } else {
        amount(args)
    }
}

fn optional_exactness(args: &mut Map<String, Value>) -> Result<Option<Exactness>, String> {
    args.remove("exact")
        .map(|value| exactness_value(&value))
        .transpose()
}

fn take_exactness(args: &mut Map<String, Value>) -> Result<Exactness, String> {
    optional_exactness(args)?.ok_or_else(|| "missing exact".into())
}

fn exactness_value(value: &Value) -> Result<Exactness, String> {
    match value.as_str() {
        Some("value") => Ok(Exactness::Value),
        Some("packet") => Ok(Exactness::Packet),
        _ => Err("exact: expected \"value\" or \"packet\"".into()),
    }
}

fn take_i32_list(
    args: &mut Map<String, Value>,
    field: &str,
    min: i32,
    max: i32,
) -> Result<Vec<i32>, String> {
    let Value::Array(values) = args
        .remove(field)
        .ok_or_else(|| format!("missing {field}"))?
    else {
        return Err(format!("{field}: expected integer list"));
    };
    if values.is_empty() {
        return Err(format!("{field}: expected a nonempty list"));
    }
    values
        .iter()
        .map(|value| integer(value, min, max, field))
        .collect()
}

fn take_usize_list(
    args: &mut Map<String, Value>,
    field: &str,
    min: i32,
    max: i32,
) -> Result<Vec<usize>, String> {
    take_i32_list(args, field, min, max)
        .map(|values| values.into_iter().map(|value| value as usize).collect())
}

fn optional_string_list(
    args: &mut Map<String, Value>,
    field: &str,
) -> Result<Option<Vec<String>>, String> {
    args.remove(field)
        .map(|value| {
            let Value::Array(values) = value else {
                return Err(format!("{field}: expected string list"));
            };
            values
                .into_iter()
                .map(|value| match value {
                    Value::String(text) if (1..=160).contains(&text.len()) => Ok(text),
                    _ => Err(format!("{field}: expected 1–160 bytes of text")),
                })
                .collect()
        })
        .transpose()
}

fn take_string_list(args: &mut Map<String, Value>, field: &str) -> Result<Vec<String>, String> {
    Ok(optional_string_list(args, field)?.unwrap_or_default())
}

fn take_text_matcher_list(
    args: &mut Map<String, Value>,
    field: &str,
) -> Result<Vec<TextMatcher>, String> {
    let Some(value) = args.remove(field) else {
        return Ok(Vec::new());
    };
    let Value::Array(values) = value else {
        return Err(format!("{field}: expected text matcher list"));
    };
    values.into_iter().map(parse_text_matcher).collect()
}

fn take_matcher_list<T>(
    args: &mut Map<String, Value>,
    field: &str,
    parse: fn(Value) -> Result<T, String>,
) -> Result<Vec<T>, String> {
    let Some(value) = args.remove(field) else {
        return Ok(Vec::new());
    };
    // Appendix C shorthand: `inventories = true` selects the carried and worn
    // inventories.
    let value = match value {
        Value::Bool(true) if field == "inventories" => {
            Value::Array(vec![Value::from("inventory"), Value::from("worn")])
        }
        value => value,
    };
    let Value::Array(values) = value else {
        return Err(format!("{field}: expected matcher list"));
    };
    values.into_iter().map(parse).collect()
}

fn parse_text_matcher(value: Value) -> Result<TextMatcher, String> {
    if let Value::String(text) = value {
        if (1..=160).contains(&text.len()) {
            return Ok(TextMatcher::Exact(text));
        }
        return Err("text matcher: expected 1–160 bytes of text".into());
    }
    let mut args = object(value, "text")?;
    let (key, matcher) = ["exact", "contains", "prefix"]
        .into_iter()
        .find_map(|key| args.remove(key).map(|value| (key, value)))
        .ok_or("text matcher: expected exact, contains, or prefix")?;
    if args
        .keys()
        .any(|key| matches!(key.as_str(), "exact" | "contains" | "prefix"))
    {
        return Err("text matcher: use one matching mode".into());
    }
    let text = match matcher {
        Value::String(text) if (1..=160).contains(&text.len()) => text,
        _ => return Err("text matcher: expected 1–160 bytes of text".into()),
    };
    empty(&args)?;
    match key {
        "exact" => Ok(TextMatcher::Exact(text)),
        "contains" => Ok(TextMatcher::Contains(text)),
        _ => Ok(TextMatcher::Prefix(text)),
    }
}

fn parse_stat_matcher(value: Value) -> Result<StatMatcher, String> {
    // Shorthand: a bare skill name selects every field of that skill.
    let value = match value {
        Value::String(skill) => {
            Value::Object(Map::from_iter([("skill".into(), Value::String(skill))]))
        }
        value => value,
    };
    let mut args = object(value, "stat matcher")?;
    let matcher = StatMatcher {
        skill: skill(&mut args)?,
        base: args.remove("base").map(constraint).transpose()?,
        effective: args.remove("effective").map(constraint).transpose()?,
        xp: args.remove("xp").map(constraint).transpose()?,
    };
    empty(&args)?;
    Ok(matcher)
}

fn parse_inventory_matcher(value: Value) -> Result<InventoryMatcher, String> {
    // Shorthand: a bare container name selects that whole container.
    let value = match value {
        Value::String(container) => Value::Object(Map::from_iter([(
            "container".into(),
            Value::String(container),
        )])),
        value => value,
    };
    let mut args = object(value, "inventory matcher")?;
    let matcher = InventoryMatcher {
        container: args.remove("container").map(Container::parse).transpose()?,
        component: optional_int(&mut args, "component", 0, i32::MAX)?,
        slot: optional_int(&mut args, "slot", 0, i32::MAX)?,
        item: optional_int(&mut args, "item", 0, i32::MAX)?,
        delta: args.remove("delta").map(constraint).transpose()?,
    };
    if matcher.container.is_none()
        && matcher.component.is_none()
        && matcher.slot.is_none()
        && matcher.item.is_none()
        && matcher.delta.is_none()
    {
        return Err("inventory matcher: expected a selector".into());
    }
    empty(&args)?;
    Ok(matcher)
}

fn parse_message_matcher(value: Value) -> Result<MessageMatcher, String> {
    let mut args = object(value, "message matcher")?;
    let matcher = MessageMatcher {
        type_: optional_int(&mut args, "type", 0, i32::MAX)?,
        text: args.remove("text").map(parse_text_matcher).transpose()?,
    };
    if matcher.type_.is_none() && matcher.text.is_none() {
        return Err("message matcher: expected type or text".into());
    }
    empty(&args)?;
    Ok(matcher)
}

fn parse_var_matcher(value: Value) -> Result<VarMatcher, String> {
    let mut args = object(value, "var matcher")?;
    let matcher = VarMatcher {
        id: take_int(&mut args, "id", 0, i32::MAX)?,
        value: args.remove("value").map(constraint).transpose()?,
    };
    empty(&args)?;
    Ok(matcher)
}

fn parse_record_matcher(value: Value) -> Result<RecordMatcher, String> {
    let mut args = object(value, "record")?;
    let kind = take_string(&mut args, "kind")?;
    let matcher = match kind.as_str() {
        "stat" => RecordMatcher::Stat(parse_stat_matcher(Value::Object(args))?),
        "inventory" => RecordMatcher::Inventory(parse_inventory_matcher(Value::Object(args))?),
        "message" => RecordMatcher::Message(parse_message_matcher(Value::Object(args))?),
        "var" => RecordMatcher::Var(parse_var_matcher(Value::Object(args))?),
        _ => return Err("record.kind: expected stat, inventory, message, or var".into()),
    };
    Ok(matcher)
}

fn parse_silent_message_filter(value: Value) -> Result<SilentMessageFilter, String> {
    match value {
        Value::Bool(true) => Ok(SilentMessageFilter {
            type_: None,
            allow: Vec::new(),
        }),
        Value::Bool(false) => Err("messages: false is not an observation selector".into()),
        value => {
            let mut args = object(value, "messages")?;
            let filter = SilentMessageFilter {
                type_: optional_int(&mut args, "type", 0, i32::MAX)?,
                allow: take_text_matcher_list(&mut args, "allow")?,
            };
            if filter.type_.is_none() && filter.allow.is_empty() {
                return Err("messages: expected type or allow list".into());
            }
            empty(&args)?;
            Ok(filter)
        }
    }
}

fn parse_count_end(value: Value) -> Result<CountEnd, String> {
    if let Some(mark) = value.as_u64().filter(|mark| *mark > 0) {
        return Ok(CountEnd::Mark(mark));
    }
    if let Value::String(label) = value {
        if (1..=160).contains(&label.len()) {
            return Ok(CountEnd::Label(label));
        }
    }
    Err("to: expected positive mark handle or label".into())
}

#[cfg(test)]
#[path = "file_scenario_tests.rs"]
mod tests;
