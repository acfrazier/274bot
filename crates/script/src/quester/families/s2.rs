//! Bank, shop, production, equipment and loadout compiled families.
use super::reach;
use crate::native::walk::Walk;
use crate::native::{ActionError, ActionHandle, NativeActions, WalkEnd};
use crate::native_bank::{BankAction, BankItem, BankMachine, BankReceipt, BankRequest};
use crate::native_equipment::{EquipmentMachine, EquipmentRequest};
use crate::native_production::{MakeMachine, MakeRequest};
use crate::native_shop::{BuyMachine, BuyRequest};
use crate::quester::compile::{
    CompileContext, CompileError, FamilyReceipt, PredicateContext, PredicatePlan, StepContext,
    StepOutcome, StepPlan, StepRun,
};
use api::selected::Truth;
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
        .ok_or_else(|| CompileError::code("unresolved-obj"))?;
    let name = item
        .name
        .as_deref()
        .ok_or_else(|| CompileError::code("unresolved-obj-name"))?;
    Ok(BankItem {
        id: item.id,
        name: Arc::from(name),
    })
}

fn named_item(cx: &CompileContext<'_>, name: &str) -> Result<BankItem, CompileError> {
    if let Ok(item) = item(cx, name) {
        return Ok(item);
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
    Ok(BankItem {
        id: item.id,
        name: Arc::from(item.name.as_deref().unwrap_or(name)),
    })
}

fn resolve_bank(
    bank: Option<api::named_banks::NamedBank>,
    cx: &StepContext<'_, '_>,
) -> Result<api::named_banks::NamedBank, ActionError> {
    bank.or_else(|| {
        cx.tick
            .cx
            .snapshot()
            .here()
            .and_then(|here| crate::bank_select::nearest_bank(here.value))
    })
    .ok_or_else(|| ActionError::Unavailable(Arc::from("no eligible bank")))
}

fn metal_family_rank(name: &str) -> Option<(usize, &str)> {
    const METALS: [&str; 8] = [
        "bronze", "iron", "steel", "black", "mithril", "adamant", "rune", "dragon",
    ];
    let (metal, suffix) = name.trim().split_once(' ')?;
    METALS
        .iter()
        .position(|known| metal.eq_ignore_ascii_case(known))
        .map(|rank| (rank, suffix))
}

fn anchor(tile: [i32; 3]) -> WorldTile {
    WorldTile {
        x: tile[0],
        z: tile[1],
        level: tile[2],
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemQty {
    obj: String,
    #[serde(default = "one")]
    qty: i32,
}
fn one() -> i32 {
    1
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum BankOp {
    Scan,
    Withdraw,
    Deposit,
    DepositAll,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BankArgs {
    op: BankOp,
    #[serde(default)]
    at: Option<String>,
    #[serde(default)]
    items: Vec<ItemQty>,
    #[serde(default)]
    keep: Vec<String>,
    #[serde(default)]
    keep_ids: Vec<i32>,
    #[serde(default)]
    partial_ok: bool,
}

pub fn compile_bank(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    let args: BankArgs =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
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
            let mut keep = args.keep_ids;
            for alias in args.keep {
                keep.push(item(cx, &alias)?.id);
            }
            actions.push(BankAction::DepositAll {
                keep: Arc::from(keep),
            });
        }
    }
    Ok(Arc::new(BankPlan {
        bank: cx.bank,
        memo_ids: Arc::from(cx.bank_items),
        actions: Arc::from(actions),
        partial_ok: args.partial_ok,
    }))
}

struct BankPlan {
    bank: Option<api::named_banks::NamedBank>,
    memo_ids: Arc<[i32]>,
    actions: Arc<[BankAction]>,
    partial_ok: bool,
}
impl StepPlan for BankPlan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        Ok(Box::new(BankRun {
            bank: resolve_bank(self.bank, cx)?,
            memo_ids: Arc::clone(&self.memo_ids),
            actions: Arc::clone(&self.actions),
            partial_ok: self.partial_ok,
            index: 0,
            walk: None,
            machine: None,
            last: None,
        }))
    }
    fn settle_timeout(&self) -> Duration {
        Duration::from_secs(12)
    }
}

struct BankRun {
    bank: api::named_banks::NamedBank,
    memo_ids: Arc<[i32]>,
    actions: Arc<[BankAction]>,
    partial_ok: bool,
    index: usize,
    walk: Option<ActionHandle<Walk>>,
    machine: Option<ActionHandle<BankMachine>>,
    last: Option<BankReceipt>,
}
impl StepRun for BankRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        if let Some(handle) = &self.walk {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(receipt)) => {
                    if !matches!(receipt.end, WalkEnd::Arrived | WalkEnd::RouteEnded) {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "bank walk failed",
                        ))));
                    }
                    self.walk = None;
                }
            }
        }
        if self.walk.is_none() && self.index == 0 && self.machine.is_none() {
            let near = cx
                .tick
                .cx
                .snapshot()
                .here()
                .is_some_and(|here| reach::within(here.value, self.bank.tile, 6));
            if !near {
                self.walk = Some(cx.tick.actions.begin::<Walk>(
                    reach::walk_request(self.bank.tile, 4, cx.required_after),
                    &mut cx.tick.cx,
                )?);
                return Poll::Pending;
            }
        }
        if let Some(handle) = &self.machine {
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
    fn cancel(&mut self, _actions: &mut NativeActions) {}
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShopArg {
    npc: String,
    anchor: Anchor,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Anchor {
    tile: [i32; 3],
    #[serde(default)]
    source: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BuyArgs {
    shop: ShopArg,
    obj: String,
    qty: i32,
    #[serde(default)]
    est_gp: u32,
    #[serde(default)]
    option: Option<String>,
}

pub fn compile_buy(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    let args: BuyArgs =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
    if args.qty < 1 || args.shop.anchor.source.trim().is_empty() {
        return Err(CompileError::code("invalid-buy"));
    }
    let npc = cx
        .selected
        .npc_by_config(&args.shop.npc)
        .ok_or_else(|| CompileError::code("unresolved-npc"))?;
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
                Poll::Ready(Ok(_)) => self.walk = None,
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
                    reach::walk_request(self.tile, 4, cx.required_after),
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MakeLoc {
    name: String,
    op: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MakeArgs {
    loc: MakeLoc,
    anchor: Anchor,
    product: String,
    qty: i32,
    #[serde(default)]
    make_x: bool,
}
pub fn compile_make(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    let args: MakeArgs =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
    if args.qty < 1 || args.anchor.source.trim().is_empty() {
        return Err(CompileError::code("invalid-make"));
    }
    let loc = cx
        .selected
        .loc_by_config(&args.loc.name)
        .ok_or_else(|| CompileError::code("unresolved-loc"))?;
    if !loc
        .ops
        .iter()
        .any(|op| op.eq_ignore_ascii_case(&args.loc.op))
    {
        return Err(CompileError::code("unsupported-loc-op"));
    }
    let product = item(cx, &args.product)?;
    Ok(Arc::new(MakePlan {
        tile: anchor(args.anchor.tile),
        loc_id: loc.id,
        loc_name: Arc::from(loc.display.as_deref().unwrap_or(&args.loc.name)),
        op: Arc::from(args.loc.op),
        product,
        qty: args.qty,
        make_x: args.make_x,
    }))
}
struct MakePlan {
    tile: WorldTile,
    loc_id: i32,
    loc_name: Arc<str>,
    op: Arc<str>,
    product: BankItem,
    qty: i32,
    make_x: bool,
}
impl StepPlan for MakePlan {
    fn begin(&self, _cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        Ok(Box::new(MakeRun {
            tile: self.tile,
            loc_id: self.loc_id,
            loc_name: Arc::clone(&self.loc_name),
            op: Arc::clone(&self.op),
            request: MakeRequest {
                product_id: self.product.id,
                product_name: Arc::clone(&self.product.name),
                qty: self.qty,
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
        if let Some(handle) = &self.walk {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(_)) => self.walk = None,
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
                    reach::walk_request(self.tile, 4, cx.required_after),
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EquipArgs {
    #[serde(default)]
    obj: Option<String>,
    #[serde(default)]
    all: bool,
}
pub fn compile_equip(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    compile_equipment(args, cx, true)
}
pub fn compile_unequip(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    compile_equipment(args, cx, false)
}
fn compile_equipment(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
    wear: bool,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    let args: EquipArgs =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
    let request = if !wear && args.all && args.obj.is_none() {
        EquipmentRequest::Strip
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
#[serde(deny_unknown_fields)]
struct LoadoutArgs {
    loadout: String,
    #[serde(default)]
    at: Option<String>,
    #[serde(default)]
    allow_lower_tier: bool,
    #[serde(default)]
    strip: bool,
}

pub fn compile_loadout(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    let args: LoadoutArgs =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
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
        format!("{}/{}", cx.path_id.0, args.loadout)
    };
    let row = cx
        .loadouts
        .resolve(&qualified)
        .ok_or_else(|| CompileError::code("unknown-loadout"))?
        .row();
    for carry in &row.carry {
        let _ = named_item(cx, &carry.item)?;
    }
    for name in row.worn.values() {
        let _ = named_item(cx, name)?;
    }
    let mut resolved = Vec::new();
    for candidate in cx.selected.items() {
        let Some(name) = candidate.name.as_deref() else {
            continue;
        };
        let relevant = row
            .carry
            .iter()
            .any(|entry| entry.item.eq_ignore_ascii_case(name))
            || row.worn.values().any(|wanted| {
                wanted.eq_ignore_ascii_case(name)
                    || (wanted.eq_ignore_ascii_case("dragon longsword")
                        && name.eq_ignore_ascii_case("rune sword"))
                    || (wanted.eq_ignore_ascii_case("rune platebody")
                        && name.eq_ignore_ascii_case("rune chainbody"))
                    || metal_family_rank(wanted).is_some_and(|(wanted_rank, wanted_suffix)| {
                        metal_family_rank(name).is_some_and(|(rank, suffix)| {
                            rank <= wanted_rank && suffix.eq_ignore_ascii_case(wanted_suffix)
                        })
                    })
            });
        if relevant {
            resolved.push(BankItem {
                id: candidate.id,
                name: Arc::from(name),
            });
        }
    }
    Ok(Arc::new(LoadoutPlan {
        bank: cx.bank,
        memo_ids: Arc::from(cx.bank_items),
        row: row.clone(),
        resolved: Arc::from(resolved),
        allow_lower_tier: args.allow_lower_tier,
        strip: args.strip,
    }))
}

struct LoadoutPlan {
    bank: Option<api::named_banks::NamedBank>,
    memo_ids: Arc<[i32]>,
    row: crate::loadouts_store::Loadout,
    resolved: Arc<[BankItem]>,
    allow_lower_tier: bool,
    strip: bool,
}
impl StepPlan for LoadoutPlan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
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
                bank_actions.push(BankAction::Withdraw {
                    item: resolve(&carry.item)?,
                    qty: i32::try_from(carry.qty).unwrap_or(i32::MAX),
                });
            }
            for (slot, name) in &self.row.worn {
                let names = if self.allow_lower_tier {
                    crate::quester::loadouts::tier_candidates(slot, name, &available, facts)
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
                } else {
                    bank_actions.push(BankAction::Withdraw {
                        item: items[0].clone(),
                        qty: 1,
                    });
                }
                worn.push(Arc::from(items));
            }
            bank_actions.push(BankAction::Close);
        }
        Ok(Box::new(LoadoutRun {
            bank: if bank_actions.is_empty() {
                None
            } else {
                Some(BankRun {
                    bank: resolve_bank(self.bank, cx)?,
                    memo_ids: Arc::clone(&self.memo_ids),
                    actions: Arc::from(bank_actions),
                    partial_ok: false,
                    index: 0,
                    walk: None,
                    machine: None,
                    last: None,
                })
            },
            worn: Arc::from(worn),
            worn_index: 0,
            equipment: None,
            strip: self.strip,
            stripped: false,
            receipt: None,
        }))
    }
}

struct LoadoutRun {
    bank: Option<BankRun>,
    worn: Arc<[Arc<[BankItem]>]>,
    worn_index: usize,
    equipment: Option<ActionHandle<EquipmentMachine>>,
    strip: bool,
    stripped: bool,
    receipt: Option<Arc<dyn FamilyReceipt>>,
}
impl StepRun for LoadoutRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
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
        if let Some(handle) = &self.equipment {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(_)) => {
                    self.equipment = None;
                    if self.strip {
                        self.stripped = true;
                    } else {
                        self.worn_index += 1;
                    }
                }
            }
        }
        let request = loop {
            if self.strip && !self.stripped {
                break Some(EquipmentRequest::Strip);
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

pub fn compile_bank_known(
    args: &serde_json::Value,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    if !args.as_object().is_some_and(serde_json::Map::is_empty) {
        return Err(CompileError::code("invalid-args"));
    }
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BankHasArgs {
    obj: String,
    #[serde(default = "one")]
    qty: i32,
}

pub fn compile_bank_has(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let args: BankHasArgs =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
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

pub fn compile_loadout_ready(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let args: LoadoutArgs =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
    let qualified = if args.loadout.contains('/') {
        args.loadout
    } else {
        format!("{}/{}", cx.path_id.0, args.loadout)
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
        .values()
        .map(|wanted| {
            let wanted_item = named_item(cx, wanted)?;
            let wanted_family = metal_family_rank(wanted);
            let mut ids = vec![wanted_item.id];
            if args.allow_lower_tier {
                ids.extend(cx.selected.items().iter().filter_map(|candidate| {
                    let name = candidate.name.as_deref()?;
                    let (wanted_rank, wanted_suffix) = wanted_family?;
                    let (rank, suffix) = metal_family_rank(name)?;
                    (rank <= wanted_rank && suffix.eq_ignore_ascii_case(wanted_suffix))
                        .then_some(candidate.id)
                }));
                if wanted.eq_ignore_ascii_case("dragon longsword") {
                    if let Ok(item) = named_item(cx, "Rune sword") {
                        ids.push(item.id);
                    }
                }
                if wanted.eq_ignore_ascii_case("rune platebody") {
                    if let Ok(item) = named_item(cx, "Rune chainbody") {
                        ids.push(item.id);
                    }
                }
                ids.sort_unstable();
                ids.dedup();
            }
            Ok(Arc::from(ids))
        })
        .collect::<Result<Vec<_>, CompileError>>()?;
    Ok(Arc::new(LoadoutReady {
        carry: Arc::from(carry),
        worn: Arc::from(worn),
        strip: args.strip,
    }))
}

struct LoadoutReady {
    carry: Arc<[(i32, i32)]>,
    worn: Arc<[Arc<[i32]>]>,
    strip: bool,
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
            equipment.value.is_empty()
        } else {
            self.worn.iter().all(|ids| {
                equipment
                    .value
                    .iter()
                    .any(|item| ids.contains(&item.def.id))
            })
        };
        if carry_ready && worn_ready {
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
