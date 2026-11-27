use super::super::fixtures::{
    self, FixtureDefinitions, SeedPolicy, MAX_COMMAND_BYTES, MAX_REASON_BYTES,
};
use api::game_data::SelectedGameData;
use api::scenario_observe::{AppliedEvent, DecodedDomainSelection, ScenarioObserveWatermark};
use api::snapshot::{GameSnapshot, ItemContainer};
use client::client::Client;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::Path,
};
use toml_edit::{Item, TableLike};

#[cfg(test)]
#[path = "seed_pack_tests.rs"]
mod tests;

const MAX_PACK_BYTES: u64 = 64 * 1024;
const MAX_OPERATIONS: usize = 64;
const MAX_GRANT_BYTES: usize = MAX_REASON_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameTable {
    Item,
    Varp,
    Varbit,
    Skill,
    Npc,
    Loc,
}

impl NameTable {
    pub(in crate::scenario_luau) const fn as_str(self) -> &'static str {
        match self {
            Self::Item => "item",
            Self::Varp => "varp",
            Self::Varbit => "varbit",
            Self::Skill => "skill",
            Self::Npc => "npc",
            Self::Loc => "loc",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedName {
    pub name: String,
    pub id: i32,
}

pub trait PackNameLookup {
    fn resolve(&self, table: NameTable, name: &str) -> Result<ResolvedName, String>;
}

/// Name lookup bound to the selected engine definitions and content tables.
pub struct SelectedPackNames<'a> {
    fixtures: &'a FixtureDefinitions,
    selected: &'a SelectedGameData,
}

impl<'a> SelectedPackNames<'a> {
    pub fn new(fixtures: &'a FixtureDefinitions, selected: &'a SelectedGameData) -> Self {
        Self { fixtures, selected }
    }
}

impl PackNameLookup for SelectedPackNames<'_> {
    fn resolve(&self, table: NameTable, name: &str) -> Result<ResolvedName, String> {
        self.fixtures.resolve_pack_name(self.selected, table, name)
    }
}

#[derive(Debug, Clone)]
pub struct SeedPack {
    sha256: String,
    operations: BTreeMap<String, PackOperation>,
}

#[derive(Debug, Clone)]
struct PackOperation {
    name: String,
    template: String,
    compiled_template: Template,
    params: BTreeMap<String, ParamKind>,
    effects: Vec<String>,
    precondition: Precondition,
    scope: Scope,
    confirmation: Confirmation,
}

#[derive(Debug, Clone)]
enum ParamKind {
    Name(NameTable),
    Int { min: i32, max: i32 },
    Skill,
    Tile,
}

#[derive(Debug, Clone)]
enum Precondition {
    None,
    FixtureIdle,
}

impl Precondition {
    const fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::FixtureIdle => "fixture-idle",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    OwnAccount,
    World,
}

impl Scope {
    const fn as_str(self) -> &'static str {
        match self {
            Self::OwnAccount => "own-account",
            Self::World => "world",
        }
    }
}

#[derive(Debug, Clone)]
struct Confirmation {
    reply: Option<String>,
    state: Vec<StateMatchSpec>,
}

impl Confirmation {
    fn label(&self) -> &'static str {
        if !self.state.is_empty() {
            "established-state"
        } else if self.reply.is_some() {
            "reply-seen"
        } else {
            "unconfirmed"
        }
    }

    fn rejects_confirmed_only(&self) -> bool {
        self.state.is_empty()
    }
}

#[derive(Debug, Clone)]
enum StateMatchSpec {
    ItemCount {
        item: String,
        container: ItemContainer,
        count: i32,
    },
    Stat {
        skill: String,
        base: i32,
        effective: i32,
        xp: i32,
    },
    Tile {
        x: i32,
        z: i32,
        level: i32,
    },
    Var {
        name: String,
        value: i32,
    },
}

#[derive(Debug, Clone)]
enum ResolvedStateMatch {
    ItemCount {
        item: String,
        item_id: i32,
        container: ItemContainer,
        count: i32,
    },
    Stat {
        skill: String,
        skill_id: i32,
        base: i32,
        effective: i32,
        xp: i32,
    },
    Tile {
        x: i32,
        z: i32,
        level: i32,
    },
    Var {
        name: String,
        varp_id: i32,
        value: i32,
    },
}

