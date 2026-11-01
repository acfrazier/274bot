//! Bank, shop, production, equipment and loadout compiled families.
use super::{begin_approach_walk, reach, walk_step_evidence, NoArgs};
use crate::acquire::run::AcquireRun;
use crate::acquire::{AcquireOutcome, AcquirePlan, AcquireRequest, OpenGate};
use crate::combat::RaisedPrayers;
use crate::native::{ActionContext, ActionError, ActionHandle, NativeActions};
use crate::native_bank::{BankAction, BankItem, BankReceipt};
use crate::native_equipment::{EquipmentMachine, EquipmentRequest};
use crate::native_production::{MakeMachine, MakeRequest};
use crate::native_shop::{BuyMachine, BuyRequest};
use crate::quester::compile::{
    CompileContext, CompileError, CompiledCarry, FamilyReceipt, PredicateContext, PredicatePlan,
    StepContext, StepOutcome, StepPlan, StepRun,
};
use crate::quester::prereq::{reach::PrereqReach, walk::PrereqWalk};
use api::bank_memory::Origin;
use api::quest_progress::{EvidenceStamp, QuestProgress};
use api::selected::{FactKey, Knowledge, Truth};
use api::snapshot::WorldTile;
use serde::Deserialize;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

const LOADOUT_OBSERVATION_TIMEOUT: Duration = Duration::from_secs(30);

fn loadout_observation_wait_detail() -> &'static Arc<str> {
    static DETAIL: std::sync::LazyLock<Arc<str>> =
        std::sync::LazyLock::new(|| Arc::from("Waiting for inventory/equipment observation"));
    &DETAIL
}

impl FamilyReceipt for BankReceipt {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn item(cx: &CompileContext<'_>, alias: &str) -> Result<BankItem, CompileError> {
    let item = cx
        .selected
        .item_by_alias(alias)
        .ok_or_else(|| CompileError::code("unresolved-obj").with_detail(alias))?;
    let name = item
        .name
        .as_deref()
        .ok_or_else(|| CompileError::code("unresolved-obj-name"))?;
    Ok(BankItem {
        id: item.id,
        name: Arc::from(name),
    })
}

#[derive(Debug)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "path-schema", serde(untagged))]
pub(super) enum QuantityDocument {
    /// Fixed item quantity; values below 1 are rejected by the compiler.
    Fixed(#[cfg_attr(feature = "path-schema", schemars(range(min = 1)))] i32),
    /// Count of a declared journal flag, optionally reduced by held items.
    Progress(ProgressQuantityDocument),
}

impl<'de> serde::Deserialize<'de> for QuantityDocument {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct QuantityVisitor;

        impl<'de> serde::de::Visitor<'de> for QuantityVisitor {
            type Value = QuantityDocument;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("an integer quantity or a progress quantity object")
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                i32::try_from(value)
                    .map(QuantityDocument::Fixed)
                    .map_err(E::custom)
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                i32::try_from(value)
                    .map(QuantityDocument::Fixed)
                    .map_err(E::custom)
            }

            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                map: M,
            ) -> Result<Self::Value, M::Error> {
                ProgressQuantityDocument::deserialize(serde::de::value::MapAccessDeserializer::new(
                    map,
                ))
                .map(QuantityDocument::Progress)
            }
        }

