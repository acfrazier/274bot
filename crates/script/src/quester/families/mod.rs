//! S1 step families and predicate plans.

pub mod combat;
pub mod dialogue;
pub mod progress_predicates;
pub mod reach;
pub mod s2;

use super::compile::{
    CompileContext, CompileError, PredicateContext, PredicatePlan, StepContext, StepOutcome,
    StepPlan, StepRun,
};
use super::path::PredicateDocument;
use crate::combat::{begin_clear_owned_prayers, ClearPrayers, Hygiene, RaisedPrayers};
use crate::dialogue_outcome::DialogueOutcome;
use crate::native::walk::Walk;
use crate::native::{ActionError, ActionHandle, NativeActions, WalkOptions, WalkReceipt};
use crate::shim::InteractReq;
use api::selected::Truth;
use api::snapshot::{ChatLineView, QuestListStatus};
use api::WorldTile;
use serde::Deserialize;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct NoArgs {}
pub(super) const MANUAL_MOVEMENT_MESSAGE: &str = "cancelled by user input";
/// One UseOn attempt inside an `until` loop. A silent miss must not consume
/// the whole step `settle_ms` (240s for Sheep Shearer). Successful shear is
/// two `p_delay(0)` ticks; 8s also covers a lagged inventory post.
const USE_ON_ROUND_MS: u64 = 8_000;

pub(super) fn manual_movement_message() -> Arc<str> {
    static REASON: std::sync::LazyLock<Arc<str>> =
        std::sync::LazyLock::new(|| Arc::from(MANUAL_MOVEMENT_MESSAGE));
    Arc::clone(&REASON)
}

/// A route refusal is a terminal for the owning step, not permission to
/// silently re-arm its approach. Park until an explicit owner retry.
fn walk_step_evidence(
    receipt: WalkReceipt,
) -> Result<api::quest_progress::EvidenceStamp, ActionError> {
    receipt.into_arrival()
}

pub fn handlers() -> &'static [super::compile::StepHandler] {
    static HANDLERS: &[super::compile::StepHandler] = &[
        super::compile::step!("walk", 1, Default, WalkArgs, compile_walk),
        super::compile::step!("talk", 1, Explicit, TalkArgs, compile_talk),
        super::compile::step!("interact", 1, Explicit, InteractArgs, compile_interact),
        super::compile::step!("use_on", 1, Explicit, UseOnArgs, compile_use_on),
        super::compile::step!("acquire", 1, Default, AcquireArgs, compile_acquire),
        super::compile::step!("wait", 1, Default, WaitArgs, compile_wait),
        super::compile::step!("bank", 1, Default, s2::BankArgs, s2::compile_bank),
        super::compile::step!("buy", 1, Default, s2::BuyArgs, s2::compile_buy),
        super::compile::step!("make", 1, Explicit, s2::MakeArgs, s2::compile_make),
        super::compile::step!("equip", 1, Default, s2::EquipArgs, s2::compile_equip),
        super::compile::step!("unequip", 1, Default, s2::EquipArgs, s2::compile_unequip),
        super::compile::step!("loadout", 1, Default, s2::LoadoutArgs, s2::compile_loadout),
        super::compile::step!("combat", 1, Explicit, combat::CombatArgs, combat::compile),
    ];
    HANDLERS
}

pub fn predicate_handlers() -> &'static [super::compile::PredicateHandler] {
    static HANDLERS: &[super::compile::PredicateHandler] = &[
        super::compile::fact!(
            "has_item",
            1,
            super::compile::ProgressRead::None,
            ObjArg,
            compile_has_item
        ),
        super::compile::fact!(
            "near",
            1,
            super::compile::ProgressRead::None,
            NearArg,
            compile_near
        ),
        super::compile::fact!(
            "message",
            1,
            super::compile::ProgressRead::None,
            MessageArg,
            compile_message
        ),
        super::compile::fact!(
            "message_state",
            1,
            super::compile::ProgressRead::None,
            MessageStateArg,
            compile_message_state
        ),
        super::compile::fact!(
            "quest_colour",
            1,
            super::compile::ProgressRead::Colour,
            ColourArg,
            compile_quest_colour
        ),
        super::compile::fact!(
            "in_combat",
            1,
            super::compile::ProgressRead::None,
            NoArgs,
            compile_in_combat
        ),
        super::compile::fact!(
            "npc_present",
            1,
            super::compile::ProgressRead::None,
            NpcArg,
            compile_npc_present
        ),
        super::compile::fact!(
            "loc_present",
            1,
            super::compile::ProgressRead::None,
            LocArg,
            compile_loc_present
        ),
        super::compile::fact!(
            "ground_item_near",
            1,
            super::compile::ProgressRead::None,
            GroundArg,
            compile_ground_item_near
        ),
        super::compile::fact!(
            "modal_open",
            1,
            super::compile::ProgressRead::None,
            NoArgs,
            compile_modal_open
        ),
        super::compile::fact!(
            "item_count_at_least",
            1,
            super::compile::ProgressRead::None,
            CountArg,
            compile_item_count_at_least
        ),
        super::compile::fact!(
            "worn",
            1,
            super::compile::ProgressRead::None,
            ObjArg,
            compile_worn
        ),
        super::compile::fact!(
            "on_level",
            1,
            super::compile::ProgressRead::None,
            LevelArg,
            compile_on_level
        ),
        super::compile::fact!(
            "in_area",
            1,
            super::compile::ProgressRead::None,
            AreaArg,
            compile_in_area
        ),
        super::compile::fact!(
            "npc_absent",
            1,
            super::compile::ProgressRead::None,
            NpcArg,
            compile_npc_absent
        ),
        super::compile::fact!(
            "skill_at_least",
            1,
            super::compile::ProgressRead::None,
            SkillArg,
            compile_skill_at_least
        ),
        super::compile::fact!(
            "hp_fraction_below",
            1,
            super::compile::ProgressRead::None,
            FractionArg,
            compile_hp_fraction_below
        ),
        super::compile::fact!(
            "prayer_points_at_least",
            1,
            super::compile::ProgressRead::None,
            LevelArg,
            compile_prayer_points
        ),
        super::compile::fact!(
            "combat_end",
            1,
            super::compile::ProgressRead::None,
            combat::CombatEndArgs,
            combat::compile_end_predicate
        ),
        super::compile::fact!(
            "stage_in",
            1,
            super::compile::ProgressRead::Journal,
            progress_predicates::StageInArgs,
            progress_predicates::compile_stage_in
        ),
        super::compile::fact!(
            "flag",
            1,
            super::compile::ProgressRead::Journal,
            progress_predicates::FlagArgs,
            progress_predicates::compile_flag
        ),
        super::compile::fact!(
            "bank_known",
            1,
            super::compile::ProgressRead::None,
            NoArgs,
            s2::compile_bank_known
        ),
        super::compile::fact!(
            "bank_has",
            1,
            super::compile::ProgressRead::None,
            s2::BankHasArgs,
            s2::compile_bank_has
        ),
        super::compile::fact!(
            "loadout_ready",
            1,
            super::compile::ProgressRead::None,
            s2::LoadoutArgs,
            s2::compile_loadout_ready
        ),
        super::compile::fact!(
            "equipment_only",
            1,
            super::compile::ProgressRead::None,
            s2::EquipmentOnlyArgs,
            s2::compile_equipment_only
        ),
    ];
    HANDLERS
}

pub fn compile_predicate(
    document: &PredicateDocument,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    match document {
        PredicateDocument::All(items) => Ok(Arc::new(AllPlan {
            items: compile_each(items, cx)?,
        })),
        PredicateDocument::Any(items) => Ok(Arc::new(AnyPlan {
            items: compile_each(items, cx)?,
        })),
        PredicateDocument::Not(inner) => Ok(Arc::new(NotPlan {
            inner: compile_predicate(inner, cx)?,
        })),
        PredicateDocument::Fact {
            kind,
            version,
            args,
        } => {
            let handler = super::compile::predicate_handlers()
                .find(|handler| handler.kind == kind && handler.version == *version)
                .ok_or_else(|| CompileError::code("unknown-predicate"))?;
            (handler.compile)(args, cx)
        }
    }
}

fn compile_each(
    items: &[PredicateDocument],
    cx: &CompileContext<'_>,
) -> Result<Vec<Arc<dyn PredicatePlan>>, CompileError> {
    items
        .iter()
        .map(|item| compile_predicate(item, cx))
        .collect()
}