impl ResolvedStateMatch {
    fn matches(
        &self,
        client: &Client,
        snapshot: &GameSnapshot,
        baseline: &ScenarioObserveWatermark,
    ) -> Result<bool, String> {
        match self {
            Self::ItemCount {
                item_id,
                container,
                count,
                ..
            } => {
                let (rows, available) = match container {
                    ItemContainer::Inventory => (snapshot.inventory(), snapshot.inventory_ready()),
                    ItemContainer::Equipment => (snapshot.equipment(), snapshot.equipment_posted()),
                    _ => return Err("seed pack item match has an unsupported container".into()),
                };
                if !available {
                    return Ok(false);
                }
                let observed = rows
                    .iter()
                    .filter(|item| item.def.id == *item_id)
                    .try_fold(0_i64, |total, item| {
                        total
                            .checked_add(i64::from(item.count))
                            .ok_or_else(|| "seed pack item count overflowed".to_string())
                    })?;
                Ok(observed == i64::from(*count))
            }
            Self::Stat {
                skill,
                skill_id,
                base,
                effective,
                xp,
            } => Ok(snapshot.stats().iter().any(|stat| {
                stat.used
                    && stat.index == *skill_id
                    && stat.name == *skill
                    && stat.base == *base
                    && stat.effective == *effective
                    && stat.xp == *xp
            })),
            Self::Tile { x, z, level } => Ok(snapshot.tile() == Some((*x, *z, *level))),
            Self::Var { varp_id, value, .. } => {
                let current = snapshot
                    .varps()
                    .iter()
                    .find(|varp| varp.index == *varp_id)
                    .map(|varp| varp.value);
                let witness = client
                    .decoded_domain_varp_witness(*varp_id)
                    .map_err(|error| {
                        format!("seed pack varp observation unavailable: {error:?}")
                    })?;
                Ok(current == Some(*value)
                    && witness.is_some_and(|witness| {
                        witness.client_session == baseline.client_session
                            && witness.source_serial > baseline.source_serial
                            && witness.value == *value
                    }))
            }
        }
    }

    fn json(&self) -> Value {
        match self {
            Self::ItemCount {
                item,
                item_id,
                container,
                count,
            } => json!({
                "kind": "item_count",
                "item": item,
                "item_id": item_id,
                "container": match container {
                    ItemContainer::Inventory => "carried",
                    ItemContainer::Equipment => "worn",
                    _ => "unsupported",
                },
                "count": count,
            }),
            Self::Stat {
                skill,
                skill_id,
                base,
                effective,
                xp,
            } => json!({
                "kind": "stat",
                "skill": skill,
                "skill_id": skill_id,
                "base": base,
                "effective": effective,
                "xp": xp,
            }),
            Self::Tile { x, z, level } => json!({
                "kind": "tile",
                "x": x,
                "z": z,
                "level": level,
            }),
            Self::Var {
                name,
                varp_id,
                value,
            } => json!({
                "kind": "var",
                "name": name,
                "varp_id": varp_id,
                "value": value,
            }),
        }
    }
}

#[derive(Debug, Clone)]
struct Template {
    parts: Vec<TemplatePart>,
}

#[derive(Debug, Clone)]
enum TemplatePart {
    Literal(String),
    Parameter(String),
}

impl Template {
    fn expand(&self, values: &BTreeMap<String, String>) -> Result<String, String> {
        // R289/R377 engine/src/network/game/client/handler/ClientCheatHandler.ts:41-47 rejects lines over 80 bytes.
        let mut command = String::with_capacity(MAX_COMMAND_BYTES);
        for part in &self.parts {
            let text = match part {
                TemplatePart::Literal(text) => text.as_str(),
                TemplatePart::Parameter(name) => values
                    .get(name)
                    .map(String::as_str)
                    .ok_or_else(|| format!("seed pack template parameter {name} is unresolved"))?,
            };
            if command.len().saturating_add(text.len()) > MAX_COMMAND_BYTES {
                return Err("seed pack command exceeds the engine's 80-byte line limit".into());
            }
            command.push_str(text);
        }
        fixtures::checked_command(command)
    }
}

#[derive(Debug)]
struct ResolvedOperation {
    command: String,
    resolved_arguments: Value,
    state: Vec<ResolvedStateMatch>,
}

pub struct PackSeedRequest<'a> {
    pub operation_name: &'a str,
    pub args: &'a Map<String, Value>,
    pub why: &'a str,
    pub policy: SeedPolicy,
    pub watermark: ScenarioObserveWatermark,
    pub grant_identity: Option<&'a str>,
}

impl SeedPack {
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = read_pack(path)?;
        Self::from_bytes(&bytes)
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() as u64 > MAX_PACK_BYTES {
            return Err("seed pack exceeds the 64 KiB size limit".into());
        }
        let source =
            std::str::from_utf8(bytes).map_err(|_| "seed pack is not valid UTF-8".to_string())?;
        let document: toml_edit::DocumentMut = source
            .parse()
            .map_err(|error| format!("seed pack TOML is invalid: {error}"))?;
        let root = document.as_table();
        ensure_fields(
            root,
            &["version", "operations"],
            &["operations"],
            "seed pack",
        )?;
        if let Some(version) = root.get("version") {
            let version = version
                .as_integer()
                .ok_or("seed pack version must be an integer")?;
            if version != 1 {
                return Err("seed pack version is not supported".into());
            }
        }
        let operation_item = root
            .get("operations")
            .ok_or("seed pack is missing operations")?;
        let operation_count = operation_item
            .as_array_of_tables()
            .map(|tables| tables.len())
            .or_else(|| operation_item.as_array().map(|array| array.len()))
            .ok_or("seed pack operations must be an array of tables")?;
        if operation_count == 0 || operation_count > MAX_OPERATIONS {
            return Err("seed pack must declare 1 through 64 operations".into());
        }

