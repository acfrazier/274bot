//! Shared wire and outcome types for native combat sessions (`api.combat`).
//!
//! The isolate decodes and validates the script's plain request object
//! ([`CombatSessionRequest::from_value`]); the FlatBuffer carries the typed
//! request to the slot, which resolves names against its selected game data
//! ([`CombatSessionRequest::resolve`]) into the native
//! [`crate::combat::CombatRequest`]. The machine itself is not changed.
use crate::combat::{
    AbortReason, Allowances, CombatReport, CombatRequest, CombatTables, MeleeMode, Pick, PrepItem,
    PrepReadiness, RangedMode, SpellRef, Style, Target, Unprotected,
};
use crate::native::ConfigError;
use api::gather_methods::SceneRegionInput;
use serde_json::{Map, Value};
use std::sync::Arc;

/// The live combat session page has the gather page's shape and wire table.
#[cfg(feature = "load")]
pub type CombatPage = crate::api_gather::GatherPage;

/// At most this many npc names or ids in one target.
pub const MAX_TARGETS: usize = 32;
/// At most this many search boxes.
pub const MAX_AREA_BOXES: usize = 16;
/// At most this many spells in a manual order (the machine indexes 32 spells).
pub const MAX_SPELLS: usize = 32;
/// Longest accepted npc name or spell alias, in bytes.
pub const MAX_NAME_BYTES: usize = 64;

/// What the session fights.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetSpec {
    /// Selected npc config names or display names.
    Names(Box<[Arc<str>]>),
    /// Npc type ids.
    Ids(Box<[i32]>),
    /// Whoever is already attacking the player.
    Attackers { npcs: bool, players: bool },
}

/// One validated `api.combat.fight` request. Defaults are the native
/// request's: nearest pick, engage radius 12, lost radius 20, 1500 ticks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatSessionRequest {
    pub target: TargetSpec,
    pub pick: Pick,
    pub not_targeting_others: bool,
    /// Search boxes `[min_x, min_z, max_x, max_z, level]`.
    pub area: Box<[[i32; 5]]>,
    pub radius: u8,
    pub lost_radius: u8,
    pub style: Style,
    pub melee_mode: Option<MeleeMode>,
    pub ranged_mode: RangedMode,
    pub spells: Option<Box<[Arc<str>]>>,
    pub fallback_spells: bool,
    pub prayer: bool,
    pub food: bool,
    pub potions: bool,
    pub retaliate: bool,
    pub budget_ticks: u16,
}

impl Default for CombatSessionRequest {
    fn default() -> Self {
        let native = CombatRequest::default();
        Self {
            target: TargetSpec::Attackers {
                npcs: true,
                players: true,
            },
            pick: Pick::Nearest,
            not_targeting_others: true,
            area: Box::new([]),
            radius: native.engage_radius,
            lost_radius: native.lost_radius,
            style: Style::Melee,
            melee_mode: None,
            ranged_mode: RangedMode::default(),
            spells: None,
            fallback_spells: false,
            prayer: true,
            food: true,
            potions: true,
            retaliate: native.retaliate,
            budget_ticks: native.budget_ticks,
        }
    }
}

const KEYS: &[&str] = &[
    "target",
    "pick",
    "notTargetingOthers",
    "area",
    "radius",
    "lostRadius",
    "style",
    "meleeMode",
    "rangedMode",
    "spells",
    "fallbackSpells",
    "prayer",
    "food",
    "potions",
    "retaliate",
    "budgetTicks",
];

const MALFORMED: &str = "invalid-settings";

fn setting(field: &str, code: &str) -> String {
    format!("invalid-setting:{field}:{code}")
}

/// A JS number that is an exact integer, else `invalid-settings`.
fn integer(value: &Value) -> Result<i64, String> {
    if let Some(value) = value.as_i64() {
        return Ok(value);
    }
    match value.as_f64() {
        Some(value) if value.is_finite() && value.fract() == 0.0 && value.abs() < 9.0e15 => {
            Ok(value as i64)
        }
        _ => Err(MALFORMED.into()),
    }
}

fn boolean(value: &Value) -> Result<bool, String> {
    value.as_bool().ok_or_else(|| MALFORMED.into())
}

fn text(value: &Value) -> Result<&str, String> {
    value.as_str().ok_or_else(|| MALFORMED.into())
}

fn ranged_u8(value: &Value, field: &str) -> Result<u8, String> {
    let value = integer(value)?;
    u8::try_from(value)
        .ok()
        .filter(|value| *value != 0)
        .ok_or_else(|| setting(field, "out-of-range"))
}

fn one_or_many(value: &Value) -> Result<Vec<&Value>, String> {
    match value {
        Value::Array(items) => Ok(items.iter().collect()),
        Value::Null => Err(MALFORMED.into()),
        other => Ok(vec![other]),
    }
}

