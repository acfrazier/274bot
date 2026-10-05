//! Bank, shop, production, equipment and loadout compiled families.
use super::{reach, walk_step_evidence, NoArgs};
use crate::bank::{BankStandAccess, Open, OpenArgs, PickKind, Select, SelectArgs};
use crate::combat::RaisedPrayers;
use crate::native::walk::Walk;
use crate::native::{ActionContext, ActionError, ActionHandle, NativeActions, WalkOptions};
use crate::native_bank::{BankAction, BankItem, BankMachine, BankReceipt, BankRequest};
use crate::native_equipment::{EquipmentMachine, EquipmentRequest};
use crate::native_production::{MakeMachine, MakeRequest};
use crate::native_shop::{BuyMachine, BuyRequest};
use crate::quester::compile::{
    CompileContext, CompileError, FamilyReceipt, PredicateContext, PredicatePlan, StepContext,
    StepOutcome, StepPlan, StepRun,
};
use api::quest_progress::{EvidenceStamp, QuestProgress};
use api::selected::{FactKey, Knowledge, Truth};
use api::snapshot::WorldTile;
use serde::Deserialize;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

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
    let item = cx
        .selected
        .resolve_item_name(name)
        .filter(|item| !item.is_certificate())
        .ok_or_else(|| CompileError::code("unresolved-loadout-item"))?;
    let display = item
        .name
        .as_deref()
        .ok_or_else(|| CompileError::code("unresolved-loadout-item"))?;
    Ok(BankItem {
        id: item.id,
        name: Arc::from(display),
    })
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
    /// Requested quantity; omitted uses 1 and values below 1 are rejected.
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
        memo_ids: Arc::from(cx.bank_items),
        actions: Arc::from(actions),
        partial_ok: args.partial_ok,
    }))
}

struct BankPlan {
    bank: Option<api::named_banks::NamedBank>,
    memo_ids: Arc<[i32]>,
    bank_required: bool,
    actions: Arc<[BankAction]>,
    partial_ok: bool,
}
impl StepPlan for BankPlan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        let memo_ids = Arc::clone(&self.memo_ids);
        let actions = Arc::clone(&self.actions);
        let run = if self.bank_required {
            BankRun::new_with_required(
                self.bank,
                true,
                memo_ids,
                actions,
                self.partial_ok,
                cx.banks,
            )
        } else {
            BankRun::new(self.bank, memo_ids, actions, self.partial_ok, cx.banks)
        };
        Ok(Box::new(run))
    }
    fn settle_timeout(&self) -> Duration {
        Duration::from_secs(12)
    }
}

struct BankRun {
    bank: Option<api::named_banks::NamedBank>,
    explicit: Option<Arc<str>>,
    required_bank_missing: bool,
    selection: Option<ActionHandle<Select>>,
    picked: bool,
    access: Option<Arc<BankStandAccess>>,
    target: Option<WorldTile>,
    memo_ids: Arc<[i32]>,
    actions: Arc<[BankAction]>,
    partial_ok: bool,
    index: usize,
    walk_started: bool,
    walk: Option<ActionHandle<Walk>>,
    open_started: bool,
    opening: Option<ActionHandle<Open>>,
    machine: Option<ActionHandle<BankMachine>>,
    last: Option<BankReceipt>,
}
impl BankRun {
    fn new(
        bank: Option<api::named_banks::NamedBank>,
        memo_ids: Arc<[i32]>,
        actions: Arc<[BankAction]>,
        partial_ok: bool,
        facts: &api::named_banks::NamedBankFacts,
    ) -> Self {
        Self::new_with_required(bank, false, memo_ids, actions, partial_ok, facts)
    }