struct AllPlan {
    items: Vec<Arc<dyn PredicatePlan>>,
}
impl PredicatePlan for AllPlan {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        Truth::all(self.items.iter().map(|item| item.evaluate(cx)))
    }
}
struct AnyPlan {
    items: Vec<Arc<dyn PredicatePlan>>,
}
impl PredicatePlan for AnyPlan {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        Truth::any(self.items.iter().map(|item| item.evaluate(cx)))
    }
}
struct NotPlan {
    inner: Arc<dyn PredicatePlan>,
}
impl PredicatePlan for NotPlan {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        !self.inner.evaluate(cx)
    }
}

fn resolve_obj(cx: &CompileContext<'_>, alias: &str) -> Result<i32, CompileError> {
    cx.selected
        .item_by_alias(alias)
        .map(|item| item.id)
        .ok_or_else(|| CompileError::code("unresolved-obj").with_detail(alias))
}

fn resolve_npc(cx: &CompileContext<'_>, alias: &str) -> Result<i32, CompileError> {
    cx.selected
        .npc_by_config(alias)
        .map(|row| row.id)
        .ok_or_else(|| CompileError::code("unresolved-npc").with_detail(alias))
}

fn resolve_loc(cx: &CompileContext<'_>, alias: &str) -> Result<i32, CompileError> {
    cx.selected
        .loc_by_config(alias)
        .map(|row| row.id)
        .ok_or_else(|| CompileError::code("unresolved-loc").with_detail(alias))
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct ObjArg {
    /// Symbolic object config name resolved in selected game data.
    obj: String,
}

fn compile_has_item(
    arg: ObjArg,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    Ok(Arc::new(HasItem {
        id: resolve_obj(cx, &arg.obj)?,
    }))
}

struct HasItem {
    id: i32,
}
impl PredicatePlan for HasItem {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        match cx.cx.snapshot().inventory() {
            None => Truth::Unknown,
            Some(inv) => truth(
                inv.value
                    .iter()
                    .any(|item| item.def.id == self.id && item.count > 0),
            ),
        }
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct NearArg {
    /// Observed target tile as x, z, level.
    tile: [i32; 3],
    /// Maximum distance in tiles.
    radius: i32,
}

fn compile_near(
    arg: NearArg,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    validate_tile_coordinates(arg.tile)?;
    Ok(Arc::new(Near {
        tile: WorldTile {
            x: arg.tile[0],
            z: arg.tile[1],
            level: arg.tile[2],
        },
        radius: arg.radius,
    }))
}

struct Near {
    tile: WorldTile,
    radius: i32,
}
impl PredicatePlan for Near {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        match cx.cx.snapshot().here() {
            None => Truth::Unknown,
            Some(here) => truth(reach::within(here.value, self.tile, self.radius)),
        }
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct MessageArg {
    /// Case-insensitive chat substrings; any one match is true.
    any: Vec<String>,
}

fn compile_message(
    arg: MessageArg,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    if arg.any.iter().any(|needle| needle.trim().is_empty()) {
        return Err(CompileError::code("invalid-args"));
    }
    Ok(Arc::new(Message {
        needles: arg
            .any
            .into_iter()
            .map(|n| n.to_ascii_lowercase())
            .collect(),
    }))
}

struct Message {
    needles: Vec<String>,
}
impl PredicatePlan for Message {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        let snapshot = cx.cx.snapshot();
        let Some(lines) = snapshot.chat_lines(cx.chat_since) else {
            return Truth::Unknown;
        };
        let hit = lines
            .value
            .iter()
            .any(|line| contains_any(line, &self.needles));
        truth(hit)
    }
}

fn contains_any(line: &ChatLineView, needles: &[String]) -> bool {
    line.username.is_none()
        && line.type_ == 0
        && needles.iter().any(|needle| {
            !needle.is_empty()
                && line
                    .text
                    .as_bytes()
                    .windows(needle.len())
                    .any(|part| part.eq_ignore_ascii_case(needle.as_bytes()))
        })
}

/// A recoverable observed state, unlike `message`'s post-step settle event.
/// The newest matching event in the available chat history wins; a clearing
/// event prevents a previous successful operation from being replayed forever.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct MessageStateArg {
    /// Events which set the observed state.
    set: Vec<String>,
    /// Events which clear the observed state.
    clear: Vec<String>,
}
fn compile_message_state(
    arg: MessageStateArg,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    if arg
        .set
        .iter()
        .chain(&arg.clear)
        .any(|needle| needle.trim().is_empty())
    {
        return Err(CompileError::code("invalid-args"));
    }
    Ok(Arc::new(MessageState {
        set: arg
            .set
            .into_iter()
            .map(|n| n.to_ascii_lowercase())
            .collect(),
        clear: arg
            .clear
            .into_iter()
            .map(|n| n.to_ascii_lowercase())
            .collect(),
    }))
}
struct MessageState {
    set: Vec<String>,
    clear: Vec<String>,
}
impl PredicatePlan for MessageState {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        let snapshot = cx.cx.snapshot();
        let Some(lines) = snapshot.chat_lines(-1) else {
            return Truth::Unknown;
        };
        let mut latest = (-1, false);
        for line in lines.value.iter() {
            let clear = contains_any(line, &self.clear);
            let set = contains_any(line, &self.set);
            if (clear || set) && line.sequence >= latest.0 {
                latest = (line.sequence, set && !clear);
            }
        }
        truth(latest.1)
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct ColourArg {
    /// Symbolic quest config name.
    quest: String,
    /// Desired tab colour state.
    is: ColourIs,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
enum ColourIs {
    NotStarted,
    InProgress,
    Complete,
}

fn compile_quest_colour(
    arg: ColourArg,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let facts = cx
        .quests
        .quest(&arg.quest)
        .map_err(|_| CompileError::code("unresolved-quest"))?;
    let want = match arg.is {
        ColourIs::NotStarted => QuestListStatus::NotStarted,
        ColourIs::InProgress => QuestListStatus::InProgress,
        ColourIs::Complete => QuestListStatus::Complete,
    };
    Ok(Arc::new(QuestColour {
        display: Arc::clone(&facts.display),
        want,
    }))
}

struct QuestColour {
    display: Arc<str>,
    want: QuestListStatus,
}
impl PredicatePlan for QuestColour {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        let snapshot = cx.cx.snapshot();
        let Some(rows) = snapshot.quest_statuses() else {
            return Truth::Unknown;
        };
        match rows
            .value
            .iter()
            .find(|row| row.name.eq_ignore_ascii_case(&self.display))
        {
            None => Truth::Unknown,
            Some(row) => truth(row.status() == self.want),
        }
    }
}

fn compile_in_combat(
    _args: NoArgs,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    Ok(Arc::new(InCombat))
}
struct InCombat;
impl PredicatePlan for InCombat {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        match cx.cx.snapshot().in_combat() {
            None => Truth::Unknown,
            Some(obs) => truth(obs.value.in_combat),
        }
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct NpcArg {
    /// Symbolic NPC config name.
    npc: String,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct LocArg {
    /// Symbolic location config name.
    loc: String,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct GroundArg {
    /// Symbolic ground-item object config name.
    obj: String,
    /// Maximum distance in tiles; omitted uses 12.
    #[serde(default)]
    radius: Option<i32>,
}

fn compile_npc_present(
    arg: NpcArg,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    Ok(Arc::new(NpcPresent {
        id: resolve_npc(cx, &arg.npc)?,
    }))
}
struct NpcPresent {
    id: i32,
}
impl PredicatePlan for NpcPresent {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        match cx.cx.snapshot().npcs() {
            None => Truth::Unknown,
            Some(npcs) => truth(
                npcs.value
                    .iter()
                    .any(|npc| npc.r#type == Some(self.id as usize)),
            ),
        }
    }
}

fn compile_npc_absent(
    arg: NpcArg,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    Ok(Arc::new(NotPlan {
        inner: Arc::new(NpcPresent {
            id: resolve_npc(cx, &arg.npc)?,
        }),
    }))
}

fn compile_loc_present(
    arg: LocArg,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let id = resolve_loc(cx, &arg.loc)?;
    Ok(Arc::new(LocPresent { id }))
}
struct LocPresent {
    id: i32,
}
impl PredicatePlan for LocPresent {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        match cx.cx.snapshot().locs() {
            None => Truth::Unknown,
            Some(locs) => truth(locs.value.iter().any(|loc| loc.id == self.id)),
        }
    }
}

fn compile_ground_item_near(
    arg: GroundArg,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    Ok(Arc::new(GroundNear {
        id: resolve_obj(cx, &arg.obj)?,
        radius: arg.radius.unwrap_or(12),
    }))
}
struct GroundNear {
    id: i32,
    radius: i32,
}
impl PredicatePlan for GroundNear {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        match cx.cx.snapshot().ground_items() {
            None => Truth::Unknown,
            Some(items) => truth(
                items
                    .value
                    .iter()
                    .any(|item| item.def.id == self.id && item.distance <= self.radius),
            ),
        }
    }
}

fn compile_modal_open(
    _args: NoArgs,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    Ok(Arc::new(ModalOpen))
}
struct ModalOpen;
impl PredicatePlan for ModalOpen {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        match cx.cx.snapshot().main_modal() {
            None => Truth::Unknown,
            Some(modal) => truth(modal.value.root >= 0),
        }
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct CountArg {
    /// Symbolic object config name counted in the inventory.
    obj: String,
    /// Required quantity; omitted uses 1. A progress quantity reads the latest journal count.
    #[serde(default)]
    qty: Option<s2::QuantityDocument>,
}

fn compile_item_count_at_least(
    arg: CountArg,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    Ok(Arc::new(CountAtLeast {
        id: resolve_obj(cx, &arg.obj)?,
        qty: s2::compile_quantity(arg.qty.unwrap_or(s2::QuantityDocument::Fixed(1)), cx)?,
    }))
}
struct CountAtLeast {
    id: i32,
    qty: s2::QuantityPlan,
}
impl PredicatePlan for CountAtLeast {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        let Some(qty) = self.qty.evaluate(cx) else {
            return Truth::Unknown;
        };
        match cx.cx.snapshot().inventory() {
            None => Truth::Unknown,
            Some(inv) => {
                let n: i32 = inv
                    .value
                    .iter()
                    .filter(|item| item.def.id == self.id)
                    .map(|item| item.count)
                    .sum();
                truth(n >= qty)
            }
        }
    }
}

fn compile_worn(
    arg: ObjArg,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    Ok(Arc::new(Worn {
        id: resolve_obj(cx, &arg.obj)?,
    }))
}
struct Worn {
    id: i32,
}
impl PredicatePlan for Worn {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        match cx.cx.snapshot().equipment() {
            None => Truth::Unknown,
            Some(eq) => truth(eq.value.iter().any(|item| item.def.id == self.id)),
        }
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct LevelArg {
    /// Skill or floor level to test.
    level: i32,
}

fn compile_on_level(
    arg: LevelArg,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    Ok(Arc::new(OnLevel { level: arg.level }))
}
struct OnLevel {
    level: i32,
}
impl PredicatePlan for OnLevel {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        match cx.cx.snapshot().here() {
            None => Truth::Unknown,
            Some(here) => truth(here.value.level == self.level),
        }
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct AreaArg {
    /// Named Path area whose boxes are tested.
    area: String,
}

fn compile_in_area(
    arg: AreaArg,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let boxes = cx
        .areas
        .get(&arg.area)
        .cloned()
        .ok_or_else(|| CompileError::code("unresolved-area"))?;
    Ok(Arc::new(InArea { boxes }))
}
struct InArea {
    boxes: Vec<[i32; 5]>,
}
impl PredicatePlan for InArea {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        match cx.cx.snapshot().here() {
            None => Truth::Unknown,
            Some(here) => truth(
                self.boxes
                    .iter()
                    .any(|b| super::path::box_contains(*b, here.value)),
            ),
        }
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct SkillArg {
    /// Case-insensitive skill name.
    skill: String,
    /// Minimum base level required.
    level: i32,
}

fn compile_skill_at_least(
    arg: SkillArg,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    Ok(Arc::new(SkillAtLeast {
        skill: Arc::from(arg.skill),
        level: arg.level,
    }))
}
struct SkillAtLeast {
    skill: Arc<str>,
    level: i32,
}
impl PredicatePlan for SkillAtLeast {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        match cx.cx.snapshot().stats() {
            None => Truth::Unknown,
            Some(stats) => truth(stats.value.iter().any(|stat| {
                stat.name.eq_ignore_ascii_case(&self.skill) && stat.base >= self.level
            })),
        }
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct FractionArg {
    /// Threshold fraction of current hitpoints divided by base hitpoints.
    below: f32,
}

fn compile_hp_fraction_below(
    arg: FractionArg,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    Ok(Arc::new(HpBelow { below: arg.below }))
}
struct HpBelow {
    below: f32,
}
impl PredicatePlan for HpBelow {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        match cx.cx.snapshot().stats() {
            None => Truth::Unknown,
            Some(stats) => {
                let Some(hp) = stats
                    .value
                    .iter()
                    .find(|s| s.name.eq_ignore_ascii_case("hitpoints"))
                else {
                    return Truth::Unknown;
                };
                if hp.base <= 0 {
                    return Truth::Unknown;
                }
                truth((hp.effective as f32 / hp.base as f32) < self.below)
            }
        }
    }
}

fn compile_prayer_points(
    arg: LevelArg,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    Ok(Arc::new(SkillAtLeast {
        skill: Arc::from("prayer"),
        level: arg.level,
    }))
}

fn truth(value: bool) -> Truth {
    if value {
        Truth::True
    } else {
        Truth::False
    }
}

pub(super) fn validate_tile(tile: [i32; 3], source: &str) -> Result<(), CompileError> {
    if source.trim().is_empty() {
        return Err(CompileError::code("tile-source-required"));
    }
    validate_tile_coordinates(tile)
}

fn validate_tile_coordinates(tile: [i32; 3]) -> Result<(), CompileError> {
    if !(0..=16383).contains(&tile[0])
        || !(0..=16383).contains(&tile[1])
        || !(0..=3).contains(&tile[2])
    {
        return Err(CompileError::code("invalid-tile"));
    }
    Ok(())
}

fn anchor_tile(anchor: Option<&AnchorArg>) -> Result<Option<WorldTile>, CompileError> {
    anchor
        .map(|a| {
            validate_tile(a.tile, &a.source)?;
            Ok(WorldTile {
                x: a.tile[0],
                z: a.tile[1],
                level: a.tile[2],
            })
        })
        .transpose()
}

fn offered(ops: &[String], op: &str) -> Result<(), CompileError> {
    if ops.iter().any(|offered| offered.eq_ignore_ascii_case(op)) {
        Ok(())
    } else {
        Err(CompileError::code("unavailable-op"))
    }
}

fn name_matches(cx: &CompileContext<'_>, name: &str, op: &str) -> usize {
    cx.selected.loc_names().map_or(0, |facts| {
        facts
            .rows
            .iter()
            .filter(|row| {
                row.display
                    .as_deref()
                    .is_some_and(|display| display.eq_ignore_ascii_case(name))
                    && row
                        .ops
                        .iter()
                        .any(|offered| offered.eq_ignore_ascii_case(op))
            })
            .count()
    })
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct WalkArgs {
    /// Destination tile as x, z, level.
    tile: [i32; 3],
    /// Source citation for this authored destination.
    #[serde(default)]
    source: String,
    /// Navigation radius in tiles; 0 means 1.
    #[serde(default)]
    radius: u16,
    /// Named danger zones this movement may cross.
    #[serde(default)]
    cross: Vec<String>,
    /// Optional protected movement mode; currently `protect`.
    #[serde(default)]
    guard: Option<String>,
    /// Override inherited teleport permission for this movement.
    #[serde(default)]
    allow_teleports: Option<bool>,
    /// Override inherited Wilderness permission for this movement.
    #[serde(default)]
    allow_wilderness: Option<bool>,
    /// Override inherited danger-zone permission for this movement.
    #[serde(default)]
    allow_danger_zones: Option<bool>,
}

impl WalkArgs {
    fn options(&self) -> WalkOptions {
        WalkOptions {
            allow_teleports: self.allow_teleports.into(),
            allow_wilderness: self.allow_wilderness.into(),
            allow_danger_zones: self.allow_danger_zones.into(),
        }
    }
}

fn compile_walk(
    arg: WalkArgs,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    Ok(Arc::new(parse_walk_plan(arg)?))
}

fn parse_walk_plan(arg: WalkArgs) -> Result<WalkPlan, CompileError> {
    let protect = match arg.guard.as_deref() {
        None | Some("") => false,
        Some("protect") => true,
        Some(_) => {
            return Err(
                CompileError::code("invalid-args").with_detail("walk: guard must be protect")
            );
        }
    };
    validate_tile(arg.tile, &arg.source)?;
    let options = arg.options();
    let cross = arg
        .cross
        .into_iter()
        .map(Arc::from)
        .collect::<Vec<_>>()
        .into_boxed_slice();
    Ok(WalkPlan {
        tile: WorldTile {
            x: arg.tile[0],
            z: arg.tile[1],
            level: arg.tile[2],
        },
        radius: arg.radius.max(1),
        options,
        cross,
        protect,
    })
}

struct WalkPlan {
    tile: WorldTile,
    radius: u16,
    options: WalkOptions,
    cross: Box<[Arc<str>]>,
    protect: bool,
}
impl StepPlan for WalkPlan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        let mut request = reach::walk_request(self.tile, self.radius, None, cx.required_after);
        request.cross = self.cross.clone();
        request.options = self.options;
        request.protect = self.protect;
        let handle = cx.tick.actions.begin::<Walk>(request, &mut cx.tick.cx)?;
        Ok(Box::new(WalkRun {
            handle,
            warning: None,
        }))
    }
}

struct WalkRun {
    handle: ActionHandle<Walk>,
    warning: Option<Arc<str>>,
}
impl StepRun for WalkRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        while let Some(event) = cx
            .tick
            .actions
            .take_walk_event(&self.handle, &mut cx.tick.cx)
        {
            self.warning = Some(event.detail);
        }
        match cx.tick.actions.poll(&self.handle, &mut cx.tick.cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(receipt)) => {
                Poll::Ready(walk_step_evidence(receipt).map(|evidence| StepOutcome {
                    progress: None,
                    evidence,
                    receipt: None,
                }))
            }
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
        }
    }
    fn cancel(&mut self, _actions: &mut NativeActions) {}
    fn waiting_for(&self) -> Option<(&'static str, &Arc<str>)> {
        self.warning
            .as_ref()
            .map(|warning| ("Walk protection", warning))
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct AnchorArg {
    /// Authored destination tile as x, z, level.
    tile: [i32; 3],
    /// Source citation for the authored destination; required by compilation.
    #[serde(default)]
    source: String,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct TalkArgs {
    /// Symbolic NPC config name to speak with.
    npc: String,
    /// Optional authored approach anchor and source citation.
    #[serde(default)]
    anchor: Option<AnchorArg>,
    /// Maximum NPC interaction distance; 0 or omitted means 1 tile.
    #[serde(default)]
    leash: u16,
    /// Text fragments to prefer, checked in the given order.
    #[serde(default)]
    prefer: Vec<String>,
    /// Optional 1-based dialogue option index.
    #[serde(default)]
    choose: Option<i32>,
    /// Expected NPC opponent when dialogue deliberately hands control to combat.
    #[serde(default)]
    expect_combat: Option<ExpectedCombatArgs>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct ExpectedCombatArgs {
    /// Symbolic NPC config name that must be the observed combat opponent.
    npc: String,
}

/// A dialogue deliberately ceded control to its authored combat opponent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TalkReceipt {
    HandedToCombat { npc_type: i32, npc_index: usize },
}

impl super::compile::FamilyReceipt for TalkReceipt {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn compile_talk(arg: TalkArgs, cx: &CompileContext<'_>) -> Result<Arc<dyn StepPlan>, CompileError> {
    let id = resolve_npc(cx, &arg.npc)?;
    let display = cx
        .selected
        .npc_by_config(&arg.npc)
        .and_then(|row| row.display.as_deref())
        .ok_or_else(|| CompileError::code("unresolved-npc").with_detail(arg.npc.as_str()))?;
    let tile = anchor_tile(arg.anchor.as_ref())?;
    offered(&cx.selected.npc_by_config(&arg.npc).unwrap().ops, "Talk-to")?;
    let expect_combat = arg
        .expect_combat
        .as_ref()
        .map(|expected| resolve_npc(cx, &expected.npc))
        .transpose()?;
    Ok(Arc::new(TalkPlan {
        id,
        npc: Arc::from(display),
        tile,
        leash: arg.leash.max(1),
        prefer: arg.prefer.into_iter().map(Arc::from).collect(),
        choose: arg.choose,
        expect_combat,
    }))
}

struct TalkPlan {
    id: i32,
    npc: Arc<str>,
    tile: Option<WorldTile>,
    leash: u16,
    prefer: Arc<[Arc<str>]>,
    choose: Option<i32>,
    expect_combat: Option<i32>,
}
impl StepPlan for TalkPlan {
    fn begin(&self, _cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        Ok(Box::new(TalkRun {
            id: self.id,
            npc: Arc::clone(&self.npc),
            tile: self.tile,
            leash: self.leash,
            prefer: Arc::clone(&self.prefer),
            choose: self.choose,
            expect_combat: self.expect_combat,
            walk: None,
            dialogue: None,
            started: false,
        }))
    }
}

struct TalkRun {
    id: i32,
    npc: Arc<str>,
    tile: Option<WorldTile>,
    leash: u16,
    prefer: Arc<[Arc<str>]>,
    choose: Option<i32>,
    expect_combat: Option<i32>,
    walk: Option<ActionHandle<Walk>>,
    dialogue: Option<ActionHandle<dialogue::Dialogue>>,
    started: bool,
}
impl StepRun for TalkRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        if let Some(handle) = &self.walk {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(receipt)) => {
                    walk_step_evidence(receipt)?;
                    self.walk = None;
                }
            }
        }
        if !self.started {
            if let Some(tile) = self.tile {
                let here = cx.tick.cx.snapshot().here();
                let near =
                    here.is_some_and(|obs| reach::within(obs.value, tile, i32::from(self.leash)));
                if !near {
                    self.walk = Some(cx.tick.actions.begin::<Walk>(
                        reach::walk_request(tile, self.leash, None, cx.required_after),
                        &mut cx.tick.cx,
                    )?);
                    return Poll::Pending;
                }
            }
            self.dialogue = Some(cx.tick.actions.begin::<dialogue::Dialogue>(
                dialogue::DialogueArgs {
                    id: self.id,
                    npc: Arc::clone(&self.npc),
                    prefer: Arc::clone(&self.prefer),
                    choose: self.choose,
                },
                &mut cx.tick.cx,
            )?);
            self.started = true;
            return Poll::Pending;
        }
        if let Some(handle) = &self.dialogue {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Ok(DialogueOutcome::Failed)) => {
                    Poll::Ready(Err(ActionError::Failed(Arc::from("dialogue failed"))))
                }
                Poll::Ready(Ok(DialogueOutcome::CombatInterrupted)) => {
                    if let Some((npc_type, npc_index)) = self
                        .expect_combat
                        .and_then(|npc_type| expected_combat_target(&cx.tick.cx, npc_type))
                    {
                        return Poll::Ready(Ok(StepOutcome {
                            progress: None,
                            evidence: cx.tick.cx.evidence(),
                            receipt: Some(Arc::new(TalkReceipt::HandedToCombat {
                                npc_type,
                                npc_index,
                            })),
                        }));
                    }
                    static REASON: std::sync::LazyLock<Arc<str>> =
                        std::sync::LazyLock::new(|| Arc::from("dialogue interrupted by combat"));
                    Poll::Ready(Err(ActionError::Blocked(Arc::clone(&REASON))))
                }
                Poll::Ready(Ok(DialogueOutcome::Completed)) => Poll::Ready(Ok(StepOutcome {
                    progress: None,
                    evidence: cx.tick.cx.evidence(),
                    receipt: None,
                })),
                Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            }
        } else {
            Poll::Ready(Ok(StepOutcome {
                progress: None,
                evidence: cx.tick.cx.evidence(),
                receipt: None,
            }))
        }
    }
    fn cancel(&mut self, _actions: &mut NativeActions) {
        self.walk = None;
        self.dialogue = None;
    }
}