        deserializer.deserialize_any(QuantityVisitor)
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct ProgressQuantityDocument {
    /// Journal flag which supplies the current item count.
    progress: ProgressCountDocument,
    /// Optional held item subtracted from the journal count.
    #[serde(default)]
    minus_item: Option<String>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct ProgressCountDocument {
    /// Symbolic quest key whose journal is read.
    quest: String,
    /// Declared counted flag in that journal.
    flag: String,
}

#[derive(Debug)]
pub(super) enum QuantityPlan {
    Fixed(i32),
    Progress {
        quest: FactKey,
        flag: FactKey,
        minus_item: Option<i32>,
    },
}

impl QuantityPlan {
    fn fixed(&self) -> Option<i32> {
        match self {
            Self::Fixed(qty) => Some(*qty),
            Self::Progress { .. } => None,
        }
    }

    pub(super) fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Option<i32> {
        self.evaluate_parts(cx.cx, cx.progress, cx.required_after)
    }

    pub(super) fn evaluate_step(&self, cx: &StepContext<'_, '_>) -> Option<i32> {
        self.evaluate_parts(&cx.tick.cx, cx.progress, cx.required_after)
    }

    fn evaluate_parts(
        &self,
        cx: &ActionContext<'_>,
        progress: &[QuestProgress],
        required_after: EvidenceStamp,
    ) -> Option<i32> {
        let Self::Progress {
            quest,
            flag: flag_key,
            minus_item,
        } = self
        else {
            return self.fixed();
        };

        let progress = progress.iter().find(|progress| {
            progress.quest == *quest && progress.evidence.run == required_after.run
        })?;
        if !matches!(&progress.stage, Knowledge::Known(_)) {
            return None;
        }
        let flag = progress.flags.iter().find(|flag| flag.flag == *flag_key)?;
        if flag.truth != Truth::True {
            return None;
        }
        let mut qty = i64::from(flag.count?);
        if let Some(id) = minus_item {
            let snapshot = cx.snapshot();
            let inventory = snapshot.inventory()?;
            let held = inventory
                .value
                .iter()
                .filter(|item| item.def.id == *id)
                .map(|item| i64::from(item.count.max(0)))
                .sum::<i64>();
            qty = qty.saturating_sub(held).max(0);
        }
        Some(qty.min(i64::from(i32::MAX)) as i32)
    }
}

pub(super) fn compile_quantity(
    document: QuantityDocument,
    cx: &CompileContext<'_>,
) -> Result<QuantityPlan, CompileError> {
    match document {
        QuantityDocument::Fixed(qty) => Ok(QuantityPlan::Fixed(qty)),
        QuantityDocument::Progress(document) => {
            if document.progress.quest.is_empty() || document.progress.flag.is_empty() {
                return Err(CompileError::code("invalid-quantity"));
            }
            cx.quests
                .quest(&document.progress.quest)
                .map_err(|_| CompileError::code("unresolved-quest"))?;
            if cx.path.0.as_ref() != document.progress.quest {
                return Err(CompileError::code("foreign-progress-quest"));
            }
            let flag = FactKey::new(&document.progress.flag);
            if !cx
                .progress
                .flags
                .iter()
                .any(|candidate| candidate.flag == flag && candidate.count.is_some())
            {
                return Err(CompileError::code("unresolved-progress-count"));
            }
            let minus_item = document
                .minus_item
                .as_deref()
                .map(|alias| item(cx, alias).map(|item| item.id))
                .transpose()?;
            Ok(QuantityPlan::Progress {
                quest: FactKey::new(&document.progress.quest),
                flag,
                minus_item,
            })
        }
    }
}

fn named_item(cx: &CompileContext<'_>, name: &str) -> Result<BankItem, CompileError> {
    named_item_with_stackable(cx, name).map(|(item, _)| item)
}

fn named_item_with_stackable(
    cx: &CompileContext<'_>,
    name: &str,
) -> Result<(BankItem, bool), CompileError> {
    let item = cx
        .selected
        .resolve_item_name(name)
        .filter(|item| !item.is_certificate())
        .ok_or_else(|| CompileError::code("unresolved-loadout-item"))?;
    let display = item
        .name
        .as_deref()
        .ok_or_else(|| CompileError::code("unresolved-loadout-item"))?;
    Ok((
        BankItem {
            id: item.id,
            name: Arc::from(display),
        },
        item.stackable,
    ))
}

fn anchor(tile: [i32; 3]) -> WorldTile {
    WorldTile {
        x: tile[0],
        z: tile[1],
        level: tile[2],
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct ItemQty {
    /// Symbolic item config name.
    obj: String,
    /// Held target for `withdraw`: the final backpack count, not a count to
    /// take, so items already held are not withdrawn again and a backpack at
    /// or above it completes without a withdrawal. Ignored by `deposit`, which
    /// deposits every held row of the item. Omitted uses 1 and values below 1
    /// are rejected.
    #[serde(default = "one")]
    #[cfg_attr(feature = "path-schema", schemars(range(min = 1)))]
    qty: i32,
}
fn one() -> i32 {
    1
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
enum BankOp {
    Scan,
    Withdraw,
    Deposit,
    DepositAll,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct BankArgs {
    /// Bank action to perform.
    op: BankOp,
    /// Bank selector: `quest_bank` or `nearest`; omitted uses the Path bank.
    #[serde(default)]
    at: Option<String>,
    /// Item quantities for withdraw/deposit operations.
    #[serde(default)]
    items: Vec<ItemQty>,
    /// Symbolic items to retain on `deposit_all`.
    #[serde(default)]
    keep: Vec<String>,
    /// Raw item ids to retain on `deposit_all`.
    #[serde(default)]
    keep_ids: Vec<i32>,
    /// Allow a partial result when requested bank operations cannot complete.
    #[serde(default)]
    partial_ok: bool,
}

pub(super) fn compile_bank(
    args: BankArgs,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    if args
        .at
        .as_deref()
        .is_some_and(|at| at != "quest_bank" && at != "nearest")
    {
        return Err(CompileError::code("invalid-bank-target"));
    }
    let mut actions = Vec::new();
    match args.op {
        BankOp::Scan => actions.push(BankAction::Scan),
        BankOp::Withdraw => {
            if args.items.is_empty() {
                return Err(CompileError::code("bank-items-empty"));
            }
            for row in args.items {
                if row.qty < 1 {
                    return Err(CompileError::code("invalid-bank-quantity"));
                }
                actions.push(BankAction::Withdraw {
                    item: item(cx, &row.obj)?,
                    qty: row.qty,
                });
            }
        }
        BankOp::Deposit => {
            if args.items.is_empty() {
                return Err(CompileError::code("bank-items-empty"));
            }
            for row in args.items {
                actions.push(BankAction::Deposit {
                    item: item(cx, &row.obj)?,
                });
            }
        }
        BankOp::DepositAll => {
            let mut keep = cx.keep_ids.to_vec();
            for id in args.keep_ids {
                if !keep.contains(&id) {
                    keep.push(id);
                }
            }
            for alias in args.keep {
                let id = item(cx, &alias)?.id;
                if !keep.contains(&id) {
                    keep.push(id);
                }
            }
            actions.push(BankAction::DepositAll {
                keep: Arc::from(keep),
            });
        }
    }
    Ok(Arc::new(BankPlan {
        bank: cx.bank,
        bank_required: cx.bank_required,
        actions: Arc::from(actions),
        partial_ok: args.partial_ok,
    }))
}

struct BankPlan {
    bank: Option<api::named_banks::NamedBank>,
    bank_required: bool,
    actions: Arc<[BankAction]>,
    partial_ok: bool,
}
impl StepPlan for BankPlan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        // An authored scan is a request to know the bank. A known memory —
        // `Hint` included (design-bank-snapshot §8 point 3) — already answers
        // it, so the step completes without a trip.
        if self
            .actions
            .iter()
            .all(|action| matches!(action, BankAction::Scan))
            && cx.tick.cx.snapshot().stock().banked_origin() != Origin::Unknown
        {
            return Ok(Box::new(KnownScan));
        }
        let request = AcquireRequest::bank_actions(
            self.bank,
            self.bank_required,
            Arc::clone(&self.actions),
            self.partial_ok,
            cx.required_after,
        );
        Ok(Box::new(BankStepRun {
            run: AcquireRun::new(request, AcquirePlan::authored(), cx.banks, &cx.tick.cx)
                .with_quester_walk_retries(),
        }))
    }
    fn settle_timeout(&self) -> Duration {
        Duration::from_secs(12)
    }
}

/// `bank {op: scan}` with a known memory: nothing to learn, no trip.
struct KnownScan;
impl StepRun for KnownScan {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        Poll::Ready(Ok(StepOutcome {
            progress: None,
            evidence: cx.tick.cx.evidence(),
            receipt: Some(Arc::new(BankReceipt {
                complete: true,
                deposited: 0,
            }) as Arc<dyn FamilyReceipt>),
        }))
    }
    fn cancel(&mut self, _actions: &mut NativeActions) {}
}

struct BankStepRun {
    run: AcquireRun,
}
impl StepRun for BankStepRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        match self.run.poll(cx.tick, OpenGate::Open) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Ready(Ok(AcquireOutcome::Done(receipt))) => {
                let bank = receipt.last_bank.unwrap_or(BankReceipt {
                    complete: receipt.complete,
                    deposited: receipt.deposited,
                });
                Poll::Ready(Ok(StepOutcome {
                    progress: None,
                    evidence: cx.tick.cx.evidence(),
                    receipt: Some(Arc::new(bank) as Arc<dyn FamilyReceipt>),
                }))
            }
            Poll::Ready(Ok(AcquireOutcome::ChildDue { .. })) => Poll::Ready(Err(
                ActionError::Unavailable(Arc::from("authored bank stage requested a child")),
            )),
        }
    }
    fn cancel(&mut self, actions: &mut NativeActions) {
        self.run.cancel(actions);
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct ShopArg {
    /// Symbolic shopkeeper NPC config name.
    npc: String,
    /// Authored shop approach anchor and its source citation.
    anchor: super::AnchorArg,
}
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct BuyArgs {
    /// Shopkeeper and approach location.
    shop: ShopArg,
    /// Symbolic item config name to buy.
    obj: String,
    /// Held target: the final backpack count, not a count to buy, so items
    /// already held are not bought again; must be at least 1.
    #[cfg_attr(feature = "path-schema", schemars(range(min = 1)))]
    qty: i32,
    /// Estimated coin budget; currently informational.
    #[serde(default)]
    est_gp: u32,
    /// Optional shop menu choice; currently informational.
    #[serde(default)]
    option: Option<String>,
}

pub(super) fn compile_buy(
    args: BuyArgs,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    if args.qty < 1 || args.shop.anchor.source.trim().is_empty() {
        return Err(CompileError::code("invalid-buy"));
    }
    let npc = cx
        .selected
        .npc_by_config(&args.shop.npc)
        .ok_or_else(|| CompileError::code("unresolved-npc").with_detail(args.shop.npc.as_str()))?;
    let item = item(cx, &args.obj)?;
    let _ = (args.est_gp, args.option);
    Ok(Arc::new(BuyPlan {
        tile: anchor(args.shop.anchor.tile),
        npc_id: npc.id,
        npc_name: Arc::from(npc.display.as_deref().unwrap_or(&args.shop.npc)),
        item,
        qty: args.qty,
    }))
}
struct BuyPlan {
    tile: WorldTile,
    npc_id: i32,
    npc_name: Arc<str>,
    item: BankItem,
    qty: i32,
}
impl StepPlan for BuyPlan {
    fn begin(&self, _cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        Ok(Box::new(BuyRun {
            tile: self.tile,
            request: BuyRequest {
                npc_id: self.npc_id,
                npc_name: Arc::clone(&self.npc_name),
                item_id: self.item.id,
                item_name: Arc::clone(&self.item.name),
                qty: self.qty,
            },
            walk: None,
            retry: crate::native::WalkRetryState::default(),
            buy: None,
        }))
    }
}
struct BuyRun {
    tile: WorldTile,
    request: BuyRequest,
    walk: Option<PrereqWalk>,
    retry: crate::native::WalkRetryState,
    buy: Option<ActionHandle<BuyMachine>>,
}
impl StepRun for BuyRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        let now = cx.tick.cx.evidence().tick;
        if let Some(walk) = self.walk.as_mut() {
            match walk.poll(cx.tick.actions, &mut cx.tick.cx, cx.tick.state) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(receipt)) => {
                    self.walk = None;
                    if self.retry.observe_approach_failure(&receipt, now) {
                        return Poll::Pending;
                    }
                    if let Err(error) = walk_step_evidence(receipt) {
                        return Poll::Ready(Err(error));
                    }
                    self.retry.reset();
                }
            }
        }
        if self.buy.is_none() {
            let near = cx
                .tick
                .cx
                .snapshot()
                .local_player()
                .is_some_and(|player| reach::within(player.value.player.network, self.tile, 6));
            if !near {
                if self.retry.waiting_for_next_tick(now) {
                    return Poll::Pending;
                }
                self.walk = Some(std::task::ready!(begin_approach_walk(
                    &mut self.retry,
                    reach::walk_request(self.tile, 4, None, cx.required_after),
                    cx,
                ))?);
                return Poll::Pending;
            }
            self.retry.reset();
            self.buy = Some(std::task::ready!(crate::native::defer_budget(
                cx.tick
                    .actions
                    .begin::<BuyMachine>(self.request.clone(), &mut cx.tick.cx)
            ))?);
            // BuyMachine emits on its first poll: poll it now (TICK-FIX #4,
            // C-BUY-PHASE).
            if !cx.tick.cx.may_continue_this_tick() {
                return Poll::Pending;
            }
        }
        match cx
            .tick
            .actions
            .poll(self.buy.as_ref().unwrap(), &mut cx.tick.cx)
        {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Ready(Ok(_)) => Poll::Ready(Ok(done(cx))),
        }
    }
    fn cancel(&mut self, _actions: &mut NativeActions) {
        if let Some(walk) = self.walk.as_mut() {
            walk.cancel();
        }
        self.walk = None;
        self.buy = None;
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct MakeLoc {
    /// Symbolic location config name.
    name: String,
    /// Optional loc operation. Omit it to use the menu item on this loc.
    #[serde(default)]
    op: Option<String>,
}
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct MakeMenu {
    /// Symbolic item config name shown in the production menu.
    obj: String,
    /// Source citation for the menu option.
    #[cfg_attr(feature = "path-schema", schemars(regex(pattern = r"\S")))]
    source: String,
}
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct MakeArgs {
    /// Optional loc trigger. With `op`, click that operation; without `op`,
    /// use the menu item on the loc. When omitted, the menu must already be open.
    #[serde(default)]
    loc: Option<MakeLoc>,
    /// Required with `loc`; the authored approach anchor and its source.
    #[serde(default)]
    anchor: Option<super::AnchorArg>,
    /// Symbolic product item config name.
    product: String,
    /// Optional production-menu row, when it differs from the product.
    #[serde(default)]
    menu: Option<MakeMenu>,
    /// Held-target quantity after this production step.
    qty: QuantityDocument,
    /// Allow Make-X only when the exact fixed quantity is not posted.
    #[serde(default)]
    make_x: bool,
}

#[derive(Clone)]
struct MakeTrigger {
    anchor: WorldTile,
    loc_id: i32,
    loc_name: Arc<str>,
    op: Option<Arc<str>>,
}
pub(super) fn compile_make(
    args: MakeArgs,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    let qty = compile_quantity(args.qty, cx)?;
    if qty.fixed().is_some_and(|qty| qty < 1) {
        return Err(CompileError::code("invalid-make"));
    }
    let trigger = match (args.loc, args.anchor) {
        (Some(loc_arg), Some(anchor_arg)) => {
            if anchor_arg.source.trim().is_empty() {
                return Err(CompileError::code("invalid-make"));
            }
            let loc = cx.selected.loc_by_config(&loc_arg.name).ok_or_else(|| {
                CompileError::code("unresolved-loc").with_detail(loc_arg.name.as_str())
            })?;
            if loc_arg.op.as_ref().is_some_and(|op| {
                !loc.ops
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(op))
            }) {
                return Err(CompileError::code("unsupported-loc-op"));
            }
            Some(MakeTrigger {
                anchor: anchor(anchor_arg.tile),
                loc_id: loc.id,
                loc_name: Arc::from(loc.display.as_deref().unwrap_or(&loc_arg.name)),
                op: loc_arg.op.map(Arc::from),
            })
        }
        (None, None) => None,
        _ => return Err(CompileError::code("invalid-make-trigger")),
    };
    let product_id = item(cx, &args.product)?.id;
    let menu_item_id = match args.menu {
        Some(menu) => {
            if menu.source.trim().is_empty() {
                return Err(CompileError::code("invalid-make-menu"));
            }
            item(cx, &menu.obj)?.id
        }
        None => product_id,
    };
    Ok(Arc::new(MakePlan {
        trigger,
        product_id,
        menu_item_id,
        items_per_process: make_items_per_process(&args.product),
        qty,
        make_x: args.make_x,
    }))
}

