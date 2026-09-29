//! S1 step families and predicate plans.

pub mod dialogue;
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
            kind: "item_count_grew",
            version: 1,
            compile: compile_item_count_grew,
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
struct MessageArg {
    any: Vec<String>,
}

fn compile_message(
    args: &serde_json::Value,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let arg: MessageArg =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
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
        let Some(lines) = snapshot.chat_lines(cx.required_after.sequence as i32) else {
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
    let lower = line.text.to_ascii_lowercase();
    needles.iter().any(|needle| lower.contains(needle.as_str()))
}

#[derive(Deserialize)]
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
    _args: &serde_json::Value,
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

#[derive(Deserialize)]
#[allow(dead_code)]
struct NameArg {
    #[serde(default)]
    npc: Option<String>,
    #[serde(default)]
    loc: Option<String>,
    #[serde(default)]
    obj: Option<String>,
    #[serde(default)]
    radius: Option<i32>,
    #[serde(default)]
    name: Option<String>,
}

fn compile_npc_present(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let arg: NameArg =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
    let alias = arg.npc.ok_or_else(|| CompileError::code("invalid-args"))?;
    let _ = resolve_npc(cx, &alias)?;
    Ok(Arc::new(NpcPresent {
        name: Arc::from(alias),
    }))
}
struct NpcPresent {
    name: Arc<str>,
}
impl PredicatePlan for NpcPresent {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        match cx.cx.snapshot().npcs() {
            None => Truth::Unknown,
            Some(npcs) => truth(npcs.value.iter().any(|npc| {
                npc.name
                    .as_deref()
                    .is_some_and(|n| n.eq_ignore_ascii_case(&self.name))
            })),
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
    let arg: NameArg =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
    let alias = arg.loc.ok_or_else(|| CompileError::code("invalid-args"))?;
    let id = resolve_loc(cx, &alias)?;
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
    let arg: NameArg =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
    let alias = arg.obj.ok_or_else(|| CompileError::code("invalid-args"))?;
    Ok(Arc::new(GroundNear {
        id: resolve_obj(cx, &alias)?,
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
    _args: &serde_json::Value,
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

#[derive(Deserialize)]
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

fn compile_item_count_grew(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    compile_has_item(args, cx)
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
        let Some(combat) = cx.cx.snapshot().in_combat() else {
            return Truth::Unknown;
        };
        let _ = combat;
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

#[derive(Deserialize)]
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
    if arg.source.is_empty() {
        return Err(CompileError::code("tile-source-required"));
    }
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
                evidence,
                ..
            })) => {
                let _ = gates;
                Poll::Ready(Ok(StepOutcome {
                    progress: None,
                    evidence,
                    receipt: None,
                }))
            }
            Poll::Ready(Ok(_)) => Poll::Ready(Err(ActionError::Failed(Arc::from("walk failed")))),
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
        }
    }
    fn cancel(&mut self, _actions: &mut NativeActions) {}
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct AnchorArg {
    tile: [i32; 3],
    #[serde(default)]
    source: String,
}

#[derive(Deserialize)]
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
    let _ = resolve_npc(cx, &arg.npc)?;
    let tile = arg.anchor.as_ref().map(|a| WorldTile {
        x: a.tile[0],
        z: a.tile[1],
        level: a.tile[2],
    });
    Ok(Arc::new(TalkPlan {
        npc: Arc::from(arg.npc),
        tile,
        leash: arg.leash.max(1),
        prefer: arg.prefer.into_iter().map(Arc::from).collect(),
        choose: arg.choose,
    }))
}

struct TalkPlan {
    npc: Arc<str>,
    tile: Option<WorldTile>,
    leash: u16,
    prefer: Arc<[Arc<str>]>,
    choose: Option<i32>,
}
impl StepPlan for TalkPlan {
    fn begin(&self, _cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        Ok(Box::new(TalkRun {
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
                Poll::Ready(Ok(_)) => Poll::Ready(Ok(StepOutcome {
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
        let id = resolve_loc(cx, &loc).ok();
        reach::ReachKind::Loc {
            id,
            name: Some(Arc::from(loc)),
        }
    } else if let Some(npc) = arg.target.npc {
        let _ = resolve_npc(cx, &npc)?;
        reach::ReachKind::Npc {
            name: Arc::from(npc),
            index: None,
        }
    } else if let Some(name) = arg.target.name {
        reach::ReachKind::Name {
            name: Arc::from(name),
        }
    } else {
        return Err(CompileError::code("invalid-args"));
    };
    let tile = arg.anchor.map(|a| WorldTile {
        x: a.tile[0],
        z: a.tile[1],
        level: a.tile[2],
    });
    Ok(Arc::new(InteractPlan {
        kind,
        op: Arc::from(arg.op),
        tile,
        radius: arg.radius.max(1),
        wait_if_missing: arg.wait_if_missing,
        settle_ms: arg.settle_ms,
    }))
}

struct InteractPlan {
    kind: reach::ReachKind,
    op: Arc<str>,
    tile: Option<WorldTile>,
    radius: i32,
    wait_if_missing: bool,
    settle_ms: Option<u64>,
}
impl StepPlan for InteractPlan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        Ok(Box::new(InteractRun {
            kind: self.kind.clone(),
            op: Arc::clone(&self.op),
            tile: self.tile,
            radius: self.radius,
            wait_if_missing: self.wait_if_missing,
            deadline: Some(
                cx.tick.cx.active_now() + Duration::from_millis(self.settle_ms.unwrap_or(20_000)),
            ),
            walk: None,
            reach: None,
            started: false,
        }))
    }
}

struct InteractRun {
    kind: reach::ReachKind,
    op: Arc<str>,
    tile: Option<WorldTile>,
    radius: i32,
    wait_if_missing: bool,
    deadline: Option<Duration>,
    walk: Option<ActionHandle<Walk>>,
    reach: Option<ActionHandle<reach::Reach>>,
    started: bool,
}
impl StepRun for InteractRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
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
                Poll::Ready(Ok(_)) => Poll::Ready(Ok(StepOutcome {
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
}

#[derive(Deserialize)]
struct UseOnTarget {
    #[serde(default)]
    npc: Option<String>,
    #[serde(default)]
    loc: Option<String>,
    #[serde(default)]
    item: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
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
    settle_message: Option<Vec<String>>,
    #[serde(default)]
    settle_ms: Option<u64>,
}

fn compile_use_on(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    let arg: UseOnArgs =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
    let _ = resolve_obj(cx, &arg.item)?;
    if let Some(npc) = &arg.target.npc {
        let _ = resolve_npc(cx, npc)?;
    }
    if let Some(loc) = &arg.target.loc {
        let _ = resolve_loc(cx, loc)?;
    }
    if let Some(item) = &arg.target.item {
        let _ = resolve_obj(cx, item)?;
    }
    if let Some(product) = &arg.product {
        let _ = resolve_obj(cx, product)?;
    }
    let tile = arg.anchor.map(|a| WorldTile {
        x: a.tile[0],
        z: a.tile[1],
        level: a.tile[2],
    });
    let kind = if arg.target.npc.is_some() {
        "npc"
    } else if arg.target.loc.is_some() {
        "loc"
    } else {
        "item"
    };
    Ok(Arc::new(UseOnPlan {
        item: Arc::from(arg.item),
        kind: Arc::from(kind),
        target_name: arg
            .target
            .npc
            .or(arg.target.loc)
            .or(arg.target.item)
            .map(Arc::from),
        tile,
        radius: arg.radius.max(1),
        settle_ms: arg.settle_ms,
    }))
}

struct UseOnPlan {
    item: Arc<str>,
    kind: Arc<str>,
    target_name: Option<Arc<str>>,
    tile: Option<WorldTile>,
    radius: i32,
    settle_ms: Option<u64>,
}
impl StepPlan for UseOnPlan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        Ok(Box::new(UseOnRun {
            item: Arc::clone(&self.item),
            kind: Arc::clone(&self.kind),
            target_name: self.target_name.clone(),
            tile: self.tile,
            radius: self.radius,
            deadline: self
                .settle_ms
                .map(|ms| cx.tick.cx.active_now() + Duration::from_millis(ms)),
            walk: None,
            emitted: false,
        }))
    }
}

struct UseOnRun {
    item: Arc<str>,
    kind: Arc<str>,
    target_name: Option<Arc<str>>,
    tile: Option<WorldTile>,
    radius: i32,
    deadline: Option<Duration>,
    walk: Option<ActionHandle<Walk>>,
    emitted: bool,
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
        if !self.emitted {
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
            let tile = self.tile.unwrap_or(WorldTile {
                x: 0,
                z: 0,
                level: 0,
            });
            cx.tick.cx.emit(InteractReq::UseOn {
                name: self.item.to_string(),
                kind: self.kind.to_string(),
                target_name: self.target_name.as_ref().map(|n| n.to_string()),
                x: tile.x,
                z: tile.z,
                level: tile.level,
                index: None,
                source_item_id: None,
                source_item_slot: None,
                target_item_id: None,
                target_item_slot: None,
            })?;
            self.emitted = true;
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
    }
}

#[derive(Deserialize)]
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
        }))
    }
}