fn expected_combat_target(
    cx: &crate::native::ActionContext<'_>,
    npc_type: i32,
) -> Option<(i32, usize)> {
    let snapshot = cx.snapshot();
    let local = snapshot.local_player()?.value;
    let combat = snapshot.in_combat()?.value;
    let target = combat.target?;
    if !combat.in_combat || target.kind != api::snapshot::ActorKind::Npc {
        return None;
    }
    snapshot.npcs()?.value.iter().find_map(|npc| {
        (npc.index == target.index
            && npc.r#type == Some(npc_type as usize)
            && npc.target
                == Some(api::snapshot::ActorTargetView {
                    kind: api::snapshot::ActorKind::Player,
                    index: local.player.index,
                }))
        .then_some((npc_type, npc.index))
    })
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct InteractTarget {
    /// Ground-item config name; exactly one target selector is required.
    #[serde(default)]
    ground: Option<String>,
    /// Location config name; exactly one target selector is required.
    #[serde(default)]
    loc: Option<String>,
    /// NPC config name; exactly one target selector is required.
    #[serde(default)]
    npc: Option<String>,
    /// Display-name location selector; exactly one target selector is required.
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct InteractArgs {
    /// Target selector; exactly one target field is required.
    target: InteractTarget,
    /// Interaction option offered by the selected target.
    op: String,
    /// Optional authored approach anchor and source citation.
    #[serde(default)]
    anchor: Option<AnchorArg>,
    /// Maximum interaction distance; 0 or omitted means 1 tile.
    #[serde(default)]
    radius: i32,
    /// Wait for a temporarily missing target instead of failing immediately.
    #[serde(default)]
    wait_if_missing: bool,
    /// Optional settle-window override in milliseconds; must be at least 1.
    #[serde(default)]
    #[cfg_attr(feature = "path-schema", schemars(range(min = 1)))]
    settle_ms: Option<u64>,
}

fn compile_interact(
    arg: InteractArgs,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    if [
        arg.target.ground.is_some(),
        arg.target.loc.is_some(),
        arg.target.npc.is_some(),
        arg.target.name.is_some(),
    ]
    .into_iter()
    .filter(|set| *set)
    .count()
        != 1
        || arg.settle_ms == Some(0)
    {
        return Err(CompileError::code("invalid-args"));
    }
    let tile = anchor_tile(arg.anchor.as_ref())?;
    let ambiguous = arg
        .target
        .name
        .as_deref()
        .is_some_and(|name| name_matches(cx, name, &arg.op) > 1);
    let kind = if let Some(ground) = arg.target.ground {
        let item = cx
            .selected
            .item_by_alias(&ground)
            .ok_or_else(|| CompileError::code("unresolved-obj").with_detail(ground.as_str()))?;
        let name = item.name.clone().unwrap_or(ground);
        reach::ReachKind::Ground {
            id: item.id,
            obj: Arc::from(name),
        }
    } else if let Some(loc) = arg.target.loc {
        let id = Some(resolve_loc(cx, &loc)?);
        offered(&cx.selected.loc_by_config(&loc).unwrap().ops, &arg.op)?;
        reach::ReachKind::Loc {
            id,
            name: Some(Arc::from(loc)),
        }
    } else if let Some(npc) = arg.target.npc {
        let id = resolve_npc(cx, &npc)?;
        offered(&cx.selected.npc_by_config(&npc).unwrap().ops, &arg.op)?;
        let display = cx
            .selected
            .npc_by_config(&npc)
            .and_then(|row| row.display.as_deref())
            .ok_or_else(|| CompileError::code("unresolved-npc").with_detail(npc.as_str()))?;
        reach::ReachKind::Npc {
            id,
            name: Arc::from(display),
        }
    } else if let Some(name) = arg.target.name {
        if name_matches(cx, &name, &arg.op) == 0 {
            return Err(CompileError::code("unresolved-name"));
        }
        reach::ReachKind::Name {
            name: Arc::from(name),
        }
    } else {
        return Err(CompileError::code("invalid-args"));
    };
    Ok(Arc::new(InteractPlan {
        kind,
        op: Arc::from(arg.op),
        tile,
        radius: arg.radius.max(1),
        wait_if_missing: arg.wait_if_missing,
        settle_ms: arg.settle_ms,
        ambiguous,
    }))
}

struct InteractPlan {
    kind: reach::ReachKind,
    op: Arc<str>,
    tile: Option<WorldTile>,
    radius: i32,
    wait_if_missing: bool,
    settle_ms: Option<u64>,
    ambiguous: bool,
}
impl StepPlan for InteractPlan {
    fn begin(&self, _cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        Ok(Box::new(InteractRun {
            kind: self.kind.clone(),
            op: Arc::clone(&self.op),
            tile: self.tile,
            radius: self.radius,
            wait_if_missing: self.wait_if_missing,
            deadline: None,
            missing_deadline: None,
            waiting: None,
            settle_duration: Duration::from_millis(self.settle_ms.unwrap_or(20_000)),
            walk: None,
            reach: None,
            started: false,
        }))
    }
    fn settle_timeout(&self) -> Duration {
        Duration::from_millis(self.settle_ms.unwrap_or(8_000))
    }
    fn compile_warning(&self) -> Option<&'static str> {
        self.ambiguous.then_some("ambiguous-name")
    }
}

struct InteractRun {
    kind: reach::ReachKind,
    op: Arc<str>,
    tile: Option<WorldTile>,
    radius: i32,
    wait_if_missing: bool,
    deadline: Option<Duration>,
    missing_deadline: Option<Duration>,
    waiting: Option<&'static str>,
    settle_duration: Duration,
    walk: Option<ActionHandle<Walk>>,
    reach: Option<ActionHandle<reach::Reach>>,
    started: bool,
}
impl StepRun for InteractRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        let available = reach::target_available(&cx.tick.cx, &self.kind, &self.op, self.radius)
            && (!matches!(self.kind, reach::ReachKind::Ground { .. })
                || cx.tick.cx.snapshot().inventory().is_some());
        if self.deadline.is_some_and(|d| cx.tick.cx.active_now() >= d) {
            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                "interact settle timeout",
            ))));
        }
        if let Some(handle) = &self.walk {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(receipt)) => {
                    walk_step_evidence(receipt)?;
                    self.walk = None;
                }
            }
        }
        if !self.started {
            if let Some(tile) = self.tile.filter(|_| !available) {
                let here = cx.tick.cx.snapshot().here();
                if !here.is_some_and(|obs| reach::within(obs.value, tile, self.radius)) {
                    self.walk = Some(cx.tick.actions.begin::<Walk>(
                        reach::walk_request(
                            tile,
                            self.radius.max(1) as u16,
                            None,
                            cx.required_after,
                        ),
                        &mut cx.tick.cx,
                    )?);
                    return Poll::Pending;
                }
            }
        }
        if self.deadline.is_none() {
            if available || !self.wait_if_missing {
                self.deadline = Some(cx.tick.cx.active_now() + self.settle_duration);
                self.missing_deadline = None;
                self.waiting = None;
            } else {
                self.waiting = Some(match &self.kind {
                    reach::ReachKind::Ground { id, .. } => {
                        match (
                            cx.tick.cx.snapshot().ground_items(),
                            cx.tick.cx.snapshot().inventory(),
                        ) {
                            (Some(rows), Some(_))
                                if !rows.value.iter().any(|row| row.def.id == *id) =>
                            {
                                "Waiting for ground spawn"
                            }
                            (Some(_), Some(_)) => "Waiting for ground target in reach",
                            _ => "Waiting for ground observation",
                        }
                    }
                    _ => "Waiting for target",
                });
                let deadline = self
                    .missing_deadline
                    .get_or_insert(cx.tick.cx.active_now() + Duration::from_secs(120));
                if cx.tick.cx.active_now() >= *deadline {
                    let (reason, name) = self
                        .waiting_for()
                        .map(|(reason, name)| (reason, name.as_ref()))
                        .unwrap_or(("Waiting for target", "target"));
                    return Poll::Ready(Err(ActionError::Unavailable(Arc::from(format!(
                        "{reason}: {name} (120s limit)"
                    )))));
                }
            }
        }
        if !self.started {
            self.reach = Some(cx.tick.actions.begin::<reach::Reach>(
                reach::ReachArgs {
                    kind: self.kind.clone(),
                    op: Arc::clone(&self.op),
                    anchor: self.tile,
                    radius: self.radius,
                    wait_if_missing: self.wait_if_missing,
                },
                &mut cx.tick.cx,
            )?);
            self.started = true;
            return Poll::Pending;
        }
        if let Some(handle) = &self.reach {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Ok(false)) => Poll::Ready(Err(ActionError::Failed(Arc::from(
                    "interact target not reached",
                )))),
                Poll::Ready(Ok(true)) => Poll::Ready(Ok(StepOutcome {
                    progress: None,
                    evidence: cx.tick.cx.evidence(),
                    receipt: None,
                })),
                Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            }
        } else {
            Poll::Ready(Ok(StepOutcome {
                progress: None,
                evidence: cx.tick.cx.evidence(),
                receipt: None,
            }))
        }
    }
    fn cancel(&mut self, _actions: &mut NativeActions) {
        self.walk = None;
        self.reach = None;
    }
    fn waiting_for(&self) -> Option<(&'static str, &Arc<str>)> {
        let name = match &self.kind {
            reach::ReachKind::Ground { obj, .. } => obj,
            reach::ReachKind::Npc { name, .. } | reach::ReachKind::Name { name } => name,
            reach::ReachKind::Loc { name, .. } => name.as_ref()?,
        };
        Some((self.waiting?, name))
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct UseOnTarget {
    /// Symbolic NPC config name; exactly one target selector is required.
    #[serde(default)]
    npc: Option<String>,
    /// Symbolic location config name; exactly one target selector is required.
    #[serde(default)]
    loc: Option<String>,
    /// Symbolic item config name; exactly one target selector is required.
    #[serde(default)]
    item: Option<String>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct UseOnUntil {
    /// Symbolic item config name whose count ends repetition.
    obj: String,
    /// Fixed or journal-count-backed quantity to reach.
    qty: s2::QuantityDocument,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct UseOnArgs {
    /// Symbolic item config name used on the target.
    item: String,
    /// NPC, location, or item target; exactly one is required.
    target: UseOnTarget,
    /// Optional authored approach anchor and source citation.
    #[serde(default)]
    anchor: Option<AnchorArg>,
    /// Maximum interaction distance; 0 or omitted means 1 tile.
    #[serde(default)]
    radius: i32,
    /// Optional symbolic product item expected from the action.
    #[serde(default)]
    product: Option<String>,
    /// Optional settle-window override in milliseconds; must be at least 1.
    #[serde(default)]
    #[cfg_attr(feature = "path-schema", schemars(range(min = 1)))]
    settle_ms: Option<u64>,
    /// Optional repeated-use goal, measured by an item count.
    #[serde(default)]
    until: Option<UseOnUntil>,
    /// Optional predicate that can end the repeated use-on attempt.
    #[serde(default)]
    no_product: Option<PredicateDocument>,
}

fn compile_use_on(
    arg: UseOnArgs,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    if [
        arg.target.npc.is_some(),
        arg.target.loc.is_some(),
        arg.target.item.is_some(),
    ]
    .into_iter()
    .filter(|set| *set)
    .count()
        != 1
        || arg.settle_ms == Some(0)
    {
        return Err(CompileError::code("invalid-args"));
    }
    let item_id = resolve_obj(cx, &arg.item)?;
    let item_name = cx
        .selected
        .item_by_alias(&arg.item)
        .and_then(|row| row.name.as_deref())
        .ok_or_else(|| CompileError::code("unresolved-obj").with_detail(arg.item.as_str()))?;
    let (kind, target_id, target_name) = if let Some(npc) = &arg.target.npc {
        let id = resolve_npc(cx, npc)?;
        let name = cx
            .selected
            .npc_by_config(npc)
            .and_then(|row| row.display.as_deref())
            .ok_or_else(|| CompileError::code("unresolved-npc").with_detail(npc.as_str()))?;
        ("npc", id, Arc::<str>::from(name))
    } else if let Some(loc) = &arg.target.loc {
        let id = resolve_loc(cx, loc)?;
        let name = cx
            .selected
            .loc_by_config(loc)
            .and_then(|row| row.display.as_deref())
            .ok_or_else(|| CompileError::code("unresolved-loc").with_detail(loc.as_str()))?;
        ("loc", id, Arc::<str>::from(name))
    } else if let Some(item) = &arg.target.item {
        let id = resolve_obj(cx, item)?;
        let name = cx
            .selected
            .item_by_alias(item)
            .and_then(|row| row.name.as_deref())
            .ok_or_else(|| CompileError::code("unresolved-obj").with_detail(item.as_str()))?;
        ("item", id, Arc::<str>::from(name))
    } else {
        return Err(CompileError::code("invalid-args"));
    };
    let tile = anchor_tile(arg.anchor.as_ref())?;
    let product = arg
        .product
        .as_deref()
        .map(|name| resolve_obj(cx, name))
        .transpose()?;
    let until = arg
        .until
        .map(|until| {
            if matches!(&until.qty, s2::QuantityDocument::Fixed(qty) if *qty < 1) {
                return Err(CompileError::code("invalid-args"));
            }
            Ok((
                resolve_obj(cx, &until.obj)?,
                s2::compile_quantity(until.qty, cx)?,
            ))
        })
        .transpose()?;
    let no_product = arg
        .no_product
        .as_ref()
        .map(|predicate| compile_predicate(predicate, cx))
        .transpose()?;
    Ok(Arc::new(UseOnPlan {
        item: Arc::from(item_name),
        item_id,
        kind: Arc::from(kind),
        target_id,
        target_name: Some(target_name),
        product,
        until,
        no_product,
        tile,
        radius: arg.radius.max(1),
        settle_ms: arg.settle_ms,
    }))
}

struct UseOnPlan {
    item: Arc<str>,
    item_id: i32,
    target_id: i32,
    product: Option<i32>,
    until: Option<(i32, s2::QuantityPlan)>,
    no_product: Option<Arc<dyn PredicatePlan>>,
    kind: Arc<str>,
    target_name: Option<Arc<str>>,
    tile: Option<WorldTile>,
    radius: i32,
    settle_ms: Option<u64>,
}
impl StepPlan for UseOnPlan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        let until = self
            .until
            .as_ref()
            .map(|(id, qty)| {
                qty.evaluate_step(cx).map(|qty| (*id, qty)).ok_or_else(|| {
                    ActionError::Unavailable(Arc::from("use_on quantity unavailable"))
                })
            })
            .transpose()?;
        Ok(Box::new(UseOnRun {
            item: Arc::clone(&self.item),
            item_id: self.item_id,
            target_id: self.target_id,
            product: self.product,
            until,
            no_product: self.no_product.clone(),
            kind: Arc::clone(&self.kind),
            target_name: self.target_name.clone(),
            tile: self.tile,
            radius: self.radius,
            deadline: None,
            settle_duration: Duration::from_millis(self.settle_ms.unwrap_or(20_000)),
            walk: None,
            interaction: None,
            round_before: None,
            round_deadline: None,
            accepted: false,
            chat_since: 0,
        }))
    }
    fn settle_timeout(&self) -> Duration {
        Duration::from_millis(self.settle_ms.unwrap_or(20_000))
    }
}

struct UseOnRun {
    item: Arc<str>,
    item_id: i32,
    target_id: i32,
    product: Option<i32>,
    until: Option<(i32, i32)>,
    no_product: Option<Arc<dyn PredicatePlan>>,
    kind: Arc<str>,
    target_name: Option<Arc<str>>,
    tile: Option<WorldTile>,
    radius: i32,
    deadline: Option<Duration>,
    settle_duration: Duration,
    walk: Option<ActionHandle<Walk>>,
    interaction: Option<ActionHandle<UseOnAction>>,
    accepted: bool,
    round_before: Option<i32>,
    round_deadline: Option<Duration>,
    chat_since: i32,
}
impl UseOnRun {
    fn clear_round(&mut self) {
        self.interaction = None;
        self.accepted = false;
        self.round_before = None;
        self.round_deadline = None;
    }
}

impl StepRun for UseOnRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        loop {
            let timed_out = self.deadline.is_some_and(|d| cx.tick.cx.active_now() >= d);
            if let Some(handle) = &self.walk {
                match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                    Poll::Ready(Ok(receipt))
                        if receipt.end == crate::native::WalkEnd::UserInput =>
                    {
                        return Poll::Ready(Err(ActionError::UserInput))
                    }
                    _ if timed_out => {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from("use_on timeout"))))
                    }
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                    Poll::Ready(Ok(receipt)) => {
                        walk_step_evidence(receipt)?;
                        self.walk = None;
                    }
                }
            }
            if timed_out {
                return Poll::Ready(Err(ActionError::Failed(Arc::from("use_on timeout"))));
            }
            if self.interaction.is_none() {
                if let Some((id, qty)) = self.until {
                    if cx.tick.cx.snapshot().inventory().is_some_and(|inventory| {
                        inventory
                            .value
                            .iter()
                            .filter(|item| item.def.id == id)
                            .map(|item| item.count)
                            .sum::<i32>()
                            >= qty
                    }) {
                        return Poll::Ready(Ok(StepOutcome {
                            progress: None,
                            evidence: cx.tick.cx.evidence(),
                            receipt: None,
                        }));
                    }
                }
                if let Some(tile) = self.tile {
                    let here = cx.tick.cx.snapshot().here();
                    if !here.is_some_and(|obs| reach::within(obs.value, tile, self.radius)) {
                        self.walk = Some(cx.tick.actions.begin::<Walk>(
                            reach::walk_request(
                                tile,
                                self.radius.max(1) as u16,
                                None,
                                cx.required_after,
                            ),
                            &mut cx.tick.cx,
                        )?);
                        return Poll::Pending;
                    }
                    // The anchor locates the initial search area. Once there,
                    // follow the observed target without re-entering that area.
                    self.tile = None;
                }
                if self.until.is_some()
                    && cx.tick.cx.snapshot().chat_modal().is_some_and(|chat| {
                        chat.value.root >= 0 && chat.value.continue_component_id >= 0
                    })
                {
                    self.interaction = Some(cx.tick.actions.begin::<UseOnAction>(
                        InteractReq::ContinueDialog { component_id: None },
                        &mut cx.tick.cx,
                    )?);
                    return Poll::Pending;
                }
                self.deadline
                    .get_or_insert(cx.tick.cx.active_now() + self.settle_duration);
                let snapshot = cx.tick.cx.snapshot();
                let Some(inventory) = snapshot.inventory() else {
                    return Poll::Pending;
                };
                let observed_id = self.until.map(|(id, _)| id).or(self.product);
                self.round_before = observed_id.map(|id| {
                    inventory
                        .value
                        .iter()
                        .filter(|row| row.def.id == id)
                        .map(|row| row.count)
                        .sum()
                });
                let Some(source) = inventory
                    .value
                    .iter()
                    .find(|row| row.def.id == self.item_id && row.count > 0)
                else {
                    return Poll::Ready(Err(ActionError::Failed(Arc::from(
                        "use_on source missing",
                    ))));
                };
                let (tile, index, target_item_slot) = match self.kind.as_ref() {
                    "loc" => {
                        let Some(loc) = reach::nearest_loc(
                            &cx.tick.cx,
                            Some(self.target_id),
                            None,
                            None,
                            self.radius,
                        ) else {
                            return Poll::Pending;
                        };
                        (loc.tile, None, None)
                    }
                    "npc" => {
                        let Some(npcs) = snapshot.npcs() else {
                            return Poll::Pending;
                        };
                        let Some(npc) = npcs
                            .value
                            .iter()
                            .filter(|row| {
                                row.r#type == Some(self.target_id as usize)
                                    && row.distance <= self.radius
                            })
                            .min_by_key(|row| row.distance)
                        else {
                            return Poll::Pending;
                        };
                        let tile = npc.tile;
                        let index = npc.index as i32;
                        if npc.distance > 1 {
                            self.walk = Some(cx.tick.actions.begin::<Walk>(
                                reach::walk_request(tile, 1, None, cx.required_after),
                                &mut cx.tick.cx,
                            )?);
                            return Poll::Pending;
                        }
                        (tile, Some(index), None)
                    }
                    _ => {
                        let Some(target) = inventory
                            .value
                            .iter()
                            .find(|row| row.def.id == self.target_id)
                        else {
                            return Poll::Pending;
                        };
                        (
                            WorldTile {
                                x: 0,
                                z: 0,
                                level: 0,
                            },
                            None,
                            Some(target.slot),
                        )
                    }
                };
                let request = InteractReq::UseOn {
                    name: self.item.to_string(),
                    // Compiled Path targets call inventory rows `item`; InteractReq uses `inv`.
                    kind: match self.kind.as_ref() {
                        "item" => "inv",
                        kind => kind,
                    }
                    .to_string(),
                    target_name: self.target_name.as_ref().map(|n| n.to_string()),
                    x: tile.x,
                    z: tile.z,
                    level: tile.level,
                    index,
                    source_item_id: Some(source.def.id),
                    source_item_slot: Some(source.slot),
                    target_item_id: (self.kind.as_ref() == "item").then_some(self.target_id),
                    target_item_slot,
                };
                self.interaction = Some(
                    cx.tick
                        .actions
                        .begin::<UseOnAction>(request, &mut cx.tick.cx)?,
                );
                return Poll::Pending;
            }
            if let Some(handle) = self.interaction.as_ref().filter(|_| !self.accepted) {
                match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                    Poll::Ready(Ok(chat_since)) => {
                        self.accepted = true;
                        self.chat_since = chat_since;
                        if self.until.is_some() && self.round_before.is_some() {
                            self.round_deadline = Some(
                                cx.tick.cx.active_now() + Duration::from_millis(USE_ON_ROUND_MS),
                            );
                        }
                    }
                }
            }
            if self.until.is_some() && self.round_before.is_none() {
                // ContinueDialog for an objbox/page is not a product round.
                self.clear_round();
                return Poll::Pending;
            }
            let pred = PredicateContext {
                cx: &cx.tick.cx,
                quests: cx.quests,
                progress: cx.progress,
                required_after: cx.required_after,
                chat_since: self.chat_since,
                outcome: None,
                bank: cx.bank,
            };
            if self
                .no_product
                .as_ref()
                .is_some_and(|predicate| predicate.evaluate(&pred) == Truth::True)
            {
                // Authored negative feedback completes a no-product round within
                // the same bounded until loop, rather than failing the whole step.
                if self.until.is_some() {
                    self.clear_round();
                    continue;
                }
                return Poll::Ready(Err(ActionError::Failed(Arc::from("use_on attempt failed"))));
            }
            let held = |id| {
                cx.tick.cx.snapshot().inventory().map(|inv| {
                    inv.value
                        .iter()
                        .filter(|row| row.def.id == id)
                        .map(|row| row.count)
                        .sum::<i32>()
                })
            };
            if let Some(before) = self.round_before {
                let observed_id = self.until.map(|(id, _)| id).or(self.product);
                if observed_id
                    .and_then(held)
                    .is_none_or(|count| count <= before)
                {
                    if self.until.is_some()
                        && self
                            .round_deadline
                            .is_some_and(|d| cx.tick.cx.active_now() >= d)
                    {
                        self.clear_round();
                        continue;
                    }
                    return Poll::Pending;
                }
            }
            if let Some((id, qty)) = self.until {
                if !held(id).is_some_and(|count| count >= qty) {
                    self.clear_round();
                    continue;
                }
            } else if self
                .product
                .is_some_and(|id| !held(id).is_some_and(|count| count > 0))
            {
                return Poll::Pending;
            }
            return Poll::Ready(Ok(StepOutcome {
                progress: None,
                evidence: cx.tick.cx.evidence(),
                receipt: None,
            }));
        }
    }
    fn cancel(&mut self, _actions: &mut NativeActions) {
        self.walk = None;
        self.interaction = None;
    }
}

