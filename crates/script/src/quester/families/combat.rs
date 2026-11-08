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
    PrayerMode, RaisedPrayers, RangedMode, SpellRef, Style, StyleKits, SwitchPolicy, Tactic,
    Target, WalkGuard,
};
use crate::dialogue_outcome::DialogueOutcome;
use crate::interact::InteractReq;
use crate::native::walk::Walk;
use crate::native::{
    ActionContext, ActionError, ActionHandle, NativeActions, NativeMachine, WalkAllow, WalkReceipt,
};
use api::game_data::wearpos_for_loadout_slot;
use api::gather_methods::SceneRegionInput;
use api::selected::Truth;
use serde::Deserialize;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;
#[path = "combat_escape.rs"]
mod escape;
use escape::{AbortEscape, EscapeGeometry, EscapeGoals};

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
/// Blocked reason when an exhausted-supplies escape walk arrives but an
/// attacker is still live or cannot be observed, so the bot is not proved safe.
pub const ABORT_ESCAPE_UNSAFE_REASON: &str =
    "combat abort escape walk arrived without proof that the attacker is gone";
/// Blocked reason when an escape walk reaches no terminal receipt within its
/// observed-tick budget, so it is abandoned and the next tier starts.
pub const ABORT_ESCAPE_STALLED_REASON: &str =
    "combat abort escape walk made no settled progress within its tick budget";

static ABORTED_COMBAT_STEP: std::sync::LazyLock<Arc<str>> =
    std::sync::LazyLock::new(|| Arc::from(ABORTED_COMBAT_STEP_REASON));
static ABORT_WALK_FAILED: std::sync::LazyLock<Arc<str>> =
    std::sync::LazyLock::new(|| Arc::from(ABORT_WALK_FAILED_REASON));
static COMBAT_FINISH_TIMEOUT: std::sync::LazyLock<Arc<str>> =
    std::sync::LazyLock::new(|| Arc::from("combat finish exceeded its tick budget"));
static COMBAT_FINISH_FAILED: std::sync::LazyLock<Arc<str>> =
    std::sync::LazyLock::new(|| Arc::from("combat finish dialogue failed"));
static ABORT_HOLD_SAFE: std::sync::LazyLock<Arc<str>> =
    std::sync::LazyLock::new(|| Arc::from(ABORT_HOLD_SAFE_REASON));
static ABORT_HOLD_UNAVAILABLE: std::sync::LazyLock<Arc<str>> =
    std::sync::LazyLock::new(|| Arc::from(ABORT_HOLD_UNAVAILABLE_REASON));
static ABORT_HOLD_ESCAPED: std::sync::LazyLock<Arc<str>> =
    std::sync::LazyLock::new(|| Arc::from(ABORT_HOLD_ESCAPED_REASON));
static ABORT_ESCAPE_UNSAFE: std::sync::LazyLock<Arc<str>> =
    std::sync::LazyLock::new(|| Arc::from(ABORT_ESCAPE_UNSAFE_REASON));
static ABORT_ESCAPE_STALLED: std::sync::LazyLock<Arc<str>> =
    std::sync::LazyLock::new(|| Arc::from(ABORT_ESCAPE_STALLED_REASON));
/// From hitpoints reaching zero, the queued death callback runs on the next NPC
/// turn, then arrival delay 1–2 plus `npc_delay(1)` precede the drop: at most
/// five server ticks (`npc_combat.rs2:78-93`, `Npc.ts:250-253,566-575`,
/// `LinkList.ts:67-70`, `npc_death.rs2:15-20`, `NpcOps.ts:97-100,557-571`,
/// `elemental_earth.rs2:1-6`). Add one observed tick for the single
/// `PLAYER_INFO` received per tick.
const LOOT_ITEM_WAIT_TICKS: u64 = 5 + 1;

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
    pub kills: u32,
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
    /// Optional named loadout for the encounter: the loadout of the opening
    /// `tactic.style`.
    #[serde(default)]
    loadout: Option<String>,
    /// Optional named loadout per combat style. The machine wears the
    /// loadout of a style when it switches to that style in the middle of a
    /// fight (when the opponent protects against the opening style, when
    /// ammunition or runes run out, and when the opponent stays out of melee
    /// reach). A style with no loadout is never a switch target. Naming the
    /// opening style here and in `loadout` is refused.
    #[serde(default)]
    loadouts: Option<StyleLoadoutsArg>,
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
    /// Opt in to START a fight with the named `target.player`. The global
    /// operator setting `pvp.start_fights` also opts in; without either the
    /// step is refused (`pvp-not-allowed`). Fighting back needs neither.
    #[serde(default)]
    pvp: bool,
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
    /// Loot one selected item until held, re-engaging up to `max_kills`.
    #[serde(default)]
    until_drop: Option<UntilDropArgs>,
}

/// Item and confirmed-kill budget for a combat step that waits for its drop.
#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct UntilDropArgs {
    /// Selected item alias to loot and hold.
    obj: String,
    /// Maximum confirmed kills, including the kill whose loot is collected.
    #[cfg_attr(feature = "path-schema", schemars(range(min = 1)))]
    max_kills: u32,
    /// Authored stop name reported when the kill budget is spent without the item.
    #[cfg_attr(feature = "path-schema", schemars(length(min = 1)))]
    #[cfg_attr(feature = "path-schema", schemars(regex(pattern = r"\S")))]
    stop: String,
}

