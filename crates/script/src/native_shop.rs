//! Snapshot-driven shop purchase machine shared by compiled cards.
//! Queued buttons are not receipts: each batch settles only after the held
//! item count grows. Opening the shop follows the rule shared with compat
//! `Shop.open` ([`crate::shop_open`]): one Trade per attempt with a
//! route-length tick deadline, and a nav walk to an unreachable keeper.
use crate::native::walk::Walk;
use crate::native::{ActionContext, ActionError, NativeMachine};
use crate::quester::families::reach::walk_request;
use crate::reach_route;
use crate::shim::InteractReq;
use crate::shop_open::{self, Decision, Facts, Keeper, OpenCore, Session};
use std::sync::Arc;
use std::task::Poll;

// Shop content adds no server delay. These are observation bounds.
// Engine L, 20 native acquisitions: maximum 1 tick from button dispatch to
// held-count change, plus 3 ticks of margin. See
// COMPAT-FINISH-029/impl/native-shop-measure/cargo-20-native-cells.log.
const ACQUISITION_SETTLE_TICKS: u64 = 4;
// Engine L, 20 byId samples: maximum 1 tick from button dispatch to inventory
// change, plus 3 ticks of margin. The call-to-change maximum was 2 ticks.
// See COMPAT-FINISH-029/impl/w3-live-rerun/report.md §4.2.
const BUY_BY_ID_SETTLE_TICKS: u64 = 4;
// Engine L, 20 native closes: maximum 1 tick from CloseModal dispatch to the
// closed snapshot, plus 3 ticks of margin. See
// COMPAT-FINISH-029/impl/native-shop-measure/cargo-20-native-cells.log.
const CLOSE_TICKS: u64 = 4;

#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
pub struct ShopBuyByIdArgs {
    #[serde(rename = "id")]
    pub item_id: f64,
    pub quantity: f64,
}

impl ShopBuyByIdArgs {
    pub fn exact_item_id(self) -> Option<i32> {
        (self.item_id.is_finite()
            && self.item_id.fract() == 0.0
            && self.item_id >= f64::from(i32::MIN)
            && self.item_id <= f64::from(i32::MAX))
        .then_some(self.item_id as i32)
    }
}

#[derive(Debug, Clone)]
pub enum BuyRequest {
    Acquisition {
        npc_id: i32,
        npc_name: Arc<str>,
        item_id: i32,
        item_name: Arc<str>,
        target_held: i32,
    },
    ById {
        item_id: i32,
        quantity: f64,
    },
}

impl BuyRequest {
    fn item_id(&self) -> i32 {
        match self {
            Self::Acquisition { item_id, .. } | Self::ById { item_id, .. } => *item_id,
        }
    }