struct UseOnAction {
    request: u64,
}
impl crate::native::NativeMachine for UseOnAction {
    type Args = InteractReq;
    type Output = i32;
    fn begin(
        request: Self::Args,
        cx: &mut crate::native::ActionContext<'_>,
    ) -> Result<Self, ActionError> {
        Ok(Self {
            request: cx.emit(request)?,
        })
    }
    fn poll(
        &mut self,
        cx: &mut crate::native::ActionContext<'_>,
    ) -> Poll<Result<i32, ActionError>> {
        match cx.interaction_receipt(self.request) {
            Some(receipt) if receipt.accepted => Poll::Ready(Ok(receipt.chat_since)),
            Some(_) => Poll::Ready(Err(ActionError::Failed(Arc::from(
                "use_on dispatch rejected",
            )))),
            None => Poll::Pending,
        }
    }
    fn cancel(&mut self) {}
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct AcquireArgs {
    /// Name of a recipe in the quest header's `acquire` table.
    recipe: String,
}

fn compile_acquire(
    arg: AcquireArgs,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    let steps = cx
        .recipes
        .get(&arg.recipe)
        .cloned()
        .ok_or_else(|| CompileError::code("unresolved-recipe"))?;
    Ok(Arc::new(AcquirePlan {
        recipe: Arc::from(arg.recipe),
        steps,
    }))
}

/// Filled by the compiler with the recipe's compiled steps.
pub struct AcquirePlan {
    pub recipe: Arc<str>,
    pub steps: Arc<[CompiledAcquireStep]>,
}

#[derive(Clone)]
pub struct CompiledAcquireStep {
    pub advances: bool,
    pub skip_if: Arc<dyn PredicatePlan>,
    pub settle: Arc<dyn PredicatePlan>,
    pub plan: Arc<dyn StepPlan>,
}

impl Default for AcquirePlan {
    fn default() -> Self {
        Self {
            recipe: Arc::from(""),
            steps: Arc::from([]),
        }
    }
}

impl StepPlan for AcquirePlan {
    fn begin(&self, _cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        Ok(Box::new(AcquireRun {
            steps: Arc::clone(&self.steps),
            current: None,
            child_outcome: None,
            index: 0,
            chat_since: 0,
            settling: false,
            settle_deadline: Duration::ZERO,
            waiting_for_read: false,
            selection_since: None,
            prayer_cleanup_owned: RaisedPrayers::empty(),
            clear_prayers: None,
        }))
    }
}

struct AcquireRun {
    steps: Arc<[CompiledAcquireStep]>,
    current: Option<Box<dyn StepRun>>,
    child_outcome: Option<StepOutcome>,
    index: usize,
    chat_since: i32,
    settling: bool,
    settle_deadline: Duration,
    waiting_for_read: bool,
    selection_since: Option<Duration>,
    prayer_cleanup_owned: RaisedPrayers,
    clear_prayers: Option<ActionHandle<ClearPrayers>>,
}
impl AcquireRun {
    fn poll_prayer_cleanup(
        &mut self,
        cx: &mut StepContext<'_, '_>,
    ) -> Poll<Result<(), ActionError>> {
        if let Some(handle) = self.clear_prayers.as_ref() {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(_)) => {
                    self.clear_prayers = None;
                    self.prayer_cleanup_owned = RaisedPrayers::empty();
                }
            }
        }
        if self.prayer_cleanup_owned.is_empty() {
            return Poll::Ready(Ok(()));
        }
        let selected = api::game_data::for_revision(cx.tick.cx.pin.revision)
            .map_err(|_| ActionError::Unavailable(Arc::from("prayer facts unavailable")))?;
        match begin_clear_owned_prayers(&selected, self.prayer_cleanup_owned, cx.tick) {
            Hygiene::Clean => {
                self.prayer_cleanup_owned = RaisedPrayers::empty();
                Poll::Ready(Ok(()))
            }
            Hygiene::Started(handle) => {
                self.clear_prayers = Some(handle);
                Poll::Pending
            }
            Hygiene::Deferred => Poll::Pending,
            Hygiene::Failed(error) => Poll::Ready(Err(error)),
        }
    }
}
impl StepRun for AcquireRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        match self.poll_prayer_cleanup(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Ready(Ok(())) => {}
        }
        if self.waiting_for_read {
            return Poll::Pending;
        }
        if self.settling {
            let truth = self.steps[self.index].settle.evaluate(&PredicateContext {
                cx: &cx.tick.cx,
                quests: cx.quests,
                progress: cx.progress,
                required_after: cx.required_after,
                chat_since: self.chat_since,
                outcome: self.child_outcome.as_ref(),
                bank: cx.bank,
            });
            if truth != Truth::True {
                if cx.tick.cx.active_now() >= self.settle_deadline {
                    return Poll::Ready(Err(ActionError::Failed(Arc::from(
                        "acquire settle timeout",
                    ))));
                }
                return Poll::Pending;
            }
            self.settling = false;
            self.index += 1;
        }
        if self.current.is_none() {
            while self.index < self.steps.len() {
                let skip = self.steps[self.index].skip_if.evaluate(&PredicateContext {
                    cx: &cx.tick.cx,
                    quests: cx.quests,
                    progress: cx.progress,
                    required_after: cx.required_after,
                    chat_since: reach::last_chat_seq(&cx.tick.cx),
                    outcome: None,
                    bank: cx.bank,
                });
                if skip == Truth::Unknown {
                    let since = self.selection_since.get_or_insert(cx.tick.cx.active_now());
                    if cx.tick.cx.active_now().saturating_sub(*since) >= Duration::from_secs(16) {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "acquire skip predicate evidence unavailable",
                        ))));
                    }
                    return Poll::Pending;
                }
                self.selection_since = None;
                if skip == Truth::True {
                    self.index += 1;
                    continue;
                }
                self.chat_since = reach::last_chat_seq(&cx.tick.cx);
                let run = self.steps[self.index].plan.begin(cx)?;
                self.current = Some(run);
                self.child_outcome = None;
                break;
            }
            if self.current.is_none() {
                return Poll::Ready(Ok(StepOutcome {
                    progress: None,
                    evidence: cx.tick.cx.evidence(),
                    receipt: None,
                }));
            }
        }
        match self.current.as_mut().unwrap().poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Ready(Ok(outcome)) => {
                self.prayer_cleanup_owned
                    .merge(self.current.as_ref().unwrap().prayer_cleanup());
                self.current = None;
                self.child_outcome = Some(outcome);
                self.settling = true;
                self.settle_deadline =
                    cx.tick.cx.active_now() + self.steps[self.index].plan.settle_timeout();
                if self.steps[self.index].advances {
                    self.waiting_for_read = true;
                }
                // Publish the child receipt before evaluating settle/next-child facts.
                Poll::Pending
            }
        }
    }
    fn needs_progress_read(&self) -> bool {
        self.waiting_for_read
    }
    fn progress_read_completed(&mut self, now: Duration) {
        self.waiting_for_read = false;
        self.settle_deadline = now + self.steps[self.index].plan.settle_timeout();
    }
    fn cancel(&mut self, actions: &mut NativeActions) {
        if let Some(run) = &mut self.current {
            run.cancel(actions);
        }
        self.clear_prayers = None;
    }
    fn prayer_cleanup(&self) -> RaisedPrayers {
        let mut owned = self.prayer_cleanup_owned;
        if let Some(run) = self.current.as_ref() {
            owned.merge(run.prayer_cleanup());
        }
        owned
    }
    fn waiting_for(&self) -> Option<(&'static str, &Arc<str>)> {
        self.current.as_ref()?.waiting_for()
    }
    fn in_flight_outcome(&self) -> Option<&StepOutcome> {
        self.current
            .as_ref()
            .and_then(|run| run.in_flight_outcome())
            .or(self.child_outcome.as_ref())
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct WaitArgs {
    /// Predicate evaluated against the latest cached evidence.
    until: PredicateDocument,
    /// Maximum ticks to wait; must be at least 1.
    #[cfg_attr(feature = "path-schema", schemars(range(min = 1)))]
    max_ticks: u64,
}

fn compile_wait(arg: WaitArgs, cx: &CompileContext<'_>) -> Result<Arc<dyn StepPlan>, CompileError> {
    if arg.max_ticks == 0 {
        return Err(CompileError::code("invalid-args"));
    }
    Ok(Arc::new(WaitPlan {
        until: compile_predicate(&arg.until, cx)?,
        max_ticks: arg.max_ticks,
    }))
}

struct WaitPlan {
    until: Arc<dyn PredicatePlan>,
    max_ticks: u64,
}
impl StepPlan for WaitPlan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        Ok(Box::new(WaitRun {
            until: Arc::clone(&self.until),
            deadline_tick: cx.tick.cx.evidence().tick.saturating_add(self.max_ticks),
            chat_since: reach::last_chat_seq(&cx.tick.cx),
        }))
    }
}

struct WaitRun {
    until: Arc<dyn PredicatePlan>,
    deadline_tick: u64,
    chat_since: i32,
}
impl StepRun for WaitRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        if cx.tick.cx.evidence().tick >= self.deadline_tick {
            return Poll::Ready(Err(ActionError::Failed(Arc::from("wait exhausted"))));
        }
        let pred = PredicateContext {
            cx: &cx.tick.cx,
            quests: cx.quests,
            progress: cx.progress,
            required_after: cx.required_after,
            chat_since: self.chat_since,
            outcome: None,
            bank: cx.bank,
        };
        if self.until.evaluate(&pred) != Truth::True {
            return Poll::Pending;
        }
        Poll::Ready(Ok(StepOutcome {
            progress: None,
            evidence: cx.tick.cx.evidence(),
            receipt: None,
        }))
    }
    fn cancel(&mut self, _actions: &mut NativeActions) {}
}

#[cfg(test)]
pub(crate) mod tests;
