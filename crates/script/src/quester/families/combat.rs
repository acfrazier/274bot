//! Shared `script::combat` adapter for typed Quester Path combat steps.
use super::super::compile::{
    CompileContext, CompileError, FamilyReceipt, PredicateContext, PredicatePlan, StepContext,
    StepOutcome, StepPlan, StepRun,
};
use super::super::path::PredicateDocument;
use super::reach::{self, Reach, ReachArgs, ReachKind};
use crate::combat::{
    AbortReason, Allowances, Combat, CombatEnd, CombatReport, CombatRequest, CombatTables,
    CompiledKit, Fallback, IntruderPolicy, MeleeMode, Pick, PrayerMode, RaisedPrayers, SpellRef,
    Style, Tactic, Target,
};
use crate::loadouts_store::WORN_SLOTS;
use crate::native::walk::Walk;
use crate::native::{ActionError, ActionHandle, NativeActions, WalkReceipt};
use api::gather_methods::SceneRegionInput;
use api::selected::Truth;
use serde::Deserialize;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

static ABORTED_COMBAT_STEP: std::sync::LazyLock<Arc<str>> =
    std::sync::LazyLock::new(|| Arc::from("combat aborted; caller must handle the failure"));
static ABORT_WALK_FAILED: std::sync::LazyLock<Arc<str>> =
    std::sync::LazyLock::new(|| Arc::from("combat abort walk did not reach a safe tile"));

