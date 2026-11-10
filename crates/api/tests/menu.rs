use api::query::menu_has_entry;
use api::snapshot::{
    menu_entries, MenuEntryQuery, MenuOperation, MenuTargetSelector, MenuTargetView,
};
use client::client::MiniMenuAction;
use client::client::{Client, ClientConfig, ClientNpc};
use client::io::ClientRevision;

fn measure_allocations(f: impl FnOnce()) -> allocation_counter::AllocationInfo {
    allocation_counter::measure(f)
}

fn cfg() -> ClientConfig {
    ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: false,
    }
}

fn client() -> Client {
    Client::new_with_revision(cfg(), ClientRevision::R377)
}

fn set_entry(client: &mut Client, index: usize, action: i32, params: [i32; 3]) {
    client.menu_action[index] = action;
    client.menu_param_a[index] = params[0];
    client.menu_param_b[index] = params[1];
    client.menu_param_c[index] = params[2];
}

fn set_npc(client: &mut Client, index: usize, type_id: usize) {
    client.npc[index] = Some(Box::new(ClientNpc {
        r#type: Some(type_id),
        ..ClientNpc::default()
    }));
}

fn hans_query() -> MenuEntryQuery {
    MenuEntryQuery {
        operation: MenuOperation::NpcOption(1),
        target: MenuTargetSelector::NpcType(0),
    }
}

#[test]
fn chat_accept_requests_do_not_claim_player_slot_zero() {
    let mut client = client();
    client.menu_num_entries = 2;
    set_entry(&mut client, 0, MiniMenuAction::ACCEPT_DUELREQ, [0, 0, 0]);
    set_entry(&mut client, 1, MiniMenuAction::ACCEPT_TRADEREQ, [0, 0, 0]);

    let entries: Vec<_> = menu_entries(&client).collect();
    assert_eq!(entries[0].target, MenuTargetView::Raw([0, 0, 0]));
    assert_eq!(entries[1].target, MenuTargetView::Raw([0, 0, 0]));
    for (operation, entry) in [
        (MenuOperation::AcceptDuelRequest, entries[0]),
        (MenuOperation::AcceptTradeRequest, entries[1]),
    ] {
        assert!(!MenuEntryQuery {
            operation,
            target: MenuTargetSelector::PlayerIndex(0),
        }
        .matches(entry));
    }
}

#[test]
fn loc_examine_decodes_typecode_from_packed_parameter() {
    let mut client = client();
    let loc_id = 0x1234;
    client.menu_num_entries = 1;
    set_entry(
        &mut client,
        0,
        MiniMenuAction::OP_LOC6,
        [loc_id << 14, 12, 34],
    );

    let entry = menu_entries(&client).next().unwrap();
    assert_eq!(entry.operation, MenuOperation::LocExamine);
    assert_eq!(
        entry.target,
        MenuTargetView::Loc {
            typecode: loc_id,
            x: 12,
            z: 34,
        }
    );
    assert!(MenuEntryQuery {
        operation: MenuOperation::LocExamine,
        target: MenuTargetSelector::Loc {
            typecode: loc_id,
            x: 12,
            z: 34,
        },
    }
    .matches(entry));
}