/// Revision-289 production yields come from server `cut_logs.dbrow` and
/// `smithing.dbrow` rows. The product data schema omits these rows, so keep
/// this map and test it against `production_yields.r289.fixture`.
/// Unknown products yield one item per process.
fn make_items_per_process(product: &str) -> i32 {
    match product {
        // `cut_logs.dbrow:4` says `data=shafts,15`.
        "arrow_shaft" => 15,
        // Smithing dart-tip rows set `product_amount` to 10 (`:1127`).
        "bronze_dart_tip" | "iron_dart_tip" | "steel_dart_tip" | "mithril_dart_tip"
        | "adamant_dart_tip" | "rune_dart_tip" => 10,
        // Smithing arrowhead rows set `product_amount` to 15 (`:1193`).
        "bronze_arrowheads" | "iron_arrowheads" | "steel_arrowheads" | "mithril_arrowheads"
        | "adamant_arrowheads" | "rune_arrowheads" => 15,
        // Smithing knife rows set `product_amount` to 5 (`:1259`).
        "bronze_knife" | "iron_knife" | "steel_knife" | "mithril_knife" | "adamant_knife"
        | "rune_knife" => 5,
        // The smithing nails row sets `product_amount` to 2 (`:1347`).
        "nails" => 2,
        _ => 1,
    }
}

struct MakePlan {
    trigger: Option<MakeTrigger>,
    product_id: i32,
    menu_item_id: i32,
    items_per_process: i32,
    qty: QuantityPlan,
    make_x: bool,
}
impl StepPlan for MakePlan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        let qty = self.qty.evaluate_step(cx).ok_or_else(|| {
            ActionError::Unavailable(Arc::from("production quantity evidence unavailable"))
        })?;
        Ok(Box::new(MakeRun {
            trigger_plan: self.trigger.clone(),
            request: MakeRequest {
                product_id: self.product_id,
                menu_id: self.menu_item_id,
                qty,
                items_per_process: self.items_per_process,
                after_trigger: false,
                make_x: self.make_x,
            },
            small_handle: None,
            trigger_reach: None,
            retry: crate::native::WalkRetryState::default(),
            make: None,
        }))
    }
    fn settle_timeout(&self) -> Duration {
        Duration::from_secs(2 * 60)
    }
}

