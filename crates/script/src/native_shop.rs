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
use std::time::Duration;

const SETTLE_BOUND: Duration = Duration::from_secs(4);
const CLOSE_BOUND: Duration = Duration::from_secs(3);

#[derive(Debug, Clone)]
pub struct BuyRequest {
    pub npc_id: i32,
    pub npc_name: Arc<str>,
    pub item_id: i32,
    pub item_name: Arc<str>,
    /// Desired held count, not an unobserved click count.
    pub qty: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuyReceipt {
    pub held: i32,
}

enum Phase {
    /// The shared open core decides each tick.
    Open,
    /// The core asked for a nav walk to an unreachable keeper.
    Walk(Box<Walk>),
    Buy,
    AwaitBatch {
        before: i32,
    },
    Close,
    AwaitClose,
}

pub struct BuyMachine {
    request: BuyRequest,
    phase: Phase,
    open: OpenCore,
    deadline: Duration,
}

impl NativeMachine for BuyMachine {
    type Args = BuyRequest;
    type Output = BuyReceipt;

    fn begin(request: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        if request.qty < 1 {
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
        Ok(Self {
            request,
            phase: Phase::Open,
            open,
            deadline: Duration::ZERO,
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
                    let snapshot = cx.snapshot();
                    let Some(held) = snapshot.stock().held(self.request.item_id) else {
                        return Poll::Pending;
                    };
                    if held >= self.request.qty {
                        self.phase = Phase::Close;
                        continue;
                    }
                    let Some(shop) = snapshot.shop() else {
                        return Poll::Pending;
                    };
                    if !shop.value.open {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "shop closed during purchase",
                        ))));
                    }
                    let Some(row) = shop
                        .value
                        .stock
                        .iter()
                        .find(|row| row.def.id == self.request.item_id && row.count > 0)
                    else {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "shop lacks requested item",
                        ))));
                    };
                    let remaining = self.request.qty - held;
                    let chunk = if remaining >= 10 && row.count >= 10 {
                        10
                    } else if remaining >= 5 && row.count >= 5 {
                        5
                    } else {
                        1
                    };
                    let (id, slot, component) = (row.def.id, row.slot, row.component_id);
                    std::task::ready!(crate::native::defer_budget(cx.emit(
                        InteractReq::ShopButton {
                            kind: "buy".into(),
                            name: self.request.item_name.to_string(),
                            id,
                            slot,
                            component,
                            chunk,
                        },
                    )))?;
                    self.deadline = cx.active_now().saturating_add(SETTLE_BOUND);
                    self.phase = Phase::AwaitBatch { before: held };
                    return Poll::Pending;
                }
                &mut Phase::AwaitBatch { before } => {
                    let held = cx.snapshot().stock().held(self.request.item_id);
                    if held.is_some_and(|held| held > before) {
                        self.phase = Phase::Buy;
                        continue;
                    }
                    if cx.active_now() >= self.deadline {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "shop purchase did not settle",
                        ))));
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
                    self.deadline = cx.active_now().saturating_add(CLOSE_BOUND);
                    self.phase = Phase::AwaitClose;
                    return Poll::Pending;
                }
                Phase::AwaitClose => {
                    if !cx.snapshot().shop().is_some_and(|shop| shop.value.open) {
                        return Poll::Ready(Ok(self.receipt(cx)));
                    }
                    if cx.active_now() >= self.deadline {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "shop did not close",
                        ))));
                    }
                    return Poll::Pending;
                }
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
        BuyReceipt {
            held: cx
                .snapshot()
                .stock()
                .held(self.request.item_id)
                .unwrap_or(0),
        }
    }

    /// The shop NPC: rows matching the request's type or name, offered to
    /// the shared chooser with their `Trade`/`Trade-with` op. The footprint
    /// is the network tile the client's `tryMove` routes to.
    fn keeper(request: &BuyRequest, cx: &ActionContext<'_>) -> Option<Keeper> {
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
                npc.r#type == usize::try_from(request.npc_id).ok()
                    || npc
                        .name
                        .as_deref()
                        .is_some_and(|name| name.eq_ignore_ascii_case(&request.npc_name))
            })
            .map(|npc| shop_open::Candidate {
                name: npc.name.as_deref().unwrap_or(&request.npc_name),
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
    use crate::quester::families::tests::with_tick;
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
        BuyRequest {
            npc_id: 123,
            npc_name: Arc::from("Shopkeeper"),
            item_id: 4,
            item_name: Arc::from("Pot"),
            qty: 2,
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
    fn shop_npc_open_sends_one_trade_until_its_tick_deadline() {
        let client = client_with_shop(false);
        let snapshot = snapshot(&client, Vec::new(), vec![npc_row()]);
        let mut ledger = None;
        let handle = start_machine(&snapshot, &mut ledger);
        let trades = |ledger: &Option<Box<crate::native::ledger::Ledger>>| {
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
        };
        let first = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(first.is_pending());
        assert_eq!(trades(&ledger), 1);
        // No reach view is posted: the deadline is the straight-line
        // distance (2) plus the slack. No wall-clock bound fails it early.
        for tick in 3..=2 + 2 + shop_open::OPEN_SLACK_TICKS {
            let polled = with_tick(&snapshot, &mut ledger, tick, |t| {
                t.actions.poll(&handle, &mut t.cx)
            });
            assert!(polled.is_pending(), "tick {tick}");
            assert_eq!(trades(&ledger), 1, "no second Trade on tick {tick}");
        }
    }
}