    fn new_with_required(
        bank: Option<api::named_banks::NamedBank>,
        bank_required: bool,
        memo_ids: Arc<[i32]>,
        actions: Arc<[BankAction]>,
        partial_ok: bool,
        facts: &api::named_banks::NamedBankFacts,
    ) -> Self {
        let explicit = if bank_required {
            bank.and_then(|requested| {
                facts
                    .banks()
                    .iter()
                    .find(|candidate| candidate.tile == requested.tile)
                    .map(|candidate| Arc::from(candidate.name))
            })
        } else {
            None
        };
        let required_bank_missing = bank_required && explicit.is_none();
        Self {
            bank,
            explicit,
            required_bank_missing,
            selection: None,
            picked: false,
            access: None,
            target: None,
            memo_ids,
            actions,
            partial_ok,
            index: 0,
            walk_started: false,
            walk: None,
            open_started: false,
            opening: None,
            machine: None,
            last: None,
        }
    }
}
impl StepRun for BankRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        if self.required_bank_missing {
            return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                "required bank is not in the bank catalog",
            ))));
        }
        if !self.picked {
            if let Some(handle) = self.selection.as_ref() {
                match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                    Poll::Ready(Ok(selected)) => {
                        self.selection = None;
                        if selected.kind == PickKind::NoCandidate {
                            return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                                "no eligible bank",
                            ))));
                        }
                        let Some(access) = selected.access else {
                            return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                                "no eligible bank",
                            ))));
                        };
                        let Some(bank) = cx
                            .banks
                            .banks()
                            .get(usize::from(selected.bank_index))
                            .copied()
                        else {
                            return Poll::Ready(Err(ActionError::Stale));
                        };
                        self.bank = Some(bank);
                        self.target = Some(selected.access_tile);
                        self.access = Some(access);
                        self.picked = true;
                    }
                }
            }
            if !self.picked {
                let Some(from) = cx.tick.cx.snapshot().here() else {
                    return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                        "player position unavailable for bank selection",
                    ))));
                };
                self.selection = Some(cx.tick.actions.begin::<Select>(
                    SelectArgs {
                        facts: Arc::clone(cx.banks),
                        from: from.value,
                        preferences: api::named_banks::BankPreferences::default(),
                        options: WalkOptions::default(),
                        explicit: self.explicit.clone(),
                    },
                    &mut cx.tick.cx,
                )?);
                return Poll::Pending;
            }
        }

        if let Some(handle) = self.walk.as_ref() {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(receipt)) => {
                    walk_step_evidence(receipt)?;
                    self.walk = None;
                }
            }
        }
        if !self.walk_started {
            self.walk_started = true;
            let target = self.target.ok_or(ActionError::Stale)?;
            let near = cx
                .tick
                .cx
                .snapshot()
                .here()
                .is_some_and(|here| reach::within(here.value, target, 0));
            if !near {
                self.walk = Some(cx.tick.actions.begin::<Walk>(
                    reach::walk_request(target, 0, None, cx.required_after),
                    &mut cx.tick.cx,
                )?);
                return Poll::Pending;
            }
        }

        if let Some(handle) = self.opening.as_ref() {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(())) => self.opening = None,
            }
        }
        if !self.open_started {
            if let Some(access) = self.access.as_ref() {
                self.opening = Some(cx.tick.actions.begin::<Open>(
                    OpenArgs {
                        access: Arc::clone(access),
                    },
                    &mut cx.tick.cx,
                )?);
                self.open_started = true;
                return Poll::Pending;
            }
        }

        if let Some(handle) = self.machine.as_ref() {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(receipt)) => {
                    if !matches!(self.actions.get(self.index), Some(BankAction::Close)) {
                        self.last = Some(receipt);
                    }
                    self.machine = None;
                    self.index += 1;
                }
            }
        }
        if let Some(action) = self.actions.get(self.index).cloned() {
            self.machine = Some(cx.tick.actions.begin::<BankMachine>(
                BankRequest {
                    bank: self.bank,
                    action,
                    memo_ids: Arc::clone(&self.memo_ids),
                    partial_ok: self.partial_ok,
                },
                &mut cx.tick.cx,
            )?);
            return Poll::Pending;
        }
        let receipt = self
            .last
            .take()
            .map(|receipt| Arc::new(receipt) as Arc<dyn FamilyReceipt>);
        Poll::Ready(Ok(StepOutcome {
            progress: None,
            evidence: cx.tick.cx.evidence(),
            receipt,
        }))
    }
    fn cancel(&mut self, actions: &mut NativeActions) {
        if let Some(handle) = self.selection.take() {
            actions.cancel(handle);
        }
        if let Some(handle) = self.walk.take() {
            actions.cancel(handle);
        }
        if let Some(handle) = self.opening.take() {
            actions.cancel(handle);
        }
        if let Some(handle) = self.machine.take() {
            actions.cancel(handle);
        }
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
    /// Number of items to buy; must be at least 1.
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
            buy: None,
        }))
    }
}
struct BuyRun {
    tile: WorldTile,
    request: BuyRequest,
    walk: Option<ActionHandle<Walk>>,
    buy: Option<ActionHandle<BuyMachine>>,
}
impl StepRun for BuyRun {
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
        if self.buy.is_none() {
            let near = cx
                .tick
                .cx
                .snapshot()
                .here()
                .is_some_and(|here| reach::within(here.value, self.tile, 6));
            if !near {
                self.walk = Some(cx.tick.actions.begin::<Walk>(
                    reach::walk_request(self.tile, 4, None, cx.required_after),
                    &mut cx.tick.cx,
                )?);
                return Poll::Pending;
            }
            self.buy = Some(
                cx.tick
                    .actions
                    .begin::<BuyMachine>(self.request.clone(), &mut cx.tick.cx)?,
            );
            return Poll::Pending;
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
    fn cancel(&mut self, _actions: &mut NativeActions) {}
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct MakeLoc {
    /// Symbolic location config name.
    name: String,
    /// Location operation used to start production.
    op: String,
}
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct MakeMenu {
    /// Symbolic item config name shown in the production menu.
    obj: String,
    /// Source citation for the menu option.
    source: String,
}
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct MakeArgs {
    /// Location action used to start production.
    loc: MakeLoc,
    /// Authored approach anchor with a source citation.
    anchor: super::AnchorArg,
    /// Symbolic product item config name.
    product: String,
    /// Optional production menu choice.
    #[serde(default)]
    menu: Option<MakeMenu>,
    /// Fixed or journal-count-backed production quantity.
    qty: QuantityDocument,
    /// Use the game's make-X option when available.
    #[serde(default)]
    make_x: bool,
}
pub(super) fn compile_make(
    args: MakeArgs,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    let qty = compile_quantity(args.qty, cx)?;
    if qty.fixed().is_some_and(|qty| qty < 1) || args.anchor.source.trim().is_empty() {
        return Err(CompileError::code("invalid-make"));
    }
    let loc = cx
        .selected
        .loc_by_config(&args.loc.name)
        .ok_or_else(|| CompileError::code("unresolved-loc").with_detail(args.loc.name.as_str()))?;
    if !loc
        .ops
        .iter()
        .any(|op| op.eq_ignore_ascii_case(&args.loc.op))
    {
        return Err(CompileError::code("unsupported-loc-op"));
    }
    let product = item(cx, &args.product)?;
    let menu_id = match args.menu {
        Some(menu) => {
            if menu.source.trim().is_empty() {
                return Err(CompileError::code("invalid-make-menu"));
            }
            item(cx, &menu.obj)?.id
        }
        None => product.id,
    };
    Ok(Arc::new(MakePlan {
        tile: anchor(args.anchor.tile),
        loc_id: loc.id,
        loc_name: Arc::from(loc.display.as_deref().unwrap_or(&args.loc.name)),
        op: Arc::from(args.loc.op),
        product,
        menu_id,
        qty,
        make_x: args.make_x,
    }))
}
struct MakePlan {
    tile: WorldTile,
    loc_id: i32,
    loc_name: Arc<str>,
    op: Arc<str>,
    product: BankItem,
    menu_id: i32,
    qty: QuantityPlan,
    make_x: bool,
}
impl StepPlan for MakePlan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        let qty = self.qty.evaluate_step(cx).ok_or_else(|| {
            ActionError::Unavailable(Arc::from("production quantity evidence unavailable"))
        })?;
        Ok(Box::new(MakeRun {
            tile: self.tile,
            loc_id: self.loc_id,
            loc_name: Arc::clone(&self.loc_name),
            op: Arc::clone(&self.op),
            request: MakeRequest {
                product_id: self.product.id,
                menu_id: self.menu_id,
                qty,
                make_x: self.make_x,
            },
            walk: None,
            trigger: None,
            make: None,
        }))
    }
    fn settle_timeout(&self) -> Duration {
        Duration::from_secs(2 * 60)
    }
}
struct MakeRun {
    tile: WorldTile,
    loc_id: i32,
    loc_name: Arc<str>,
    op: Arc<str>,
    request: MakeRequest,
    walk: Option<ActionHandle<Walk>>,
    trigger: Option<ActionHandle<reach::Reach>>,
    make: Option<ActionHandle<MakeMachine>>,
}
impl StepRun for MakeRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        if self.request.qty <= 0 {
            return Poll::Ready(Ok(done(cx)));
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
        if self.trigger.is_none() && self.make.is_none() {
            let near = cx
                .tick
                .cx
                .snapshot()
                .here()
                .is_some_and(|here| reach::within(here.value, self.tile, 6));
            if !near {
                self.walk = Some(cx.tick.actions.begin::<Walk>(
                    reach::walk_request(self.tile, 4, None, cx.required_after),
                    &mut cx.tick.cx,
                )?);
                return Poll::Pending;
            }
            self.trigger = Some(cx.tick.actions.begin::<reach::Reach>(
                reach::ReachArgs {
                    kind: reach::ReachKind::Loc {
                        id: Some(self.loc_id),
                        name: Some(Arc::clone(&self.loc_name)),
                    },
                    op: Arc::clone(&self.op),
                    anchor: Some(self.tile),
                    radius: 8,
                    wait_if_missing: true,
                    target_tile: None,
                    reachable_only: false,
                },
                &mut cx.tick.cx,
            )?);
            return Poll::Pending;
        }
        if let Some(handle) = &self.trigger {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(false)) => {
                    return Poll::Ready(Err(ActionError::Failed(Arc::from(
                        "production trigger failed",
                    ))))
                }
                Poll::Ready(Ok(true)) => {
                    self.trigger = None;
                    self.make = Some(
                        cx.tick
                            .actions
                            .begin::<MakeMachine>(self.request.clone(), &mut cx.tick.cx)?,
                    );
                    return Poll::Pending;
                }
            }
        }
        match cx
            .tick
            .actions
            .poll(self.make.as_ref().unwrap(), &mut cx.tick.cx)
        {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Ready(Ok(_)) => Poll::Ready(Ok(done(cx))),
        }
    }
    fn cancel(&mut self, _actions: &mut NativeActions) {}
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
        let handle = cx
            .tick
            .actions
            .begin::<EquipmentMachine>(self.request.clone(), &mut cx.tick.cx)?;
        Ok(Box::new(EquipmentRun { handle }))
    }
}
struct EquipmentRun {
    handle: ActionHandle<EquipmentMachine>,
}
impl StepRun for EquipmentRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        match cx.tick.actions.poll(&self.handle, &mut cx.tick.cx) {
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
    let mut carry_ids = Vec::with_capacity(row.carry.len());
    for carry in &row.carry {
        let item = named_item(cx, &carry.item)?;
        carry_ids.push(item.id);
        if !resolved.iter().any(|known: &BankItem| known.id == item.id) {
            resolved.push(item);
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
        let canonical = cx
            .selected
            .resolve_item_name(name)
            .is_some_and(|item| item.id == candidate.id);
        let relevant = canonical
            && (carry_ids.contains(&candidate.id)
                || worn_items.iter().any(|(slot, wanted)| {
                    wanted.id == candidate.id
                        || (wanted.name.eq_ignore_ascii_case("dragon longsword")
                            && name.eq_ignore_ascii_case("rune sword"))
                        || (wanted.name.eq_ignore_ascii_case("rune platebody")
                            && name.eq_ignore_ascii_case("rune chainbody"))
                        || crate::melee_weapons::same_or_lower_metal_item(name, &wanted.name)
                        || (slot.eq_ignore_ascii_case("righthand")
                            && crate::melee_weapons::same_or_lower_melee_weapon(name, &wanted.name))
                }));
        if relevant && !resolved.iter().any(|known| known.id == candidate.id) {
            resolved.push(BankItem {
                id: candidate.id,
                name: Arc::from(name),
            });
        }
    }
    Ok(Arc::new(LoadoutPlan {
        bank: cx.bank,
        bank_required: cx.bank_required,
        memo_ids: Arc::from(cx.bank_items),
        row: Arc::new(row.clone()),
        resolved: Arc::from(resolved),
        melee_family,
        keep_ids: Arc::from(cx.keep_ids),
        allow_lower_tier: args.allow_lower_tier,
        strip: args.strip,
        exclusive: args.exclusive,
    }))
}

#[derive(Clone)]
struct LoadoutPlan {
    bank: Option<api::named_banks::NamedBank>,
    bank_required: bool,
    memo_ids: Arc<[i32]>,
    row: Arc<crate::loadouts_store::Loadout>,
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
        let mut bank_actions = Vec::new();
        let mut worn = Vec::new();
        if !self.strip {
            for carry in &self.row.carry {
                let item = resolve(&carry.item)?;
                let qty = i32::try_from(carry.qty).unwrap_or(i32::MAX);
                if !snapshot.inventory().is_some_and(|inventory| {
                    inventory
                        .value
                        .iter()
                        .filter(|row| row.def.id == item.id)
                        .map(|row| i64::from(row.count.max(0)))
                        .sum::<i64>()
                        >= i64::from(qty)
                }) {
                    bank_actions.push(BankAction::Withdraw { item, qty });
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
                if self.allow_lower_tier {
                    bank_actions.push(BankAction::WithdrawAny {
                        items: Arc::from(items.clone()),
                        qty: 1,
                    });
                } else if !snapshot.equipment().is_some_and(|equipment| {
                    equipment
                        .value
                        .iter()
                        .any(|row| row.count > 0 && row.def.id == items[0].id)
                }) && !snapshot.inventory().is_some_and(|inventory| {
                    inventory
                        .value
                        .iter()
                        .any(|row| row.count > 0 && row.def.id == items[0].id)
                }) {
                    bank_actions.push(BankAction::Withdraw {
                        item: items[0].clone(),
                        qty: 1,
                    });
                }
                worn.push(Arc::from(items));
            }
            if !bank_actions.is_empty() {
                bank_actions.push(BankAction::Close);
            }
        }
        Ok(LoadoutRun {
            bank: if bank_actions.is_empty() {
                None
            } else {
                let memo_ids = Arc::clone(&self.memo_ids);
                let actions = Arc::from(bank_actions);
                Some(if self.bank_required {
                    BankRun::new_with_required(self.bank, true, memo_ids, actions, false, cx.banks)
                } else {
                    BankRun::new(self.bank, memo_ids, actions, false, cx.banks)
                })
            },
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
}

impl StepRun for LoadoutObservationWait {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        if self.run.is_none() {
            let snapshot = cx.tick.cx.snapshot();
            if snapshot.inventory().is_none() || snapshot.equipment().is_none() {
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
        self.run.as_ref().and_then(|run| run.waiting_for())
    }

    fn in_flight_outcome(&self) -> Option<&StepOutcome> {
        self.run.as_ref().and_then(|run| run.in_flight_outcome())
    }
}

struct LoadoutRun {
    bank: Option<BankRun>,
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
                let (Some(inventory), Some(capacity)) =
                    (snapshot.inventory(), snapshot.inventory_capacity())
                else {
                    return Poll::Pending;
                };
                let held_stack = inventory
                    .value
                    .iter()
                    .find(|row| row.count > 0 && row.def.id == extra.def.id);
                let space = if extra.def.stackable {
                    held_stack.map_or_else(
                        || {
                            inventory.value.iter().filter(|row| row.count > 0).count()
                                < usize::from(capacity.value)
                        },
                        |held| {
                            i64::from(held.count) + i64::from(extra.count) <= i64::from(i32::MAX)
                        },
                    )
                } else {
                    inventory.value.iter().filter(|row| row.count > 0).count()
                        < usize::from(capacity.value)
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
                self.equipment = Some(cx.tick.actions.begin::<EquipmentMachine>(
                    EquipmentRequest::Unequip {
                        id: extra.def.id,
                        name: Arc::from(name),
                    },
                    &mut cx.tick.cx,
                )?);
                self.removing = true;
                return Poll::Pending;
            }
        }
        if let Some(bank) = &mut self.bank {
            match bank.poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(outcome)) => {
                    self.receipt = outcome.receipt;
                    self.bank = None;
                }
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
            self.equipment = Some(
                cx.tick
                    .actions
                    .begin::<EquipmentMachine>(request, &mut cx.tick.cx)?,
            );
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

struct BankKnown;
impl PredicatePlan for BankKnown {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        if cx.bank.known() {
            Truth::True
        } else {
            Truth::False
        }
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
        match cx.bank.count(self.id) {
            Some(count) => {
                if count >= self.qty {
                    Truth::True
                } else {
                    Truth::False
                }
            }
            None => Truth::Unknown,
        }
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
    let carry = row
        .carry
        .iter()
        .map(|entry| {
            Ok((
                named_item(cx, &entry.item)?.id,
                i32::try_from(entry.qty).unwrap_or(i32::MAX),
            ))
        })
        .collect::<Result<Vec<_>, CompileError>>()?;
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
        carry: Arc::from(carry),
        worn: Arc::from(worn),
        keep_ids: Arc::from(cx.keep_ids),
        strip: args.strip,
        exclusive: args.exclusive,
    }))
}

struct LoadoutReady {
    carry: Arc<[(i32, i32)]>,
    worn: Arc<[Arc<[i32]>]>,
    strip: bool,
    keep_ids: Arc<[i32]>,
    exclusive: bool,
}
impl PredicatePlan for LoadoutReady {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        let snapshot = cx.cx.snapshot();
        let Some(inventory) = snapshot.inventory() else {
            return Truth::Unknown;
        };
        let Some(equipment) = snapshot.equipment() else {
            return Truth::Unknown;
        };
        let carry_ready = self.carry.iter().all(|(id, qty)| {
            inventory
                .value
                .iter()
                .filter(|item| item.def.id == *id)
                .map(|item| item.count)
                .sum::<i32>()
                >= *qty
        });
        let worn_ready = if self.strip {
            equipment
                .value
                .iter()
                .all(|row| row.count <= 0 || self.keep_ids.contains(&row.def.id))
        } else {
            self.worn.iter().all(|ids| {
                equipment
                    .value
                    .iter()
                    .any(|item| item.count > 0 && ids.contains(&item.def.id))
            })
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