        let mut operations = BTreeMap::new();
        if let Some(tables) = operation_item.as_array_of_tables() {
            for table in tables.iter() {
                insert_operation(&mut operations, table)?;
            }
        } else if let Some(array) = operation_item.as_array() {
            for value in array.iter() {
                let table = value
                    .as_inline_table()
                    .ok_or("seed pack operations must contain only tables")?;
                insert_operation(&mut operations, table)?;
            }
        }
        Ok(Self {
            sha256: format!("{:x}", Sha256::digest(bytes)),
            operations,
        })
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn validate_admission(
        &self,
        confirmed_only: bool,
        grant_identity: Option<&str>,
    ) -> Result<(), String> {
        if let Some(grant) = grant_identity {
            validate_grant_identity(grant)?;
        }
        for operation in self.operations.values() {
            if operation.scope == Scope::World && grant_identity.is_none() {
                return Err(format!(
                    "world-scope seed pack operation {:?} requires a shell grant identity",
                    operation.name
                ));
            }
            if confirmed_only && operation.confirmation.rejects_confirmed_only() {
                return Err(format!(
                    "confirmed-only scenario refuses seed pack operation {:?} with {} confirmation",
                    operation.name,
                    operation.confirmation.label()
                ));
            }
        }
        Ok(())
    }

    /// Resolve an operation before arming the observer. The returned selection
    /// includes only typed varp state checks, which need a decoded source witness.
    pub fn selection(
        &self,
        operation_name: &str,
        args: &Map<String, Value>,
        names: &dyn PackNameLookup,
    ) -> Result<DecodedDomainSelection, String> {
        let operation = self
            .operations
            .get(operation_name)
            .ok_or_else(|| format!("unknown seed pack operation {operation_name:?}"))?;
        let resolved = Self::resolve_operation(operation, args, names)?;
        let mut selection = DecodedDomainSelection::new();
        for matcher in resolved.state {
            if let ResolvedStateMatch::Var { varp_id, .. } = matcher {
                selection = selection.varp(varp_id);
            }
        }
        Ok(selection)
    }

    pub fn start(
        &self,
        client: &mut Client,
        snapshot: &GameSnapshot,
        names: &dyn PackNameLookup,
        request: PackSeedRequest<'_>,
    ) -> Result<PendingPackSeed, String> {
        let PackSeedRequest {
            operation_name,
            args,
            why,
            policy,
            watermark,
            grant_identity,
        } = request;
        fixtures::validate_reason(why)?;
        fixtures::require_ingame_snapshot(snapshot)?;
        let operation = self
            .operations
            .get(operation_name)
            .ok_or_else(|| format!("unknown seed pack operation {operation_name:?}"))?;
        if policy == SeedPolicy::ConfirmedOnly && operation.confirmation.rejects_confirmed_only() {
            return Err(format!(
                "confirmed-only scenario refuses seed pack operation {:?} with {} confirmation",
                operation.name,
                operation.confirmation.label()
            ));
        }
        if operation.scope == Scope::World && grant_identity.is_none() {
            return Err(format!(
                "world-scope seed pack operation {:?} requires a shell grant identity",
                operation.name
            ));
        }
        if let Some(grant) = grant_identity {
            validate_grant_identity(grant)?;
        }
        if matches!(operation.precondition, Precondition::FixtureIdle) {
            fixtures::require_fixture_idle(snapshot)?;
        }
        let resolved = Self::resolve_operation(operation, args, names)?;
        let chat_sequence = fixtures::chat_head_sequence(snapshot);
        fixtures::send_cheat_once(client, &resolved.command)?;

        Ok(PendingPackSeed {
            operation: operation.clone(),
            why: why.to_owned().into_boxed_str(),
            pack_sha256: self.sha256.clone().into_boxed_str(),
            command: resolved.command,
            resolved_arguments: resolved.resolved_arguments,
            resolved_state: resolved.state,
            grant_identity: grant_identity.map(str::to_owned),
            baseline: watermark,
            start_tick: snapshot.tick(),
            chat_sequence,
            tick_budget: fixtures::DEFAULT_OBSERVED_TICK_BUDGET,
            reply: None,
        })
    }