fn name_list(value: &Value, field: &str) -> Result<Box<[Arc<str>]>, String> {
    let items = one_or_many(value)?;
    if items.is_empty() {
        return Err(setting(field, "empty"));
    }
    if items.len() > MAX_TARGETS {
        return Err(setting(field, "too-many"));
    }
    let mut names: Vec<Arc<str>> = Vec::with_capacity(items.len());
    for item in items {
        let name = text(item)?.trim();
        if name.is_empty() {
            return Err(setting(field, "empty"));
        }
        if name.len() > MAX_NAME_BYTES {
            return Err(setting(field, "too-long"));
        }
        if names.iter().any(|known| known.eq_ignore_ascii_case(name)) {
            return Err(setting(field, "duplicate"));
        }
        names.push(Arc::from(name));
    }
    Ok(names.into_boxed_slice())
}

fn target(value: &Value) -> Result<TargetSpec, String> {
    let Value::Object(object) = value else {
        return Err(MALFORMED.into());
    };
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "npc" | "npcId" | "attackers"))
    {
        return Err(MALFORMED.into());
    }
    match object.len() {
        0 => return Err(setting("target", "required")),
        1 => {}
        _ => return Err(setting("target", "ambiguous")),
    }
    if let Some(value) = object.get("npc") {
        return name_list(value, "target").map(TargetSpec::Names);
    }
    if let Some(value) = object.get("npcId") {
        let items = one_or_many(value)?;
        if items.is_empty() {
            return Err(setting("target", "empty"));
        }
        if items.len() > MAX_TARGETS {
            return Err(setting("target", "too-many"));
        }
        let mut ids = Vec::with_capacity(items.len());
        for item in items {
            let id = integer(item)?;
            let id = i32::try_from(id)
                .ok()
                .filter(|id| *id >= 0)
                .ok_or_else(|| setting("target", "invalid-id"))?;
            if ids.contains(&id) {
                return Err(setting("target", "duplicate"));
            }
            ids.push(id);
        }
        return Ok(TargetSpec::Ids(ids.into_boxed_slice()));
    }
    let attackers = object.get("attackers").expect("one known key");
    let (npcs, players) = match text(attackers)? {
        "npcs" => (true, false),
        "players" => (false, true),
        "any" => (true, true),
        _ => return Err(setting("target", "unsupported")),
    };
    Ok(TargetSpec::Attackers { npcs, players })
}

fn area_box(value: &Value) -> Result<[i32; 5], String> {
    let Value::Object(object) = value else {
        return Err(MALFORMED.into());
    };
    if object.len() != 5 {
        return Err(MALFORMED.into());
    }
    let field = |name: &str| -> Result<i64, String> {
        object
            .get(name)
            .ok_or(MALFORMED.to_string())
            .and_then(integer)
    };
    let (min_x, min_z, max_x, max_z, level) = (
        field("min_x")?,
        field("min_z")?,
        field("max_x")?,
        field("max_z")?,
        field("level")?,
    );
    let coordinate = |value: i64| (0..=16383).contains(&value);
    if ![min_x, min_z, max_x, max_z].into_iter().all(coordinate)
        || !(0..=3).contains(&level)
        || min_x > max_x
        || min_z > max_z
    {
        return Err(setting("area", "invalid-box"));
    }
    Ok([
        min_x as i32,
        min_z as i32,
        max_x as i32,
        max_z as i32,
        level as i32,
    ])
}

impl CombatSessionRequest {
    /// Decode one plain request object. A non-object is `invalid-args`; an
    /// unknown key, a wrong type or a non-integer number is
    /// `invalid-settings`; a semantic failure is
    /// `invalid-setting:<field>:<code>`.
    pub fn from_value(value: &Value) -> Result<Self, String> {
        let Value::Object(object) = value else {
            return Err("invalid-args".into());
        };
        Self::from_object(object)
    }