enum MakeSmallHandle {
    Walk(PrereqWalk),
    UseOn(ActionHandle<super::UseOnAction>),
}

struct MakeRun {
    trigger_plan: Option<MakeTrigger>,
    request: MakeRequest,
    small_handle: Option<MakeSmallHandle>,
    trigger_reach: Option<PrereqReach>,
    retry: crate::native::WalkRetryState,
    make: Option<ActionHandle<MakeMachine>>,
}

fn make_use_on_request(
    trigger: &MakeTrigger,
    loc: &api::snapshot::LocView,
    menu_id: i32,
    cx: &ActionContext<'_>,
) -> Result<crate::shim::InteractReq, ActionError> {
    let source_item = cx
        .snapshot()
        .inventory()
        .and_then(|items| {
            items
                .value
                .iter()
                .find(|item| item.def.id == menu_id && item.count > 0)
        })
        .ok_or_else(|| {
            ActionError::Unavailable(Arc::from("production trigger item is not held"))
        })?;
    let item_name = source_item.def.name.as_ref().ok_or_else(|| {
        ActionError::Unavailable(Arc::from("production trigger item name is unavailable"))
    })?;
    Ok(crate::shim::InteractReq::UseOn {
        name: item_name.clone(),
        kind: "loc".to_owned(),
        target_name: Some(trigger.loc_name.to_string()),
        x: loc.tile.x,
        z: loc.tile.z,
        level: loc.tile.level,
        index: None,
        source_item_id: Some(menu_id),
        source_item_slot: Some(source_item.slot),
        target_item_id: Some(trigger.loc_id),
        target_item_slot: None,
    })
}

impl StepRun for MakeRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        if self.request.qty <= 0 {
            return Poll::Ready(Ok(done(cx)));
        }
        let now = cx.tick.cx.evidence().tick;
        let mut accepted = false;
        if let Some(handle) = &mut self.small_handle {
            match handle {
                MakeSmallHandle::Walk(handle) => {
                    match handle.poll(cx.tick.actions, &mut cx.tick.cx, cx.tick.state) {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                        Poll::Ready(Ok(receipt)) => {
                            self.small_handle = None;
                            if self.retry.observe_approach_failure(&receipt, now) {
                                return Poll::Pending;
                            }
                            walk_step_evidence(receipt)?;
                            self.retry.reset();
                        }
                    }
                }
                MakeSmallHandle::UseOn(handle) => {
                    match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                        Poll::Ready(Ok(_)) => {
                            self.small_handle = None;
                            accepted = true;
                        }
                    }
                }
            }
        }
        if self.retry.waiting_for_next_tick(now) {
            return Poll::Pending;
        }
        if self.small_handle.is_none()
            && self.trigger_reach.is_none()
            && self.make.is_none()
            && !accepted
        {
            if let Some(trigger) = self.trigger_plan.clone() {
                let here = cx
                    .tick
                    .cx
                    .snapshot()
                    .local_player()
                    .map(|player| player.value.player.network);
                if !here.is_some_and(|here| reach::within(here, trigger.anchor, 6)) {
                    self.small_handle = Some(MakeSmallHandle::Walk(std::task::ready!(
                        begin_approach_walk(
                            &mut self.retry,
                            reach::walk_request(trigger.anchor, 4, None, cx.required_after),
                            cx,
                        )
                    )?));
                    return Poll::Pending;
                }
                let loc = {
                    let snapshot = cx.tick.cx.snapshot();
                    snapshot.locs().and_then(|locs| {
                        locs.value
                            .iter()
                            .filter(|loc| {
                                loc.id == trigger.loc_id
                                    && reach::within(trigger.anchor, loc.tile, reach::PROBE_RADIUS)
                            })
                            .min_by_key(|loc| {
                                let dx =
                                    (i64::from(loc.tile.x) - i64::from(trigger.anchor.x)).abs();
                                let dz =
                                    (i64::from(loc.tile.z) - i64::from(trigger.anchor.z)).abs();
                                dx.max(dz)
                            })
                            .cloned()
                    })
                };
                let Some(loc) = loc else {
                    return Poll::Pending;
                };
                if !reach::loc_arrived(&cx.tick.cx, &loc) {
                    let here = cx
                        .tick
                        .cx
                        .snapshot()
                        .local_player()
                        .map(|player| player.value.player.network);
                    self.small_handle = Some(MakeSmallHandle::Walk(std::task::ready!(
                        begin_approach_walk(
                            &mut self.retry,
                            reach::loc_walk_request(&loc, here, cx.required_after),
                            cx,
                        )
                    )?));
                    return Poll::Pending;
                }
                if let Some(op) = trigger.op.as_ref() {
                    self.trigger_reach = Some(std::task::ready!(crate::native::defer_budget(
                        PrereqReach::begin(
                            reach::ReachArgs {
                                kind: reach::ReachKind::Loc {
                                    id: Some(trigger.loc_id),
                                    name: Some(Arc::clone(&trigger.loc_name)),
                                },
                                op: Arc::clone(op),
                                anchor: Some(trigger.anchor),
                                radius: 8,
                                wait_if_missing: true,
                                target_tile: None,
                                reachable_only: false,
                            },
                            cx.tick.actions,
                            &mut cx.tick.cx,
                        )
                    ))?);
                } else {
                    let request =
                        make_use_on_request(&trigger, &loc, self.request.menu_id, &cx.tick.cx)?;
                    self.small_handle = Some(MakeSmallHandle::UseOn(std::task::ready!(
                        crate::native::defer_budget(
                            cx.tick
                                .actions
                                .begin::<super::UseOnAction>(request, &mut cx.tick.cx,)
                        )
                    )?));
                }
                return Poll::Pending;
            }
            self.make = Some(
                cx.tick
                    .actions
                    .begin::<MakeMachine>(self.request.clone(), &mut cx.tick.cx)?,
            );
        }
        if let Some(handle) = &mut self.trigger_reach {
            match handle.poll(cx.tick.actions, &mut cx.tick.cx, cx.tick.state) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(false)) => {
                    return Poll::Ready(Err(ActionError::Failed(Arc::from(
                        "production trigger failed",
                    ))))
                }
                Poll::Ready(Ok(true)) => accepted = true,
            }
        }
        if accepted {
            self.trigger_reach = None;
            self.small_handle = None;
            self.request.after_trigger = true;
            self.make = Some(
                cx.tick
                    .actions
                    .begin::<MakeMachine>(self.request.clone(), &mut cx.tick.cx)?,
            );
            if !cx.tick.cx.may_continue_this_tick() {
                return Poll::Pending;
            }
        }
        let Some(handle) = self.make.as_ref() else {
            return Poll::Pending;
        };
        match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Ready(Ok(receipt)) if receipt.held >= self.request.qty => {
                Poll::Ready(Ok(done(cx)))
            }
            Poll::Ready(Ok(_)) => {
                self.make = None;
                if self.trigger_plan.is_none() {
                    return Poll::Ready(Err(ActionError::Blocked(Arc::from(
                        "production menu closed before the held target; add a loc trigger to retry",
                    ))));
                }
                Poll::Pending
            }
        }
    }
    fn cancel(&mut self, _actions: &mut NativeActions) {
        if let Some(MakeSmallHandle::Walk(walk)) = self.small_handle.as_mut() {
            walk.cancel();
        }
        if let Some(trigger) = self.trigger_reach.as_mut() {
            trigger.cancel();
        }
        self.small_handle = None;
        self.trigger_reach = None;
        self.make = None;
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct EquipArgs {
    /// Symbolic item config name; omitted only when stripping all equipment.
    #[serde(default)]
    obj: Option<String>,
    /// Strip all worn items when `obj` is omitted.
    #[serde(default)]
    all: bool,
}
pub(super) fn compile_equip(
    args: EquipArgs,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    compile_equipment(args, cx, true)
}
pub(super) fn compile_unequip(
    args: EquipArgs,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    compile_equipment(args, cx, false)
}
fn compile_equipment(
    args: EquipArgs,
    cx: &CompileContext<'_>,
    wear: bool,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    let request = if !wear && args.all && args.obj.is_none() {
        EquipmentRequest::Strip {
            keep: Arc::from(cx.keep_ids),
        }
    } else {
        let obj = args.obj.ok_or_else(|| CompileError::code("missing-obj"))?;
        let item = item(cx, &obj)?;
        if wear {
            EquipmentRequest::Wear {
                id: item.id,
                name: item.name,
            }
        } else {
            EquipmentRequest::Unequip {
                id: item.id,
                name: item.name,
            }
        }
    };
    Ok(Arc::new(EquipmentPlan { request }))
}
struct EquipmentPlan {
    request: EquipmentRequest,
}
impl StepPlan for EquipmentPlan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        let equipment = cx
            .tick
            .actions
            .begin::<EquipmentMachine>(self.request.clone(), &mut cx.tick.cx)?;
        Ok(Box::new(EquipmentRun { equipment }))
    }
}
struct EquipmentRun {
    equipment: ActionHandle<EquipmentMachine>,
}
impl StepRun for EquipmentRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        match cx.tick.actions.poll(&self.equipment, &mut cx.tick.cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Ready(Ok(_)) => Poll::Ready(Ok(done(cx))),
        }
    }
    fn cancel(&mut self, _actions: &mut NativeActions) {}
}

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct LoadoutArgs {
    /// Name or Path-qualified name of the loadout to apply.
    loadout: String,
    /// Bank selector: `quest_bank` or `nearest`.
    #[serde(default)]
    at: Option<String>,
    /// Allow lower-tier alternatives when resolving the loadout.
    #[serde(default)]
    allow_lower_tier: bool,
    /// Remove worn equipment that is not in the loadout.
    #[serde(default)]
    strip: bool,
    /// Require exactly the listed worn items; remove other worn items into inventory.
    #[serde(default)]
    exclusive: bool,
}