/// The named loadout of each combat style (PVP-DESIGN-1026 §4.1).
#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct StyleLoadoutsArg {
    /// Name or Path-qualified name of the melee loadout.
    #[serde(default)]
    melee: Option<String>,
    /// Name or Path-qualified name of the ranged loadout.
    #[serde(default)]
    ranged: Option<String>,
    /// Name or Path-qualified name of the magic loadout.
    #[serde(default)]
    mage: Option<String>,
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
    /// Skip a `prefer` entry that already answered a page in this
    /// conversation, so a repeated menu can be answered once and left.
    /// `line_rules` and `choose` are unaffected. With this option on,
    /// `prefer` holds at most 32 entries; a longer list fails compilation.
    #[serde(default)]
    answer_once: bool,
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
    /// A named player to fight. Where the game allows player combat (the
    /// Wilderness and a duel pen) and the step or the global setting opts in.
    #[serde(default)]
    player: Option<String>,
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
    if args.until_drop.is_some()
        && (args.finish.is_some() || args.until.is_some() || args.win.is_some())
    {
        return Err(CompileError::code("conflicting-combat-until-drop"));
    }
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

    let kits = compile_kits(args.loadout.as_deref(), args.loadouts.as_ref(), style, cx)?;
    let mut loot = compile_loot(&args.loot, cx)?;
    let until_drop = match args.until_drop {
        Some(args) => {
            let (required, loot_item) = compile_until_drop(args, cx)?;
            if !loot.iter().any(|item| item.id == required.item_id) {
                loot.push(loot_item);
            }
            Some(required)
        }
        None => None,
    };
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
    let reengage_budget = if let Some(until_drop) = &until_drop {
        until_drop.max_kills
    } else if until.is_some() || win.is_some() {
        content_reengagement_budget(args.kill_budget_ticks)
    } else {
        0
    };
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

    let cross: Arc<[Arc<str>]> = args.cross.into_iter().map(Arc::from).collect();
    let request = CombatRequest {
        target,
        tactic: Tactic::Open,
        style,
        melee_mode: args.melee_mode,
        ranged_style: args.tactic.ranged_style,
        kits,
        switch: SwitchPolicy::for_style(style),
        spells,
        fallback_spells: args.fallback_spells,
        stand,
        search_bounds,
        cross: Some(Arc::clone(&cross)),
        engage_radius: args.tactic.engage_radius,
        lost_radius: args.lost_radius,
        budget_ticks: args.kill_budget_ticks,
        allow: Allowances::default(),
        fallback: Fallback::Abort,
        intruder: IntruderPolicy::default(),
        retaliate: args.tactic.auto_retaliate,
        prayer_mode: PrayerMode::Hold,
        pvp: args.pvp,
        ..CombatRequest::default()
    };

    Ok(CombatPlan {
        request: Arc::new(request),
        tables,
        cross,
        protect,
        until,
        win,
        loot: Arc::from(loot),
        finish,
        until_drop,
        reengage_budget,
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
    if let Some(name) = args.player {
        let name = name.trim();
        if name.is_empty() || name.len() > crate::api_combat::MAX_NAME_BYTES {
            return Err(CompileError::code("invalid-combat-target"));
        }
        if args.pick.is_some() || args.not_targeting_others.is_some() {
            return Err(CompileError::code("invalid-combat-target"));
        }
        return Ok(Target::Player {
            name: Arc::from(name),
        });
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

/// The pinned R289 NPC rows top out at 700 ticks
/// (`content/scripts/areas/area_desert/configs/desert.npc:65`;
/// `content/scripts/areas/area_varrock/configs/varrock.npc:626`).
/// `NPC_DEL` passes this rate to `World.removeNpc`
/// (`engine/src/engine/script/handlers/NpcOps.ts:93-95`), which scales it down
/// by player count (`engine/src/engine/World.ts:1312-1333,1731-1734`).
/// The combat machine ends a missing-target attempt after three ticks
/// (`combat/machine.rs:1201-1207`), so allow enough non-kill retries to cover
/// one maximum respawn interval. A shorter requested combat budget needs more
/// retries. Zero uses the engine's 1,500-tick default (`combat/machine.rs:271-275`).
const MAX_CONTENT_NPC_RESPAWN_TICKS: u32 = 700;
const TARGET_GONE_TICKS: u32 = 3;
const DEFAULT_COMBAT_BUDGET_TICKS: u32 = 1_500;

fn content_reengagement_budget(budget_ticks: u16) -> u32 {
    let attempt_ticks = if budget_ticks == 0 {
        DEFAULT_COMBAT_BUDGET_TICKS
    } else {
        u32::from(budget_ticks)
    };
    let shortest_end_ticks = attempt_ticks.min(TARGET_GONE_TICKS);
    MAX_CONTENT_NPC_RESPAWN_TICKS.div_ceil(shortest_end_ticks) - 1
}

/// Confirmed kills a quantity-aware `until` loop spends on missed loot before
/// the step fails. A settled loot miss re-engages like any chance outcome
/// (AGENTS.md: retry within a bounded, content-justified budget). Separate
/// from the per-fight `kill_budget_ticks` and the non-kill `reengage_budget`.
///
/// Policy: one shared constant for every loot-and-`until` step, sized from
/// the hardest known authored case. A step whose drop is rarer, or whose
/// target is larger, may therefore end at this limit.
///
/// Rule: enough kills that the hardest known target is at least 99% likely to
/// be reached. That is The Knight's Sword on 377 (`paths/377/squire.json`,
/// step `dwarf-iron-bar-drops`): two iron bars, which a dwarf drops on 3 of
/// `random(128)` (content `scripts/drop tables/scripts/dwarf.rs2:47-48`). The
/// smallest `n` with P(Binomial(n, 3/128) >= 2) >= 0.99 is 281 (0.99012; 280
/// gives 0.98992). In the same 281 kills: Doric's two iron ores at 4/128
/// (`dwarf.rs2:13,39-42`) reach 0.99866; Ikov's 20 limpwurt roots at 22/128
/// (`hobgoblin.rs2:63-64`) reach 0.9994 or more; Gobdip's three goblin armour
/// at 5/128 (`goblin.rs2:75-76`) reach 0.99894.
const DROP_LOOP_KILL_BUDGET: u32 = 281;

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
        answer_once: args.answer_once,
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

/// The kit of every named style (PVP-DESIGN-1026 §4.1): `loadout` is the kit of
/// the opening `style`, `loadouts` names the kit of each style. Naming the
/// opening style in both is refused, since the two would disagree about it.
fn compile_kits(
    loadout: Option<&str>,
    loadouts: Option<&StyleLoadoutsArg>,
    style: Style,
    cx: &CompileContext<'_>,
) -> Result<StyleKits, CompileError> {
    let mut kits = StyleKits::default();
    if let Some(name) = loadout {
        kits = kits.with(style, compile_kit(name, cx)?);
    }
    if let Some(named) = loadouts {
        for (kit_style, name) in [
            (Style::Melee, named.melee.as_deref()),
            (Style::Ranged, named.ranged.as_deref()),
            (Style::Mage, named.mage.as_deref()),
        ] {
            let Some(name) = name else {
                continue;
            };
            if kit_style == style && loadout.is_some() {
                return Err(CompileError::code("conflicting-combat-loadouts"));
            }
            kits = kits.with(kit_style, compile_kit(name, cx)?);
        }
    }
    Ok(kits)
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
    let (mut two_hander, mut shield) = (false, false);
    for (slot, item) in &row.worn {
        // The wear position the server uses (`quiver` is 13, `legs` 7), not the
        // slot name's index in the loadout's own list.
        let slot = wearpos_for_loadout_slot(&slot.to_ascii_lowercase())
            .and_then(|position| u8::try_from(position).ok())
            .ok_or_else(|| CompileError::code("invalid-loadout-slot"))?;
        let id = resolve_loadout_item(cx, item)?.0;
        // A two-hander and a shield cannot be worn together: `try_equip` vacates
        // one for the other (`player/scripts/equip.rs2:355-358,371`).
        two_hander |= slot == 3
            && cx
                .selected
                .item_by_id(id)
                .is_some_and(|item| item.is_two_handed());
        shield |= slot == 5;
        worn.push((slot, id));
    }
    if two_hander && shield {
        return Err(CompileError::code("two-handed-kit-with-shield"));
    }
    // Carry ids come from the compiled loadout rows the native consumers use:
    // shared display names can collide (e.g. `Coins` is both 617 and 995),
    // while the compiled rows retain the authored alias identity.
    let compiled_carry = super::s2::compile_loadout_carry(&qualified, row, cx)?;
    let mut carry = Vec::with_capacity(compiled_carry.len());
    for entry in compiled_carry.iter() {
        let qty = u32::try_from(entry.qty)
            .ok()
            .filter(|qty| *qty > 0)
            .ok_or_else(|| CompileError::code("invalid-combat-loadout-quantity"))?;
        carry.push((entry.item.id, qty));
    }
    Ok(Arc::new(CompiledKit {
        worn: Arc::from(worn),
        carry: Arc::from(carry),
        special: None,
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

fn compile_until_drop(
    args: UntilDropArgs,
    cx: &CompileContext<'_>,
) -> Result<(UntilDropConfig, LootItem), CompileError> {
    if args.max_kills == 0 {
        return Err(CompileError::code("invalid-combat-until-drop-kill-budget"));
    }
    if args.stop.trim().is_empty() {
        return Err(CompileError::code("invalid-combat-until-drop-stop"));
    }
    let (item_id, item_name) = resolve_loadout_item(cx, &args.obj)?;
    Ok((
        UntilDropConfig {
            item_id,
            max_kills: args.max_kills,
            stop: Arc::from(args.stop),
        },
        LootItem {
            id: item_id,
            name: item_name,
            qty: 1,
        },
    ))
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
    let revision = match cx.selected.revision() {
        274 => api::selected::ClientRevision::R274,
        289 => api::selected::ClientRevision::R289,
        377 => api::selected::ClientRevision::R377,
        _ => return Err(CompileError::code("unsupported-revision")),
    };
    let selected = api::game_data::for_revision(revision)
        .map_err(|_| CompileError::code("combat-tables-unavailable"))?;
    CombatTables::build(selected).map_err(|_| CompileError::code("combat-tables-unavailable"))
}

#[derive(Clone)]
struct LootItem {
    id: i32,
    name: Arc<str>,
    qty: u32,
}

#[derive(Clone)]
struct UntilDropConfig {
    item_id: i32,
    max_kills: u32,
    stop: Arc<str>,
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
    until_drop: Option<UntilDropConfig>,
    /// `until_drop` uses its authored kill budget; `until`/`win` span a content respawn window.
    reengage_budget: u32,
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
            until_drop: self.until_drop.clone(),
            reengage_budget: self.reengage_budget,
            action: None,
            phase: Phase::Combat,
            loot_index: 0,
            loot_wait_deadline: None,
            loot_missing_this_kill: false,
            loot_retake_tick: None,
            last_report: None,
            kills: 0,
            nonkill_reengagements: 0,
            last_outcome: None,
            target_gone_restarts: 0,
            stand_walk_failures: 0,
            raised_prayers: RaisedPrayers::empty(),
            manual_takeover_watch: crate::native::ManualTakeoverWatch::default(),
            finish_target_index: None,
            finish_ticks_elapsed: 0,
            finish_last_tick: None,
            trace_event: None,
            abort_escape: AbortEscape::new(),
            return_retry: crate::native::WalkRetryState::default(),
        };
        if !run.until_drop_item_held(cx) {
            run.begin_combat(cx)?;
        }
        run.manual_takeover_watch = crate::native::ManualTakeoverWatch::begin(&cx.tick.cx);

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
            if let Some(op) = guard.tick(&snapshot, cx.condition(), cx.cure_poison()) {
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
        GuardOp::Drink { name, .. } => Some(InteractReq::Held {
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
    until_drop: Option<UntilDropConfig>,
    action: Option<Action>,
    phase: Phase,
    loot_index: usize,
    loot_wait_deadline: Option<u64>,
    loot_missing_this_kill: bool,
    /// First rebuilt-scene failure tick for the current row. `Some` also
    /// marks its one retake as spent; every row advancement clears it.
    loot_retake_tick: Option<u64>,
    last_report: Option<CombatReport>,
    kills: u32,
    last_outcome: Option<StepOutcome>,
    target_gone_restarts: u8,
    reengage_budget: u32,
    /// Non-kill re-engagements used by this step; successful kills do not reset the bound.
    nonkill_reengagements: u32,
    /// Stand-approach walk failures every fight attempt of this step spent. A
    /// kill-budget end rebegins the fight inside the same step, so the bound
    /// of three spans attempts; the field starts at zero and only the step's
    /// end discards it.
    stand_walk_failures: u8,
    raised_prayers: RaisedPrayers,
    manual_takeover_watch: crate::native::ManualTakeoverWatch,
    finish: Option<FinishConfig>,
    finish_target_index: Option<usize>,
    finish_ticks_elapsed: u64,
    finish_last_tick: Option<u64>,
    trace_event: Option<StepTraceEvent>,
    /// Destinations tried and the last walk failure of the current abort.
    abort_escape: AbortEscape,
    return_retry: crate::native::WalkRetryState,
}

impl CombatRun {
    /// Mirror the live attempt's stand-approach count. The run outlives one
    /// fight attempt: a kill-budget end starts the next attempt from this
    /// count, so the three-failure bound is the step's, not the attempt's.
    fn observe_stand_walks(&mut self) {
        let Some(Action::Combat(handle)) = self.action.as_ref() else {
            return;
        };
        if let Some(failures) = handle.inspect(|machine| machine.stand_walk_failures()) {
            self.stand_walk_failures = failures;
        }
    }

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

    fn begin_combat(&mut self, cx: &mut StepContext<'_, '_>) -> Result<(), ActionError> {
        self.finish_target_index = None;
        self.finish_ticks_elapsed = 0;
        self.finish_last_tick = None;
        let handle = cx.tick.actions.begin::<Combat>(
            (
                Arc::clone(&self.request),
                Arc::clone(&self.tables),
                cx.tick
                    .pvp_globals
                    .unwrap_or_else(crate::native::PvpGlobals::fail_closed),
            ),
            &mut cx.tick.cx,
        )?;
        handle.seed_stand_walk_failures(self.stand_walk_failures);
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
        self.return_retry.action_submitted();
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

    /// The next escape request from the observed position. It sends one exact
    /// tier, and the host router decides reachability. `None` when every tier
    /// of the pass is spent or empty.
    fn abort_walk_goals(
        &mut self,
        report: CombatReport,
        cx: &StepContext<'_, '_>,
    ) -> Option<EscapeGoals> {
        let snapshot = cx.tick.cx.snapshot();
        snapshot.here()?;
        // Retreat geometry compares packet-time network tiles. Rendered poses
        // interpolate and can point the retreat toward the attacker.
        let here = snapshot.local_player()?.value.player.network;
        let reference = self.abort_reference(report, &snapshot);
        // The chase envelope from the NPC's current tile is up to
        // `2 * maxrange + attackrange` (Npc.ts:652-667; the tiers in
        // `combat_escape.rs` give the derivation).
        let chase_range = match reference {
            Some(reference) if reference.actor.kind == api::snapshot::ActorKind::Npc => self
                .tables
                .selected()
                .npc_name(reference.npc_type)
                .map(|npc| {
                    npc.maxrange
                        .saturating_mul(2)
                        .saturating_add(npc.attackrange)
                        .max(0)
                })
                .unwrap_or(10),
            _ => 0,
        };
        let geometry = EscapeGeometry {
            here,
            threat: reference.map(|reference| reference.tile),
            stand: self.request.stand,
            chase_range,
        };
        let reach = crate::reach_route::live(snapshot.reach().map(|reach| reach.value), Some(here));
        self.abort_escape
            .next_goals(&geometry, cx.tick.frame.compiled.collision, reach)
    }

    /// The escape walk for `goals`: exact Area goals from one tier. The router
    /// chooses among these goals.
    fn escape_walk_request(
        &self,
        goals: EscapeGoals,
        cx: &StepContext<'_, '_>,
    ) -> crate::native::WalkRequest {
        let mut request = reach::walk_request(goals.target, 0, None, cx.required_after);
        request.arrival = nav_planner::contract::ArrivalKind::Area;
        request.candidate_goals = Some(goals.goals);
        request.cross = self.cross.iter().cloned().collect();
        request
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
        let goals = self
            .abort_walk_goals(report, cx)
            .ok_or_else(|| ActionError::Blocked(Arc::clone(&ABORT_WALK_FAILED)))?;
        let budget_ticks = goals.budget_ticks;
        let mut request = self.escape_walk_request(goals, cx);
        let snapshot = cx.tick.cx.snapshot();
        let allow_prayer = self.abort_protection_allowed(&snapshot);
        request.protect = allow_prayer;
        request.food_guard = true;
        request.allow = WalkAllow {
            prayer: allow_prayer,
            food: true,
            escape: true,
        };
        let handle = cx.tick.actions.begin::<Walk>(request, &mut cx.tick.cx)?;
        self.abort_escape
            .start_walk(cx.tick.cx.evidence().tick, budget_ticks);
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

    /// The Unattackable abort of an Attacker target hands back to the caller
    /// once the bot is safe: the caller's own walk-out runs from the retained
    /// combat receipt. Every other abort ends the step as a failure.
    fn abort_hands_back_to_caller(&self) -> bool {
        matches!(&self.request.target, Target::Attacker { .. })
            && self.last_report.is_some_and(|report| {
                matches!(report.end, CombatEnd::Aborted(AbortReason::Unattackable))
            })
    }

    /// The error of an abort whose escape attempts are all spent: the last
    /// walk failure when a walk failed, otherwise `fallback`.
    fn abort_failure(&self, fallback: &Arc<str>) -> ActionError {
        self.abort_escape
            .last_failure()
            .cloned()
            .unwrap_or_else(|| ActionError::Blocked(Arc::clone(fallback)))
    }

    fn begin_abort_escape_walk(
        &mut self,
        cx: &mut StepContext<'_, '_>,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        let Some(report) = self.last_report else {
            return Poll::Ready(Err(self.abort_failure(&ABORT_WALK_FAILED)));
        };
        let Some(goals) = self.abort_walk_goals(report, cx) else {
            return Poll::Ready(Err(self.abort_failure(&ABORT_WALK_FAILED)));
        };
        let budget_ticks = goals.budget_ticks;
        let mut request = self.escape_walk_request(goals, cx);
        request.protect = false;
        request.food_guard = false;
        request.cure_guard = true;
        request.allow = WalkAllow {
            prayer: false,
            food: false,
            escape: true,
        };
        let handle = match cx.tick.actions.begin::<Walk>(request, &mut cx.tick.cx) {
            Ok(handle) => handle,
            Err(error) => return Poll::Ready(Err(error)),
        };
        self.abort_escape
            .start_walk(cx.tick.cx.evidence().tick, budget_ticks);
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
            Err(_) => Poll::Ready(Err(self.abort_failure(&ABORT_WALK_FAILED))),
        }
    }

    fn on_return_walk(
        &mut self,
        receipt: WalkReceipt,
        cx: &mut StepContext<'_, '_>,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        self.action = None;
        let route_ended_at_stand =
            matches!(&receipt.end, crate::native::WalkEnd::RouteEnded)
                && crate::native::WalkRetryState::retryable_receipt(&receipt)
                && self.request.stand.is_some_and(|stand| {
                    cx.tick.cx.snapshot().local_player().is_some_and(|local| {
                        crate::combat::select::stand_reached(stand, local.value)
                    })
                });
        if route_ended_at_stand {
            self.return_retry.reset();
            return self.rebegin_combat(cx, true);
        }
        if self
            .return_retry
            .observe_approach_failure(&receipt, cx.tick.cx.evidence().tick)
        {
            return Poll::Pending;
        }
        receipt.into_arrival()?;
        self.return_retry.reset();
        self.rebegin_combat(cx, true)
    }

    /// Ends an escape walk that stays unsettled past its observed-tick
    /// budget. Dropping the handle revokes the host request, so the router
    /// stops following it. The walk then ends as a failed receipt, and the
    /// handlers of a failed walk start the next tier from the observed
    /// position, as for a failure the host reported.
    fn expire_escape_walk(
        &mut self,
        cx: &mut StepContext<'_, '_>,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        let tracked = matches!(
            self.phase,
            Phase::WalkingOutAfterAbort | Phase::EscapingAfterAbortHold
        ) && matches!(self.action, Some(Action::Walk(_)));
        if !tracked || !self.abort_escape.walk_expired(cx.tick.cx.evidence().tick) {
            return Poll::Pending;
        }
        self.action = None;
        let receipt = WalkReceipt {
            goal: None,
            request_id: 0,
            evidence: cx.tick.cx.evidence(),
            end: crate::native::WalkEnd::Failed,
            blocked: None,
            detail: Some(Arc::clone(&ABORT_ESCAPE_STALLED)),
            refusal: None,
            assessment: None,
            escape: None,
            prereq_allow: None,
        };
        if matches!(self.phase, Phase::WalkingOutAfterAbort) {
            self.on_abort_walk(receipt, cx)
        } else {
            self.on_abort_escape_walk(receipt, cx)
        }
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
            Ok(_) if !has_attacker && self.abort_hands_back_to_caller() => {
                return self.finish(cx);
            }
            Ok(_) => ActionError::Blocked(Arc::clone(&ABORTED_COMBAT_STEP)),
            Err(error @ (ActionError::UserInput | ActionError::Cancelled)) => {
                return Poll::Ready(Err(error));
            }
            Err(error @ ActionError::NeedsEvidence(_)) if !has_attacker => {
                return Poll::Ready(Err(error));
            }
            Err(error) => {
                // A failed route is information: plan again from the observed
                // position while the attacker is live, preferring the next
                // destination tier. The walk guard keeps eating on every attempt.
                self.abort_escape.record_failure(error.clone());
                if let Some(report) = self.last_report.filter(|_| has_attacker) {
                    return self.begin_abort_safety_walk(report, cx);
                }
                error
            }
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
        cx: &mut StepContext<'_, '_>,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        self.action = None;
        match receipt.into_arrival() {
            Ok(_) if self.abort_hands_back_to_caller() => {
                // The exhausted pass runs without prayer or food, and a tier
                // may name tiles that are only farther from the attacker.
                // Arrival alone is not safety: complete only on an observed
                // absence of every attacker. Otherwise plan the next tier.
                if has_live_attacker(&cx.tick.cx.snapshot(), &self.tables) == Some(false) {
                    self.finish(cx)
                } else {
                    self.abort_escape
                        .record_failure(ActionError::Blocked(Arc::clone(&ABORT_ESCAPE_UNSAFE)));
                    self.begin_abort_escape_walk(cx)
                }
            }
            Ok(_) => Poll::Ready(Err(ActionError::Blocked(Arc::clone(&ABORT_HOLD_ESCAPED)))),
            Err(
                error @ (ActionError::UserInput
                | ActionError::Cancelled
                | ActionError::NeedsEvidence(_)),
            ) => Poll::Ready(Err(error)),
            Err(error) => {
                self.abort_escape.record_failure(error);
                self.begin_abort_escape_walk(cx)
            }
        }
    }

    fn on_combat_report(
        &mut self,
        report: CombatReport,
        cx: &mut StepContext<'_, '_>,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        self.action = None;
        self.raised_prayers = RaisedPrayers::empty();
        if matches!(report.end, CombatEnd::Killed) {
            self.kills = self.kills.saturating_add(1);
        }
        self.last_report = Some(report);
        self.refresh_outcome();
        self.trace_event = Some(StepTraceEvent::CombatSubOperationEnd {
            target: self.request.target.clone(),
            end: report.end,
        });
        match report.end {
            // A configured finish owns the post-kill dialogue; other runs proceed
            // to loot only after the combat machine reports `Killed`
            // (design-combat.md §2.3, §5; combat-s3a-ReviewCombatFable.md F5).
            CombatEnd::Killed if self.finish.is_some() => {
                self.enter_finish_wait(cx.tick.cx.evidence().tick);
                Poll::Pending
            }
            CombatEnd::Killed => {
                self.phase = Phase::Loot;
                self.loot_index = 0;
                self.loot_wait_deadline = Some(
                    cx.tick
                        .cx
                        .evidence()
                        .tick
                        .saturating_add(LOOT_ITEM_WAIT_TICKS),
                );
                self.loot_missing_this_kill = false;
                self.loot_retake_tick = None;
                Poll::Pending
            }
            CombatEnd::TargetGone => {
                if (self.until.is_none() && self.win.is_none() && self.until_drop.is_none())
                    || self.should_stop(cx)
                {
                    return self.finish(cx);
                }
                if !self.reserve_nonkill_reengagement() {
                    return self.reengagement_budget_failure();
                }
                if let Some(stand) = self.request.stand {
                    self.return_retry.reset();
                    match self.begin_return_to_stand(stand, cx) {
                        Ok(()) => Poll::Pending,
                        Err(error) => Poll::Ready(Err(error)),
                    }
                } else {
                    self.rebegin_combat(cx, true)
                }
            }
            CombatEnd::NoTarget | CombatEnd::Budget => {
                if (self.until.is_none() && self.win.is_none() && self.until_drop.is_none())
                    || self.should_stop(cx)
                {
                    self.finish(cx)
                } else {
                    self.rebegin_after_nonkill(cx, false)
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
            CombatEnd::Aborted(_) => {
                self.abort_escape = AbortEscape::new();
                self.begin_abort_safety_walk(report, cx)
            }
        }
    }

    fn outcome_for(report: CombatReport, target_gone_restarts: u8, kills: u32) -> StepOutcome {
        StepOutcome {
            progress: None,
            evidence: report.evidence,
            receipt: Some(Arc::new(CombatReceipt {
                report,
                target_gone_restarts,
                kills,
            })),
        }
    }

    fn refresh_outcome(&mut self) {
        let Some(report) = self.last_report else {
            self.last_outcome = None;
            return;
        };
        self.last_outcome = Some(Self::outcome_for(
            report,
            self.target_gone_restarts,
            self.kills,
        ));
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

    fn reserve_nonkill_reengagement(&mut self) -> bool {
        if self.nonkill_reengagements >= self.reengage_budget {
            return false;
        }
        self.nonkill_reengagements += 1;
        true
    }

    fn rebegin_after_nonkill(
        &mut self,
        cx: &mut StepContext<'_, '_>,
        target_gone: bool,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        if !self.reserve_nonkill_reengagement() {
            return self.reengagement_budget_failure();
        }
        self.rebegin_combat(cx, target_gone)
    }

    fn reengagement_budget_failure(&self) -> Poll<Result<StepOutcome, ActionError>> {
        let detail = self.last_report.as_ref().map_or_else(
            || "combat re-engagement budget exhausted without a report".to_owned(),
            |report| format!("combat re-engagement budget exhausted; last report: {report:?}"),
        );
        Poll::Ready(Err(ActionError::Failed(Arc::from(detail))))
    }

    fn begin_loot(&mut self, cx: &mut StepContext<'_, '_>) -> Result<LootStart, ActionError> {
        if self.loot.is_empty() {
            return Ok(LootStart::Complete);
        }
        let snapshot = cx.tick.cx.snapshot();
        let tick = cx.tick.cx.evidence().tick;
        let wait_expired = self
            .loot_wait_deadline
            .is_some_and(|deadline| tick > deadline);
        let Some(inventory) = snapshot.inventory() else {
            return if wait_expired {
                Err(ActionError::NeedsEvidence(Arc::from([])))
            } else {
                Ok(LootStart::Waiting)
            };
        };
        let radius = reach::ground_area(self.request.stand, i32::from(self.request.lost_radius))
            .radius
            .max(reach::PROBE_RADIUS);
        let area = reach::Area::new(self.request.stand, radius);
        while let Some(item) = self.loot.get(self.loot_index) {
            let held = inventory
                .value
                .iter()
                .filter(|row| row.def.id == item.id)
                .map(|row| i64::from(row.count))
                .sum::<i64>();
            if held >= i64::from(item.qty) {
                self.loot_index += 1;
                self.loot_retake_tick = None;
                continue;
            }
            let target_tile = match self.ground_loot_tile(cx, item, area) {
                Some(Some(tile)) => tile,
                Some(None) if wait_expired => {
                    self.loot_missing_this_kill = true;
                    self.loot_index += 1;
                    self.loot_retake_tick = None;
                    continue;
                }
                Some(None) => return Ok(LootStart::Waiting),
                None if wait_expired => {
                    return Err(ActionError::NeedsEvidence(Arc::from([])));
                }
                None => return Ok(LootStart::Waiting),
            };
            let handle = cx.tick.actions.begin::<Reach>(
                ReachArgs {
                    kind: ReachKind::Ground {
                        id: item.id,
                        obj: Arc::clone(&item.name),
                    },
                    op: Arc::from("Take"),
                    anchor: self.request.stand,
                    radius,
                    wait_if_missing: false,
                    target_tile: Some(target_tile),
                    reachable_only: false,
                },
                &mut cx.tick.cx,
            )?;
            self.action = Some(Action::Loot(handle));
            return Ok(LootStart::Started);
        }
        Ok(LootStart::Complete)
    }

    fn ground_loot_tile(
        &self,
        cx: &StepContext<'_, '_>,
        item: &LootItem,
        area: reach::Area,
    ) -> Option<Option<api::WorldTile>> {
        cx.tick.cx.snapshot().ground_items()?;
        Some(
            reach::choose_ground(&cx.tick.cx, item.id, None, area, &reach::Avoid::default())
                .any()
                .map(|choice| choice.tile),
        )
    }

    fn enter_finish_wait(&mut self, tick: u64) {
        self.phase = Phase::FinishWait;
        self.finish_ticks_elapsed = 0;
        self.finish_last_tick = Some(tick);
        self.last_report = None;
        self.last_outcome = None;
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
            Ok(LootStart::Complete) => self.after_loot_complete(cx),
            Err(error) => Poll::Ready(Err(error)),
        }
    }

    /// Decide whether a settled loot pass completed the step or may re-engage.
    fn after_loot_complete(
        &mut self,
        cx: &mut StepContext<'_, '_>,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        let should_stop = self.should_stop(cx);
        let missing_required_loot = !should_stop
            && self.until_drop.is_none()
            && self.loot_missing_this_kill
            && self
                .until
                .as_ref()
                .is_some_and(|predicate| self.evaluate_predicate(predicate, cx) == Truth::False);
        self.loot_wait_deadline = None;
        self.loot_missing_this_kill = false;
        if should_stop {
            return self.finish(cx);
        }
        if let Some(until_drop) = &self.until_drop {
            if self.kills >= until_drop.max_kills {
                return Poll::Ready(Err(ActionError::Blocked(Arc::clone(&until_drop.stop))));
            }
            return self.rebegin_combat(cx, false);
        }
        if missing_required_loot {
            if self.kills >= DROP_LOOP_KILL_BUDGET {
                return self.drop_loop_budget_failure();
            }
            return self.rebegin_combat(cx, false);
        }
        if self.until.is_none() && self.win.is_none() {
            self.finish(cx)
        } else {
            self.rebegin_combat(cx, false)
        }
    }

    fn drop_loop_budget_failure(&self) -> Poll<Result<StepOutcome, ActionError>> {
        let detail = self.last_report.as_ref().map_or_else(
            || {
                format!(
                    "required combat loot was not acquired within {DROP_LOOP_KILL_BUDGET} kills"
                )
            },
            |report| {
                format!(
                    "required combat loot was not acquired within {DROP_LOOP_KILL_BUDGET} kills; last report: {report:?}"
                )
            },
        );
        Poll::Ready(Err(ActionError::Failed(Arc::from(detail))))
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

    fn until_drop_item_held(&self, cx: &StepContext<'_, '_>) -> bool {
        let Some(required) = self.until_drop.as_ref() else {
            return false;
        };
        cx.tick.cx.snapshot().inventory().is_some_and(|inventory| {
            inventory
                .value
                .iter()
                .filter(|item| item.def.id == required.item_id)
                .map(|item| i64::from(item.count))
                .sum::<i64>()
                > 0
        })
    }

    fn should_stop(&self, cx: &StepContext<'_, '_>) -> bool {
        self.until_drop_item_held(cx)
            || self
                .until
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
        self.observe_stand_walks();
        if self.manual_takeover_watch.interrupted(&cx.tick.cx) {
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
            self.enter_finish_wait(cx.tick.cx.evidence().tick);
            return Poll::Pending;
        }
        if matches!(self.phase, Phase::Loot) && self.action.is_none() {
            if self
                .loot_retake_tick
                .is_some_and(|failed_at| cx.tick.cx.evidence().tick <= failed_at)
            {
                return Poll::Pending;
            }
            return self.next_loot_or_reengage(cx);
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
        if matches!(self.phase, Phase::ReturningToStand) && self.return_retry.pending() {
            let tick = cx.tick.cx.evidence().tick;
            if self.return_retry.waiting_for_next_tick(tick) {
                return Poll::Pending;
            }
            let Some(stand) = self.request.stand else {
                return Poll::Ready(Err(ActionError::Stale));
            };
            match crate::native::defer_budget(self.begin_return_to_stand(stand, cx)) {
                Poll::Pending => {
                    self.return_retry.defer_until_next_tick(tick);
                    return Poll::Pending;
                }
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(())) => return Poll::Pending,
            }
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
            ActionPoll::Pending => self.expire_escape_walk(cx),
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
                // An unrouteable loot row counts as missed when this step still
                // needs loot; optional drops remain non-blocking.
                self.action = None;
                self.loot_index += 1;
                self.loot_retake_tick = None;
                self.loot_missing_this_kill = true;
                self.next_loot_or_reengage(cx)
            }
            ActionPoll::Failed(ActionError::Stale) if matches!(&self.phase, Phase::Loot) => {
                // A rebuilt scene cancels the queued ground Take before the
                // item op runs, but the item stays on the floor: a rebuild
                // only rewrites the player's build area (`BuildArea.ts:57-91`),
                // the op removes the object when it takes it
                // (`ObjOps.ts:137-164`, `pickup.rs2:15`), and a click inside
                // origin ± 52 is still accepted (`OpObjHandler.ts:19-28`).
                // Re-address the same item once, on the next observed tick so
                // the new Take binds the new scene; a second cancel for that
                // item is missed loot like an unreachable drop.
                self.action = None;
                // The current row remains owned until its Take settles.
                // Its first failure tick also marks its one retake as spent.
                if self.loot_retake_tick.is_some() {
                    self.loot_retake_tick = None;
                    self.loot_index += 1;
                    self.loot_missing_this_kill = true;
                    self.next_loot_or_reengage(cx)
                } else {
                    self.loot_retake_tick = Some(cx.tick.cx.evidence().tick);
                    Poll::Pending
                }
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
                    AbortHoldOutcome::ThreatFree if self.abort_hands_back_to_caller() => {
                        self.finish(cx)
                    }
                    AbortHoldOutcome::ThreatFree => {
                        Poll::Ready(Err(ActionError::Blocked(Arc::clone(&ABORT_HOLD_SAFE))))
                    }
                    AbortHoldOutcome::SuppliesExhausted => {
                        // Re-plan from the observed position, but keep every
                        // earlier failed request's goal tiles excluded.
                        self.abort_escape.begin_pass();
                        self.begin_abort_escape_walk(cx)
                    }
                }
            }
            ActionPoll::Combat(report) => self.on_combat_report(report, cx),
            ActionPoll::Walk(receipt) if matches!(&self.phase, Phase::WalkingOutAfterAbort) => {
                self.on_abort_walk(receipt, cx)
            }
            ActionPoll::Walk(receipt) if matches!(&self.phase, Phase::EscapingAfterAbortHold) => {
                self.on_abort_escape_walk(receipt, cx)
            }
            ActionPoll::Walk(receipt) => self.on_return_walk(receipt, cx),
            ActionPoll::Loot(taken) => {
                self.action = None;
                self.loot_index += 1;
                self.loot_retake_tick = None;
                if !taken {
                    // A stale or absent drop is not evidence for another click.
                    // The next combat run can produce a fresh ground row.
                    self.loot_missing_this_kill = true;
                }
                self.next_loot_or_reengage(cx)
            }
            ActionPoll::Dialogue(DialogueOutcome::Completed) => self.finish_completed(cx),
            ActionPoll::Dialogue(DialogueOutcome::Failed) => {
                self.action = None;
                Poll::Ready(Err(ActionError::Blocked(Arc::clone(&COMBAT_FINISH_FAILED))))
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