    fn from_object(object: &Map<String, Value>) -> Result<Self, String> {
        if object.keys().any(|key| !KEYS.contains(&key.as_str())) {
            return Err(MALFORMED.into());
        }
        let mut request = Self::default();
        let get = |key: &str| object.get(key).filter(|value| !value.is_null());
        request.target = match get("target") {
            Some(value) => target(value)?,
            None => return Err(setting("target", "required")),
        };
        if let Some(value) = get("style") {
            request.style = match text(value)? {
                "melee" => Style::Melee,
                "ranged" => Style::Ranged,
                "magic" => Style::Mage,
                _ => return Err(setting("style", "unsupported")),
            };
        }
        let npc_target = !matches!(request.target, TargetSpec::Attackers { .. });
        if let Some(value) = get("pick") {
            request.pick = match text(value)? {
                "nearest" => Pick::Nearest,
                "random" => Pick::Random,
                "lowestHealth" => Pick::LowestHealth,
                _ => return Err(setting("pick", "unsupported")),
            };
            if !npc_target {
                return Err(setting("pick", "requires-npc-target"));
            }
        }
        if let Some(value) = get("notTargetingOthers") {
            request.not_targeting_others = boolean(value)?;
            if !npc_target {
                return Err(setting("notTargetingOthers", "requires-npc-target"));
            }
        }
        if let Some(value) = get("area") {
            let boxes = one_or_many(value)?;
            if boxes.is_empty() {
                return Err(setting("area", "empty"));
            }
            if boxes.len() > MAX_AREA_BOXES {
                return Err(setting("area", "too-many"));
            }
            request.area = boxes
                .into_iter()
                .map(area_box)
                .collect::<Result<Vec<_>, _>>()?
                .into_boxed_slice();
        }
        if let Some(value) = get("radius") {
            request.radius = ranged_u8(value, "radius")?;
        }
        if let Some(value) = get("lostRadius") {
            request.lost_radius = ranged_u8(value, "lostRadius")?;
        }
        if let Some(value) = get("meleeMode") {
            request.melee_mode = Some(match text(value)? {
                "accurate" => MeleeMode::Accurate,
                "aggressive" => MeleeMode::Aggressive,
                "defensive" => MeleeMode::Defensive,
                "controlled" => MeleeMode::Controlled,
                _ => return Err(setting("meleeMode", "unsupported")),
            });
            if request.style != Style::Melee {
                return Err(setting("meleeMode", "requires-melee"));
            }
        }
        if let Some(value) = get("rangedMode") {
            request.ranged_mode = match text(value)? {
                "accurate" => RangedMode::Accurate,
                "rapid" => RangedMode::Rapid,
                "longRange" => RangedMode::LongRange,
                _ => return Err(setting("rangedMode", "unsupported")),
            };
            if request.style != Style::Ranged {
                return Err(setting("rangedMode", "requires-ranged"));
            }
        }
        if let Some(value) = get("spells") {
            let Value::Array(items) = value else {
                return Err(MALFORMED.into());
            };
            if items.is_empty() {
                return Err(setting("spells", "empty"));
            }
            if items.len() > MAX_SPELLS {
                return Err(setting("spells", "too-many"));
            }
            request.spells = Some(name_list(value, "spells")?);
            if request.style != Style::Mage {
                return Err(setting("spells", "requires-magic"));
            }
        }
        if let Some(value) = get("fallbackSpells") {
            request.fallback_spells = boolean(value)?;
            if request.style != Style::Mage {
                return Err(setting("fallbackSpells", "requires-magic"));
            }
        }
        for (key, slot) in [
            ("prayer", &mut request.prayer),
            ("food", &mut request.food),
            ("potions", &mut request.potions),
            ("retaliate", &mut request.retaliate),
        ] {
            if let Some(value) = get(key) {
                *slot = boolean(value)?;
            }
        }
        if let Some(value) = get("budgetTicks") {
            let ticks = integer(value)?;
            request.budget_ticks = u16::try_from(ticks)
                .ok()
                .filter(|ticks| *ticks != 0)
                .ok_or_else(|| setting("budgetTicks", "out-of-range"))?;
        }
        Ok(request)
    }

    /// The invariants every admitted request holds; the host re-checks a
    /// decoded wire request with this before it resolves it.
    pub fn check(&self) -> Result<(), String> {
        let names_ok = |names: &[Arc<str>], max: usize| {
            !names.is_empty()
                && names.len() <= max
                && names.iter().enumerate().all(|(index, name)| {
                    !name.trim().is_empty()
                        && name.len() <= MAX_NAME_BYTES
                        && !names[..index]
                            .iter()
                            .any(|other| other.eq_ignore_ascii_case(name))
                })
        };
        let target_ok = match &self.target {
            TargetSpec::Names(names) => names_ok(names, MAX_TARGETS),
            TargetSpec::Ids(ids) => {
                !ids.is_empty()
                    && ids.len() <= MAX_TARGETS
                    && ids
                        .iter()
                        .enumerate()
                        .all(|(index, id)| *id >= 0 && !ids[..index].contains(id))
            }
            TargetSpec::Attackers { npcs, players } => *npcs || *players,
        };
        if !target_ok {
            return Err(setting("target", "invalid"));
        }
        let box_ok = |&[min_x, min_z, max_x, max_z, level]: &[i32; 5]| {
            [min_x, min_z, max_x, max_z]
                .iter()
                .all(|value| (0..=16383).contains(value))
                && (0..=3).contains(&level)
                && min_x <= max_x
                && min_z <= max_z
        };
        if self.area.len() > MAX_AREA_BOXES || !self.area.iter().all(box_ok) {
            return Err(setting("area", "invalid-box"));
        }
        if self.radius == 0 {
            return Err(setting("radius", "out-of-range"));
        }
        if self.lost_radius == 0 {
            return Err(setting("lostRadius", "out-of-range"));
        }
        if self.budget_ticks == 0 {
            return Err(setting("budgetTicks", "out-of-range"));
        }
        if self.melee_mode.is_some() && self.style != Style::Melee {
            return Err(setting("meleeMode", "requires-melee"));
        }
        if let Some(spells) = &self.spells {
            if self.style != Style::Mage || !names_ok(spells, MAX_SPELLS) {
                return Err(setting("spells", "invalid"));
            }
        }
        if self.fallback_spells && self.style != Style::Mage {
            return Err(setting("fallbackSpells", "requires-magic"));
        }
        Ok(())
    }

