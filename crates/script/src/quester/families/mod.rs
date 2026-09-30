//! S1 step families and predicate plans.

pub mod dialogue;
pub mod progress_predicates;
pub mod reach;

use super::compile::{
    CompileContext, CompileError, PredicateContext, PredicatePlan, StepContext, StepOutcome,
    StepPlan, StepRun,
};
use super::path::PredicateDocument;
use crate::native::walk::Walk;
use crate::native::{ActionError, ActionHandle, NativeActions, WalkEnd, WalkReceipt};
use crate::shim::InteractReq;
use api::selected::Truth;
use api::snapshot::{ChatLineView, QuestListStatus};
use api::WorldTile;
use serde::Deserialize;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

pub fn handlers() -> &'static [super::compile::StepHandler] {
    &[
        super::compile::StepHandler {
            kind: "walk",
            version: 1,
            compile: compile_walk,
        },
        super::compile::StepHandler {
            kind: "talk",
            version: 1,
            compile: compile_talk,
        },
        super::compile::StepHandler {
            kind: "interact",
            version: 1,
            compile: compile_interact,
        },
        super::compile::StepHandler {
            kind: "use_on",
            version: 1,
            compile: compile_use_on,
        },
        super::compile::StepHandler {
            kind: "acquire",
            version: 1,
            compile: compile_acquire,
        },
        super::compile::StepHandler {
            kind: "wait",
            version: 1,
            compile: compile_wait,
        },
    ]
}

pub fn predicate_handlers() -> &'static [super::compile::PredicateHandler] {
    &[
        super::compile::PredicateHandler {
            kind: "has_item",
            version: 1,
            compile: compile_has_item,
        },
        super::compile::PredicateHandler {
            kind: "near",
            version: 1,
            compile: compile_near,
        },
        super::compile::PredicateHandler {
            kind: "message",
            version: 1,
            compile: compile_message,
        },
        super::compile::PredicateHandler {
            kind: "message_state",
            version: 1,
            compile: compile_message_state,
        },
        super::compile::PredicateHandler {
            kind: "quest_colour",
            version: 1,
            compile: compile_quest_colour,
        },
        super::compile::PredicateHandler {
            kind: "in_combat",
            version: 1,
            compile: compile_in_combat,
        },
        super::compile::PredicateHandler {
            kind: "npc_present",
            version: 1,
            compile: compile_npc_present,
        },
        super::compile::PredicateHandler {
            kind: "loc_present",
            version: 1,
            compile: compile_loc_present,
        },
        super::compile::PredicateHandler {
            kind: "ground_item_near",
            version: 1,
            compile: compile_ground_item_near,
        },
        super::compile::PredicateHandler {
            kind: "modal_open",
            version: 1,
            compile: compile_modal_open,
        },
        super::compile::PredicateHandler {
            kind: "item_count_at_least",
            version: 1,
            compile: compile_item_count_at_least,
        },
        super::compile::PredicateHandler {
            kind: "worn",
            version: 1,
            compile: compile_worn,
        },
        super::compile::PredicateHandler {
            kind: "on_level",
            version: 1,
            compile: compile_on_level,
        },
        super::compile::PredicateHandler {
            kind: "in_area",
            version: 1,
            compile: compile_in_area,
        },
        super::compile::PredicateHandler {
            kind: "npc_absent",
            version: 1,
            compile: compile_npc_absent,
        },
        super::compile::PredicateHandler {
            kind: "skill_at_least",
            version: 1,
            compile: compile_skill_at_least,
        },
        super::compile::PredicateHandler {
            kind: "hp_fraction_below",
            version: 1,
            compile: compile_hp_fraction_below,
        },
        super::compile::PredicateHandler {
            kind: "prayer_points_at_least",
            version: 1,
            compile: compile_prayer_points,
        },
        super::compile::PredicateHandler {
            kind: "stage_in",
            version: 1,
            compile: progress_predicates::compile_stage_in,
        },
        super::compile::PredicateHandler {
            kind: "flag",
            version: 1,
            compile: progress_predicates::compile_flag,
        },
    ]
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
            let handler = predicate_handlers()
                .iter()
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
        .ok_or_else(|| CompileError::code("unresolved-obj"))
}