    fn resolve_operation(
        operation: &PackOperation,
        args: &Map<String, Value>,
        names: &dyn PackNameLookup,
    ) -> Result<ResolvedOperation, String> {
        for name in operation.params.keys() {
            if !args.contains_key(name) {
                return Err(format!("missing seed pack argument {name:?}"));
            }
        }
        if let Some(name) = args
            .keys()
            .find(|name| !operation.params.contains_key(*name))
        {
            return Err(format!("unexpected seed pack argument {name:?}"));
        }

        let mut command_values = BTreeMap::new();
        let mut argument_values = Map::new();
        for (name, kind) in &operation.params {
            let value = args
                .get(name)
                .ok_or_else(|| format!("missing seed pack argument {name:?}"))?;
            let (rendered, evidence) = match kind {
                ParamKind::Name(table) => {
                    let input = value.as_str().ok_or_else(|| {
                        format!("seed pack argument {name:?} must be a name string")
                    })?;
                    fixtures::validate_name(input, table.as_str())?;
                    let resolved = names.resolve(*table, input)?;
                    validate_resolved_name(&resolved, *table)?;
                    (
                        resolved.name.clone(),
                        json!({
                            "kind": "name",
                            "table": table.as_str(),
                            "name": resolved.name,
                            "id": resolved.id,
                        }),
                    )
                }
                ParamKind::Int { min, max } => {
                    let number = integer_value(value, *min, *max, name)?;
                    (number.to_string(), json!({"kind": "int", "value": number}))
                }
                ParamKind::Skill => {
                    let input = value.as_str().ok_or_else(|| {
                        format!("seed pack argument {name:?} must be a skill name string")
                    })?;
                    fixtures::validate_name(input, "skill")?;
                    let resolved = names.resolve(NameTable::Skill, input)?;
                    validate_resolved_name(&resolved, NameTable::Skill)?;
                    (
                        resolved.name.clone(),
                        json!({
                            "kind": "skill",
                            "name": resolved.name,
                            "id": resolved.id,
                        }),
                    )
                }
                ParamKind::Tile => {
                    let tile = tile_value(value, name)?;
                    (
                        tile_command_arguments(&tile),
                        json!({"kind": "tile", "x": tile.x, "z": tile.z, "level": tile.level}),
                    )
                }
            };
            command_values.insert(name.clone(), rendered);
            argument_values.insert(name.clone(), evidence);
        }

        let state = operation
            .confirmation
            .state
            .iter()
            .map(|matcher| resolve_state_match(matcher, names))
            .collect::<Result<Vec<_>, _>>()?;
        let command = operation.compiled_template.expand(&command_values)?;
        Ok(ResolvedOperation {
            command,
            resolved_arguments: Value::Object(argument_values),
            state,
        })
    }
}

pub struct PendingPackSeed {
    operation: PackOperation,
    why: Box<str>,
    pack_sha256: Box<str>,
    command: String,
    resolved_arguments: Value,
    resolved_state: Vec<ResolvedStateMatch>,
    grant_identity: Option<String>,
    baseline: ScenarioObserveWatermark,
    start_tick: u32,
    chat_sequence: Option<i32>,
    tick_budget: u32,
    reply: Option<fixtures::ChatObservation>,
}

impl PendingPackSeed {
    pub fn poll(
        &mut self,
        client: &mut Client,
        snapshot: &GameSnapshot,
        watermark: ScenarioObserveWatermark,
        _events: &[Option<AppliedEvent>],
    ) -> Result<Option<Value>, String> {
        fixtures::check_watermark(&self.baseline, &watermark)?;
        if fixtures::has_fresh_access_refusal(snapshot, self.chat_sequence) {
            return Err("seed pack command was refused by the selected engine".into());
        }
        if self.reply.is_none() {
            if let Some(expected) = self.operation.confirmation.reply.as_deref() {
                self.reply =
                    fixtures::find_fresh_exact_chat(snapshot, self.chat_sequence, expected)?;
            }
        }

        let elapsed = snapshot.tick().wrapping_sub(self.start_tick);
        let state_matches = if elapsed > 0 && !self.resolved_state.is_empty() {
            let mut all_match = true;
            for matcher in &self.resolved_state {
                if !matcher.matches(client, snapshot, &self.baseline)? {
                    all_match = false;
                    break;
                }
            }
            all_match
        } else {
            false
        };
        let complete = match (
            self.operation.confirmation.reply.is_some(),
            !self.resolved_state.is_empty(),
        ) {
            (false, false) => elapsed >= 1,
            (true, false) => self.reply.is_some(),
            (false, true) => state_matches,
            (true, true) => self.reply.is_some() && state_matches,
        };
        if complete {
            let label = self.operation.confirmation.label();
            let reply = self.reply.as_ref().map(|reply| {
                json!({
                    "text": reply.line,
                    "chat_sequence": reply.sequence,
                    "type": 0,
                })
            });
            let state = self
                .resolved_state
                .iter()
                .map(ResolvedStateMatch::json)
                .collect::<Vec<_>>();
            return Ok(Some(json!({
                "operation": self.operation.name,
                "status": label,
                "confirmation": label,
                "why": self.why.as_ref(),
                "pack_sha256": self.pack_sha256.as_ref(),
                "template": self.operation.template,
                "command": self.command,
                "send": "sent",
                "resolved_arguments": self.resolved_arguments,
                "effects": self.operation.effects,
                "precondition": self.operation.precondition.as_str(),
                "scope": self.operation.scope.as_str(),
                "grant_identity": self.grant_identity,
                "confirmation_spec": {
                    "reply_exact": self.operation.confirmation.reply,
                    "state": state,
                },
                "reply": reply,
                "state_matches": state_matches,
                "observed_ticks": elapsed,
            })));
        }
        if elapsed >= self.tick_budget {
            return Err(format!(
                "seed pack operation {:?} did not meet its declared confirmation within {} observed ticks",
                self.operation.name, self.tick_budget
            ));
        }
        Ok(None)
    }
}