pub(super) fn compile_loadout(
    args: LoadoutArgs,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    if args.exclusive && (args.strip || args.allow_lower_tier) {
        return Err(CompileError::code("exclusive-loadout-requires-exact-items"));
    }
    if args
        .at
        .as_deref()
        .is_some_and(|at| at != "quest_bank" && at != "nearest")
    {
        return Err(CompileError::code("invalid-bank-target"));
    }
    let qualified = if args.loadout.contains('/') {
        args.loadout.clone()
    } else {
        format!("{}/{}", cx.path.0, args.loadout)
    };
    let row = cx
        .loadouts
        .resolve(&qualified)
        .ok_or_else(|| CompileError::code("unknown-loadout"))?
        .row();
    let mut resolved = Vec::new();
    let carry = compile_loadout_carry(&qualified, row, cx)?;
    for entry in carry.iter() {
        let item = &entry.item;
        if !resolved.iter().any(|known: &BankItem| known.id == item.id) {
            resolved.push(item.clone());
        }
    }
    let mut worn_items = Vec::with_capacity(row.worn.len());
    for (slot, wanted) in &row.worn {
        let item = named_item(cx, wanted)?;
        if !resolved.iter().any(|known| known.id == item.id) {
            resolved.push(item.clone());
        }
        worn_items.push((slot.clone(), item));
    }
    let melee_family: Arc<[api::game_data::EquipmentNameEntry]> = Arc::from(
        cx.selected
            .equipment_names()
            .map(|facts| facts.melee_weapons.clone())
            .unwrap_or_default(),
    );
    for candidate in cx.selected.items() {
        if candidate.is_certificate() {
            continue;
        }
        let Some(name) = candidate.name.as_deref() else {
            continue;
        };
        // Name resolution can scan aliases and ambiguous display rows. Only
        // candidates that match this loadout need that canonicality check.
        let relevant = carry.iter().any(|entry| entry.item.id == candidate.id)
            || worn_items.iter().any(|(slot, wanted)| {
                wanted.id == candidate.id
                    || (wanted.name.eq_ignore_ascii_case("dragon longsword")
                        && name.eq_ignore_ascii_case("rune sword"))
                    || (wanted.name.eq_ignore_ascii_case("rune platebody")
                        && name.eq_ignore_ascii_case("rune chainbody"))
                    || crate::melee_weapons::same_or_lower_metal_item(name, &wanted.name)
                    || (slot.eq_ignore_ascii_case("righthand")
                        && crate::melee_weapons::same_or_lower_melee_weapon(name, &wanted.name))
            });
        if relevant
            && cx
                .selected
                .resolve_item_name(name)
                .is_some_and(|item| item.id == candidate.id)
            && !resolved.iter().any(|known| known.id == candidate.id)
        {
            resolved.push(BankItem {
                id: candidate.id,
                name: Arc::from(name),
            });
        }
    }
    Ok(Arc::new(LoadoutPlan {
        bank: cx.bank,
        bank_required: cx.bank_required,
        row: Arc::new(row.clone()),
        carry,
        resolved: Arc::from(resolved),
        melee_family,
        keep_ids: Arc::from(cx.keep_ids),
        allow_lower_tier: args.allow_lower_tier,
        strip: args.strip,
        exclusive: args.exclusive,
    }))
}

pub(super) fn compile_loadout_carry(
    qualified: &str,
    row: &api::loadout::Loadout,
    cx: &CompileContext<'_>,
) -> Result<Arc<[CompiledCarry]>, CompileError> {
    if let Some(carry) = cx.loadout_carry.get(qualified) {
        return Ok(Arc::clone(carry));
    }
    // Operator-only rows have no authored Path carry to reuse.
    row.carry
        .iter()
        .map(|entry| {
            let (item, stackable) = named_item_with_stackable(cx, &entry.item)?;
            Ok(CompiledCarry {
                item,
                qty: i32::try_from(entry.qty).unwrap_or(i32::MAX),
                stackable,
                latch_index: u8::MAX,
            })
        })
        .collect()
}

#[derive(Clone)]
struct LoadoutPlan {
    bank: Option<api::named_banks::NamedBank>,
    bank_required: bool,
    row: Arc<api::loadout::Loadout>,
    carry: Arc<[CompiledCarry]>,
    resolved: Arc<[BankItem]>,
    melee_family: Arc<[api::game_data::EquipmentNameEntry]>,
    keep_ids: Arc<[i32]>,
    allow_lower_tier: bool,
    strip: bool,
    exclusive: bool,
}

impl StepPlan for LoadoutPlan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        let snapshot = cx.tick.cx.snapshot();
        if snapshot.inventory().is_none() || snapshot.equipment().is_none() {
            return Ok(Box::new(LoadoutObservationWait {
                plan: self.clone(),
                run: None,
                deadline: cx.tick.cx.active_now() + LOADOUT_OBSERVATION_TIMEOUT,
            }));
        }
        Ok(Box::new(self.begin_observed(cx)?))
    }
}

