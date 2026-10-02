//! Shared `script::combat` adapter for typed Quester Path combat steps.
use super::super::compile::{
    CompileContext, CompileError, FamilyReceipt, PredicateContext, PredicatePlan, StepContext,
    StepOutcome, StepPlan, StepRun,
};
use super::super::path::PredicateDocument;
use super::reach::{self, Reach, ReachArgs, ReachKind};
use crate::combat::{
    AbortReason, Allowances, Combat, CombatEnd, CombatReport, CombatRequest, CombatTables,
    CompiledKit, Fallback, IntruderPolicy, MeleeMode, Pick, PrayerMode, Style, Tactic, Target,
};
use crate::loadouts_store::WORN_SLOTS;
use crate::native::walk::Walk;
use crate::native::{ActionError, ActionHandle, NativeActions, WalkEnd, WalkReceipt};
use api::gather_methods::SceneRegionInput;
use api::selected::Truth;
use serde::Deserialize;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

static ABORTED_COMBAT_STEP: std::sync::LazyLock<Arc<str>> =
    std::sync::LazyLock::new(|| Arc::from("combat aborted; caller must handle the failure"));
static RETURN_TO_STAND_FAILED: std::sync::LazyLock<Arc<str>> =
    std::sync::LazyLock::new(|| Arc::from("combat target-loss walk did not reach the stand"));

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
#[serde(deny_unknown_fields)]
struct CombatArgs {
    target: TargetArgs,
    tactic: TacticArgs,
    #[serde(default)]
    melee_mode: Option<MeleeMode>,
    #[serde(default)]
    loadout: Option<String>,
    #[serde(default)]
    spells: Option<Vec<String>>,
    #[serde(default, alias = "anchor")]
    stand: Option<super::AnchorArg>,
    #[serde(default)]
    area: Option<AreaArg>,
    lost_radius: u8,
    kill_budget_ticks: u16,
    #[serde(default)]
    loot: Vec<String>,
    #[serde(default)]
    until: Option<PredicateDocument>,
    #[serde(default)]
    win: Option<PredicateDocument>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetArgs {
    #[serde(default)]
    npc: Option<NpcArg>,
    #[serde(default)]
    attacker: Option<AttackerArgs>,
    #[serde(default)]
    player: Option<serde_json::Value>,
    #[serde(default)]
    not_targeting_others: Option<bool>,
    #[serde(default)]
    pick: Option<PickArg>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum NpcArg {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AttackerArgs {
    npcs: bool,
    players: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum PickArg {
    Nearest,
    Random,
    LowestHealth,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TacticArgs {
    kind: String,
    style: String,
    engage_radius: u8,
    #[serde(default = "default_auto_retaliate")]
    auto_retaliate: bool,
    #[serde(default)]
    fallback: Option<String>,
}

fn default_auto_retaliate() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(untagged)]
enum AreaArg {
    Named(String),
    Inline(InlineArea),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InlineArea {
    #[serde(default, rename = "box")]
    one: Option<[i32; 5]>,
    #[serde(default)]
    boxes: Option<Vec<[i32; 5]>>,
    source: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CombatEndArgs {
    end: CombatEndName,
}

#[derive(Deserialize)]
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
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    let args: CombatArgs =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
    if args
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
    if args.tactic.style != "melee" {
        return Err(CompileError::code("unsupported-combat-style"));
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

    let request = CombatRequest {
        target,
        tactic: Tactic::Open,
        style: Style::Melee,
        melee_mode: args.melee_mode,
        kit,
        spells: None,
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

    Ok(Arc::new(CombatPlan {
        request: Arc::new(request),
        tables,
        until,
        win,
        loot: Arc::from(loot),
    }))
}

pub(super) fn compile_end_predicate(
    args: &serde_json::Value,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let args: CombatEndArgs =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
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
    if let Some(item) = cx.selected.item_by_alias(name) {
        let display = item.name.as_deref().unwrap_or(name);
        return Ok((item.id, Arc::from(display)));
    }
    let item = cx
        .selected
        .items()
        .iter()
        .find(|item| {
            item.name
                .as_deref()
                .is_some_and(|known| known.eq_ignore_ascii_case(name))
        })
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
}

impl CombatRun {
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
            reach::walk_request(stand, 1, cx.required_after),
            &mut cx.tick.cx,
        )?;
        self.phase = Phase::ReturningToStand;
        self.action = Some(Action::Walk(handle));
        Ok(())
    }

    fn on_return_walk(
        &mut self,
        receipt: WalkReceipt,
        cx: &mut StepContext<'_, '_>,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        self.action = None;
        if !matches!(receipt.end, WalkEnd::Arrived) {
            return Poll::Ready(Err(ActionError::Blocked(Arc::clone(
                &RETURN_TO_STAND_FAILED,
            ))));
        }
        self.rebegin_combat(cx, true)
    }

    fn on_combat_report(
        &mut self,
        report: CombatReport,
        cx: &mut StepContext<'_, '_>,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        self.action = None;
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
            CombatEnd::Aborted(_) => {
                // Abort ends the boss step; the caller decides what follows
                // (design-combat.md §2.2, §2.6).
                Poll::Ready(Err(ActionError::Blocked(Arc::clone(&ABORTED_COMBAT_STEP))))
            }
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
            ActionPoll::Failed(error) => Poll::Ready(Err(error)),
            ActionPoll::Combat(report) => self.on_combat_report(report, cx),
            ActionPoll::Walk(receipt) => self.on_return_walk(receipt, cx),
            ActionPoll::Loot(taken) => {
                self.action = None;
                if !taken {
                    // A stale or absent drop is not evidence for another click.
                    // The next combat run can produce a fresh ground row.
                }
                match self.begin_loot(cx) {
                    Ok(LootStart::Waiting | LootStart::Started) => Poll::Pending,
                    Ok(LootStart::Complete) => {
                        if self.until.is_none() && self.win.is_none() || self.should_stop(cx) {
                            self.finish(cx)
                        } else {
                            self.rebegin_combat(cx, false)
                        }
                    }
                    Err(error) => Poll::Ready(Err(error)),
                }
            }
        }
    }

    fn cancel(&mut self, _actions: &mut NativeActions) {
        self.action = None;
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
mod tests;
