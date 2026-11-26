//! File-scenario admission and checks. This layer has no script runtime authority.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use api::game_data::SelectedGameData;
use api::snapshot::{GameSnapshot, ItemView, WorldTile};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::named_predicates::{BankExactParams, StandingBankClosedParams};
use crate::{
    NamedPredicateGate, NamedPredicateParams, PredicateAuthority, PredicateContext, Proof,
    ScenarioSettings,
};

const MAX_ROLES: usize = 10;
const MAX_MARKS: usize = 256;
const MAX_RECORDS: usize = 4096;

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

#[derive(Clone, Copy, Debug)]
pub struct Mark {
    pub session: u64,
    pub tick: u64,
    pub source_serial: u64,
}

type Slots = Vec<(i32, i32, i32)>;

#[derive(Debug)]
struct MarkData {
    at: Mark,
    action_floor: usize,
    tile: Option<(i32, i32, i32)>,
    xp: Vec<(i32, i32)>,
    inventory: Option<Slots>,
    equipment: Slots,
    bank: Option<Slots>,
}

#[derive(Debug)]
struct Action {
    session: u64,
    id: u64,
    sent: bool,
}

/// A role's bounded journal. Session checks apply when the caller resolves a mark.
#[derive(Debug, Default)]
pub struct FileJournal {
    marks: BTreeMap<u64, MarkData>,
    actions: Vec<Action>,
}

impl FileJournal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mark(
        &mut self,
        handle: u64,
        snapshot: &GameSnapshot,
        session: u64,
        source_serial: u64,
    ) -> Result<Mark, String> {
        if self.marks.len() == MAX_MARKS || self.marks.contains_key(&handle) {
            return Err("scenario mark capacity or duplicate handle".into());
        }
        let at = Mark {
            session,
            tick: u64::from(snapshot.tick()),
            source_serial,
        };
        self.marks.insert(
            handle,
            MarkData {
                at,
                action_floor: self.actions.len(),
                tile: snapshot.tile(),
                xp: snapshot
                    .stats()
                    .iter()
                    .map(|stat| (stat.index, stat.xp))
                    .collect(),
                inventory: Container::Inventory.slots(snapshot),
                equipment: slots_of(snapshot.equipment()),
                bank: Container::Bank.slots(snapshot),
            },
        );
        Ok(at)
    }

    pub fn mark_at(&self, handle: u64) -> Option<Mark> {
        self.marks.get(&handle).map(|mark| mark.at)
    }

    pub fn record_action(&mut self, session: u64, id: u64, sent: bool) -> Result<(), String> {
        if self.actions.len() == MAX_RECORDS
            || self
                .actions
                .last()
                .is_some_and(|last| last.id == id && last.session == session)
        {
            return Err("scenario action journal capacity or duplicate root".into());
        }
        self.actions.push(Action { session, id, sent });
        Ok(())
    }

    pub fn note_sent(&mut self, session: u64, id: u64) -> Result<(), String> {
        let action = self
            .actions
            .last_mut()
            .filter(|action| action.session == session && action.id == id && !action.sent)
            .ok_or("action send does not match an admitted root")?;
        action.sent = true;
        Ok(())
    }

    fn get_mark(&self, handle: u64, session: u64) -> Result<&MarkData, String> {
        self.marks
            .get(&handle)
            .filter(|mark| mark.at.session == session)
            .ok_or_else(|| format!("unknown or foreign-session mark {handle}"))
    }

    pub fn action_sent(&self, handle: u64, session: u64) -> Result<bool, String> {
        let mark = self.get_mark(handle, session)?;
        match &self.actions[mark.action_floor..] {
            [] => Ok(false),
            [action] if action.session == session => Ok(action.sent),
            _ => Err("action_sent: expected one matching root with a native send".into()),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Container {
    Inventory,
    Equipment,
    Bank,
    Component(i32),
}

impl Container {
    fn parse(value: Value) -> Result<Self, String> {
        match value.as_str() {
            Some("inventory") => Ok(Self::Inventory),
            Some("worn") => Ok(Self::Equipment),
            Some("bank") => Ok(Self::Bank),
            Some(_) => Err("unknown container".into()),
            None => integer(&value, 0, i32::MAX, "where").map(Self::Component),
        }
    }

    fn slots(self, snapshot: &GameSnapshot) -> Option<Slots> {
        self.items(snapshot).map(slots_of)
    }

    fn items(self, snapshot: &GameSnapshot) -> Option<&[ItemView]> {
        match self {
            Self::Inventory => (snapshot.inventory_size() > 0).then(|| snapshot.inventory()),
            Self::Equipment => Some(snapshot.equipment()),
            Self::Bank => Proof::BankOpen
                .check(snapshot, None)
                .then(|| snapshot.bank()),
            Self::Component(id) => [snapshot.inventory(), snapshot.equipment(), snapshot.bank()]
                .into_iter()
                .find(|rows| rows.iter().any(|row| row.component_id == id)),
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

fn slots_of(items: &[ItemView]) -> Slots {
    items
        .iter()
        .map(|item| (item.slot, item.def.id, item.count))
        .collect()
}

#[derive(Debug)]
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
}

/// Names with a runtime-independent evaluator. Registration consumes this same list.
pub const FILE_CHECK_ENTRIES: &[(&str, &str, &str)] = {
    macro_rules! check_entries {
        ($($name:literal),* $(,)?) => {
            &[$((concat!("expect.", $name), concat!("watch.", $name), concat!("probe.", $name))),*]
        };
    }
    check_entries!(
        "action_sent",
        "active_continue",
        "all",
        "amount_prompt",
        "arrived",
        "arrived_near",
        "arrived_ring",
        "bank_closed",
        "bank_exact",
        "bank_item",
        "bank_item_id",
        "bank_item_id_at_most",
        "bank_item_at_most",
        "bank_open",
        "chat_closed",
        "chat_choice",
        "container_unchanged",
        "equipment_id",
        "essence_mine",
        "ingame_scene_2",
        "item_count",
        "item",
        "item_at_most",
        "item_delta",
        "item_id",
        "item_id_at_most",
        "item_id_in_full_pack",
        "loc_id_near",
        "no_active_continue",
        "npc_at",
        "npc_near",
        "render_view_ready",
        "side_tab_available",
        "standing_bank_closed",
        "stat",
        "stat_at_most",
        "stat_value",
        "tile",
        "tutorial_closed",
        "xp_since",
    )
};

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
                amount: amount(&mut args)?,
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
                    amount: amount(&mut args)?,
                }
            }
            "container_unchanged" => Self::ContainerUnchanged {
                container: Container::parse(args.remove("where").ok_or("missing where")?)?,
                since: handle(&mut args, "since")?,
            },
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
            | Self::Item { since: Some(_), .. } => false,
            _ => true,
        }
    }

    pub fn is_gameplay(&self) -> bool {
        match self {
            Self::ActionSent(_) => false,
            Self::All(rows) => rows.iter().any(Self::is_gameplay),
            _ => true,
        }
    }

    pub fn check(
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

fn bind_named(name: &str, params: NamedPredicateParams) -> Result<FileCheck, String> {
    NamedPredicateGate::bind(name, params, PredicateAuthority::Script)
        .map(FileCheck::Named)
        .map_err(|error| error.to_string())
}

fn mark_slots(mark: &MarkData, container: Container) -> Result<&Slots, String> {
    match container {
        Container::Inventory => mark.inventory.as_ref(),
        Container::Equipment => Some(&mark.equipment),
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

#[cfg(test)]
#[path = "file_scenario_tests.rs"]
mod tests;