struct AcquireRun {
    steps: Vec<CompiledAcquireStep>,
    current: Option<Box<dyn StepRun>>,
    index: usize,
}
impl StepRun for AcquireRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        loop {
            if self.current.is_none() {
                while self.index < self.steps.len() {
                    let skip = self.steps[self.index].skip_if.evaluate(&PredicateContext {
                        cx: &cx.tick.cx,
                        quests: cx.quests,
                        progress: cx.progress,
                        required_after: cx.required_after,
                        outcome: None,
                    });
                    if skip == Truth::True {
                        self.index += 1;
                        continue;
                    }
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
                    self.index += 1;
                }
            }
        }
    }
    fn cancel(&mut self, actions: &mut NativeActions) {
        if let Some(run) = &mut self.current {
            run.cancel(actions);
        }
    }
}

#[derive(Deserialize)]
struct WaitArgs {
    #[serde(default)]
    max_ticks: u64,
}

fn compile_wait(
    args: &serde_json::Value,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    let arg: WaitArgs = serde_json::from_value(args.clone()).unwrap_or(WaitArgs { max_ticks: 50 });
    Ok(Arc::new(WaitPlan {
        max_ticks: arg.max_ticks.max(1),
    }))
}

struct WaitPlan {
    max_ticks: u64,
}
impl StepPlan for WaitPlan {
    fn begin(&self, _cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        Ok(Box::new(WaitRun {
            remaining: self.max_ticks,
        }))
    }
}

struct WaitRun {
    remaining: u64,
}
impl StepRun for WaitRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        if self.remaining == 0 {
            return Poll::Ready(Err(ActionError::Failed(Arc::from("wait exhausted"))));
        }
        self.remaining -= 1;
        Poll::Ready(Ok(StepOutcome {
            progress: None,
            evidence: cx.tick.cx.evidence(),
            receipt: None,
        }))
    }
    fn cancel(&mut self, _actions: &mut NativeActions) {}
}
