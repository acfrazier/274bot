use super::*;
use crate::native::{ActionContext, ActionError, HostEffect, InteractionReceipt};
use crate::quester::families::tests::with_tick;
use api::interact::{Interactions, SendResult, SCENE_READY};
use api::quest_progress::EvidenceStamp;
use api::snapshot::{
    ChatLineView, Family, GameSnapshot, ItemActionFamily, ItemContainer, ItemView, NpcView,
    SceneView, WorldTile,
};
use client::client::{Client, ClientConfig};
use client::config::if_type::{ComponentType, IfType, IfTypeMut};
use client::io::{ClientStream, Packet, ServerProt};
use std::net::TcpListener;
use std::task::Poll;

fn client_with_shop(open: bool) -> Client {
    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp/274bot-buy-ordering-cache".into(),
        members: true,
        lowmem: false,
    });
    let cache = std::sync::Arc::get_mut(&mut client.cache).unwrap();
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
    client.ingame = true;
    client.scene_state = SCENE_READY;
    client.map_build_base_x = 3200;
    client.map_build_base_z = 3200;
    client.bump_gens(ServerProt::IF_OPENMAIN);
    client
}

fn attached_client_with_shop(open: bool) -> (TcpListener, Client) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
    let addr = listener.local_addr().expect("listener address");
    let mut client = client_with_shop(open);
    client.stream = Some(
        ClientStream::connect(&addr.ip().to_string(), addr.port()).expect("loopback client stream"),
    );
    client.bump_gens(ServerProt::REBUILD_NORMAL);
    (listener, client)
}

fn open_shop(client: &mut Client) {
    let mut packet = Packet::new(vec![0x0e, 0xf0, 0x0e, 0xee]);
    client.apply_if_openmain_side(&mut packet);
    client.bump_gens(ServerProt::IF_OPENMAIN);
}

fn snapshot(client: &Client, inventory: Vec<ItemView>, npcs: Vec<NpcView>) -> GameSnapshot {
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(SCENE_READY);
    snapshot.rebuild_family(client, Family::Shop);
    snapshot.rebuild_family(client, Family::Modals);
    snapshot.seed_inventory(inventory, 28);
    snapshot.seed_npcs(npcs);
    snapshot
}

fn attached_snapshot(
    client: &Client,
    inventory: Vec<ItemView>,
    npcs: Vec<NpcView>,
) -> GameSnapshot {
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(client);
    snapshot.seed_ingame(SCENE_READY);
    snapshot.seed_scene(SceneView {
        available: true,
        ..Default::default()
    });
    snapshot.seed_inventory(inventory, 28);
    snapshot.seed_npcs(npcs);
    snapshot
}