fn resolve_state_match(
    matcher: &StateMatchSpec,
    names: &dyn PackNameLookup,
) -> Result<ResolvedStateMatch, String> {
    match matcher {
        StateMatchSpec::ItemCount {
            item,
            container,
            count,
        } => {
            let resolved = names.resolve(NameTable::Item, item)?;
            validate_resolved_name(&resolved, NameTable::Item)?;
            Ok(ResolvedStateMatch::ItemCount {
                item: resolved.name,
                item_id: resolved.id,
                container: *container,
                count: *count,
            })
        }
        StateMatchSpec::Stat {
            skill,
            base,
            effective,
            xp,
        } => {
            let resolved = names.resolve(NameTable::Skill, skill)?;
            validate_resolved_name(&resolved, NameTable::Skill)?;
            Ok(ResolvedStateMatch::Stat {
                skill: resolved.name,
                skill_id: resolved.id,
                base: *base,
                effective: *effective,
                xp: *xp,
            })
        }
        StateMatchSpec::Tile { x, z, level } => Ok(ResolvedStateMatch::Tile {
            x: *x,
            z: *z,
            level: *level,
        }),
        StateMatchSpec::Var { name, value } => {
            let resolved = names.resolve(NameTable::Varp, name)?;
            validate_resolved_name(&resolved, NameTable::Varp)?;
            Ok(ResolvedStateMatch::Var {
                name: resolved.name,
                varp_id: resolved.id,
                value: *value,
            })
        }
    }
}

fn parse_template(source: &str, params: &BTreeMap<String, ParamKind>) -> Result<Template, String> {
    if source.is_empty() || !source.is_ascii() {
        return Err("seed pack command template must be non-empty ASCII".into());
    }
    if source.bytes().any(|byte| byte.is_ascii_control()) {
        return Err("seed pack command template cannot contain control characters".into());
    }
    for name in params.keys() {
        validate_parameter_name(name)?;
    }

    let bytes = source.as_bytes();
    let mut parts = Vec::new();
    let mut used = BTreeSet::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] == b'}' {
            return Err("seed pack template has an unmatched closing brace".into());
        }
        if bytes[cursor] == b'{' {
            let start = cursor + 1;
            let end = bytes[start..]
                .iter()
                .position(|byte| *byte == b'}')
                .map(|offset| start + offset)
                .ok_or("seed pack template has an unmatched opening brace")?;
            let name = &source[start..end];
            validate_parameter_name(name)?;
            if !params.contains_key(name) {
                return Err(format!(
                    "seed pack template uses undeclared parameter {name:?}"
                ));
            }
            used.insert(name.to_owned());
            parts.push(TemplatePart::Parameter(name.to_owned()));
            cursor = end + 1;
        } else {
            let start = cursor;
            while cursor < bytes.len() && !matches!(bytes[cursor], b'{' | b'}') {
                cursor += 1;
            }
            parts.push(TemplatePart::Literal(source[start..cursor].to_owned()));
        }
    }

    if used.len() != params.len() {
        return Err("seed pack template must use every declared parameter".into());
    }
    let Some(TemplatePart::Literal(first)) = parts.first() else {
        return Err("seed pack template must start with one command name".into());
    };
    let command_end = first
        .bytes()
        .position(|byte| byte == b' ')
        .unwrap_or(first.len());
    validate_command_name(&first[..command_end])?;
    let has_parameters = !used.is_empty();
    if has_parameters {
        if command_end == first.len() || !first[command_end..].bytes().all(|byte| byte == b' ') {
            return Err("seed pack command name must be separated from its parameters".into());
        }
        for (index, part) in parts.iter().enumerate().skip(1) {
            match part {
                TemplatePart::Literal(text) => {
                    if index + 1 == parts.len() || !separator_text(text) {
                        return Err(
                            "seed pack template permits only spaces or commas outside placeholders"
                                .into(),
                        );
                    }
                }
                TemplatePart::Parameter(_) => {
                    if matches!(parts.get(index - 1), Some(TemplatePart::Parameter(_))) {
                        return Err("seed pack placeholders need a separator".into());
                    }
                }
            }
        }
    } else if command_end != first.len() || parts.len() != 1 {
        return Err("a command without parameters cannot contain raw argument text".into());
    }
    Ok(Template { parts })
}

fn separator_text(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| matches!(byte, b' ' | b','))
}

fn validate_command_name(name: &str) -> Result<(), String> {
    let name = name.strip_prefix('~').unwrap_or(name);
    let mut bytes = name.bytes();
    if !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        || !bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err("seed pack template must start with one lower-case command name".into());
    }
    Ok(())
}

fn validate_operation_name(name: &str) -> Result<(), String> {
    let mut bytes = name.bytes();
    if !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        || !bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
    {
        return Err("seed pack operation names must use lower-case ASCII identifiers".into());
    }
    Ok(())
}