    fn by_id(&self) -> bool {
        matches!(self, Self::ById { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuyReceipt {
    pub held: i32,
    pub bought: i32,
}

#[derive(Debug, Clone, Copy, Default)]
struct BatchFacts {
    held: i32,
    side: i32,
    stock: i32,
    coins: i32,
}

enum Phase {
    /// The shared open core decides each tick.
    Open,
    /// The core asked for a nav walk to an unreachable keeper.
    Walk(Box<Walk>),
    Buy,
    AwaitBatch {
        before: BatchFacts,
        sent_tick: u64,
        timeout_ticks: u64,
    },
    Close,
    AwaitClose,
    Done,
}

pub struct BuyMachine {
    request: BuyRequest,
    phase: Phase,
    open: OpenCore,
    deadline_tick: u64,
    baseline_held: i32,
    server_closes: u64,
}

impl NativeMachine for BuyMachine {
    type Args = BuyRequest;
    type Output = BuyReceipt;

    fn begin(request: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let baseline_held = cx.snapshot().stock().held(request.item_id()).unwrap_or(0);
        let server_closes = cx
            .snapshot()
            .shop()
            .map_or(0, |shop| shop.value.server_closes);
        let (phase, open) = match &request {
            BuyRequest::Acquisition { target_held, .. } => {
                if *target_held < 1 {
                    return Err(ActionError::Unavailable(Arc::from(
                        "buy quantity must be positive",
                    )));
                }
                // A modal already up with no usable page waits a tick for its page
                // before the core trades.
                let facts = open_facts(cx);
                let modal_up = cx.snapshot().shop().is_some_and(|shop| shop.value.open);
                let open = if modal_up && !facts.usable {
                    OpenCore::awaiting_page(&facts)
                } else {
                    OpenCore::default()
                };
                (Phase::Open, open)
            }
            BuyRequest::ById { quantity, .. } => {
                let phase = if *quantity > 0.0
                    && !quantity.is_nan()
                    && cx.snapshot().shop().is_some_and(|shop| shop.value.open)
                {
                    Phase::Buy
                } else {
                    Phase::Done
                };
                (phase, OpenCore::default())
            }
        };
        Ok(Self {
            request,
            phase,
            open,
            deadline_tick: 0,
            baseline_held,
            server_closes,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        loop {
            match &mut self.phase {
                Phase::Open => {
                    if cx.snapshot().npcs().is_none() {
                        // No NPC rows observed yet: nothing to choose from.
                        return Poll::Pending;
                    }
                    let facts = open_facts(cx);
                    let request = &self.request;
                    let decision = self.open.tick(&facts, || Self::keeper(request, cx));
                    match std::task::ready!(self.act(decision, &facts, cx))? {
                        true => {
                            self.phase = Phase::Buy;
                            continue;
                        }
                        false => return Poll::Pending,
                    }
                }
                Phase::Walk(walk) => {
                    let arrived = match walk.poll(cx) {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(Ok(receipt)) => receipt.into_arrival().is_ok(),
                        Poll::Ready(Err(_)) => false,
                    };
                    self.phase = Phase::Open;
                    let facts = open_facts(cx);
                    let request = &self.request;
                    let decision = self.open.walked(arrived, || Self::keeper(request, cx));
                    match std::task::ready!(self.act(decision, &facts, cx))? {
                        true => {
                            self.phase = Phase::Buy;
                            continue;
                        }
                        false => return Poll::Pending,
                    }
                }
                Phase::Buy => {
                    let item_id = self.request.item_id();
                    let by_id = self.request.by_id();
                    let snapshot = cx.snapshot();
                    let held = match snapshot.stock().held(item_id) {
                        Some(held) => held,
                        None if by_id => 0,
                        None => return Poll::Pending,
                    };
                    let complete = match &self.request {
                        BuyRequest::Acquisition { target_held, .. } => held >= *target_held,
                        BuyRequest::ById { quantity, .. } => {
                            f64::from(held.saturating_sub(self.baseline_held)) >= *quantity
                        }
                    };
                    if complete {
                        self.phase = if by_id { Phase::Done } else { Phase::Close };
                        continue;
                    }
                    let Some(shop) = snapshot.shop() else {
                        if by_id {
                            return Poll::Ready(Ok(self.receipt(cx)));
                        }
                        return Poll::Pending;
                    };
                    if !shop.value.open {
                        if by_id {
                            return Poll::Ready(Ok(self.receipt(cx)));
                        }
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "shop closed during purchase",
                        ))));
                    }
                    if by_id
                        && (shop.value.server_closes != self.server_closes
                            || !shop.value.player_available)
                    {
                        return Poll::Ready(Ok(self.receipt(cx)));
                    }
                    let row = if by_id {
                        shop.value.stock.iter().find(|row| row.def.id == item_id)
                    } else {
                        shop.value
                            .stock
                            .iter()
                            .find(|row| row.def.id == item_id && row.count > 0)
                    };
                    let Some(row) = row else {
                        if by_id {
                            return Poll::Ready(Ok(self.receipt(cx)));
                        }
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "shop lacks requested item",
                        ))));
                    };
                    if by_id && row.count == 0 {
                        return Poll::Ready(Ok(self.receipt(cx)));
                    }
                    let remaining = match &self.request {
                        BuyRequest::Acquisition { target_held, .. } => {
                            f64::from(target_held.saturating_sub(held))
                        }
                        BuyRequest::ById { quantity, .. } => {
                            *quantity - f64::from(held.saturating_sub(self.baseline_held))
                        }
                    };
                    if remaining <= 0.0 {
                        return Poll::Ready(Ok(self.receipt(cx)));
                    }
                    let chunk = if remaining >= 10.0 && row.count >= 10 {
                        10
                    } else if remaining >= 5.0 && row.count >= 5 {
                        5
                    } else {
                        1
                    };
                    let (id, slot, component) = (row.def.id, row.slot, row.component_id);
                    let name = match &self.request {
                        BuyRequest::Acquisition { item_name, .. } => item_name.to_string(),
                        BuyRequest::ById { .. } => row.def.name.clone().unwrap_or_default(),
                    };
                    let before = if by_id {
                        let Some(before) = Self::purchase_facts(cx, item_id) else {
                            return Poll::Ready(Ok(self.receipt(cx)));
                        };
                        before
                    } else {
                        BatchFacts {
                            held,
                            ..BatchFacts::default()
                        }
                    };
                    std::task::ready!(crate::native::defer_budget(cx.emit(
                        InteractReq::ShopButton {
                            kind: "buy".into(),
                            name,
                            id,
                            slot,
                            component,
                            chunk,
                        },
                    )))?;
                    let sent_tick = cx.evidence().tick;
                    let timeout_ticks = if by_id {
                        BUY_BY_ID_SETTLE_TICKS
                    } else {
                        ACQUISITION_SETTLE_TICKS
                    };
                    self.phase = Phase::AwaitBatch {
                        before,
                        sent_tick,
                        timeout_ticks,
                    };
                    return Poll::Pending;
                }
                Phase::AwaitBatch {
                    before,
                    sent_tick,
                    timeout_ticks,
                } => {
                    let item_id = self.request.item_id();
                    if self.request.by_id() {
                        let snapshot = cx.snapshot();
                        let Some(shop) = snapshot.shop() else {
                            return Poll::Ready(Ok(self.receipt(cx)));
                        };
                        if !shop.value.open
                            || !shop.value.player_available
                            || shop.value.server_closes != self.server_closes
                        {
                            return Poll::Ready(Ok(self.receipt(cx)));
                        }
                        if let Some(after) = Self::purchase_facts(cx, item_id) {
                            let item_delta = after.held.saturating_sub(before.held);
                            let side_delta = after.side.saturating_sub(before.side);
                            let stock_delta = before.stock.saturating_sub(after.stock);
                            if item_delta > 0
                                && item_delta == side_delta
                                && item_delta == stock_delta
                                && after.coins <= before.coins
                            {
                                self.phase = Phase::Buy;
                                continue;
                            }
                        }
                        if cx.evidence().tick.saturating_sub(*sent_tick) >= *timeout_ticks {
                            return Poll::Ready(Ok(self.receipt(cx)));
                        }
                    } else {
                        let held = cx.snapshot().stock().held(item_id);
                        if held.is_some_and(|held| held > before.held) {
                            self.phase = Phase::Buy;
                            continue;
                        }
                        if cx.evidence().tick.saturating_sub(*sent_tick) >= *timeout_ticks {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "shop purchase did not settle",
                            ))));
                        }
                    }
                    return Poll::Pending;
                }
                Phase::Close => {
                    if !cx.snapshot().shop().is_some_and(|shop| shop.value.open) {
                        return Poll::Ready(Ok(self.receipt(cx)));
                    }
                    std::task::ready!(crate::native::defer_budget(
                        cx.emit(InteractReq::CloseModal)
                    ))?;
                    self.deadline_tick = cx.evidence().tick.saturating_add(CLOSE_TICKS);
                    self.phase = Phase::AwaitClose;
                    return Poll::Pending;
                }
                Phase::AwaitClose => {
                    if !cx.snapshot().shop().is_some_and(|shop| shop.value.open) {
                        return Poll::Ready(Ok(self.receipt(cx)));
                    }
                    if cx.evidence().tick >= self.deadline_tick {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "shop did not close",
                        ))));
                    }
                    return Poll::Pending;
                }
                Phase::Done => return Poll::Ready(Ok(self.receipt(cx))),
            }
        }
    }

    fn cancel(&mut self) {
        if let Phase::Walk(walk) = &mut self.phase {
            walk.cancel();
        }
    }
}

