//! Compat `Shop.buyById` machine. The script opened the shop; this machine
//! only presses Buy on the open page and settles each button on matching
//! held, side-pack, stock and coin facts. It has no keeper, so it cannot use
//! the acquisition machine's ordered Trade roundtrip ([`crate::acquire::buy`]);
//! both share [`crate::acquire::buy::buy_chunk`] for the 1/5/10 batch choice.
use crate::acquire::buy::buy_chunk;
use crate::interact::InteractReq;
use crate::native::{ActionContext, ActionError, NativeMachine};
use std::task::Poll;

// Engine L `shop.rs2:23-25,59-98,168-181` checks live price, stock, coins and
// inventory space per request, then calls the item-transfer routine; these
// paths define no fixed server-delay wait. This value is an observation bound.
// Engine L, 20 byId samples: maximum 1 tick from button dispatch to inventory
// change, plus 3 ticks of margin. The call-to-change maximum was 2 ticks.
// See COMPAT-FINISH-029/impl/w3-live-rerun/report.md §4.2.
const BUY_BY_ID_SETTLE_TICKS: u64 = 4;

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

/// One exact item id and a positive unit quantity, counted from the held
/// count at `begin`.
#[derive(Debug, Clone, Copy)]
pub struct BuyRequest {
    pub item_id: i32,
    pub quantity: f64,
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
    Buy,
    AwaitBatch { before: BatchFacts, sent_tick: u64 },
    Done,
}

pub struct BuyMachine {
    request: BuyRequest,
    phase: Phase,
    baseline_held: i32,
    server_closes: u64,
}

impl NativeMachine for BuyMachine {
    type Args = BuyRequest;
    type Output = BuyReceipt;

    fn begin(request: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let baseline_held = cx.snapshot().stock().held(request.item_id).unwrap_or(0);
        let server_closes = cx
            .snapshot()
            .shop()
            .map_or(0, |shop| shop.value.server_closes);
        let phase = if request.quantity > 0.0
            && !request.quantity.is_nan()
            && cx.snapshot().shop().is_some_and(|shop| shop.value.open)
        {
            Phase::Buy
        } else {
            Phase::Done
        };
        Ok(Self {
            request,
            phase,
            baseline_held,
            server_closes,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        loop {
            match &mut self.phase {
                Phase::Buy => {
                    let item_id = self.request.item_id;
                    let snapshot = cx.snapshot();
                    let held = snapshot.stock().held(item_id).unwrap_or(0);
                    let bought = f64::from(held.saturating_sub(self.baseline_held));
                    if bought >= self.request.quantity {
                        self.phase = Phase::Done;
                        continue;
                    }
                    let Some(shop) = snapshot.shop() else {
                        return Poll::Ready(Ok(self.receipt(cx)));
                    };
                    if !shop.value.open
                        || shop.value.server_closes != self.server_closes
                        || !shop.value.player_available
                    {
                        return Poll::Ready(Ok(self.receipt(cx)));
                    }
                    // Only the first exact-id row counts: later duplicate
                    // stock does not replace a depleted first row.
                    let Some(row) = shop
                        .value
                        .stock
                        .iter()
                        .find(|row| row.def.id == item_id)
                        .filter(|row| row.count > 0)
                    else {
                        return Poll::Ready(Ok(self.receipt(cx)));
                    };
                    let chunk = buy_chunk(self.request.quantity - bought, row.count);
                    let (id, slot, component) = (row.def.id, row.slot, row.component_id);
                    let name = row.def.name.clone().unwrap_or_default();
                    let Some(before) = Self::purchase_facts(cx, item_id) else {
                        return Poll::Ready(Ok(self.receipt(cx)));
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
                    self.phase = Phase::AwaitBatch {
                        before,
                        sent_tick: cx.evidence().tick,
                    };
                    return Poll::Pending;
                }
                Phase::AwaitBatch { before, sent_tick } => {
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
                    if let Some(after) = Self::purchase_facts(cx, self.request.item_id) {
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
                    if cx.evidence().tick.saturating_sub(*sent_tick) >= BUY_BY_ID_SETTLE_TICKS {
                        return Poll::Ready(Ok(self.receipt(cx)));
                    }
                    return Poll::Pending;
                }
                Phase::Done => return Poll::Ready(Ok(self.receipt(cx))),
            }
        }
    }

    fn cancel(&mut self) {}
}

impl BuyMachine {
    fn receipt(&self, cx: &ActionContext<'_>) -> BuyReceipt {
        let held = cx
            .snapshot()
            .stock()
            .held(self.request.item_id)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::{HostEffect, InteractionReceipt};
    use crate::quester::families::tests::with_tick;
    use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer, ItemView, NpcView};
    use client::client::{Client, ClientConfig};
    use client::config::if_type::{ComponentType, IfType, IfTypeMut};
    use client::io::ServerProt;
    use std::sync::Arc;

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

    fn start_by_id_machine(
        snapshot: &GameSnapshot,
        ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
        quantity: f64,
    ) -> crate::native::ActionHandle<BuyMachine> {
        with_tick(snapshot, ledger, 1, |tick| {
            tick.actions
                .begin::<BuyMachine>(
                    BuyRequest {
                        item_id: 4,
                        quantity,
                    },
                    &mut tick.cx,
                )
                .unwrap()
        })
    }

    #[test]
    fn buy_by_id_uses_exact_id_and_waits_for_each_matching_receipt() {
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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

    /// An `if_close` (an NPC's hit, `npc_combat_melee.rs2:31,36`) shuts the
    /// shop under an outstanding buy batch. No button goes to the closed
    /// shop and the call returns what was bought.
    #[test]
    fn a_shop_closed_mid_batch_returns_the_count_without_another_button() {
        if !api::test_support::game_data_available() {
            return;
        }
        for landed in [false, true] {
            let open = client_with_shop(true);
            let closed = client_with_shop(false);
            let before = snapshot(&open, vec![item(995, "Coins", 10)], Vec::new());
            let held = if landed {
                vec![item(995, "Coins", 9), item(4, "Pot", 1)]
            } else {
                vec![item(995, "Coins", 10)]
            };
            let after = snapshot(&closed, held, Vec::new());
            let mut ledger = None;
            let handle = start_by_id_machine(&before, &mut ledger, 2.0);
            let polled = with_tick(&before, &mut ledger, 2, |t| {
                t.actions.poll(&handle, &mut t.cx)
            });
            assert!(polled.is_pending());
            assert_eq!(buttons(&ledger), 1, "the first batch");
            sent(&mut ledger, 2);
            let result = with_tick(&after, &mut ledger, 3, |t| {
                t.actions.poll(&handle, &mut t.cx)
            });
            assert_eq!(buttons(&ledger), 1, "no button to a closed shop");
            assert_eq!(
                result,
                Poll::Ready(Ok(BuyReceipt {
                    held: i32::from(landed),
                    bought: i32::from(landed),
                }))
            );
        }
    }
}