fn tile_distance(a: api::WorldTile, b: api::WorldTile) -> i32 {
    if a.level != b.level {
        i32::MAX
    } else {
        (a.x - b.x).abs().max((a.z - b.z).abs())
    }
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
    /// Optional named or inline search area.
    #[serde(default)]
    area: Option<AreaArg>,
    /// Maximum distance in tiles before the target is considered lost.
    lost_radius: u8,
    /// Maximum game ticks to spend on a combat attempt.
    kill_budget_ticks: u16,
    /// Item config names to loot after a kill.
    #[serde(default)]
    loot: Vec<String>,
    /// Optional predicate that ends the combat loop when true.
    #[serde(default)]
    until: Option<PredicateDocument>,
    /// Optional success predicate that ends the combat loop when true.
    #[serde(default)]
    win: Option<PredicateDocument>,
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
    /// Attack style; only `melee` is currently supported.
    style: String,
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
    let style = match args.tactic.style.as_str() {
        "melee" => Style::Melee,
        "mage" => Style::Mage,
        _ => return Err(CompileError::code("unsupported-combat-style")),
    };
    if style != Style::Mage
        && args
            .spells
            .as_ref()
            .is_some_and(|spells| !spells.is_empty())
    {
        return Err(CompileError::code("unsupported-combat-spells"));
    }
    let target = compile_target(args.target, cx)?;
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
        until_ticks: 0,
    };

    Ok(CombatPlan {
        request: Arc::new(request),
        tables,
        until,
        win,
        loot: Arc::from(loot),
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

fn compile_loot(loot: &[String], cx: &CompileContext<'_>) -> Result<Vec<LootItem>, CompileError> {
    let mut rows = Vec::with_capacity(loot.len());
    for name in loot {
        let (id, display) = resolve_loadout_item(cx, name)?;
        if rows.iter().any(|row: &LootItem| row.id == id) {
            return Err(CompileError::code("duplicate-combat-loot-item"));
        }
        rows.push(LootItem { id, name: display });
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

struct LootItem {
    id: i32,
    name: Arc<str>,
}

struct CombatPlan {
    request: Arc<CombatRequest>,
    tables: Arc<CombatTables>,
    until: Option<Arc<dyn PredicatePlan>>,
    win: Option<Arc<dyn PredicatePlan>>,
    loot: Arc<[LootItem]>,
}

impl StepPlan for CombatPlan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        let mut run = CombatRun {
            request: Arc::clone(&self.request),
            tables: Arc::clone(&self.tables),
            until: self.until.as_ref().map(Arc::clone),
            win: self.win.as_ref().map(Arc::clone),
            loot: Arc::clone(&self.loot),
            action: None,
            phase: Phase::Combat,
            loot_index: 0,
            last_report: None,
            last_outcome: None,
            target_gone_restarts: 0,
            raised_prayers: RaisedPrayers::empty(),
            walk_outcome_seq_at_begin: cx.tick.cx.observed_walk_outcome_seq,
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
    Loot,
}

#[allow(
    clippy::large_enum_variant,
    reason = "Keep native machines inline instead of allocating on every combat/loot transition."
)]
enum Action {
    Combat(ActionHandle<Combat>),
    Walk(ActionHandle<Walk>),
    Loot(ActionHandle<Reach>),
}

enum ActionPoll {
    Pending,
    Failed(ActionError),
    Combat(CombatReport),
    Walk(WalkReceipt),
    Loot(bool),
}

enum LootStart {
    Waiting,
    Started,
    Complete,
}

struct CombatRun {
    request: Arc<CombatRequest>,
    tables: Arc<CombatTables>,
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
        if let Some(Action::Combat(handle)) = self.action.as_ref() {
            self.raised_prayers = handle.prayer_cleanup();
        }
    }

    fn prayer_cleanup(&self) -> RaisedPrayers {
        match self.action.as_ref() {
            Some(Action::Combat(handle)) => handle.prayer_cleanup(),
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
        let handle = cx.tick.actions.begin::<Walk>(
            reach::walk_request(stand, 1, None, cx.required_after),
            &mut cx.tick.cx,
        )?;
        self.phase = Phase::ReturningToStand;
        self.action = Some(Action::Walk(handle));
        Ok(())
    }

    fn abort_walk_destination(
        &self,
        report: CombatReport,
        cx: &StepContext<'_, '_>,
    ) -> Option<api::WorldTile> {
        let snapshot = cx.tick.cx.snapshot();
        let here = snapshot.here()?.value;
        let target = report.engaged.and_then(|engaged| match engaged.kind {
            api::snapshot::ActorKind::Npc => {
                let index = usize::from(engaged.index);
                let kind = usize::try_from(report.engaged_npc_type).ok()?;
                snapshot
                    .npcs()?
                    .value
                    .iter()
                    .find(|npc| npc.index == index && npc.r#type == Some(kind))
                    .map(|npc| npc.tile)
            }
            api::snapshot::ActorKind::Player => snapshot
                .players()?
                .value
                .iter()
                .find(|player| player.index == usize::from(engaged.index))
                .map(|player| player.actor.tile),
        });
        let Some(target) = target else {
            return self.request.stand;
        };
        // The engine drops a melee NPC's target only once the player is more
        // than `maxrange + 1` from the NPC's spawn, which the client never
        // sees, and the NPC itself may be up to `maxrange` from that spawn.
        // Measured from the NPC's current tile, the chase envelope is
        // therefore up to `2 * maxrange + attackrange` (Npc.ts:652-667).
        let chase_range = if report
            .engaged
            .is_some_and(|actor| actor.kind == api::snapshot::ActorKind::Npc)
        {
            self.tables
                .selected()
                .npc_name(report.engaged_npc_type)
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

    fn begin_abort_walk(
        &mut self,
        report: CombatReport,
        cx: &mut StepContext<'_, '_>,
    ) -> Result<(), ActionError> {
        let destination = self
            .abort_walk_destination(report, cx)
            .ok_or_else(|| ActionError::Blocked(Arc::clone(&ABORT_WALK_FAILED)))?;
        let handle = cx.tick.actions.begin::<Walk>(
            reach::walk_request(destination, 1, None, cx.required_after),
            &mut cx.tick.cx,
        )?;
        self.phase = Phase::WalkingOutAfterAbort;
        self.action = Some(Action::Walk(handle));
        Ok(())
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

    fn on_abort_walk(&mut self, receipt: WalkReceipt) -> Poll<Result<StepOutcome, ActionError>> {
        self.action = None;
        let error = match receipt.into_arrival() {
            Ok(_) => ActionError::Blocked(Arc::clone(&ABORTED_COMBAT_STEP)),
            Err(error) => error,
        };
        Poll::Ready(Err(error))
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
                if matches!(&self.request.target, Target::Attacker { .. }) =>
            {
                // M4's caller owns the walk-out after this observed report
                // (design-combat-s3.md §5.1).
                self.finish(cx)
            }
            CombatEnd::Aborted(_) => match self.begin_abort_walk(report, cx) {
                Ok(()) => Poll::Pending,
                Err(
                    error @ (ActionError::UserInput
                    | ActionError::Cancelled
                    | ActionError::NeedsEvidence(_)),
                ) => Poll::Ready(Err(error)),
                Err(_) => Poll::Ready(Err(ActionError::Blocked(Arc::clone(&ABORT_WALK_FAILED)))),
            },
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
            self.loot_index += 1;
            let held = inventory
                .value
                .iter()
                .filter(|row| row.def.id == item.id)
                .map(|row| row.count)
                .sum::<i32>();
            if held > 0 {
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
                },
                &mut cx.tick.cx,
            )?;
            self.action = Some(Action::Loot(handle));
            return Ok(LootStart::Started);
        }
        Ok(LootStart::Complete)
    }

    fn next_loot_or_reengage(
        &mut self,
        cx: &mut StepContext<'_, '_>,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        match self.begin_loot(cx) {
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
            quests: cx.quests,
            progress: cx.progress,
            required_after: cx.required_after,
            chat_since,
            outcome: self.last_outcome.as_ref(),
            bank: cx.bank,
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
        if matches!(self.phase, Phase::Loot) && self.action.is_none() {
            match self.begin_loot(cx) {
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
        if !matches!(self.phase, Phase::Loot) && self.should_stop(cx) {
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
            Some(Action::Loot(handle)) => match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => ActionPoll::Pending,
                Poll::Ready(Ok(taken)) => ActionPoll::Loot(taken),
                Poll::Ready(Err(error)) => ActionPoll::Failed(error),
            },
            None => ActionPoll::Pending,
        };
        match polled {
            ActionPoll::Pending => Poll::Pending,
            ActionPoll::Failed(error) if matches!(&self.phase, Phase::WalkingOutAfterAbort) => {
                if matches!(
                    error,
                    ActionError::UserInput | ActionError::Cancelled | ActionError::NeedsEvidence(_)
                ) {
                    Poll::Ready(Err(error))
                } else {
                    Poll::Ready(Err(ActionError::Blocked(Arc::clone(&ABORT_WALK_FAILED))))
                }
            }
            ActionPoll::Failed(ActionError::Blocked(_)) if matches!(&self.phase, Phase::Loot) => {
                // Loot is optional: an unreachable ground item does not end
                // the combat step. Other errors remain visible to the caller.
                self.action = None;
                self.next_loot_or_reengage(cx)
            }
            ActionPoll::Failed(error) => {
                self.action = None;
                Poll::Ready(Err(error))
            }
            ActionPoll::Combat(report) => self.on_combat_report(report, cx),
            ActionPoll::Walk(receipt) if matches!(&self.phase, Phase::WalkingOutAfterAbort) => {
                self.on_abort_walk(receipt)
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
        }
    }

    fn cancel(&mut self, _actions: &mut NativeActions) {
        self.capture_prayer_cleanup();
        self.action = None;
    }
    fn prayer_cleanup(&self) -> RaisedPrayers {
        CombatRun::prayer_cleanup(self)
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