#[test]
fn menu_entries_decode_operation_and_structured_target_identity() {
    let mut client = client();
    set_npc(&mut client, 7, 0);
    set_npc(&mut client, 8, 1);
    client.menu_num_entries = 7;
    set_entry(
        &mut client,
        0,
        MiniMenuAction::_PRIORITY + MiniMenuAction::OP_NPC1,
        [7, 0, 0],
    );
    set_entry(&mut client, 1, MiniMenuAction::OP_NPC1, [8, 0, 0]);
    set_entry(&mut client, 2, MiniMenuAction::OP_LOC2, [0x1234, 12, 34]);
    set_entry(&mut client, 3, MiniMenuAction::OP_HELD4, [995, 3, 321]);
    set_entry(&mut client, 4, MiniMenuAction::IF_BUTTON, [0, 0, 876]);
    set_entry(&mut client, 5, MiniMenuAction::WALK, [0, 45, 67]);
    set_entry(&mut client, 6, 1999, [4, 5, 6]);
    client.menu_option[0] = "Attack @yel@not Hans".into();

    let entries: Vec<_> = menu_entries(&client).collect();
    assert_eq!(entries.len(), 7);
    assert_eq!(
        entries[0],
        api::snapshot::MenuEntryView {
            operation: MenuOperation::NpcOption(1),
            target: MenuTargetView::Npc {
                index: 7,
                type_id: Some(0),
            },
            priority: true,
        }
    );
    assert_eq!(
        entries[1].target,
        MenuTargetView::Npc {
            index: 8,
            type_id: Some(1),
        }
    );
    assert_eq!(
        entries[2],
        api::snapshot::MenuEntryView {
            operation: MenuOperation::LocOption(2),
            target: MenuTargetView::Loc {
                typecode: 0x1234,
                x: 12,
                z: 34,
            },
            priority: false,
        }
    );
    assert_eq!(
        entries[3].target,
        MenuTargetView::InventoryItem {
            id: 995,
            slot: 3,
            component_id: 321,
        }
    );
    assert_eq!(entries[4].target, MenuTargetView::Component { id: 876 });
    assert_eq!(
        entries[5].target,
        MenuTargetView::CursorPosition { x: 45, y: 67 }
    );
    assert_eq!(entries[6].operation, MenuOperation::Unknown(1999));
    assert_eq!(entries[6].target, MenuTargetView::Raw([4, 5, 6]));

    let hans = hans_query();
    assert!(menu_has_entry(&client, Some(hans)));
    assert!(!menu_has_entry(
        &client,
        Some(MenuEntryQuery {
            operation: MenuOperation::NpcOption(2),
            target: MenuTargetSelector::NpcType(0),
        })
    ));
    assert!(!menu_has_entry(
        &client,
        Some(MenuEntryQuery {
            operation: MenuOperation::NpcOption(1),
            target: MenuTargetSelector::NpcType(2),
        })
    ));
    assert!(!menu_has_entry(&client, None));

    client.menu_num_entries = -1;
    assert!(menu_entries(&client).next().is_none());
    assert!(!menu_has_entry(&client, Some(hans)));

    client.gens.session = client.gens.session.wrapping_add(1);
    client.menu_num_entries = 0;
    assert!(!menu_has_entry(&client, Some(hans)));
    client.menu_num_entries = 1;
    set_entry(&mut client, 0, MiniMenuAction::OP_NPC1, [7, 0, 0]);
    assert!(menu_has_entry(&client, Some(hans)));
}

#[test]
fn menu_query_allocates_nothing_armed_disarmed_and_after_session_replacement() {
    let mut client = client();
    set_npc(&mut client, 7, 0);
    client.menu_num_entries = 1;
    set_entry(&mut client, 0, MiniMenuAction::OP_NPC1, [7, 0, 0]);
    let query = hans_query();
    assert!(menu_has_entry(&client, Some(query)));

    const CALLS: usize = 1_000;
    let disarmed = measure_allocations(|| {
        for _ in 0..CALLS {
            std::hint::black_box(menu_has_entry(&client, None));
        }
    });
    let armed = measure_allocations(|| {
        for _ in 0..CALLS {
            std::hint::black_box(menu_has_entry(&client, Some(query)));
        }
    });
    let arm_disarm_cycles = measure_allocations(|| {
        for _ in 0..CALLS {
            std::hint::black_box(menu_has_entry(&client, Some(query)));
            std::hint::black_box(menu_has_entry(&client, None));
        }
    });
    let session_replacement = measure_allocations(|| {
        for _ in 0..CALLS {
            client.gens.session = client.gens.session.wrapping_add(1);
            client.menu_num_entries = 0;
            std::hint::black_box(menu_has_entry(&client, Some(query)));
            std::hint::black_box(menu_has_entry(&client, None));
            client.menu_num_entries = 1;
        }
    });

    for measured in [disarmed, armed, arm_disarm_cycles, session_replacement] {
        assert_eq!(measured.count_total, 0);
        assert_eq!(measured.bytes_total, 0);
    }
    assert!(menu_has_entry(&client, Some(query)));
}