impl LoadoutPlan {
    fn begin_observed(&self, cx: &mut StepContext<'_, '_>) -> Result<LoadoutRun, ActionError> {
        let snapshot = cx.tick.cx.snapshot();
        let skill = |name: &str| {
            snapshot.stats().and_then(|stats| {
                stats
                    .value
                    .iter()
                    .find(|stat| stat.name.eq_ignore_ascii_case(name))
                    .map(|stat| stat.base)
            })
        };
        let quest_complete = |name: &str| {
            snapshot.quest_statuses().is_some_and(|rows| {
                rows.value.iter().any(|row| {
                    row.name.eq_ignore_ascii_case(name)
                        && row.status() == api::snapshot::QuestListStatus::Complete
                })
            })
        };
        let facts = crate::quester::loadouts::TierFacts {
            attack: skill("attack").unwrap_or(0),
            defence: skill("defence").unwrap_or(0),
            ranged: skill("ranged").unwrap_or(0),
            lost_city: quest_complete("Lost City"),
            heroes: quest_complete("Heroes' Quest"),
            dragon_slayer: quest_complete("Dragon Slayer"),
        };
        let available = self
            .resolved
            .iter()
            .map(|candidate| candidate.name.to_string())
            .collect::<Vec<_>>();
        let resolve = |name: &str| {
            self.resolved
                .iter()
                .find(|item| item.name.eq_ignore_ascii_case(name))
                .cloned()
                .ok_or_else(|| ActionError::Unavailable(Arc::from("unresolved loadout item")))
        };
        let stock = snapshot.stock();
        let mut bank_actions = Vec::new();
        let mut worn = Vec::new();
        if !self.strip {
            for carry in self.carry.iter() {
                if stock.holds(carry.item.id, carry.qty) != Truth::True {
                    bank_actions.push(BankAction::Withdraw {
                        item: carry.item.clone(),
                        qty: carry.qty,
                    });
                }
            }
            for (slot, name) in &self.row.worn {
                let names = if self.allow_lower_tier {
                    crate::quester::loadouts::tier_candidates(
                        slot,
                        name,
                        &available,
                        facts,
                        &self.melee_family,
                    )
                } else {
                    vec![name.clone()]
                };
                if names.is_empty() {
                    return Err(ActionError::Unavailable(Arc::from(
                        "no stat/quest-legal loadout tier",
                    )));
                }
                let items = names
                    .iter()
                    .map(|name| resolve(name))
                    .collect::<Result<Vec<_>, _>>()?;
                // Tier resolution from `Stock` (design-bank-snapshot §4 F4):
                // a legal tier already carried needs no bank action; else the
                // strongest tier the memory banks is a plain `Withdraw`; only
                // an `Unknown`/`Hint` bank with no banked tier falls back to
                // `WithdrawAny` at the open table (a `Session` bank with none
                // is refused in place by the run, D4).
                let carried = items
                    .iter()
                    .any(|item| stock.has(item.id, 1) == Truth::True);
                if !carried {
                    if let Some(banked) = items
                        .iter()
                        .find(|item| stock.bank_has(item.id, 1) == Truth::True)
                    {
                        bank_actions.push(BankAction::Withdraw {
                            item: banked.clone(),
                            qty: 1,
                        });
                    } else if self.allow_lower_tier {
                        bank_actions.push(BankAction::WithdrawAny {
                            items: Arc::from(items.clone()),
                            qty: 1,
                        });
                    } else {
                        bank_actions.push(BankAction::Withdraw {
                            item: items[0].clone(),
                            qty: 1,
                        });
                    }
                }
                worn.push(Arc::from(items));
            }
            // No `Close` here: the trip leaves the bank open for the next
            // step's run to reuse (design-bank-snapshot §4 F3).
        }
        Ok(LoadoutRun {
            bank: (!bank_actions.is_empty()).then(|| {
                AcquireRun::new(
                    AcquireRequest::bank_actions(
                        self.bank,
                        self.bank_required,
                        Arc::from(bank_actions),
                        false,
                        cx.required_after,
                    ),
                    AcquirePlan::authored(),
                    cx.banks,
                    &cx.tick.cx,
                )
                .with_quester_walk_retries()
            }),
            worn: Arc::from(worn),
            keep_ids: Arc::clone(&self.keep_ids),
            worn_index: 0,
            equipment: None,
            strip: self.strip,
            stripped: false,
            exclusive: self.exclusive,
            removing: false,
            receipt: None,
        })
    }
}

struct LoadoutObservationWait {
    plan: LoadoutPlan,
    run: Option<LoadoutRun>,
    deadline: Duration,
}

impl StepRun for LoadoutObservationWait {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        if self.run.is_none() {
            let snapshot = cx.tick.cx.snapshot();
            if snapshot.inventory().is_none() || snapshot.equipment().is_none() {
                if cx.tick.cx.active_now() >= self.deadline {
                    return Poll::Ready(Err(ActionError::NeedsEvidence(Arc::from([]))));
                }
                return Poll::Pending;
            }
            match self.plan.begin_observed(cx) {
                Ok(run) => self.run = Some(run),
                Err(error) => return Poll::Ready(Err(error)),
            }
        }
        self.run.as_mut().expect("Loadout plan is ready").poll(cx)
    }

    fn cancel(&mut self, actions: &mut NativeActions) {
        if let Some(run) = &mut self.run {
            run.cancel(actions);
        }
    }

    fn prayer_cleanup(&self) -> RaisedPrayers {
        self.run
            .as_ref()
            .map_or_else(RaisedPrayers::empty, |run| run.prayer_cleanup())
    }

    fn needs_progress_read(&self) -> bool {
        self.run
            .as_ref()
            .is_some_and(|run| run.needs_progress_read())
    }

    fn progress_read_completed(&mut self, now: Duration) {
        if let Some(run) = &mut self.run {
            run.progress_read_completed(now);
        }
    }

    fn waiting_for(&self) -> Option<(&'static str, &Arc<str>)> {
        match &self.run {
            Some(run) => run.waiting_for(),
            None => Some(("Loadout observation", loadout_observation_wait_detail())),
        }
    }

    fn in_flight_outcome(&self) -> Option<&StepOutcome> {
        self.run.as_ref().and_then(|run| run.in_flight_outcome())
    }
}

