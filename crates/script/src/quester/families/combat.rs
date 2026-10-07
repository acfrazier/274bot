//! Shared `script::combat` adapter for typed Quester Path combat steps.
use super::super::compile::{
    CompileContext, CompileError, FamilyReceipt, PredicateContext, PredicatePlan, StepContext,
    StepOutcome, StepPlan, StepRun, StepTraceEvent,
};
use super::super::path::PredicateDocument;
use super::dialogue::{Dialogue, DialogueArgs, DialogueOptions, DialogueTarget};
use super::reach::{self, Reach, ReachArgs, ReachKind};
use super::{compile_dialogue_options, DialogueOptionsDocument, LineRuleDocument};
use crate::combat::{
    AbortReason, ActorRef, Allowances, Combat, CombatEnd, CombatReport, CombatRequest,
    CombatTables, CompiledKit, Fallback, GuardOp, GuardRefusal, IntruderPolicy, MeleeMode, Pick,
    PrayerMode, RaisedPrayers, RangedMode, SpellRef, Style, Tactic, Target, WalkGuard,
};
use crate::dialogue_outcome::DialogueOutcome;
use crate::loadouts_store::WORN_SLOTS;
use crate::native::walk::Walk;
use crate::native::{
    ActionContext, ActionError, ActionHandle, NativeActions, NativeMachine, WalkAllow, WalkReceipt,
};
use crate::shim::InteractReq;
use api::gather_methods::SceneRegionInput;
use api::selected::Truth;
use serde::Deserialize;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

/// Blocked reason after a combat abort walk arrives with no live attacker.
pub const ABORTED_COMBAT_STEP_REASON: &str = "combat aborted; caller must handle the failure";
/// Blocked reason when a combat abort walk cannot be formed or does not arrive.
pub const ABORT_WALK_FAILED_REASON: &str = "combat abort walk did not reach a safe tile";
/// Blocked reason when the guarded abort hold observes disengagement.
pub const ABORT_HOLD_SAFE_REASON: &str =
    "combat abort guarded hold ended after three consecutive threat-free ticks";
/// Blocked reason when the guarded abort hold cannot admit its guard.
pub const ABORT_HOLD_UNAVAILABLE_REASON: &str =
    "combat abort safety hold could not start its food guard";
/// Blocked reason after the single supplies-exhausted escape walk arrives.
pub const ABORT_HOLD_ESCAPED_REASON: &str =
    "combat abort exhausted guard supplies; completed one escape walk and parked";

static ABORTED_COMBAT_STEP: std::sync::LazyLock<Arc<str>> =
    std::sync::LazyLock::new(|| Arc::from(ABORTED_COMBAT_STEP_REASON));
static ABORT_WALK_FAILED: std::sync::LazyLock<Arc<str>> =
    std::sync::LazyLock::new(|| Arc::from(ABORT_WALK_FAILED_REASON));
static COMBAT_FINISH_TIMEOUT: std::sync::LazyLock<Arc<str>> =
    std::sync::LazyLock::new(|| Arc::from("combat finish exceeded its tick budget"));
static COMBAT_FINISH_FAILED: std::sync::LazyLock<Arc<str>> =
    std::sync::LazyLock::new(|| Arc::from("combat finish dialogue failed"));
static COMBAT_FINISH_INTERRUPTED: std::sync::LazyLock<Arc<str>> =
    std::sync::LazyLock::new(|| Arc::from("combat interrupted the post-transform dialogue"));
static ABORT_HOLD_SAFE: std::sync::LazyLock<Arc<str>> =
    std::sync::LazyLock::new(|| Arc::from(ABORT_HOLD_SAFE_REASON));
static ABORT_HOLD_UNAVAILABLE: std::sync::LazyLock<Arc<str>> =
    std::sync::LazyLock::new(|| Arc::from(ABORT_HOLD_UNAVAILABLE_REASON));
static ABORT_HOLD_ESCAPED: std::sync::LazyLock<Arc<str>> =
    std::sync::LazyLock::new(|| Arc::from(ABORT_HOLD_ESCAPED_REASON));

fn tile_distance(a: api::WorldTile, b: api::WorldTile) -> i32 {
    if a.level != b.level {
        i32::MAX
    } else {
        (a.x - b.x).abs().max((a.z - b.z).abs())
    }
}

#[derive(Clone, Copy)]
struct AbortThreat {
    actor: ActorRef,
    npc_type: i32,
    tile: api::WorldTile,
    weight: i32,
}

fn has_live_attacker(
    snapshot: &api::snapshot::SnapshotView<'_>,
    tables: &CombatTables,
) -> Option<bool> {
    let local_index = snapshot.local_player()?.value.player.index;
    let observed_hitmarks = snapshot.hitmarks();
    let hitmarks = observed_hitmarks.as_ref().map(|row| &row.value);
    let npcs = snapshot.npcs();
    let players = snapshot.players();
    let npc_attacker = npcs.as_ref().map(|rows| {
        rows.value.iter().any(|npc| {
            crate::combat::threats::npc_local_attack_evidence(npc, local_index, hitmarks, tables)
                .is_live()
        })
    });
    let player_attacker = players.as_ref().map(|rows| {
        rows.value.iter().any(|player| {
            crate::combat::threats::actor_local_attack_evidence(
                &player.actor,
                local_index,
                hitmarks,
                tables,
            )
            .is_live()
        })
    });
    if npc_attacker == Some(true) || player_attacker == Some(true) {
        return Some(true);
    }
    (npcs.is_some() && players.is_some() && hitmarks.is_some()).then_some(false)
}

#[derive(Debug, Clone, Copy)]
pub struct CombatReceipt {
    pub report: CombatReport,
    pub target_gone_restarts: u8,
}

impl FamilyReceipt for CombatReceipt {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Proof that the combat target transformed into its post-combat NPC and its
/// continuation dialogue completed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishReceipt {
    pub npc_type: i32,
    pub npc_name: Arc<str>,
    pub npc_index: usize,
}

impl FamilyReceipt for FinishReceipt {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct CombatArgs {
    /// Target selector; exactly one supported target mode is required.
    target: TargetArgs,
    /// Combat action and style configuration.
    tactic: TacticArgs,
    /// Optional melee attack mode.
    #[serde(default)]
    melee_mode: Option<MeleeMode>,
    /// Optional named loadout for the encounter.
    #[serde(default)]
    loadout: Option<String>,
    /// Selected spell aliases in explicit manual order; null uses native selection.
    #[serde(default)]
    spells: Option<Vec<String>>,
    /// Permit native selection after the explicit manual order is exhausted.
    #[serde(default)]
    fallback_spells: bool,
    /// Optional sourced stand tile; `anchor` is accepted as an alias.
    #[serde(default, alias = "anchor")]
    stand: Option<super::AnchorArg>,
    /// Danger-zone permissions for this step's return and abort walks.
    #[serde(default)]
    cross: Vec<String>,
    /// Use protect to enable WalkGuard protection on this step's own walks.
    #[serde(default)]
    guard: Option<String>,
    /// Optional named or inline search area.
    #[serde(default)]
    area: Option<AreaArg>,
    /// Maximum distance in tiles before the target is considered lost.
    lost_radius: u8,
    /// Maximum game ticks to spend on a combat attempt.
    kill_budget_ticks: u16,
    /// Optional loot configs with per-kill inventory thresholds, not total requirements.
    #[serde(default)]
    loot: Vec<LootArg>,
    /// Optional dialogue with the exact NPC produced by the defeated target.
    #[serde(default)]
    finish: Option<FinishArgs>,
    /// Optional predicate that ends the combat loop when true.
    #[serde(default)]
    until: Option<PredicateDocument>,
    /// Optional success predicate that ends the combat loop when true.
    #[serde(default)]
    win: Option<PredicateDocument>,
}
/// One optional loot item, either a single-item or explicit per-kill threshold.
#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
enum LootArg {
    /// Optionally take a selected item when its inventory count is below one.
    Alias(String),
    /// Optionally take a selected item below its authored inventory threshold.
    Quantity(LootQuantityArg),
}

/// Optional per-kill looting threshold for one selected item.
#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct LootQuantityArg {
    /// Selected item config, resolved to an exact item identity.
    obj: String,
    /// Positive inventory threshold for optional looting; caller until/settle proves the total.
    qty: u32,
}