fn apply_local_close(client: &mut Client) {
    let snapshot = attached_snapshot(client, Vec::new(), Vec::new());
    let closes_before = client.main_modal_packet_state().close_packets;
    let output_before = client.out.pos;
    let result = Interactions::new(&snapshot, client).close_modal();
    assert!(matches!(result, SendResult::Sent { .. }));
    assert_eq!(
        client.main_modal_id, -1,
        "Client::CloseModal clears the main page"
    );
    assert_eq!(
        client.side_modal_id, -1,
        "Client::CloseModal clears the side page"
    );
    assert_eq!(
        client.chat_modal_id, -1,
        "Client::CloseModal clears the chat page"
    );
    assert_eq!(
        client.main_modal_packet_state().close_packets,
        closes_before,
        "a local API close is not a server IF_CLOSE observation"
    );
    assert!(
        client.out.pos > output_before,
        "the Client API wrote CLOSE_MODAL"
    );
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

fn npc_row(index: usize, npc_type: usize, distance: i32) -> NpcView {
    let tile = WorldTile {
        x: 3200 + distance,
        z: 3200,
        level: 0,
    };
    NpcView {
        index,
        r#type: Some(npc_type),
        name: Some("Shopkeeper".to_owned()),
        actions: vec![Some("Trade".to_owned())],
        tile,
        distance,
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

fn request(qty: i32) -> BuyRequest {
    BuyRequest {
        npc_id: 123,
        npc_name: std::sync::Arc::from("Shopkeeper"),
        item_id: 4,
        item_name: std::sync::Arc::from("Pot"),
        qty,
    }
}

fn start_machine(
    snapshot: &GameSnapshot,
    ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
    qty: i32,
) -> crate::native::ActionHandle<BuyMachine> {
    with_tick(snapshot, ledger, 1, |tick| {
        tick.actions
            .begin::<BuyMachine>(request(qty), &mut tick.cx)
            .unwrap()
    })
}

fn poll_machine(
    snapshot: &GameSnapshot,
    handle: &crate::native::ActionHandle<BuyMachine>,
    ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
    tick: u64,
) -> Poll<Result<BuyReceipt, ActionError>> {
    with_tick(snapshot, ledger, tick, |native| {
        native.actions.poll(handle, &mut native.cx)
    })
}

#[derive(Default)]
struct ActionLog {
    trades: Vec<i32>,
    buttons: Vec<i32>,
    closes: usize,
}

fn last_request(ledger: &Option<Box<crate::native::ledger::Ledger>>) -> &InteractReq {
    match &ledger
        .as_ref()
        .unwrap()
        .outbox
        .last()
        .expect("queued native action")
        .effect
    {
        HostEffect::Interaction(request) => request,
        _ => panic!("expected an interaction"),
    }
}

fn drain_interaction(
    ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
    log: &mut ActionLog,
    tick: u64,
) {
    let action = ledger
        .as_mut()
        .unwrap()
        .outbox
        .pop()
        .expect("queued native action");
    let authority = action.authority();
    match &action.effect {
        HostEffect::Interaction(InteractReq::Npc {
            action,
            index: Some(index),
            ..
        }) if action == "Trade" => log.trades.push(*index),
        HostEffect::Interaction(InteractReq::ShopButton { chunk, .. }) => log.buttons.push(*chunk),
        HostEffect::Interaction(InteractReq::CloseModal) => log.closes += 1,
        HostEffect::Interaction(request) => panic!("unexpected native interaction {request:?}"),
        _ => panic!("expected an interaction"),
    }
    ledger.as_mut().unwrap().complete_interaction(
        &authority,
        InteractionReceipt {
            request_id: authority.request_id().get(),
            evidence: EvidenceStamp {
                run: authority.run(),
                tick,
                sequence: tick,
            },
            accepted: true,
            chat_since: 0,
        },
    );
}

fn drain_trade(
    ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
    log: &mut ActionLog,
    tick: u64,
    index: i32,
) {
    assert!(matches!(
        last_request(ledger),
        InteractReq::Npc { name, action, index: Some(got) }
            if name == "Shopkeeper" && action == "Trade" && *got == index
    ));
    drain_interaction(ledger, log, tick);
}

fn drain_button(
    ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
    log: &mut ActionLog,
    tick: u64,
    chunk: i32,
) {
    assert!(matches!(
        last_request(ledger),
        InteractReq::ShopButton { kind, name, id, chunk: got, .. }
            if kind == "buy" && name == "Pot" && *id == 4 && *got == chunk
    ));
    drain_interaction(ledger, log, tick);
}

fn close_with_api(
    client: &mut Client,
    ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
    log: &mut ActionLog,
    tick: u64,
) {
    assert!(matches!(last_request(ledger), InteractReq::CloseModal));
    apply_local_close(client);
    drain_interaction(ledger, log, tick);
}

fn assert_no_queued_action(ledger: &Option<Box<crate::native::ledger::Ledger>>) {
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
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

fn coins_refusal() -> ChatLineView {
    ChatLineView {
        type_: 0,
        username: None,
        text: "You don't have enough coins.".to_owned(),
        sequence: 1,
    }
}

#[test]
fn same_named_wrong_type_is_never_traded_and_exact_keeper_index_survives_recovery() {
    let mut client = client_with_shop(false);
    let first_npcs = vec![npc_row(3, 124, 1), npc_row(7, 123, 6)];
    let closed = snapshot(&client, Vec::new(), first_npcs);
    let mut ledger = None;
    let handle = start_machine(&closed, &mut ledger, 5);
    let mut log = ActionLog::default();

    assert!(poll_machine(&closed, &handle, &mut ledger, 2).is_pending());
    drain_trade(&mut ledger, &mut log, 2, 7);
    assert_eq!(
        log.trades,
        [7],
        "the nearer same-name NPC has the wrong type"
    );

    open_shop(&mut client);
    let page = snapshot(&client, Vec::new(), Vec::new());
    assert!(poll_machine(&page, &handle, &mut ledger, 3).is_pending());
    drain_button(&mut ledger, &mut log, 3, 5);

    client.apply_if_close();
    let recovery_npcs = vec![npc_row(8, 123, 1), npc_row(7, 123, 8), npc_row(3, 124, 0)];
    let closed = snapshot(&client, Vec::new(), recovery_npcs);
    assert!(poll_machine(&closed, &handle, &mut ledger, 4).is_pending());
    drain_trade(&mut ledger, &mut log, 4, 7);
    assert_eq!(
        log.trades,
        [7, 7],
        "recovery retains the selected type and index"
    );
    assert_eq!(log.buttons, [5]);
    drop(handle);
}

#[test]
fn initially_open_wrong_shop_requires_real_local_clear_and_server_close_count() {
    let (_listener, mut client) = attached_client_with_shop(true);
    let before_close = client.main_modal_packet_state().close_packets;
    let npc = vec![npc_row(7, 123, 2)];
    let initial = snapshot(&client, Vec::new(), npc.clone());
    let mut ledger = None;
    let handle = start_machine(&initial, &mut ledger, 1);
    let mut log = ActionLog::default();

    assert!(poll_machine(&initial, &handle, &mut ledger, 2).is_pending());
    close_with_api(&mut client, &mut ledger, &mut log, 2);
    assert_eq!(log.closes, 1);
    assert!(log.trades.is_empty());
    assert!(log.buttons.is_empty());

    let mut local_clear = snapshot(&client, Vec::new(), npc.clone());
    assert!(!local_clear.shop().open);
    assert_eq!(local_clear.shop().server_closes, before_close);
    // An unrelated family rebuild must not turn local absence into a close receipt.
    local_clear.rebuild_family(&client, Family::Inventory);
    assert!(poll_machine(&local_clear, &handle, &mut ledger, 3).is_pending());
    assert_no_queued_action(&ledger);
    assert!(log.trades.is_empty());
    assert!(log.buttons.is_empty());

    let still_local = snapshot(&client, Vec::new(), npc.clone());
    assert!(poll_machine(&still_local, &handle, &mut ledger, 4).is_pending());
    assert_no_queued_action(&ledger);
    assert!(log.trades.is_empty());

    client.apply_if_close();
    let server_closed = snapshot(&client, Vec::new(), npc);
    assert_eq!(server_closed.shop().server_closes, before_close + 1);
    assert!(poll_machine(&server_closed, &handle, &mut ledger, 5).is_pending());
    drain_trade(&mut ledger, &mut log, 5, 7);
    assert_eq!(log.trades, [7]);
    assert!(log.buttons.is_empty(), "the old page never receives a buy");
    drop(handle);
}

#[test]
fn own_trade_page_needs_a_later_unspent_tick_and_no_idle_page_is_adopted() {
    let mut client = client_with_shop(false);
    let closed = snapshot(&client, Vec::new(), vec![npc_row(7, 123, 2)]);
    let mut ledger = None;
    let handle = start_machine(&closed, &mut ledger, 1);
    let mut log = ActionLog::default();

    assert!(poll_machine(&closed, &handle, &mut ledger, 2).is_pending());
    drain_trade(&mut ledger, &mut log, 2, 7);

    open_shop(&mut client);
    let page = snapshot(&client, Vec::new(), Vec::new());
    assert!(poll_machine(&page, &handle, &mut ledger, 2).is_pending());
    assert_no_queued_action(&ledger);
    assert!(
        log.buttons.is_empty(),
        "a page on the Trade send tick is too early"
    );

    let spent = with_tick(&page, &mut ledger, 3, |native| {
        spend_interaction(&mut native.cx);
        native.actions.poll(&handle, &mut native.cx)
    });
    assert!(spent.is_pending());
    assert_no_queued_action(&ledger);
    assert!(
        log.buttons.is_empty(),
        "another interaction spent this tick"
    );

    assert!(poll_machine(&page, &handle, &mut ledger, 4).is_pending());
    drain_button(&mut ledger, &mut log, 4, 1);
    assert_eq!(log.trades, [7]);
    assert_eq!(log.buttons, [1]);
    drop(handle);
}

#[test]
fn a_fresh_unreachable_trade_reply_retries_without_waiting_for_a_shop_page() {
    let mut client = client_with_shop(false);
    let npcs = vec![npc_row(7, 123, 2)];
    let closed = snapshot(&client, Vec::new(), npcs.clone());
    let mut ledger = None;
    let handle = start_machine(&closed, &mut ledger, 1);
    let mut log = ActionLog::default();
    assert!(poll_machine(&closed, &handle, &mut ledger, 2).is_pending());
    drain_trade(&mut ledger, &mut log, 2, 7);

    let mut rejected = snapshot(&client, Vec::new(), npcs);
    rejected.seed_chat_lines(vec![ChatLineView {
        type_: 0,
        username: None,
        text: "I can't reach that!".to_owned(),
        sequence: 1,
    }]);
    assert!(poll_machine(&rejected, &handle, &mut ledger, 3).is_pending());
    drain_trade(&mut ledger, &mut log, 3, 7);
    assert_eq!(log.trades, [7, 7]);
    assert_eq!(log.closes, 0, "a rejected Trade never opened a page");
    assert!(log.buttons.is_empty());

    assert!(poll_machine(&rejected, &handle, &mut ledger, 4).is_pending());
    assert_no_queued_action(&ledger);
    open_shop(&mut client);
    let page = snapshot(&client, Vec::new(), Vec::new());
    assert!(poll_machine(&page, &handle, &mut ledger, 5).is_pending());
    drain_button(&mut ledger, &mut log, 5, 1);
    assert_eq!(
        log.trades,
        [7, 7],
        "the old refusal cannot reject the retry"
    );
    drop(handle);
}

#[test]
fn rejected_reconciliation_trade_does_not_settle_or_repeat_the_buy() {
    let (_listener, mut client) = attached_client_with_shop(false);
    let npcs = vec![npc_row(7, 123, 2)];
    let closed = snapshot(&client, Vec::new(), npcs.clone());
    let mut ledger = None;
    let handle = start_machine(&closed, &mut ledger, 5);
    let mut log = ActionLog::default();
    assert!(poll_machine(&closed, &handle, &mut ledger, 2).is_pending());
    drain_trade(&mut ledger, &mut log, 2, 7);
    open_shop(&mut client);
    let page = snapshot(&client, Vec::new(), Vec::new());
    assert!(poll_machine(&page, &handle, &mut ledger, 3).is_pending());
    drain_button(&mut ledger, &mut log, 3, 5);

    client.apply_if_close();
    let closed = snapshot(&client, Vec::new(), npcs.clone());
    assert!(poll_machine(&closed, &handle, &mut ledger, 4).is_pending());
    drain_trade(&mut ledger, &mut log, 4, 7);
    let mut rejected = snapshot(&client, Vec::new(), npcs);
    rejected.seed_chat_lines(vec![ChatLineView {
        type_: 0,
        username: None,
        text: "I can't reach that!".to_owned(),
        sequence: 1,
    }]);
    assert!(poll_machine(&rejected, &handle, &mut ledger, 5).is_pending());
    drain_trade(&mut ledger, &mut log, 5, 7);
    assert_eq!(handle.inspect(|machine| machine.bought), Some(0));
    assert_eq!(
        handle.inspect(|machine| machine.pending.as_ref().is_some_and(|buy| !buy.resolved)),
        Some(true)
    );
    assert_eq!(log.buttons, [5]);

    open_shop(&mut client);
    let supplied = snapshot(&client, vec![item(4, "Pot", 5)], Vec::new());
    assert!(poll_machine(&supplied, &handle, &mut ledger, 6).is_pending());
    close_with_api(&mut client, &mut ledger, &mut log, 6);
    client.apply_if_close();
    let counted = snapshot(&client, vec![item(4, "Pot", 5)], Vec::new());
    let Poll::Ready(Ok(receipt)) = poll_machine(&counted, &handle, &mut ledger, 7) else {
        panic!("the counted final close must finish the reconciled purchase");
    };
    assert_eq!(receipt.bought, 5);
    assert_eq!(log.trades, [7, 7, 7]);
    assert_eq!(log.buttons, [5]);
    assert_eq!(log.closes, 1);
    drop(handle);
}

#[test]
fn explicit_trade_rejections_keep_the_original_three_attempt_limit() {
    let client = client_with_shop(false);
    let npcs = vec![npc_row(7, 123, 2)];
    let closed = snapshot(&client, Vec::new(), npcs.clone());
    let mut ledger = None;
    let handle = start_machine(&closed, &mut ledger, 1);
    let mut log = ActionLog::default();
    assert!(poll_machine(&closed, &handle, &mut ledger, 2).is_pending());
    drain_trade(&mut ledger, &mut log, 2, 7);
    for sequence in 1..=3 {
        let tick = 2 + sequence as u64;
        let mut rejected = snapshot(&client, Vec::new(), npcs.clone());
        rejected.seed_chat_lines(vec![ChatLineView {
            type_: 0,
            username: None,
            text: "I can't reach that!".to_owned(),
            sequence,
        }]);
        let result = poll_machine(&rejected, &handle, &mut ledger, tick);
        if sequence < 3 {
            assert!(result.is_pending());
            drain_trade(&mut ledger, &mut log, tick, 7);
        } else {
            assert!(matches!(
                result,
                Poll::Ready(Err(ActionError::Failed(message)))
                    if message.as_ref() == "no usable shop after 3 Trade attempts"
            ));
        }
    }
    assert_eq!(log.trades, [7, 7, 7]);
    assert!(log.buttons.is_empty());
    assert_eq!(log.closes, 0);
    drop(handle);
}

#[test]
fn a_late_engine_rejection_ends_the_unanswered_trade_without_a_late_page() {
    let client = client_with_shop(false);
    let npcs = vec![npc_row(7, 123, 2)];
    let closed = snapshot(&client, Vec::new(), npcs.clone());
    let mut ledger = None;
    let handle = start_machine(&closed, &mut ledger, 1);
    let mut log = ActionLog::default();
    assert!(poll_machine(&closed, &handle, &mut ledger, 2).is_pending());
    drain_trade(&mut ledger, &mut log, 2, 7);
    let expired = 3 + shop_open::OPEN_CAP_TICKS;
    assert!(poll_machine(&closed, &handle, &mut ledger, expired).is_pending());
    assert_no_queued_action(&ledger);
    assert_eq!(
        log.trades,
        [7],
        "a timeout alone cannot authorize another Trade"
    );

    let mut rejected = snapshot(&client, Vec::new(), npcs);
    rejected.seed_chat_lines(vec![ChatLineView {
        type_: 0,
        username: None,
        text: "I can't reach that!".to_owned(),
        sequence: 1,
    }]);
    assert!(poll_machine(&rejected, &handle, &mut ledger, expired + 1).is_pending());
    drain_trade(&mut ledger, &mut log, expired + 1, 7);
    assert_eq!(log.trades, [7, 7]);
    assert_eq!(log.closes, 0);
    assert!(log.buttons.is_empty());
    drop(handle);
}

#[test]
fn late_trade_page_is_closed_and_counted_before_retry_trade() {
    let (_listener, mut client) = attached_client_with_shop(false);
    let npcs = vec![npc_row(7, 123, 2)];
    let closed = snapshot(&client, Vec::new(), npcs.clone());
    let mut ledger = None;
    let handle = start_machine(&closed, &mut ledger, 15);
    let mut log = ActionLog::default();

    assert!(poll_machine(&closed, &handle, &mut ledger, 2).is_pending());
    drain_trade(&mut ledger, &mut log, 2, 7);
    open_shop(&mut client);
    let first_page = snapshot(&client, Vec::new(), Vec::new());
    assert!(poll_machine(&first_page, &handle, &mut ledger, 3).is_pending());
    drain_button(&mut ledger, &mut log, 3, 10);

    client.apply_if_close();
    let closed = snapshot(&client, Vec::new(), npcs.clone());
    assert!(poll_machine(&closed, &handle, &mut ledger, 4).is_pending());
    drain_trade(&mut ledger, &mut log, 4, 7);
    let first_recovery_trade = 4;
    let deadline = first_recovery_trade + shop_open::OPEN_CAP_TICKS;

    for tick in 5..=deadline + 1 {
        assert!(poll_machine(&closed, &handle, &mut ledger, tick).is_pending());
        assert_no_queued_action(&ledger);
        assert_eq!(log.trades, [7, 7], "no overlapping Trade at tick {tick}");
    }

    // A later raw close still cannot end the first recovery Trade.
    client.apply_if_close();
    let closed_again = snapshot(&client, Vec::new(), npcs.clone());
    assert!(poll_machine(&closed_again, &handle, &mut ledger, deadline + 2).is_pending());
    assert_no_queued_action(&ledger);
    assert_eq!(log.trades, [7, 7]);

    open_shop(&mut client);
    let late_page = snapshot(&client, vec![item(4, "Pot", 10)], npcs.clone());
    assert!(poll_machine(&late_page, &handle, &mut ledger, deadline + 3).is_pending());
    assert_eq!(handle.inspect(|machine| machine.bought), Some(10));
    assert_eq!(
        handle.inspect(|machine| machine.pending.is_none()),
        Some(true)
    );
    close_with_api(&mut client, &mut ledger, &mut log, deadline + 3);
    assert_eq!(
        log.buttons,
        [10],
        "the late page is reconciled before another buy"
    );

    let locally_closed = snapshot(&client, vec![item(4, "Pot", 10)], npcs.clone());
    assert!(poll_machine(&locally_closed, &handle, &mut ledger, deadline + 4).is_pending());
    assert_no_queued_action(&ledger);
    assert_eq!(log.trades, [7, 7]);

    client.apply_if_close();
    let counted_close = snapshot(&client, vec![item(4, "Pot", 10)], npcs.clone());
    assert!(poll_machine(&counted_close, &handle, &mut ledger, deadline + 5).is_pending());
    drain_trade(&mut ledger, &mut log, deadline + 5, 7);
    assert_eq!(log.trades, [7, 7, 7]);

    open_shop(&mut client);
    let same_tick_page = snapshot(&client, vec![item(4, "Pot", 10)], Vec::new());
    assert!(poll_machine(&same_tick_page, &handle, &mut ledger, deadline + 5).is_pending());
    assert_no_queued_action(&ledger);
    assert_eq!(log.buttons, [10]);

    let replacement_page = snapshot(&client, vec![item(4, "Pot", 10)], Vec::new());
    assert!(poll_machine(&replacement_page, &handle, &mut ledger, deadline + 6).is_pending());
    drain_button(&mut ledger, &mut log, deadline + 6, 5);
    assert_eq!(log.buttons, [10, 5]);

    let full_pack = snapshot(&client, vec![item(4, "Pot", 15)], Vec::new());
    assert!(poll_machine(&full_pack, &handle, &mut ledger, deadline + 7).is_pending());
    close_with_api(&mut client, &mut ledger, &mut log, deadline + 7);
    let no_server_close = snapshot(&client, vec![item(4, "Pot", 15)], Vec::new());
    assert!(poll_machine(&no_server_close, &handle, &mut ledger, deadline + 8).is_pending());
    client.apply_if_close();
    let final_close = snapshot(&client, vec![item(4, "Pot", 15)], Vec::new());
    let Poll::Ready(Ok(receipt)) = poll_machine(&final_close, &handle, &mut ledger, deadline + 9)
    else {
        panic!("the final close barrier should produce a receipt");
    };
    assert_eq!(receipt.held, 15);
    assert_eq!(receipt.bought, 15);
    assert_eq!(receipt.refusal, None);
    assert_eq!(log.buttons, [10, 5]);
    assert_eq!(log.trades, [7, 7, 7]);
}

#[derive(Clone, Copy)]
enum RemainingModal {
    Chat,
    Side,
}

fn foreign_close_case(remaining: Option<RemainingModal>) {
    let (_listener, mut client) = attached_client_with_shop(false);
    let npcs = vec![npc_row(7, 123, 2)];
    let closed = snapshot(&client, Vec::new(), npcs.clone());
    let mut ledger = None;
    let handle = start_machine(&closed, &mut ledger, 1);
    let mut log = ActionLog::default();

    assert!(poll_machine(&closed, &handle, &mut ledger, 2).is_pending());
    drain_trade(&mut ledger, &mut log, 2, 7);
    open_shop(&mut client);
    let page = snapshot(&client, Vec::new(), Vec::new());
    assert!(poll_machine(&page, &handle, &mut ledger, 3).is_pending());
    drain_button(&mut ledger, &mut log, 3, 1);

    client.apply_if_close();
    match remaining {
        Some(RemainingModal::Chat) => client.chat_modal_id = 4800,
        Some(RemainingModal::Side) => client.side_modal_id = 4800,
        None => {}
    }
    if remaining.is_some() {
        client.bump_gens(ServerProt::IF_OPENMAIN);
    }
    let foreign_close = snapshot(&client, Vec::new(), npcs.clone());
    assert!(!foreign_close.shop().open);
    assert_eq!(foreign_close.shop().server_closes, 1);
    match remaining {
        Some(RemainingModal::Chat) => assert_eq!(foreign_close.modals().chat, 4800),
        Some(RemainingModal::Side) => assert_eq!(foreign_close.modals().side, 4800),
        None => assert_eq!(foreign_close.modals().main, -1),
    }
    assert!(poll_machine(&foreign_close, &handle, &mut ledger, 4).is_pending());

    let fresh_tick = if remaining.is_some() {
        close_with_api(&mut client, &mut ledger, &mut log, 4);
        assert_eq!(log.trades, [7]);
        assert_eq!(
            log.closes, 1,
            "a remaining local modal must close before Trade"
        );

        let no_modal = snapshot(&client, Vec::new(), npcs.clone());
        assert_eq!(no_modal.modals().main, -1);
        assert_eq!(no_modal.modals().side, -1);
        assert_eq!(no_modal.modals().chat, -1);
        assert!(poll_machine(&no_modal, &handle, &mut ledger, 5).is_pending());
        drain_trade(&mut ledger, &mut log, 5, 7);
        6
    } else {
        drain_trade(&mut ledger, &mut log, 4, 7);
        assert_eq!(log.closes, 0, "no local modal means Trade directly");
        5
    };
    assert_eq!(log.trades, [7, 7]);

    open_shop(&mut client);
    let fresh_page = snapshot(&client, vec![item(4, "Pot", 1)], Vec::new());
    assert!(poll_machine(&fresh_page, &handle, &mut ledger, fresh_tick).is_pending());
    close_with_api(&mut client, &mut ledger, &mut log, fresh_tick);
    assert_eq!(
        log.buttons,
        [1],
        "the old dispatch is reconciled before any next button"
    );

    let no_close_yet = snapshot(&client, vec![item(4, "Pot", 1)], Vec::new());
    assert!(poll_machine(&no_close_yet, &handle, &mut ledger, fresh_tick + 1).is_pending());
    client.apply_if_close();
    let closed = snapshot(&client, vec![item(4, "Pot", 1)], Vec::new());
    let Poll::Ready(Ok(receipt)) = poll_machine(&closed, &handle, &mut ledger, fresh_tick + 2)
    else {
        panic!("the server close should release the final receipt");
    };
    assert_eq!(receipt.held, 1);
    assert_eq!(receipt.bought, 1);
    assert_eq!(receipt.refusal, None);
}

#[test]
fn foreign_chat_modal_after_shop_close_is_closed_before_recovery_trade() {
    foreign_close_case(Some(RemainingModal::Chat));
}

#[test]
fn foreign_side_modal_after_shop_close_is_closed_before_recovery_trade() {
    foreign_close_case(Some(RemainingModal::Side));
}

#[test]
fn foreign_close_without_a_remaining_modal_trades_directly() {
    foreign_close_case(None);
}

#[test]
fn close_and_open_in_one_observation_cannot_bypass_a_new_trade_page() {
    let (_listener, mut client) = attached_client_with_shop(false);
    let npcs = vec![npc_row(7, 123, 2)];
    let closed = snapshot(&client, Vec::new(), npcs.clone());
    let mut ledger = None;
    let handle = start_machine(&closed, &mut ledger, 1);
    let mut log = ActionLog::default();

    assert!(poll_machine(&closed, &handle, &mut ledger, 2).is_pending());
    drain_trade(&mut ledger, &mut log, 2, 7);
    open_shop(&mut client);
    let first_page = snapshot(&client, Vec::new(), Vec::new());
    assert!(poll_machine(&first_page, &handle, &mut ledger, 3).is_pending());
    drain_button(&mut ledger, &mut log, 3, 1);

    // Both server packets are visible in one snapshot. This page predates recovery Trade.
    client.apply_if_close();
    open_shop(&mut client);
    let close_and_open = snapshot(&client, Vec::new(), npcs.clone());
    assert_eq!(close_and_open.shop().server_closes, 1);
    assert!(close_and_open.shop().open);
    assert!(poll_machine(&close_and_open, &handle, &mut ledger, 4).is_pending());
    close_with_api(&mut client, &mut ledger, &mut log, 4);
    assert_eq!(log.trades, [7]);
    assert_eq!(log.buttons, [1]);

    let absent_page = snapshot(&client, Vec::new(), npcs.clone());
    assert!(poll_machine(&absent_page, &handle, &mut ledger, 5).is_pending());
    drain_trade(&mut ledger, &mut log, 5, 7);
    assert_eq!(log.trades, [7, 7]);

    open_shop(&mut client);
    let fresh_page = snapshot(&client, vec![item(4, "Pot", 1)], Vec::new());
    assert!(poll_machine(&fresh_page, &handle, &mut ledger, 6).is_pending());
    close_with_api(&mut client, &mut ledger, &mut log, 6);
    assert_eq!(log.buttons, [1]);

    let locally_closed = snapshot(&client, vec![item(4, "Pot", 1)], Vec::new());
    assert!(poll_machine(&locally_closed, &handle, &mut ledger, 7).is_pending());
    client.apply_if_close();
    let counted_close = snapshot(&client, vec![item(4, "Pot", 1)], Vec::new());
    let Poll::Ready(Ok(receipt)) = poll_machine(&counted_close, &handle, &mut ledger, 8) else {
        panic!("the final close count should release the receipt");
    };
    assert_eq!(receipt.bought, 1);
    assert_eq!(receipt.held, 1);
}

fn partial_refusal_case(refusal_first: bool) {
    let (_listener, mut client) = attached_client_with_shop(false);
    let npcs = vec![npc_row(7, 123, 2)];
    let closed = snapshot(&client, Vec::new(), npcs.clone());
    let mut ledger = None;
    let handle = start_machine(&closed, &mut ledger, 5);
    let mut log = ActionLog::default();

    assert!(poll_machine(&closed, &handle, &mut ledger, 2).is_pending());
    drain_trade(&mut ledger, &mut log, 2, 7);
    open_shop(&mut client);
    let page = snapshot(&client, Vec::new(), Vec::new());
    assert!(poll_machine(&page, &handle, &mut ledger, 3).is_pending());
    drain_button(&mut ledger, &mut log, 3, 5);

    if refusal_first {
        let mut refusal = snapshot(&client, Vec::new(), Vec::new());
        refusal.seed_chat_lines(vec![coins_refusal()]);
        assert!(poll_machine(&refusal, &handle, &mut ledger, 4).is_pending());
        assert_no_queued_action(&ledger);

        client.apply_if_close();
        let mut closed = snapshot(&client, Vec::new(), npcs.clone());
        closed.seed_chat_lines(vec![coins_refusal()]);
        assert!(poll_machine(&closed, &handle, &mut ledger, 5).is_pending());
        drain_trade(&mut ledger, &mut log, 5, 7);
    } else {
        let partial_pack = snapshot(&client, vec![item(4, "Pot", 3)], Vec::new());
        assert!(poll_machine(&partial_pack, &handle, &mut ledger, 4).is_pending());
        assert_no_queued_action(&ledger);

        client.apply_if_close();
        let mut closed = snapshot(&client, vec![item(4, "Pot", 3)], npcs.clone());
        closed.seed_chat_lines(vec![coins_refusal()]);
        assert!(poll_machine(&closed, &handle, &mut ledger, 5).is_pending());
        drain_trade(&mut ledger, &mut log, 5, 7);
    }

    open_shop(&mut client);
    let mut fresh_page = snapshot(&client, vec![item(4, "Pot", 3)], Vec::new());
    fresh_page.seed_chat_lines(vec![coins_refusal()]);
    assert!(poll_machine(&fresh_page, &handle, &mut ledger, 6).is_pending());
    close_with_api(&mut client, &mut ledger, &mut log, 6);
    assert_eq!(log.buttons, [5], "neither ordering admits another button");

    let locally_closed = snapshot(&client, vec![item(4, "Pot", 3)], Vec::new());
    assert!(poll_machine(&locally_closed, &handle, &mut ledger, 7).is_pending());
    client.apply_if_close();
    let final_close = snapshot(&client, vec![item(4, "Pot", 3)], Vec::new());
    let Poll::Ready(Ok(receipt)) = poll_machine(&final_close, &handle, &mut ledger, 8) else {
        panic!("the final close should return the refusal receipt");
    };
    assert_eq!(receipt.held, 3);
    assert_eq!(receipt.bought, 3);
    assert_eq!(receipt.refusal, Some(PurchaseRefusal::Coins));
}

#[test]
fn partial_three_of_five_and_coins_refusal_survive_recovery_chat_first() {
    partial_refusal_case(true);
}

#[test]
fn partial_three_of_five_and_coins_refusal_survive_recovery_pack_first() {
    partial_refusal_case(false);
}

#[test]
fn full_pack_evidence_before_split_roundtrip_page_is_counted_once() {
    let (_listener, mut client) = attached_client_with_shop(false);
    let npcs = vec![npc_row(7, 123, 2)];
    let closed = snapshot(&client, Vec::new(), npcs.clone());
    let mut ledger = None;
    let handle = start_machine(&closed, &mut ledger, 1);
    let mut log = ActionLog::default();

    assert!(poll_machine(&closed, &handle, &mut ledger, 2).is_pending());
    drain_trade(&mut ledger, &mut log, 2, 7);
    open_shop(&mut client);
    let page = snapshot(&client, Vec::new(), Vec::new());
    assert!(poll_machine(&page, &handle, &mut ledger, 3).is_pending());
    drain_button(&mut ledger, &mut log, 3, 1);

    // Observe IF_CLOSE first. The recovery Trade follows while no page is open.
    client.apply_if_close();
    let closed = snapshot(&client, Vec::new(), npcs.clone());
    assert!(poll_machine(&closed, &handle, &mut ledger, 4).is_pending());
    drain_trade(&mut ledger, &mut log, 4, 7);

    // The full inventory update arrives before the replacement page.
    let early_pack = snapshot(&client, vec![item(4, "Pot", 1)], Vec::new());
    assert!(poll_machine(&early_pack, &handle, &mut ledger, 5).is_pending());
    assert_no_queued_action(&ledger);
    assert_eq!(handle.inspect(|machine| machine.bought), Some(1));
    assert_eq!(
        handle.inspect(|machine| machine.pending.is_none()),
        Some(true)
    );
    assert_eq!(log.buttons, [1]);

    assert!(poll_machine(&early_pack, &handle, &mut ledger, 5).is_pending());
    assert_no_queued_action(&ledger);
    assert_eq!(handle.inspect(|machine| machine.bought), Some(1));

    open_shop(&mut client);
    let fresh_page = snapshot(&client, vec![item(4, "Pot", 1)], Vec::new());
    assert!(poll_machine(&fresh_page, &handle, &mut ledger, 6).is_pending());
    close_with_api(&mut client, &mut ledger, &mut log, 6);
    assert_eq!(
        log.buttons,
        [1],
        "the late page does not count the chunk twice"
    );

    let locally_closed = snapshot(&client, vec![item(4, "Pot", 1)], Vec::new());
    assert!(poll_machine(&locally_closed, &handle, &mut ledger, 7).is_pending());
    client.apply_if_close();
    let final_close = snapshot(&client, vec![item(4, "Pot", 1)], Vec::new());
    let Poll::Ready(Ok(receipt)) = poll_machine(&final_close, &handle, &mut ledger, 8) else {
        panic!("the final close should release the receipt");
    };
    assert_eq!(receipt.held, 1);
    assert_eq!(receipt.bought, 1);
}

#[test]
fn two_unanswered_dispatches_need_fresh_pages_and_the_last_close_barrier() {
    let (_listener, mut client) = attached_client_with_shop(false);
    let npcs = vec![npc_row(7, 123, 2)];
    let closed = snapshot(&client, Vec::new(), npcs.clone());
    let mut ledger = None;
    let handle = start_machine(&closed, &mut ledger, 1);
    let mut log = ActionLog::default();

    assert!(poll_machine(&closed, &handle, &mut ledger, 2).is_pending());
    drain_trade(&mut ledger, &mut log, 2, 7);
    open_shop(&mut client);
    let first_page = snapshot(&client, Vec::new(), Vec::new());
    assert!(poll_machine(&first_page, &handle, &mut ledger, 3).is_pending());
    drain_button(&mut ledger, &mut log, 3, 1);

    for _ in 0..4 {
        assert!(poll_machine(&first_page, &handle, &mut ledger, 3).is_pending());
        assert_no_queued_action(&ledger);
    }
    assert_eq!(
        log.buttons,
        [1],
        "repeated polls at one tick do not expire a dispatch"
    );

    client.apply_if_close();
    let closed = snapshot(&client, Vec::new(), npcs.clone());
    assert!(poll_machine(&closed, &handle, &mut ledger, 5).is_pending());
    drain_trade(&mut ledger, &mut log, 5, 7);
    assert_eq!(log.closes, 0, "the absent page needs no forced CloseModal");

    open_shop(&mut client);
    let first_recovery_page = snapshot(&client, Vec::new(), Vec::new());
    assert!(poll_machine(&first_recovery_page, &handle, &mut ledger, 6).is_pending());
    drain_button(&mut ledger, &mut log, 6, 1);
    assert_eq!(
        log.buttons,
        [1, 1],
        "the first fresh page permits one retry"
    );

    for _ in 0..4 {
        assert!(poll_machine(&first_recovery_page, &handle, &mut ledger, 6).is_pending());
        assert_no_queued_action(&ledger);
    }
    assert_eq!(
        log.buttons,
        [1, 1],
        "the retry does not expire without tick advance"
    );

    client.apply_if_close();
    let closed = snapshot(&client, Vec::new(), npcs.clone());
    assert!(poll_machine(&closed, &handle, &mut ledger, 8).is_pending());
    drain_trade(&mut ledger, &mut log, 8, 7);

    open_shop(&mut client);
    let second_recovery_page = snapshot(&client, Vec::new(), Vec::new());
    assert!(poll_machine(&second_recovery_page, &handle, &mut ledger, 9).is_pending());
    close_with_api(&mut client, &mut ledger, &mut log, 9);
    assert_eq!(log.buttons, [1, 1]);
    assert_eq!(log.trades, [7, 7, 7]);

    let local_close_only = snapshot(&client, Vec::new(), Vec::new());
    assert!(poll_machine(&local_close_only, &handle, &mut ledger, 10).is_pending());
    assert_no_queued_action(&ledger);
    assert_eq!(
        log.closes, 1,
        "local clearing alone is not the final barrier"
    );

    client.apply_if_close();
    let final_close = snapshot(&client, Vec::new(), Vec::new());
    let Poll::Ready(Err(ActionError::Failed(reason))) =
        poll_machine(&final_close, &handle, &mut ledger, 11)
    else {
        panic!("two unanswered fresh pages should fail after the final close barrier");
    };
    assert_eq!(&*reason, "shop purchase did not settle");
    assert_eq!(log.buttons, [1, 1]);
    assert_eq!(log.trades, [7, 7, 7]);
}

#[test]
fn changed_run_returns_stale_and_drops_the_unresolved_dispatch() {
    let mut client = client_with_shop(false);
    let closed = snapshot(&client, Vec::new(), vec![npc_row(7, 123, 2)]);
    let mut ledger = None;
    let handle = start_machine(&closed, &mut ledger, 5);
    let mut log = ActionLog::default();

    assert!(poll_machine(&closed, &handle, &mut ledger, 2).is_pending());
    drain_trade(&mut ledger, &mut log, 2, 7);
    open_shop(&mut client);
    let page = snapshot(&client, Vec::new(), Vec::new());
    assert!(poll_machine(&page, &handle, &mut ledger, 3).is_pending());
    drain_button(&mut ledger, &mut log, 3, 5);
    assert_eq!(
        handle.inspect(|machine| machine.pending.is_some()),
        Some(true)
    );

    let later_pack = snapshot(&client, vec![item(4, "Pot", 5)], Vec::new());
    let stale = with_tick(&later_pack, &mut ledger, 4, |native| {
        native.cx.evidence.run.session += 1;
        native.actions.poll(&handle, &mut native.cx)
    });
    assert!(matches!(stale, Poll::Ready(Err(ActionError::Stale))));
    assert_eq!(log.buttons, [5]);

    // The caller retires a stale run's handle. Its pending batch cannot emit or settle later.
    drop(handle);
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    assert_eq!(log.buttons, [5]);
    assert_eq!(log.trades, [7]);
}

#[test]
fn initial_trade_budget_denial_retries_without_consuming_an_open_attempt() {
    let client = client_with_shop(false);
    let page = snapshot(&client, Vec::new(), vec![npc_row(7, 123, 2)]);
    let mut ledger = None;
    let handle = start_machine(&page, &mut ledger, 1);
    let denied = with_tick(&page, &mut ledger, 2, |native| {
        spend_interaction(&mut native.cx);
        native.actions.poll(&handle, &mut native.cx)
    });
    assert!(denied.is_pending());
    assert_no_queued_action(&ledger);
    assert!(poll_machine(&page, &handle, &mut ledger, 3).is_pending());
    let mut log = ActionLog::default();
    drain_trade(&mut ledger, &mut log, 3, 7);
    assert_eq!(log.trades, [7]);
    assert!(log.buttons.is_empty());
}

#[test]
fn initial_close_budget_denial_retries_the_close_not_a_trade() {
    let client = client_with_shop(true);
    let page = snapshot(&client, Vec::new(), vec![npc_row(7, 123, 2)]);
    let mut ledger = None;
    let handle = start_machine(&page, &mut ledger, 1);
    let denied = with_tick(&page, &mut ledger, 2, |native| {
        spend_interaction(&mut native.cx);
        native.actions.poll(&handle, &mut native.cx)
    });
    assert!(denied.is_pending());
    assert_no_queued_action(&ledger);
    assert!(poll_machine(&page, &handle, &mut ledger, 3).is_pending());
    assert!(matches!(last_request(&ledger), InteractReq::CloseModal));
}

#[test]
fn unchanged_pending_purchase_polls_allocate_nothing_and_keep_bounded_state() {
    let mut client = client_with_shop(false);
    let closed = snapshot(&client, Vec::new(), vec![npc_row(7, 123, 2)]);
    let mut ledger = None;
    let handle = start_machine(&closed, &mut ledger, 5);
    let mut log = ActionLog::default();
    assert!(poll_machine(&closed, &handle, &mut ledger, 2).is_pending());
    drain_trade(&mut ledger, &mut log, 2, 7);
    open_shop(&mut client);
    let page = snapshot(&client, Vec::new(), Vec::new());
    assert!(poll_machine(&page, &handle, &mut ledger, 3).is_pending());
    drain_button(&mut ledger, &mut log, 3, 5);
    let measured = allocation_counter::measure(|| {
        for _ in 0..1_000 {
            assert!(poll_machine(&page, &handle, &mut ledger, 3).is_pending());
        }
    });
    assert_eq!(measured.count_total, 0);
    assert_no_queued_action(&ledger);
    assert_eq!(log.buttons, [5]);
    let machine_bytes = std::mem::size_of::<BuyMachine>();
    assert!(
        machine_bytes <= 512,
        "bounded shop stage grew to {machine_bytes} bytes"
    );
    println!(
        "A4 purchase state bytes: machine={machine_bytes} dispatch={} request={} receipt={}; unchanged_polls=1000 allocations={}",
        std::mem::size_of::<Dispatch>(),
        std::mem::size_of::<BuyRequest>(),
        std::mem::size_of::<BuyReceipt>(),
        measured.count_total,
    );
}

#[test]
fn three_hundred_stackable_units_are_counted_without_narrowing_or_saturation() {
    let (_listener, mut client) = attached_client_with_shop(false);
    let cache = Arc::get_mut(&mut client.cache).unwrap();
    cache.objs.resize(315, client::config::ObjType::default());
    cache.objs[314] = client::config::ObjType {
        id: 314,
        name: "Feather".into(),
        stackable: true,
        ..Default::default()
    };
    client.set_iface_mut(
        3900,
        IfTypeMut {
            link_obj_type: Some(vec![315, 0]),
            link_obj_number: Some(vec![1_000, 0]),
            ..Default::default()
        },
    );
    let npcs = vec![npc_row(7, 123, 2)];
    let closed = snapshot(&client, Vec::new(), npcs);
    let mut ledger = None;
    let handle = with_tick(&closed, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<BuyMachine>(
                BuyRequest {
                    item_id: 314,
                    item_name: Arc::from("Feather"),
                    qty: 300,
                    ..request(300)
                },
                &mut tick.cx,
            )
            .unwrap()
    });
    let mut log = ActionLog::default();
    assert!(poll_machine(&closed, &handle, &mut ledger, 2).is_pending());
    drain_trade(&mut ledger, &mut log, 2, 7);
    open_shop(&mut client);
    let page = snapshot(&client, Vec::new(), Vec::new());
    assert!(poll_machine(&page, &handle, &mut ledger, 3).is_pending());
    assert!(matches!(
        last_request(&ledger),
        InteractReq::ShopButton {
            id: 314,
            chunk: 10,
            ..
        }
    ));
    drain_interaction(&mut ledger, &mut log, 3);
    let mut inventory = Vec::new();
    for held in (10..=300).step_by(10) {
        let tick = 3 + u64::try_from(held / 10).unwrap();
        let mut feather = item(314, "Feather", held);
        feather.def.stackable = true;
        inventory = vec![feather];
        let page = snapshot(&client, inventory.clone(), Vec::new());
        assert!(poll_machine(&page, &handle, &mut ledger, tick).is_pending());
        assert!(held <= 300);
        if held < 300 {
            assert!(matches!(
                last_request(&ledger),
                InteractReq::ShopButton {
                    id: 314,
                    chunk: 10,
                    ..
                }
            ));
            drain_interaction(&mut ledger, &mut log, tick);
        } else {
            close_with_api(&mut client, &mut ledger, &mut log, tick);
        }
    }
    client.apply_if_close();
    let closed = snapshot(&client, inventory, Vec::new());
    let Poll::Ready(Ok(receipt)) = poll_machine(&closed, &handle, &mut ledger, 34) else {
        panic!("the counted final close must complete the 300-unit purchase");
    };
    assert_eq!(receipt.bought, 300);
    assert_eq!(receipt.held, 300);
    assert_eq!(log.buttons, vec![10; 30]);
    assert_eq!(
        log.buttons.len(),
        30,
        "one resolved dispatch per observed 10-unit transfer"
    );
    assert_eq!(log.closes, 1);
    assert_no_queued_action(&ledger);
}

#[test]
fn one_acquisition_buys_two_needs_and_waits_for_each_counted_final_close() {
    use crate::acquire::{
        plan, AcquireOutcome, AcquireRequest, AcquireVerdict, BankChoice, BankStage, CloseBank,
        CoinPolicy, Need, NeedKind, OpenGate, Presence, SourceSet, VendorChoice, WorldFacts,
    };

    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let bronze = data.item_by_alias("bronze_axe").unwrap();
    let iron = data.item_by_alias("iron_axe").unwrap();
    let keeper = api::shop_facts::vendor_by_keeper_config(&data, "bob", bronze.id)
        .unwrap()
        .unwrap();
    let (_listener, mut client) = attached_client_with_shop(false);
    let cache = Arc::get_mut(&mut client.cache).unwrap();
    cache.objs.resize(
        usize::try_from(bronze.id.max(iron.id)).unwrap() + 1,
        client::config::ObjType::default(),
    );
    for fact in [bronze, iron] {
        cache.objs[usize::try_from(fact.id).unwrap()] = client::config::ObjType {
            id: fact.id,
            name: fact.name.clone().unwrap(),
            ..Default::default()
        };
    }
    client.set_iface_mut(
        3900,
        IfTypeMut {
            link_obj_type: Some(vec![bronze.id + 1, iron.id + 1]),
            link_obj_number: Some(vec![10, 10]),
            ..Default::default()
        },
    );
    let npcs = || vec![npc_row(7, usize::try_from(keeper.keeper_id).unwrap(), 2)];
    let pack = |bronze_count, iron_count| {
        let mut rows = vec![item(995, "Coins", 1_000)];
        for (fact, count) in [(bronze, bronze_count), (iron, iron_count)] {
            if count > 0 {
                let mut row = item(fact.id, fact.name.as_deref().unwrap(), count);
                row.slot = i32::try_from(rows.len()).unwrap();
                rows.push(row);
            }
        }
        rows
    };
    let initial = snapshot(&client, pack(0, 0), npcs());
    let request = AcquireRequest {
        needs: [(bronze, 1), (iron, 2)]
            .into_iter()
            .map(|(fact, target)| Need {
                item: fact.id,
                name: Arc::from(fact.name.as_deref().unwrap()),
                target,
                minimum: target,
                presence: Presence::Held,
                kind: NeedKind::Required,
                sources: SourceSet::PACK | SourceSet::SHOP,
                alternatives: Arc::from([]),
            })
            .collect(),
        deposit: None,
        bank: BankChoice {
            bank: None,
            explicit: None,
            required: false,
            preferences: Default::default(),
        },
        stage: BankStage::Planned,
        partial_ok: true,
        close: CloseBank::Close,
        vendor: VendorChoice::Explicit {
            keeper_config: Arc::from("bob"),
            anchor: None,
        },
        coins: CoinPolicy::ForPurchase {
            sources: SourceSet::PACK,
        },
        walk: Default::default(),
        required_after: EvidenceStamp {
            run: api::selected::RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 0,
            sequence: 0,
        },
    };
    let banks = Arc::new(api::named_banks::NamedBankFacts::from_banks(Vec::new()));
    let mut ledger = None;
    let mut run = with_tick(&initial, &mut ledger, 1, |tick| {
        let AcquireVerdict::Plan(plan) = plan::plan(
            &request,
            &tick.cx.snapshot().stock(),
            WorldFacts {
                data: Some(&data),
                members: false,
            },
        )
        .unwrap() else {
            panic!("both missing axes require purchases");
        };
        assert_eq!(plan.purchase_count, 2);
        crate::acquire::run::AcquireRun::new(request, plan, &banks, &tick.cx)
    });
    let mut log = ActionLog::default();
    let poll = |run: &mut crate::acquire::run::AcquireRun,
                page: &GameSnapshot,
                ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
                tick| {
        with_tick(page, ledger, tick, |native| {
            run.poll(native, OpenGate::Open)
        })
    };

    assert!(poll(&mut run, &initial, &mut ledger, 2).is_pending());
    drain_interaction(&mut ledger, &mut log, 2);
    assert_eq!(log.trades, [7]);
    open_shop(&mut client);
    let page = snapshot(&client, pack(0, 0), npcs());
    assert!(poll(&mut run, &page, &mut ledger, 3).is_pending());
    assert!(
        matches!(last_request(&ledger), InteractReq::ShopButton { id, chunk: 1, .. } if *id == bronze.id)
    );
    drain_interaction(&mut ledger, &mut log, 3);
    let page = snapshot(&client, pack(1, 0), npcs());
    assert!(poll(&mut run, &page, &mut ledger, 4).is_pending());
    close_with_api(&mut client, &mut ledger, &mut log, 4);
    client.apply_if_close();
    let closed = snapshot(&client, pack(1, 0), npcs());
    assert!(poll(&mut run, &closed, &mut ledger, 5).is_pending());
    drain_interaction(&mut ledger, &mut log, 5);
    assert_eq!(log.trades, [7, 7]);

    open_shop(&mut client);
    let page = snapshot(&client, pack(1, 0), npcs());
    assert!(poll(&mut run, &page, &mut ledger, 6).is_pending());
    assert!(
        matches!(last_request(&ledger), InteractReq::ShopButton { id, chunk: 1, .. } if *id == iron.id)
    );
    drain_interaction(&mut ledger, &mut log, 6);
    let page = snapshot(&client, pack(1, 1), npcs());
    assert!(poll(&mut run, &page, &mut ledger, 7).is_pending());
    assert!(
        matches!(last_request(&ledger), InteractReq::ShopButton { id, chunk: 1, .. } if *id == iron.id)
    );
    drain_interaction(&mut ledger, &mut log, 7);
    let page = snapshot(&client, pack(1, 2), npcs());
    assert!(poll(&mut run, &page, &mut ledger, 8).is_pending());
    close_with_api(&mut client, &mut ledger, &mut log, 8);
    let locally_closed = snapshot(&client, pack(1, 2), npcs());
    assert!(poll(&mut run, &locally_closed, &mut ledger, 9).is_pending());
    assert_no_queued_action(&ledger);
    client.apply_if_close();
    let closed = snapshot(&client, pack(1, 2), npcs());
    let Poll::Ready(Ok(AcquireOutcome::Done(receipt))) = poll(&mut run, &closed, &mut ledger, 10)
    else {
        panic!("both counted final closes must precede acquisition completion");
    };
    assert!(receipt.complete);
    assert_eq!(receipt.bought, 3);
    assert_eq!(log.buttons, [1, 1, 1]);
    assert_eq!(log.buttons.len(), 3, "three resolved observed transfers");
    assert_eq!(log.closes, 2);
    assert_no_queued_action(&ledger);
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

/// A failed approach to an unreachable keeper is not terminal (AGENTS.md: a
/// walk failure is never terminal until its bounded retries are spent): the
/// walk is planned again from the observed position on a later tick.
#[test]
fn shop_keeper_walk_replans_after_a_failed_approach() {
    use crate::native::WalkEnd;
    use crate::quester::families::tests::{local_player, post_walk_receipt, with_tick_reach};
    if !api::test_support::game_data_available() {
        return;
    }
    let client = client_with_shop(false);
    let origin = WorldTile {
        x: 3190,
        z: 3200,
        level: 0,
    };
    let mut keeper = npc_row(7, 123, 0);
    keeper.distance = 10;
    let mut before = snapshot(&client, Vec::new(), vec![keeper]);
    before.seed_local_player(local_player(origin));
    let mut origin_reach = reach_from(origin);
    origin_reach.step.fill(0);
    let mut ledger = None;
    let handle = start_machine(&before, &mut ledger, 2);

    let first = with_tick_reach(&before, &origin_reach, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    });
    assert!(first.is_pending());
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        HostEffect::Walk(_)
    ));
    let first_request_id = ledger
        .as_ref()
        .unwrap()
        .outbox
        .last()
        .unwrap()
        .request_id
        .get();
    assert_eq!(
        post_walk_receipt(&mut ledger, 3, WalkEnd::NoPath, Some("first route failed")),
        first_request_id
    );

    let moved = WorldTile { x: 3191, ..origin };
    let mut after_failure = snapshot(&client, Vec::new(), vec![npc_row(7, 123, 0)]);
    after_failure.seed_local_player(local_player(moved));
    let moved_reach = reach_from(moved);
    let failed = with_tick_reach(&after_failure, &moved_reach, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    });
    assert!(failed.is_pending());
    assert!(
        !ledger.as_ref().unwrap().outbox.iter().any(|entry| {
            matches!(&entry.effect, HostEffect::Walk(_))
                && entry.request_id.get() != first_request_id
        }),
        "retry starts on a later tick"
    );

    let retry = with_tick_reach(&after_failure, &moved_reach, &mut ledger, 4, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    });
    assert!(retry.is_pending());
    let retry_request_id = ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .rev()
        .find(|entry| matches!(&entry.effect, HostEffect::Walk(_)))
        .unwrap()
        .request_id
        .get();
    assert_ne!(retry_request_id, first_request_id);
    assert_eq!(
        post_walk_receipt(&mut ledger, 5, WalkEnd::Arrived, None),
        retry_request_id
    );

    let target = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let mut arrived = snapshot(&client, Vec::new(), vec![npc_row(7, 123, 0)]);
    arrived.seed_local_player(local_player(target));
    let target_reach = reach_from(target);
    let result = with_tick_reach(&arrived, &target_reach, &mut ledger, 5, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    });
    assert!(result.is_pending());
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        HostEffect::Interaction(InteractReq::Npc {
            name,
            action,
            index: Some(7),
        }) if name == "Shopkeeper" && action == "Trade"
    ));
}

