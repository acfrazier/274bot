use super::*;
use client::client::{ClientConfig, ClientRevision};
use client::config::if_type::ComponentType;
use client::config::{Cache, IfType, IfTypeMut};
use client::io::{Packet, ServerProt};
use std::sync::Arc;

fn client_with_inventory() -> Client {
    let mut interfaces = vec![None; 4];
    interfaces[3] = Some(Box::new(IfType {
        r#type: ComponentType::TYPE_INV,
        obj_ops: true,
        ..Default::default()
    }));

    let mut overlays = vec![None; 4];
    overlays[3] = Some(Arc::new(IfTypeMut {
        link_obj_type: Some(vec![0]),
        link_obj_number: Some(vec![0]),
        ..Default::default()
    }));

    let mut client = Client::from_shared_with_revision(
        ClientConfig {
            host: String::new(),
            port: 0,
            cache_dir: String::new(),
            members: true,
            lowmem: false,
        },
        Arc::new(Cache::default()),
        Arc::new(interfaces),
        Arc::new(overlays),
        ClientRevision::R274,
    );
    client.ingame = true;
    client.gens.session = 12;
    client.side_icon[3] = 3;
    client
}

fn host_session() -> SessionKey {
    SessionKey::new(SlotKey::from_incarnation(1).unwrap(), 4)
}

#[test]
fn current_watermark_is_available_without_an_armed_observer() {
    let client = client_with_inventory();
    let before = current_watermark(&client, host_session()).unwrap();
    assert_eq!(before.session, host_session());
    assert_eq!(before.client_session, 12);
    assert_eq!(before.source_serial, 0);
    assert_eq!(before.application_serial, 0);
}

#[test]
fn carried_selection_binds_exact_component_to_each_applied_event() {
    let mut client = client_with_inventory();
    let session = host_session();
    let adapter = ScenarioObserveAdapter::arm(
        &mut client,
        session,
        DecodedDomainSelection::new().carried_inventory(),
        4,
    )
    .unwrap();
    assert_eq!(adapter.carried_inventory_component(), Some(3));
    assert_eq!(adapter.inventory_scope(3), Some(InventoryScope::Carried));
    assert_eq!(adapter.inventory_scope(2), Some(InventoryScope::Other));

    let mut packet = Packet::new(vec![0, 3, 0]);
    client.handle_packet(ServerProt::UPDATE_INV_FULL, &mut packet);

    let mut events = [None; 4];
    let drain = adapter.drain(&mut client, &mut events);
    assert_eq!(drain.written, 2);
    for event in events[..2].iter().flatten() {
        assert_eq!(event.session, session);
        assert_eq!(event.inventory_scope, Some(InventoryScope::Carried));
    }
    assert!(matches!(
        events[0].unwrap().event,
        DecodedDomainEvent::InventoryMarker {
            component: 3,
            form: InventoryForm::Full,
            ..
        }
    ));
    assert!(matches!(
        events[1].unwrap().event,
        DecodedDomainEvent::InventorySlot {
            component: 3,
            slot: 0,
            item_id: None,
            count: 0,
            ..
        }
    ));
    assert_eq!(adapter.watermark(&client).unwrap().session, session);
}

#[test]
fn empty_selection_is_valid_and_missing_carried_scope_fails_closed() {
    let mut client = client_with_inventory();
    let adapter = ScenarioObserveAdapter::arm(
        &mut client,
        host_session(),
        DecodedDomainSelection::new(),
        0,
    )
    .unwrap();
    let mut events = [None; 1];
    let drain = adapter.drain(&mut client, &mut events);
    assert_eq!(drain.written, 0);
    assert_eq!(drain.remaining, 0);
    assert_eq!(drain.fault, None);

    let mut client = client_with_inventory();
    client.side_icon[3] = -1;
    assert!(matches!(
        ScenarioObserveAdapter::arm(
            &mut client,
            host_session(),
            DecodedDomainSelection::new().carried_inventory(),
            4,
        ),
        Err(ScenarioObserveError::MissingCarriedInventoryComponent)
    ));
    assert_eq!(
        client.decoded_domain_observer_watermark(),
        Err(DecodedDomainFault::NotArmed)
    );
}

#[test]
fn explicit_component_selection_does_not_infer_carried_scope() {
    let mut client = client_with_inventory();
    let adapter = ScenarioObserveAdapter::arm(
        &mut client,
        host_session(),
        DecodedDomainSelection::new().inventory_component(3),
        4,
    )
    .unwrap();
    assert_eq!(adapter.carried_inventory_component(), None);
    assert_eq!(adapter.inventory_scope(3), None);
}