/// Dialogue which completes the post-combat transformation phase.
#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct FinishArgs {
    /// Selected config of the transformed NPC.
    npc: String,
    /// Ordered text fragments used by the shared dialogue answer selector.
    #[serde(default)]
    prefer: Vec<String>,
    /// Optional one-based dialogue option index.
    #[serde(default)]
    choose: Option<i32>,
    /// Current-page text rules checked before the general preferences.
    #[serde(default)]
    line_rules: Vec<LineRuleDocument>,
    /// Refuse missing or ambiguous answer text instead of choosing a fallback.
    #[serde(default)]
    strict: bool,
    /// Positive game-tick budget for the complete transformation and dialogue.
    max_ticks: u32,
}

#[derive(Clone)]
struct FinishConfig {
    npc_type: i32,
    npc_name: Arc<str>,
    original_npc_types: Arc<[i32]>,
    options: DialogueOptions,
    max_ticks: u32,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct TargetArgs {
    /// One or more NPC config names to target.
    #[serde(default)]
    npc: Option<NpcArg>,
    /// Select NPCs or players that are already attacking the player.
    #[serde(default)]
    attacker: Option<AttackerArgs>,
    /// Player targeting is represented in the wire format but is unsupported.
    #[serde(default)]
    player: Option<serde_json::Value>,
    /// Require the chosen NPC not to be targeting another player.
    #[serde(default)]
    not_targeting_others: Option<bool>,
    /// Policy used when choosing among matching targets.
    #[serde(default)]
    pick: Option<PickArg>,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
enum NpcArg {
    /// A single NPC config name.
    One(String),
    /// Several alternative NPC config names.
    Many(Vec<String>),
}

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct AttackerArgs {
    /// Include NPC attackers.
    npcs: bool,
    /// Include player attackers.
    players: bool,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
enum PickArg {
    /// Choose the closest matching target.
    Nearest,
    /// Choose a matching target at random.
    Random,
    /// Choose the matching target with the lowest health.
    LowestHealth,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct TacticArgs {
    /// Combat opening mode; only `open` is currently supported.
    kind: String,
    /// Attack style; `melee`, `ranged`, and `mage` are supported.
    style: String,
    #[serde(default)]
    ranged_style: RangedMode,
    /// Maximum distance at which to engage a target.
    engage_radius: u8,
    /// Whether to enable auto-retaliation; defaults to true.
    #[serde(default = "default_auto_retaliate")]
    auto_retaliate: bool,
    /// Optional fallback; only `abort` is currently supported.
    #[serde(default)]
    fallback: Option<String>,
}

fn default_auto_retaliate() -> bool {
    true
}

fn validate_style_options(style: Style, melee_mode: Option<MeleeMode>) -> Result<(), CompileError> {
    if style == Style::Ranged && melee_mode.is_some() {
        return Err(CompileError::code("invalid-args"));
    }
    Ok(())
}

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
enum AreaArg {
    /// Name of an area declared in the quest header.
    Named(String),
    /// Inline one or more sourced region boxes.
    Inline(InlineArea),
}

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct InlineArea {
    /// Optional single region box `[x1, z1, x2, z2, level]`.
    #[serde(default, rename = "box")]
    one: Option<[i32; 5]>,
    /// Optional list of region boxes `[x1, z1, x2, z2, level]`.
    #[serde(default)]
    boxes: Option<Vec<[i32; 5]>>,
    /// Source citation for these authored region boxes.
    source: String,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct CombatEndArgs {
    /// Combat outcome to match.
    end: CombatEndName,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
enum CombatEndName {
    Killed,
    TargetGone,
    NoTarget,
    Budget,
    Died,
    Aborted,
    AbortedUnattackable,
}

pub(super) fn compile(
    args: CombatArgs,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    Ok(Arc::new(compile_plan(args, cx)?))
}

fn compile_plan(args: CombatArgs, cx: &CompileContext<'_>) -> Result<CombatPlan, CompileError> {
    let protect = match args.guard.as_deref() {
        None | Some("") => false,
        Some("protect") => true,
        Some(_) => {
            return Err(
                CompileError::code("invalid-args").with_detail("combat: guard must be protect")
            );
        }
    };
    let style = match args.tactic.style.as_str() {
        "melee" => Style::Melee,
        "ranged" => Style::Ranged,
        "mage" => Style::Mage,
        _ => return Err(CompileError::code("unsupported-combat-style")),
    };
    validate_style_options(style, args.melee_mode)?;
    if style != Style::Mage
        && args
            .spells
            .as_ref()
            .is_some_and(|spells| !spells.is_empty())
    {
        return Err(CompileError::code("unsupported-combat-spells"));
    }
    let target = compile_target(args.target, cx)?;
    let finish = args
        .finish
        .map(|finish| compile_finish(finish, &target, cx))
        .transpose()?;
    if args.tactic.kind != "open" {
        return Err(CompileError::code("unsupported-combat-tactic"));
    }

    if args.tactic.engage_radius == 0 || args.lost_radius == 0 {
        return Err(CompileError::code("invalid-combat-radius"));
    }
    if args
        .tactic
        .fallback
        .as_deref()
        .is_some_and(|fallback| fallback != "abort")
    {
        return Err(CompileError::code("unsupported-combat-fallback"));
    }

    let stand = super::anchor_tile(args.stand.as_ref())?;
    let search_bounds = compile_search_bounds(args.area, cx)?;

    let kit = args
        .loadout
        .as_deref()
        .map(|name| compile_kit(name, cx))
        .transpose()?;
    let loot = compile_loot(&args.loot, cx)?;
    let until = args
        .until
        .as_ref()
        .map(|predicate| super::compile_predicate(predicate, cx))
        .transpose()?;
    let win = args
        .win
        .as_ref()
        .map(|predicate| super::compile_predicate(predicate, cx))
        .transpose()?;
    let tables = build_tables(cx)?;
    let spells = if style == Style::Mage {
        args.spells
            .map(|names| {
                if names.is_empty() || names.len() > u8::MAX as usize {
                    return Err(CompileError::code("invalid-combat-spells"));
                }
                names
                    .into_iter()
                    .map(|name| {
                        let index = crate::combat::style::magic::spell_index(&tables, &name)
                            .ok_or_else(|| CompileError::code("unresolved-combat-spell"))?;
                        Ok(SpellRef {
                            alias: Arc::from(
                                tables.selected().spells()[usize::from(index)]
                                    .source_row
                                    .as_str(),
                            ),
                        })
                    })
                    .collect::<Result<Arc<[SpellRef]>, CompileError>>()
            })
            .transpose()?
    } else {
        None
    };

    let request = CombatRequest {
        target,
        tactic: Tactic::Open,
        style,
        melee_mode: args.melee_mode,
        ranged_style: args.tactic.ranged_style,
        kit,
        spells,
        fallback_spells: args.fallback_spells,
        stand,
        search_bounds,
        engage_radius: args.tactic.engage_radius,
        lost_radius: args.lost_radius,
        budget_ticks: args.kill_budget_ticks,
        allow: Allowances::default(),
        fallback: Fallback::Abort,
        intruder: IntruderPolicy::default(),
        retaliate: args.tactic.auto_retaliate,
        prayer_mode: PrayerMode::Hold,
        ..CombatRequest::default()
    };

    Ok(CombatPlan {
        request: Arc::new(request),
        tables,
        cross: args.cross.into_iter().map(Arc::from).collect(),
        protect,
        until,
        win,
        loot: Arc::from(loot),
        finish,
    })
}

pub(super) fn compile_end_predicate(
    args: CombatEndArgs,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    Ok(Arc::new(CombatEndPredicate { expected: args.end }))
}

fn compile_target(args: TargetArgs, cx: &CompileContext<'_>) -> Result<Target, CompileError> {
    let target_count = [
        args.npc.is_some(),
        args.attacker.is_some(),
        args.player.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    if target_count != 1 {
        return Err(CompileError::code("invalid-combat-target"));
    }
    if args.player.is_some() {
        return Err(CompileError::code("unsupported-combat-player-target"));
    }
    if let Some(npc) = args.npc {
        let not_targeting_others = args
            .not_targeting_others
            .ok_or_else(|| CompileError::code("invalid-combat-target"))?;
        let pick = match args
            .pick
            .ok_or_else(|| CompileError::code("invalid-combat-target"))?
        {
            PickArg::Nearest => Pick::Nearest,
            PickArg::Random => Pick::Random,
            PickArg::LowestHealth => Pick::LowestHealth,
        };
        let names = match npc {
            NpcArg::One(name) => vec![name],
            NpcArg::Many(names) => names,
        };
        if names.is_empty() {
            return Err(CompileError::code("invalid-combat-target"));
        }
        let mut types = Vec::with_capacity(names.len());
        for name in names {
            let id = super::resolve_npc(cx, &name)?;
            if types.contains(&id) {
                return Err(CompileError::code("duplicate-combat-target"));
            }
            types.push(id);
        }
        return Ok(Target::Npc {
            types: Arc::from(types),
            pick,
            not_targeting_others,
        });
    }
    if args.pick.is_some() || args.not_targeting_others.is_some() {
        return Err(CompileError::code("invalid-combat-target"));
    }
    let Some(attacker) = args.attacker else {
        return Err(CompileError::code("invalid-combat-target"));
    };
    if !attacker.npcs && !attacker.players {
        return Err(CompileError::code("invalid-combat-target"));
    }
    Ok(Target::Attacker {
        npcs: attacker.npcs,
        players: attacker.players,
    })
}

fn compile_finish(
    args: FinishArgs,
    target: &Target,
    cx: &CompileContext<'_>,
) -> Result<FinishConfig, CompileError> {
    if args.max_ticks == 0 {
        return Err(CompileError::code("invalid-combat-finish-budget"));
    }
    let Target::Npc { types, .. } = target else {
        return Err(CompileError::code("invalid-combat-finish-target"));
    };
    let npc_type = super::resolve_npc(cx, &args.npc)?;
    if types.contains(&npc_type) {
        return Err(CompileError::code("invalid-combat-finish-target"));
    }
    let npc_name = cx
        .selected
        .npc_by_config(&args.npc)
        .and_then(|row| row.display.as_deref())
        .ok_or_else(|| CompileError::code("unresolved-npc"))?;
    let options = compile_dialogue_options(DialogueOptionsDocument {
        prefer: args.prefer,
        choose: args.choose,
        line_rules: args.line_rules,
        strict: args.strict,
        gap_ticks: None,
    })?;
    Ok(FinishConfig {
        npc_type,
        npc_name: Arc::from(npc_name),
        original_npc_types: Arc::clone(types),
        options,
        max_ticks: args.max_ticks,
    })
}
fn compile_search_bounds(
    area: Option<AreaArg>,
    cx: &CompileContext<'_>,
) -> Result<Option<Arc<[SceneRegionInput]>>, CompileError> {
    let Some(area) = area else {
        return Ok(None);
    };
    let boxes = match area {
        AreaArg::Named(name) => cx
            .areas
            .get(&name)
            .cloned()
            .ok_or_else(|| CompileError::code("unknown-area"))?,
        AreaArg::Inline(area) => {
            if area.source.trim().is_empty() {
                return Err(CompileError::code("tile-source-required"));
            }
            let mut boxes = area.boxes.unwrap_or_default();
            if let Some(one) = area.one {
                boxes.push(one);
            }
            boxes
        }
    };
    if boxes.is_empty() {
        return Err(CompileError::code("invalid-combat-area"));
    }
    let mut bounds = Vec::with_capacity(boxes.len());
    for [x1, z1, x2, z2, level] in boxes {
        if !(0..=16383).contains(&x1)
            || !(0..=16383).contains(&z1)
            || !(0..=16383).contains(&x2)
            || !(0..=16383).contains(&z2)
            || !(0..=3).contains(&level)
        {
            return Err(CompileError::code("invalid-combat-area"));
        }
        bounds.push(SceneRegionInput {
            min_x: x1.min(x2),
            min_z: z1.min(z2),
            max_x: x1.max(x2),
            max_z: z1.max(z2),
            level,
        });
    }
    Ok(Some(Arc::from(bounds)))
}

fn compile_kit(name: &str, cx: &CompileContext<'_>) -> Result<Arc<CompiledKit>, CompileError> {
    if name.trim().is_empty() {
        return Err(CompileError::code("unknown-loadout"));
    }
    let qualified = if name.contains('/') {
        name.to_owned()
    } else {
        format!("{}/{}", cx.path.0, name)
    };
    let row = cx
        .loadouts
        .resolve(&qualified)
        .ok_or_else(|| CompileError::code("unknown-loadout"))?
        .row();
    if !row.unassigned.is_empty() {
        return Err(CompileError::code("unsupported-combat-loadout-items"));
    }
    let mut worn = Vec::with_capacity(row.worn.len());
    for (slot, item) in &row.worn {
        let slot = WORN_SLOTS
            .iter()
            .position(|known| known.eq_ignore_ascii_case(slot))
            .and_then(|slot| u8::try_from(slot).ok())
            .ok_or_else(|| CompileError::code("invalid-loadout-slot"))?;
        let id = resolve_loadout_item(cx, item)?.0;
        worn.push((slot, id));
    }
    let mut carry = Vec::with_capacity(row.carry.len());
    for entry in &row.carry {
        if entry.qty == 0 {
            return Err(CompileError::code("invalid-combat-loadout-quantity"));
        }
        carry.push((resolve_loadout_item(cx, &entry.item)?.0, entry.qty));
    }
    Ok(Arc::new(CompiledKit {
        worn: Arc::from(worn),
        carry: Arc::from(carry),
    }))
}

fn compile_loot(loot: &[LootArg], cx: &CompileContext<'_>) -> Result<Vec<LootItem>, CompileError> {
    let mut rows = Vec::with_capacity(loot.len());
    for entry in loot {
        let (name, qty) = match entry {
            LootArg::Alias(name) => (name.as_str(), 1),
            LootArg::Quantity(row) => (row.obj.as_str(), row.qty),
        };
        if qty == 0 {
            return Err(CompileError::code("invalid-combat-loot-quantity"));
        }
        let (id, display) = resolve_loadout_item(cx, name)?;
        if rows.iter().any(|row: &LootItem| row.id == id) {
            return Err(CompileError::code("duplicate-combat-loot-item"));
        }
        rows.push(LootItem {
            id,
            name: display,
            qty,
        });
    }
    Ok(rows)
}

fn resolve_loadout_item(
    cx: &CompileContext<'_>,
    name: &str,
) -> Result<(i32, Arc<str>), CompileError> {
    let item = cx
        .selected
        .resolve_item_name(name)
        .ok_or_else(|| CompileError::code("unresolved-loadout-item"))?;
    Ok((item.id, Arc::from(item.name.as_deref().unwrap_or(name))))
}

fn build_tables(cx: &CompileContext<'_>) -> Result<Arc<CombatTables>, CompileError> {
    let pin = cx
        .selected
        .selected_pin()
        .map_err(|_| CompileError::code("missing-pin"))?;
    let selected = api::game_data::for_revision(pin.revision)
        .map_err(|_| CompileError::code("combat-tables-unavailable"))?;
    let selected_pin = selected
        .selected_pin()
        .map_err(|_| CompileError::code("missing-pin"))?;
    if selected_pin.as_ref() != pin.as_ref() {
        return Err(CompileError::code("combat-pin-mismatch"));
    }
    CombatTables::build(selected).map_err(|_| CompileError::code("combat-tables-unavailable"))
}

#[derive(Clone)]
struct LootItem {
    id: i32,
    name: Arc<str>,
    qty: u32,
}

struct CombatPlan {
    request: Arc<CombatRequest>,
    tables: Arc<CombatTables>,
    cross: Arc<[Arc<str>]>,
    protect: bool,
    until: Option<Arc<dyn PredicatePlan>>,
    win: Option<Arc<dyn PredicatePlan>>,
    loot: Arc<[LootItem]>,
    finish: Option<FinishConfig>,
}

impl StepPlan for CombatPlan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        let mut run = CombatRun {
            request: Arc::clone(&self.request),
            tables: Arc::clone(&self.tables),
            cross: Arc::clone(&self.cross),
            protect: self.protect,
            until: self.until.as_ref().map(Arc::clone),
            win: self.win.as_ref().map(Arc::clone),
            loot: Arc::clone(&self.loot),
            finish: self.finish.clone(),
            action: None,
            phase: Phase::Combat,
            loot_index: 0,
            last_report: None,
            last_outcome: None,
            target_gone_restarts: 0,
            raised_prayers: RaisedPrayers::empty(),
            walk_outcome_seq_at_begin: cx.tick.cx.observed_walk_outcome_seq,
            finish_target_index: None,
            finish_ticks_elapsed: 0,
            finish_last_tick: None,
            trace_event: None,
        };
        run.begin_combat(cx)?;
        Ok(Box::new(run))
    }

    fn settle_timeout(&self) -> Duration {
        Duration::from_secs(8)
    }
}

enum Phase {
    Combat,
    ReturningToStand,
    WalkingOutAfterAbort,
    HoldingAfterAbort,
    EscapingAfterAbortHold,
    Loot,
    FinishWait,
    FinishDialogue,
}

#[allow(
    clippy::large_enum_variant,
    reason = "Keep native machines inline instead of allocating on every combat/loot transition."
)]
enum Action {
    Combat(ActionHandle<Combat>),
    Walk(ActionHandle<Walk>),
    AbortHold(ActionHandle<AbortHold>),
    Loot(ActionHandle<Reach>),
    Dialogue(ActionHandle<Dialogue>),
}

enum ActionPoll {
    Pending,
    Failed(ActionError),
    Combat(CombatReport),
    Walk(WalkReceipt),
    AbortHold(AbortHoldOutcome),
    Loot(bool),
    Dialogue(DialogueOutcome),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AbortHoldOutcome {
    ThreatFree,
    SuppliesExhausted,
}

struct AbortHoldArgs {
    tables: Arc<CombatTables>,
    allow_prayer: bool,
}

struct AbortHold {
    tables: Arc<CombatTables>,
    guard: Option<WalkGuard>,
    protect_unavailable: bool,
    last_threat_tick: Option<u64>,
    threat_free_ticks: u8,
    terminal_outcome: Option<AbortHoldOutcome>,
    owned_prayers: RaisedPrayers,
    cleanup: [i32; 2],
    cleanup_len: u8,
    cleanup_index: u8,
    cleanup_pending: Option<(u64, u64)>,
    cleanup_retries: u8,
}

impl NativeMachine for AbortHold {
    type Args = AbortHoldArgs;
    type Output = AbortHoldOutcome;

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let snapshot = cx.snapshot();
        let here = snapshot
            .here()
            .map(|row| row.value)
            .unwrap_or(api::WorldTile {
                x: 0,
                z: 0,
                level: 0,
            });
        let mut request = reach::walk_request(here, 0, None, cx.evidence());
        request.protect = args.allow_prayer;
        request.allow = WalkAllow {
            prayer: args.allow_prayer,
            food: true,
            escape: true,
        };
        let (guard, protect_unavailable) = if args.allow_prayer {
            match WalkGuard::begin_with(&request, &snapshot, Arc::clone(&args.tables)) {
                Ok(guard) => (guard, false),
                Err(
                    GuardRefusal::PrayerTooLow
                    | GuardRefusal::PrayerDisallowed
                    | GuardRefusal::Snapshot,
                ) => (
                    WalkGuard::begin_food_only_with(&request, &snapshot, Arc::clone(&args.tables))
                        .map_err(|_| ActionError::Blocked(Arc::clone(&ABORT_HOLD_UNAVAILABLE)))?,
                    true,
                ),
                Err(_) => {
                    return Err(ActionError::Blocked(Arc::clone(&ABORT_HOLD_UNAVAILABLE)));
                }
            }
        } else {
            (
                WalkGuard::begin_food_only_with(&request, &snapshot, Arc::clone(&args.tables))
                    .map_err(|_| ActionError::Blocked(Arc::clone(&ABORT_HOLD_UNAVAILABLE)))?,
                true,
            )
        };
        Ok(Self {
            tables: args.tables,
            guard: Some(guard),
            protect_unavailable,
            last_threat_tick: None,
            threat_free_ticks: 0,
            terminal_outcome: None,
            owned_prayers: RaisedPrayers::empty(),
            cleanup: [0; 2],
            cleanup_len: 0,
            cleanup_index: 0,
            cleanup_pending: None,
            cleanup_retries: 0,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        if self.terminal_outcome.is_some() {
            return self.poll_cleanup(cx);
        }

        let snapshot = cx.snapshot();
        let live_attacker = has_live_attacker(&snapshot, &self.tables);
        self.observe_threat_free(cx.evidence().tick, live_attacker);
        if let Some(guard) = self.guard.as_mut() {
            if let Some(op) = guard.tick(&snapshot) {
                if matches!(&op, GuardOp::Unprotectable { .. }) {
                    self.protect_unavailable = true;
                }
                if let Some(request) = guard_interaction(&op) {
                    match cx.emit(request) {
                        Ok(_) => guard.admitted(&op, &snapshot),
                        Err(
                            ActionError::BudgetExhausted | ActionError::Busy | ActionError::Held,
                        ) => {
                            return Poll::Pending;
                        }
                        Err(error) => return Poll::Ready(Err(error)),
                    }
                }
            }
        }

        if self.threat_free_ticks >= 3 {
            self.begin_end(AbortHoldOutcome::ThreatFree);
            return Poll::Pending;
        }
        // The single exhausted escape runs from a live attacker. Without one
        // the threat-free horizon above ends the hold instead.
        if live_attacker == Some(true)
            && self.protect_unavailable
            && self
                .guard
                .as_ref()
                .and_then(|guard| guard.has_food(&snapshot))
                == Some(false)
        {
            self.begin_end(AbortHoldOutcome::SuppliesExhausted);
            return Poll::Pending;
        }
        Poll::Pending
    }

    fn cancel(&mut self) {
        if let Some(guard) = self.guard.take() {
            self.owned_prayers.merge(guard.prayer_cleanup());
        }
    }

    fn prayer_cleanup(&self) -> RaisedPrayers {
        let mut owned = self.owned_prayers;
        if let Some(guard) = self.guard.as_ref() {
            owned.merge(guard.prayer_cleanup());
        }
        owned
    }
}

impl AbortHold {
    fn observe_threat_free(&mut self, tick: u64, live_attacker: Option<bool>) {
        if self.last_threat_tick == Some(tick) {
            if live_attacker != Some(false) {
                self.threat_free_ticks = 0;
            }
            return;
        }
        let consecutive = self
            .last_threat_tick
            .is_some_and(|last| last.checked_add(1) == Some(tick));
        self.last_threat_tick = Some(tick);
        self.threat_free_ticks = if live_attacker == Some(false) {
            if consecutive {
                self.threat_free_ticks.saturating_add(1)
            } else {
                1
            }
        } else {
            0
        };
    }

    fn begin_end(&mut self, outcome: AbortHoldOutcome) {
        self.terminal_outcome = Some(outcome);
        let Some(guard) = self.guard.take() else {
            return;
        };
        self.owned_prayers.merge(guard.prayer_cleanup());
        let (op, fallback) = guard.end();
        if let GuardOp::IfButton { component } = op {
            self.push_cleanup(component);
        }
        if let Some(component) = fallback {
            self.push_cleanup(component);
        }
    }

    fn push_cleanup(&mut self, component: i32) {
        if component > 0 && usize::from(self.cleanup_len) < self.cleanup.len() {
            self.cleanup[usize::from(self.cleanup_len)] = component;
            self.cleanup_len += 1;
        }
    }

    fn poll_cleanup(
        &mut self,
        cx: &mut ActionContext<'_>,
    ) -> Poll<Result<AbortHoldOutcome, ActionError>> {
        if let Some((request_id, sent_tick)) = self.cleanup_pending {
            if let Some(receipt) = cx.interaction_receipt(request_id) {
                self.cleanup_pending = None;
                if receipt.accepted || self.cleanup_retries >= 1 {
                    self.cleanup_index += 1;
                    self.cleanup_retries = 0;
                } else {
                    self.cleanup_retries += 1;
                }
            } else if cx.evidence().tick.saturating_sub(sent_tick) >= 2 {
                self.cleanup_pending = None;
                if self.cleanup_retries >= 1 {
                    self.cleanup_index += 1;
                    self.cleanup_retries = 0;
                } else {
                    self.cleanup_retries += 1;
                }
            } else {
                return Poll::Pending;
            }
        }

        if self.cleanup_index < self.cleanup_len {
            let component = self.cleanup[usize::from(self.cleanup_index)];
            match cx.emit(InteractReq::IfButton {
                component_id: component,
            }) {
                Ok(request_id) => {
                    self.cleanup_pending = Some((request_id, cx.evidence().tick));
                    return Poll::Pending;
                }
                Err(ActionError::BudgetExhausted | ActionError::Busy | ActionError::Held) => {
                    return Poll::Pending;
                }
                Err(error) => return Poll::Ready(Err(error)),
            }
        }

        self.terminal_outcome.map_or_else(
            || {
                Poll::Ready(Err(ActionError::Blocked(Arc::clone(
                    &ABORT_HOLD_UNAVAILABLE,
                ))))
            },
            |outcome| Poll::Ready(Ok(outcome)),
        )
    }
}

fn guard_interaction(op: &GuardOp) -> Option<InteractReq> {
    match op {
        GuardOp::IfButton { component } => Some(InteractReq::IfButton {
            component_id: *component,
        }),
        GuardOp::Drink { name } => Some(InteractReq::Held {
            name: name.to_string(),
            action: "Drink".to_owned(),
            slot: None,
            target_item_id: None,
        }),
        GuardOp::Eat { name } => Some(InteractReq::Held {
            name: name.to_string(),
            action: "Eat".to_owned(),
            slot: None,
            target_item_id: None,
        }),
        GuardOp::Locked { .. } | GuardOp::Unprotectable { .. } => None,
    }
}

enum LootStart {
    Waiting,
    Started,
    Complete,
}

struct CombatRun {
    request: Arc<CombatRequest>,
    tables: Arc<CombatTables>,
    cross: Arc<[Arc<str>]>,
    protect: bool,
    until: Option<Arc<dyn PredicatePlan>>,
    win: Option<Arc<dyn PredicatePlan>>,
    loot: Arc<[LootItem]>,
    action: Option<Action>,
    phase: Phase,
    loot_index: usize,
    last_report: Option<CombatReport>,
    last_outcome: Option<StepOutcome>,
    target_gone_restarts: u8,
    raised_prayers: RaisedPrayers,
    walk_outcome_seq_at_begin: u64,
    finish: Option<FinishConfig>,
    finish_target_index: Option<usize>,
    finish_ticks_elapsed: u64,
    finish_last_tick: Option<u64>,
    trace_event: Option<StepTraceEvent>,
}

/// Whether walk outcome `seq` was cancelled by user input. The cancel reason
/// lives in the isolate's observed scene, which exists only with `load`.
#[cfg(feature = "load")]
fn user_walk_cancelled(seq: u64) -> bool {
    crate::observed::with(|scene| {
        let latest = scene.latest();
        latest.walk_outcome_seq() == Some(seq)
            && latest.walk_outcome_cancel_reason()
                == Some(crate::isolate_fb::WalkCancelReason::UserInput)
    })
}

#[cfg(not(feature = "load"))]
fn user_walk_cancelled(_seq: u64) -> bool {
    false
}

impl CombatRun {
    fn capture_prayer_cleanup(&mut self) {
        match self.action.as_ref() {
            Some(Action::Combat(handle)) => {
                self.raised_prayers = handle.prayer_cleanup();
            }
            Some(Action::AbortHold(handle)) => {
                self.raised_prayers.merge(handle.machine_prayer_cleanup());
            }
            _ => {}
        }
    }

    /// Pin only the current local combat target, then follow that NPC slot
    /// through its transform. An unrelated NPC with the finish type cannot
    /// start the dialogue.
    fn observe_finish_transform(&mut self, cx: &StepContext<'_, '_>) -> bool {
        let Some(finish) = self.finish.as_ref() else {
            return false;
        };
        let snapshot = cx.tick.cx.snapshot();
        let Some(npcs) = snapshot.npcs() else {
            return false;
        };
        let Some(combat) = snapshot.in_combat() else {
            return false;
        };
        if let Some(target) = combat.value.target {
            if target.kind != api::snapshot::ActorKind::Npc {
                self.finish_target_index = None;
                return false;
            }
            let original = npcs
                .value
                .iter()
                .find(|npc| npc.index == target.index)
                .and_then(|npc| npc.r#type)
                .is_some_and(|npc_type| {
                    finish
                        .original_npc_types
                        .iter()
                        .any(|original| usize::try_from(*original).ok() == Some(npc_type))
                });
            if combat.value.in_combat && original {
                self.finish_target_index = Some(target.index);
                return false;
            }
            if self.finish_target_index != Some(target.index) {
                self.finish_target_index = None;
                return false;
            }
        }
        let Some(index) = self.finish_target_index else {
            return false;
        };
        let finish_type = usize::try_from(finish.npc_type).ok();
        npcs.value
            .iter()
            .any(|npc| npc.index == index && npc.r#type == finish_type)
    }

    fn prayer_cleanup(&self) -> RaisedPrayers {
        match self.action.as_ref() {
            Some(Action::Combat(handle)) => handle.prayer_cleanup(),
            Some(Action::AbortHold(handle)) => {
                let mut owned = self.raised_prayers;
                owned.merge(handle.machine_prayer_cleanup());
                owned
            }
            _ => self.raised_prayers,
        }
    }

    fn user_interrupted_since_begin(&self, cx: &StepContext<'_, '_>) -> bool {
        let current_seq = cx.tick.cx.observed_walk_outcome_seq;
        if current_seq == self.walk_outcome_seq_at_begin {
            return false;
        }
        user_walk_cancelled(current_seq)
    }

    fn begin_combat(&mut self, cx: &mut StepContext<'_, '_>) -> Result<(), ActionError> {
        self.finish_target_index = None;
        self.finish_ticks_elapsed = 0;
        self.finish_last_tick = None;
        let handle = cx.tick.actions.begin::<Combat>(
            (Arc::clone(&self.request), Arc::clone(&self.tables)),
            &mut cx.tick.cx,
        )?;
        self.phase = Phase::Combat;
        self.action = Some(Action::Combat(handle));
        Ok(())
    }
    fn begin_return_to_stand(
        &mut self,
        stand: api::WorldTile,
        cx: &mut StepContext<'_, '_>,
    ) -> Result<(), ActionError> {
        let mut request = reach::walk_request(stand, 1, None, cx.required_after);
        request.cross = self.cross.iter().cloned().collect();
        request.protect = self.protect;
        let handle = cx.tick.actions.begin::<Walk>(request, &mut cx.tick.cx)?;
        self.phase = Phase::ReturningToStand;
        self.action = Some(Action::Walk(handle));
        Ok(())
    }

    fn abort_reference(
        &self,
        report: CombatReport,
        snapshot: &api::snapshot::SnapshotView<'_>,
    ) -> Option<AbortThreat> {
        let engaged = report.engaged.and_then(|engaged| match engaged.kind {
            api::snapshot::ActorKind::Npc => {
                let index = usize::from(engaged.index);
                let npc_type = report.engaged_npc_type;
                let kind = usize::try_from(npc_type).ok()?;
                snapshot
                    .npcs()?
                    .value
                    .iter()
                    .find(|npc| npc.index == index && npc.r#type == Some(kind))
                    .map(|npc| AbortThreat {
                        actor: engaged,
                        npc_type,
                        tile: npc.network,
                        weight: 0,
                    })
            }
            api::snapshot::ActorKind::Player => snapshot
                .players()?
                .value
                .iter()
                .find(|player| player.index == usize::from(engaged.index))
                .map(|player| AbortThreat {
                    actor: engaged,
                    npc_type: -1,
                    tile: player.network,
                    weight: 0,
                }),
        });
        // An engaged actor that is no longer observed cannot steer the
        // retreat; run from whichever live attacker remains instead.
        engaged.or_else(|| self.heaviest_live_attacker(snapshot))
    }

    fn heaviest_live_attacker(
        &self,
        snapshot: &api::snapshot::SnapshotView<'_>,
    ) -> Option<AbortThreat> {
        let observed_hitmarks = snapshot.hitmarks();
        let hitmarks = observed_hitmarks.as_ref().map(|row| &row.value);
        let local_index = snapshot.local_player()?.value.player.index;
        let mut heaviest: Option<AbortThreat> = None;
        if let Some(npcs) = snapshot.npcs() {
            for npc in npcs.value.iter() {
                if !crate::combat::threats::npc_local_attack_evidence(
                    npc,
                    local_index,
                    hitmarks,
                    &self.tables,
                )
                .is_live()
                {
                    continue;
                }
                let Some(npc_type) = npc.r#type.and_then(|kind| i32::try_from(kind).ok()) else {
                    continue;
                };
                let Ok(index) = u16::try_from(npc.index) else {
                    continue;
                };
                let weight = self
                    .tables
                    .selected()
                    .npc_name(npc_type)
                    .and_then(crate::combat::facts::npc_max_hit)
                    .map_or(0, |max_hit| i32::from(max_hit) * 10);
                let candidate = AbortThreat {
                    actor: ActorRef {
                        kind: api::snapshot::ActorKind::Npc,
                        index,
                    },
                    npc_type,
                    tile: npc.network,
                    weight,
                };
                if heaviest.is_none_or(|current| weight > current.weight) {
                    heaviest = Some(candidate);
                }
            }
        }
        if let Some(players) = snapshot.players() {
            for player in players.value.iter() {
                if !crate::combat::threats::actor_local_attack_evidence(
                    &player.actor,
                    local_index,
                    hitmarks,
                    &self.tables,
                )
                .is_live()
                {
                    continue;
                }
                let Ok(index) = u16::try_from(player.index) else {
                    continue;
                };
                let candidate = AbortThreat {
                    actor: ActorRef {
                        kind: api::snapshot::ActorKind::Player,
                        index,
                    },
                    npc_type: -1,
                    tile: player.network,
                    weight: player.combat_level.max(0),
                };
                if heaviest.is_none_or(|current| candidate.weight > current.weight) {
                    heaviest = Some(candidate);
                }
            }
        }
        heaviest
    }

    fn abort_walk_destination(
        &self,
        report: CombatReport,
        cx: &StepContext<'_, '_>,
    ) -> Option<api::WorldTile> {
        let snapshot = cx.tick.cx.snapshot();
        snapshot.here()?;
        // Retreat geometry compares packet-time network tiles. Rendered poses
        // interpolate and can point the retreat toward the attacker.
        let here = snapshot.local_player()?.value.player.network;
        let Some(reference) = self.abort_reference(report, &snapshot) else {
            return self.request.stand;
        };
        let target = reference.tile;
        // The engine drops a melee NPC's target only once the player is more
        // than `maxrange + 1` from the NPC's spawn, which the client never
        // sees, and the NPC itself may be up to `maxrange` from that spawn.
        // Measured from the NPC's current tile, the chase envelope is
        // therefore up to `2 * maxrange + attackrange` (Npc.ts:652-667).
        let chase_range = if reference.actor.kind == api::snapshot::ActorKind::Npc {
            self.tables
                .selected()
                .npc_name(reference.npc_type)
                .map(|npc| {
                    npc.maxrange
                        .saturating_mul(2)
                        .saturating_add(npc.attackrange)
                        .max(0)
                })
                .unwrap_or(10)
        } else {
            0
        };
        if let Some(stand) = self.request.stand {
            if tile_distance(stand, target) > chase_range.saturating_add(1) {
                return Some(stand);
            }
        }

        let retreat_distance = chase_range.saturating_add(3).max(2);
        let dx = (here.x - target.x).signum();
        let dz = (here.z - target.z).signum();
        [(dx, dz), (dx, 0), (0, dz), (1, 0), (-1, 0), (0, 1), (0, -1)]
            .into_iter()
            .find_map(|(x, z)| {
                if x == 0 && z == 0 {
                    return None;
                }
                let destination = api::WorldTile {
                    x: here.x.saturating_add(x.saturating_mul(retreat_distance)),
                    z: here.z.saturating_add(z.saturating_mul(retreat_distance)),
                    level: here.level,
                };
                (0..=16_383)
                    .contains(&destination.x)
                    .then_some(destination)
                    .filter(|_| (0..=16_383).contains(&destination.z))
            })
    }
    fn abort_protection_allowed(&self, snapshot: &api::snapshot::SnapshotView<'_>) -> bool {
        self.protect
            && snapshot.stats().is_some_and(|stats| {
                stats
                    .value
                    .iter()
                    .find(|stat| stat.index == 5)
                    .is_some_and(|prayer| {
                        prayer.base >= crate::combat::guard::MIN_PROTECT_PRAYER_LEVEL
                    })
            })
    }

    fn begin_abort_walk(
        &mut self,
        report: CombatReport,
        cx: &mut StepContext<'_, '_>,
    ) -> Result<(), ActionError> {
        let destination = self
            .abort_walk_destination(report, cx)
            .ok_or_else(|| ActionError::Blocked(Arc::clone(&ABORT_WALK_FAILED)))?;
        let mut request = reach::walk_request(destination, 1, None, cx.required_after);
        request.cross = self.cross.iter().cloned().collect();
        let snapshot = cx.tick.cx.snapshot();
        let allow_prayer = self.abort_protection_allowed(&snapshot);
        request.protect = allow_prayer;
        request.food_guard = !allow_prayer;
        request.allow = WalkAllow {
            prayer: allow_prayer,
            food: true,
            escape: true,
        };
        let handle = cx.tick.actions.begin::<Walk>(request, &mut cx.tick.cx)?;
        self.phase = Phase::WalkingOutAfterAbort;
        self.action = Some(Action::Walk(handle));
        Ok(())
    }

    fn begin_abort_hold(
        &mut self,
        cx: &mut StepContext<'_, '_>,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        let handle = match cx.tick.actions.begin::<AbortHold>(
            AbortHoldArgs {
                tables: Arc::clone(&self.tables),
                allow_prayer: self.abort_protection_allowed(&cx.tick.cx.snapshot()),
            },
            &mut cx.tick.cx,
        ) {
            Ok(handle) => handle,
            Err(
                error @ (ActionError::UserInput
                | ActionError::Cancelled
                | ActionError::NeedsEvidence(_)),
            ) => {
                return Poll::Ready(Err(error));
            }
            Err(_) => return self.begin_abort_escape_walk(cx),
        };
        self.phase = Phase::HoldingAfterAbort;
        self.action = Some(Action::AbortHold(handle));
        Poll::Pending
    }

    fn begin_abort_escape_walk(
        &mut self,
        cx: &mut StepContext<'_, '_>,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        let Some(report) = self.last_report else {
            return Poll::Ready(Err(ActionError::Blocked(Arc::clone(&ABORT_WALK_FAILED))));
        };
        let Some(destination) = self.abort_walk_destination(report, cx) else {
            return Poll::Ready(Err(ActionError::Blocked(Arc::clone(&ABORT_WALK_FAILED))));
        };
        let mut request = reach::walk_request(destination, 1, None, cx.required_after);
        request.cross = self.cross.iter().cloned().collect();
        request.protect = false;
        request.allow = WalkAllow {
            prayer: false,
            food: false,
            escape: true,
        };
        let handle = match cx.tick.actions.begin::<Walk>(request, &mut cx.tick.cx) {
            Ok(handle) => handle,
            Err(error) => return Poll::Ready(Err(error)),
        };
        self.phase = Phase::EscapingAfterAbortHold;
        self.action = Some(Action::Walk(handle));
        Poll::Pending
    }

    fn begin_abort_safety_walk(
        &mut self,
        report: CombatReport,
        cx: &mut StepContext<'_, '_>,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        match self.begin_abort_walk(report, cx) {
            Ok(()) => Poll::Pending,
            Err(error @ (ActionError::UserInput | ActionError::Cancelled)) => {
                Poll::Ready(Err(error))
            }
            Err(error @ ActionError::NeedsEvidence(_))
                if has_live_attacker(&cx.tick.cx.snapshot(), &self.tables) != Some(true) =>
            {
                Poll::Ready(Err(error))
            }
            Err(_) if has_live_attacker(&cx.tick.cx.snapshot(), &self.tables) == Some(true) => {
                self.begin_abort_hold(cx)
            }
            Err(_) => Poll::Ready(Err(ActionError::Blocked(Arc::clone(&ABORT_WALK_FAILED)))),
        }
    }

    fn on_return_walk(
        &mut self,
        receipt: WalkReceipt,
        cx: &mut StepContext<'_, '_>,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        self.action = None;
        receipt.into_arrival()?;
        self.rebegin_combat(cx, true)
    }

    fn on_abort_walk(
        &mut self,
        receipt: WalkReceipt,
        cx: &mut StepContext<'_, '_>,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        self.action = None;
        let has_attacker = {
            let snapshot = cx.tick.cx.snapshot();
            has_live_attacker(&snapshot, &self.tables) == Some(true)
        };
        let error = match receipt.into_arrival() {
            Ok(_) => ActionError::Blocked(Arc::clone(&ABORTED_COMBAT_STEP)),
            Err(error @ (ActionError::UserInput | ActionError::Cancelled)) => {
                return Poll::Ready(Err(error));
            }
            Err(error @ ActionError::NeedsEvidence(_)) if !has_attacker => {
                return Poll::Ready(Err(error));
            }
            Err(error) => error,
        };
        if has_attacker {
            self.begin_abort_hold(cx)
        } else {
            Poll::Ready(Err(error))
        }
    }

    fn on_abort_escape_walk(
        &mut self,
        receipt: WalkReceipt,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        self.action = None;
        match receipt.into_arrival() {
            Ok(_) => Poll::Ready(Err(ActionError::Blocked(Arc::clone(&ABORT_HOLD_ESCAPED)))),
            Err(error) => Poll::Ready(Err(error)),
        }
    }

    fn on_combat_report(
        &mut self,
        report: CombatReport,
        cx: &mut StepContext<'_, '_>,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        self.action = None;
        self.raised_prayers = RaisedPrayers::empty();
        self.last_report = Some(report);
        self.refresh_outcome();
        self.trace_event = Some(StepTraceEvent::CombatSubOperationEnd {
            target: self.request.target.clone(),
            end: report.end,
        });
        match report.end {
            // Loot begins only after the combat machine reports `Killed`
            // (design-combat.md §2.3, §5; combat-s3a-ReviewCombatFable.md F5).
            CombatEnd::Killed => {
                self.phase = Phase::Loot;
                self.loot_index = 0;
                Poll::Pending
            }
            CombatEnd::TargetGone => {
                // Imp's next attempt is anchored at stand, so wait for its
                // observed arrival before re-beginning (design-combat.md §1.1,
                // §2.6; design-combat-s3.md §5.1).
                if (self.until.is_none() && self.win.is_none()) || self.should_stop(cx) {
                    return self.finish(cx);
                }
                if let Some(stand) = self.request.stand {
                    match self.begin_return_to_stand(stand, cx) {
                        Ok(()) => Poll::Pending,
                        Err(error) => Poll::Ready(Err(error)),
                    }
                } else {
                    self.rebegin_combat(cx, true)
                }
            }
            CombatEnd::NoTarget | CombatEnd::Budget => {
                if (self.until.is_none() && self.win.is_none()) || self.should_stop(cx) {
                    self.finish(cx)
                } else {
                    self.rebegin_combat(cx, false)
                }
            }
            CombatEnd::Died => self.finish(cx),
            CombatEnd::Aborted(AbortReason::Unattackable)
                if matches!(&self.request.target, Target::Attacker { .. })
                    && has_live_attacker(&cx.tick.cx.snapshot(), &self.tables) != Some(true) =>
            {
                // Preserve the caller-owned walk-out when no live attacker
                // remains; otherwise use the same guarded escape as other aborts.
                self.finish(cx)
            }
            CombatEnd::Aborted(_) => self.begin_abort_safety_walk(report, cx),
        }
    }

    fn outcome_for(report: CombatReport, target_gone_restarts: u8) -> StepOutcome {
        StepOutcome {
            progress: None,
            evidence: report.evidence,
            receipt: Some(Arc::new(CombatReceipt {
                report,
                target_gone_restarts,
            })),
        }
    }

    fn refresh_outcome(&mut self) {
        let Some(report) = self.last_report else {
            self.last_outcome = None;
            return;
        };
        self.last_outcome = Some(Self::outcome_for(report, self.target_gone_restarts));
    }

    fn rebegin_combat(
        &mut self,
        cx: &mut StepContext<'_, '_>,
        target_gone: bool,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        match self.begin_combat(cx) {
            Ok(()) => {
                if target_gone {
                    self.target_gone_restarts = self.target_gone_restarts.saturating_add(1);
                    self.refresh_outcome();
                }
                Poll::Pending
            }
            Err(error) => Poll::Ready(Err(error)),
        }
    }

    fn begin_loot(&mut self, cx: &mut StepContext<'_, '_>) -> Result<LootStart, ActionError> {
        let snapshot = cx.tick.cx.snapshot();
        let Some(inventory) = snapshot.inventory() else {
            return Ok(LootStart::Waiting);
        };
        while let Some(item) = self.loot.get(self.loot_index) {
            let held = inventory
                .value
                .iter()
                .filter(|row| row.def.id == item.id)
                .map(|row| i64::from(row.count))
                .sum::<i64>();
            if held >= i64::from(item.qty) {
                self.loot_index += 1;
                continue;
            }
            let handle = cx.tick.actions.begin::<Reach>(
                ReachArgs {
                    kind: ReachKind::Ground {
                        id: item.id,
                        obj: Arc::clone(&item.name),
                    },
                    op: Arc::from("Take"),
                    anchor: self.request.stand,
                    radius: i32::from(self.request.lost_radius),
                    wait_if_missing: false,
                    target_tile: None,
                    reachable_only: false,
                },
                &mut cx.tick.cx,
            )?;
            self.loot_index += 1;
            self.action = Some(Action::Loot(handle));
            return Ok(LootStart::Started);
        }
        Ok(LootStart::Complete)
    }

    fn spend_finish_tick_budget(&mut self, tick: u64) -> bool {
        let elapsed = match self.finish_last_tick {
            Some(last_tick) => {
                self.finish_last_tick = Some(last_tick.max(tick));
                tick.saturating_sub(last_tick)
            }
            None => {
                self.finish_last_tick = Some(tick);
                0
            }
        };
        self.finish_ticks_elapsed = self.finish_ticks_elapsed.saturating_add(elapsed);
        self.finish
            .as_ref()
            .is_some_and(|finish| self.finish_ticks_elapsed <= u64::from(finish.max_ticks))
    }

    fn wait_for_finish_dialogue(
        &mut self,
        cx: &mut StepContext<'_, '_>,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        let ready_and_quiet = {
            let snapshot = cx.tick.cx.snapshot();
            let chat_ready = snapshot.chat_modal().is_some_and(|chat| {
                super::dialogue::chat_page_open(chat.value.root, chat.value.continue_component_id)
            });
            let combat_ended = snapshot
                .in_combat()
                .is_some_and(|combat| !combat.value.in_combat);
            chat_ready && combat_ended
        };
        if !ready_and_quiet {
            return Poll::Pending;
        }
        let Some(finish) = self.finish.as_ref() else {
            return Poll::Ready(Err(ActionError::Blocked(Arc::clone(&COMBAT_FINISH_FAILED))));
        };
        let handle = cx.tick.actions.begin::<Dialogue>(
            DialogueArgs {
                target: DialogueTarget::Continuation,
                options: finish.options.clone(),
            },
            &mut cx.tick.cx,
        );
        match handle {
            Ok(handle) => {
                self.phase = Phase::FinishDialogue;
                self.action = Some(Action::Dialogue(handle));
                // The finish page is already up: drive it now (TICK-FIX #14,
                // C-DIALOGUE-ADOPT). The reentry is in FinishDialogue, so it
                // polls the Dialogue and cannot begin another.
                if cx.tick.cx.may_continue_this_tick() {
                    return self.poll(cx);
                }
                Poll::Pending
            }
            Err(error) => Poll::Ready(Err(error)),
        }
    }

    fn finish_completed(
        &mut self,
        cx: &StepContext<'_, '_>,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        self.action = None;
        let (Some(finish), Some(npc_index)) = (self.finish.as_ref(), self.finish_target_index)
        else {
            return Poll::Ready(Err(ActionError::Blocked(Arc::clone(&COMBAT_FINISH_FAILED))));
        };
        Poll::Ready(Ok(StepOutcome {
            progress: None,
            evidence: cx.tick.cx.evidence(),
            receipt: Some(Arc::new(FinishReceipt {
                npc_type: finish.npc_type,
                npc_name: Arc::clone(&finish.npc_name),
                npc_index,
            })),
        }))
    }

    fn next_loot_or_reengage(
        &mut self,
        cx: &mut StepContext<'_, '_>,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        match std::task::ready!(crate::native::defer_budget(self.begin_loot(cx))) {
            Ok(LootStart::Waiting | LootStart::Started) => Poll::Pending,
            Ok(LootStart::Complete) => {
                if (self.until.is_none() && self.win.is_none()) || self.should_stop(cx) {
                    self.finish(cx)
                } else {
                    self.rebegin_combat(cx, false)
                }
            }
            Err(error) => Poll::Ready(Err(error)),
        }
    }

    fn evaluate_predicate(
        &self,
        predicate: &Arc<dyn PredicatePlan>,
        cx: &StepContext<'_, '_>,
    ) -> Truth {
        let chat_since = reach::last_chat_seq(&cx.tick.cx);
        let context = PredicateContext {
            cx: &cx.tick.cx,
            pairs: cx.tick.pairs,
            quests: cx.quests,
            progress: cx.progress,
            required_after: cx.required_after,
            chat_since,
            outcome: self.last_outcome.as_ref(),
        };
        predicate.evaluate(&context)
    }

    fn should_stop(&self, cx: &StepContext<'_, '_>) -> bool {
        self.until
            .as_ref()
            .is_some_and(|predicate| self.evaluate_predicate(predicate, cx) == Truth::True)
            || self
                .win
                .as_ref()
                .is_some_and(|predicate| self.evaluate_predicate(predicate, cx) == Truth::True)
    }

    fn finish(&mut self, cx: &StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        self.action = None;
        Poll::Ready(Ok(StepOutcome {
            progress: None,
            evidence: cx.tick.cx.evidence(),
            receipt: self
                .last_outcome
                .as_ref()
                .and_then(|outcome| outcome.receipt.as_ref().map(Arc::clone)),
        }))
    }
}

impl StepRun for CombatRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        self.capture_prayer_cleanup();
        if self.user_interrupted_since_begin(cx) {
            self.action = None;
            return Poll::Ready(Err(ActionError::UserInput));
        }
        if matches!(self.phase, Phase::FinishWait | Phase::FinishDialogue)
            && !self.spend_finish_tick_budget(cx.tick.cx.evidence().tick)
        {
            self.action = None;
            return Poll::Ready(Err(ActionError::Blocked(Arc::clone(
                &COMBAT_FINISH_TIMEOUT,
            ))));
        }
        if matches!(self.phase, Phase::FinishWait) {
            return self.wait_for_finish_dialogue(cx);
        }
        if matches!(self.phase, Phase::Combat)
            && matches!(self.action.as_ref(), Some(Action::Combat(_)))
            && self.observe_finish_transform(cx)
        {
            self.capture_prayer_cleanup();
            self.action = None;
            self.phase = Phase::FinishWait;
            self.finish_ticks_elapsed = 0;
            self.finish_last_tick = Some(cx.tick.cx.evidence().tick);
            self.last_report = None;
            self.last_outcome = None;
            return Poll::Pending;
        }
        if matches!(self.phase, Phase::Loot) && self.action.is_none() {
            match std::task::ready!(crate::native::defer_budget(self.begin_loot(cx))) {
                Ok(LootStart::Waiting | LootStart::Started) => return Poll::Pending,
                Ok(LootStart::Complete) => {
                    if (self.until.is_none() && self.win.is_none()) || self.should_stop(cx) {
                        return self.finish(cx);
                    }
                    return self.rebegin_combat(cx, false);
                }
                Err(error) => return Poll::Ready(Err(error)),
            }
        }
        if !matches!(
            self.phase,
            Phase::Loot
                | Phase::HoldingAfterAbort
                | Phase::EscapingAfterAbortHold
                | Phase::FinishWait
                | Phase::FinishDialogue
        ) && self.should_stop(cx)
        {
            return self.finish(cx);
        }
        let polled = match self.action.as_ref() {
            Some(Action::Combat(handle)) => match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => ActionPoll::Pending,
                Poll::Ready(Ok(report)) => ActionPoll::Combat(report),
                Poll::Ready(Err(error)) => ActionPoll::Failed(error),
            },
            Some(Action::Walk(handle)) => match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => ActionPoll::Pending,
                Poll::Ready(Ok(receipt)) => ActionPoll::Walk(receipt),
                Poll::Ready(Err(error)) => ActionPoll::Failed(error),
            },
            Some(Action::AbortHold(handle)) => {
                match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                    Poll::Pending => ActionPoll::Pending,
                    Poll::Ready(Ok(outcome)) => ActionPoll::AbortHold(outcome),
                    Poll::Ready(Err(error)) => ActionPoll::Failed(error),
                }
            }
            Some(Action::Loot(handle)) => match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => ActionPoll::Pending,
                Poll::Ready(Ok(taken)) => ActionPoll::Loot(taken),
                Poll::Ready(Err(error)) => ActionPoll::Failed(error),
            },
            None => ActionPoll::Pending,
            Some(Action::Dialogue(handle)) => match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => ActionPoll::Pending,
                Poll::Ready(Ok(outcome)) => ActionPoll::Dialogue(outcome),
                Poll::Ready(Err(error)) => ActionPoll::Failed(error),
            },
        };
        match polled {
            ActionPoll::Pending => Poll::Pending,
            ActionPoll::Failed(error) if matches!(&self.phase, Phase::WalkingOutAfterAbort) => {
                if matches!(&error, ActionError::UserInput | ActionError::Cancelled) {
                    Poll::Ready(Err(error))
                } else {
                    self.action = None;
                    if has_live_attacker(&cx.tick.cx.snapshot(), &self.tables) == Some(true) {
                        self.begin_abort_hold(cx)
                    } else if matches!(&error, ActionError::NeedsEvidence(_)) {
                        Poll::Ready(Err(error))
                    } else {
                        Poll::Ready(Err(ActionError::Blocked(Arc::clone(&ABORT_WALK_FAILED))))
                    }
                }
            }
            ActionPoll::Failed(ActionError::Blocked(_)) if matches!(&self.phase, Phase::Loot) => {
                // Loot is optional: an unreachable ground item does not end
                // the combat step. Other errors remain visible to the caller.
                self.action = None;
                self.next_loot_or_reengage(cx)
            }
            ActionPoll::Failed(ActionError::Failed(reason))
                if matches!(&self.phase, Phase::FinishDialogue) =>
            {
                self.action = None;
                Poll::Ready(Err(ActionError::Blocked(Arc::from(format!(
                    "{}: {reason}",
                    COMBAT_FINISH_FAILED.as_ref()
                )))))
            }
            ActionPoll::Failed(error) => {
                self.action = None;
                Poll::Ready(Err(error))
            }
            ActionPoll::AbortHold(outcome) => {
                self.action = None;
                match outcome {
                    AbortHoldOutcome::ThreatFree => {
                        Poll::Ready(Err(ActionError::Blocked(Arc::clone(&ABORT_HOLD_SAFE))))
                    }
                    AbortHoldOutcome::SuppliesExhausted => self.begin_abort_escape_walk(cx),
                }
            }
            ActionPoll::Combat(report) => self.on_combat_report(report, cx),
            ActionPoll::Walk(receipt) if matches!(&self.phase, Phase::WalkingOutAfterAbort) => {
                self.on_abort_walk(receipt, cx)
            }
            ActionPoll::Walk(receipt) if matches!(&self.phase, Phase::EscapingAfterAbortHold) => {
                self.on_abort_escape_walk(receipt)
            }
            ActionPoll::Walk(receipt) => self.on_return_walk(receipt, cx),
            ActionPoll::Loot(taken) => {
                self.action = None;
                if !taken {
                    // A stale or absent drop is not evidence for another click.
                    // The next combat run can produce a fresh ground row.
                }
                self.next_loot_or_reengage(cx)
            }
            ActionPoll::Dialogue(DialogueOutcome::Completed) => self.finish_completed(cx),
            ActionPoll::Dialogue(DialogueOutcome::Failed) => {
                self.action = None;
                Poll::Ready(Err(ActionError::Blocked(Arc::clone(&COMBAT_FINISH_FAILED))))
            }
            ActionPoll::Dialogue(DialogueOutcome::CombatInterrupted) => {
                self.action = None;
                Poll::Ready(Err(ActionError::Blocked(Arc::clone(
                    &COMBAT_FINISH_INTERRUPTED,
                ))))
            }
        }
    }