impl BuyMachine {
    fn receipt(&self, cx: &ActionContext<'_>) -> BuyReceipt {
        let held = cx
            .snapshot()
            .stock()
            .held(self.request.item_id())
            .unwrap_or(0);
        BuyReceipt {
            held,
            bought: held.saturating_sub(self.baseline_held).max(0),
        }
    }

    fn purchase_facts(cx: &ActionContext<'_>, item_id: i32) -> Option<BatchFacts> {
        let snapshot = cx.snapshot();
        let shop = snapshot.shop()?;
        if !shop.value.open || !shop.value.player_available {
            return None;
        }
        let side = shop
            .value
            .player
            .iter()
            .filter(|row| row.def.id == item_id)
            .map(|row| row.count)
            .fold(0_i32, i32::saturating_add);
        let stock = shop
            .value
            .stock
            .iter()
            .filter(|row| row.def.id == item_id)
            .map(|row| row.count)
            .fold(0_i32, i32::saturating_add);
        Some(BatchFacts {
            held: snapshot.stock().held(item_id).unwrap_or(0),
            side,
            stock,
            coins: snapshot.stock().held(995).unwrap_or(0),
        })
    }

    /// The shop NPC chooser is used only by acquisition requests.
    fn keeper(request: &BuyRequest, cx: &ActionContext<'_>) -> Option<Keeper> {
        let BuyRequest::Acquisition {
            npc_id, npc_name, ..
        } = request
        else {
            return None;
        };
        let snapshot = cx.snapshot();
        let npcs = snapshot.npcs()?;
        let here = snapshot
            .local_player()
            .map(|player| player.value.player.network);
        let view = reach_route::live(snapshot.reach().map(|reach| reach.value), here);
        let candidates = npcs
            .value
            .iter()
            .filter(|npc| {
                npc.r#type == usize::try_from(*npc_id).ok()
                    || npc
                        .name
                        .as_deref()
                        .is_some_and(|name| name.eq_ignore_ascii_case(npc_name))
            })
            .map(|npc| shop_open::Candidate {
                name: npc.name.as_deref().unwrap_or(npc_name),
                action: npc
                    .actions
                    .iter()
                    .flatten()
                    .find(|op| shop_open::is_trade_op(op))
                    .map(String::as_str),
                index: i32::try_from(npc.index).unwrap_or(-1),
                tile: if npc.size >= 1 { npc.network } else { npc.tile },
                size: npc.size,
                distance: npc.distance,
            });
        shop_open::choose_keeper(candidates, view.zip(here))
    }