fn validate_parameter_name(name: &str) -> Result<(), String> {
    let mut bytes = name.bytes();
    if !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        || !bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err("seed pack parameter names must use lower-case ASCII identifiers".into());
    }
    Ok(())
}

fn validate_resolved_name(name: &ResolvedName, table: NameTable) -> Result<(), String> {
    fixtures::validate_name(&name.name, table.as_str())?;
    if name.id < 0 {
        return Err(format!(
            "selected {} name has a negative id",
            table.as_str()
        ));
    }
    Ok(())
}

fn integer_value(value: &Value, minimum: i32, maximum: i32, name: &str) -> Result<i32, String> {
    let number = value
        .as_number()
        .ok_or_else(|| format!("seed pack argument {name:?} must be an integer"))?;
    let parsed = if let Some(value) = number.as_i64() {
        value
    } else if let Some(value) = number.as_u64() {
        i64::try_from(value)
            .map_err(|_| format!("seed pack argument {name:?} is outside its declared bounds"))?
    } else if let Some(value) = number.as_f64() {
        if !value.is_finite() || value.fract() != 0.0 {
            return Err(format!(
                "seed pack argument {name:?} must be finite and integral"
            ));
        }
        if value < f64::from(minimum) || value > f64::from(maximum) {
            return Err(format!(
                "seed pack argument {name:?} is outside its declared bounds"
            ));
        }
        value as i64
    } else {
        return Err(format!("seed pack argument {name:?} must be an integer"));
    };
    if parsed < i64::from(minimum) || parsed > i64::from(maximum) {
        return Err(format!(
            "seed pack argument {name:?} is outside its declared bounds"
        ));
    }
    i32::try_from(parsed)
        .map_err(|_| format!("seed pack argument {name:?} is outside its declared bounds"))
}

fn tile_value(value: &Value, name: &str) -> Result<api::snapshot::WorldTile, String> {
    let fields = value
        .as_object()
        .ok_or_else(|| format!("seed pack argument {name:?} must be a tile table"))?;
    if fields.len() != 3
        || !["x", "z", "level"]
            .iter()
            .all(|field| fields.contains_key(*field))
    {
        return Err(format!(
            "seed pack tile argument {name:?} needs only x, z and level"
        ));
    }
    // R289/R377 engine/src/network/game/client/handler/ClientCheatHandler.ts:577-610 maps 0..255 squares and 0..63 local
    // coordinates to world x/z 0..16,383, with planes 0..3.
    Ok(api::snapshot::WorldTile {
        x: integer_value(&fields["x"], 0, 16_383, name)?,
        z: integer_value(&fields["z"], 0, 16_383, name)?,
        level: integer_value(&fields["level"], 0, 3, name)?,
    })
}

fn insert_operation(
    operations: &mut BTreeMap<String, PackOperation>,
    table: &dyn TableLike,
) -> Result<(), String> {
    let operation = parse_operation(table)?;
    let name = operation.name.clone();
    if operations.insert(name.clone(), operation).is_some() {
        return Err(format!("seed pack operation name {name:?} is duplicated"));
    }
    Ok(())
}

fn parse_operation(table: &dyn TableLike) -> Result<PackOperation, String> {
    let context = "seed pack operation";
    ensure_fields(
        table,
        &[
            "name",
            "template",
            "params",
            "effects",
            "precondition",
            "scope",
            "confirm",
        ],
        &[
            "name",
            "template",
            "effects",
            "precondition",
            "scope",
            "confirm",
        ],
        context,
    )?;
    let name = string_field(table, "name", context)?.to_owned();
    validate_operation_name(&name)?;
    let template = string_field(table, "template", context)?.to_owned();
    let params = table
        .get("params")
        .map(parse_params)
        .transpose()?
        .unwrap_or_default();
    let effects_item = required_item(table, "effects", context)?;
    let effects_array = effects_item
        .as_array()
        .ok_or("seed pack operation effects must be a text array")?;
    if effects_array.is_empty() {
        return Err(format!(
            "seed pack operation {name:?} has no declared effects"
        ));
    }
    let mut effects = Vec::with_capacity(effects_array.len());
    for effect in effects_array.iter() {
        let effect = effect
            .as_str()
            .ok_or("seed pack operation effects must contain only text")?;
        if effect.is_empty() || effect.bytes().any(|byte| byte.is_ascii_control()) {
            return Err(format!(
                "seed pack operation {name:?} has an empty or invalid effect"
            ));
        }
        effects.push(effect.to_owned());
    }
    let precondition = match string_field(table, "precondition", context)? {
        "none" => Precondition::None,
        "fixture-idle" => Precondition::FixtureIdle,
        value => return Err(format!("unknown seed pack precondition {value:?}")),
    };
    let scope = match string_field(table, "scope", context)? {
        "own-account" => Scope::OwnAccount,
        "world" => Scope::World,
        value => return Err(format!("unknown seed pack scope {value:?}")),
    };
    let confirmation = parse_confirmation(required_item(table, "confirm", context)?)?;
    let compiled_template = parse_template(&template, &params)?;
    Ok(PackOperation {
        name,
        template,
        compiled_template,
        params,
        effects,
        precondition,
        scope,
        confirmation,
    })
}

