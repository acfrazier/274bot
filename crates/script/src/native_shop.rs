//! Snapshot-driven shop purchase machine shared by compiled cards.
//! Queued buttons are not receipts: each batch settles only after the held
//! item count grows.
use crate::native::{ActionContext, ActionError, NativeMachine};
use crate::shim::InteractReq;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

const OPEN_BOUND: Duration = Duration::from_secs(10);
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
    Open,
    AwaitOpen,
    Buy,
    AwaitBatch { before: i32 },
    Close,
    AwaitClose,
}

pub struct BuyMachine {
    request: BuyRequest,
    phase: Phase,
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
        Ok(Self {
            request,
            phase: Phase::Open,
            deadline: cx.active_now().saturating_add(OPEN_BOUND),
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        loop {
            match self.phase {
                Phase::Open => {
                    if cx.snapshot().shop().is_some_and(|shop| shop.value.open) {
                        self.phase = Phase::Buy;
                        continue;
                    }
                    let Some(npcs) = cx.snapshot().npcs() else {
                        return Poll::Pending;
                    };
                    let npc = npcs
                        .value
                        .iter()
                        .filter(|npc| {
                            npc.r#type == usize::try_from(self.request.npc_id).ok()
                                || npc.name.as_deref().is_some_and(|name| {
                                    name.eq_ignore_ascii_case(&self.request.npc_name)
                                })
                        })
                        .min_by_key(|npc| npc.distance);
                    let Some(npc) = npc else {
                        return Poll::Pending;
                    };
                    let trade = npc.actions.iter().flatten().find(|action| {
                        action.eq_ignore_ascii_case("Trade")
                            || action.eq_ignore_ascii_case("Trade-with")
                    });
                    let Some(trade) = trade else {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "shop NPC has no Trade action",
                        ))));
                    };
                    std::task::ready!(crate::native::defer_budget(
                        cx.emit(InteractReq::Npc {
                            name: npc
                                .name
                                .clone()
                                .unwrap_or_else(|| self.request.npc_name.to_string()),
                            action: trade.clone(),
                            index: i32::try_from(npc.index).ok(),
                        },)
                    ))?;
                    self.phase = Phase::AwaitOpen;
                    return Poll::Pending;
                }
                Phase::AwaitOpen => {
                    if cx.snapshot().shop().is_some_and(|shop| shop.value.open) {
                        self.phase = Phase::Buy;
                        continue;
                    }
                    if cx.active_now() >= self.deadline {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "shop did not open",
                        ))));
                    }
                    return Poll::Pending;
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
                Phase::AwaitBatch { before } => {
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

    fn cancel(&mut self) {}
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
}