fn resolve_npc(cx: &CompileContext<'_>, alias: &str) -> Result<i32, CompileError> {
    cx.selected
        .npc_by_config(alias)
        .map(|row| row.id)
        .ok_or_else(|| CompileError::code("unresolved-npc"))
}

fn resolve_loc(cx: &CompileContext<'_>, alias: &str) -> Result<i32, CompileError> {
    cx.selected
        .loc_by_config(alias)
        .map(|row| row.id)
        .ok_or_else(|| CompileError::code("unresolved-loc"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObjArg {
    obj: String,
}

fn compile_has_item(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let arg: ObjArg =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NearArg {
    tile: [i32; 3],
    radius: i32,
}

fn compile_near(
    args: &serde_json::Value,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let arg: NearArg =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageArg {
    any: Vec<String>,
}

fn compile_message(
    args: &serde_json::Value,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let arg: MessageArg =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
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
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageStateArg {
    set: Vec<String>,
    clear: Vec<String>,
}
fn compile_message_state(
    args: &serde_json::Value,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let arg: MessageStateArg =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ColourArg {
    quest: String,
    is: String,
}

fn compile_quest_colour(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let arg: ColourArg =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
    let facts = cx
        .quests
        .quest(&arg.quest)
        .map_err(|_| CompileError::code("unresolved-quest"))?;
    let want = match arg.is.as_str() {
        "not_started" => QuestListStatus::NotStarted,
        "in_progress" => QuestListStatus::InProgress,
        "complete" => QuestListStatus::Complete,
        _ => return Err(CompileError::code("invalid-args")),
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
    args: &serde_json::Value,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    if !args.as_object().is_some_and(serde_json::Map::is_empty) {
        return Err(CompileError::code("invalid-args"));
    }
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NpcArg {
    npc: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LocArg {
    loc: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GroundArg {
    obj: String,
    #[serde(default)]
    radius: Option<i32>,
}

fn compile_npc_present(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let arg: NpcArg =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
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
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let present = compile_npc_present(args, cx)?;
    Ok(Arc::new(NotPlan { inner: present }))
}

fn compile_loc_present(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let arg: LocArg =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
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
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let arg: GroundArg =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
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
    args: &serde_json::Value,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    if !args.as_object().is_some_and(serde_json::Map::is_empty) {
        return Err(CompileError::code("invalid-args"));
    }
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CountArg {
    obj: String,
    #[serde(default)]
    qty: Option<i32>,
}

fn compile_item_count_at_least(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let arg: CountArg =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
    Ok(Arc::new(CountAtLeast {
        id: resolve_obj(cx, &arg.obj)?,
        qty: arg.qty.unwrap_or(1),
    }))
}
struct CountAtLeast {
    id: i32,
    qty: i32,
}
impl PredicatePlan for CountAtLeast {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        match cx.cx.snapshot().inventory() {
            None => Truth::Unknown,
            Some(inv) => {
                let n: i32 = inv
                    .value
                    .iter()
                    .filter(|item| item.def.id == self.id)
                    .map(|item| item.count)
                    .sum();
                truth(n >= self.qty)
            }
        }
    }
}

fn compile_worn(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let arg: ObjArg =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LevelArg {
    level: i32,
}

fn compile_on_level(
    args: &serde_json::Value,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let arg: LevelArg =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AreaArg {
    area: String,
}

fn compile_in_area(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let arg: AreaArg =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillArg {
    skill: String,
    level: i32,
}

fn compile_skill_at_least(
    args: &serde_json::Value,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let arg: SkillArg =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FractionArg {
    below: f32,
}

fn compile_hp_fraction_below(
    args: &serde_json::Value,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let arg: FractionArg =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
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
    args: &serde_json::Value,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let arg: SkillArg =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WalkArgs {
    tile: [i32; 3],
    #[serde(default)]
    source: String,
    #[serde(default)]
    radius: u16,
}

fn compile_walk(
    args: &serde_json::Value,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    let arg: WalkArgs =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
    validate_tile(arg.tile, &arg.source)?;
    Ok(Arc::new(WalkPlan {
        tile: WorldTile {
            x: arg.tile[0],
            z: arg.tile[1],
            level: arg.tile[2],
        },
        radius: arg.radius.max(1),
    }))
}

struct WalkPlan {
    tile: WorldTile,
    radius: u16,
}
impl StepPlan for WalkPlan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        let handle = cx.tick.actions.begin::<Walk>(
            reach::walk_request(self.tile, self.radius, cx.required_after),
            &mut cx.tick.cx,
        )?;
        Ok(Box::new(WalkRun { handle }))
    }
}

struct WalkRun {
    handle: ActionHandle<Walk>,
}
impl StepRun for WalkRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        match cx.tick.actions.poll(&self.handle, &mut cx.tick.cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(WalkReceipt {
                end: WalkEnd::Arrived,
                evidence,
                ..
            }))
            | Poll::Ready(Ok(WalkReceipt {
                end: WalkEnd::RouteEnded,
                evidence,
                ..
            })) => Poll::Ready(Ok(StepOutcome {
                progress: None,
                evidence,
                receipt: None,
            })),
            Poll::Ready(Ok(WalkReceipt {
                end: WalkEnd::NeedsEvidence(gates),
                ..
            })) => Poll::Ready(Err(ActionError::Failed(Arc::from(format!(
                "walk needs live quest evidence: {gates:?}"
            ))))),
            Poll::Ready(Ok(_)) => Poll::Ready(Err(ActionError::Failed(Arc::from("walk failed")))),
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
        }
    }
    fn cancel(&mut self, _actions: &mut NativeActions) {}
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnchorArg {
    tile: [i32; 3],
    #[serde(default)]
    source: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TalkArgs {
    npc: String,
    #[serde(default)]
    anchor: Option<AnchorArg>,
    #[serde(default)]
    leash: u16,
    #[serde(default)]
    prefer: Vec<String>,
    #[serde(default)]
    choose: Option<i32>,
}

fn compile_talk(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    let arg: TalkArgs =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
    let id = resolve_npc(cx, &arg.npc)?;
    let display = cx
        .selected
        .npc_by_config(&arg.npc)
        .and_then(|row| row.display.as_deref())
        .ok_or_else(|| CompileError::code("unresolved-npc"))?;
    let tile = anchor_tile(arg.anchor.as_ref())?;
    offered(&cx.selected.npc_by_config(&arg.npc).unwrap().ops, "Talk-to")?;
    Ok(Arc::new(TalkPlan {
        id,
        npc: Arc::from(display),
        tile,
        leash: arg.leash.max(1),
        prefer: arg.prefer.into_iter().map(Arc::from).collect(),
        choose: arg.choose,
    }))
}

struct TalkPlan {
    id: i32,
    npc: Arc<str>,
    tile: Option<WorldTile>,
    leash: u16,
    prefer: Arc<[Arc<str>]>,
    choose: Option<i32>,
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
                Poll::Ready(Ok(_)) => self.walk = None,
            }
        }
        if !self.started {
            if let Some(tile) = self.tile {
                let here = cx.tick.cx.snapshot().here();
                let near =
                    here.is_some_and(|obs| reach::within(obs.value, tile, i32::from(self.leash)));
                if !near {
                    self.walk = Some(cx.tick.actions.begin::<Walk>(
                        reach::walk_request(tile, self.leash, cx.required_after),
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
                Poll::Ready(Ok(false)) => {
                    Poll::Ready(Err(ActionError::Failed(Arc::from("dialogue failed"))))
                }
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
        self.dialogue = None;
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InteractTarget {
    #[serde(default)]
    ground: Option<String>,
    #[serde(default)]
    loc: Option<String>,
    #[serde(default)]
    npc: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InteractArgs {
    target: InteractTarget,
    op: String,
    #[serde(default)]
    anchor: Option<AnchorArg>,
    #[serde(default)]
    radius: i32,
    #[serde(default)]
    wait_if_missing: bool,
    #[serde(default)]
    settle_ms: Option<u64>,
}

fn compile_interact(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    let arg: InteractArgs =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
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
            .ok_or_else(|| CompileError::code("unresolved-obj"))?;
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
            .ok_or_else(|| CompileError::code("unresolved-npc"))?;
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
                Poll::Ready(Ok(_)) => self.walk = None,
            }
        }
        if !self.started {
            if let Some(tile) = self.tile.filter(|_| !available) {
                let here = cx.tick.cx.snapshot().here();
                if !here.is_some_and(|obs| reach::within(obs.value, tile, self.radius)) {
                    self.walk = Some(cx.tick.actions.begin::<Walk>(
                        reach::walk_request(tile, self.radius.max(1) as u16, cx.required_after),
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UseOnTarget {
    #[serde(default)]
    npc: Option<String>,
    #[serde(default)]
    loc: Option<String>,
    #[serde(default)]
    item: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UseOnArgs {
    item: String,
    target: UseOnTarget,
    #[serde(default)]
    anchor: Option<AnchorArg>,
    #[serde(default)]
    radius: i32,
    #[serde(default)]
    product: Option<String>,
    #[serde(default)]
    settle_ms: Option<u64>,
}

fn compile_use_on(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    let arg: UseOnArgs =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
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
        .ok_or_else(|| CompileError::code("unresolved-obj"))?;
    let (kind, target_id, target_name) = if let Some(npc) = &arg.target.npc {
        let id = resolve_npc(cx, npc)?;
        let name = cx
            .selected
            .npc_by_config(npc)
            .and_then(|row| row.display.as_deref())
            .ok_or_else(|| CompileError::code("unresolved-npc"))?;
        ("npc", id, Arc::<str>::from(name))
    } else if let Some(loc) = &arg.target.loc {
        let id = resolve_loc(cx, loc)?;
        let name = cx
            .selected
            .loc_by_config(loc)
            .and_then(|row| row.display.as_deref())
            .ok_or_else(|| CompileError::code("unresolved-loc"))?;
        ("loc", id, Arc::<str>::from(name))
    } else if let Some(item) = &arg.target.item {
        let id = resolve_obj(cx, item)?;
        let name = cx
            .selected
            .item_by_alias(item)
            .and_then(|row| row.name.as_deref())
            .ok_or_else(|| CompileError::code("unresolved-obj"))?;
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
    Ok(Arc::new(UseOnPlan {
        item: Arc::from(item_name),
        item_id,
        kind: Arc::from(kind),
        target_id,
        target_name: Some(target_name),
        product,
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
    kind: Arc<str>,
    target_name: Option<Arc<str>>,
    tile: Option<WorldTile>,
    radius: i32,
    settle_ms: Option<u64>,
}
impl StepPlan for UseOnPlan {
    fn begin(&self, _cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        Ok(Box::new(UseOnRun {
            item: Arc::clone(&self.item),
            item_id: self.item_id,
            target_id: self.target_id,
            product: self.product,
            kind: Arc::clone(&self.kind),
            target_name: self.target_name.clone(),
            tile: self.tile,
            radius: self.radius,
            deadline: None,
            settle_duration: Duration::from_millis(self.settle_ms.unwrap_or(20_000)),
            walk: None,
            interaction: None,
            accepted: false,
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
    kind: Arc<str>,
    target_name: Option<Arc<str>>,
    tile: Option<WorldTile>,
    radius: i32,
    deadline: Option<Duration>,
    settle_duration: Duration,
    walk: Option<ActionHandle<Walk>>,
    interaction: Option<ActionHandle<UseOnAction>>,
    accepted: bool,
}
impl StepRun for UseOnRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        if self.deadline.is_some_and(|d| cx.tick.cx.active_now() >= d) {
            return Poll::Ready(Err(ActionError::Failed(Arc::from("use_on timeout"))));
        }
        if let Some(handle) = &self.walk {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(_)) => self.walk = None,
            }
        }
        if self.interaction.is_none() {
            if let Some(tile) = self.tile {
                let here = cx.tick.cx.snapshot().here();
                if !here.is_some_and(|obs| reach::within(obs.value, tile, self.radius)) {
                    self.walk = Some(cx.tick.actions.begin::<Walk>(
                        reach::walk_request(tile, self.radius.max(1) as u16, cx.required_after),
                        &mut cx.tick.cx,
                    )?);
                    return Poll::Pending;
                }
            }
            self.deadline
                .get_or_insert(cx.tick.cx.active_now() + self.settle_duration);
            let snapshot = cx.tick.cx.snapshot();
            let Some(inventory) = snapshot.inventory() else {
                return Poll::Pending;
            };
            let Some(source) = inventory
                .value
                .iter()
                .find(|row| row.def.id == self.item_id && row.count > 0)
            else {
                return Poll::Ready(Err(ActionError::Failed(Arc::from("use_on source missing"))));
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
                    (npc.tile, Some(npc.index as i32), None)
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
                kind: self.kind.to_string(),
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
                Poll::Ready(Ok(())) => self.accepted = true,
            }
        }
        if self.product.is_some_and(|id| {
            !cx.tick.cx.snapshot().inventory().is_some_and(|inv| {
                inv.value
                    .iter()
                    .any(|row| row.def.id == id && row.count > 0)
            })
        }) {
            return Poll::Pending;
        }
        Poll::Ready(Ok(StepOutcome {
            progress: None,
            evidence: cx.tick.cx.evidence(),
            receipt: None,
        }))
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
    type Output = ();
    fn begin(
        request: Self::Args,
        cx: &mut crate::native::ActionContext<'_>,
    ) -> Result<Self, ActionError> {
        Ok(Self {
            request: cx.emit(request)?,
        })
    }
    fn poll(&mut self, cx: &mut crate::native::ActionContext<'_>) -> Poll<Result<(), ActionError>> {
        match cx.interaction_receipt(self.request) {
            Some(receipt) if receipt.accepted => Poll::Ready(Ok(())),
            Some(_) => Poll::Ready(Err(ActionError::Failed(Arc::from(
                "use_on dispatch rejected",
            )))),
            None => Poll::Pending,
        }
    }
    fn cancel(&mut self) {}
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AcquireArgs {
    recipe: String,
}

fn compile_acquire(
    args: &serde_json::Value,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    let arg: AcquireArgs =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
    Ok(Arc::new(AcquirePlan {
        recipe: Arc::from(arg.recipe),
        steps: Vec::new(),
    }))
}

/// Filled by the compiler with the recipe's compiled steps.
pub struct AcquirePlan {
    pub recipe: Arc<str>,
    pub steps: Vec<CompiledAcquireStep>,
}

#[derive(Clone)]
pub struct CompiledAcquireStep {
    pub skip_if: Arc<dyn PredicatePlan>,
    pub settle: Arc<dyn PredicatePlan>,
    pub plan: Arc<dyn StepPlan>,
}

impl Default for AcquirePlan {
    fn default() -> Self {
        Self {
            recipe: Arc::from(""),
            steps: Vec::new(),
        }
    }
}

impl StepPlan for AcquirePlan {
    fn begin(&self, _cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        Ok(Box::new(AcquireRun {
            steps: self.steps.clone(),
            current: None,
            index: 0,
            chat_since: 0,
            settling: false,
            settle_deadline: Duration::ZERO,
        }))
    }
}

struct AcquireRun {
    steps: Vec<CompiledAcquireStep>,
    current: Option<Box<dyn StepRun>>,
    index: usize,
    chat_since: i32,
    settling: bool,
    settle_deadline: Duration,
}
impl StepRun for AcquireRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        loop {
            if self.settling {
                let truth = self.steps[self.index].settle.evaluate(&PredicateContext {
                    cx: &cx.tick.cx,
                    quests: cx.quests,
                    progress: cx.progress,
                    required_after: cx.required_after,
                    chat_since: self.chat_since,
                    outcome: None,
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
                    });
                    if skip == Truth::True {
                        self.index += 1;
                        continue;
                    }
                    self.chat_since = reach::last_chat_seq(&cx.tick.cx);
                    let run = self.steps[self.index].plan.begin(cx)?;
                    self.current = Some(run);
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
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(_)) => {
                    self.current = None;
                    self.settling = true;
                    self.settle_deadline =
                        cx.tick.cx.active_now() + self.steps[self.index].plan.settle_timeout();
                }
            }
        }
    }
    fn cancel(&mut self, actions: &mut NativeActions) {
        if let Some(run) = &mut self.current {
            run.cancel(actions);
        }
    }
    fn waiting_for(&self) -> Option<(&'static str, &Arc<str>)> {
        self.current.as_ref()?.waiting_for()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitArgs {
    until: PredicateDocument,
    max_ticks: u64,
}

fn compile_wait(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    let arg: WaitArgs =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
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