    /// Carry out one core decision: `Ready(Ok(true))` when the shop is open.
    fn act(
        &mut self,
        decision: Decision,
        facts: &Facts,
        cx: &mut ActionContext<'_>,
    ) -> Poll<Result<bool, ActionError>> {
        match decision {
            Decision::Wait => Poll::Ready(Ok(false)),
            Decision::Opened => Poll::Ready(Ok(true)),
            Decision::Failed(reason) => Poll::Ready(Err(ActionError::Failed(Arc::from(reason)))),
            Decision::Trade(keeper) => {
                std::task::ready!(crate::native::defer_budget(cx.emit(InteractReq::Npc {
                    name: keeper.name.clone(),
                    action: keeper.action.clone(),
                    index: Some(keeper.index),
                })))?;
                self.open.traded(facts, &keeper);
                Poll::Ready(Ok(false))
            }
            Decision::Walk(tile) => {
                let walk = Walk::begin(walk_request(tile, 1, None, cx.evidence()), cx)?;
                self.phase = Phase::Walk(Box::new(walk));
                Poll::Ready(Ok(false))
            }
        }
    }
}

/// What the shared open core reads from a native snapshot. The session is
/// the run key ([`api::selected::RunKey`]): a relog is a new session.
fn open_facts(cx: &ActionContext<'_>) -> Facts {
    let evidence = cx.evidence();
    let snapshot = cx.snapshot();
    let (chat_seq, cant_reach_seq) = snapshot.chat_lines(0).map_or((0, 0), |lines| {
        lines.value.iter().fold((0, 0), |(all, refused), line| {
            let refused_here =
                line.username.is_none() && line.type_ == 0 && shop_open::is_cant_reach(&line.text);
            (
                all.max(line.sequence),
                if refused_here {
                    refused.max(line.sequence)
                } else {
                    refused
                },
            )
        })
    });
    Facts {
        tick: Some(evidence.tick),
        session: Session {
            epoch: evidence.run.run,
            login: evidence.run.session,
        },
        chat_seq,
        cant_reach_seq,
        usable: snapshot
            .shop()
            .is_some_and(|shop| shop.value.open && !shop.value.stock.is_empty()),
        trade_taken: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::{ActionContext, HostEffect, InteractionReceipt};
    use crate::quester::families::tests::{local_player, with_tick, with_tick_reach};
    use api::snapshot::{
        GameSnapshot, ItemActionFamily, ItemContainer, ItemView, NpcView, WorldTile,
    };
    use client::client::{Client, ClientConfig};
    use client::config::if_type::{ComponentType, IfType, IfTypeMut};
    use client::io::ServerProt;

    fn client_with_shop(open: bool) -> Client {
        let mut client = Client::new(ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp/274bot-no-shop-cache".into(),
            members: true,
            lowmem: false,
        });
        let cache = Arc::get_mut(&mut client.cache).unwrap();
        cache.objs.resize(5, client::config::ObjType::default());
        cache.objs[4] = client::config::ObjType {
            id: 4,
            name: "Pot".into(),
            ..Default::default()
        };
        client.set_iface(
            3824,
            IfType {
                id: 3824,
                layer_id: 3824,
                r#type: ComponentType::TYPE_LAYER,
                ..Default::default()
            },
        );
        client.set_iface(
            3900,
            IfType {
                id: 3900,
                layer_id: 3824,
                r#type: ComponentType::TYPE_INV,
                iop: [
                    Some("Value".into()),
                    Some("Buy 1".into()),
                    Some("Buy 5".into()),
                    Some("Buy 10".into()),
                    None,
                ],
                ..Default::default()
            },
        );
        client.set_iface_mut(
            3900,
            IfTypeMut {
                link_obj_type: Some(vec![5, 0]),
                link_obj_number: Some(vec![10, 0]),
                ..Default::default()
            },
        );
        client.set_iface(
            3822,
            IfType {
                id: 3822,
                layer_id: 3822,
                r#type: ComponentType::TYPE_LAYER,
                ..Default::default()
            },
        );
        client.set_iface(
            3823,
            IfType {
                id: 3823,
                layer_id: 3822,
                r#type: ComponentType::TYPE_INV,
                ..Default::default()
            },
        );
        client.set_iface_mut(
            3823,
            IfTypeMut {
                link_obj_type: Some(vec![0]),
                link_obj_number: Some(vec![0]),
                ..Default::default()
            },
        );
        client.main_modal_id = if open { 3824 } else { -1 };
        client.side_modal_id = if open { 3822 } else { -1 };
        client.bump_gens(ServerProt::IF_OPENMAIN);
        client
    }
    fn encode_rows(rows: &[(i32, i32)]) -> (Vec<i32>, Vec<i32>) {
        let mut ids = Vec::with_capacity(rows.len() + 1);
        let mut counts = Vec::with_capacity(rows.len() + 1);
        for (id, count) in rows {
            ids.push(*id + 1);
            counts.push(*count);
        }
        ids.push(0);
        counts.push(0);
        (ids, counts)
    }

    fn client_with_shop_rows(open: bool, stock: &[(i32, i32)], player: &[(i32, i32)]) -> Client {
        let mut client = client_with_shop(open);
        let max_id = stock
            .iter()
            .chain(player)
            .map(|(id, _)| *id)
            .max()
            .unwrap_or(4)
            .max(4);
        let cache = Arc::get_mut(&mut client.cache).unwrap();
        cache
            .objs
            .resize(max_id as usize + 1, client::config::ObjType::default());
        for (id, _) in stock.iter().chain(player) {
            cache.objs[*id as usize] = client::config::ObjType {
                id: *id,
                name: "Pot".into(),
                ..Default::default()
            };
        }
        let (stock_ids, stock_counts) = encode_rows(stock);
        client.set_iface_mut(
            3900,
            IfTypeMut {
                link_obj_type: Some(stock_ids),
                link_obj_number: Some(stock_counts),
                ..Default::default()
            },
        );
        let (player_ids, player_counts) = encode_rows(player);
        client.set_iface_mut(
            3823,
            IfTypeMut {
                link_obj_type: Some(player_ids),
                link_obj_number: Some(player_counts),
                ..Default::default()
            },
        );
        client
    }

    fn item(id: i32, name: &str, count: i32) -> ItemView {
        ItemView {
            def: api::obj_names::ItemDefView {
                id,
                name: Some(name.to_owned()),
                stackable: false,
                members: false,
                base_value: 0,
                noted: false,
                certificate_link: -1,
                certificate_template: -1,
            },
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count,
            actions: Vec::new(),
            component_id: 3823,
        }
    }

    fn snapshot(client: &Client, inventory: Vec<ItemView>, npcs: Vec<NpcView>) -> GameSnapshot {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.rebuild_family(client, api::snapshot::Family::Shop);
        snapshot.seed_inventory(inventory, 28);
        snapshot.seed_npcs(npcs);
        snapshot
    }

    fn npc_row() -> NpcView {
        let tile = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        NpcView {
            index: 7,
            r#type: Some(123),
            name: Some("Shopkeeper".to_owned()),
            actions: vec![Some("Trade".to_owned())],
            tile,
            distance: 2,
            animation: -1,
            animation_frame: 0,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: -1,
            health: 0,
            total_health: 0,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
            level: 0,
            size: 1,
            network: tile,
            x: tile.x,
            z: tile.z,
            yaw: 0,
        }
    }

    fn request() -> BuyRequest {
        BuyRequest::Acquisition {
            npc_id: 123,
            npc_name: Arc::from("Shopkeeper"),
            item_id: 4,
            item_name: Arc::from("Pot"),
            target_held: 2,
        }
    }

    fn spend_interaction(cx: &mut ActionContext<'_>) {
        cx.action_id = cx.ledger.as_ref().unwrap().owner.as_ref().unwrap().id.get();
        let request_id = cx
            .emit(InteractReq::ContinueDialog { component_id: None })
            .unwrap();
        let evidence = cx.evidence();
        let ledger = cx.ledger.as_mut().unwrap();
        let action = ledger.outbox.pop().expect("budget-filling interaction");
        assert_eq!(action.request_id.get(), request_id);
        ledger.complete_interaction(
            &action.authority(),
            InteractionReceipt {
                request_id,
                evidence,
                accepted: true,
                chat_since: 0,
            },
        );
    }

    fn start_machine(
        snapshot: &GameSnapshot,
        ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
    ) -> crate::native::ActionHandle<BuyMachine> {
        with_tick(snapshot, ledger, 1, |tick| {
            tick.actions
                .begin::<BuyMachine>(request(), &mut tick.cx)
                .unwrap()
        })
    }
    fn start_by_id_machine(
        snapshot: &GameSnapshot,
        ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
        quantity: f64,
    ) -> crate::native::ActionHandle<BuyMachine> {
        with_tick(snapshot, ledger, 1, |tick| {
            tick.actions
                .begin::<BuyMachine>(
                    BuyRequest::ById {
                        item_id: 4,
                        quantity,
                    },
                    &mut tick.cx,
                )
                .unwrap()
        })
    }

    fn assert_deferred(
        snapshot: &GameSnapshot,
        handle: &crate::native::ActionHandle<BuyMachine>,
        ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
        expected: impl FnOnce(&HostEffect),
    ) {
        let denied = with_tick(snapshot, ledger, 2, |tick| {
            spend_interaction(&mut tick.cx);
            tick.actions.poll(handle, &mut tick.cx)
        });
        assert!(denied.is_pending(), "budget denial must remain Pending");
        assert!(ledger.as_ref().unwrap().outbox.is_empty());

        let retry = with_tick(snapshot, ledger, 3, |tick| {
            tick.actions.poll(handle, &mut tick.cx)
        });
        assert!(retry.is_pending());
        expected(&ledger.as_ref().unwrap().outbox[0].effect);
    }

    #[test]
    fn shop_npc_open_budget_denial_retries_next_tick() {
        let client = client_with_shop(false);
        let snapshot = snapshot(&client, Vec::new(), vec![npc_row()]);
        let mut ledger = None;
        let handle = start_machine(&snapshot, &mut ledger);

        assert_deferred(&snapshot, &handle, &mut ledger, |effect| {
            assert!(matches!(
                effect,
                HostEffect::Interaction(InteractReq::Npc {
                    name,
                    action,
                    index: Some(7),
                }) if name == "Shopkeeper" && action == "Trade"
            ));
        });
    }

    #[test]
    fn shop_buy_button_budget_denial_retries_next_tick() {
        let client = client_with_shop(true);
        let snapshot = snapshot(&client, Vec::new(), Vec::new());
        let mut ledger = None;
        let handle = start_machine(&snapshot, &mut ledger);

        assert_deferred(&snapshot, &handle, &mut ledger, |effect| {
            assert!(matches!(
                effect,
                HostEffect::Interaction(InteractReq::ShopButton {
                    kind,
                    name,
                    id: 4,
                    slot: 0,
                    component: 3900,
                    chunk: 1,
                }) if kind == "buy" && name == "Pot"
            ));
        });
    }

    #[test]
    fn shop_close_budget_denial_retries_next_tick() {
        let client = client_with_shop(true);
        let snapshot = snapshot(&client, vec![item(4, "Pot", 2)], Vec::new());
        let mut ledger = None;
        let handle = start_machine(&snapshot, &mut ledger);

        assert_deferred(&snapshot, &handle, &mut ledger, |effect| {
            assert!(matches!(
                effect,
                HostEffect::Interaction(InteractReq::CloseModal)
            ));
        });
    }
    #[test]
    fn buy_by_id_uses_exact_id_and_waits_for_each_matching_receipt() {
        let before_client = client_with_shop_rows(true, &[(5, 1), (4, 2), (4, 3)], &[]);
        let before = snapshot(&before_client, vec![item(995, "Coins", 10)], Vec::new());
        let rows: Vec<_> = before
            .shop()
            .stock
            .iter()
            .map(|row| (row.def.id, row.slot, row.count))
            .collect();
        assert_eq!(rows, vec![(5, 0, 1), (4, 1, 2), (4, 2, 3)]);

        let mut ledger = None;
        let handle = start_by_id_machine(&before, &mut ledger, 2.0);
        let first = with_tick(&before, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(first.is_pending());
        assert_eq!(buttons(&ledger), 1);
        assert!(matches!(
            &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
            HostEffect::Interaction(InteractReq::ShopButton {
                kind,
                name,
                id: 4,
                slot: 1,
                component: 3900,
                chunk: 1,
            }) if kind == "buy" && name == "Pot"
        ));
        sent(&mut ledger, 2);

        let held_only_client = client_with_shop_rows(true, &[(5, 1), (4, 2), (4, 3)], &[]);
        let held_only = snapshot(
            &held_only_client,
            vec![item(995, "Coins", 10), item(4, "Pot", 1)],
            Vec::new(),
        );
        let held_only_result = with_tick(&held_only, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            held_only_result.is_pending(),
            "held delta alone is not a receipt"
        );
        assert_eq!(buttons(&ledger), 1);

        let side_only_client = client_with_shop_rows(true, &[(5, 1), (4, 2), (4, 3)], &[(4, 1)]);
        let side_only = snapshot(
            &side_only_client,
            vec![item(995, "Coins", 10), item(4, "Pot", 1)],
            Vec::new(),
        );
        let side_only_result = with_tick(&side_only, &mut ledger, 4, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            side_only_result.is_pending(),
            "held and side-pack deltas without stock loss are not a receipt"
        );
        assert_eq!(buttons(&ledger), 1);

        let coin_mismatch_client =
            client_with_shop_rows(true, &[(5, 1), (4, 1), (4, 3)], &[(4, 1)]);
        let coin_mismatch = snapshot(
            &coin_mismatch_client,
            vec![item(995, "Coins", 11), item(4, "Pot", 1)],
            Vec::new(),
        );
        let coin_mismatch_result = with_tick(&coin_mismatch, &mut ledger, 5, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            coin_mismatch_result.is_pending(),
            "a coin increase rejects otherwise matching item, side-pack, and stock deltas"
        );
        assert_eq!(buttons(&ledger), 1);

        let matched_client = client_with_shop_rows(true, &[(5, 1), (4, 1), (4, 3)], &[(4, 1)]);
        let matched = snapshot(
            &matched_client,
            vec![item(995, "Coins", 9), item(4, "Pot", 1)],
            Vec::new(),
        );
        let second = with_tick(&matched, &mut ledger, 6, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(second.is_pending());
        assert_eq!(buttons(&ledger), 2);
        assert!(matches!(
            &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
            HostEffect::Interaction(InteractReq::ShopButton {
                kind,
                name,
                id: 4,
                slot: 1,
                component: 3900,
                chunk: 1,
            }) if kind == "buy" && name == "Pot"
        ));
        sent(&mut ledger, 6);

        let after_two_client = client_with_shop_rows(true, &[(5, 1), (4, 0), (4, 3)], &[(4, 2)]);
        let after_two = snapshot(
            &after_two_client,
            vec![item(995, "Coins", 8), item(4, "Pot", 2)],
            Vec::new(),
        );
        let pending_buttons = buttons(&ledger);
        let result = with_tick(&after_two, &mut ledger, 7, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert_eq!(
            result,
            Poll::Ready(Ok(BuyReceipt { held: 2, bought: 2 })),
            "matching held, side-pack, stock, and coin facts settle the actual count"
        );
        assert_eq!(buttons(&ledger), pending_buttons);
    }

    #[test]
    fn buy_by_id_rounds_a_positive_fraction_up() {
        let before_client = client_with_shop_rows(true, &[(4, 2)], &[]);
        let before = snapshot(&before_client, vec![item(995, "Coins", 10)], Vec::new());
        let mut ledger = None;
        let handle = start_by_id_machine(&before, &mut ledger, 1.5);
        let first = with_tick(&before, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(first.is_pending());
        assert_eq!(buttons(&ledger), 1);
        sent(&mut ledger, 2);

        let after_first_client = client_with_shop_rows(true, &[(4, 1)], &[(4, 1)]);
        let after_first = snapshot(
            &after_first_client,
            vec![item(995, "Coins", 9), item(4, "Pot", 1)],
            Vec::new(),
        );
        let second = with_tick(&after_first, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            second.is_pending(),
            "a positive fractional quantity still requests its final unit"
        );
        assert_eq!(buttons(&ledger), 2);
        sent(&mut ledger, 3);

        let after_second_client = client_with_shop_rows(true, &[(4, 0)], &[(4, 2)]);
        let after_second = snapshot(
            &after_second_client,
            vec![item(995, "Coins", 8), item(4, "Pot", 2)],
            Vec::new(),
        );
        let settled = with_tick(&after_second, &mut ledger, 4, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert_eq!(settled, Poll::Ready(Ok(BuyReceipt { held: 2, bought: 2 })));
        assert_eq!(buttons(&ledger), 2);
    }

    #[test]
    fn buy_by_id_stops_at_the_first_depleted_exact_id_row() {
        let client = client_with_shop_rows(true, &[(5, 1), (4, 0), (4, 3)], &[]);
        let snapshot = snapshot(&client, vec![item(995, "Coins", 10)], Vec::new());
        let mut ledger = None;
        let handle = start_by_id_machine(&snapshot, &mut ledger, 2.0);
        let result = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert_eq!(
            result,
            Poll::Ready(Ok(BuyReceipt { held: 0, bought: 0 })),
            "later duplicate stock does not replace the first exact-ID row"
        );
        assert_eq!(buttons(&ledger), 0);
    }

    fn trades(ledger: &Option<Box<crate::native::ledger::Ledger>>) -> usize {
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .filter(|action| {
                matches!(
                    &action.effect,
                    HostEffect::Interaction(InteractReq::Npc { action, .. }) if action == "Trade"
                )
            })
            .count()
    }

    fn buttons(ledger: &Option<Box<crate::native::ledger::Ledger>>) -> usize {
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .filter(|action| {
                matches!(
                    &action.effect,
                    HostEffect::Interaction(InteractReq::ShopButton { .. })
                )
            })
            .count()
    }

    /// The host sent the last queued interaction.
    fn sent(ledger: &mut Option<Box<crate::native::ledger::Ledger>>, tick: u64) {
        let ledger = ledger.as_mut().unwrap();
        let authority = ledger.outbox.last().unwrap().authority();
        ledger.complete_interaction(
            &authority,
            InteractionReceipt {
                request_id: authority.request_id().get(),
                evidence: api::quest_progress::EvidenceStamp {
                    run: authority.run(),
                    tick,
                    sequence: tick,
                },
                accepted: true,
                chat_since: 0,
            },
        );
    }

    /// A reach view round the keeper at (3200, 3200) with every step open,
    /// flooded from `origin` (rank 0 there, nothing else ranked).
    fn reach_from(origin: WorldTile) -> api::query::ReachQueryView {
        const SIDE: i32 = 31;
        let (base_x, base_z) = (3185, 3185);
        let n = (SIDE * SIDE) as usize;
        let i = ((origin.x - base_x) * SIDE + (origin.z - base_z)) as usize;
        let mut reachable = vec![0u32; n.div_ceil(32)];
        reachable[i / 32] |= 1 << (i % 32);
        let mut exact_rank = vec![u16::MAX; n];
        exact_rank[i] = 0;
        api::query::ReachQueryView {
            available: true,
            base_x,
            base_z,
            level: 0,
            width: SIDE,
            height: SIDE,
            walkable: vec![u32::MAX; n.div_ceil(32)],
            reachable: reachable.clone(),
            reachable_adj: reachable,
            exact_rank: exact_rank.clone(),
            adjacent_rank: exact_rank,
            step: vec![u8::MAX; n],
            canlight: Vec::new(),
        }
    }

    /// Poll one Trade attempt to its end: the tick of the next Trade.
    fn next_trade_tick(
        snapshot: &GameSnapshot,
        reach: Option<&api::query::ReachQueryView>,
        handle: &crate::native::ActionHandle<BuyMachine>,
        ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
        from: u64,
    ) -> u64 {
        let before = trades(ledger);
        for tick in from..from + 2 * shop_open::OPEN_CAP_TICKS {
            let polled = match reach {
                Some(reach) => with_tick_reach(snapshot, reach, ledger, tick, |t| {
                    t.actions.poll(handle, &mut t.cx)
                }),
                None => with_tick(snapshot, ledger, tick, |t| {
                    t.actions.poll(handle, &mut t.cx)
                }),
            };
            assert!(polled.is_pending(), "tick {tick}");
            if trades(ledger) > before {
                assert_eq!(trades(ledger), before + 1, "one Trade on tick {tick}");
                sent(ledger, tick);
                return tick;
            }
        }
        panic!("no Trade after tick {from}");
    }

    /// No reach view is posted: the straight-line distance (2) is no bound
    /// on the walk, so the attempt waits the cap. No wall-clock bound fails
    /// it early.
    #[test]
    fn shop_npc_open_without_a_reach_view_waits_the_cap() {
        let client = client_with_shop(false);
        let mut snapshot = snapshot(&client, Vec::new(), vec![npc_row()]);
        snapshot.seed_local_player(local_player(WorldTile {
            x: 3198,
            z: 3200,
            level: 0,
        }));
        let mut ledger = None;
        let handle = start_machine(&snapshot, &mut ledger);
        let first = next_trade_tick(&snapshot, None, &handle, &mut ledger, 2);
        assert_eq!(first, 2);
        let second = next_trade_tick(&snapshot, None, &handle, &mut ledger, 3);
        assert_eq!(second, first + shop_open::OPEN_CAP_TICKS + 1);
    }

    /// A posted view flooded from another tile (stale: the player moved
    /// since) is not a measured route either: the cap. The same view from
    /// the player's own tile measures one step, and that attempt ends at
    /// Trade + 1 + slack.
    #[test]
    fn shop_npc_open_with_a_stale_reach_view_waits_the_cap() {
        let here = WorldTile {
            x: 3198,
            z: 3200,
            level: 0,
        };
        let client = client_with_shop(false);
        let mut snapshot = snapshot(&client, Vec::new(), vec![npc_row()]);
        snapshot.seed_local_player(local_player(here));

        let stale = reach_from(WorldTile { x: 3190, ..here });
        let mut ledger = None;
        let handle = start_machine(&snapshot, &mut ledger);
        let first = next_trade_tick(&snapshot, Some(&stale), &handle, &mut ledger, 2);
        let second = next_trade_tick(&snapshot, Some(&stale), &handle, &mut ledger, first + 1);
        assert_eq!(second, first + shop_open::OPEN_CAP_TICKS + 1);

        let live = reach_from(here);
        let mut ledger = None;
        let handle = start_machine(&snapshot, &mut ledger);
        let first = next_trade_tick(&snapshot, Some(&live), &handle, &mut ledger, 2);
        let second = next_trade_tick(&snapshot, Some(&live), &handle, &mut ledger, first + 1);
        assert_eq!(second, first + 1 + shop_open::OPEN_SLACK_TICKS + 1);
    }

    /// An `if_close` (an NPC's hit, `npc_combat_melee.rs2:31,36`) shuts the
    /// shop under an outstanding buy batch. No button goes to the closed
    /// shop, and the purchase ends with an error: at once when the batch
    /// landed, within the settle bound when nothing moved.
    #[test]
    fn a_shop_closed_mid_batch_ends_the_purchase_without_another_button() {
        for landed in [false, true] {
            let open = client_with_shop(true);
            let closed = client_with_shop(false);
            let before = snapshot(&open, Vec::new(), Vec::new());
            let held = if landed {
                vec![item(4, "Pot", 1)]
            } else {
                Vec::new()
            };
            let after = snapshot(&closed, held, Vec::new());
            let mut ledger = None;
            let handle = start_machine(&before, &mut ledger);
            let polled = with_tick(&before, &mut ledger, 2, |t| {
                t.actions.poll(&handle, &mut t.cx)
            });
            assert!(polled.is_pending());
            assert_eq!(buttons(&ledger), 1, "the first batch");
            sent(&mut ledger, 2);
            let mut tick = 3;
            let result = loop {
                let polled = with_tick(&after, &mut ledger, tick, |t| {
                    t.actions.poll(&handle, &mut t.cx)
                });
                assert_eq!(
                    buttons(&ledger),
                    1,
                    "no button to a closed shop, tick {tick}"
                );
                if let Poll::Ready(result) = polled {
                    break result;
                }
                tick += 1;
                assert!(tick < 20, "the purchase must end");
            };
            let Err(ActionError::Failed(reason)) = result else {
                panic!("expected a failure, got {result:?}");
            };
            if landed {
                assert_eq!(tick, 3);
                assert_eq!(&*reason, "shop closed during purchase");
            } else {
                assert_eq!(&*reason, "shop purchase did not settle");
            }
        }
    }
}