struct LoadoutRun {
    bank: Option<AcquireRun>,
    worn: Arc<[Arc<[BankItem]>]>,
    worn_index: usize,
    equipment: Option<ActionHandle<EquipmentMachine>>,
    keep_ids: Arc<[i32]>,
    strip: bool,
    stripped: bool,
    exclusive: bool,
    removing: bool,
    receipt: Option<Arc<dyn FamilyReceipt>>,
}
impl LoadoutRun {
    fn begin_equipment(
        &mut self,
        request: EquipmentRequest,
        cx: &mut StepContext<'_, '_>,
    ) -> Result<(), ActionError> {
        self.removing = matches!(request, EquipmentRequest::Unequip { .. });
        self.equipment = Some(
            cx.tick
                .actions
                .begin::<EquipmentMachine>(request, &mut cx.tick.cx)?,
        );
        Ok(())
    }
}
impl StepRun for LoadoutRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        if let Some(handle) = &self.equipment {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(_)) => {
                    self.equipment = None;
                    if self.removing {
                        self.removing = false;
                    } else if self.strip {
                        self.stripped = true;
                    } else {
                        self.worn_index += 1;
                    }
                }
            }
        }
        if let Some(bank) = &mut self.bank {
            match bank.poll(cx.tick, OpenGate::Open) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(AcquireOutcome::Done(receipt))) => {
                    let receipt = receipt.last_bank.unwrap_or(BankReceipt {
                        complete: receipt.complete,
                        deposited: receipt.deposited,
                    });
                    self.receipt = Some(Arc::new(receipt) as Arc<dyn FamilyReceipt>);
                    self.bank = None;
                }
                Poll::Ready(Ok(AcquireOutcome::ChildDue { .. })) => {
                    return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                        "authored bank stage requested a child",
                    ))));
                }
            }
        }
        // Exclusive stripping follows the trip without a separate Close. On
        // 289, Unequip/Strip is accepted while open and closes the bank itself.
        if self.exclusive {
            let snapshot = cx.tick.cx.snapshot();
            let Some(equipment) = snapshot.equipment() else {
                return Poll::Pending;
            };
            if let Some(extra) = equipment.value.iter().find(|row| {
                row.count > 0
                    && !self
                        .worn
                        .iter()
                        .any(|items| items.iter().any(|wanted| wanted.id == row.def.id))
            }) {
                let stock = snapshot.stock();
                let (Some(held), Some(free)) = (stock.held(extra.def.id), stock.free_slots())
                else {
                    return Poll::Pending;
                };
                let space = if extra.def.stackable && held > 0 {
                    i64::from(held) + i64::from(extra.count) <= i64::from(i32::MAX)
                } else {
                    free > 0
                };
                if !space {
                    return Poll::Ready(Err(ActionError::Blocked(Arc::from(
                        "exclusive loadout: inventory space required to remove worn items",
                    ))));
                }
                let Some(name) = extra.def.name.as_deref() else {
                    return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                        "exclusive loadout: worn item name unavailable",
                    ))));
                };
                self.begin_equipment(
                    EquipmentRequest::Unequip {
                        id: extra.def.id,
                        name: Arc::from(name),
                    },
                    cx,
                )?;
                return Poll::Pending;
            }
        }
        let request = loop {
            if self.strip && !self.stripped {
                break Some(EquipmentRequest::Strip {
                    keep: Arc::clone(&self.keep_ids),
                });
            }
            let Some(items) = self.worn.get(self.worn_index) else {
                break None;
            };
            let snapshot = cx.tick.cx.snapshot();
            let Some(equipment) = snapshot.equipment() else {
                return Poll::Pending;
            };
            if equipment.value.iter().any(|row| {
                row.count > 0 && items.iter().any(|candidate| candidate.id == row.def.id)
            }) {
                self.worn_index += 1;
                continue;
            }
            let Some(inventory) = snapshot.inventory() else {
                return Poll::Pending;
            };
            let Some(item) = items
                .iter()
                .find(|candidate| {
                    inventory
                        .value
                        .iter()
                        .any(|row| row.count > 0 && row.def.id == candidate.id)
                })
                .cloned()
            else {
                return Poll::Ready(Err(ActionError::Failed(Arc::from(
                    "withdrawn loadout tier is not held",
                ))));
            };
            break Some(EquipmentRequest::Wear {
                id: item.id,
                name: Arc::clone(&item.name),
            });
        };
        if let Some(request) = request {
            self.begin_equipment(request, cx)?;
            return Poll::Pending;
        }
        Poll::Ready(Ok(StepOutcome {
            progress: None,
            evidence: cx.tick.cx.evidence(),
            receipt: self.receipt.take(),
        }))
    }
    fn cancel(&mut self, _actions: &mut NativeActions) {}
}

pub(super) fn compile_bank_known(
    _args: NoArgs,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    Ok(Arc::new(BankKnown))
}

/// `True` once the account's bank memory is known (`Hint` or `Session`);
/// `Unknown` — never `False` — while it was never observed (design-bank-
/// snapshot §6 bug 6). The runner resolves the `Unknown` with one
/// provisioning scan, so an authored `scan` guarded by `bank_known` is
/// skipped after the memory learns the bank.
struct BankKnown;
impl PredicatePlan for BankKnown {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        if cx.cx.snapshot().stock().banked_origin() == Origin::Unknown {
            Truth::Unknown
        } else {
            Truth::True
        }
    }
    fn requires_bank(&self) -> bool {
        true
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct BankHasArgs {
    /// Symbolic item config name.
    obj: String,
    /// Minimum count; omitted or below 1 uses 1.
    #[serde(default = "one")]
    qty: i32,
}

pub(super) fn compile_bank_has(
    args: BankHasArgs,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    Ok(Arc::new(BankHas {
        id: item(cx, &args.obj)?.id,
        qty: args.qty.max(1),
    }))
}

struct BankHas {
    id: i32,
    qty: i32,
}
impl PredicatePlan for BankHas {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        cx.cx.snapshot().stock().bank_has(self.id, self.qty)
    }

    fn requires_bank(&self) -> bool {
        true
    }
}

pub(super) fn compile_loadout_ready(
    args: LoadoutArgs,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    if args.exclusive && (args.strip || args.allow_lower_tier) {
        return Err(CompileError::code("exclusive-loadout-requires-exact-items"));
    }
    let qualified = if args.loadout.contains('/') {
        args.loadout
    } else {
        format!("{}/{}", cx.path.0, args.loadout)
    };
    let row = cx
        .loadouts
        .resolve(&qualified)
        .ok_or_else(|| CompileError::code("unknown-loadout"))?
        .row();
    let carry = compile_loadout_carry(&qualified, row, cx)?;
    let worn = row
        .worn
        .iter()
        .map(|(slot, wanted)| {
            let wanted_item = named_item(cx, wanted)?;
            let mut ids = vec![wanted_item.id];
            if args.allow_lower_tier {
                ids.extend(cx.selected.items().iter().filter_map(|candidate| {
                    if candidate.is_certificate() {
                        return None;
                    }
                    let name = candidate.name.as_deref()?;
                    let canonical = cx
                        .selected
                        .resolve_item_name(name)
                        .is_some_and(|item| item.id == candidate.id);
                    let same_tier =
                        crate::melee_weapons::same_or_lower_metal_item(name, &wanted_item.name)
                            || (slot.eq_ignore_ascii_case("righthand")
                                && crate::melee_weapons::same_or_lower_melee_weapon(
                                    name,
                                    &wanted_item.name,
                                ));
                    let special = (wanted_item.name.eq_ignore_ascii_case("dragon longsword")
                        && name.eq_ignore_ascii_case("rune sword"))
                        || (wanted_item.name.eq_ignore_ascii_case("rune platebody")
                            && name.eq_ignore_ascii_case("rune chainbody"));
                    (canonical && (same_tier || special)).then_some(candidate.id)
                }));
                ids.sort_unstable();
                ids.dedup();
            }
            Ok(Arc::from(ids))
        })
        .collect::<Result<Vec<_>, CompileError>>()?;
    Ok(Arc::new(LoadoutReady {
        carry,
        worn: Arc::from(worn),
        keep_ids: Arc::from(cx.keep_ids),
        strip: args.strip,
        exclusive: args.exclusive,
    }))
}