fn parse_params(item: &Item) -> Result<BTreeMap<String, ParamKind>, String> {
    let table = item
        .as_table_like()
        .ok_or("seed pack params must be a table")?;
    let mut params = BTreeMap::new();
    for (key, value) in table.iter() {
        let name = key;
        validate_parameter_name(name)?;
        let context = format!("seed pack parameter {name:?}");
        let definition = value
            .as_table_like()
            .ok_or_else(|| format!("{context} must be a table"))?;
        let kind = string_field(definition, "kind", &context)?;
        let parsed = match kind {
            "name" => {
                ensure_fields(definition, &["kind", "table"], &["kind", "table"], &context)?;
                ParamKind::Name(parse_name_table(
                    string_field(definition, "table", &context)?,
                    &context,
                )?)
            }
            "int" => {
                ensure_fields(
                    definition,
                    &["kind", "min", "max"],
                    &["kind", "min", "max"],
                    &context,
                )?;
                let min = integer_field(definition, "min", &context)?;
                let max = integer_field(definition, "max", &context)?;
                if min > max {
                    return Err(format!("{context} has min greater than max"));
                }
                ParamKind::Int { min, max }
            }
            "skill" => {
                ensure_fields(definition, &["kind"], &["kind"], &context)?;
                ParamKind::Skill
            }
            "tile" => {
                ensure_fields(definition, &["kind"], &["kind"], &context)?;
                ParamKind::Tile
            }
            value => return Err(format!("{context} has unknown kind {value:?}")),
        };
        params.insert(name.to_owned(), parsed);
    }
    Ok(params)
}

fn parse_name_table(value: &str, context: &str) -> Result<NameTable, String> {
    match value {
        "item" => Ok(NameTable::Item),
        "varp" => Ok(NameTable::Varp),
        "varbit" => Ok(NameTable::Varbit),
        "skill" => Ok(NameTable::Skill),
        "npc" => Ok(NameTable::Npc),
        "loc" => Ok(NameTable::Loc),
        _ => Err(format!("{context} declares unknown name table {value:?}")),
    }
}

fn parse_confirmation(item: &Item) -> Result<Confirmation, String> {
    if let Some(value) = item.as_str() {
        return if value == "none" {
            Ok(Confirmation {
                reply: None,
                state: Vec::new(),
            })
        } else {
            Err("seed pack confirmation string must be `none`".into())
        };
    }
    let spec = item
        .as_table_like()
        .ok_or("seed pack confirmation must be `none` or a table")?;
    ensure_fields(spec, &["reply", "state"], &[], "seed pack confirmation")?;
    let reply = spec.get("reply").map(parse_reply).transpose()?;
    let state = spec
        .get("state")
        .map(parse_state_matches)
        .transpose()?
        .unwrap_or_default();
    if reply.is_none() && state.is_empty() {
        return Err("seed pack confirmation needs a reply or at least one state match".into());
    }
    Ok(Confirmation { reply, state })
}

fn parse_reply(item: &Item) -> Result<String, String> {
    let table = item
        .as_table_like()
        .ok_or("seed pack reply confirmation must be a table")?;
    ensure_fields(table, &["exact"], &["exact"], "seed pack reply")?;
    let exact = string_field(table, "exact", "seed pack reply")?;
    if exact.is_empty() || exact.bytes().any(|byte| byte.is_ascii_control()) {
        return Err("seed pack exact reply must be non-empty text without controls".into());
    }
    Ok(exact.to_owned())
}

fn parse_state_matches(item: &Item) -> Result<Vec<StateMatchSpec>, String> {
    let array = item
        .as_array()
        .ok_or("seed pack state confirmation must be an array")?;
    let mut matches = Vec::with_capacity(array.len());
    for value in array.iter() {
        let table = value
            .as_inline_table()
            .ok_or("seed pack state entries must be typed inline tables")?;
        matches.push(parse_state_match(table)?);
    }
    Ok(matches)
}