    /// Resolve names against the slot's selected game data into the native
    /// request. A refusal is the keyed setting error the host reports as
    /// `invalid-setting:<field>:<code>`.
    pub fn resolve(&self, tables: &CombatTables) -> Result<CombatRequest, ConfigError> {
        let target = match &self.target {
            TargetSpec::Names(names) => {
                let mut types: Vec<i32> = Vec::with_capacity(names.len());
                for name in names.iter() {
                    let before = types.len();
                    resolve_name(tables, name, &mut types);
                    if types.len() == before {
                        return Err(refuse("target", "unknown-npc"));
                    }
                }
                npc_target(tables, types, self)?
            }
            TargetSpec::Ids(ids) => {
                if ids.iter().any(|id| tables.npc(*id).is_none()) {
                    return Err(refuse("target", "unknown-npc"));
                }
                npc_target(tables, ids.to_vec(), self)?
            }
            TargetSpec::Attackers { npcs, players } => Target::Attacker {
                npcs: *npcs,
                players: *players,
            },
        };
        let spells = match &self.spells {
            None => None,
            Some(names) => Some(
                names
                    .iter()
                    .map(|name| {
                        let index = crate::combat::style::magic::spell_index(tables, name)
                            .ok_or_else(|| refuse("spells", "unknown-spell"))?;
                        Ok(SpellRef {
                            alias: Arc::from(
                                tables.selected().spells()[usize::from(index)]
                                    .source_row
                                    .as_str(),
                            ),
                        })
                    })
                    .collect::<Result<Arc<[SpellRef]>, ConfigError>>()?,
            ),
        };
        if self.style == Style::Mage && tables.selected().spells().is_empty() {
            return Err(refuse("style", "unavailable"));
        }
        Ok(CombatRequest {
            target,
            style: self.style,
            melee_mode: self.melee_mode,
            ranged_style: self.ranged_mode,
            spells,
            fallback_spells: self.fallback_spells,
            search_bounds: (!self.area.is_empty()).then(|| {
                self.area
                    .iter()
                    .map(|&[min_x, min_z, max_x, max_z, level]| SceneRegionInput {
                        min_x,
                        min_z,
                        max_x,
                        max_z,
                        level,
                    })
                    .collect()
            }),
            engage_radius: self.radius,
            lost_radius: self.lost_radius,
            budget_ticks: self.budget_ticks,
            allow: Allowances {
                prayer: self.prayer,
                food: self.food,
                potions: self.potions,
                ..Allowances::default()
            },
            retaliate: self.retaliate,
            ..CombatRequest::default()
        })
    }
}

/// A config name matches exactly; otherwise every npc with that display name
/// (ASCII case-insensitive) that has combat facts.
fn resolve_name(tables: &CombatTables, name: &str, types: &mut Vec<i32>) {
    let selected = tables.selected();
    if let Some(row) = selected.npc_by_config(name) {
        if !types.contains(&row.id) {
            types.push(row.id);
        }
        return;
    }
    let Some(names) = selected.npc_names() else {
        return;
    };
    for row in &names.rows {
        if row
            .display
            .as_deref()
            .is_some_and(|display| display.eq_ignore_ascii_case(name))
            && tables.npc_fact(row.id).is_some_and(|fact| fact.attackable)
            && !types.contains(&row.id)
        {
            types.push(row.id);
        }
    }
}

fn npc_target(
    tables: &CombatTables,
    types: Vec<i32>,
    request: &CombatSessionRequest,
) -> Result<Target, ConfigError> {
    if types.len() > MAX_TARGETS {
        return Err(refuse("target", "too-many"));
    }
    if !types
        .iter()
        .any(|id| tables.npc_fact(*id).is_some_and(|fact| fact.attackable))
    {
        return Err(refuse("target", "unattackable"));
    }
    Ok(Target::Npc {
        types: Arc::from(types),
        pick: request.pick,
        not_targeting_others: request.not_targeting_others,
    })
}

fn refuse(field: &str, code: &str) -> ConfigError {
    ConfigError::new(field, code, "combat request")
}

/// How the Combat machine reported a settled fight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportEnd {
    Killed,
    TargetGone,
    NoTarget,
    Budget,
    Died,
    Aborted,
}

impl ReportEnd {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Killed => "killed",
            Self::TargetGone => "target-gone",
            Self::NoTarget => "no-target",
            Self::Budget => "budget",
            Self::Died => "died",
            Self::Aborted => "aborted",
        }
    }

    pub fn code(self) -> u8 {
        match self {
            Self::Killed => 1,
            Self::TargetGone => 2,
            Self::NoTarget => 3,
            Self::Budget => 4,
            Self::Died => 5,
            Self::Aborted => 6,
        }
    }

    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            1 => Self::Killed,
            2 => Self::TargetGone,
            3 => Self::NoTarget,
            4 => Self::Budget,
            5 => Self::Died,
            6 => Self::Aborted,
            _ => return None,
        })
    }
}