    fn cancel(&mut self, _actions: &mut NativeActions) {
        self.capture_prayer_cleanup();
        self.action = None;
    }
    fn prayer_cleanup(&self) -> RaisedPrayers {
        CombatRun::prayer_cleanup(self)
    }

    fn take_trace_event(&mut self) -> Option<StepTraceEvent> {
        self.trace_event.take()
    }
    fn in_flight_outcome(&self) -> Option<&StepOutcome> {
        self.last_outcome.as_ref()
    }
}

struct CombatEndPredicate {
    expected: CombatEndName,
}

impl PredicatePlan for CombatEndPredicate {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        let Some(report) = cx
            .outcome
            .and_then(|outcome| outcome.receipt.as_ref())
            .and_then(|receipt| receipt.as_any().downcast_ref::<CombatReceipt>())
            .map(|receipt| receipt.report)
        else {
            // No Quester combat report is a known miss, not missing snapshot
            // evidence. Path skip_if of the producing combat step can then
            // start it, and the caller walk after Aborted(Unattackable) can
            // stay skipped until that report exists (design-combat.md:664).
            return Truth::False;
        };
        if self.expected.matches(report.end) {
            Truth::True
        } else {
            Truth::False
        }
    }
}

impl CombatEndName {
    fn matches(&self, end: CombatEnd) -> bool {
        matches!(
            (self, end),
            (Self::Killed, CombatEnd::Killed)
                | (Self::TargetGone, CombatEnd::TargetGone)
                | (Self::NoTarget, CombatEnd::NoTarget)
                | (Self::Budget, CombatEnd::Budget)
                | (Self::Died, CombatEnd::Died)
                | (Self::Aborted, CombatEnd::Aborted(_))
                | (
                    Self::AbortedUnattackable,
                    CombatEnd::Aborted(AbortReason::Unattackable)
                )
        )
    }
}

#[cfg(test)]
#[path = "combat_tests.rs"]
pub(crate) mod tests;