struct LoadoutReady {
    carry: Arc<[CompiledCarry]>,
    worn: Arc<[Arc<[i32]>]>,
    strip: bool,
    keep_ids: Arc<[i32]>,
    exclusive: bool,
}
impl PredicatePlan for LoadoutReady {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        let snapshot = cx.cx.snapshot();
        if snapshot.inventory().is_none() {
            return Truth::Unknown;
        }
        let Some(equipment) = snapshot.equipment() else {
            return Truth::Unknown;
        };
        let stock = snapshot.stock();
        let carry_ready = self
            .carry
            .iter()
            .all(|carry| stock.holds(carry.item.id, carry.qty) == Truth::True);
        let worn_ready = if self.strip {
            equipment
                .value
                .iter()
                .all(|row| row.count <= 0 || self.keep_ids.contains(&row.def.id))
        } else {
            self.worn
                .iter()
                .all(|ids| stock.wears_any(ids) == Truth::True)
        };
        let no_extra_equipment = !self.exclusive
            || equipment.value.iter().all(|item| {
                item.count <= 0 || self.worn.iter().any(|ids| ids.contains(&item.def.id))
            });
        if carry_ready && worn_ready && no_extra_equipment {
            Truth::True
        } else {
            Truth::False
        }
    }
}

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct EquipmentOnlyArgs {
    /// Exact set of object aliases that must be worn; an empty list requires no equipment.
    objs: Vec<String>,
}

pub(super) fn compile_equipment_only(
    args: EquipmentOnlyArgs,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let mut ids = args
        .objs
        .iter()
        .map(|alias| item(cx, alias).map(|item| item.id))
        .collect::<Result<Vec<_>, _>>()?;
    ids.sort_unstable();
    ids.dedup();
    Ok(Arc::new(EquipmentOnly {
        ids: Arc::from(ids),
    }))
}

struct EquipmentOnly {
    ids: Arc<[i32]>,
}

impl PredicatePlan for EquipmentOnly {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        let Some(equipment) = cx.cx.snapshot().equipment() else {
            return Truth::Unknown;
        };
        let exact = equipment
            .value
            .iter()
            .all(|item| item.count <= 0 || self.ids.contains(&item.def.id))
            && self.ids.iter().all(|id| {
                equipment
                    .value
                    .iter()
                    .any(|item| item.count > 0 && item.def.id == *id)
            });
        if exact {
            Truth::True
        } else {
            Truth::False
        }
    }
}

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct PackOnlyArgs {
    /// Object aliases the pack may hold; none of them is required. The Path's
    /// protected items (the set `bank deposit_all` always keeps) are allowed too.
    objs: Vec<String>,
}

pub(super) fn compile_pack_only(
    args: PackOnlyArgs,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let mut ids = cx.keep_ids.to_vec();
    for alias in &args.objs {
        ids.push(item(cx, alias)?.id);
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(Arc::new(PackOnly {
        ids: Arc::from(ids),
    }))
}

/// True when every held pack row is an allowed id: a `bank deposit_all`
/// with the same keep list would move nothing.
struct PackOnly {
    ids: Arc<[i32]>,
}

impl PredicatePlan for PackOnly {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        let Some(inventory) = cx.cx.snapshot().inventory() else {
            return Truth::Unknown;
        };
        if inventory
            .value
            .iter()
            .all(|item| item.count <= 0 || self.ids.binary_search(&item.def.id).is_ok())
        {
            Truth::True
        } else {
            Truth::False
        }
    }
}

fn done(cx: &StepContext<'_, '_>) -> StepOutcome {
    StepOutcome {
        progress: None,
        evidence: cx.tick.cx.evidence(),
        receipt: None,
    }
}
#[cfg(test)]
#[path = "loadout_tests.rs"]
mod loadout_tests;

#[cfg(test)]
#[path = "production_state_size_tests.rs"]
mod production_state_size_tests;

#[cfg(test)]
mod make_retry_tests {
    use super::*;
    use crate::quester::families::tests::{local_player, post_walk_receipt, with_tick};
    use api::snapshot::GameSnapshot;

    fn with_step<R>(
        tick: &mut crate::native::NativeTick<'_>,
        f: impl FnOnce(&mut StepContext<'_, '_>) -> R,
    ) -> R {
        let quests = api::quest_facts::QuestCatalog::empty();
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let choices = crate::quester::choices::QuestChoices::default();
        let required_after = tick.cx.evidence();
        f(&mut StepContext {
            tick,
            quests: &quests,
            progress: &[],
            required_after,
            banks: &banks,
            choices: &choices,
        })
    }

    #[test]
    fn make_approach_waits_a_tick_retries_twice_and_keeps_last_detail() {
        let tile = WorldTile {
            x: 12,
            z: 34,
            level: 0,
        };
        let start = WorldTile {
            x: 30,
            z: 40,
            level: 0,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(start));
        let mut ledger = None;
        let mut run = MakeRun {
            trigger_plan: Some(MakeTrigger {
                anchor: tile,
                loc_id: 41,
                loc_name: Arc::from("Anvil"),
                op: Some(Arc::from("Smith")),
            }),
            request: MakeRequest {
                product_id: 1,
                menu_id: 1,
                qty: 1,
                items_per_process: 1,
                after_trigger: false,
                make_x: false,
            },
            small_handle: None,
            retry: crate::native::WalkRetryState::default(),
            trigger_reach: None,
            make: None,
        };

        assert!(with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        for (failure_tick, detail) in [
            (2, "first make route failed"),
            (4, "second make route failed"),
        ] {
            post_walk_receipt(
                &mut ledger,
                failure_tick,
                crate::native::WalkEnd::NoPath,
                Some(detail),
            );
            assert!(with_tick(&snapshot, &mut ledger, failure_tick, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
            if failure_tick == 2 {
                assert!(with_tick(&snapshot, &mut ledger, failure_tick, |tick| {
                    with_step(tick, |cx| run.poll(cx))
                })
                .is_pending());
                assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
            }
            let retry_tick = failure_tick + 1;
            assert!(with_tick(&snapshot, &mut ledger, retry_tick, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
            let retry = ledger.as_ref().unwrap().outbox.last().unwrap();
            let crate::native::HostEffect::Walk(request) = &retry.effect else {
                panic!("a make route retry should submit a new walk");
            };
            assert_eq!(request.required_after.tick, retry_tick);
        }

        post_walk_receipt(
            &mut ledger,
            6,
            crate::native::WalkEnd::NoPath,
            Some("third make route failed"),
        );
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 6, |tick| {
                with_step(tick, |cx| run.poll(cx))
            }),
            Poll::Ready(Err(ActionError::Blocked(detail)))
                if detail.as_ref() == "third make route failed"
        ));
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
    }
}
#[cfg(test)]
mod production_yield_tests {
    use super::make_items_per_process;

    #[test]
    fn make_yields_match_revision_289_content_rows() {
        // Source rows: cut_logs.dbrow:1-7 and smithing.dbrow:1123-1317,1343-1352.
        let mut section = None;
        let mut product = None;
        let mut checked = 0;

        for line in include_str!("production_yields.r289.fixture").lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(name) = line
                .strip_prefix('[')
                .and_then(|line| line.strip_suffix(']'))
            {
                section = Some(name);
                product = None;
                continue;
            }
            if let Some(name) = line.strip_prefix("data=product,") {
                product = Some(name);
                continue;
            }
            if let Some(amount) = line.strip_prefix("data=product_amount,") {
                let product = product.expect("smithing row product");
                assert_eq!(section, Some(product), "{product} content row");
                let amount = amount.parse::<i32>().expect("smithing yield");
                assert_eq!(make_items_per_process(product), amount, "{product}");
                checked += 1;
                continue;
            }
            if let Some(amount) = line.strip_prefix("data=shafts,") {
                assert_eq!(section, Some("fletching_normal"));
                let amount = amount.parse::<i32>().expect("fletching yield");
                assert_eq!(make_items_per_process("arrow_shaft"), amount);
                checked += 1;
            }
        }

        assert_eq!(checked, 20);
        assert_eq!(make_items_per_process("unlisted_product"), 1);
    }
}