fn abort_reason(reason: AbortReason) -> &'static str {
    match reason {
        AbortReason::Unprotected(Unprotected::NoFood) => "no-food",
        AbortReason::Unprotected(Unprotected::Dragonfire) => "dragonfire",
        AbortReason::Unprotected(Unprotected::NoAmmo) => "no-ammo",
        AbortReason::Unprotected(Unprotected::NoRunes) => "no-runes",
        AbortReason::Unattackable => "unattackable",
        AbortReason::SafespotBroken => "safespot-broken",
        AbortReason::Retreated => "retreated",
        AbortReason::RetreatFailed => "retreat-failed",
        AbortReason::PrepFailed(PrepItem::Weapon) => "prep-failed:weapon",
        AbortReason::PrepFailed(PrepItem::Shield) => "prep-failed:shield",
        AbortReason::PrepFailed(PrepItem::Ammo) => "prep-failed:ammo",
        AbortReason::PrepFailed(PrepItem::Staff) => "prep-failed:staff",
        AbortReason::PrepFailed(PrepItem::Arm) => "prep-failed:arm",
        AbortReason::PrepReadiness(PrepReadiness::CombatRootMissing) => {
            "prep-readiness:combat-root-missing"
        }
        AbortReason::PrepReadiness(PrepReadiness::CombatRootWrong) => {
            "prep-readiness:combat-root-wrong"
        }
        AbortReason::PrepReadiness(PrepReadiness::StyleEchoMissing) => {
            "prep-readiness:style-echo-missing"
        }
        AbortReason::Unresponsive => "unresponsive",
    }
}

/// The public, compact view of one [`CombatReport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatSummary {
    pub end: ReportEnd,
    /// The abort reason; only for [`ReportEnd::Aborted`].
    pub reason: Option<Arc<str>>,
    pub npc_type: i32,
    pub ticks: u16,
    pub swings: u16,
    pub casts: u16,
    pub damage_taken: u16,
    pub food: u8,
    pub prayer_doses: u8,
    pub boost_doses: u8,
    pub antifire_doses: u8,
    pub protect_switches: u8,
}

impl From<&CombatReport> for CombatSummary {
    fn from(report: &CombatReport) -> Self {
        use crate::combat::CombatEnd as End;
        let (end, reason) = match report.end {
            End::Killed => (ReportEnd::Killed, None),
            End::TargetGone => (ReportEnd::TargetGone, None),
            End::NoTarget => (ReportEnd::NoTarget, None),
            End::Budget => (ReportEnd::Budget, None),
            End::Died => (ReportEnd::Died, None),
            End::Aborted(reason) => (ReportEnd::Aborted, Some(Arc::from(abort_reason(reason)))),
        };
        Self {
            end,
            reason,
            npc_type: report.engaged_npc_type,
            ticks: report.ticks,
            swings: report.swings,
            casts: report.casts,
            damage_taken: report.damage_taken,
            food: report.food,
            prayer_doses: report.prayer_doses,
            boost_doses: report.boost_doses,
            antifire_doses: report.antifire_doses,
            protect_switches: report.protect_switches,
        }
    }
}

/// Why a live fight ended before Combat settled it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptCause {
    Pause,
    UserInput,
    Died,
    Reconnect,
}

impl InterruptCause {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::UserInput => "user-input",
            Self::Died => "died",
            Self::Reconnect => "reconnect",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "pause" => Self::Pause,
            "user-input" => Self::UserInput,
            "died" => Self::Died,
            "reconnect" => Self::Reconnect,
            _ => return None,
        })
    }
}

/// One retained terminal outcome, correlated to the isolate's request token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CombatEnd {
    Fought { token: u64, report: CombatSummary },
    Stopped { token: u64 },
    Interrupted { token: u64, cause: InterruptCause },
    Refused { token: u64, reason: Arc<str> },
    Failed { token: u64, reason: Arc<str> },
}

impl CombatEnd {
    pub fn token(&self) -> u64 {
        match self {
            Self::Fought { token, .. }
            | Self::Stopped { token }
            | Self::Interrupted { token, .. }
            | Self::Refused { token, .. }
            | Self::Failed { token, .. } => *token,
        }
    }
}