fn parse_state_match(table: &dyn TableLike) -> Result<StateMatchSpec, String> {
    let kind = string_field(table, "kind", "seed pack state match")?;
    match kind {
        "item_count" => {
            ensure_fields(
                table,
                &["kind", "item", "container", "count"],
                &["kind", "item", "container", "count"],
                "seed pack item-count match",
            )?;
            let item = string_field(table, "item", "seed pack item-count match")?.to_owned();
            fixtures::validate_name(&item, "item")?;
            let container = match string_field(table, "container", "seed pack item-count match")? {
                "carried" => ItemContainer::Inventory,
                "worn" => ItemContainer::Equipment,
                value => return Err(format!("unknown seed pack item container {value:?}")),
            };
            let count = integer_field(table, "count", "seed pack item-count match")?;
            if count < 0 {
                return Err("seed pack item count cannot be negative".into());
            }
            Ok(StateMatchSpec::ItemCount {
                item,
                container,
                count,
            })
        }
        "stat" => {
            ensure_fields(
                table,
                &["kind", "skill", "base", "effective", "xp"],
                &["kind", "skill", "base", "effective", "xp"],
                "seed pack stat match",
            )?;
            let skill = string_field(table, "skill", "seed pack stat match")?.to_owned();
            fixtures::validate_name(&skill, "skill")?;
            let base = integer_field(table, "base", "seed pack stat match")?;
            let effective = integer_field(table, "effective", "seed pack stat match")?;
            let xp = integer_field(table, "xp", "seed pack stat match")?;
            if xp < 0 {
                return Err("seed pack stat XP cannot be negative".into());
            }
            Ok(StateMatchSpec::Stat {
                skill,
                base,
                effective,
                xp,
            })
        }
        "tile" => {
            ensure_fields(
                table,
                &["kind", "x", "z", "level"],
                &["kind", "x", "z", "level"],
                "seed pack tile match",
            )?;
            let x = integer_field(table, "x", "seed pack tile match")?;
            let z = integer_field(table, "z", "seed pack tile match")?;
            let level = integer_field(table, "level", "seed pack tile match")?;
            validate_tile(x, z, level)?;
            Ok(StateMatchSpec::Tile { x, z, level })
        }
        "var" => {
            ensure_fields(
                table,
                &["kind", "name", "value"],
                &["kind", "name", "value"],
                "seed pack var match",
            )?;
            let name = string_field(table, "name", "seed pack var match")?.to_owned();
            fixtures::validate_name(&name, "varp")?;
            let value = integer_field(table, "value", "seed pack var match")?;
            Ok(StateMatchSpec::Var { name, value })
        }
        value => Err(format!("unknown seed pack state match kind {value:?}")),
    }
}

fn required_item<'a>(
    table: &'a dyn TableLike,
    key: &str,
    context: &str,
) -> Result<&'a Item, String> {
    table
        .get(key)
        .ok_or_else(|| format!("{context} is missing required field {key:?}"))
}

fn string_field<'a>(table: &'a dyn TableLike, key: &str, context: &str) -> Result<&'a str, String> {
    required_item(table, key, context)?
        .as_str()
        .ok_or_else(|| format!("{context} field {key:?} must be text"))
}

fn integer_field(table: &dyn TableLike, key: &str, context: &str) -> Result<i32, String> {
    let value = required_item(table, key, context)?
        .as_integer()
        .ok_or_else(|| format!("{context} field {key:?} must be an integer"))?;
    i32::try_from(value)
        .map_err(|_| format!("{context} field {key:?} is outside signed 32-bit range"))
}

fn ensure_fields(
    table: &dyn TableLike,
    allowed: &[&str],
    required: &[&str],
    context: &str,
) -> Result<(), String> {
    for (key, _) in table.iter() {
        let name = key;
        if !allowed.contains(&name) {
            return Err(format!("{context} has unknown field {name:?}"));
        }
    }
    for field in required {
        if !table.contains_key(field) {
            return Err(format!("{context} is missing required field {field:?}"));
        }
    }
    Ok(())
}

fn validate_tile(x: i32, z: i32, level: i32) -> Result<(), String> {
    if !(0..=16_383).contains(&x) || !(0..=16_383).contains(&z) || !(0..=3).contains(&level) {
        return Err("seed pack tile is outside the selected engine's coordinate bounds".into());
    }
    Ok(())
}

fn tile_command_arguments(tile: &api::snapshot::WorldTile) -> String {
    // R289/R377 engine/src/network/game/client/handler/ClientCheatHandler.ts:577-610 takes plane, map x/z and local x/z.
    let map_x = tile.x >> 6;
    let map_z = tile.z >> 6;
    let local_x = tile.x & 63;
    let local_z = tile.z & 63;
    format!("{},{},{},{},{}", tile.level, map_x, map_z, local_x, local_z)
}

fn validate_grant_identity(grant: &str) -> Result<(), String> {
    if grant.is_empty()
        || grant.len() > MAX_GRANT_BYTES
        || grant.bytes().any(|byte| !(0x21..=0x7e).contains(&byte))
    {
        return Err("seed pack grant identity must be 1 through 160 printable ASCII bytes".into());
    }
    Ok(())
}

fn read_pack(path: &Path) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|error| format!("seed pack: {}", error.kind()))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("seed pack metadata: {}", error.kind()))?;
    if !metadata.is_file() {
        return Err("seed pack must be a regular file".into());
    }
    let declared = metadata.len();
    if declared > MAX_PACK_BYTES {
        return Err("seed pack exceeds the 64 KiB size limit".into());
    }
    let capacity = usize::try_from(declared).map_err(|_| "seed pack is too large".to_string())?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(MAX_PACK_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("seed pack read: {}", error.kind()))?;
    if bytes.len() > capacity || bytes.len() as u64 != declared {
        return Err("seed pack changed while it was being read".into());
    }
    Ok(bytes)
}