/// A spent keeper-walk budget is terminal, and the terminal failure keeps the
/// last receipt's own reason instead of only the shared core's fixed text.
/// The shared budget is QUEST_WALK_MAX_ATTEMPTS requests in all: the initial
/// one and two fresh plans from the observed position.
#[test]
fn shop_keeper_walk_terminal_failure_keeps_the_last_failure_detail() {
    use crate::native::WalkEnd;
    use crate::quester::families::tests::{local_player, post_walk_receipt, with_tick_reach};
    if !api::test_support::game_data_available() {
        return;
    }
    let client = client_with_shop(false);
    let origin = WorldTile {
        x: 3190,
        z: 3200,
        level: 0,
    };
    let mut keeper = npc_row(7, 123, 0);
    keeper.distance = 10;
    let mut before = snapshot(&client, Vec::new(), vec![keeper]);
    before.seed_local_player(local_player(origin));
    let mut origin_reach = reach_from(origin);
    origin_reach.step.fill(0);
    let mut ledger = None;
    let handle = start_machine(&before, &mut ledger, 2);

    let first = with_tick_reach(&before, &origin_reach, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    });
    assert!(first.is_pending());
    post_walk_receipt(&mut ledger, 3, WalkEnd::NoPath, Some("first route failed"));

    let moved = WorldTile { x: 3191, ..origin };
    let mut after_failure = snapshot(&client, Vec::new(), vec![npc_row(7, 123, 0)]);
    after_failure.seed_local_player(local_player(moved));
    let moved_reach = reach_from(moved);

    // Attempt 1 of 3: one fresh plan is admitted on the next tick.
    let failed = with_tick_reach(&after_failure, &moved_reach, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    });
    assert!(failed.is_pending());
    let retry = with_tick_reach(&after_failure, &moved_reach, &mut ledger, 4, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    });
    assert!(retry.is_pending());
    post_walk_receipt(&mut ledger, 5, WalkEnd::NoPath, Some("second route failed"));

    // Attempt 2 of 3: the last admitted fresh plan.
    let failed_again = with_tick_reach(&after_failure, &moved_reach, &mut ledger, 5, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    });
    assert!(failed_again.is_pending());
    let last = with_tick_reach(&after_failure, &moved_reach, &mut ledger, 6, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    });
    assert!(last.is_pending());
    post_walk_receipt(&mut ledger, 7, WalkEnd::Failed, Some("third route failed"));

    // Attempt 3 of 3 fails: the budget is spent, so the open is terminal and
    // names the last receipt's reason.
    let Poll::Ready(Err(ActionError::Failed(message))) =
        with_tick_reach(&after_failure, &moved_reach, &mut ledger, 7, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
    else {
        panic!("a spent keeper-walk budget is terminal");
    };
    assert!(
        message.contains("third route failed"),
        "the terminal failure must name the last receipt: {message}"
    );
}