impl From<CombatEnd> for Value {
    fn from(end: CombatEnd) -> Self {
        match end {
            CombatEnd::Fought { token, report } => serde_json::json!({
                "end": "fought",
                "token": token,
                "report": {
                    "end": report.end.as_str(),
                    "reason": report.reason.as_deref(),
                    "npcType": report.npc_type,
                    "ticks": report.ticks,
                    "swings": report.swings,
                    "casts": report.casts,
                    "damageTaken": report.damage_taken,
                    "food": report.food,
                    "prayerDoses": report.prayer_doses,
                    "boostDoses": report.boost_doses,
                    "antifireDoses": report.antifire_doses,
                    "protectSwitches": report.protect_switches,
                },
            }),
            CombatEnd::Stopped { token } => serde_json::json!({
                "end": "stopped",
                "token": token,
            }),
            CombatEnd::Interrupted { token, cause } => serde_json::json!({
                "end": "interrupted",
                "token": token,
                "cause": cause.as_str(),
            }),
            CombatEnd::Refused { token, reason } => serde_json::json!({
                "end": "refused",
                "token": token,
                "reason": reason,
            }),
            CombatEnd::Failed { token, reason } => serde_json::json!({
                "end": "failed",
                "token": token,
                "reason": reason,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::selected::ClientRevision;
    use serde_json::json;

    fn decode(value: Value) -> Result<CombatSessionRequest, String> {
        CombatSessionRequest::from_value(&value)
    }

    fn tables() -> Arc<CombatTables> {
        CombatTables::build(api::game_data::for_revision(ClientRevision::R289).unwrap()).unwrap()
    }

    #[test]
    fn defaults_are_the_native_request_defaults() {
        let request = decode(json!({ "target": { "npc": "cow" } })).unwrap();
        let native = CombatRequest::default();
        assert_eq!(request.radius, native.engage_radius);
        assert_eq!(request.lost_radius, native.lost_radius);
        assert_eq!(request.budget_ticks, native.budget_ticks);
        assert_eq!(request.pick, Pick::Nearest);
        assert!(request.not_targeting_others && request.prayer && request.food);
        assert!(request.potions && request.retaliate);
        assert_eq!(request.style, Style::Melee);
        assert_eq!(request.check(), Ok(()));
    }

    #[test]
    fn every_key_decodes_and_passes_the_shared_invariants() {
        let request = decode(json!({
            "target": { "npcId": [81, 397] },
            "pick": "random",
            "notTargetingOthers": false,
            "area": { "min_x": 3253, "min_z": 3255, "max_x": 3265, "max_z": 3297, "level": 0 },
            "radius": 30,
            "lostRadius": 40,
            "style": "magic",
            "spells": ["wind_strike", "Water Strike"],
            "fallbackSpells": true,
            "prayer": false,
            "food": false,
            "potions": false,
            "retaliate": false,
            "budgetTicks": 65535
        }))
        .unwrap();
        assert_eq!(request.target, TargetSpec::Ids(Box::new([81, 397])));
        assert_eq!(request.area.as_ref(), &[[3253, 3255, 3265, 3297, 0]]);
        assert_eq!(request.style, Style::Mage);
        assert_eq!(request.spells.as_ref().unwrap().len(), 2);
        assert_eq!(request.budget_ticks, u16::MAX);
        assert!(!request.prayer && !request.food && !request.potions && !request.retaliate);
        assert_eq!(request.check(), Ok(()));
        let attackers = decode(json!({ "target": { "attackers": "players" } })).unwrap();
        assert_eq!(
            attackers.target,
            TargetSpec::Attackers {
                npcs: false,
                players: true
            }
        );
    }

    #[test]
    fn malformed_and_semantic_refusals_use_the_gather_vocabulary() {
        let cases = [
            (json!(5), "invalid-args"),
            (json!([]), "invalid-args"),
            (
                json!({ "target": { "npc": "cow" }, "tactic": "open" }),
                "invalid-settings",
            ),
            (
                json!({ "target": { "npc": "cow" }, "prayerMode": "hold" }),
                "invalid-settings",
            ),
            (
                json!({ "target": { "npc": "cow" }, "radius": 1.5 }),
                "invalid-settings",
            ),
            (json!({ "target": { "npc": 7 } }), "invalid-settings"),
            (
                json!({ "target": { "npc": "cow", "pick": "x" } }),
                "invalid-settings",
            ),
            (json!({ "target": "cow" }), "invalid-settings"),
            (
                json!({ "target": { "npc": "cow" }, "prayer": 1 }),
                "invalid-settings",
            ),
            (json!({}), "invalid-setting:target:required"),
            (json!({ "target": {} }), "invalid-setting:target:required"),
            (
                json!({ "target": { "npc": "cow", "npcId": 81 } }),
                "invalid-setting:target:ambiguous",
            ),
            (
                json!({ "target": { "npc": [] } }),
                "invalid-setting:target:empty",
            ),
            (
                json!({ "target": { "npc": " " } }),
                "invalid-setting:target:empty",
            ),
            (
                json!({ "target": { "npc": ["cow", "COW"] } }),
                "invalid-setting:target:duplicate",
            ),
            (
                json!({ "target": { "npcId": -1 } }),
                "invalid-setting:target:invalid-id",
            ),
            (
                json!({ "target": { "attackers": "everyone" } }),
                "invalid-setting:target:unsupported",
            ),
            (
                json!({ "target": { "attackers": "npcs" }, "pick": "nearest" }),
                "invalid-setting:pick:requires-npc-target",
            ),
            (
                json!({ "target": { "npc": "cow" }, "area": [] }),
                "invalid-setting:area:empty",
            ),
            (
                json!({ "target": { "npc": "cow" }, "area": { "min_x": 9, "min_z": 0, "max_x": 1, "max_z": 1, "level": 0 } }),
                "invalid-setting:area:invalid-box",
            ),
            (
                json!({ "target": { "npc": "cow" }, "area": { "min_x": 0, "min_z": 0, "max_x": 1, "max_z": 1, "level": 4 } }),
                "invalid-setting:area:invalid-box",
            ),
            (
                json!({ "target": { "npc": "cow" }, "radius": 0 }),
                "invalid-setting:radius:out-of-range",
            ),
            (
                json!({ "target": { "npc": "cow" }, "lostRadius": 256 }),
                "invalid-setting:lostRadius:out-of-range",
            ),
            (
                json!({ "target": { "npc": "cow" }, "style": "kick" }),
                "invalid-setting:style:unsupported",
            ),
            (
                json!({ "target": { "npc": "cow" }, "style": "ranged", "meleeMode": "accurate" }),
                "invalid-setting:meleeMode:requires-melee",
            ),
            (
                json!({ "target": { "npc": "cow" }, "rangedMode": "rapid" }),
                "invalid-setting:rangedMode:requires-ranged",
            ),
            (
                json!({ "target": { "npc": "cow" }, "spells": ["wind_strike"] }),
                "invalid-setting:spells:requires-magic",
            ),
            (
                json!({ "target": { "npc": "cow" }, "style": "magic", "spells": [] }),
                "invalid-setting:spells:empty",
            ),
            (
                json!({ "target": { "npc": "cow" }, "fallbackSpells": true }),
                "invalid-setting:fallbackSpells:requires-magic",
            ),
            (
                json!({ "target": { "npc": "cow" }, "budgetTicks": 70000 }),
                "invalid-setting:budgetTicks:out-of-range",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(
                decode(input.clone()).err().as_deref(),
                Some(expected),
                "{input}"
            );
        }
    }

    #[test]
    fn host_check_rejects_a_forged_wire_request() {
        let valid = decode(json!({ "target": { "npc": "cow" } })).unwrap();
        for forged in [
            CombatSessionRequest {
                radius: 0,
                ..valid.clone()
            },
            CombatSessionRequest {
                target: TargetSpec::Names(Box::new([])),
                ..valid.clone()
            },
            CombatSessionRequest {
                melee_mode: Some(MeleeMode::Accurate),
                style: Style::Ranged,
                ..valid.clone()
            },
            CombatSessionRequest {
                area: Box::new([[0, 0, 1, 1, 9]]),
                ..valid.clone()
            },
            CombatSessionRequest {
                target: TargetSpec::Attackers {
                    npcs: false,
                    players: false,
                },
                ..valid.clone()
            },
        ] {
            assert!(forged.check().is_err(), "{forged:?}");
        }
    }

    #[test]
    fn names_resolve_against_selected_content() {
        let tables = tables();
        let cow = tables.selected().npc_by_config("cow").unwrap().id;
        let native = decode(json!({ "target": { "npc": "cow" } }))
            .unwrap()
            .resolve(&tables)
            .unwrap();
        let Target::Npc { types, .. } = &native.target else {
            panic!("npc target");
        };
        assert_eq!(types.as_ref(), &[cow]);

        // A display name takes every attackable npc so named.
        let native = decode(json!({ "target": { "npc": "Chicken" } }))
            .unwrap()
            .resolve(&tables)
            .unwrap();
        let Target::Npc { types, .. } = &native.target else {
            panic!("npc target");
        };
        assert!(!types.is_empty());
        for id in types.iter() {
            let row = tables.npc(*id).unwrap();
            assert!(row
                .display
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case("chicken")));
            assert!(tables.npc_fact(*id).unwrap().attackable);
        }

        let refusal = |value: Value| {
            let error = decode(value).unwrap().resolve(&tables).unwrap_err();
            format!("invalid-setting:{}:{}", error.field, error.code)
        };
        assert_eq!(
            refusal(json!({ "target": { "npc": "no_such_npc" } })),
            "invalid-setting:target:unknown-npc"
        );
        assert_eq!(
            refusal(json!({ "target": { "npcId": 999_999 } })),
            "invalid-setting:target:unknown-npc"
        );
        let unattackable = tables
            .selected()
            .npc_names()
            .unwrap()
            .rows
            .iter()
            .find(|row| tables.npc_fact(row.id).is_some_and(|fact| !fact.attackable))
            .unwrap()
            .id;
        assert_eq!(
            refusal(json!({ "target": { "npcId": unattackable } })),
            "invalid-setting:target:unattackable"
        );
        assert_eq!(
            refusal(
                json!({ "target": { "npc": "cow" }, "style": "magic", "spells": ["no_such_spell"] })
            ),
            "invalid-setting:spells:unknown-spell"
        );
    }

    #[test]
    fn every_request_field_reaches_the_native_request() {
        let tables = tables();
        let native = decode(json!({
            "target": { "npc": "cow" },
            "pick": "lowestHealth",
            "notTargetingOthers": false,
            "area": [{ "min_x": 3253, "min_z": 3255, "max_x": 3265, "max_z": 3297, "level": 0 }],
            "radius": 9,
            "lostRadius": 11,
            "style": "magic",
            "spells": ["wind_strike"],
            "fallbackSpells": true,
            "prayer": false,
            "food": false,
            "potions": false,
            "retaliate": false,
            "budgetTicks": 77
        }))
        .unwrap()
        .resolve(&tables)
        .unwrap();
        let Target::Npc {
            pick,
            not_targeting_others,
            ..
        } = &native.target
        else {
            panic!("npc target");
        };
        assert_eq!(*pick, Pick::LowestHealth);
        assert!(!not_targeting_others);
        let bounds = native.search_bounds.as_deref().unwrap();
        assert_eq!(
            (
                bounds[0].min_x,
                bounds[0].min_z,
                bounds[0].max_x,
                bounds[0].max_z
            ),
            (3253, 3255, 3265, 3297)
        );
        assert_eq!((native.engage_radius, native.lost_radius), (9, 11));
        assert_eq!(native.style, Style::Mage);
        assert_eq!(
            native.spells.as_deref().unwrap()[0].alias.as_ref(),
            "magic_spell_wind_strike"
        );
        assert!(native.fallback_spells);
        assert!(!native.allow.prayer && !native.allow.food && !native.allow.potions);
        assert!(native.allow.equipment && native.allow.retaliate_toggle);
        assert!(!native.retaliate);
        assert_eq!(native.budget_ticks, 77);
        assert_eq!(native.tactic, crate::combat::Tactic::Open);
        assert_eq!(native.prayer_mode, crate::combat::PrayerMode::Hold);
    }

    #[test]
    fn reports_map_to_public_ends_and_abort_reasons() {
        let mut report = CombatReport {
            end: crate::combat::CombatEnd::Aborted(AbortReason::PrepFailed(PrepItem::Ammo)),
            evidence: api::quest_progress::EvidenceStamp {
                run: api::selected::RunKey {
                    slot: 1,
                    run: 1,
                    session: 1,
                },
                tick: 1,
                sequence: 1,
            },
            engaged: None,
            engaged_npc_type: 81,
            ticks: 12,
            swings: 4,
            casts: 0,
            damage_taken: 2,
            food: 1,
            prayer_doses: 0,
            boost_doses: 0,
            antifire_doses: 0,
            hits_while_protected: 0,
            protect_switches: 1,
            intruders: 0,
            ammo_pickups: 0,
            restorations: 0,
            locked_ticks: 0,
            multi_op_plans: 0,
            melee_mode_fallback: None,
            flick_resets: 0,
            flick_misses: 0,
            flick_fallback: false,
        };
        let summary = CombatSummary::from(&report);
        assert_eq!(summary.end, ReportEnd::Aborted);
        assert_eq!(summary.reason.as_deref(), Some("prep-failed:ammo"));
        let value = Value::from(CombatEnd::Fought {
            token: 4,
            report: summary,
        });
        assert_eq!(
            value,
            json!({
                "end": "fought", "token": 4,
                "report": {
                    "end": "aborted", "reason": "prep-failed:ammo", "npcType": 81,
                    "ticks": 12, "swings": 4, "casts": 0, "damageTaken": 2, "food": 1,
                    "prayerDoses": 0, "boostDoses": 0, "antifireDoses": 0, "protectSwitches": 1
                }
            })
        );
        for (readiness, expected) in [
            (
                PrepReadiness::CombatRootMissing,
                "prep-readiness:combat-root-missing",
            ),
            (
                PrepReadiness::CombatRootWrong,
                "prep-readiness:combat-root-wrong",
            ),
            (
                PrepReadiness::StyleEchoMissing,
                "prep-readiness:style-echo-missing",
            ),
        ] {
            report.end = crate::combat::CombatEnd::Aborted(AbortReason::PrepReadiness(readiness));
            assert_eq!(
                CombatSummary::from(&report).reason.as_deref(),
                Some(expected)
            );
        }

        report.end = crate::combat::CombatEnd::Killed;
        let summary = CombatSummary::from(&report);
        assert_eq!((summary.end, summary.reason), (ReportEnd::Killed, None));
        for end in [
            ReportEnd::Killed,
            ReportEnd::TargetGone,
            ReportEnd::NoTarget,
            ReportEnd::Budget,
            ReportEnd::Died,
            ReportEnd::Aborted,
        ] {
            assert_eq!(ReportEnd::from_code(end.code()), Some(end));
        }
        for cause in [
            InterruptCause::Pause,
            InterruptCause::UserInput,
            InterruptCause::Died,
            InterruptCause::Reconnect,
        ] {
            assert_eq!(InterruptCause::parse(cause.as_str()), Some(cause));
        }
        assert_eq!(
            Value::from(CombatEnd::Interrupted {
                token: 2,
                cause: InterruptCause::UserInput
            }),
            json!({ "end": "interrupted", "token": 2, "cause": "user-input" })
        );
    }

    #[test]
    fn request_and_terminal_stay_compact() {
        assert!(std::mem::size_of::<CombatSessionRequest>() <= 96);
        assert!(std::mem::size_of::<CombatEnd>() <= 64);
    }
}
