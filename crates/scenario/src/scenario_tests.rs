use super::scenarios::{
    combat::*, navigation::*, pair::*, production::*, script_basics::*, shop::*,
};
use super::*;

#[test]
fn budget_s_from_parses_rs2b0t_style_seconds() {
    assert_eq!(budget_s_from(None), None);
    assert_eq!(budget_s_from(Some("")), None);
    assert_eq!(budget_s_from(Some("0")), None);
    assert_eq!(budget_s_from(Some("nope")), None);
    assert_eq!(budget_s_from(Some("300")), Some(Duration::from_secs(300)));
    assert_eq!(budget_s_from(Some(" 60 ")), Some(Duration::from_secs(60)));
}

#[test]
fn native_seed_validation_requires_real_base_and_certificate_definitions() {
    let mut objs = vec![client::config::ObjType::default(); 3];
    objs[1].id = 1;
    objs[1].stackable = false;
    objs[2].id = 2;
    objs[2].stackable = true;
    objs[2].certlink = 1;
    objs[2].certtemplate = 0;
    let seed = NativeSeed {
        unnoted_id: 1,
        debug_alias: "base",
        note_alias: Some("cert_base"),
        quantity: 28,
        note_id: Some(2),
    };
    assert!(native_seed_definition_valid(&objs, seed));

    objs[2].certlink = 0;
    assert!(!native_seed_definition_valid(&objs, seed));
    objs[2].certlink = 1;
    objs[2].certtemplate = -1;
    assert!(!native_seed_definition_valid(&objs, seed));
    assert!(!native_seed_definition_valid(
        &objs,
        NativeSeed {
            note_id: None,
            ..seed
        }
    ));
    objs[2].certtemplate = 0;
    objs[2].stackable = false;
    assert!(!native_seed_definition_valid(&objs, seed));
    objs[2].stackable = true;
    objs[1].id = 99;
    assert!(!native_seed_definition_valid(&objs, seed));
    objs[1].id = 1;
    objs[1].certlink = 2;
    assert!(!native_seed_definition_valid(&objs, seed));
    objs[1].certlink = -1;
    objs[1].stackable = true;
    assert!(native_seed_definition_valid(
        &objs,
        NativeSeed {
            note_alias: None,
            note_id: None,
            quantity: 100,
            ..seed
        }
    ));
    objs[1].stackable = false;
    assert!(!native_seed_definition_valid(
        &objs,
        NativeSeed {
            note_alias: None,
            note_id: None,
            quantity: 28,
            ..seed
        }
    ));
    assert!(native_seed_definition_valid(
        &objs,
        NativeSeed {
            note_alias: None,
            note_id: None,
            quantity: 1,
            ..seed
        }
    ));
}

fn native_seed_client() -> Client {
    use client::client::ClientConfig;
    use client::dash3d::ClientPlayer;
    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: false,
    });
    client.ingame = true;
    client.scene_state = 2;
    client.map_build_base_x = 3200;
    client.map_build_base_z = 3200;
    client.local_player = Some(ClientPlayer::at(20, 20));
    plant_production_objs(&mut client);
    client
}

fn plant_obj(client: &mut Client, obj: client::config::ObjType) {
    let cache = std::sync::Arc::get_mut(&mut client.cache).expect("sole cache owner");
    let id = obj.id as usize;
    if cache.objs.len() <= id {
        cache
            .objs
            .resize(id + 1, client::config::ObjType::default());
    }
    cache.objs[id] = obj;
}

fn unnoted_obj(id: i32, stackable: bool) -> client::config::ObjType {
    client::config::ObjType {
        id,
        stackable,
        certlink: -1,
        certtemplate: -1,
        ..Default::default()
    }
}

fn certificate_obj(note_id: i32, base_id: i32) -> client::config::ObjType {
    client::config::ObjType {
        id: note_id,
        stackable: true,
        certlink: base_id,
        certtemplate: 0,
        ..Default::default()
    }
}

fn plant_production_objs(client: &mut Client) {
    for obj in [
        unnoted_obj(HAMMER_ID, false),
        unnoted_obj(BRONZE_BAR_ID, false),
        certificate_obj(BRONZE_BAR_CERT_ID, BRONZE_BAR_ID),
        unnoted_obj(STEEL_BAR_ID, false),
        certificate_obj(STEEL_BAR_CERT_ID, STEEL_BAR_ID),
        unnoted_obj(MITHRIL_BAR_ID, false),
        certificate_obj(MITHRIL_BAR_CERT_ID, MITHRIL_BAR_ID),
        unnoted_obj(NEEDLE_ID, true),
        unnoted_obj(THREAD_ID, true),
        unnoted_obj(COINS_ID, true),
        unnoted_obj(SOFT_LEATHER_ID, false),
        certificate_obj(LEATHER_CERT_ID, SOFT_LEATHER_ID),
        unnoted_obj(HARD_LEATHER_ID, false),
        certificate_obj(HARD_LEATHER_CERT_ID, HARD_LEATHER_ID),
        unnoted_obj(GREEN_DRAGON_LEATHER_ID, false),
        certificate_obj(GREEN_DRAGON_LEATHER_CERT_ID, GREEN_DRAGON_LEATHER_ID),
        unnoted_obj(TINDERBOX_ID, false),
        unnoted_obj(LOGS_ID, false),
        certificate_obj(LOGS_CERT_ID, LOGS_ID),
        unnoted_obj(OAK_LOGS_ID, false),
        certificate_obj(OAK_LOGS_CERT_ID, OAK_LOGS_ID),
        unnoted_obj(LOBSTER_ID, false),
        certificate_obj(NOTED_LOBSTER_ID, LOBSTER_ID),
    ] {
        plant_obj(client, obj);
    }
}

fn seed_step(scenario: &Scenario) -> &Step {
    scenario
        .steps
        .iter()
        .find(|step| {
            matches!(step.kind, StepKind::Perform { .. })
                && (step.name.starts_with("seed Smithing")
                    || step.name.starts_with("seed Crafting")
                    || step.name.starts_with("seed Firemaking")
                    || step.name.starts_with("seed banked lobster")
                    || step.name.starts_with("seed banked coins"))
        })
        .unwrap_or_else(|| panic!("{} has a native inventory seed", scenario.name))
}

fn send_seed(name: &str, client: &mut Client) -> (bool, String) {
    let scenario = get(name).unwrap_or_else(|| panic!("{name} is registered"));
    let step = seed_step(&scenario);
    let StepKind::Perform { send } = &step.kind else {
        panic!("{name} seed must be a Perform step");
    };
    let snapshot = GameSnapshot::new();
    let before = client.out.pos;
    let ok = send(client, &snapshot);
    let written = String::from_utf8_lossy(&client.out.data()[before..client.out.pos]).into_owned();
    (ok, written)
}

fn send_combat_seed(name: &str, client: &mut Client) -> (bool, String) {
    let scenario = get(name).unwrap_or_else(|| panic!("{name} is registered"));
    let step = scenario
        .steps
        .iter()
        .find(|step| {
            step.name == "prepare melee stats, food and gear on the safe tile before Start"
        })
        .unwrap_or_else(|| panic!("{name} has a melee preparation seed"));
    let StepKind::Perform { send } = &step.kind else {
        panic!("{name} seed must be a Perform step");
    };
    let snapshot = GameSnapshot::new();
    let before = client.out.pos;
    let ok = send(client, &snapshot);
    let written = String::from_utf8_lossy(&client.out.data()[before..client.out.pos]).into_owned();
    (ok, written)
}

fn attach_loopback(client: &mut Client) -> (std::net::TcpListener, std::net::TcpStream) {
    use client::io::{ClientStream, ServerProt};
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    client.stream = Some(ClientStream::connect("127.0.0.1", port).unwrap());
    let (peer, _) = listener.accept().unwrap();
    client.bump_gens(ServerProt::UPDATE_INV_FULL);
    (listener, peer)
}

fn plant_bank_side(client: &mut Client, id: i32, count: i32) {
    use client::config::if_type::{ComponentType, IfType, IfTypeMut};
    client.set_iface(
        700,
        IfType {
            id: 700,
            layer_id: 700,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![701]),
            ..Default::default()
        },
    );
    client.set_iface(
        701,
        IfType {
            id: 701,
            layer_id: 700,
            r#type: ComponentType::TYPE_INV,
            iop: [Some("Deposit All".into()), None, None, None, None],
            ..Default::default()
        },
    );
    client.set_iface_mut(
        701,
        IfTypeMut {
            link_obj_type: Some(vec![id + 1, 0]),
            link_obj_number: Some(vec![count, 0]),
            ..Default::default()
        },
    );
    client.side_modal_id = 700;
}

fn deposit_step(scenario: &Scenario, id: i32, count: i32) -> &Step {
    scenario
        .steps
        .iter()
        .find(|step| {
            matches!(
                (&step.kind, &step.wait.arm),
                (
                    StepKind::Repeat { .. },
                    Proof::BankItemId {
                        id: wait_id,
                        count: wait_count
                    }
                ) if *wait_id == id && *wait_count == count
            )
        })
        .unwrap_or_else(|| {
            panic!(
                "{} deposits obj {id} x{count} through a Repeat wait",
                scenario.name
            )
        })
}

fn send_deposit(name: &str, id: i32, count: i32, side_id: i32, side_count: i32) -> (bool, i32) {
    use api::snapshot::Family;
    let mut client = native_seed_client();
    plant_bank_side(&mut client, side_id, side_count);
    let _peer = attach_loopback(&mut client);
    let mut snapshot = GameSnapshot::new();
    assert!(snapshot.rebuild_family(&client, Family::BankSide));
    assert!(
        snapshot.rebuild(&client) || !snapshot.bank_side().is_empty(),
        "bank-side rows must rebuild from the planted deposit component"
    );
    if snapshot.bank_side().is_empty() {
        client.bump_gens(client::io::ServerProt::UPDATE_INV_FULL);
        assert!(snapshot.rebuild_family(&client, Family::BankSide));
    }
    assert_eq!(
        snapshot
            .bank_side()
            .iter()
            .map(|item| item.def.id)
            .collect::<Vec<_>>(),
        vec![side_id],
        "bank-side must expose the planted inv row"
    );
    let scenario = get(name).unwrap_or_else(|| panic!("{name} is registered"));
    let step = deposit_step(&scenario, id, count);
    let StepKind::Repeat { send } = &step.kind else {
        panic!("{name} deposit must be a Repeat step");
    };
    let ok = send(&mut client, &snapshot);
    (ok, client.menu_param_a.first().copied().unwrap_or(-1))
}

#[test]
fn production_native_seeds_give_verified_certificates_through_real_send() {
    for (name, expected, forbidden) in [
        (
            "smithing_bot",
            &["give hammer 1", "give cert_bronze_bar 28"][..],
            &["givebank", "give bronze_bar 28", "bronze_bar_cert"][..],
        ),
        (
            "smithing_bot_platebody",
            &["give hammer 1", "give cert_bronze_bar 30"],
            &["givebank", "give cert_bronze_bar 28", "give bronze_bar 30"],
        ),
        (
            "smithing_bot_nails",
            &["give hammer 1", "give cert_steel_bar 28"],
            &[
                "givebank",
                "give steel_bar 28",
                "cert_bronze_bar",
                "bronze_bar",
            ],
        ),
        (
            "smithing_bot_mithril",
            &["give hammer 1", "give cert_mithril_bar 28"],
            &[
                "givebank",
                "give mithril_bar 28",
                "cert_bronze_bar",
                "bronze_bar",
            ],
        ),
        (
            "leather_crafter",
            &["give needle 1", "give thread 100", "give cert_leather 28"],
            &[
                "givebank",
                "give leather 28",
                "leather_cert",
                "cert_hard_leather",
                "cert_dragon_leather",
            ],
        ),
        (
            "leather_crafter_hard_body",
            &[
                "give needle 1",
                "give thread 100",
                "give cert_hard_leather 28",
            ],
            &[
                "givebank",
                "give hard_leather 28",
                "give cert_leather 28",
                "cert_dragon_leather",
            ],
        ),
        (
            "leather_crafter_green_body",
            &[
                "give needle 1",
                "give thread 100",
                "give cert_dragon_leather 56",
            ],
            &[
                "givebank",
                "give dragon_leather 56",
                "give cert_leather 28",
                "cert_hard_leather",
            ],
        ),
        (
            "leather_crafter_chaps",
            &["give needle 1", "give thread 100", "give cert_leather 56"],
            &[
                "givebank",
                "give leather 56",
                "give cert_leather 28",
                "cert_hard_leather",
                "cert_dragon_leather",
            ],
        ),
        (
            "leather_crafter_thread_shop",
            &["give needle 1", "give coins 1000", "give cert_leather 28"],
            &[
                "givebank",
                "give thread",
                "give leather 28",
                "cert_hard_leather",
                "cert_dragon_leather",
            ],
        ),
        (
            "firemaker",
            &["give tinderbox 1", "give cert_logs 28"],
            &["givebank", "give logs 28", "logs_cert", "cert_oak_logs"],
        ),
        (
            "firemaker_oak",
            &["give tinderbox 1", "give cert_oak_logs 28"],
            &["givebank", "give oak_logs 28", "give cert_logs 28"],
        ),
        (
            "herblore_secondaries",
            &["give cert_lobster 50"][..],
            &["givebank", "give lobster 50", "lobster_cert", "give coins"][..],
        ),
        (
            "herblore_secondaries_newt",
            &["give coins 5000"],
            &[
                "givebank",
                "give cert_coins",
                "cert_lobster",
                "give lobster",
            ],
        ),
    ] {
        let mut client = native_seed_client();
        let (ok, written) = send_seed(name, &mut client);
        assert!(
            ok,
            "{name} seed must accept loaded certificate defs: {written}"
        );
        for needle in expected {
            assert!(
                written.contains(needle),
                "{name} must send {needle}: {written}"
            );
        }
        for needle in forbidden {
            assert!(
                !written.contains(needle),
                "{name} must not send {needle}: {written}"
            );
        }
    }
}

#[test]
fn production_native_seed_send_rejects_broken_certificate_definitions() {
    let mut client = native_seed_client();
    {
        let cache = std::sync::Arc::get_mut(&mut client.cache).expect("sole cache owner");
        cache.objs[BRONZE_BAR_CERT_ID as usize].certlink = 0;
    }
    let (ok, written) = send_seed("smithing_bot", &mut client);
    assert!(!ok, "wrong certlink must fail closed: {written}");
    assert!(
        !written.contains("give "),
        "rejected seed must not send give: {written}"
    );

    let mut client = native_seed_client();
    {
        let cache = std::sync::Arc::get_mut(&mut client.cache).expect("sole cache owner");
        cache.objs[BRONZE_BAR_CERT_ID as usize].certtemplate = -1;
    }
    assert!(!send_seed("smithing_bot", &mut client).0);

    let mut client = native_seed_client();
    {
        let cache = std::sync::Arc::get_mut(&mut client.cache).expect("sole cache owner");
        cache.objs[BRONZE_BAR_CERT_ID as usize].stackable = false;
    }
    assert!(!send_seed("smithing_bot", &mut client).0);

    let mut client = native_seed_client();
    {
        let cache = std::sync::Arc::get_mut(&mut client.cache).expect("sole cache owner");
        cache.objs[HARD_LEATHER_CERT_ID as usize].certlink = SOFT_LEATHER_ID;
    }
    assert!(!send_seed("leather_crafter_hard_body", &mut client).0);

    let mut client = native_seed_client();
    {
        let cache = std::sync::Arc::get_mut(&mut client.cache).expect("sole cache owner");
        cache.objs[NOTED_LOBSTER_ID as usize].certlink = 0;
    }
    assert!(!send_seed("herblore_secondaries", &mut client).0);
}

#[test]
fn production_native_deposit_dispatches_certificate_rows_by_inv_id() {
    let (ok, dispatched) = send_deposit("smithing_bot", BRONZE_BAR_ID, 28, BRONZE_BAR_CERT_ID, 28);
    assert!(
        ok,
        "noted bronze bars must deposit from the certificate row"
    );
    assert_eq!(
        dispatched, BRONZE_BAR_CERT_ID,
        "Deposit All must fire on the note inv id, not the unnoted base"
    );

    let (ok, _) = send_deposit("smithing_bot", BRONZE_BAR_ID, 28, BRONZE_BAR_ID, 28);
    assert!(
        !ok,
        "unnoted bronze bars on bank-side must not satisfy the noted seed"
    );

    let (ok, dispatched) = send_deposit("smithing_bot", HAMMER_ID, 1, HAMMER_ID, 1);
    assert!(ok, "single unnoted hammer still deposits as itself");
    assert_eq!(dispatched, HAMMER_ID);

    let (ok, dispatched) = send_deposit(
        "leather_crafter_hard_body",
        HARD_LEATHER_ID,
        28,
        HARD_LEATHER_CERT_ID,
        28,
    );
    assert!(ok, "hard leather must deposit cert_hard_leather");
    assert_eq!(dispatched, HARD_LEATHER_CERT_ID);

    let (ok, _) = send_deposit(
        "leather_crafter_hard_body",
        HARD_LEATHER_ID,
        28,
        LEATHER_CERT_ID,
        28,
    );
    assert!(
        !ok,
        "soft leather certificates must not seed the hard variant"
    );

    let (ok, dispatched) = send_deposit(
        "leather_crafter_green_body",
        GREEN_DRAGON_LEATHER_ID,
        56,
        GREEN_DRAGON_LEATHER_CERT_ID,
        56,
    );
    assert!(ok, "green dragon leather must deposit cert_dragon_leather");
    assert_eq!(dispatched, GREEN_DRAGON_LEATHER_CERT_ID);

    let (ok, _) = send_deposit(
        "leather_crafter_green_body",
        GREEN_DRAGON_LEATHER_ID,
        56,
        LEATHER_CERT_ID,
        56,
    );
    assert!(
        !ok,
        "soft leather certificates must not seed the green dragon variant"
    );

    let (ok, dispatched) = send_deposit(
        "leather_crafter_chaps",
        SOFT_LEATHER_ID,
        56,
        LEATHER_CERT_ID,
        56,
    );
    assert!(ok, "chaps must deposit cert_leather");
    assert_eq!(dispatched, LEATHER_CERT_ID);

    let (ok, dispatched) = send_deposit(
        "leather_crafter_thread_shop",
        SOFT_LEATHER_ID,
        28,
        LEATHER_CERT_ID,
        28,
    );
    assert!(ok, "thread shop must deposit cert_leather");
    assert_eq!(dispatched, LEATHER_CERT_ID);

    let (ok, dispatched) = send_deposit(
        "leather_crafter_thread_shop",
        COINS_ID,
        LEATHER_THREAD_SHOP_COIN_SEED,
        COINS_ID,
        LEATHER_THREAD_SHOP_COIN_SEED,
    );
    assert!(ok, "thread shop must deposit stackable coins");
    assert_eq!(dispatched, COINS_ID);

    let (ok, dispatched) = send_deposit("firemaker_oak", OAK_LOGS_ID, 28, OAK_LOGS_CERT_ID, 28);
    assert!(ok, "oak logs must deposit cert_oak_logs");
    assert_eq!(dispatched, OAK_LOGS_CERT_ID);

    let (ok, dispatched) = send_deposit(
        "smithing_bot_platebody",
        BRONZE_BAR_ID,
        30,
        BRONZE_BAR_CERT_ID,
        30,
    );
    assert!(ok, "platebody must deposit the 30-bar certificate stack");
    assert_eq!(dispatched, BRONZE_BAR_CERT_ID);

    let (ok, dispatched) = send_deposit(
        "smithing_bot_nails",
        STEEL_BAR_ID,
        28,
        STEEL_BAR_CERT_ID,
        28,
    );
    assert!(ok, "nails must deposit the steel bar certificate stack");
    assert_eq!(dispatched, STEEL_BAR_CERT_ID);

    let (ok, dispatched) = send_deposit(
        "smithing_bot_mithril",
        MITHRIL_BAR_ID,
        28,
        MITHRIL_BAR_CERT_ID,
        28,
    );
    assert!(ok, "mithril must deposit the mithril bar certificate stack");
    assert_eq!(dispatched, MITHRIL_BAR_CERT_ID);

    let (ok, dispatched) = send_deposit(
        "herblore_secondaries",
        LOBSTER_ID,
        HERBLORE_EGG_FOOD_SEED,
        NOTED_LOBSTER_ID,
        HERBLORE_EGG_FOOD_SEED,
    );
    assert!(ok, "herblore eggs must deposit cert_lobster");
    assert_eq!(dispatched, NOTED_LOBSTER_ID);

    let (ok, _) = send_deposit(
        "herblore_secondaries",
        LOBSTER_ID,
        HERBLORE_EGG_FOOD_SEED,
        LOBSTER_ID,
        HERBLORE_EGG_FOOD_SEED,
    );
    assert!(
        !ok,
        "unnoted lobster on bank-side must not satisfy the noted seed"
    );

    let (ok, dispatched) = send_deposit(
        "herblore_secondaries_newt",
        COINS_ID,
        HERBLORE_NEWT_COIN_SEED,
        COINS_ID,
        HERBLORE_NEWT_COIN_SEED,
    );
    assert!(ok, "herblore newt must deposit stackable coins");
    assert_eq!(dispatched, COINS_ID);
}

/// Eggs headed FAIL: deposit hard-rejected with bank UI open, gen1,
/// bank_loaded=false, bank_side empty. Soft-wait on that transient.
#[test]
fn herblore_native_deposit_soft_waits_open_unloaded_bank() {
    use api::snapshot::Family;
    use client::config::if_type::{ComponentType, IfType, IfTypeMut};
    use client::io::{Packet, ServerProt};

    let mut client = native_seed_client();
    client.set_iface(
        600,
        IfType {
            id: 600,
            layer_id: 600,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![601]),
            ..Default::default()
        },
    );
    client.set_iface(
        601,
        IfType {
            id: 601,
            layer_id: 600,
            r#type: ComponentType::TYPE_INV,
            iop: [Some("Withdraw 1".into()), None, None, None, None],
            ..Default::default()
        },
    );
    client.set_iface_mut(
        601,
        IfTypeMut {
            link_obj_type: Some(vec![0]),
            link_obj_number: Some(vec![0]),
            ..Default::default()
        },
    );
    let _peer = attach_loopback(&mut client);
    let mut open = Packet::new(vec![2, 88]);
    client.handle_packet(ServerProt::IF_OPENMAIN, &mut open);

    let mut snapshot = GameSnapshot::new();
    assert!(snapshot.rebuild_family(&client, Family::Bank));
    assert!(
        snapshot.bank_component_id() >= 0,
        "bank UI must be open like the headed eggs capture"
    );
    assert!(
        !snapshot.bank_loaded(),
        "open without a full must stay unloaded"
    );
    assert!(snapshot.bank_side().is_empty());

    let scenario = get("herblore_secondaries").expect("herblore_secondaries");
    let step = deposit_step(&scenario, LOBSTER_ID, HERBLORE_EGG_FOOD_SEED);
    let StepKind::Repeat { send } = &step.kind else {
        panic!("deposit must be Repeat");
    };
    let before_out = client.out.pos;
    assert!(
        send(&mut client, &snapshot),
        "open unloaded bank must soft-wait, not hard-reject like headed eggs"
    );
    assert_eq!(
        client.out.pos, before_out,
        "soft-wait must not emit a deposit op while bank_side is unpublished"
    );
}

/// Eggs headed root cause: Repeat open re-sends Use-quickly after the
/// bank is already loaded. Skip that send so deposit still sees a loaded
/// current session.
#[test]
fn herblore_open_seed_bank_skips_booth_when_current_bank_loaded() {
    use api::snapshot::Family;
    use client::config::if_type::{ComponentType, IfType, IfTypeMut};
    use client::io::{Packet, ServerProt};

    let mut client = native_seed_client();
    client.set_iface(
        600,
        IfType {
            id: 600,
            layer_id: 600,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![601]),
            ..Default::default()
        },
    );
    client.set_iface(
        601,
        IfType {
            id: 601,
            layer_id: 600,
            r#type: ComponentType::TYPE_INV,
            iop: [Some("Withdraw 1".into()), None, None, None, None],
            ..Default::default()
        },
    );
    client.set_iface_mut(
        601,
        IfTypeMut {
            link_obj_type: Some(vec![0]),
            link_obj_number: Some(vec![0]),
            ..Default::default()
        },
    );
    let _peer = attach_loopback(&mut client);
    let mut full = Packet::new(vec![2, 89, 0]);
    client.handle_packet(ServerProt::UPDATE_INV_FULL, &mut full);
    let mut open = Packet::new(vec![2, 88]);
    client.handle_packet(ServerProt::IF_OPENMAIN, &mut open);

    let mut snapshot = GameSnapshot::new();
    assert!(snapshot.rebuild_family(&client, Family::Bank));
    assert!(snapshot.bank_component_id() >= 0);
    assert!(
        snapshot.bank_loaded(),
        "full before open must mark the current session loaded"
    );

    let scenario = get("herblore_secondaries").expect("herblore_secondaries");
    let open_step = scenario
        .steps
        .iter()
        .find(|step| step.name == "open and acknowledge the lobster seed bank")
        .expect("eggs lobster bank open");
    let StepKind::Repeat { send } = &open_step.kind else {
        panic!("eggs open must be Repeat");
    };
    let before = client.out.pos;
    assert!(
        send(&mut client, &snapshot),
        "loaded current bank must skip booth re-open"
    );
    assert_eq!(
        client.out.pos, before,
        "skip must not emit another booth Use-quickly"
    );
}

/// Capture 2026-09-18T06-11-18: booth 2213@3091,3243 is not operable from
/// canonical DRAYNOR_BANK 3093,3243 (Chebyshev 2). The seed stand must be
/// the stock-facing open adjacent, or open_booth_at refuses Unreachable.
#[test]
fn herblore_newt_approach_operates_draynor_booth_old_stand_does_not() {
    use api::query::loc_approach;
    use api::snapshot::{LocLayer, LocView, SceneView};

    let booth = DRAYNOR_BANK_BOOTH;
    let loc = LocView {
        typecode: 0,
        info: 0,
        id: DRAYNOR_BANK_BOOTH_ID,
        name: Some("Bank booth".into()),
        description: None,
        actions: vec![Some("Use".into()), Some("Use-quickly".into())],
        tile: booth,
        distance: 0,
        layer: LocLayer::Ground,
        shape: 10,
        angle: 1,
        width: 1,
        length: 1,
        footprint_width: 1,
        footprint_length: 1,
        block_walk: true,
        block_range: true,
        active: true,
        animation: -1,
        map_function: -1,
        map_scene: -1,
        force_approach: 0,
    };
    // Capture scene base 3040,3192; only the booth column and two east
    // tiles matter. Booth tile 0x100 = WALK_SCENERY; approach and old
    // stand are open 0.
    let base_x = 3040;
    let base_z = 3192;
    let width = 104;
    let height = 104;
    let mut flags = vec![0; (width * height) as usize];
    let idx = |x: i32, z: i32| ((x - base_x) * height + (z - base_z)) as usize;
    flags[idx(booth.x, booth.z)] = client::dash3d::CollisionFlag::WALK_SCENERY;
    let scene = SceneView {
        available: true,
        base_x,
        base_z,
        level: 0,
        width,
        height,
        collision_flags: flags,
    };
    assert_eq!(
        loc_approach::can_operate_from(&loc, &scene, DRAYNOR_BANK_APPROACH),
        Some(true),
        "east-adjacent 3092,3243 must operate the capture booth"
    );
    assert_eq!(
        loc_approach::can_operate_from(&loc, &scene, DRAYNOR_BANK),
        Some(false),
        "canonical 3093,3243 is Chebyshev 2 and must not operate"
    );

    let newt = get("herblore_secondaries_newt").expect("herblore_secondaries_newt");
    let start = newt
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("newt starts");
    let seed = newt.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::ArrivedNear {
        x: DRAYNOR_BANK_APPROACH.x,
        z: DRAYNOR_BANK_APPROACH.z,
        level: DRAYNOR_BANK_APPROACH.level,
        radius: 8,
    }));
    assert!(!seed.contains(&Proof::ArrivedNear {
        x: DRAYNOR_BANK.x,
        z: DRAYNOR_BANK.z,
        level: DRAYNOR_BANK.level,
        radius: 8,
    }));
}

#[test]
fn smithing_bot_deposit_watch_covers_full_first_trip_only() {
    for (name, product_min) in [
        ("smithing_bot", 1),
        ("smithing_bot_platebody", 1),
        ("smithing_bot_nails", STEEL_NAILS_OUTPUT),
        ("smithing_bot_mithril", 1),
    ] {
        let s = get(name).unwrap_or_else(|| panic!("{name} is registered"));
        assert_eq!(
            s.settings.deadline, SCRIPT_GOLD_DEADLINE,
            "{name}: global deadline stays 180s"
        );
        let deposit = s
            .steps
            .iter()
            .find(|step| step.name == "watch script-smithed product enter a fresh bank")
            .unwrap_or_else(|| panic!("{name} has product deposit watch"));
        assert_eq!(
                deposit.wait.budget_ticks, SMITHING_PRODUCT_DEPOSIT_WATCH_TICKS,
                "{name}: deposit dirty-budget covers remaining forge (engine 5/bar + L1-4 + Make-10 + bank), not equated to engine ticks"
            );
        assert!(
            matches!(
                deposit.wait.arm,
                Proof::BankItemId { count, .. } if count == product_min
            ),
            "{name}: deposit proof stays fresh bank item ≥{product_min}"
        );
        let product = s
            .steps
            .iter()
            .find(|step| step.name == "watch the selected smithing product after Start")
            .unwrap_or_else(|| panic!("{name} watches pack product"));
        assert_eq!(
            product.wait.budget_ticks, SCRIPT_GOLD_WATCH_TICKS,
            "{name}: first-product arm keeps the ordinary gold watch"
        );
        assert!(
            matches!(
                product.wait.arm,
                Proof::ItemId { count, .. } if count == product_min
            ),
            "{name}: pack product proves ≥{product_min}"
        );
        let further = s
            .steps
            .iter()
            .find(|step| step.name == "watch further Smithing XP after restock")
            .unwrap_or_else(|| panic!("{name} watches post-restock XP"));
        assert_eq!(
            further.wait.budget_ticks, SCRIPT_GOLD_WATCH_TICKS,
            "{name}: post-restock arm is not loosened"
        );
        assert!(
            matches!(further.wait.arm, Proof::FreshStatXpGain { .. }),
            "{name}: further XP stays FreshStatXpGain"
        );
    }
}

#[test]
fn herblore_eggs_deposit_watch_covers_natural_cycle_only() {
    let eggs = get("herblore_secondaries").expect("herblore_secondaries");
    assert_eq!(
        eggs.settings.deadline, HERBLORE_EGG_DEADLINE,
        "eggs wall covers food-exhaust + dungeon bank + return; newt keeps 180s"
    );
    let deposit = eggs
        .steps
        .iter()
        .find(|step| step.name == "watch script-taken eggs enter a fresh Edgeville bank")
        .expect("deposit watch");
    assert_eq!(
            deposit.wait.budget_ticks, HERBLORE_EGG_DEPOSIT_WATCH_TICKS,
            "deposit dirty-budget is runner increments sized to 100-tick respawn + remaining food + dungeon, not equated to engine ticks"
        );
    assert!(
        matches!(
            deposit.wait.arm,
            Proof::BankItemId {
                id: RED_SPIDERS_EGGS_ID,
                count: 1
            }
        ),
        "deposit proof stays fresh bank 223 ≥1"
    );
    let first = eggs
        .steps
        .iter()
        .find(|step| step.name == "watch exact red spiders' eggs 223 from the ground after Start")
        .expect("first egg");
    assert_eq!(
        first.wait.budget_ticks, SCRIPT_GOLD_WATCH_TICKS,
        "first-egg arm keeps the ordinary gold watch"
    );
    let empty = eggs
        .steps
        .iter()
        .find(|step| step.name == "watch the pack empty of eggs after deposit")
        .expect("empty pack");
    assert_eq!(
        empty.wait.budget_ticks, SCRIPT_GOLD_WATCH_TICKS,
        "empty-pack arm is not loosened"
    );
    let close = eggs
        .steps
        .iter()
        .find(|step| step.name == "watch the script close its egg bank")
        .expect("close");
    assert_eq!(
        close.wait.budget_ticks, SCRIPT_GOLD_WATCH_TICKS,
        "close arm is not loosened"
    );
    let ret = eggs
        .steps
        .iter()
        .find(|step| step.name == "watch return to the egg field after banking")
        .expect("return");
    assert_eq!(
        ret.wait.budget_ticks, HERBLORE_EGG_RETURN_WATCH_TICKS,
        "return dirty-budget covers reverse dungeon only"
    );
    let further = eggs
        .steps
        .iter()
        .find(|step| step.name == "watch further exact eggs after return")
        .expect("further");
    assert_eq!(
        further.wait.budget_ticks, SCRIPT_GOLD_WATCH_TICKS,
        "further-egg arm is not loosened"
    );
    let newt = get("herblore_secondaries_newt").expect("herblore_secondaries_newt");
    assert_eq!(
        newt.settings.deadline, SCRIPT_GOLD_DEADLINE,
        "newt deadline stays 180s"
    );
}

#[test]
fn production_native_seed_proofs_bound_exact_counts_before_start() {
    for (name, note_id, note_count, unnoted_id) in [
        ("smithing_bot", BRONZE_BAR_CERT_ID, 28, BRONZE_BAR_ID),
        (
            "smithing_bot_platebody",
            BRONZE_BAR_CERT_ID,
            30,
            BRONZE_BAR_ID,
        ),
        ("smithing_bot_nails", STEEL_BAR_CERT_ID, 28, STEEL_BAR_ID),
        (
            "smithing_bot_mithril",
            MITHRIL_BAR_CERT_ID,
            28,
            MITHRIL_BAR_ID,
        ),
        ("leather_crafter", LEATHER_CERT_ID, 28, SOFT_LEATHER_ID),
        (
            "leather_crafter_hard_body",
            HARD_LEATHER_CERT_ID,
            28,
            HARD_LEATHER_ID,
        ),
        (
            "leather_crafter_green_body",
            GREEN_DRAGON_LEATHER_CERT_ID,
            56,
            GREEN_DRAGON_LEATHER_ID,
        ),
        (
            "leather_crafter_chaps",
            LEATHER_CERT_ID,
            56,
            SOFT_LEATHER_ID,
        ),
        (
            "leather_crafter_thread_shop",
            LEATHER_CERT_ID,
            28,
            SOFT_LEATHER_ID,
        ),
        ("firemaker", LOGS_CERT_ID, 28, LOGS_ID),
        ("firemaker_oak", OAK_LOGS_CERT_ID, 28, OAK_LOGS_ID),
        (
            "herblore_secondaries",
            NOTED_LOBSTER_ID,
            HERBLORE_EGG_FOOD_SEED,
            LOBSTER_ID,
        ),
    ] {
        let scenario = get(name).unwrap_or_else(|| panic!("{name} is registered"));
        let start = scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap_or_else(|| panic!("{name} starts a script"));
        let before = &scenario.steps[..start];
        let seed_at = before
            .iter()
            .position(|step| std::ptr::eq(step, seed_step(&scenario)))
            .unwrap();
        let deposit_at = before
            .iter()
            .position(|step| {
                matches!(
                    (&step.kind, &step.wait.arm),
                    (
                        StepKind::Repeat { .. },
                        Proof::BankItemId { id, count }
                    ) if *id == unnoted_id && *count == note_count
                )
            })
            .unwrap_or_else(|| panic!("{name} deposits unnoted {unnoted_id} x{note_count}"));
        let close_at = before
            .iter()
            .position(|step| matches!(step.wait.arm, Proof::BankClosed))
            .unwrap_or_else(|| panic!("{name} closes the seed bank"));
        assert!(seed_at < deposit_at, "{name} gives before deposit");
        assert!(deposit_at < close_at, "{name} deposits before close");
        let given = before[seed_at + 1..deposit_at]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(
            given.contains(&Proof::ItemId {
                id: note_id,
                count: note_count
            }),
            "{name} must prove the given note count"
        );
        assert!(
            given.contains(&Proof::ItemIdAtMost {
                id: note_id,
                count: note_count
            }),
            "{name} must bound the given note count"
        );
        let after = before[deposit_at + 1..close_at]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(
            after.contains(&Proof::BankItemId {
                id: unnoted_id,
                count: note_count
            }),
            "{name} must prove the unnoted bank count"
        );
        assert!(
            after.contains(&Proof::BankItemIdAtMost {
                id: unnoted_id,
                count: note_count
            }),
            "{name} must bound the unnoted bank count"
        );
        assert!(
            after.contains(&Proof::ItemIdAtMost {
                id: note_id,
                count: 0
            }),
            "{name} must prove note removal from pack"
        );
        assert!(
            after.contains(&Proof::BankItemIdAtMost {
                id: note_id,
                count: 0
            }),
            "{name} must prove no noted bank remainder"
        );
    }

    // Stackable non-note branch (coins): pack give → bank deposit → pack empty.
    let newt = get("herblore_secondaries_newt").expect("herblore_secondaries_newt");
    let start = newt
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("newt starts");
    let before = &newt.steps[..start];
    let seed_at = before
        .iter()
        .position(|step| std::ptr::eq(step, seed_step(&newt)))
        .unwrap();
    let deposit_at = before
        .iter()
        .position(|step| {
            matches!(
                (&step.kind, &step.wait.arm),
                (
                    StepKind::Repeat { .. },
                    Proof::BankItemId {
                        id: COINS_ID,
                        count: HERBLORE_NEWT_COIN_SEED
                    }
                )
            )
        })
        .expect("newt deposits coins");
    let close_at = before
        .iter()
        .position(|step| matches!(step.wait.arm, Proof::BankClosed))
        .expect("newt closes seed bank");
    assert!(seed_at < deposit_at);
    assert!(deposit_at < close_at);
    let given: Vec<_> = before[seed_at + 1..deposit_at]
        .iter()
        .map(|s| s.wait.arm)
        .collect();
    assert!(given.contains(&Proof::ItemId {
        id: COINS_ID,
        count: HERBLORE_NEWT_COIN_SEED
    }));
    assert!(given.contains(&Proof::ItemIdAtMost {
        id: COINS_ID,
        count: HERBLORE_NEWT_COIN_SEED
    }));
    let after: Vec<_> = before[deposit_at + 1..close_at]
        .iter()
        .map(|s| s.wait.arm)
        .collect();
    assert!(after.contains(&Proof::BankItemId {
        id: COINS_ID,
        count: HERBLORE_NEWT_COIN_SEED
    }));
    assert!(after.contains(&Proof::BankItemIdAtMost {
        id: COINS_ID,
        count: HERBLORE_NEWT_COIN_SEED
    }));
    assert!(after.contains(&Proof::ItemIdAtMost {
        id: COINS_ID,
        count: 0
    }));
}

#[test]
fn leather_crafter_thread_shop_seeds_zero_thread_and_orders_purchase_return_craft() {
    let s = get("leather_crafter_thread_shop").expect("thread shop registered");
    assert_eq!(s.name, "leather_crafter_thread_shop");
    assert_eq!(s.settings.start_script, Some("LeatherCrafter"));
    assert_eq!(
        s.settings.script_settings_inject,
        Some(LEATHER_CRAFTER_INJECT)
    );
    assert_eq!(s.settings.deadline, SCRIPT_GOLD_DEADLINE);
    assert_eq!(
        s.proof,
        Proof::ItemId {
            id: LEATHER_GLOVES_ID,
            count: 1
        }
    );

    let start = s
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("StartScript");
    let before: Vec<_> = s.steps[..start].iter().map(|st| st.wait.arm).collect();
    assert!(
        before.contains(&Proof::ItemIdAtMost {
            id: THREAD_ID,
            count: 0
        }),
        "pack zero-thread baseline before Start"
    );
    assert!(
        before.contains(&Proof::BankItemIdAtMost {
            id: THREAD_ID,
            count: 0
        }),
        "bank zero-thread baseline before Start"
    );
    assert!(
        before.contains(&Proof::BankItemId {
            id: COINS_ID,
            count: LEATHER_THREAD_SHOP_COIN_SEED
        }),
        "exact banked coin seed"
    );
    assert!(
        before.contains(&Proof::BankItemId {
            id: NEEDLE_ID,
            count: 1
        }),
        "banked needle"
    );
    assert!(
        before.contains(&Proof::BankItemId {
            id: SOFT_LEATHER_ID,
            count: 28
        }),
        "banked soft leather"
    );
    assert!(
        before.contains(&Proof::ItemIdAtMost {
            id: LEATHER_GLOVES_ID,
            count: 0
        }),
        "no seeded gloves product in pack"
    );
    assert!(
        before.contains(&Proof::BankItemIdAtMost {
            id: LEATHER_GLOVES_ID,
            count: 0
        }),
        "no seeded gloves product in bank"
    );
    assert_eq!(s.steps[start - 1].wait.arm, Proof::BankClosed);

    let after: Vec<_> = s.steps[start + 1..].iter().map(|st| st.wait.arm).collect();
    assert_eq!(
        after,
        vec![
            Proof::ItemId {
                id: THREAD_ID,
                count: 1
            },
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: LEATHER_THREAD_SHOP_FUND - 1
            },
            Proof::ArrivedNear {
                x: AL_KHARID_BANK.x,
                z: AL_KHARID_BANK.z,
                level: AL_KHARID_BANK.level,
                radius: 8,
            },
            Proof::BankClosed,
            Proof::FreshStatXpGain {
                id: CRAFTING_STAT,
                min: 1
            },
            Proof::ItemId {
                id: LEATHER_GLOVES_ID,
                count: 1
            },
        ],
        "post-Start order: thread, coin spend, bank return, BankClosed, fresh XP, gloves"
    );

    // Old four leather fixtures stay registered and still seed thread.
    for name in [
        "leather_crafter",
        "leather_crafter_hard_body",
        "leather_crafter_green_body",
        "leather_crafter_chaps",
    ] {
        assert!(get(name).is_some(), "{name} preserved");
        let mut client = native_seed_client();
        let (ok, written) = send_seed(name, &mut client);
        assert!(ok, "{name} seed still ok: {written}");
        assert!(
            written.contains("give thread 100"),
            "{name} still seeds thread: {written}"
        );
    }
}

#[test]
fn render_smoke_registered_as_a_one_shot_scene2_capture() {
    let s = get("render_smoke").expect("render_smoke is registered");
    assert_eq!(s.name, "render_smoke");
    assert_eq!(s.seed.profiles, [("test", "test")]);
    assert!(!s.seed.mainland, "no hop needed for a smoke capture");
    assert_eq!(s.steps.len(), 1, "one capture step");
    assert!(matches!(
        s.steps[0].kind,
        StepKind::Shot { label: "scene2" }
    ));
    assert_eq!(s.proof.name(), "stat(16)>=0");
}

#[test]
fn bank_fletcher_seed_close_uses_native_modal_path() {
    use client::client::{Client, ClientConfig};
    use client::dash3d::ClientPlayer;
    use client::io::{ClientStream, ServerProt};
    use std::net::TcpListener;

    let step = bank_fletcher_close_seed_bank();
    let StepKind::Perform { send } = &step.kind else {
        panic!("seed-bank close must be a Perform step");
    };

    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: false,
    });
    client.ingame = true;
    client.scene_state = 2;
    client.map_build_base_x = 3200;
    client.map_build_base_z = 3200;
    client.local_player = Some(ClientPlayer::at(20, 20));
    client.main_modal_id = 100;
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    client.stream = Some(ClientStream::connect("127.0.0.1", port).unwrap());
    let _peer = listener.accept().unwrap();
    client.bump_gens(ServerProt::IF_OPENMAIN);
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(&client);

    assert!(send(&mut client, &snapshot));
    assert_eq!(
        client.main_modal_id, -1,
        "native close clears local modal state"
    );
}

#[test]
fn nav_full_is_a_mainland_follow_to_a_cross_square_destination() {
    let s = get("nav_full").expect("nav_full is registered");
    assert_eq!(s.name, "nav_full");
    assert_eq!(s.seed.profiles, [("test", "test")]);
    assert!(s.seed.mainland, "the mainland hop lands the Lumbridge tele");
    assert_eq!(s.steps.len(), 1, "one follow step");
    let (dest, arm) = match &s.steps[0].kind {
        StepKind::Follow { dest } => (
            *dest,
            match &s.steps[0].wait.arm {
                Proof::Arrived { x, z, level } => (*x, *z, *level),
                other => panic!("follow arm must be arrived, got {other:?}"),
            },
        ),
        _ => panic!("nav_full step must be Follow"),
    };
    // The destination is a concrete pack tile ~44 tiles north of the
    // mainland landing, crossing the z=3264 mapsquare boundary into
    // (50,51) — a square the old 2-square pack never baked.
    assert_eq!(
        dest,
        WorldTile {
            x: 3220,
            z: 3264,
            level: 0
        }
    );
    assert_eq!(arm, (3220, 3264, 0));
    assert_eq!(s.proof.name(), "arrived(3220,3264,0)");
}

#[test]
fn lamp_redemption_orders_real_script_work_injection_native_episode_and_fresh_resume() {
    const LAMP_ID: i32 = 2528;
    const STRENGTH_STAT: i32 = 2;
    const PRAYER_STAT: i32 = 5;

    let scenario = get("lamp_redemption").expect("lamp redemption is registered");
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("BoneBurier Start");
    assert_eq!(scenario.settings.start_script, Some("BoneBurier"));
    assert_eq!(
        scenario.steps[start + 1].wait.arm,
        Proof::FreshStatXpGain {
            id: PRAYER_STAT,
            min: 1
        },
        "real BoneBurier work must precede lamp injection"
    );
    assert_eq!(
        scenario.steps[start + 2].wait.arm,
        Proof::NoActiveContinue,
        "pre-injection dialogue state must settle before the episode"
    );

    let injection = &scenario.steps[start + 3];
    assert_eq!(
        injection.wait.arm,
        Proof::ItemId {
            id: LAMP_ID,
            count: 1
        }
    );
    let StepKind::Perform { send } = &injection.kind else {
        panic!("lamp injection must be one server cheat send");
    };
    let mut client = native_seed_client();
    let snapshot = GameSnapshot::new();
    assert!(send(&mut client, &snapshot));
    assert!(
        emitted_has(&client, "give macro_genilamp 1"),
        "fixture must use the selected-289 obj token, not a display-name alias"
    );

    assert!(matches!(
        scenario.steps[start + 4].kind,
        StepKind::ObserveLampRedemption {
            lamp_id: LAMP_ID,
            reward_stat: STRENGTH_STAT,
        }
    ));
    assert_eq!(scenario.steps[start + 4].wait.arm, Proof::NoActiveContinue);
    assert_eq!(
        scenario.steps[start + 5].wait.arm,
        Proof::FreshStatXpGain {
            id: PRAYER_STAT,
            min: 1
        },
        "the terminal work must use a baseline captured after native release"
    );
    assert_eq!(
        scenario.proof,
        Proof::FreshStatXpGain {
            id: PRAYER_STAT,
            min: 1
        }
    );
    assert!(
        scenario.steps[start..]
            .iter()
            .all(|step| !matches!(step.kind, StepKind::DrainDialogs { .. } | StepKind::Relog)),
        "the fixture must not drain the reward or restart the script itself"
    );
}

#[test]
fn maze_owned_is_registered_and_uses_one_authentic_macro_trigger() {
    let scenario = get("maze_owned").expect("maze_owned is registered");
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("passive catalog Start");
    assert_eq!(scenario.settings.start_script, Some("TradeBot"));
    assert!(
        start > 0,
        "mainland hop must stand on an unblocked courtyard tile before Start"
    );
    let stand = &scenario.steps[start - 1];
    assert_eq!(
        stand.wait.arm,
        Proof::Arrived {
            x: 3221,
            z: 3218,
            level: 0,
        },
        "pre-trigger stand must be the verified unblocked Whoops-fallback origin, exact"
    );
    let StepKind::Perform { send } = &stand.kind else {
        panic!("the courtyard stand is a one-shot tele, never Repeat or Walk");
    };
    let mut stand_client = native_seed_client();
    assert!(send(&mut stand_client, &GameSnapshot::new()));
    assert!(
        emitted_has(&stand_client, "tele 0,50,50,21,18"),
        "seed must use packed 0_50_50_21_18, the content return-fallback origin"
    );
    for forbidden in ["mazeend", "xplamp", "give "] {
        assert!(
            !emitted_has(&stand_client, forbidden),
            "courtyard seed must not force Maze completion via {forbidden:?}"
        );
    }
    assert!(
        scenario
            .steps
            .iter()
            .all(|step| !matches!(step.wait.arm, Proof::NpcNameNear { .. })),
        "macro_maze starts automatically; transient NPC visibility must not gate progress"
    );
    assert!(
        scenario.steps[start + 1..].iter().all(|step| !matches!(
            step.kind,
            StepKind::Perform { .. } | StepKind::Repeat { .. }
        )),
        "the one-shot trigger belongs on the armed observer, never a later Repeat"
    );
    assert_eq!(
        scenario.steps[start + 1..].len(),
        1,
        "the armed Maze observer owns the only post-Start step"
    );
    let observer = &scenario.steps[start + 1];
    let StepKind::ObserveMazeCompletion {
        spawns,
        shrine,
        shrine_radius,
        min_progress,
        entry_shot,
        trigger,
    } = observer.kind
    else {
        panic!("Maze completion uses the bounded native observer");
    };
    assert_eq!(
        trigger,
        Some("~macro_event 8"),
        "the observer must send the one authentic trigger after capturing the baseline"
    );
    assert_eq!(spawns, MAZE_SPAWNS);
    assert_eq!(shrine, MAZE_SHRINE);
    assert_eq!(shrine_radius, 4);
    assert_eq!(min_progress, 8);
    assert_eq!(entry_shot, "maze_owned entered");
    assert_eq!(
        observer.wait.arm,
        Proof::IngameScene2,
        "the observer keeps the ready-scene arm; do not replace the NPC wait with ProofAlways"
    );
    for forbidden in ["mazeend", "xplamp", "give ", "tele "] {
        assert!(
            !trigger.unwrap().contains(forbidden.trim()),
            "Maze trigger must not seed or force completion via {forbidden:?}"
        );
    }
    assert_eq!(scenario.proof.name(), "ingame_scene_2");
    assert_eq!(scenario.settings.terminal_shot, Some("maze_owned final"));
}

#[test]
fn script_trade_is_a_two_profile_fleet_with_a_rust_acceptor_companion() {
    let s = get("script_trade").expect("script_trade is registered");
    assert_eq!(s.name, "script_trade");
    assert_eq!(s.seed.profiles, [("test", "test"), ("test2", "test2")]);
    assert!(
        s.seed.mainland,
        "the hop lands both bots before the trade tele"
    );
    assert_eq!(
        s.steps.len(),
        5,
        "seed coins, relog, tele, StartScript, watch coins"
    );
    assert!(matches!(s.steps[0].kind, StepKind::Perform { .. }));
    assert!(matches!(s.steps[1].kind, StepKind::Relog));
    assert_eq!(s.steps[1].wait.arm, Proof::SideTabAvailable { index: 3 });
    assert!(matches!(s.steps[2].kind, StepKind::Perform { .. }));
    assert!(matches!(s.steps[3].kind, StepKind::StartScript));
    assert_eq!(
        s.steps[4].wait.arm,
        Proof::ItemAtMost {
            name: "Coins",
            count: 0
        }
    );
    assert_eq!(
        s.proof.name(),
        "has_item(Coins)<=0",
        "terminal proof is zero coins on the driven slot"
    );
    assert_eq!(
        s.companions.len(),
        1,
        "profile 1 rust-teles and rust-accepts"
    );
    assert_eq!(s.companions[0].profile, 1);
    assert_eq!(s.settings.start_script, Some("TradeBot"));
    assert_eq!(s.settings.inject_companion_as, Some("partner"));
    assert!(s.settings.full_rate);
    assert!(s.settings.require_mainland_base);
    assert_eq!(s.settings.nav.engine_speed_ms, Some(600));
    assert!(names().contains(&"script_trade"));
}

#[test]
fn script_trade_starts_tradebot_after_relog_and_courtyard_tele() {
    let s = get("script_trade").unwrap();
    let i = s
        .steps
        .iter()
        .position(|st| matches!(st.kind, StepKind::StartScript))
        .expect("StartScript step");
    assert!(
        matches!(s.steps[i - 1].kind, StepKind::Perform { .. }),
        "StartScript follows the courtyard tele"
    );
    assert_eq!(
        s.steps[i - 1].wait.arm,
        Proof::Arrived {
            x: 3220,
            z: 3220,
            level: 0,
        }
    );
    assert!(
        matches!(s.steps[i - 2].kind, StepKind::Relog),
        "StartScript follows the relog"
    );
    assert_eq!(s.steps[i].wait.arm, Proof::Stat { id: 16, min: 0 });
    assert_eq!(s.steps[i].wait.budget_ticks, 1);
}

#[test]
fn script_trade_companion_blocks_when_trade_open_without_accept_id() {
    use api::snapshot::GameSnapshot;
    use client::client::{Client, ClientConfig};
    use client::dash3d::ClientPlayer;
    use client::io::ServerProt;

    let mut c = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: false,
    });
    c.ingame = true;
    c.scene_state = 2;
    c.local_player = Some(ClientPlayer::at(20, 20));
    // TRADEMAIN without Accept/Decline buttons in the modal tree.
    c.main_modal_id = 3323;
    c.bump_gens(ServerProt::IF_OPENMAIN_SIDE);

    let mut snap = GameSnapshot::default();
    snap.rebuild(&c);
    let trade = snap.trade();
    assert!(trade.offer_open, "offer screen must be open");
    assert!(
        trade.accept_component_id < 0,
        "accept id must be missing without iface buttons"
    );
    assert_eq!(
        trade_accept_missing_block(trade),
        Some("BLOCKED: missing trade accept com")
    );

    c.main_modal_id = 3443; // TRADECONFIRM
    c.bump_gens(ServerProt::IF_OPENMAIN_SIDE);
    snap.rebuild(&c);
    let trade = snap.trade();
    assert!(trade.confirm_open, "confirm screen must be open");
    assert!(trade.accept_component_id < 0);
    assert_eq!(
        trade_accept_missing_block(trade),
        Some("BLOCKED: missing trade accept com")
    );
}

#[test]
fn gold_script_scenarios_register_start_script_names() {
    let cases = [
        ("bone_burier", "BoneBurier"),
        ("chicken_killer", "ChickenKiller"),
        ("thiever", "Thiever"),
        ("alcher", "Alcher"),
        ("bank_fletcher", "BankFletcher"),
        ("script_trade", "TradeBot"),
    ];
    for (name, card) in cases {
        let s = get(name).unwrap_or_else(|| panic!("{name} is registered"));
        assert_eq!(
            s.settings.start_script,
            Some(card),
            "{name} must start the {card} catalog card"
        );
        assert!(
            s.settings.require_mainland_base,
            "{name} waits for mainland scene 2"
        );
        assert_eq!(
            s.settings.nav.engine_speed_ms,
            Some(600),
            "{name} cheats speed 600 so a leftover nav speed 300 is not inherited"
        );
    }
    for name in ["thiever", "bank_fletcher"] {
        let s = get(name).unwrap_or_else(|| panic!("{name} is registered"));
        assert!(
            s.steps
                .iter()
                .any(|st| matches!(st.kind, StepKind::DrainDialogs { .. })),
            "{name} drains advancestat level-up IFs with DrainDialogs"
        );
    }
    let alcher = get("alcher").expect("alcher");
    assert_eq!(
        alcher.settings.start_script,
        Some("Alcher"),
        "get(\"alcher\") has start_script: Some(\"Alcher\")"
    );
    assert!(
        alcher.settings.script_settings_inject.is_some(),
        "Alcher injects the items bag"
    );
    assert!(
        alcher
            .steps
            .iter()
            .any(|st| matches!(st.wait.arm, Proof::Stat { id: 6, min: 55 })),
        "alcher seed waits for Magic ≥ 55 before Start/watch, not arrival alone"
    );
    let thiever = get("thiever").expect("thiever");
    assert!(
        thiever
            .settings
            .script_settings_inject
            .is_some_and(|rows| rows.iter().any(|r| r.id == "loot")),
        "Thiever injects loot off"
    );
    assert!(names().contains(&"chicken_killer"));
    assert!(names().contains(&"bank_fletcher"));
}

#[test]
fn gold_scripts_start_the_catalog_after_the_last_seed_wait() {
    fn start_idx(name: &str) -> usize {
        let s = get(name).unwrap_or_else(|| panic!("{name} is registered"));
        s.steps
            .iter()
            .position(|st| matches!(st.kind, StepKind::StartScript))
            .unwrap_or_else(|| panic!("{name} has a StartScript step after the last seed wait"))
    }

    // bone_burier: after relog SideTabAvailable, before watch bury chat
    let bone = get("bone_burier").unwrap();
    let i = start_idx("bone_burier");
    assert!(
        matches!(bone.steps[i - 1].kind, StepKind::Relog),
        "bone_burier StartScript follows the relog"
    );
    assert_eq!(
        bone.steps[i - 1].wait.arm,
        Proof::SideTabAvailable { index: 3 }
    );
    assert_eq!(
        bone.steps[i + 1].wait.arm,
        Proof::StatXpGain { id: 5, min: 22 }
    );
    assert_eq!(bone.steps[i].wait.arm, Proof::Stat { id: 16, min: 0 });
    assert_eq!(bone.steps[i].wait.budget_ticks, 1);

    // chicken_killer: anchors at Start — host tele to pen after seed, before watch XP
    let chickens = get("chicken_killer").unwrap();
    let i = start_idx("chicken_killer");
    assert_eq!(
        chickens.steps[i - 1].wait.arm,
        Proof::ArrivedNear {
            x: 3235,
            z: 3295,
            level: 0,
            radius: 8,
        },
        "chicken_killer StartScript follows pen tele (3235,3295)"
    );
    assert_eq!(
        chickens.steps[i + 1].wait.arm,
        Proof::StatXpGain { id: 2, min: 1 }
    );
    assert!(
        chickens
            .settings
            .script_settings_inject
            .as_ref()
            .map(|rows| !rows.iter().any(|r| r.id == "banking"))
            .unwrap_or(true),
        "chicken_killer banking stays off (no inject)"
    );
    assert_eq!(chickens.settings.start_script, Some("ChickenKiller"));
    assert!(chickens.settings.require_mainland_base);

    // chicken_killer_bank: Falador pen after seed, loot-count inject, then
    // combat / exact feather / fresh deposit / return r6 / new feathers.
    let bank = get("chicken_killer_bank").unwrap();
    let i = start_idx("chicken_killer_bank");
    assert_eq!(
        bank.steps[i - 1].wait.arm,
        Proof::ItemIdAtMost {
            id: FEATHER_ID,
            count: 0
        },
        "chicken_killer_bank StartScript follows empty-feather ack"
    );
    assert_eq!(
        bank.steps[i - 4].wait.arm,
        Proof::ArrivedNear {
            x: 3029,
            z: 3294,
            level: 0,
            radius: 8,
        },
        "chicken_killer_bank teles to Falador south chickens"
    );
    assert_eq!(bank.settings.start_script, Some("ChickenKiller"));
    assert!(bank.settings.require_mainland_base);

    // thiever: after tele + DrainDialogs, before watch XP
    let thiever = get("thiever").unwrap();
    let i = start_idx("thiever");
    assert!(
        matches!(thiever.steps[i - 1].kind, StepKind::DrainDialogs { .. }),
        "thiever StartScript follows DrainDialogs"
    );
    assert_eq!(
        thiever.steps[i + 1].wait.arm,
        Proof::StatXpGain { id: 17, min: 1 }
    );

    // alcher: after Magic ≥55 and tele ArrivedNear West bank, before watch XP
    let alcher = get("alcher").unwrap();
    let i = start_idx("alcher");
    assert!(
        alcher.steps[..i]
            .iter()
            .any(|st| matches!(st.wait.arm, Proof::Stat { id: 6, min: 55 })),
        "alcher StartScript is after Magic ≥55"
    );
    assert_eq!(
        alcher.steps[i - 1].wait.arm,
        Proof::ArrivedNear {
            x: 3185,
            z: 3440,
            level: 0,
            radius: 6,
        }
    );
    assert_eq!(
        alcher.steps[i + 1].wait.arm,
        Proof::StatXpGain { id: 6, min: 1 }
    );

    // bank_fletcher: after last seed wait (tele + drain), before watch XP
    let fletcher = get("bank_fletcher").unwrap();
    let i = start_idx("bank_fletcher");
    assert!(
        matches!(fletcher.steps[i - 1].kind, StepKind::DrainDialogs { .. }),
        "bank_fletcher StartScript follows the last seed wait"
    );
    assert_eq!(
        fletcher.steps[i + 1].wait.arm,
        Proof::StatXpGain { id: 9, min: 1 }
    );
}

#[test]
fn bank_fletcher_string_modes_are_registered_with_id_strict_proofs() {
    let string = get("bank_fletcher_string").expect("string scenario registered");
    assert_eq!(string.settings.start_script, Some("BankFletcher"));
    assert_eq!(string.settings.deadline, SCRIPT_GOLD_DEADLINE);
    let inject = settings_inject_map(string.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("material"),
        Some(&Value::String("Willow logs".into()))
    );
    assert_eq!(
        inject.get("product"),
        Some(&Value::String("String short bow".into()))
    );
    assert!(
        !inject.contains_key("mode"),
        "old catalog has no mode setting"
    );
    let string_start = string
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert_eq!(
        string.steps[string_start + 1].wait.arm,
        Proof::StatXpGain { id: 9, min: 66 },
        "the XP baseline must be armed before the first seeded pair finishes"
    );
    let string_seed_arms = string.steps[..string_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(string_seed_arms.contains(&Proof::BankItemId { id: 60, count: 28 }));
    assert!(string_seed_arms.contains(&Proof::BankItemId {
        id: 1777,
        count: 28,
    }));
    assert!(string_seed_arms.contains(&Proof::BankItemIdAtMost { id: 849, count: 0 }));
    assert_eq!(string.steps[string_start - 1].wait.arm, Proof::BankClosed);
    let string_arms = string
        .steps
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(string_arms.contains(&Proof::ItemId { id: 849, count: 2 }));
    assert!(string_arms.contains(&Proof::BankItemId { id: 849, count: 2 }));
    assert!(string_arms.contains(&Proof::ItemId { id: 60, count: 14 }));
    assert_eq!(string.proof, Proof::StatXpGain { id: 9, min: 67 });

    let combined = get("bank_fletcher_cut_string").expect("combined scenario registered");
    assert_eq!(combined.settings.start_script, Some("BankFletcher"));
    assert_eq!(combined.settings.deadline, SCRIPT_GOLD_DEADLINE);
    let inject = settings_inject_map(combined.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("mode"),
        Some(&Value::String("cut+string".into()))
    );
    assert_eq!(
        inject.get("material"),
        Some(&Value::String("Willow logs".into()))
    );
    assert_eq!(
        inject.get("product"),
        Some(&Value::String("Short bow".into()))
    );
    let combined_start = combined
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert_eq!(
        combined.steps[combined_start + 1].wait.arm,
        Proof::StatXpGain { id: 9, min: 66 },
        "the XP baseline must be armed before the cut pair finishes"
    );
    let combined_seed_arms = combined.steps[..combined_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(combined_seed_arms.contains(&Proof::BankItemId {
        id: 1777,
        count: 28,
    }));
    assert!(combined_seed_arms.contains(&Proof::BankItemIdAtMost { id: 1519, count: 0 }));
    assert!(combined_seed_arms.contains(&Proof::BankItemIdAtMost { id: 60, count: 0 }));
    assert!(combined_seed_arms.contains(&Proof::BankItemIdAtMost { id: 849, count: 0 }));
    assert_eq!(
        combined.steps[combined_start - 1].wait.arm,
        Proof::BankClosed
    );
    let combined_arms = combined
        .steps
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(combined_arms.contains(&Proof::ItemId { id: 60, count: 2 }));
    assert!(combined_arms.contains(&Proof::BankItemId { id: 60, count: 2 }));
    assert!(combined_arms.contains(&Proof::ItemId { id: 849, count: 2 }));
    let close = combined_arms
        .iter()
        .rposition(|proof| *proof == Proof::BankClosed)
        .unwrap();
    assert_eq!(
        combined_arms[close + 1..close + 4],
        [
            Proof::StatXpGain { id: 9, min: 100 },
            Proof::ItemId { id: 849, count: 2 },
            Proof::ItemIdAtMost { id: 60, count: 0 },
        ],
        "stringing XP must be observed before the exact inputs are exhausted"
    );
    assert_eq!(combined.proof, Proof::ItemId { id: 849, count: 2 });
    assert!(names().contains(&"bank_fletcher_string"));
    assert!(names().contains(&"bank_fletcher_cut_string"));
}

#[test]
fn remaining_production_options_are_registered_with_ordered_exact_proofs() {
    let defaults = get("alcher_defaults").expect("default Alcher scenario registered");
    assert_eq!(defaults.settings.start_script, Some("Alcher"));
    assert_eq!(defaults.settings.deadline, SCRIPT_GOLD_DEADLINE);
    let inject = settings_inject_map(defaults.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("items"), Some(&Value::Array(vec![])));
    assert_eq!(inject.get("alchs"), Some(&Value::from(1.0)));
    let start = defaults
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let seed = defaults.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::BankItemId { id: 855, count: 1 }));
    assert!(seed.contains(&Proof::BankItemIdAtMost { id: 856, count: 0 }));
    assert!(seed.contains(&Proof::ItemAtMost {
        name: "Rune chainbody",
        count: 0,
    }));
    assert_eq!(defaults.steps[start - 1].wait.arm, Proof::BankClosed);
    assert_eq!(
        defaults.steps[start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>(),
        vec![
            Proof::ItemId { id: 856, count: 1 },
            Proof::StatXpGain { id: 6, min: 65 },
            Proof::ItemIdAtMost { id: 856, count: 0 },
            Proof::ItemIdAtMost { id: 561, count: 0 },
            Proof::ItemId {
                id: 995,
                count: 768
            },
            Proof::ItemAtMost {
                name: "Rune chainbody",
                count: 0,
            },
        ]
    );

    for (name, material, product, first_product, first_count) in [
        ("bank_fletcher_shafts", "Logs", "Arrow shafts", 52, 405),
        ("bank_fletcher_headless", "Logs", "Headless arrows", 53, 30),
    ] {
        let scenario = get(name).unwrap_or_else(|| panic!("{name} registered"));
        assert_eq!(scenario.settings.start_script, Some("BankFletcher"));
        assert_eq!(scenario.settings.deadline, SCRIPT_GOLD_DEADLINE);
        let inject = settings_inject_map(scenario.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("material"),
            Some(&Value::String(material.into()))
        );
        assert_eq!(inject.get("product"), Some(&Value::String(product.into())));
        assert!(!inject.contains_key("mode"));
        let start = scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        assert_eq!(scenario.steps[start - 1].wait.arm, Proof::BankClosed);
        let watch = scenario.steps[start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert_eq!(watch[0], Proof::StatXpGain { id: 9, min: 1 });
        assert!(watch.contains(&Proof::ItemId {
            id: first_product,
            count: first_count,
        }));
        let closed = watch
            .iter()
            .position(|arm| *arm == Proof::BankClosed)
            .unwrap();
        assert!(matches!(
            watch[closed + 1],
            Proof::FreshStatXpGain { id: 9, min: 1 }
        ));
        assert_eq!(
            scenario.proof,
            Proof::ItemId {
                id: first_product,
                count: 1
            }
        );
        assert!(names().contains(&name));
    }
}

#[test]
fn bank_cells_register_their_cards_injects_and_watch_chain() {
    for (name, card, deadline) in [
        ("auto_fighter_bank", "AutoFighter", SCRIPT_GOLD_DEADLINE),
        ("moss_giant_bank", "MossGiant", SCRIPT_GOLD_DEADLINE),
        ("moss_giant_bank_start", "MossGiant", SCRIPT_GOLD_DEADLINE),
        ("hill_giant_bank", "HillGiant", SCRIPT_GOLD_DEADLINE),
        (
            "rock_crab_bank_seeded",
            "RockCrab",
            Duration::from_secs(360),
        ),
        // The frozen trip reaches the field ~128s in (two banks).
        (
            "chaos_druid_bank",
            "ChaosDruidKiller",
            Duration::from_secs(300),
        ),
        ("ardy_fighter_bank", "ArdyFighter", SCRIPT_GOLD_DEADLINE),
    ] {
        let scenario = get(name).unwrap_or_else(|| panic!("{name} registered"));
        assert_eq!(scenario.settings.start_script, Some(card));
        assert_eq!(scenario.settings.deadline, deadline, "{name}");
        assert_eq!(scenario.settings.terminal_shot, Some(name));
        assert!(scenario.settings.full_rate && scenario.seed.mainland);
        let start = scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap_or_else(|| panic!("{name} starts the card"));
        assert!(
            scenario.steps[..start].iter().any(|step| step.wait.arm
                == Proof::EquipmentId {
                    id: COMBAT_SCIMITAR_ID
                }),
            "{name} wields its weapon before the hostile-field teleport"
        );
        let watch = scenario.steps[start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(
            watch.contains(&Proof::BankClosed),
            "{name} watches its own bank close"
        );
        assert!(
            watch.iter().any(|arm| matches!(
                arm,
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1
                }
            )),
            "{name} needs fresh work after the bank return"
        );
    }
    let auto = settings_inject_map(
        get("auto_fighter_bank")
            .unwrap()
            .settings
            .script_settings_inject,
    )
    .unwrap();
    assert_eq!(auto.get("banking"), Some(&Value::String("Auto".into())));
    assert_eq!(auto.get("bankAtLootSlots"), Some(&Value::from(1.0)));
    // Guaranteed kill loot is still absent before Start; burial is off
    // so the bank deposit must consume the script-looted Bones.
    assert_eq!(auto.get("loot"), Some(&serde_json::json!(["Bones"])));
    assert_eq!(auto.get("buryBones"), Some(&Value::Bool(false)));
    let auto_scenario = get("auto_fighter_bank").unwrap();
    let auto_start = auto_scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    for step in &auto_scenario.steps[..auto_start] {
        assert!(
            !matches!(step.wait.arm, Proof::ItemId { id, .. } if id == BONES_ID),
            "auto_fighter_bank must not prepare a deposit-class item before Start"
        );
    }
    {
        let id = BONES_ID;
        assert!(
            auto_scenario.steps[..auto_start].iter().any(|step| {
                matches!(step.wait.arm, Proof::ItemIdAtMost { id: got, count: 0 } if got == id)
            }),
            "auto_fighter_bank must confirm empty deposit-class id {id} before Start"
        );
    }
    for hill_name in ["hill_giant_bank", "hill_giant_loot_deposit"] {
        let hill =
            settings_inject_map(get(hill_name).unwrap().settings.script_settings_inject).unwrap();
        assert_eq!(
            hill.get("lootSlots"),
            Some(&Value::from(1.0)),
            "{hill_name}"
        );
        assert_eq!(
            hill.get("buryBones"),
            Some(&Value::Bool(false)),
            "{hill_name}"
        );
    }
    let chaos = settings_inject_map(
        get("chaos_druid_bank")
            .unwrap()
            .settings
            .script_settings_inject,
    )
    .unwrap();
    assert_eq!(
        chaos.get("location"),
        Some(&Value::String("Edgeville Dungeon".into()))
    );
    let ardy = settings_inject_map(
        get("ardy_fighter_bank")
            .unwrap()
            .settings
            .script_settings_inject,
    )
    .unwrap();
    assert_eq!(
        ardy.get("bankStrategy"),
        Some(&Value::String("Loot count".into()))
    );
    assert_eq!(ardy.get("bankEveryItems"), Some(&Value::from(1.0)));
    assert_eq!(ardy.get("target"), Some(&Value::String("Guard".into())));
    let ardy_scenario = get("ardy_fighter_bank").unwrap();
    let ardy_start = ardy_scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    for step in &ardy_scenario.steps[..ardy_start] {
        assert!(
            !matches!(step.wait.arm, Proof::ItemId { id, .. } if GUARD_DROP_IDS.contains(&id)),
            "ardy_fighter_bank must not prepare a deposit-class item before Start"
        );
    }
    for &id in &GUARD_DROP_IDS {
        assert!(
            ardy_scenario.steps[..ardy_start].iter().any(|step| {
                matches!(step.wait.arm, Proof::ItemIdAtMost { id: got, count: 0 } if got == id)
            }),
            "ardy_fighter_bank must confirm empty deposit-class id {id} before Start"
        );
    }
    let ardy_watch = ardy_scenario.steps[ardy_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(
        ardy_watch.contains(&Proof::BankItemIdAny {
            ids: &GUARD_DROP_IDS,
            count: 1,
        }),
        "ardy_fighter_bank watches a Guard drop enter a fresh bank"
    );
}

#[test]
fn hazard_camp_cells_register_their_cards_injects_and_watch_chain() {
    let rock = get("rock_crab_bank").expect("rock_crab_bank registered");
    assert_eq!(rock.settings.start_script, Some("RockCrab"));
    assert_eq!(rock.settings.deadline, SCRIPT_GOLD_DEADLINE);
    let inject = settings_inject_map(rock.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("bankStrategy"),
        Some(&Value::String("Loot count".into()))
    );
    assert_eq!(inject.get("bankEveryItems"), Some(&Value::from(1.0)));
    let start = rock
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert!(rock.steps[..start]
        .iter()
        .any(|step| step.name == "acknowledge dormant Rocks in the supported field before Start"));
    assert!(rock.steps[..start].iter().any(|step| step.wait.arm
        == Proof::EquipmentId {
            id: COMBAT_SCIMITAR_ID
        }));
    for &id in &ROCK_CRAB_BANK_DEPOSIT {
        assert!(
            rock.steps[..start].iter().any(|step| {
                matches!(step.wait.arm, Proof::ItemIdAtMost { id: got, count: 0 } if got == id)
            }),
            "rock_crab_bank must confirm empty deposit-class id {id} before Start"
        );
    }
    let watch = rock.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(watch.contains(&Proof::BankItemIdAny {
        ids: &ROCK_CRAB_BANK_DEPOSIT,
        count: 1,
    }));
    assert!(watch.contains(&Proof::BankClosed));
    let seeded = get("rock_crab_bank_seeded").expect("rock_crab_bank_seeded registered");
    assert_eq!(seeded.settings.start_script, Some("RockCrab"));
    assert_eq!(seeded.settings.deadline, Duration::from_secs(360));
    let seeded_start = seeded
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert!(seeded.steps[..seeded_start].iter().any(|step| {
        step.wait.arm
            == Proof::ItemId {
                id: UNCUT_SAPPHIRE_ID,
                count: 1,
            }
    }));
    let seeded_watch = seeded.steps[seeded_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seeded_watch.contains(&Proof::ArrivedNear {
        x: 2725,
        z: 3491,
        level: 0,
        radius: 6,
    }));
    assert!(seeded_watch.contains(&Proof::ItemIdAtMost {
        id: UNCUT_SAPPHIRE_ID,
        count: 0,
    }));
    assert!(seeded_watch.contains(&Proof::BankItemId {
        id: UNCUT_SAPPHIRE_ID,
        count: 1,
    }));
    assert!(seeded_watch.contains(&Proof::BankClosed));
    assert!(seeded_watch.contains(&Proof::ArrivedNear {
        x: 2710,
        z: 3717,
        level: 0,
        radius: 6,
    }));

    let dragon = get("green_dragon_bank").expect("green_dragon_bank registered");
    assert_eq!(dragon.settings.start_script, Some("GreenDragon"));
    let inject = settings_inject_map(dragon.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("escape"),
        Some(&Value::String("Flee to bank".into()))
    );
    let start = dragon
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert!(dragon.steps[..start].iter().any(|step| step.wait.arm
        == Proof::EquipmentId {
            id: RUNE_SCIMITAR_ID
        }));
    assert!(dragon.steps[..start].iter().any(|step| step.wait.arm
        == Proof::EquipmentId {
            id: DRAGONFIRE_SHIELD_ID
        }));
    let watch = dragon.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(watch.contains(&Proof::BankItemIdAny {
        ids: &GREEN_DRAGON_BANK_DEPOSIT,
        count: 1,
    }));

    let tele = get("green_dragon_tele").expect("green_dragon_tele registered");
    let inject = settings_inject_map(tele.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("escape"),
        Some(&Value::String("Teleport to Varrock".into()))
    );
    let start = tele
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert!(tele.steps[..start].iter().any(|step| step.wait.arm
        == Proof::Stat {
            id: MAGIC_STAT,
            min: VARROCK_TELE_MAGIC,
        }));
    let watch = tele.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(watch.contains(&Proof::StatXpGain {
        id: MAGIC_STAT,
        min: 1,
    }));
    assert!(watch.contains(&Proof::ArrivedNear {
        x: VARROCK_TELE_LAND.x,
        z: VARROCK_TELE_LAND.z,
        level: VARROCK_TELE_LAND.level,
        radius: 8,
    }));

    let approach = get("fire_giant_approach").expect("fire_giant_approach registered");
    assert_eq!(approach.settings.start_script, Some("FireGiant"));
    assert!(approach.steps.iter().any(|step| matches!(
        step.wait.arm,
        Proof::QuestDone {
            name: "Waterfall Quest"
        }
    )));
    let start = approach
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let seed = approach.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(approach.steps[..start].iter().any(|step| step.wait.arm
        == Proof::ArrivedNear {
            x: FIRE_GIANT_RAFT.x,
            z: FIRE_GIANT_RAFT.z,
            level: FIRE_GIANT_RAFT.level,
            radius: 5,
        }));
    assert!(seed.contains(&Proof::EquipmentId {
        id: COMBAT_SCIMITAR_ID,
    }));
    assert!(approach.steps[..start].iter().any(|step| step.name
            == "wield and acknowledge the prepared Adamant scimitar before the hostile-field teleport"));
    assert!(seed.contains(&Proof::NoActiveContinue));
    let wear_i = approach.steps[..start]
            .iter()
            .position(|step| {
                step.name
                    == "wield and acknowledge the prepared Adamant scimitar before the hostile-field teleport"
            })
            .expect("fire_giant_approach wear step");
    let drain_i = approach.steps[..start]
        .iter()
        .position(|step| {
            step.name == "drain setstat level-up dialogs before the hostile-field teleport"
        })
        .expect("fire_giant_approach setstat drain");
    let tele_i = approach.steps[..start]
        .iter()
        .position(|step| {
            step.name == "teleport into the hostile field only after preparation is acknowledged"
        })
        .expect("fire_giant_approach hostile tele");
    assert!(
            wear_i < drain_i && drain_i < tele_i && tele_i < start,
            "wear then setstat drain then tele then Start: wear={wear_i} drain={drain_i} tele={tele_i} start={start}"
        );
    assert!(matches!(
        approach.steps[drain_i].kind,
        StepKind::DrainDialogs { choice: 1 }
    ));
    assert!(approach.steps[start + 1..].iter().any(|step| step.wait.arm
        == Proof::ArrivedNear {
            x: FIRE_GIANT_ROOM.x,
            z: FIRE_GIANT_ROOM.z,
            level: FIRE_GIANT_ROOM.level,
            radius: 16,
        }));

    let bank = get("fire_giant_bank").expect("fire_giant_bank registered");
    let start = bank
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let watch = bank.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(watch.contains(&Proof::ArrivedNear {
        x: FIRE_GIANT_WASH.x,
        z: FIRE_GIANT_WASH.z,
        level: FIRE_GIANT_WASH.level,
        radius: 6,
    }));
    assert!(watch.contains(&Proof::BankItemId {
        id: BIG_BONES_ID,
        count: 1,
    }));
    assert!(watch.contains(&Proof::BankClosed));
}

#[test]
fn prepared_remaining_combat_cells_use_source_derived_profiles_and_long_budgets() {
    const DEFENCE: i32 = 1;
    let start_idx = |scenario: &Scenario| {
        scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .expect("prepared case starts the card")
    };
    let names_cards_and_levels = [
        ("green_dragon_special_prepared", "GreenDragon", 70),
        ("green_dragon_potions_prepared", "GreenDragon", 70),
        ("green_dragon_bank_prepared", "GreenDragon", 99),
        ("green_dragon_bank_default_prepared", "GreenDragon", 99),
        ("green_dragon_tele_prepared", "GreenDragon", 99),
        ("fire_giant_bank_prepared", "FireGiant", 99),
        ("fire_giant_camelot_prepared", "FireGiant", 99),
        ("moss_giant_prepared", "MossGiant", 70),
        ("hill_giant_bank_prepared", "HillGiant", 70),
    ];
    for (name, card, level) in names_cards_and_levels {
        let scenario = get(name).unwrap_or_else(|| panic!("{name} registered"));
        assert_eq!(scenario.settings.start_script, Some(card), "{name}");
        let (deadline_secs, min_watch_ticks) = match name {
            "green_dragon_bank_prepared" => (600, 1500),
            "green_dragon_bank_default_prepared" => (720, 1800),
            "green_dragon_tele_prepared" => (480, 1200),
            "fire_giant_bank_prepared" | "fire_giant_camelot_prepared" => (600, 1500),
            _ => (300, 750),
        };
        assert_eq!(
            scenario.settings.deadline,
            Duration::from_secs(deadline_secs),
            "{name}: named combat/travel qualification wall budget"
        );
        assert_eq!(scenario.settings.terminal_shot, Some(name), "{name}");
        let start = scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap_or_else(|| panic!("{name} starts the card"));
        assert!(
            scenario.steps[..start].iter().any(|step| {
                step.wait.arm
                    == Proof::Stat {
                        id: DEFENCE,
                        min: level,
                    }
            }),
            "{name}: exact prepared profile includes Defence {level}"
        );
        for id in [RUNE_CHAINBODY_ID, 1079, 1163] {
            assert!(
                scenario.steps[..start]
                    .iter()
                    .any(|step| step.wait.arm == Proof::EquipmentId { id }),
                "{name}: prepared armour {id} is worn before Start"
            );
        }
        assert!(
            scenario.steps[start + 1..]
                .iter()
                .all(|step| step.wait.budget_ticks >= min_watch_ticks),
            "{name}: post-Start dirty budgets must not pre-empt the named wall"
        );
    }

    let green_bank = get("green_dragon_bank_prepared").unwrap();
    let green_bank_inject =
        settings_inject_map(green_bank.settings.script_settings_inject).unwrap();
    assert_eq!(
        green_bank_inject.get("foodReserve"),
        Some(&Value::from(26.0))
    );
    assert_eq!(
        green_bank_inject.get("foodWithdraw"),
        Some(&Value::from(27.0))
    );
    assert_eq!(
        green_bank_inject.get("loot"),
        Some(&Value::Array(vec![
            Value::String("Dragon bones".into()),
            Value::String("Dragonhide".into()),
        ]))
    );
    let green_bank_start = start_idx(&green_bank);
    assert!(green_bank.steps[..green_bank_start].iter().any(|step| {
        step.wait.arm
            == Proof::ItemId {
                id: LOBSTER_ID,
                count: 26,
            }
    }));
    let green_food = green_bank.steps[..green_bank_start]
        .iter()
        .position(|step| {
            step.wait.arm
                == Proof::ItemId {
                    id: LOBSTER_ID,
                    count: 26,
                }
        })
        .unwrap();
    for id in [1113, 1079, 1163] {
        let worn = green_bank.steps[..green_bank_start]
            .iter()
            .position(|step| step.wait.arm == Proof::EquipmentId { id })
            .unwrap();
        assert!(
            worn < green_food,
            "green full-pack food is stocked only after armour frees pack slots"
        );
    }
    for id in GREEN_DRAGON_BANK_DEPOSIT {
        assert!(green_bank.steps[..green_bank_start]
            .iter()
            .any(|step| { step.wait.arm == Proof::ItemIdAtMost { id, count: 0 } }));
    }

    let green_default = get("green_dragon_bank_default_prepared").unwrap();
    let green_default_inject =
        settings_inject_map(green_default.settings.script_settings_inject).unwrap();
    assert_eq!(
        green_default_inject.get("foodReserve"),
        Some(&Value::from(26.0))
    );
    assert_eq!(
        green_default_inject.get("foodWithdraw"),
        Some(&Value::from(27.0))
    );
    assert!(
        !green_default_inject.contains_key("loot"),
        "default-loot Green must omit loot injection so the generated catalog is exercised"
    );
    let green_default_start = start_idx(&green_default);
    let green_default_watch: Vec<_> = green_default.steps[green_default_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect();
    assert!(green_default_watch.contains(&Proof::ItemId {
        id: DRAGON_BONES_ID,
        count: 1,
    }));
    assert!(green_default_watch.contains(&Proof::ItemId {
        id: GREEN_DRAGONHIDE_ID,
        count: 1,
    }));
    assert!(green_default_watch.contains(&Proof::BankItemId {
        id: GREEN_DRAGONHIDE_ID,
        count: 1,
    }));
    assert!(
        !green_default_watch.contains(&Proof::BankItemIdAny {
            ids: &GREEN_DRAGON_BANK_DEPOSIT,
            count: 1,
        }),
        "hide-only deposit must not equate bones-or-hide"
    );
    assert!(green_default_watch.contains(&Proof::ItemId {
        id: LOBSTER_ID,
        count: 27,
    }));
    for id in [DRAGON_BONES_ID, 537, GREEN_DRAGONHIDE_ID, 1754] {
        assert!(green_default.steps[..green_default_start]
            .iter()
            .any(|step| { step.wait.arm == Proof::ItemIdAtMost { id, count: 0 } }));
    }

    let moss_prepared = get("moss_giant_prepared").unwrap();
    let moss_inject = settings_inject_map(moss_prepared.settings.script_settings_inject).unwrap();
    assert_eq!(
        moss_inject.get("combatStyle"),
        Some(&Value::String("melee".into()))
    );
    assert_eq!(
        moss_inject.get("meleeStyle"),
        Some(&Value::String("strength".into()))
    );
    assert_eq!(moss_inject.get("buryBones"), Some(&Value::Bool(false)));
    let moss_start = start_idx(&moss_prepared);
    assert!(moss_prepared.steps[..moss_start].iter().any(|step| {
        step.wait.arm
            == Proof::ItemId {
                id: LOBSTER_ID,
                count: MOSS_GIANT_FOOD,
            }
    }));
    assert!(moss_prepared.steps[..moss_start]
        .iter()
        .any(|step| step.wait.arm
            == Proof::EquipmentId {
                id: RUNE_SCIMITAR_ID
            }));
    for id in [BIG_BONES_ID, 533] {
        assert!(moss_prepared.steps[..moss_start]
            .iter()
            .any(|step| step.wait.arm == Proof::ItemIdAtMost { id, count: 0 }));
    }
    assert!(
        !moss_prepared.steps[moss_start + 1..]
            .iter()
            .any(|step| matches!(
                step.wait.arm,
                Proof::BankClosed | Proof::BankItemId { .. } | Proof::BankItemIdAny { .. }
            )),
        "moss_giant_prepared is the fight-first core, not a bank cell"
    );

    let hill_prepared = get("hill_giant_bank_prepared").unwrap();
    let hill_inject = settings_inject_map(hill_prepared.settings.script_settings_inject).unwrap();
    assert_eq!(
        hill_inject.get("meleeStyle"),
        Some(&Value::String("strength".into()))
    );
    assert_eq!(hill_inject.get("buryBones"), Some(&Value::Bool(false)));
    assert_eq!(hill_inject.get("lootSlots"), Some(&Value::from(1.0)));
    let hill_start = start_idx(&hill_prepared);
    assert!(hill_prepared.steps[..hill_start].iter().any(|step| {
        step.wait.arm
            == Proof::ItemId {
                id: TROUT_ID,
                count: HILL_GIANT_FOOD,
            }
    }));
    assert!(hill_prepared.steps[..hill_start]
        .iter()
        .any(|step| step.wait.arm
            == Proof::EquipmentId {
                id: COMBAT_SCIMITAR_ID
            }));
    assert!(hill_prepared.steps[..hill_start]
        .iter()
        .any(|step| step.wait.arm
            == Proof::ItemId {
                id: BRASS_KEY_ID,
                count: 1
            }));
    for id in [BIG_BONES_ID, 533, LIMPWURT_ROOT_ID, 226] {
        assert!(hill_prepared.steps[..hill_start]
            .iter()
            .any(|step| step.wait.arm == Proof::ItemIdAtMost { id, count: 0 }));
    }
    let hill_watch: Vec<_> = hill_prepared.steps[hill_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect();
    assert!(hill_watch.contains(&Proof::BankItemIdAny {
        ids: &HILL_GIANT_BANK_DEPOSIT,
        count: 1,
    }));
    assert!(hill_watch.contains(&Proof::ItemId {
        id: TROUT_ID,
        count: 12,
    }));
    assert!(
        !hill_watch.contains(&Proof::BankItemId {
            id: BIG_BONES_ID,
            count: 1,
        }),
        "prepared Hill must accept either earned deposit class, not bones-only"
    );

    let green_tele = get("green_dragon_tele_prepared").unwrap();
    let green_tele_start = start_idx(&green_tele);
    let green_tele_inject =
        settings_inject_map(green_tele.settings.script_settings_inject).unwrap();
    assert_eq!(
        green_tele_inject.get("panicHp"),
        Some(&Value::from(98.0)),
        "the prepared teleport uses the source's configured panic threshold"
    );
    assert!(green_tele.steps[..green_tele_start]
        .iter()
        .any(|step| step.wait.arm == Proof::Stat { id: 3, min: 70 }));
    assert!(green_tele.steps[..green_tele_start]
        .iter()
        .any(|step| step.wait.arm == Proof::StatAtMost { id: 3, max: 70 }));
    assert!(green_tele.steps[..green_tele_start].iter().any(|step| {
        step.wait.arm
            == Proof::ItemIdAtMost {
                id: LOBSTER_ID,
                count: 0,
            }
    }));
    let safe_trigger = green_tele.steps[..green_tele_start]
        .iter()
        .find(|step| {
            step.name
                == "prepare and acknowledge 70 of 99 Hitpoints for the configured panic trigger"
        })
        .expect("prepared teleport has a high-HP panic trigger");
    let StepKind::Perform { send } = &safe_trigger.kind else {
        panic!("the high-HP panic trigger is one native fixture operation");
    };
    let mut client = native_seed_client();
    assert!(send(&mut client, &GameSnapshot::new()));
    assert!(emitted_has(&client, "~hit 29"));
    assert!(
        !emitted_has(&client, "~1hp"),
        "the prepared teleport must never expose the player at one hitpoint"
    );

    let fire_bank = get("fire_giant_bank_prepared").unwrap();
    let fire_bank_inject = settings_inject_map(fire_bank.settings.script_settings_inject).unwrap();
    assert_eq!(FIRE_GIANT_BANK_PREPARED_INITIAL_FOOD, 1);
    assert_eq!(FIRE_GIANT_BANK_PREPARED_RESTOCK, 25);
    assert_eq!(
        fire_bank_inject.get("foodWithdraw"),
        Some(&Value::from(FIRE_GIANT_BANK_PREPARED_RESTOCK as f64))
    );
    assert_eq!(
        fire_bank_inject.get("loot"),
        Some(&Value::Array(vec![Value::String("Big bones".into())]))
    );
    let fire_start = start_idx(&fire_bank);
    assert!(fire_bank.steps[..fire_start].iter().any(|step| {
        step.wait.arm
            == Proof::ItemId {
                id: LOBSTER_ID,
                count: FIRE_GIANT_BANK_PREPARED_INITIAL_FOOD,
            }
    }));
    let fire_food = fire_bank.steps[..fire_start]
        .iter()
        .position(|step| {
            step.wait.arm
                == Proof::ItemId {
                    id: LOBSTER_ID,
                    count: FIRE_GIANT_BANK_PREPARED_INITIAL_FOOD,
                }
        })
        .unwrap();
    for id in [1113, 1079, 1163] {
        let worn = fire_bank.steps[..fire_start]
            .iter()
            .position(|step| step.wait.arm == Proof::EquipmentId { id })
            .unwrap();
        assert!(
            worn < fire_food,
            "fire pressure food is stocked only after armour frees pack slots"
        );
    }
    for id in [GLARIALS_AMULET_ID, ROPE_ID] {
        assert!(
            fire_bank.steps[..fire_start]
                .iter()
                .any(|step| step.wait.arm == Proof::ItemId { id, count: 1 }),
            "prepared FireGiant keeps required traversal item {id}"
        );
    }
    assert!(fire_bank.steps[..fire_start].iter().any(|step| {
        step.wait.arm
            == Proof::ItemIdAtMost {
                id: BIG_BONES_ID,
                count: 0,
            }
    }));
    let fire_watch = fire_bank.steps[fire_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    let earned_deposit = fire_watch
        .iter()
        .position(|arm| {
            *arm == Proof::BankItemId {
                id: BIG_BONES_ID,
                count: 1,
            }
        })
        .expect("earned Big bones reach a fresh bank");
    let restock = fire_watch
        .iter()
        .position(|arm| {
            *arm == Proof::ItemId {
                id: LOBSTER_ID,
                count: FIRE_GIANT_BANK_PREPARED_RESTOCK,
            }
        })
        .expect("prepared FireGiant restocks exactly 25 Lobsters");
    let closed = fire_watch
        .iter()
        .position(|arm| *arm == Proof::BankClosed)
        .expect("prepared FireGiant closes the bank");
    let returned = fire_watch
        .iter()
        .position(|arm| {
            *arm == Proof::ArrivedNear {
                x: FIRE_GIANT_ROOM.x,
                z: FIRE_GIANT_ROOM.z,
                level: FIRE_GIANT_ROOM.level,
                radius: 10,
            }
        })
        .expect("prepared FireGiant returns to the room");
    let fresh_xp = fire_watch
        .iter()
        .position(|arm| {
            *arm == Proof::FreshStatXpGain {
                id: STRENGTH_STAT,
                min: 1,
            }
        })
        .expect("prepared FireGiant earns fresh Strength XP after return");
    assert_eq!(
        FIRE_GIANT_BANK_PREPARED_RESTOCK + 1 + 1,
        27,
        "restock leaves one cargo slot free"
    );
    assert!(
        earned_deposit < restock && restock < closed && closed < returned && returned < fresh_xp,
        "earned deposit, exact restock, close, return and fresh XP stay ordered"
    );

    let camelot = get("fire_giant_camelot_prepared").unwrap();
    let camelot_inject = settings_inject_map(camelot.settings.script_settings_inject).unwrap();
    assert_eq!(FIRE_GIANT_CAMELOT_PREPARED_RESTOCK, 24);
    assert_eq!(
        camelot_inject.get("foodWithdraw"),
        Some(&Value::from(FIRE_GIANT_CAMELOT_PREPARED_RESTOCK as f64))
    );
    assert_ne!(
        camelot_inject.get("foodWithdraw"),
        Some(&Value::from(FIRE_GIANT_BANK_PREPARED_RESTOCK as f64)),
        "Camelot inject must not reuse the barrel restock line"
    );
    let camelot_start = start_idx(&camelot);
    let camelot_watch = camelot.steps[camelot_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    let camelot_restock = camelot_watch
        .iter()
        .position(|arm| {
            *arm == Proof::ItemId {
                id: LOBSTER_ID,
                count: FIRE_GIANT_CAMELOT_PREPARED_RESTOCK,
            }
        })
        .expect("prepared Camelot restocks exactly 24 Lobsters");
    assert!(
        camelot.steps[..camelot_start].iter().any(|step| {
            step.wait.arm
                == Proof::ItemId {
                    id: AIR_RUNE_ID,
                    count: 15,
                }
        }),
        "Camelot pre-Start acknowledges Air carry before Start"
    );
    assert!(
        camelot.steps[..camelot_start].iter().any(|step| {
            step.wait.arm
                == Proof::ItemId {
                    id: LAW_RUNE_ID,
                    count: 3,
                }
        }),
        "Camelot pre-Start acknowledges Law carry before Start"
    );
    assert_eq!(
        FIRE_GIANT_CAMELOT_PREPARED_RESTOCK + 1 + 1 + 1,
        27,
        "Camelot worn amulet: food slots plus rope plus Air plus Law stacks leave one cargo slot free"
    );
    assert_eq!(
        FIRE_GIANT_BANK_PREPARED_RESTOCK + 1 + 1,
        27,
        "barrel prepared invariant unchanged: 25 food plus amulet plus rope"
    );
    let camelot_closed = camelot_watch
        .iter()
        .position(|arm| *arm == Proof::BankClosed)
        .expect("prepared Camelot closes the bank");
    let camelot_returned = camelot_watch
        .iter()
        .position(|arm| {
            *arm == Proof::ArrivedNear {
                x: FIRE_GIANT_ROOM.x,
                z: FIRE_GIANT_ROOM.z,
                level: FIRE_GIANT_ROOM.level,
                radius: 10,
            }
        })
        .expect("prepared Camelot returns to the room");
    assert!(
        camelot_restock < camelot_closed && camelot_closed < camelot_returned,
        "Camelot restock, close, and return stay ordered"
    );
}

/// The bank cells' pre-Start proofs only acknowledge the scoped weapon;
/// the seed closure is what has to hand it over. A fixture that acked a
/// weapon it never seeded failed the cell at the ack (rock_crab_bank,
/// step 12). The loot-count RockCrab cell also draws nothing from the
/// bank: its PeriodicBank trip deposits and returns, so no bank window is
/// seeded there while the restocking cells keep theirs.

#[test]
fn bank_cells_seed_the_weapon_they_acknowledge_and_only_real_bank_windows() {
    use client::client::{Client, ClientConfig};
    use client::dash3d::ClientPlayer;
    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: false,
    });
    client.ingame = true;
    client.scene_state = 2;
    client.map_build_base_x = 3200;
    client.map_build_base_z = 3200;
    client.local_player = Some(ClientPlayer::at(20, 20));
    let snapshot = GameSnapshot::new();

    let mut seed = |name: &str| {
        let scenario = get(name).unwrap_or_else(|| panic!("{name} is registered"));
        let prepare = scenario
            .steps
            .iter()
            .find(|step| step.name.starts_with("prepare melee stats"))
            .unwrap_or_else(|| panic!("{name} prepares on the safe tile before Start"));
        let StepKind::Perform { send } = &prepare.kind else {
            panic!("{name} preparation must be a Perform step");
        };
        let before = client.out.pos;
        assert!(send(&mut client, &snapshot));
        String::from_utf8_lossy(&client.out.data()[before..client.out.pos]).into_owned()
    };

    for (name, alias) in [
        ("auto_fighter_bank", "adamant_scimitar"),
        ("moss_giant_bank", "adamant_scimitar"),
        ("moss_giant_bank_start", "adamant_scimitar"),
        ("hill_giant_bank", "adamant_scimitar"),
        ("hill_giant_bank_prepared", "adamant_scimitar"),
        ("hill_giant_loot_deposit", "adamant_scimitar"),
        ("chaos_druid_bank", "adamant_scimitar"),
        ("ardy_fighter_bank", "adamant_scimitar"),
        ("rock_crab_bank", "adamant_scimitar"),
        ("rock_crab_bank_seeded", "adamant_scimitar"),
        ("green_dragon_bank", "rune_scimitar"),
        ("green_dragon_bank_default_prepared", "rune_scimitar"),
        ("green_dragon_tele", "rune_scimitar"),
        ("fire_giant_bank", "adamant_scimitar"),
    ] {
        let written = seed(name);
        assert!(
            written.contains(&format!("give {alias} 1")),
            "{name} must seed the weapon its own ack and wear steps expect: {written}"
        );
        assert!(
            written.contains("~clearinv"),
            "{name} wipes the pack before its own seed: {written}"
        );
    }

    // The loot-count RockCrab cell seeds no bank stock; the restocking
    // cells keep the window their BankRun withdraws from.
    let rock = seed("rock_crab_bank");
    assert!(
        !rock.contains("givebank"),
        "rock_crab_bank's PeriodicBank trip restocks nothing: {rock}"
    );
    let seeded = seed("rock_crab_bank_seeded");
    assert!(
        seeded.contains("give uncut_sapphire 1"),
        "rock_crab_bank_seeded seeds the PeriodicBank trigger cargo: {seeded}"
    );
    assert!(
        !seeded.contains("givebank"),
        "rock_crab_bank_seeded's PeriodicBank trip restocks nothing: {seeded}"
    );
    let dragon = seed("green_dragon_bank");
    assert!(
        dragon.contains("givebank lobster 24"),
        "green_dragon_bank keeps its withdraw window: {dragon}"
    );
    let green_default = seed("green_dragon_bank_default_prepared");
    assert!(
        green_default.contains("givebank lobster 40"),
        "default-loot Green keeps the prepared withdraw window: {green_default}"
    );
    assert!(
        !green_default.contains("give dragon")
            && !green_default.contains("give green_dragonhide")
            && !green_default.contains("givebank dragon")
            && !green_default.contains("givebank green_dragonhide"),
        "default-loot Green must not seed bones or hide: {green_default}"
    );
    let hill_prepared = seed("hill_giant_bank_prepared");
    assert!(
        hill_prepared.contains("givebank trout 12"),
        "prepared Hill keeps the card's foodWithdraw window: {hill_prepared}"
    );
    assert!(
        !hill_prepared.contains("give big_bones")
            && !hill_prepared.contains("give limpwurt")
            && !hill_prepared.contains("givebank big_bones")
            && !hill_prepared.contains("givebank limpwurt"),
        "prepared Hill must not seed earned-kill cargo: {hill_prepared}"
    );
    let moss_start = seed("moss_giant_bank_start");
    assert!(
        moss_start.contains("give big_bones 1"),
        "moss_giant_bank_start seeds declared deposit cargo: {moss_start}"
    );
    assert!(
        moss_start.contains("givebank lobster 24"),
        "moss_giant_bank_start keeps the MossGiant withdraw window: {moss_start}"
    );
    assert!(
        !moss_start.contains("give lobster"),
        "moss_giant_bank_start must not seed trip food (BankRun before Fight): {moss_start}"
    );
}

#[test]
fn moss_giant_bank_start_orders_startup_banking_before_fresh_combat_xp() {
    let scenario = get("moss_giant_bank_start").expect("moss_giant_bank_start registered");
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let pre_start: Vec<_> = scenario.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect();
    assert!(
        pre_start.contains(&Proof::ItemId {
            id: BIG_BONES_ID,
            count: 1,
        }),
        "startup cell acknowledges seeded Big bones before Start"
    );
    assert!(
        !pre_start.contains(&Proof::ItemIdAtMost {
            id: BIG_BONES_ID,
            count: 0,
        }),
        "startup cell must not forbid seeded Big bones before Start"
    );
    let post_start: Vec<_> = scenario.steps[start + 1..]
        .iter()
        .map(|step| (step.name, step.wait.arm))
        .collect();
    assert!(
        !post_start.iter().any(|(name, arm)| {
            *name == "watch Strength XP from the selected melee style after Start"
                && *arm
                    == Proof::StatXpGain {
                        id: STRENGTH_STAT,
                        min: 1,
                    }
        }),
        "bank-first cell must not wait pre-bank Strength XP"
    );
    let watch_arms: Vec<_> = post_start.iter().map(|(_, arm)| *arm).collect();
    let bones_bank = watch_arms
        .iter()
        .position(|arm| {
            *arm == Proof::BankItemId {
                id: BIG_BONES_ID,
                count: 1,
            }
        })
        .expect("startup bank watch for seeded Big bones");
    let lobster_restock = watch_arms
        .iter()
        .position(|arm| {
            *arm == Proof::ItemId {
                id: LOBSTER_ID,
                count: 20,
            }
        })
        .expect("startup restock watch");
    let closed = watch_arms
        .iter()
        .position(|arm| *arm == Proof::BankClosed)
        .expect("bank close watch");
    let fresh_xp = watch_arms
        .iter()
        .position(|arm| {
            matches!(
                arm,
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1
                }
            )
        })
        .expect("fresh Strength XP after return");
    assert!(
        bones_bank < lobster_restock && lobster_restock < closed && closed < fresh_xp,
        "startup banking watches must stay ordered before fresh combat XP"
    );
}

#[test]
fn hill_giant_loot_deposit_arms_deposit_only_after_combat_loot() {
    let scenario = get("hill_giant_loot_deposit").expect("hill_giant_loot_deposit registered");
    assert_eq!(
        scenario.proof,
        Proof::BankItemId {
            id: BIG_BONES_ID,
            count: 1,
        }
    );
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let watch: Vec<_> = scenario.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect();
    let xp = watch
        .iter()
        .position(|arm| {
            *arm == Proof::StatXpGain {
                id: STRENGTH_STAT,
                min: 1,
            }
        })
        .expect("combat-first Strength XP");
    let loot = watch
        .iter()
        .position(|arm| {
            *arm == Proof::ItemId {
                id: BIG_BONES_ID,
                count: 1,
            }
        })
        .expect("looted Big bones in pack");
    let deposit = watch
        .iter()
        .position(|arm| {
            *arm == Proof::BankItemId {
                id: BIG_BONES_ID,
                count: 1,
            }
        })
        .expect("fresh Varrock West deposit");
    assert!(xp < loot && loot < deposit);
    assert!(!watch.contains(&Proof::BankClosed));
    assert!(!watch.iter().any(|arm| matches!(
        arm,
        Proof::FreshStatXpGain {
            id: STRENGTH_STAT,
            min: 1
        }
    )));
    let full = get("hill_giant_bank").expect("hill_giant_bank registered");
    let full_start = full
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let full_watch: Vec<_> = full.steps[full_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect();
    let full_xp = full_watch
        .iter()
        .position(|arm| {
            *arm == Proof::StatXpGain {
                id: STRENGTH_STAT,
                min: 1,
            }
        })
        .unwrap();
    assert_eq!(
        full_watch[full_xp + 1],
        Proof::BankItemId {
            id: BIG_BONES_ID,
            count: 1,
        },
        "frozen full-cycle cell still arms bank deposit at first XP"
    );
}

#[test]
fn hostile_fields_follow_safe_pretele_precondition_acknowledgements() {
    for name in [
        "chaos_druid",
        "moss_giant",
        "hill_giant",
        "auto_fighter",
        "rock_crab",
        "green_dragon",
        "fire_giant",
        "ardy_fighter",
        "rock_crab_bank",
        "green_dragon_bank",
        "green_dragon_tele",
        "fire_giant_approach",
        "fire_giant_bank",
    ] {
        let scenario = get(name).unwrap_or_else(|| panic!("{name} registered"));
        let start = scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let arrival = scenario.steps[..start]
            .iter()
            .position(|step| matches!(step.wait.arm, Proof::ArrivedNear { .. }))
            .unwrap_or_else(|| panic!("{name} field arrival"));
        for proof in [
            Proof::Stat {
                id: 0,
                min: COMBAT_ATTACK_LEVEL,
            },
            Proof::Stat {
                id: STRENGTH_STAT,
                min: COMBAT_ATTACK_LEVEL,
            },
            Proof::Stat {
                id: 3,
                min: COMBAT_ATTACK_LEVEL,
            },
        ] {
            let ack = scenario.steps[..start]
                .iter()
                .position(|step| step.wait.arm == proof)
                .unwrap_or_else(|| panic!("{name} missing {proof:?}"));
            assert!(
                ack < arrival,
                "{name} must acknowledge {proof:?} on the safe tile"
            );
        }
        for step in &scenario.steps[arrival + 1..start] {
            assert!(
                !matches!(
                    step.wait.arm,
                    Proof::Stat { .. }
                        | Proof::Item { .. }
                        | Proof::ItemId { .. }
                        | Proof::ItemIdAtMost { .. }
                ),
                "{name} leaves a precondition acknowledgement after the hostile-field tele: {}",
                step.name
            );
        }
    }

    let coal = get("coal_trucks").expect("coal_trucks");
    let start = coal
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let arrival = coal.steps[..start]
        .iter()
        .position(|step| matches!(step.wait.arm, Proof::ArrivedNear { .. }))
        .expect("coal-mine arrival");
    for proof in [
        Proof::Stat { id: 0, min: 48 },
        Proof::Stat {
            id: STRENGTH_STAT,
            min: 48,
        },
        Proof::Stat { id: 1, min: 48 },
        Proof::Stat { id: 3, min: 48 },
        Proof::Stat {
            id: MINING_STAT,
            min: 60,
        },
        Proof::ItemId {
            id: RUNE_PICKAXE_ID,
            count: 1,
        },
        Proof::ItemIdAtMost {
            id: COAL_ID,
            count: 0,
        },
    ] {
        let ack = coal.steps[..start]
            .iter()
            .position(|step| step.wait.arm == proof)
            .unwrap_or_else(|| panic!("coal_trucks missing {proof:?}"));
        assert!(
            ack < arrival,
            "coal_trucks must acknowledge {proof:?} before the bat mine"
        );
    }
}

#[test]
fn alcher_generated_custom_cases_select_custom_and_ack_exact_ids() {
    for (name, custom_item) in [
        ("alcher_custom_alias", "adamant_scimitar"),
        ("alcher_custom_name", "Adamant scimitar"),
    ] {
        let scenario = get(name).unwrap_or_else(|| panic!("{name} is registered"));
        assert_eq!(scenario.settings.start_script, Some("Alcher"));
        assert_eq!(scenario.settings.deadline, SCRIPT_GOLD_DEADLINE);
        assert_eq!(scenario.settings.terminal_shot, Some(name));
        let inject = settings_inject_map(scenario.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("items"),
            Some(&Value::Array(vec![Value::String("custom".into())]))
        );
        assert_eq!(
            inject.get("customItem"),
            Some(&Value::String(custom_item.into()))
        );
        assert_eq!(inject.get("alchs"), Some(&Value::from(1.0)));
        let start = scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        assert_eq!(scenario.steps[start - 1].wait.arm, Proof::BankClosed);
        let seed_arms = scenario.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed_arms.contains(&Proof::BankItemId {
            id: ADAMANT_SCIMITAR_ID,
            count: 1,
        }));
        assert!(seed_arms.contains(&Proof::BankItemId {
            id: NATURE_RUNE_ID,
            count: 1,
        }));
        assert!(seed_arms.contains(&Proof::BankItemId {
            id: STAFF_OF_FIRE_ID,
            count: 1,
        }));
        assert!(seed_arms.contains(&Proof::BankItemIdAtMost {
            id: CERT_ADAMANT_SCIMITAR_ID,
            count: 0,
        }));
        assert!(seed_arms.contains(&Proof::ItemIdAtMost {
            id: CERT_ADAMANT_SCIMITAR_ID,
            count: 0,
        }));
        assert!(seed_arms.contains(&Proof::ItemIdAtMost {
            id: COINS_ID,
            count: 0,
        }));
        let watch_arms = scenario.steps[start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert_eq!(
            watch_arms[0],
            Proof::ItemId {
                id: CERT_ADAMANT_SCIMITAR_ID,
                count: 1,
            },
            "{name} must observe the noted id before XP"
        );
        assert!(watch_arms.contains(&Proof::StatXpGain {
            id: 6,
            min: HIGH_ALCH_MAGIC_XP,
        }));
        assert!(watch_arms.contains(&Proof::ItemId {
            id: COINS_ID,
            count: ADAMANT_SCIMITAR_ALCH_COINS,
        }));
        assert_eq!(
            scenario.proof,
            Proof::StatXpGain {
                id: 6,
                min: HIGH_ALCH_MAGIC_XP,
            }
        );
        assert!(names().contains(&name));
    }

    let historical = get("alcher_custom").expect("historical custom case preserved");
    let inject = settings_inject_map(historical.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("customItem"),
        Some(&Value::String("rune_chainbody".into()))
    );
}

#[test]
fn alcher_option_rows_preserve_frozen_settings_seeds_and_inner_deadline() {
    let low = get("alcher_low").expect("alcher_low is registered");
    assert_eq!(low.settings.start_script, Some("Alcher"));
    assert_eq!(low.settings.deadline, SCRIPT_GOLD_DEADLINE);
    assert_eq!(low.settings.terminal_shot, Some("alcher_low"));
    let inject = settings_inject_map(low.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("items"),
        Some(&Value::Array(vec![Value::String("rune_chainbody".into())]))
    );
    assert_eq!(inject.get("alchs"), Some(&Value::from(10.0)));
    assert_eq!(inject.get("spell"), Some(&Value::String("Low".into())));
    let start = low
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert_eq!(low.steps[start - 1].wait.arm, Proof::BankClosed);
    let seed_arms = low.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed_arms.contains(&Proof::Stat {
        id: MAGIC_STAT,
        min: 25
    }));
    assert!(seed_arms.contains(&Proof::BankItemId {
        id: RUNE_CHAINBODY_ID,
        count: 12,
    }));
    assert!(seed_arms.contains(&Proof::BankItemId {
        id: NATURE_RUNE_ID,
        count: 200,
    }));
    assert!(seed_arms.contains(&Proof::BankItemId {
        id: STAFF_OF_FIRE_ID,
        count: 1,
    }));
    assert!(seed_arms.contains(&Proof::BankItemIdAtMost {
        id: FIRE_BATTLESTAFF_ID,
        count: 0,
    }));
    assert_eq!(
        low.steps[start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>(),
        vec![
            Proof::ItemId {
                id: CERT_RUNE_CHAINBODY_ID,
                count: 1,
            },
            Proof::EquipmentId {
                id: STAFF_OF_FIRE_ID,
            },
            Proof::StatXpGain {
                id: MAGIC_STAT,
                min: LOW_ALCH_MAGIC_XP,
            },
            Proof::ItemId {
                id: COINS_ID,
                count: RUNE_CHAINBODY_LOW_ALCH_COINS,
            },
        ]
    );
    assert_eq!(
        low.proof,
        Proof::StatXpGain {
            id: MAGIC_STAT,
            min: LOW_ALCH_MAGIC_XP,
        }
    );

    let staff = get("alcher_fire_battlestaff").expect("alcher_fire_battlestaff is registered");
    assert_eq!(staff.settings.start_script, Some("Alcher"));
    assert_eq!(staff.settings.deadline, SCRIPT_GOLD_DEADLINE);
    assert_eq!(
        staff.settings.terminal_shot,
        Some("alcher_fire_battlestaff")
    );
    let inject = settings_inject_map(staff.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("items"),
        Some(&Value::Array(vec![Value::String("rune_chainbody".into())]))
    );
    assert_eq!(inject.get("alchs"), Some(&Value::from(8.0)));
    assert!(!inject.contains_key("spell"));
    let start = staff
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert_eq!(staff.steps[start - 1].wait.arm, Proof::BankClosed);
    let seed_arms = staff.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed_arms.contains(&Proof::Stat {
        id: MAGIC_STAT,
        min: 70
    }));
    assert!(seed_arms.contains(&Proof::Stat {
        id: ATTACK_STAT,
        min: 40
    }));
    assert!(seed_arms.contains(&Proof::BankItemId {
        id: FIRE_BATTLESTAFF_ID,
        count: 1,
    }));
    assert!(seed_arms.contains(&Proof::BankItemId {
        id: NATURE_RUNE_ID,
        count: 200,
    }));
    assert!(seed_arms.contains(&Proof::BankItemId {
        id: RUNE_CHAINBODY_ID,
        count: 8,
    }));
    assert!(seed_arms.contains(&Proof::BankItemIdAtMost {
        id: STAFF_OF_FIRE_ID,
        count: 0,
    }));
    assert_eq!(
        staff.steps[start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>(),
        vec![
            Proof::EquipmentId {
                id: FIRE_BATTLESTAFF_ID,
            },
            Proof::ItemId {
                id: CERT_RUNE_CHAINBODY_ID,
                count: 1,
            },
            Proof::StatXpGain {
                id: MAGIC_STAT,
                min: HIGH_ALCH_MAGIC_XP,
            },
            Proof::ItemId {
                id: COINS_ID,
                count: RUNE_CHAINBODY_HIGH_ALCH_COINS,
            },
        ]
    );
    assert_eq!(
        staff.proof,
        Proof::StatXpGain {
            id: MAGIC_STAT,
            min: HIGH_ALCH_MAGIC_XP,
        }
    );
}

#[test]
fn alcher_swarm_drain_preserves_frozen_seed_settings_and_macro_event_inject() {
    let swarm = get("alcher_swarm_drain").expect("alcher_swarm_drain is registered");
    assert_eq!(swarm.settings.start_script, Some("Alcher"));
    assert_eq!(swarm.settings.deadline, Duration::from_secs(420));
    assert_eq!(swarm.settings.terminal_shot, Some("alcher_swarm_drain"));
    let inject = settings_inject_map(swarm.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("items"),
        Some(&Value::Array(vec![
            Value::String("rune_chainbody".into()),
            Value::String("yew_longbow".into()),
        ]))
    );
    assert_eq!(inject.get("alchs"), Some(&Value::from(20.0)));
    assert!(!inject.contains_key("spell"));
    let start = swarm
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert_eq!(swarm.steps[start - 1].wait.arm, Proof::BankClosed);
    let seed_arms = swarm.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed_arms.contains(&Proof::Stat {
        id: MAGIC_STAT,
        min: 70
    }));
    assert!(seed_arms.contains(&Proof::BankItemId {
        id: RUNE_CHAINBODY_ID,
        count: 20,
    }));
    assert!(seed_arms.contains(&Proof::BankItemId {
        id: YEW_LONGBOW_ID,
        count: 8,
    }));
    assert!(seed_arms.contains(&Proof::BankItemId {
        id: NATURE_RUNE_ID,
        count: 200,
    }));
    assert!(seed_arms.contains(&Proof::BankItemId {
        id: STAFF_OF_FIRE_ID,
        count: 1,
    }));
    let post = swarm.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert_eq!(
        post,
        vec![
            Proof::ItemId {
                id: CERT_RUNE_CHAINBODY_ID,
                count: 1,
            },
            Proof::EquipmentId {
                id: STAFF_OF_FIRE_ID,
            },
            Proof::StatXpGain {
                id: MAGIC_STAT,
                min: HIGH_ALCH_MAGIC_XP,
            },
            Proof::ItemId {
                id: COINS_ID,
                count: RUNE_CHAINBODY_HIGH_ALCH_COINS,
            },
            Proof::ItemIdAtMost {
                id: CERT_RUNE_CHAINBODY_ID,
                count: 19,
            },
            Proof::ItemIdAtMost {
                id: NATURE_RUNE_ID,
                count: 19,
            },
            Proof::NpcNameTargetingLocal {
                name: SWARM_NPC_NAME,
            },
        ]
    );
    let inject_step = swarm
        .steps
        .iter()
        .find(|step| step.name.contains("macro_event"))
        .expect("swarm inject step");
    assert!(matches!(inject_step.kind, StepKind::Repeat { .. }));
    assert_eq!(inject_step.wait.budget_ticks, 50);
    assert_eq!(SWARM_MACRO_EVENT_CHEAT, "~macro_event 1");
    assert_eq!(SWARM_NPC_NAME, "Swarm");
    assert_eq!(
        swarm.proof,
        Proof::StatXpGain {
            id: MAGIC_STAT,
            min: HIGH_ALCH_MAGIC_XP,
        }
    );
}

type SnapshotPred = Box<dyn Fn(&mut Client, &GameSnapshot) -> bool + Send + Sync>;

fn swarm_inject_send(scenario: &Scenario) -> &SnapshotPred {
    let step = scenario
        .steps
        .iter()
        .find(|step| step.name.contains("macro_event"))
        .expect("swarm inject step");
    match &step.kind {
        StepKind::Repeat { send } => send,
        _ => panic!("swarm inject must re-invoke send until Swarm targets the local player"),
    }
}

fn swarm_snapshot(client: &mut Client) -> GameSnapshot {
    use client::io::ServerProt;
    for prot in [
        ServerProt::PLAYER_INFO,
        ServerProt::NPC_INFO,
        ServerProt::MESSAGE_GAME,
    ] {
        client.bump_gens(prot);
    }
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(client);
    snapshot
}

fn plant_swarm_npc(client: &mut Client, face_entity: i32) {
    use client::client::ClientNpc;
    use client::config::NpcType;
    let cache = std::sync::Arc::get_mut(&mut client.cache).expect("sole cache owner");
    if cache.npcs.is_empty() {
        cache.npcs.push(NpcType::default());
    }
    cache.npcs[0].name = SWARM_NPC_NAME.into();
    let mut npc = ClientNpc {
        r#type: Some(0),
        ..Default::default()
    };
    npc.entity.face_entity = face_entity;
    client.npc[1] = Some(Box::new(npc));
    client.npc_ids[0] = 1;
    client.npc_count = 1;
    client.self_slot = 0;
}

#[test]
fn alcher_swarm_inject_sends_once_then_holds_without_fresh_rejection() {
    let swarm = get("alcher_swarm_drain").expect("alcher_swarm_drain is registered");
    let send = swarm_inject_send(&swarm);
    let mut client = native_seed_client();
    let snapshot = GameSnapshot::new();
    assert!(send(&mut client, &snapshot));
    assert!(emitted_has(&client, SWARM_MACRO_EVENT_CHEAT));
    let pos = client.out.pos;
    assert!(send(&mut client, &snapshot));
    assert_eq!(
        client.out.pos, pos,
        "a sent packet is not spawn; do not resend without a newer please_finish"
    );
    assert!(
        !swarm_macro_event_should_send(&snapshot, true, i32::MIN),
        "no chat rejection must not authorize another send"
    );
}

#[test]
fn alcher_swarm_inject_retries_only_on_a_fresh_busy_rejection() {
    let swarm = get("alcher_swarm_drain").expect("alcher_swarm_drain is registered");
    let send = swarm_inject_send(&swarm);
    let mut client = native_seed_client();
    let snapshot = GameSnapshot::new();
    assert!(send(&mut client, &snapshot));
    client.out.pos = 0;

    client.add_chat(0, SWARM_BUSY_REJECT, "");
    let rejected = swarm_snapshot(&mut client);
    let first_seq = swarm_busy_reject_seq(&rejected).expect("busy reject is sequenced");
    assert!(swarm_macro_event_should_send(&rejected, true, i32::MIN));
    assert!(send(&mut client, &rejected));
    assert!(emitted_has(&client, SWARM_MACRO_EVENT_CHEAT));
    client.out.pos = 0;
    assert!(send(&mut client, &rejected));
    assert!(
        !emitted_has(&client, SWARM_MACRO_EVENT_CHEAT),
        "the same please_finish sequence is not a fresh rejection"
    );
    assert!(!swarm_macro_event_should_send(&rejected, true, first_seq));

    client.add_chat(0, SWARM_BUSY_REJECT, "");
    let again = swarm_snapshot(&mut client);
    let second_seq = swarm_busy_reject_seq(&again).expect("a newer busy reject");
    assert!(second_seq > first_seq);
    assert!(swarm_macro_event_should_send(&again, true, first_seq));
    assert!(send(&mut client, &again));
    assert!(emitted_has(&client, SWARM_MACRO_EVENT_CHEAT));
}

#[test]
fn alcher_swarm_inject_does_not_duplicate_an_accepted_targeting_swarm() {
    let swarm = get("alcher_swarm_drain").expect("alcher_swarm_drain is registered");
    let send = swarm_inject_send(&swarm);
    let mut client = native_seed_client();
    plant_swarm_npc(&mut client, api::snapshot::PLAYER_FACE_BASE);
    let accepted = swarm_snapshot(&mut client);
    assert!(swarm_macro_event_accepted(&accepted));
    assert!(!swarm_macro_event_should_send(&accepted, false, i32::MIN));
    assert!(send(&mut client, &accepted));
    assert!(
        !emitted_has(&client, SWARM_MACRO_EVENT_CHEAT),
        "accepted Swarm targeting local must not emit another ~macro_event 1"
    );

    client.add_chat(0, SWARM_BUSY_REJECT, "");
    let stale_reject = swarm_snapshot(&mut client);
    assert!(swarm_macro_event_accepted(&stale_reject));
    assert!(send(&mut client, &stale_reject));
    assert!(
        !emitted_has(&client, SWARM_MACRO_EVENT_CHEAT),
        "please_finish must not retrigger once Swarm already targets the player"
    );

    let mut other = native_seed_client();
    plant_swarm_npc(&mut other, api::snapshot::PLAYER_FACE_BASE + 3);
    let untargeted = swarm_snapshot(&mut other);
    assert!(!swarm_macro_event_accepted(&untargeted));
    assert!(swarm_macro_event_should_send(&untargeted, false, i32::MIN));
}

#[test]
fn inventory_production_cases_register_exact_ids_and_bank_cycles() {
    let bronze = get("dart_fletcher").expect("dart_fletcher");
    assert_eq!(bronze.settings.start_script, Some("DartFletcher"));
    assert_eq!(bronze.settings.deadline, SCRIPT_GOLD_DEADLINE);
    let inject = settings_inject_map(bronze.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("tier"), Some(&Value::String("Bronze".into())));
    let bronze_start = bronze
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert_ne!(bronze.steps[bronze_start - 1].wait.arm, Proof::BankClosed);
    let bronze_seed = bronze.steps[..bronze_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(bronze_seed.contains(&Proof::ItemId {
        id: BRONZE_DART_TIP_ID,
        count: 100,
    }));
    assert!(bronze_seed.contains(&Proof::ItemId {
        id: FEATHER_ID,
        count: 100,
    }));
    assert!(bronze_seed.contains(&Proof::ItemIdAtMost {
        id: BRONZE_DART_ID,
        count: 0,
    }));
    let bronze_watch = bronze.steps[bronze_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert_eq!(
        bronze_watch[0],
        Proof::StatXpGain {
            id: FLETCHING_STAT,
            min: 1
        }
    );
    assert!(bronze_watch.contains(&Proof::ItemId {
        id: BRONZE_DART_ID,
        count: 10,
    }));
    assert!(bronze_watch.contains(&Proof::ItemId {
        id: BRONZE_DART_ID,
        count: 20,
    }));
    assert!(bronze_watch.contains(&Proof::ItemIdAtMost {
        id: BRONZE_DART_TIP_ID,
        count: 90,
    }));
    assert!(bronze_watch.contains(&Proof::ItemIdAtMost {
        id: FEATHER_ID,
        count: 90,
    }));
    assert!(bronze_watch.contains(&Proof::ItemIdAtMost {
        id: IRON_DART_ID,
        count: 0,
    }));
    assert_eq!(
        bronze.proof,
        Proof::StatXpGain {
            id: FLETCHING_STAT,
            min: 2
        }
    );

    let iron = get("dart_fletcher_iron").expect("dart_fletcher_iron");
    let inject = settings_inject_map(iron.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("tier"), Some(&Value::String("Iron".into())));
    let iron_start = iron
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let iron_seed = iron.steps[..iron_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(iron_seed.contains(&Proof::Stat {
        id: FLETCHING_STAT,
        min: 22,
    }));
    assert!(iron_seed.contains(&Proof::ItemId {
        id: IRON_DART_TIP_ID,
        count: 100,
    }));
    let iron_watch = iron.steps[iron_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(iron_watch.contains(&Proof::ItemId {
        id: IRON_DART_ID,
        count: 10,
    }));
    assert!(iron_watch.contains(&Proof::ItemIdAtMost {
        id: BRONZE_DART_ID,
        count: 0,
    }));

    let herb = get("herb_cleaner").expect("herb_cleaner");
    assert_eq!(herb.settings.start_script, Some("HerbCleaner"));
    let inject = settings_inject_map(herb.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("herbs"), Some(&Value::Array(vec![])));
    let herb_start = herb
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert_eq!(herb.steps[herb_start - 1].wait.arm, Proof::BankClosed);
    let herb_seed = herb.steps[..herb_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(herb_seed.contains(&Proof::Stat {
        id: HERBLORE_STAT,
        min: 3,
    }));
    assert!(herb_seed.contains(&Proof::BankItemId {
        id: UNIDENTIFIED_GUAM_ID,
        count: 30,
    }));
    assert!(herb_seed.contains(&Proof::ItemIdAtMost {
        id: GUAM_LEAF_ID,
        count: 0,
    }));
    let herb_watch = herb.steps[herb_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert_eq!(
        herb_watch[0],
        Proof::StatXpGain {
            id: HERBLORE_STAT,
            min: 1
        }
    );
    assert!(herb_watch.contains(&Proof::ItemId {
        id: GUAM_LEAF_ID,
        count: 28,
    }));
    assert!(herb_watch.contains(&Proof::BankItemId {
        id: GUAM_LEAF_ID,
        count: 28,
    }));
    assert!(herb_watch.contains(&Proof::ItemId {
        id: UNIDENTIFIED_GUAM_ID,
        count: 1,
    }));
    assert_eq!(
        herb.proof,
        Proof::StatXpGain {
            id: HERBLORE_STAT,
            min: 2
        }
    );

    let named_herb = get("herb_cleaner_named").expect("herb_cleaner_named");
    let inject = settings_inject_map(named_herb.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("herbs"),
        Some(&Value::Array(vec![Value::String("Guam leaf".into())]))
    );
    let named_herb_start = named_herb
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let named_herb_seed = named_herb.steps[..named_herb_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(named_herb_seed.contains(&Proof::Stat {
        id: HERBLORE_STAT,
        min: 5,
    }));
    assert!(named_herb_seed.contains(&Proof::BankItemId {
        id: UNIDENTIFIED_MARENTILL_ID,
        count: 4,
    }));
    let named_herb_watch = named_herb.steps[named_herb_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(named_herb_watch.contains(&Proof::BankItemId {
        id: UNIDENTIFIED_MARENTILL_ID,
        count: 4,
    }));
    assert!(named_herb_watch.contains(&Proof::ItemIdAtMost {
        id: UNIDENTIFIED_MARENTILL_ID,
        count: 0,
    }));

    let empty_herb = get("herb_cleaner_empty_bank").expect("herb_cleaner_empty_bank");
    assert_eq!(empty_herb.settings.start_script, Some("HerbCleaner"));
    assert_eq!(empty_herb.settings.deadline, Duration::from_secs(420));
    let inject = settings_inject_map(empty_herb.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("herbs"),
        Some(&Value::Array(vec![
            Value::String("Guam leaf".into()),
            Value::String("Marrentill".into()),
        ]))
    );
    let empty_start = empty_herb
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let empty_seed = empty_herb.steps[..empty_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(empty_seed.contains(&Proof::Stat {
        id: HERBLORE_STAT,
        min: 20,
    }));
    assert!(empty_seed.contains(&Proof::BankItemId {
        id: UNIDENTIFIED_GUAM_ID,
        count: 20,
    }));
    assert!(empty_seed.contains(&Proof::BankItemIdAtMost {
        id: UNIDENTIFIED_MARENTILL_ID,
        count: 0,
    }));
    assert_eq!(
        empty_herb.proof,
        Proof::StatXpGain {
            id: HERBLORE_STAT,
            min: 1
        }
    );

    let gems = get("gem_cutter").expect("gem_cutter");
    assert_eq!(gems.settings.start_script, Some("GemCutter"));
    let inject = settings_inject_map(gems.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("gems"), Some(&Value::Array(vec![])));
    let gems_start = gems
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert_eq!(gems.steps[gems_start - 1].wait.arm, Proof::BankClosed);
    let gems_seed = gems.steps[..gems_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(gems_seed.contains(&Proof::Stat {
        id: CRAFTING_STAT,
        min: 20,
    }));
    assert!(gems_seed.contains(&Proof::BankItemId {
        id: UNCUT_SAPPHIRE_ID,
        count: 28,
    }));
    assert!(gems_seed.contains(&Proof::BankItemId {
        id: CHISEL_ID,
        count: 1,
    }));
    let gems_watch = gems.steps[gems_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(gems_watch.contains(&Proof::ItemId {
        id: SAPPHIRE_ID,
        count: 27,
    }));
    assert!(gems_watch.contains(&Proof::BankItemId {
        id: SAPPHIRE_ID,
        count: 27,
    }));
    assert!(gems_watch.contains(&Proof::ItemId {
        id: CHISEL_ID,
        count: 1,
    }));
    assert!(gems_watch.contains(&Proof::ItemIdAtMost {
        id: CRUSHED_GEMSTONE_ID,
        count: 0,
    }));
    assert_eq!(
        gems.proof,
        Proof::StatXpGain {
            id: CRAFTING_STAT,
            min: 2
        }
    );

    let named_gems = get("gem_cutter_named").expect("gem_cutter_named");
    let inject = settings_inject_map(named_gems.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("gems"),
        Some(&Value::Array(vec![Value::String("Sapphire".into())]))
    );
    let named_gems_start = named_gems
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let named_gems_seed = named_gems.steps[..named_gems_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(named_gems_seed.contains(&Proof::BankItemId {
        id: UNCUT_OPAL_ID,
        count: 4,
    }));
    let named_gems_watch = named_gems.steps[named_gems_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(named_gems_watch.contains(&Proof::BankItemId {
        id: UNCUT_OPAL_ID,
        count: 4,
    }));
    assert!(named_gems_watch.contains(&Proof::ItemIdAtMost {
        id: UNCUT_OPAL_ID,
        count: 0,
    }));

    for name in [
        "dart_fletcher",
        "dart_fletcher_iron",
        "herb_cleaner",
        "herb_cleaner_named",
        "gem_cutter",
        "gem_cutter_named",
    ] {
        assert!(names().contains(&name));
    }
}

#[test]
fn location_world_cases_register_exact_settings_and_witnesses() {
    let door = get("door_opener").expect("door_opener");
    assert_eq!(door.settings.start_script, Some("DoorOpener"));
    assert_eq!(door.settings.deadline, SCRIPT_GOLD_DEADLINE);
    let inject = settings_inject_map(door.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("stand"),
        Some(&Value::String("3208,3212,0".into()))
    );
    let door_start = door
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let door_seed = door.steps[..door_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(door_seed.contains(&Proof::LocActionNear {
        id: CLOSED_ID,
        x: 3208,
        z: 3211,
        level: 0,
        radius: 1,
        action: "Open",
        present: true,
    }));
    assert_eq!(
        door.steps[door_start + 1].wait.arm,
        Proof::LocIdNear {
            id: OPEN_ID,
            x: 3208,
            z: 3211,
            level: 0,
            radius: 3,
        }
    );

    let gate = get("door_opener_gate").expect("door_opener_gate");
    let inject = settings_inject_map(gate.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("obstacle"), Some(&Value::String("gate".into())));
    assert_eq!(
        inject.get("stand"),
        Some(&Value::String("3213,3260,0".into()))
    );
    let gate_start = gate
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let gate_seed = gate.steps[..gate_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(gate_seed.contains(&Proof::LocActionNear {
        id: GATE_CLOSED_ID,
        x: 3213,
        z: 3261,
        level: 0,
        radius: 1,
        action: "Open",
        present: true,
    }));
    assert_eq!(
        gate.steps[gate_start + 1].wait.arm,
        Proof::LocIdNear {
            id: GATE_OPEN_ID,
            x: 3213,
            z: 3261,
            level: 0,
            radius: 3,
        }
    );

    let gnome = get("gnome_course").expect("gnome_course");
    assert_eq!(gnome.settings.start_script, Some("GnomeCourse"));
    assert!(gnome.settings.script_settings_inject.is_none());
    let gnome_start = gnome
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let gnome_watch = gnome.steps[gnome_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert_eq!(
        gnome_watch[0],
        Proof::StatXpGain {
            id: AGILITY_STAT,
            min: 1
        }
    );
    assert!(gnome_watch.contains(&Proof::ArrivedNear {
        x: 2474,
        z: 3429,
        level: 0,
        radius: 3,
    }));
    assert!(gnome_watch.contains(&Proof::ArrivedNear {
        x: 2487,
        z: 3420,
        level: 0,
        radius: 3,
    }));
    assert!(gnome_watch.contains(&Proof::ArrivedNear {
        x: 2484,
        z: 3431,
        level: 0,
        radius: 6,
    }));
    assert_eq!(
        gnome.proof,
        Proof::StatXpGain {
            id: AGILITY_STAT,
            min: 94
        }
    );

    let radius = get("gnome_course_radius").expect("gnome_course_radius");
    let inject = settings_inject_map(radius.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("searchRadius").and_then(Value::as_f64),
        Some(8.0)
    );

    let flax = get("flax_picker").expect("flax_picker");
    assert_eq!(flax.settings.start_script, Some("FlaxPicker"));
    let flax_start = flax
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let flax_seed = flax.steps[..flax_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(flax_seed.contains(&Proof::ItemIdAtMost {
        id: FLAX_ID,
        count: 0,
    }));
    let flax_watch = flax.steps[flax_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(flax_watch.contains(&Proof::ItemId {
        id: FLAX_ID,
        count: 28,
    }));
    assert!(flax_watch.contains(&Proof::BankItemId {
        id: FLAX_ID,
        count: 28,
    }));
    assert!(flax_watch.contains(&Proof::BankClosed));
    assert!(flax_watch.contains(&Proof::ArrivedNear {
        x: 2741,
        z: 3444,
        level: 0,
        radius: 12,
    }));
    assert_eq!(
        flax.proof,
        Proof::ItemId {
            id: FLAX_ID,
            count: 1,
        }
    );

    for name in [
        "door_opener",
        "door_opener_gate",
        "gnome_course",
        "gnome_course_radius",
        "flax_picker",
    ] {
        assert!(names().contains(&name));
    }
}

#[test]
fn wildy_agility_registers_the_full_ordered_course_on_the_observed_player_plane() {
    let wildy = get("wildy_agility").expect("wildy_agility");
    assert_eq!(wildy.settings.start_script, Some("WildyAgility"));
    assert_eq!(wildy.settings.deadline, SCRIPT_GOLD_DEADLINE);
    let inject = settings_inject_map(wildy.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("acquireFoodAtStart"), Some(&Value::Bool(false)));
    assert_eq!(inject.get("minFood").and_then(Value::as_f64), Some(0.0));

    let start = wildy
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("StartScript");
    let seed = wildy.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(wildy.steps[..start]
        .iter()
        .any(|step| step.name.contains("Agility 52")));
    assert!(seed.contains(&Proof::Stat {
        id: HITPOINTS_STAT,
        min: 40,
    }));
    assert!(seed.contains(&Proof::ItemId { id: 379, count: 5 }));
    assert!(seed.contains(&Proof::ArrivedNear {
        x: 2998,
        z: 3916,
        level: 0,
        radius: 2,
    }));

    let watch = wildy.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert_eq!(
        watch,
        vec![
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 15,
            },
            Proof::ArrivedNear {
                x: 2998,
                z: 3934,
                level: 0,
                radius: 2,
            },
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 27,
            },
            Proof::ArrivedNear {
                x: 3004,
                z: 3947,
                level: 0,
                radius: 3,
            },
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 47,
            },
            Proof::ArrivedNear {
                x: 3005,
                z: 3958,
                level: 0,
                radius: 3,
            },
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 67,
            },
            Proof::ArrivedNear {
                x: 2996,
                z: 3960,
                level: 0,
                radius: 3,
            },
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 87,
            },
            Proof::ArrivedNear {
                x: 2994,
                z: 3945,
                level: 0,
                radius: 3,
            },
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 586,
            },
            Proof::ArrivedNear {
                x: 2994,
                z: 3933,
                level: 0,
                radius: 3,
            },
            Proof::ArrivedNear {
                x: 3004,
                z: 3947,
                level: 0,
                radius: 3,
            },
        ]
    );
    assert_eq!(
        wildy.proof,
        Proof::StatXpGain {
            id: AGILITY_STAT,
            min: 598,
        }
    );
    assert!(names().contains(&"wildy_agility"));
}

#[test]
fn brimhaven_agility_registers_fee_tags_ticket_and_subsequent_xp() {
    let brim = get("brimhaven_agility").expect("brimhaven_agility");
    assert_eq!(brim.settings.start_script, Some("BrimhavenAgility"));
    assert_eq!(brim.settings.deadline, SCRIPT_GOLD_DEADLINE);
    let inject = settings_inject_map(brim.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("stealRestock"), Some(&Value::Bool(false)));
    assert_eq!(
        inject.get("bankAtTickets").and_then(Value::as_f64),
        Some(1000.0)
    );

    let start = brim
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("StartScript");
    let seed = brim.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(brim.steps[..start].iter().any(|step| step.name
        == "seed Agility 52, 1000 Coins and ten Lobsters, then tele to the arena entrance"));
    assert!(seed.contains(&Proof::ChatClosed));
    assert!(seed.contains(&Proof::ItemId {
        id: 995,
        count: 1000,
    }));
    assert!(seed.contains(&Proof::ItemId { id: 379, count: 10 }));
    assert!(seed.contains(&Proof::ItemIdAtMost { id: 2996, count: 0 }));
    assert!(seed.contains(&Proof::ArrivedNear {
        x: 2809,
        z: 3194,
        level: 0,
        radius: 2,
    }));
    assert!(!seed
        .iter()
        .any(|proof| matches!(proof, Proof::ItemId { id: 2996, .. })));

    let watch = brim.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert_eq!(
        watch,
        vec![
            Proof::ItemIdAtMost {
                id: 995,
                count: 800,
            },
            Proof::Varp { id: 309, min: 2 },
            Proof::ArrivedNear {
                x: 2805,
                z: 9590,
                level: 3,
                radius: 2,
            },
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 1,
            },
            Proof::Chat {
                needle: "tag the next",
            },
            Proof::Varp { id: 309, min: 15 },
            Proof::ItemIdAtMost { id: 2996, count: 0 },
            Proof::ItemId { id: 2996, count: 1 },
            Proof::FreshStatXpGain {
                id: AGILITY_STAT,
                min: 1,
            },
        ]
    );
    assert_eq!(
        brim.proof,
        Proof::FreshStatXpGain {
            id: AGILITY_STAT,
            min: 1,
        }
    );
    assert!(names().contains(&"brimhaven_agility"));
}

#[test]
fn superheater_cases_register_exact_ids_and_bank_cycles() {
    let bronze = get("superheater").expect("superheater");
    assert_eq!(bronze.settings.start_script, Some("Superheater"));
    assert_eq!(bronze.settings.deadline, SCRIPT_GOLD_DEADLINE);
    let inject = settings_inject_map(bronze.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("bar"), Some(&Value::String("Bronze".into())));
    let bronze_start = bronze
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert_eq!(bronze.steps[bronze_start - 1].wait.arm, Proof::BankClosed);
    let bronze_seed = bronze.steps[..bronze_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(bronze_seed.contains(&Proof::Stat {
        id: MAGIC_STAT,
        min: SUPERHEAT_MAGIC,
    }));
    assert!(bronze_seed.contains(&Proof::Stat {
        id: SMITHING_STAT,
        min: BRONZE_SMITHING,
    }));
    assert!(bronze_seed.contains(&Proof::BankItemId {
        id: STAFF_OF_FIRE_ID,
        count: 1,
    }));
    assert!(bronze_seed.contains(&Proof::BankItemId {
        id: NATURE_RUNE_ID,
        count: SUPERHEATER_NATURES_SEED,
    }));
    assert!(bronze_seed.contains(&Proof::BankItemId {
        id: COPPER_ORE_ID,
        count: SUPERHEATER_ORE_SEED,
    }));
    assert!(bronze_seed.contains(&Proof::BankItemId {
        id: TIN_ORE_ID,
        count: SUPERHEATER_ORE_SEED,
    }));
    assert!(bronze_seed.contains(&Proof::ItemIdAtMost {
        id: BRONZE_BAR_ID,
        count: 0,
    }));
    let bronze_watch = bronze.steps[bronze_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert_eq!(
        bronze_watch[0],
        Proof::StatXpGain {
            id: MAGIC_STAT,
            min: 1
        }
    );
    assert!(bronze_watch.contains(&Proof::StatXpGain {
        id: SMITHING_STAT,
        min: 1
    }));
    assert!(bronze_watch.contains(&Proof::ItemId {
        id: BRONZE_BAR_ID,
        count: 1,
    }));
    assert!(bronze_watch.contains(&Proof::BankItemId {
        id: BRONZE_BAR_ID,
        count: 1,
    }));
    assert!(bronze_watch.contains(&Proof::ItemId {
        id: COPPER_ORE_ID,
        count: 1,
    }));
    assert!(bronze_watch.contains(&Proof::ItemId {
        id: TIN_ORE_ID,
        count: 1,
    }));
    assert!(bronze_watch.contains(&Proof::ItemIdAtMost {
        id: NATURE_RUNE_ID,
        count: 49,
    }));
    assert!(bronze_watch.contains(&Proof::BankClosed));
    assert_eq!(
        bronze.proof,
        Proof::StatXpGain {
            id: SMITHING_STAT,
            min: 2
        }
    );

    let steel = get("superheater_steel").expect("superheater_steel");
    let inject = settings_inject_map(steel.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("bar"), Some(&Value::String("Steel".into())));
    let steel_start = steel
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let steel_seed = steel.steps[..steel_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(steel_seed.contains(&Proof::Stat {
        id: SMITHING_STAT,
        min: STEEL_SMITHING,
    }));
    assert!(steel_seed.contains(&Proof::BankItemId {
        id: IRON_ORE_ID,
        count: SUPERHEATER_ORE_SEED,
    }));
    assert!(steel_seed.contains(&Proof::BankItemId {
        id: COAL_ID,
        count: SUPERHEATER_COAL_SEED,
    }));
    let steel_watch = steel.steps[steel_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(steel_watch.contains(&Proof::ItemId {
        id: STEEL_BAR_ID,
        count: 1,
    }));
    assert!(steel_watch.contains(&Proof::ItemId {
        id: COAL_ID,
        count: 2,
    }));
    assert!(steel_watch.contains(&Proof::ItemIdAtMost {
        id: IRON_BAR_ID,
        count: 0,
    }));

    let alt = get("superheater_fire_battlestaff").expect("superheater_fire_battlestaff");
    let inject = settings_inject_map(alt.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("bar"), Some(&Value::String("Bronze".into())));
    let alt_start = alt
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert_eq!(alt.steps[alt_start - 1].wait.arm, Proof::BankClosed);
    let alt_seed = alt.steps[..alt_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(alt_seed.contains(&Proof::BankItemId {
        id: FIRE_BATTLESTAFF_ID,
        count: 1,
    }));
    assert!(alt_seed.contains(&Proof::BankItemIdAtMost {
        id: STAFF_OF_FIRE_ID,
        count: 0,
    }));
    let alt_watch = alt.steps[alt_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(alt_watch.contains(&Proof::ItemId {
        id: BRONZE_BAR_ID,
        count: 1,
    }));
    assert!(alt_watch.contains(&Proof::ItemIdAtMost {
        id: STAFF_OF_FIRE_ID,
        count: 0,
    }));

    let silver = get("superheater_silver_low_natures").expect("superheater_silver_low_natures");
    assert_eq!(silver.settings.start_script, Some("Superheater"));
    assert_eq!(silver.settings.deadline, SCRIPT_GOLD_DEADLINE);
    let inject = settings_inject_map(silver.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("bar"), Some(&Value::String("Silver".into())));
    assert_eq!(inject.get("natures"), Some(&Value::from(28.0)));
    let silver_start = silver
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert_eq!(silver.steps[silver_start - 1].wait.arm, Proof::BankClosed);
    let silver_seed = silver.steps[..silver_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(silver_seed.contains(&Proof::Stat {
        id: SMITHING_STAT,
        min: SILVER_SMITHING,
    }));
    assert!(silver_seed.contains(&Proof::BankItemId {
        id: STAFF_OF_FIRE_ID,
        count: 1,
    }));
    assert!(silver_seed.contains(&Proof::BankItemId {
        id: NATURE_RUNE_ID,
        count: SUPERHEATER_NATURES_SEED,
    }));
    assert!(silver_seed.contains(&Proof::BankItemId {
        id: SILVER_ORE_ID,
        count: SUPERHEATER_ORE_SEED,
    }));
    assert!(!silver_seed.iter().any(|arm| matches!(
        arm,
        Proof::BankItemId {
            id: COPPER_ORE_ID | TIN_ORE_ID | IRON_ORE_ID | COAL_ID,
            ..
        }
    )));
    assert!(silver_seed.contains(&Proof::ItemIdAtMost {
        id: SILVER_BAR_ID,
        count: 0,
    }));
    let silver_watch = silver.steps[silver_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert_eq!(
        silver_watch[0],
        Proof::StatXpGain {
            id: MAGIC_STAT,
            min: 1
        }
    );
    assert!(silver_watch.contains(&Proof::ItemId {
        id: SILVER_BAR_ID,
        count: 1,
    }));
    assert!(silver_watch.contains(&Proof::ItemIdAtMost {
        id: NATURE_RUNE_ID,
        count: SUPERHEATER_NATURES_MIN - 1,
    }));
    assert!(silver_watch.contains(&Proof::BankItemId {
        id: SILVER_BAR_ID,
        count: 1,
    }));
    assert!(silver_watch.contains(&Proof::ItemId {
        id: SILVER_ORE_ID,
        count: SUPERHEATER_SINGLE_ORE_TRIP,
    }));
    assert!(silver_watch.contains(&Proof::ItemId {
        id: NATURE_RUNE_ID,
        count: SUPERHEATER_NATURES_MIN,
    }));
    assert!(silver_watch.contains(&Proof::BankClosed));
    assert!(silver_watch.contains(&Proof::FreshStatXpGain {
        id: MAGIC_STAT,
        min: 1
    }));
    assert!(silver_watch.contains(&Proof::FreshStatXpGain {
        id: SMITHING_STAT,
        min: 1
    }));
    assert_eq!(
        silver.proof,
        Proof::FreshStatXpGain {
            id: SMITHING_STAT,
            min: 1
        }
    );

    let mithril = get("superheater_mithril").expect("superheater_mithril");
    let inject = settings_inject_map(mithril.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("bar"), Some(&Value::String("Mithril".into())));
    let mithril_start = mithril
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let mithril_seed = mithril.steps[..mithril_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(mithril_seed.contains(&Proof::Stat {
        id: SMITHING_STAT,
        min: MITHRIL_SMITHING,
    }));
    assert!(mithril_seed.contains(&Proof::BankItemId {
        id: MITHRIL_ORE_ID,
        count: SUPERHEATER_ORE_SEED,
    }));
    assert!(mithril_seed.contains(&Proof::BankItemId {
        id: COAL_ID,
        count: SUPERHEATER_COAL_SEED,
    }));
    let mithril_watch = mithril.steps[mithril_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(mithril_watch.contains(&Proof::ItemId {
        id: MITHRIL_BAR_ID,
        count: 1,
    }));
    assert!(mithril_watch.contains(&Proof::ItemId {
        id: MITHRIL_ORE_ID,
        count: 1,
    }));
    assert!(mithril_watch.contains(&Proof::ItemId {
        id: COAL_ID,
        count: 4,
    }));
    assert!(mithril_watch.contains(&Proof::ItemIdAtMost {
        id: IRON_BAR_ID,
        count: 0,
    }));
    assert_eq!(
        mithril.proof,
        Proof::StatXpGain {
            id: SMITHING_STAT,
            min: 2
        }
    );

    for name in [
        "superheater",
        "superheater_steel",
        "superheater_fire_battlestaff",
        "superheater_silver_low_natures",
        "superheater_mithril",
    ] {
        assert!(names().contains(&name));
    }
}

#[test]
fn chicken_killer_bank_registers_loot_count_feather_trip() {
    let bank = get("chicken_killer_bank").expect("chicken_killer_bank");
    assert_eq!(bank.settings.start_script, Some("ChickenKiller"));
    assert_eq!(bank.settings.deadline, SCRIPT_GOLD_DEADLINE);
    let inject = settings_inject_map(bank.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("bankStrategy"),
        Some(&Value::String("Loot count".into()))
    );
    assert_eq!(inject.get("bankEveryItems"), Some(&Value::from(1.0)));
    assert_eq!(
        inject.get("lootMatch"),
        Some(&Value::String("feather".into()))
    );
    assert_eq!(
        inject.get("combatStyle"),
        Some(&Value::String("melee".into()))
    );
    assert!(inject.get("buryBones").is_none());
    let start = bank
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let seed = bank.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::ArrivedNear {
        x: 3029,
        z: 3294,
        level: 0,
        radius: 8,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: FEATHER_ID,
        count: 0,
    }));
    let watch = bank.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert_eq!(
        watch,
        vec![
            Proof::StatXpGain {
                id: STRENGTH_STAT,
                min: 1
            },
            Proof::ItemId {
                id: FEATHER_ID,
                count: 1
            },
            Proof::BankItemId {
                id: FEATHER_ID,
                count: 1
            },
            Proof::ItemIdAtMost {
                id: FEATHER_ID,
                count: 0
            },
            Proof::ArrivedNear {
                x: 3029,
                z: 3294,
                level: 0,
                radius: 6,
            },
            Proof::BankClosed,
            Proof::ItemId {
                id: FEATHER_ID,
                count: 1
            },
        ]
    );
    assert_eq!(
        bank.proof,
        Proof::ItemId {
            id: FEATHER_ID,
            count: 1
        }
    );
    let core = get("chicken_killer").unwrap();
    assert!(
        core.settings
            .script_settings_inject
            .as_ref()
            .map(|rows| !rows.iter().any(|row| row.id == "bankStrategy"))
            .unwrap_or(true),
        "default chicken_killer banking stays off"
    );
    assert!(names().contains(&"chicken_killer_bank"));
}

#[test]
fn vial_filler_cases_register_fountain_fill_and_bank_cycles() {
    let west = get("vial_filler").expect("vial_filler");
    assert_eq!(west.settings.start_script, Some("VialFiller"));
    assert_eq!(west.settings.deadline, SCRIPT_GOLD_DEADLINE);
    let inject = settings_inject_map(west.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("bank"),
        Some(&Value::String("Falador West".into()))
    );
    assert_eq!(inject.get("buyVials"), Some(&Value::Bool(false)));
    let start = west
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert_eq!(west.steps[start - 1].wait.arm, Proof::BankClosed);
    let seed = west.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::ArrivedNear {
        x: 2946,
        z: 3368,
        level: 0,
        radius: 6,
    }));
    assert!(seed.contains(&Proof::BankItemId {
        id: EMPTY_VIAL_ID,
        count: VIAL_EMPTY_SEED,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: VIAL_OF_WATER_ID,
        count: 0,
    }));
    assert!(seed.contains(&Proof::BankItemIdAtMost {
        id: VIAL_OF_WATER_ID,
        count: 0,
    }));
    let watch = west.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    let fountain = Proof::ArrivedNear {
        x: 2949,
        z: 3381,
        level: 0,
        radius: 4,
    };
    let water = Proof::ItemId {
        id: VIAL_OF_WATER_ID,
        count: 1,
    };
    let pack_empty_water = Proof::ItemIdAtMost {
        id: VIAL_OF_WATER_ID,
        count: 0,
    };
    assert_eq!(
        watch,
        vec![
            fountain,
            water,
            Proof::ItemIdAtMost {
                id: EMPTY_VIAL_ID,
                count: 0,
            },
            Proof::BankItemId {
                id: VIAL_OF_WATER_ID,
                count: 1,
            },
            pack_empty_water,
            Proof::ItemId {
                id: EMPTY_VIAL_ID,
                count: 1,
            },
            Proof::BankClosed,
            fountain,
            water,
        ]
    );
    let first_water = watch.iter().position(|arm| *arm == water).unwrap();
    let bank_water = watch
        .iter()
        .position(|arm| {
            *arm == Proof::BankItemId {
                id: VIAL_OF_WATER_ID,
                count: 1,
            }
        })
        .unwrap();
    let pack_empty = watch
        .iter()
        .position(|arm| *arm == pack_empty_water)
        .unwrap();
    let further_water = watch.iter().rposition(|arm| *arm == water).unwrap();
    assert!(first_water < bank_water);
    assert!(bank_water < pack_empty);
    assert!(pack_empty < further_water);
    assert_ne!(first_water, further_water);
    assert_eq!(west.proof, water);

    let east = get("vial_filler_east").expect("vial_filler_east");
    let inject = settings_inject_map(east.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("bank"),
        Some(&Value::String("Falador East".into()))
    );
    assert_eq!(inject.get("buyVials"), Some(&Value::Bool(false)));
    let east_start = east
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let east_seed = east.steps[..east_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(east_seed.contains(&Proof::ArrivedNear {
        x: 3013,
        z: 3355,
        level: 0,
        radius: 6,
    }));
    let east_watch = east.steps[east_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert_eq!(east_watch, watch);

    for name in ["vial_filler", "vial_filler_east"] {
        assert!(names().contains(&name));
    }
}

#[test]
fn potion_maker_cases_register_staged_unf_finished_and_bank_cycles() {
    let custom = get("potion_maker").expect("potion_maker");
    assert_eq!(custom.settings.start_script, Some("PotionMaker"));
    assert_eq!(custom.settings.deadline, SCRIPT_GOLD_DEADLINE);
    let inject = settings_inject_map(custom.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("herb"), Some(&Value::String("Custom".into())));
    assert_eq!(
        inject.get("herbCustom"),
        Some(&Value::String("Guam leaf".into()))
    );
    assert_eq!(
        inject.get("secondary"),
        Some(&Value::String("Custom".into()))
    );
    assert_eq!(
        inject.get("secondaryCustom"),
        Some(&Value::String("Eye of newt".into()))
    );
    let start = custom
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert_eq!(custom.steps[start - 1].wait.arm, Proof::BankClosed);
    let seed = custom.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::QuestDone {
        name: "Druidic Ritual",
    }));
    assert!(seed.contains(&Proof::Stat {
        id: HERBLORE_STAT,
        min: GUAM_HERBLORE,
    }));
    assert!(seed.contains(&Proof::BankItemId {
        id: GUAM_LEAF_ID,
        count: POTION_BATCH_SEED,
    }));
    assert!(seed.contains(&Proof::BankItemId {
        id: VIAL_OF_WATER_ID,
        count: POTION_BATCH_SEED,
    }));
    assert!(seed.contains(&Proof::BankItemId {
        id: EYE_OF_NEWT_ID,
        count: POTION_BATCH_SEED,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: GUAM_UNF_ID,
        count: 0,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: ATTACK_POTION_3_ID,
        count: 0,
    }));
    let watch = custom.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert_eq!(
        watch[0],
        Proof::ItemId {
            id: GUAM_UNF_ID,
            count: 1,
        }
    );
    assert!(watch.contains(&Proof::ItemId {
        id: ATTACK_POTION_3_ID,
        count: 1,
    }));
    assert!(watch.contains(&Proof::ItemIdAtMost {
        id: GUAM_UNF_ID,
        count: 0,
    }));
    assert!(watch.contains(&Proof::StatXpGain {
        id: HERBLORE_STAT,
        min: 1,
    }));
    assert!(watch.contains(&Proof::BankItemId {
        id: ATTACK_POTION_3_ID,
        count: 1,
    }));
    assert!(watch.contains(&Proof::ItemId {
        id: VIAL_OF_WATER_ID,
        count: 1,
    }));
    assert!(watch.contains(&Proof::BankClosed));
    assert_eq!(
        custom.proof,
        Proof::ItemId {
            id: ATTACK_POTION_3_ID,
            count: 1,
        }
    );
    let unf_idx = watch
        .iter()
        .position(|arm| {
            *arm == Proof::ItemId {
                id: GUAM_UNF_ID,
                count: 1,
            }
        })
        .unwrap();
    let finished_idx = watch
        .iter()
        .position(|arm| {
            *arm == Proof::ItemId {
                id: ATTACK_POTION_3_ID,
                count: 1,
            }
        })
        .unwrap();
    let restock_empty = watch
        .iter()
        .position(|arm| {
            *arm == Proof::ItemIdAtMost {
                id: ATTACK_POTION_3_ID,
                count: 0,
            }
        })
        .unwrap();
    assert!(unf_idx < finished_idx);
    assert!(finished_idx < restock_empty);
    assert!(restock_empty < watch.len() - 1);

    let named = get("potion_maker_named").expect("potion_maker_named");
    let inject = settings_inject_map(named.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("herb"),
        Some(&Value::String("Ranarr weed".into()))
    );
    assert_eq!(
        inject.get("secondary"),
        Some(&Value::String("Snape grass".into()))
    );
    let named_start = named
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let named_seed = named.steps[..named_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(named_seed.contains(&Proof::Stat {
        id: HERBLORE_STAT,
        min: RANARR_HERBLORE,
    }));
    assert!(named_seed.contains(&Proof::BankItemId {
        id: RANARR_WEED_ID,
        count: POTION_BATCH_SEED,
    }));
    assert!(named_seed.contains(&Proof::BankItemId {
        id: SNAPE_GRASS_ID,
        count: POTION_BATCH_SEED,
    }));
    assert!(named_seed.contains(&Proof::BankItemId {
        id: GUAM_LEAF_ID,
        count: 14,
    }));
    let named_watch = named.steps[named_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(named_watch.contains(&Proof::ItemId {
        id: RANARR_UNF_ID,
        count: 1,
    }));
    assert!(named_watch.contains(&Proof::ItemId {
        id: PRAYER_POTION_3_ID,
        count: 1,
    }));
    assert!(named_watch.contains(&Proof::ItemIdAtMost {
        id: GUAM_UNF_ID,
        count: 0,
    }));
    assert!(named_watch.contains(&Proof::ItemIdAtMost {
        id: ATTACK_POTION_3_ID,
        count: 0,
    }));
    assert!(named_watch.contains(&Proof::BankItemId {
        id: GUAM_LEAF_ID,
        count: 14,
    }));

    for name in ["potion_maker", "potion_maker_named"] {
        assert!(names().contains(&name));
    }
}

#[test]
fn tanner_bot_cases_register_conversion_and_bank_cycles() {
    let soft = get("tanner_bot").expect("tanner_bot");
    assert_eq!(soft.settings.start_script, Some("TannerBot"));
    assert_eq!(soft.settings.deadline, SCRIPT_GOLD_DEADLINE);
    let inject = settings_inject_map(soft.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("hideType"),
        Some(&Value::String("Soft leather".into()))
    );
    assert_eq!(inject.get("buyThread"), Some(&Value::Bool(false)));
    let start = soft
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert_eq!(soft.steps[start - 1].wait.arm, Proof::BankClosed);
    let seed = soft.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::ArrivedNear {
        x: 3269,
        z: 3167,
        level: 0,
        radius: 6,
    }));
    assert!(seed.contains(&Proof::BankItemId {
        id: COW_HIDE_ID,
        count: TANNER_HIDE_SEED,
    }));
    assert!(seed.contains(&Proof::BankItemId {
        id: COINS_ID,
        count: TANNER_COIN_SEED,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: SOFT_LEATHER_ID,
        count: 0,
    }));
    assert!(seed.contains(&Proof::BankItemIdAtMost {
        id: SOFT_LEATHER_ID,
        count: 0,
    }));
    let watch = soft.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    let tanner = Proof::ArrivedNear {
        x: 3277,
        z: 3191,
        level: 0,
        radius: 4,
    };
    let leather = Proof::ItemId {
        id: SOFT_LEATHER_ID,
        count: 1,
    };
    let pack_empty_leather = Proof::ItemIdAtMost {
        id: SOFT_LEATHER_ID,
        count: 0,
    };
    assert_eq!(
        watch,
        vec![
            tanner,
            leather,
            Proof::ItemIdAtMost {
                id: COW_HIDE_ID,
                count: 0,
            },
            Proof::BankItemId {
                id: SOFT_LEATHER_ID,
                count: 1,
            },
            pack_empty_leather,
            Proof::ItemId {
                id: COW_HIDE_ID,
                count: 1,
            },
            Proof::BankClosed,
            tanner,
            leather,
        ]
    );
    let first_leather = watch.iter().position(|arm| *arm == leather).unwrap();
    let bank_leather = watch
        .iter()
        .position(|arm| {
            *arm == Proof::BankItemId {
                id: SOFT_LEATHER_ID,
                count: 1,
            }
        })
        .unwrap();
    let pack_empty = watch
        .iter()
        .position(|arm| *arm == pack_empty_leather)
        .unwrap();
    let further_leather = watch.iter().rposition(|arm| *arm == leather).unwrap();
    assert!(first_leather < bank_leather);
    assert!(bank_leather < pack_empty);
    assert!(pack_empty < further_leather);
    assert_ne!(first_leather, further_leather);
    assert_eq!(soft.proof, leather);

    let hard = get("tanner_bot_hard").expect("tanner_bot_hard");
    let inject = settings_inject_map(hard.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("hideType"),
        Some(&Value::String("Hard leather".into()))
    );
    assert_eq!(inject.get("buyThread"), Some(&Value::Bool(false)));
    let hard_start = hard
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let hard_watch = hard.steps[hard_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    let hard_leather = Proof::ItemId {
        id: HARD_LEATHER_ID,
        count: 1,
    };
    assert_eq!(hard_watch[1], hard_leather);
    assert!(hard_watch.contains(&Proof::BankItemId {
        id: HARD_LEATHER_ID,
        count: 1,
    }));
    assert!(hard_watch.contains(&Proof::ItemIdAtMost {
        id: HARD_LEATHER_ID,
        count: 0,
    }));
    assert!(!hard_watch.contains(&leather));
    assert_eq!(hard.proof, hard_leather);

    for name in ["tanner_bot", "tanner_bot_hard"] {
        assert!(names().contains(&name));
    }
}

#[test]
fn rune_crafter_cases_register_altar_conversion_and_bank_cycles() {
    let air = get("rune_crafter").expect("rune_crafter");
    assert_eq!(air.settings.start_script, Some("RuneCrafter"));
    assert_eq!(air.settings.deadline, SCRIPT_GOLD_DEADLINE);
    let inject = settings_inject_map(air.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("rune"), Some(&Value::String("Air runes".into())));
    assert_eq!(inject.get("mode"), Some(&Value::String("Solo".into())));
    let start = air
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert_eq!(air.steps[start - 1].wait.arm, Proof::BankClosed);
    let seed = air.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::ArrivedNear {
        x: 3013,
        z: 3355,
        level: 0,
        radius: 6,
    }));
    assert!(seed.contains(&Proof::Stat {
        id: RUNECRAFT_STAT,
        min: 1,
    }));
    assert!(seed.contains(&Proof::BankItemId {
        id: RUNE_ESSENCE_ID,
        count: RUNE_ESSENCE_SEED,
    }));
    assert!(seed.contains(&Proof::BankItemId {
        id: AIR_TALISMAN_ID,
        count: 1,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: AIR_RUNE_ID,
        count: 0,
    }));
    assert!(seed.contains(&Proof::BankItemIdAtMost {
        id: AIR_RUNE_ID,
        count: 0,
    }));
    assert!(seed.contains(&Proof::BankItemIdAtMost {
        id: NOTED_ESSENCE_ID,
        count: 0,
    }));
    let watch = air.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    let ruins = Proof::ArrivedNear {
        x: 2988,
        z: 3294,
        level: 0,
        radius: 4,
    };
    let crafted = Proof::ItemId {
        id: AIR_RUNE_ID,
        count: 1,
    };
    let pack_empty_runes = Proof::ItemIdAtMost {
        id: AIR_RUNE_ID,
        count: 0,
    };
    assert_eq!(
        watch,
        vec![
            Proof::ItemId {
                id: RUNE_ESSENCE_ID,
                count: 1,
            },
            ruins,
            Proof::StatXpGain {
                id: RUNECRAFT_STAT,
                min: 1,
            },
            crafted,
            Proof::ItemIdAtMost {
                id: RUNE_ESSENCE_ID,
                count: 0,
            },
            ruins,
            Proof::BankItemId {
                id: AIR_RUNE_ID,
                count: 1,
            },
            pack_empty_runes,
            Proof::ItemId {
                id: RUNE_ESSENCE_ID,
                count: 1,
            },
            Proof::BankClosed,
            ruins,
            crafted,
        ]
    );
    let first_xp = watch
        .iter()
        .position(|arm| {
            *arm == Proof::StatXpGain {
                id: RUNECRAFT_STAT,
                min: 1,
            }
        })
        .unwrap();
    let first_rune = watch.iter().position(|arm| *arm == crafted).unwrap();
    let bank_runes = watch
        .iter()
        .position(|arm| {
            *arm == Proof::BankItemId {
                id: AIR_RUNE_ID,
                count: 1,
            }
        })
        .unwrap();
    let pack_empty = watch
        .iter()
        .position(|arm| *arm == pack_empty_runes)
        .unwrap();
    let further_rune = watch.iter().rposition(|arm| *arm == crafted).unwrap();
    assert!(watch[1] == ruins);
    assert!(first_xp < first_rune);
    assert!(first_rune < bank_runes);
    assert!(bank_runes < pack_empty);
    assert!(pack_empty < further_rune);
    assert_ne!(first_rune, further_rune);
    assert_eq!(air.proof, crafted);

    let earth = get("rune_crafter_earth").expect("rune_crafter_earth");
    let inject = settings_inject_map(earth.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("rune"),
        Some(&Value::String("Earth runes".into()))
    );
    assert_eq!(inject.get("mode"), Some(&Value::String("Solo".into())));
    let earth_start = earth
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let earth_seed = earth.steps[..earth_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(earth_seed.contains(&Proof::ArrivedNear {
        x: 3253,
        z: 3420,
        level: 0,
        radius: 6,
    }));
    assert!(earth_seed.contains(&Proof::Stat {
        id: RUNECRAFT_STAT,
        min: 9,
    }));
    let earth_watch = earth.steps[earth_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    let earth_ruins = Proof::ArrivedNear {
        x: 3303,
        z: 3477,
        level: 0,
        radius: 4,
    };
    let earth_rune = Proof::ItemId {
        id: EARTH_RUNE_ID,
        count: 1,
    };
    assert_eq!(earth_watch[1], earth_ruins);
    assert_eq!(
        earth_watch[2],
        Proof::StatXpGain {
            id: RUNECRAFT_STAT,
            min: 1,
        }
    );
    assert_eq!(earth_watch[3], earth_rune);
    assert!(earth_watch.contains(&Proof::BankItemId {
        id: EARTH_RUNE_ID,
        count: 1,
    }));
    assert!(earth_watch.contains(&Proof::ItemIdAtMost {
        id: EARTH_RUNE_ID,
        count: 0,
    }));
    assert!(!earth_watch.contains(&crafted));
    assert_eq!(earth.proof, earth_rune);

    let mule = get("mule_crafter").expect("mule_crafter");
    assert_eq!(mule.settings.start_script, Some("MuleCrafter"));
    let inject = settings_inject_map(mule.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("rune"), Some(&Value::String("Air rune".into())));
    assert_eq!(inject.get("mode"), Some(&Value::String("Crafter".into())));
    assert_eq!(inject.get("partner"), Some(&Value::String("".into())));
    assert_eq!(inject.get("bankFill"), Some(&Value::Bool(true)));
    let mule_start = mule
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let mule_watch = mule.steps[mule_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    let mule_ruins = Proof::ArrivedNear {
        x: 2983,
        z: 3288,
        level: 0,
        radius: 4,
    };
    assert_eq!(mule_watch[1], mule_ruins);
    assert_eq!(
        mule_watch[2],
        Proof::StatXpGain {
            id: RUNECRAFT_STAT,
            min: 1,
        }
    );
    assert_eq!(mule_watch[3], crafted);
    assert_ne!(mule_watch[1], ruins);
    assert_eq!(mule.proof, crafted);

    for name in ["rune_crafter", "rune_crafter_earth", "mule_crafter"] {
        assert!(names().contains(&name));
    }
}

#[test]
fn ardy_thieving_cases_register_stall_guard_knight_and_bank_cycles() {
    let cakes = get("ardy_cakes").expect("ardy_cakes");
    assert_eq!(cakes.settings.start_script, Some("ArdyCakes"));
    assert_eq!(cakes.settings.deadline, ARDY_CAKES_DEADLINE);
    let inject = settings_inject_map(cakes.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("guardResponse"),
        Some(&Value::String("Flee".into()))
    );
    assert_eq!(inject.get("solveClues"), Some(&Value::Bool(false)));
    assert!(inject.get("bankStrategy").is_none());
    let start = cakes
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let seed = cakes.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::ArrivedNear {
        x: 2668,
        z: 3312,
        level: 0,
        radius: 6,
    }));
    assert!(seed.contains(&Proof::Stat {
        id: THIEVING_STAT,
        min: 5,
    }));
    assert!(seed.contains(&Proof::Stat {
        id: HITPOINTS_STAT,
        min: 40,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: CAKE_ID,
        count: 0,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: BREAD_ID,
        count: 0,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: CHOCOLATE_SLICE_ID,
        count: 0,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: CHOCOLATE_CAKE_ID,
        count: 0,
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: KNIFE_ID,
        count: 22,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: KNIFE_ID,
        count: 22,
    }));
    let watch = cakes.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    let cake = Proof::ItemId {
        id: CAKE_ID,
        count: 1,
    };
    let pack_empty_cake = Proof::ItemIdAtMost {
        id: CAKE_ID,
        count: 0,
    };
    let stand = Proof::ArrivedNear {
        x: 2668,
        z: 3312,
        level: 0,
        radius: 6,
    };
    let bank = Proof::ArrivedNear {
        x: ARDY_BANK.x,
        z: ARDY_BANK.z,
        level: ARDY_BANK.level,
        radius: 6,
    };
    assert_eq!(
        watch,
        vec![
            Proof::StatXpGain {
                id: THIEVING_STAT,
                min: 1
            },
            cake,
            bank,
            Proof::BankItemId {
                id: CAKE_ID,
                count: 1
            },
            pack_empty_cake,
            stand,
            Proof::BankClosed,
            cake,
        ]
    );
    let first_cake = watch.iter().position(|arm| *arm == cake).unwrap();
    let bank_cake = watch
        .iter()
        .position(|arm| {
            *arm == Proof::BankItemId {
                id: CAKE_ID,
                count: 1,
            }
        })
        .unwrap();
    let pack_empty = watch
        .iter()
        .position(|arm| *arm == pack_empty_cake)
        .unwrap();
    let further_cake = watch.iter().rposition(|arm| *arm == cake).unwrap();
    assert!(first_cake < bank_cake);
    assert!(bank_cake < pack_empty);
    assert!(pack_empty < further_cake);
    assert_ne!(first_cake, further_cake);
    assert_eq!(cakes.proof, cake);
    assert_eq!(
        ARDY_BANK,
        WorldTile {
            x: 2655,
            z: 3286,
            level: 0
        }
    );

    let guard = get("ardy_thiever").expect("ardy_thiever");
    assert_eq!(guard.settings.start_script, Some("ArdyThiever"));
    let inject = settings_inject_map(guard.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("thieveTarget"),
        Some(&Value::String("Guard".into()))
    );
    assert_eq!(
        inject.get("guardResponse"),
        Some(&Value::String("Flee".into()))
    );
    assert_eq!(inject.get("solveClues"), Some(&Value::Bool(false)));
    assert_eq!(inject.get("bankAtLootSlots"), Some(&Value::from(1.0)));
    assert_eq!(inject.get("foodTarget"), Some(&Value::from(1.0)));
    assert_eq!(inject.get("restockAtFood"), Some(&Value::from(0.0)));
    assert!(inject.get("bankStrategy").is_none());
    let guard_start = guard
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let guard_seed = guard.steps[..guard_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(guard_seed.contains(&Proof::ArrivedNear {
        x: 2661,
        z: 3306,
        level: 0,
        radius: 8,
    }));
    assert!(guard_seed.contains(&Proof::Stat {
        id: THIEVING_STAT,
        min: 40,
    }));
    assert!(guard_seed.contains(&Proof::ItemIdAtMost {
        id: COINS_ID,
        count: 0,
    }));
    let guard_watch = guard.steps[guard_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    let coins = Proof::ItemId {
        id: COINS_ID,
        count: 1,
    };
    let pack_empty_coins = Proof::ItemIdAtMost {
        id: COINS_ID,
        count: 0,
    };
    let market = Proof::ArrivedNear {
        x: 2661,
        z: 3306,
        level: 0,
        radius: 6,
    };
    assert_eq!(
        guard_watch,
        vec![
            Proof::StatXpGain {
                id: THIEVING_STAT,
                min: 1
            },
            coins,
            Proof::BankItemId {
                id: COINS_ID,
                count: 1
            },
            pack_empty_coins,
            market,
            Proof::BankClosed,
            coins,
        ]
    );
    let first_coins = guard_watch.iter().position(|arm| *arm == coins).unwrap();
    let further_coins = guard_watch.iter().rposition(|arm| *arm == coins).unwrap();
    assert_ne!(first_coins, further_coins);
    assert_eq!(guard.proof, coins);

    let knight = get("ardy_thiever_knight").expect("ardy_thiever_knight");
    assert_eq!(knight.settings.start_script, Some("ArdyThiever"));
    let inject = settings_inject_map(knight.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("thieveTarget"),
        Some(&Value::String("Knight of Ardougne".into()))
    );
    assert_eq!(inject.get("bankAtLootSlots"), Some(&Value::from(1.0)));
    let knight_start = knight
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let knight_seed = knight.steps[..knight_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(knight_seed.contains(&Proof::Stat {
        id: THIEVING_STAT,
        min: 55,
    }));
    assert!(!knight_seed.contains(&Proof::Stat {
        id: THIEVING_STAT,
        min: 40,
    }));
    let knight_watch = knight.steps[knight_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert_eq!(knight_watch, guard_watch);
    assert_eq!(knight.proof, coins);

    for name in ["ardy_cakes", "ardy_thiever", "ardy_thiever_knight"] {
        assert!(names().contains(&name));
    }
}

#[test]
fn alternate_camp_and_fight_option_cases_register() {
    let strength = Proof::StatXpGain {
        id: STRENGTH_STAT,
        min: 1,
    };
    let cakes_fight = get("ardy_cakes_fight").expect("ardy_cakes_fight");
    assert_eq!(cakes_fight.settings.start_script, Some("ArdyCakes"));
    assert_eq!(cakes_fight.settings.deadline, SCRIPT_GOLD_DEADLINE);
    let inject = settings_inject_map(cakes_fight.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("guardResponse"),
        Some(&Value::String("Fight".into()))
    );
    assert_eq!(inject.get("solveClues"), Some(&Value::Bool(false)));
    let start = cakes_fight
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let seed = cakes_fight.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::ArrivedNear {
        x: 2668,
        z: 3312,
        level: 0,
        radius: 6,
    }));
    assert!(seed.contains(&Proof::Stat {
        id: THIEVING_STAT,
        min: 5,
    }));
    assert!(seed.contains(&Proof::Stat {
        id: 0,
        min: COMBAT_ATTACK_LEVEL,
    }));
    assert!(seed.contains(&Proof::Stat {
        id: STRENGTH_STAT,
        min: COMBAT_ATTACK_LEVEL,
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: KNIFE_ID,
        count: 22,
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: COMBAT_SCIMITAR_ID,
        count: 1,
    }));
    assert!(seed.contains(&Proof::EquipmentId {
        id: COMBAT_SCIMITAR_ID,
    }));
    let watch = cakes_fight.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert_eq!(
        watch,
        vec![
            Proof::StatXpGain {
                id: THIEVING_STAT,
                min: 1
            },
            Proof::ItemId {
                id: CAKE_ID,
                count: 1
            },
            strength,
        ]
    );
    assert_eq!(cakes_fight.proof, strength);
    assert!(!watch
        .iter()
        .any(|arm| matches!(arm, Proof::BankItemId { .. })));

    let thiever_fight = get("ardy_thiever_fight").expect("ardy_thiever_fight");
    assert_eq!(thiever_fight.settings.start_script, Some("ArdyThiever"));
    let inject = settings_inject_map(thiever_fight.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("guardResponse"),
        Some(&Value::String("Fight".into()))
    );
    assert_eq!(
        inject.get("thieveTarget"),
        Some(&Value::String("Guard".into()))
    );
    // One full opening stall session: the 289 Guard catch comes from it.
    assert_eq!(inject.get("foodTarget"), Some(&Value::from(27.0)));
    assert_eq!(inject.get("restockAtFood"), Some(&Value::from(0.0)));
    assert_eq!(inject.get("bankAtLootSlots"), Some(&Value::from(1.0)));
    let start = thiever_fight
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let seed = thiever_fight.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::Stat {
        id: THIEVING_STAT,
        min: 40,
    }));
    assert!(seed.contains(&Proof::Stat {
        id: 0,
        min: COMBAT_ATTACK_LEVEL,
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: COMBAT_SCIMITAR_ID,
        count: 1,
    }));
    assert!(seed.contains(&Proof::EquipmentId {
        id: COMBAT_SCIMITAR_ID,
    }));
    let watch = thiever_fight.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    let coins = Proof::ItemId {
        id: COINS_ID,
        count: 1,
    };
    assert_eq!(
        watch,
        vec![
            strength,
            Proof::StatXpGain {
                id: THIEVING_STAT,
                min: 1
            },
            coins,
            Proof::BankItemId {
                id: COINS_ID,
                count: 1
            },
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0
            },
            Proof::ArrivedNear {
                x: 2661,
                z: 3306,
                level: 0,
                radius: 6,
            },
            Proof::BankClosed,
            coins,
        ]
    );
    assert_eq!(thiever_fight.proof, coins);

    let tower = get("chaos_druid_tower").expect("chaos_druid_tower");
    assert_eq!(tower.settings.start_script, Some("ChaosDruidKiller"));
    let inject = settings_inject_map(tower.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("location"),
        Some(&Value::String("Chaos Druid Tower".into()))
    );
    assert_eq!(
        inject.get("combatStyleIndex"),
        Some(&Value::String("1".into()))
    );
    let start = tower
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let seed = tower.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::ArrivedNear {
        x: 2562,
        z: 3356,
        level: 0,
        radius: 4,
    }));
    assert!(seed.contains(&Proof::Stat {
        id: THIEVING_STAT,
        min: 46,
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: LOBSTER_ID,
        count: CHAOS_DRUID_FOOD,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: UNIDENTIFIED_GUAM_ID,
        count: 0,
    }));
    assert_eq!(tower.steps[start + 1].wait.arm, strength);
    assert_eq!(tower.proof, strength);

    let yanille = get("chaos_druid_yanille").expect("chaos_druid_yanille");
    assert_eq!(yanille.settings.start_script, Some("ChaosDruidKiller"));
    let inject = settings_inject_map(yanille.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("location"),
        Some(&Value::String("Yanille Dungeon".into()))
    );
    let start = yanille
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let seed = yanille.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::ArrivedNear {
        x: 2580,
        z: 9501,
        level: 0,
        radius: 8,
    }));
    assert!(seed.contains(&Proof::Stat {
        id: AGILITY_STAT,
        min: 40,
    }));
    assert!(!seed.contains(&Proof::Stat {
        id: THIEVING_STAT,
        min: 46,
    }));
    assert_eq!(yanille.steps[start + 1].wait.arm, strength);
    assert_eq!(yanille.proof, strength);

    for name in [
        "ardy_cakes_fight",
        "chaos_druid_tower",
        "chaos_druid_yanille",
    ] {
        assert!(names().contains(&name));
        let scenario = get(name).unwrap();
        assert_eq!(scenario.settings.deadline, SCRIPT_GOLD_DEADLINE);
    }
    // The opening stall session and Guard fight come before the gold chain.
    assert!(names().contains(&"ardy_thiever_fight"));
    assert_eq!(thiever_fight.settings.deadline, ARDY_THIEVER_FIGHT_DEADLINE);
}

#[test]
fn resource_world_cases_register_gnome_log_bank_fletch_and_coal_truck() {
    let chop = get("gnome_chop").expect("gnome_chop");
    assert_eq!(chop.settings.start_script, Some("GnomeMagicChopper"));
    assert_eq!(chop.settings.deadline, SCRIPT_GOLD_DEADLINE);
    let inject = settings_inject_map(chop.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("fletchLogs"), Some(&Value::Bool(false)));
    let start = chop
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let seed = chop.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::ArrivedNear {
        x: 2433,
        z: 3409,
        level: 0,
        radius: 1,
    }));
    assert!(seed.contains(&Proof::LocActionNear {
        id: 1306,
        x: 2432,
        z: 3410,
        level: 0,
        radius: 0,
        action: "Chop down",
        present: true,
    }));
    assert!(seed.contains(&Proof::Stat {
        id: WOODCUTTING_STAT,
        min: 75,
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: RUNE_AXE_ID,
        count: 1,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: RUNE_AXE_ID,
        count: 1,
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: KNIFE_ID,
        count: 26,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: KNIFE_ID,
        count: 26,
    }));
    assert!(!seed.contains(&Proof::ItemId { id: 1353, count: 1 }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: MAGIC_LOGS_ID,
        count: 0,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: UNSTRUNG_MAGIC_SHORTBOW_ID,
        count: 0,
    }));
    let watch = chop.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    let logs = Proof::ItemId {
        id: MAGIC_LOGS_ID,
        count: 1,
    };
    let pack_empty_logs = Proof::ItemIdAtMost {
        id: MAGIC_LOGS_ID,
        count: 0,
    };
    let upstairs = Proof::ArrivedNear {
        x: GNOME_BANK_STAND.x,
        z: GNOME_BANK_STAND.z,
        level: GNOME_BANK_STAND.level,
        radius: 8,
    };
    let ground = Proof::ArrivedNear {
        x: GNOME_BANK_STAIR_SOUTH.x,
        z: GNOME_BANK_STAIR_SOUTH.z,
        level: GNOME_BANK_STAIR_SOUTH.level,
        radius: 30,
    };
    assert_eq!(
        watch,
        vec![
            Proof::StatXpGain {
                id: WOODCUTTING_STAT,
                min: 1
            },
            logs,
            upstairs,
            Proof::BankItemId {
                id: MAGIC_LOGS_ID,
                count: 1
            },
            pack_empty_logs,
            ground,
            Proof::BankClosed,
            logs,
        ]
    );
    let first_logs = watch.iter().position(|arm| *arm == logs).unwrap();
    let bank_logs = watch
        .iter()
        .position(|arm| {
            *arm == Proof::BankItemId {
                id: MAGIC_LOGS_ID,
                count: 1,
            }
        })
        .unwrap();
    let pack_empty = watch
        .iter()
        .position(|arm| *arm == pack_empty_logs)
        .unwrap();
    let further_logs = watch.iter().rposition(|arm| *arm == logs).unwrap();
    assert!(first_logs < bank_logs);
    assert!(bank_logs < pack_empty);
    assert!(pack_empty < further_logs);
    assert_ne!(first_logs, further_logs);
    assert_eq!(chop.proof, logs);
    assert_eq!(GNOME_BANK_STAND.level, 1);
    assert_eq!(GNOME_BANK_STAIR_SOUTH.level, 0);

    let short = get("gnome_fletch_short").expect("gnome_fletch_short");
    assert_eq!(short.settings.start_script, Some("GnomeMagicChopper"));
    let inject = settings_inject_map(short.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("fletchLogs"), Some(&Value::Bool(true)));
    let short_start = short
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let short_seed = short.steps[..short_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(short_seed.contains(&Proof::Stat {
        id: FLETCHING_STAT,
        min: 80,
    }));
    assert!(short_seed.contains(&Proof::StatAtMost {
        id: FLETCHING_STAT,
        max: 84,
    }));
    assert!(short_seed.contains(&Proof::ItemId {
        id: RUNE_AXE_ID,
        count: 1,
    }));
    assert!(short_seed.contains(&Proof::ItemIdAtMost {
        id: RUNE_AXE_ID,
        count: 1,
    }));
    assert!(short_seed.contains(&Proof::ItemId {
        id: KNIFE_ID,
        count: GNOME_BALLAST_KNIVES,
    }));
    assert!(short_seed.contains(&Proof::ItemIdAtMost {
        id: KNIFE_ID,
        count: GNOME_BALLAST_KNIVES,
    }));
    assert!(!short_seed.contains(&Proof::Stat {
        id: FLETCHING_STAT,
        min: 85,
    }));
    let short_watch = short.steps[short_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    let unstrung_short = Proof::ItemId {
        id: UNSTRUNG_MAGIC_SHORTBOW_ID,
        count: 1,
    };
    assert_eq!(
        short_watch[0],
        Proof::StatXpGain {
            id: WOODCUTTING_STAT,
            min: 1
        }
    );
    assert_eq!(short_watch[1], logs);
    assert_eq!(
        short_watch[2],
        Proof::StatXpGain {
            id: FLETCHING_STAT,
            min: 1
        }
    );
    assert_eq!(short_watch[3], unstrung_short);
    assert!(short_watch.contains(&Proof::BankItemId {
        id: UNSTRUNG_MAGIC_SHORTBOW_ID,
        count: 1,
    }));
    assert!(!short_watch.contains(&Proof::BankItemId {
        id: UNSTRUNG_MAGIC_LONGBOW_ID,
        count: 1,
    }));
    assert!(!short_watch.contains(&Proof::ItemId {
        id: MAGIC_SHORTBOW_ID,
        count: 1,
    }));
    let first_product = short_watch
        .iter()
        .position(|arm| *arm == unstrung_short)
        .unwrap();
    let further_chop = short_watch.iter().rposition(|arm| *arm == logs).unwrap();
    assert!(first_product < further_chop);
    assert_eq!(short.proof, logs);

    let long = get("gnome_fletch_long").expect("gnome_fletch_long");
    let inject = settings_inject_map(long.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("fletchLogs"), Some(&Value::Bool(true)));
    let long_start = long
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let long_seed = long.steps[..long_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(long_seed.contains(&Proof::Stat {
        id: FLETCHING_STAT,
        min: 85,
    }));
    assert!(!long_seed.contains(&Proof::StatAtMost {
        id: FLETCHING_STAT,
        max: 84,
    }));
    let long_watch = long.steps[long_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(long_watch.contains(&Proof::ItemId {
        id: UNSTRUNG_MAGIC_LONGBOW_ID,
        count: 1,
    }));
    assert!(long_watch.contains(&Proof::BankItemId {
        id: UNSTRUNG_MAGIC_LONGBOW_ID,
        count: 1,
    }));
    assert!(!long_watch.contains(&unstrung_short));
    assert_eq!(long.proof, logs);

    let coal = get("coal_trucks").expect("coal_trucks");
    assert_eq!(coal.settings.start_script, Some("CoalTrucks"));
    assert_eq!(coal.settings.deadline, SCRIPT_GOLD_DEADLINE);
    assert!(coal.settings.script_settings_inject.is_none());
    let coal_start = coal
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let coal_seed = coal.steps[..coal_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(coal_seed.contains(&Proof::ArrivedNear {
        x: 2582,
        z: 3481,
        level: 0,
        radius: 8,
    }));
    assert!(coal_seed.contains(&Proof::Stat {
        id: MINING_STAT,
        min: 60,
    }));
    assert!(coal_seed.contains(&Proof::ItemId { id: 1275, count: 1 }));
    assert!(coal_seed.contains(&Proof::ItemId {
        id: KNIFE_ID,
        count: COAL_BALLAST_KNIVES,
    }));
    assert!(coal_seed.contains(&Proof::ItemIdAtMost {
        id: KNIFE_ID,
        count: COAL_BALLAST_KNIVES,
    }));
    assert!(coal_seed.contains(&Proof::ItemIdAtMost {
        id: COAL_ID,
        count: 0,
    }));
    let coal_watch = coal.steps[coal_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    let coal_item = Proof::ItemId {
        id: COAL_ID,
        count: 1,
    };
    let truck = Proof::ArrivedNear {
        x: COAL_MINE_TRUCK_STAND.x,
        z: COAL_MINE_TRUCK_STAND.z,
        level: COAL_MINE_TRUCK_STAND.level,
        radius: 4,
    };
    assert_eq!(
        coal_watch,
        vec![
            Proof::StatXpGain {
                id: MINING_STAT,
                min: 1
            },
            coal_item,
            truck,
            Proof::ItemIdAtMost {
                id: COAL_ID,
                count: 0
            },
            coal_item,
        ]
    );
    assert!(!coal_watch.contains(&Proof::BankItemId {
        id: COAL_ID,
        count: 1,
    }));
    let first_coal = coal_watch.iter().position(|arm| *arm == coal_item).unwrap();
    let truck_i = coal_watch.iter().position(|arm| *arm == truck).unwrap();
    let further_coal = coal_watch
        .iter()
        .rposition(|arm| *arm == coal_item)
        .unwrap();
    assert!(first_coal < truck_i);
    assert!(truck_i < further_coal);
    assert_eq!(coal.proof, coal_item);

    for name in [
        "gnome_chop",
        "gnome_fletch_short",
        "gnome_fletch_long",
        "coal_trucks",
    ] {
        assert!(names().contains(&name));
    }
}

#[test]
fn gnome_fletch_magic_chop_watches_cover_the_roll_tail() {
    // selected289 woodcut.rs2: Rune axe on a Magic tree is successchance
    // 7,21; stat_random at WC 75 gives 17/256. One roll per 4 engine ticks,
    // first at +3. The first arm also carries the card's gear bank trip.
    let p = 17.0_f64 / 256.0;
    let dirties_per_engine_tick = 1.25;
    let gear_trip_ticks = 50.0;
    let chop_ticks =
        f64::from(MAGIC_TREE_CHOP_WATCH_TICKS) / dirties_per_engine_tick - gear_trip_ticks - 3.0;
    let rolls = (chop_ticks / 4.0).floor() + 1.0;
    let miss = (1.0 - p).powf(rolls);
    assert!(
        miss <= 0.01,
        "first Magic log must land inside the watch for >=99% of correct runs, miss={miss}"
    );

    for name in ["gnome_fletch_short", "gnome_fletch_long"] {
        let s = get(name).unwrap_or_else(|| panic!("{name} registered"));
        assert_eq!(s.settings.deadline, MAGIC_TREE_CHOP_DEADLINE, "{name}");
        let start = s
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let watch = &s.steps[start + 1..];
        let logs = Proof::ItemId {
            id: MAGIC_LOGS_ID,
            count: 1,
        };
        let first_wc = &watch[0];
        assert_eq!(
            first_wc.wait.arm,
            Proof::StatXpGain {
                id: WOODCUTTING_STAT,
                min: 1
            },
            "{name}"
        );
        let further = watch
            .iter()
            .rposition(|step| step.wait.arm == logs)
            .unwrap();
        assert_eq!(further, watch.len() - 1, "{name}: further chop is last");
        for (i, step) in watch.iter().enumerate() {
            let expected = if i == 0 || i == further {
                MAGIC_TREE_CHOP_WATCH_TICKS
            } else {
                SCRIPT_GOLD_WATCH_TICKS
            };
            assert_eq!(
                step.wait.budget_ticks, expected,
                "{name}: {} budget; only Magic roll arms are widened",
                step.name
            );
        }
    }
}

#[test]
fn station_production_cases_register_cook_smelt_and_spin_cycles() {
    let cook = get("cook_bot").expect("cook_bot");
    assert_eq!(cook.settings.start_script, Some("CookBot"));
    assert_eq!(cook.settings.deadline, SCRIPT_GOLD_DEADLINE);
    let inject = settings_inject_map(cook.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("fish"),
        Some(&Value::String("Raw salmon".into()))
    );
    assert_eq!(
        inject.get("location"),
        Some(&Value::String("Catherby".into()))
    );
    assert_eq!(inject.get("surface"), Some(&Value::String("Range".into())));
    let start = cook
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let seed = cook.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::ArrivedNear {
        x: 2809,
        z: 3441,
        level: 0,
        radius: 6,
    }));
    assert!(seed.contains(&Proof::Stat {
        id: COOKING_STAT,
        min: COOKING_FIXTURE_LEVEL,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: RAW_SALMON_ID,
        count: 0,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: SALMON_ID,
        count: 0,
    }));
    assert!(seed.contains(&Proof::BankItemId {
        id: RAW_SALMON_ID,
        count: COOK_RAW_SEED,
    }));
    let watch = cook.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    let salmon = Proof::ItemId {
        id: SALMON_ID,
        count: 1,
    };
    let raw = Proof::ItemId {
        id: RAW_SALMON_ID,
        count: 1,
    };
    let range = Proof::ArrivedNear {
        x: CATHERBY_RANGE_STAND.x,
        z: CATHERBY_RANGE_STAND.z,
        level: CATHERBY_RANGE_STAND.level,
        radius: 8,
    };
    let cooking_xp = Proof::StatXpGain {
        id: COOKING_STAT,
        min: 1,
    };
    assert_eq!(watch[0], range);
    assert_eq!(watch[1], cooking_xp);
    assert_eq!(watch[2], salmon);
    let first_xp = watch.iter().position(|arm| *arm == cooking_xp).unwrap();
    let first_product = watch.iter().position(|arm| *arm == salmon).unwrap();
    let bank_product = watch
        .iter()
        .position(|arm| {
            *arm == Proof::BankItemId {
                id: SALMON_ID,
                count: 1,
            }
        })
        .unwrap();
    let restock = watch.iter().position(|arm| *arm == raw).unwrap();
    let further = watch.iter().rposition(|arm| *arm == salmon).unwrap();
    assert!(first_xp < first_product);
    assert!(first_product < bank_product);
    assert!(bank_product < restock);
    assert!(restock < further);
    assert_ne!(first_product, further);
    assert_eq!(cook.proof, salmon);

    let lobster = get("cook_bot_lobster").expect("cook_bot_lobster");
    assert_eq!(lobster.settings.start_script, Some("CookBot"));
    let inject = settings_inject_map(lobster.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("fish"),
        Some(&Value::String("Raw lobster".into()))
    );
    assert_eq!(inject.get("surface"), Some(&Value::String("Range".into())));
    let lobster_start = lobster
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let lobster_watch = lobster.steps[lobster_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(lobster_watch.contains(&Proof::ItemId {
        id: LOBSTER_ID,
        count: 1,
    }));
    assert!(lobster_watch.contains(&Proof::BankItemId {
        id: LOBSTER_ID,
        count: 1,
    }));
    assert!(!lobster_watch.contains(&salmon));
    assert_eq!(
        lobster.proof,
        Proof::ItemId {
            id: LOBSTER_ID,
            count: 1,
        }
    );

    let bronze = get("smelter_bot").expect("smelter_bot");
    assert_eq!(bronze.settings.start_script, Some("SmelterBot"));
    let inject = settings_inject_map(bronze.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("bar"), Some(&Value::String("Bronze".into())));
    let bronze_start = bronze
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let bronze_seed = bronze.steps[..bronze_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(bronze_seed.contains(&Proof::Stat {
        id: SMITHING_STAT,
        min: 1,
    }));
    assert!(bronze_seed.contains(&Proof::BankItemId {
        id: COPPER_ORE_ID,
        count: SMELT_ORE_SEED,
    }));
    assert!(bronze_seed.contains(&Proof::BankItemId {
        id: TIN_ORE_ID,
        count: SMELT_ORE_SEED,
    }));
    let bronze_watch = bronze.steps[bronze_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    let bar = Proof::ItemId {
        id: BRONZE_BAR_ID,
        count: 1,
    };
    let smith_xp = Proof::StatXpGain {
        id: SMITHING_STAT,
        min: 1,
    };
    let first_xp = bronze_watch
        .iter()
        .position(|arm| *arm == smith_xp)
        .unwrap();
    let first_bar = bronze_watch.iter().position(|arm| *arm == bar).unwrap();
    assert!(first_xp < first_bar);
    assert!(bronze_watch.contains(&Proof::ArrivedNear {
        x: AL_KHARID_FURNACE.x,
        z: AL_KHARID_FURNACE.z,
        level: AL_KHARID_FURNACE.level,
        radius: 8,
    }));
    assert_eq!(bronze.proof, bar);

    let steel = get("smelter_bot_steel").expect("smelter_bot_steel");
    let inject = settings_inject_map(steel.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("bar"), Some(&Value::String("Steel".into())));
    let steel_start = steel
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let steel_seed = steel.steps[..steel_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(steel_seed.contains(&Proof::Stat {
        id: SMITHING_STAT,
        min: 30,
    }));
    assert!(steel_seed.contains(&Proof::BankItemId {
        id: IRON_ORE_ID,
        count: SMELT_ORE_SEED,
    }));
    assert!(steel_seed.contains(&Proof::BankItemId {
        id: COAL_ID,
        count: STEEL_COAL_SEED,
    }));
    let steel_watch = steel.steps[steel_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(steel_watch.contains(&Proof::ItemId {
        id: STEEL_BAR_ID,
        count: 1,
    }));
    assert!(!steel_watch.contains(&bar));
    assert_eq!(
        steel.proof,
        Proof::ItemId {
            id: STEEL_BAR_ID,
            count: 1,
        }
    );

    let spin = get("flax_spinner").expect("flax_spinner");
    assert_eq!(spin.settings.start_script, Some("FlaxSpinner"));
    assert!(spin.settings.script_settings_inject.is_none());
    let spin_start = spin
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let spin_seed = spin.steps[..spin_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(spin_seed.contains(&Proof::ArrivedNear {
        x: 2722,
        z: 3493,
        level: 0,
        radius: 8,
    }));
    assert!(spin_seed.contains(&Proof::Stat {
        id: CRAFTING_STAT,
        min: 10,
    }));
    assert!(spin_seed.contains(&Proof::BankItemId {
        id: FLAX_ID,
        count: FLAX_SPIN_SEED,
    }));
    let spin_watch = spin.steps[spin_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    let string = Proof::ItemId {
        id: BOW_STRING_ID,
        count: 1,
    };
    let craft_xp = Proof::StatXpGain {
        id: CRAFTING_STAT,
        min: 1,
    };
    let wheel = Proof::ArrivedNear {
        x: FLAX_SPINNER_WHEEL.x,
        z: FLAX_SPINNER_WHEEL.z,
        level: FLAX_SPINNER_WHEEL.level,
        radius: 8,
    };
    assert_eq!(spin_watch[0], wheel);
    assert_eq!(spin_watch[1], craft_xp);
    assert_eq!(spin_watch[2], string);
    assert_eq!(FLAX_SPINNER_WHEEL.level, 1);
    assert_eq!(spin.proof, string);

    for name in [
        "cook_bot",
        "cook_bot_lobster",
        "smelter_bot",
        "smelter_bot_steel",
        "flax_spinner",
    ] {
        assert!(names().contains(&name));
    }
}

#[test]
fn flax_aio_and_secondary_cases_register_collection_cycles() {
    let aio = get("flax_aio").expect("flax_aio");
    assert_eq!(aio.settings.start_script, Some("FlaxAIO"));
    assert_eq!(aio.settings.deadline, SCRIPT_GOLD_DEADLINE);
    let inject = settings_inject_map(aio.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("picking"), Some(&Value::Bool(true)));
    assert_eq!(inject.get("spinning"), Some(&Value::Bool(true)));
    let start = aio
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let seed = aio.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::ArrivedNear {
        x: 2741,
        z: 3444,
        level: 0,
        radius: 6,
    }));
    assert!(seed.contains(&Proof::Stat {
        id: CRAFTING_STAT,
        min: 10,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: FLAX_ID,
        count: 0,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: BOW_STRING_ID,
        count: 0,
    }));
    let watch = aio.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    let flax = Proof::ItemId {
        id: FLAX_ID,
        count: 1,
    };
    let string = Proof::ItemId {
        id: BOW_STRING_ID,
        count: 1,
    };
    let craft_xp = Proof::StatXpGain {
        id: CRAFTING_STAT,
        min: 1,
    };
    let wheel = Proof::ArrivedNear {
        x: FLAX_SPINNER_WHEEL.x,
        z: FLAX_SPINNER_WHEEL.z,
        level: FLAX_SPINNER_WHEEL.level,
        radius: 8,
    };
    let first_flax = watch.iter().position(|arm| *arm == flax).unwrap();
    let first_wheel = watch.iter().position(|arm| *arm == wheel).unwrap();
    let first_xp = watch.iter().position(|arm| *arm == craft_xp).unwrap();
    let first_string = watch.iter().position(|arm| *arm == string).unwrap();
    let bank_string = watch
        .iter()
        .position(|arm| {
            *arm == Proof::BankItemId {
                id: BOW_STRING_ID,
                count: 1,
            }
        })
        .unwrap();
    let further = watch.iter().rposition(|arm| *arm == flax).unwrap();
    assert!(first_flax < first_wheel);
    assert!(first_wheel < first_xp);
    assert!(first_xp < first_string);
    assert!(first_string < bank_string);
    assert!(bank_string < further);
    assert_ne!(first_flax, further);
    assert_eq!(aio.proof, flax);
    assert!(!watch.contains(&Proof::ItemId {
        id: BALL_OF_WOOL_ID,
        count: 1,
    }));

    let pick = get("flax_aio_pick").expect("flax_aio_pick");
    assert_eq!(pick.settings.start_script, Some("FlaxAIO"));
    let inject = settings_inject_map(pick.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("picking"), Some(&Value::Bool(true)));
    assert_eq!(inject.get("spinning"), Some(&Value::Bool(false)));
    let pick_start = pick
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let pick_watch = pick.steps[pick_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(pick_watch.contains(&Proof::ItemId {
        id: FLAX_ID,
        count: 28,
    }));
    assert!(pick_watch.contains(&Proof::BankItemId {
        id: FLAX_ID,
        count: 28,
    }));
    assert!(!pick_watch.contains(&string));
    assert!(!pick_watch.contains(&craft_xp));
    assert_eq!(
        pick.proof,
        Proof::ItemId {
            id: FLAX_ID,
            count: 1,
        }
    );

    let spin = get("flax_aio_spin").expect("flax_aio_spin");
    assert_eq!(spin.settings.start_script, Some("FlaxAIO"));
    let inject = settings_inject_map(spin.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("picking"), Some(&Value::Bool(false)));
    assert_eq!(inject.get("spinning"), Some(&Value::Bool(true)));
    let spin_start = spin
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let spin_seed = spin.steps[..spin_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(spin_seed.contains(&Proof::ArrivedNear {
        x: 2725,
        z: 3493,
        level: 0,
        radius: 8,
    }));
    assert!(spin_seed.contains(&Proof::BankItemId {
        id: FLAX_ID,
        count: FLAX_SPIN_SEED,
    }));
    let spin_watch = spin.steps[spin_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert_eq!(spin_watch[0], wheel);
    assert_eq!(spin_watch[1], craft_xp);
    assert_eq!(spin_watch[2], string);
    assert_eq!(spin.proof, string);

    let eggs = get("herblore_secondaries").expect("herblore_secondaries");
    assert_eq!(eggs.settings.start_script, Some("HerbloreSecondaries"));
    assert_eq!(eggs.settings.deadline, HERBLORE_EGG_DEADLINE);
    let inject = settings_inject_map(eggs.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("secondary"),
        Some(&Value::String("Red spiders' eggs".into()))
    );
    assert_eq!(
        inject.get("loadout"),
        Some(&Value::String("Scenario Herblore food".into()))
    );
    assert_eq!(inject.get("foodWithdraw"), Some(&Value::from(10.0)));
    let loadouts = eggs
        .settings
        .fixture_loadouts
        .expect("eggs pins scriptFood via fixture loadout");
    assert_eq!(loadouts[0].name, "Scenario Herblore food");
    assert_eq!(loadouts[0].carry, &[("Lobster", 10)]);
    let eggs_start = eggs
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let eggs_seed = eggs.steps[..eggs_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    // native_bank_seed arrival radius is 8 (not the old givebank radius 1).
    assert!(eggs_seed.contains(&Proof::ArrivedNear {
        x: EDGEVILLE_BANK_APPROACH.x,
        z: EDGEVILLE_BANK_APPROACH.z,
        level: EDGEVILLE_BANK_APPROACH.level,
        radius: 8,
    }));
    assert!(eggs_seed.contains(&Proof::ArrivedNear {
        x: 3120,
        z: 9952,
        level: 0,
        radius: 8,
    }));
    assert!(eggs_seed.contains(&Proof::ItemId {
        id: NOTED_LOBSTER_ID,
        count: HERBLORE_EGG_FOOD_SEED,
    }));
    assert!(eggs_seed.contains(&Proof::BankItemId {
        id: LOBSTER_ID,
        count: HERBLORE_EGG_FOOD_SEED,
    }));
    assert!(eggs_seed.contains(&Proof::ItemIdAtMost {
        id: NOTED_LOBSTER_ID,
        count: 0,
    }));
    assert!(eggs_seed.contains(&Proof::BankItemIdAtMost {
        id: NOTED_LOBSTER_ID,
        count: 0,
    }));
    assert!(eggs_seed.contains(&Proof::ItemId {
        id: LOBSTER_ID,
        count: HERBLORE_EGG_FOOD_CARRY,
    }));
    assert!(eggs_seed.contains(&Proof::ItemIdAtMost {
        id: LOBSTER_ID,
        count: HERBLORE_EGG_FOOD_CARRY,
    }));
    let seed_deposit = eggs.steps[..eggs_start]
        .iter()
        .find(|step| {
            matches!(
                (&step.kind, &step.wait.arm),
                (
                    StepKind::Repeat { .. },
                    Proof::BankItemId {
                        id: LOBSTER_ID,
                        count: HERBLORE_EGG_FOOD_SEED
                    }
                )
            )
        })
        .expect("native lobster deposit wait");
    assert_eq!(seed_deposit.wait.budget_ticks, 200);
    assert_eq!(
        EDGEVILLE_BANK_BOOTH,
        WorldTile {
            x: 3096,
            z: 3493,
            level: 0
        }
    );
    assert_eq!(EDGEVILLE_BANK_BOOTH_ID, 2213);
    assert!(eggs_seed.contains(&Proof::LocActionNear {
        id: EDGEVILLE_BANK_BOOTH_ID,
        x: EDGEVILLE_BANK_BOOTH.x,
        z: EDGEVILLE_BANK_BOOTH.z,
        level: EDGEVILLE_BANK_BOOTH.level,
        radius: 0,
        action: "Use-quickly",
        present: true,
    }));
    assert!(eggs.steps[..eggs_start].iter().any(|step| step.name
        == "acknowledge exact Edgeville booth identity and Use-quickly action before bank send"));
    assert!(eggs.steps[..eggs_start]
        .iter()
        .any(|step| step.name == "deposit the noted lobster seed through the bank window"));
    assert!(eggs_seed.contains(&Proof::ItemIdAtMost {
        id: RED_SPIDERS_EGGS_ID,
        count: 0,
    }));
    let eggs_watch = eggs.steps[eggs_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    let egg = Proof::ItemId {
        id: RED_SPIDERS_EGGS_ID,
        count: 1,
    };
    let newt = Proof::ItemId {
        id: EYE_OF_NEWT_ID,
        count: 1,
    };
    let first_egg = eggs_watch.iter().position(|arm| *arm == egg).unwrap();
    let bank_egg = eggs_watch
        .iter()
        .position(|arm| {
            *arm == Proof::BankItemId {
                id: RED_SPIDERS_EGGS_ID,
                count: 1,
            }
        })
        .unwrap();
    let further_egg = eggs_watch.iter().rposition(|arm| *arm == egg).unwrap();
    assert!(first_egg < bank_egg);
    assert!(bank_egg < further_egg);
    assert!(!eggs_watch.contains(&newt));
    assert_eq!(eggs.proof, egg);
    let deposit = eggs
        .steps
        .iter()
        .find(|step| step.name == "watch script-taken eggs enter a fresh Edgeville bank")
        .expect("egg deposit watch");
    assert_eq!(
        deposit.wait.budget_ticks, HERBLORE_EGG_DEPOSIT_WATCH_TICKS,
        "deposit dirty-budget covers food-exhaust + dungeon bank, not 150"
    );
    let ret = eggs
        .steps
        .iter()
        .find(|step| step.name == "watch return to the egg field after banking")
        .expect("egg return watch");
    assert_eq!(
        ret.wait.budget_ticks, HERBLORE_EGG_RETURN_WATCH_TICKS,
        "return dirty-budget covers reverse dungeon, not 150"
    );
    let first = eggs
        .steps
        .iter()
        .find(|step| step.name == "watch exact red spiders' eggs 223 from the ground after Start")
        .expect("first egg watch");
    assert_eq!(
        first.wait.budget_ticks, SCRIPT_GOLD_WATCH_TICKS,
        "first-egg arm stays 150"
    );

    let buy = get("herblore_secondaries_newt").expect("herblore_secondaries_newt");
    assert_eq!(buy.settings.start_script, Some("HerbloreSecondaries"));
    assert_eq!(buy.settings.deadline, SCRIPT_GOLD_DEADLINE);
    let inject = settings_inject_map(buy.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("secondary"),
        Some(&Value::String("Eye of newt".into()))
    );
    let buy_start = buy
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let buy_seed = buy.steps[..buy_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(buy_seed.contains(&Proof::ArrivedNear {
        x: DRAYNOR_BANK_APPROACH.x,
        z: DRAYNOR_BANK_APPROACH.z,
        level: DRAYNOR_BANK_APPROACH.level,
        radius: 8,
    }));
    assert_eq!(
        DRAYNOR_BANK,
        WorldTile {
            x: 3093,
            z: 3243,
            level: 0
        },
        "canonical Draynor bank tile stays 3093,3243"
    );
    assert!(buy_seed.contains(&Proof::LocActionNear {
        id: DRAYNOR_BANK_BOOTH_ID,
        x: DRAYNOR_BANK_BOOTH.x,
        z: DRAYNOR_BANK_BOOTH.z,
        level: DRAYNOR_BANK_BOOTH.level,
        radius: 0,
        action: "Use-quickly",
        present: true,
    }));
    assert_eq!(
        DRAYNOR_BANK_BOOTH,
        WorldTile {
            x: 3091,
            z: 3243,
            level: 0
        }
    );
    assert_eq!(DRAYNOR_BANK_BOOTH_ID, 2213);
    assert!(buy.steps[..buy_start].iter().any(|step| {
        step.name
            == "acknowledge exact Draynor booth identity and Use-quickly action before bank send"
    }));
    let coin_open = buy.steps[..buy_start]
        .iter()
        .find(|step| step.name == "open and acknowledge the coin seed bank")
        .expect("newt coin bank open");
    assert!(
        matches!(coin_open.kind, StepKind::Repeat { .. }),
        "newt open must Repeat exact booth, not one-shot nearest Perform"
    );
    assert!(buy_seed.contains(&Proof::ArrivedNear {
        x: 3012,
        z: 3259,
        level: 0,
        radius: 6,
    }));
    assert!(buy_seed.contains(&Proof::ItemId {
        id: COINS_ID,
        count: HERBLORE_NEWT_COIN_SEED,
    }));
    assert!(buy_seed.contains(&Proof::BankItemId {
        id: COINS_ID,
        count: HERBLORE_NEWT_COIN_SEED,
    }));
    assert!(buy_seed.contains(&Proof::ItemIdAtMost {
        id: COINS_ID,
        count: 0,
    }));
    assert!(buy.steps[..buy_start]
        .iter()
        .any(|step| step.name == "deposit the coin seed through the bank window"));
    let buy_watch = buy.steps[buy_start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(buy_watch.contains(&newt));
    assert!(buy_watch.contains(&Proof::BankItemId {
        id: EYE_OF_NEWT_ID,
        count: 1,
    }));
    assert!(!buy_watch.contains(&egg));
    assert_eq!(buy.proof, newt);

    for name in [
        "flax_aio",
        "flax_aio_pick",
        "flax_aio_spin",
        "herblore_secondaries",
        "herblore_secondaries_newt",
    ] {
        assert!(names().contains(&name));
    }
}

#[test]
fn combat_core_cases_register_melee_cycles() {
    let strength = Proof::StatXpGain {
        id: STRENGTH_STAT,
        min: 1,
    };
    let attack = Proof::Stat {
        id: 0,
        min: COMBAT_ATTACK_LEVEL,
    };
    let chaos = get("chaos_druid").expect("chaos_druid");
    assert_eq!(chaos.settings.start_script, Some("ChaosDruidKiller"));
    assert_eq!(chaos.settings.deadline, SCRIPT_GOLD_DEADLINE);
    let inject = settings_inject_map(chaos.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("location"),
        Some(&Value::String("Edgeville Dungeon".into()))
    );
    assert_eq!(
        inject.get("combatStyleIndex"),
        Some(&Value::String("1".into()))
    );
    let start = chaos
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let seed = chaos.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::ArrivedNear {
        x: 3110,
        z: 9936,
        level: 0,
        radius: 14,
    }));
    assert!(seed.contains(&attack));
    assert!(seed.contains(&Proof::Stat {
        id: STRENGTH_STAT,
        min: COMBAT_ATTACK_LEVEL,
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: LOBSTER_ID,
        count: CHAOS_DRUID_FOOD,
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: COMBAT_SCIMITAR_ID,
        count: 1,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: UNIDENTIFIED_GUAM_ID,
        count: 0,
    }));
    assert_eq!(chaos.steps[start + 1].wait.arm, strength);
    assert_eq!(chaos.proof, strength);

    let tower = get("chaos_druid_tower").expect("chaos_druid_tower");
    assert_eq!(tower.settings.start_script, Some("ChaosDruidKiller"));
    let inject = settings_inject_map(tower.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("location"),
        Some(&Value::String("Chaos Druid Tower".into()))
    );
    assert_eq!(
        inject.get("combatStyleIndex"),
        Some(&Value::String("1".into()))
    );
    let start = tower
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let seed = tower.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::ArrivedNear {
        x: 2562,
        z: 3356,
        level: 0,
        radius: 4,
    }));
    assert!(seed.contains(&Proof::Stat {
        id: THIEVING_STAT,
        min: 46,
    }));
    assert_eq!(tower.proof, strength);

    let yanille = get("chaos_druid_yanille").expect("chaos_druid_yanille");
    assert_eq!(yanille.settings.start_script, Some("ChaosDruidKiller"));
    let inject = settings_inject_map(yanille.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("location"),
        Some(&Value::String("Yanille Dungeon".into()))
    );
    let start = yanille
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let seed = yanille.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::ArrivedNear {
        x: 2580,
        z: 9501,
        level: 0,
        radius: 8,
    }));
    assert!(seed.contains(&Proof::Stat {
        id: AGILITY_STAT,
        min: 40,
    }));
    assert_eq!(yanille.proof, strength);

    let moss = get("moss_giant").expect("moss_giant");
    assert_eq!(moss.settings.start_script, Some("MossGiant"));
    let inject = settings_inject_map(moss.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("combatStyle"),
        Some(&Value::String("melee".into()))
    );
    assert_eq!(
        inject.get("meleeStyle"),
        Some(&Value::String("strength".into()))
    );
    assert_eq!(inject.get("buryBones"), Some(&Value::Bool(false)));
    let start = moss
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let seed = moss.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::ArrivedNear {
        x: 2553,
        z: 3406,
        level: 0,
        radius: 10,
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: LOBSTER_ID,
        count: MOSS_GIANT_FOOD,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: BIG_BONES_ID,
        count: 0,
    }));
    assert_eq!(moss.proof, strength);

    let hill = get("hill_giant").expect("hill_giant");
    assert_eq!(hill.settings.start_script, Some("HillGiant"));
    let inject = settings_inject_map(hill.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("meleeStyle"),
        Some(&Value::String("strength".into()))
    );
    assert_eq!(inject.get("buryBones"), Some(&Value::Bool(false)));
    assert!(!inject.contains_key("weapon"));
    let start = hill
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let seed = hill.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::ArrivedNear {
        x: 3110,
        z: 9832,
        level: 0,
        radius: 16,
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: TROUT_ID,
        count: HILL_GIANT_FOOD,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: BIG_BONES_ID,
        count: 0,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: LIMPWURT_ROOT_ID,
        count: 0,
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: BRASS_KEY_ID,
        count: 1,
    }));
    assert_eq!(hill.proof, strength);

    let auto = get("auto_fighter").expect("auto_fighter");
    assert_eq!(auto.settings.start_script, Some("AutoFighter"));
    let inject = settings_inject_map(auto.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("target"), Some(&Value::String("Guard".into())));
    assert_eq!(
        inject.get("spot"),
        Some(&Value::String("Start position".into()))
    );
    assert_eq!(inject.get("banking"), Some(&Value::String("None".into())));
    assert_eq!(inject.get("solveClues"), Some(&Value::Bool(false)));
    assert_eq!(inject.get("useSpecial"), Some(&Value::Bool(false)));
    let start = auto
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let seed = auto.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::ArrivedNear {
        x: 2661,
        z: 3306,
        level: 0,
        radius: 8,
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: TROUT_ID,
        count: AUTO_FIGHTER_FOOD,
    }));
    assert_eq!(auto.proof, strength);

    for name in [
        "chaos_druid",
        "chaos_druid_tower",
        "chaos_druid_yanille",
        "moss_giant",
        "hill_giant",
        "auto_fighter",
        "rock_crab",
        "green_dragon",
        "fire_giant",
        "ardy_fighter",
    ] {
        assert!(names().contains(&name));
        let scenario = get(name).unwrap();
        assert_eq!(scenario.settings.deadline, SCRIPT_GOLD_DEADLINE);
        assert!(!scenario
            .steps
            .iter()
            .any(|step| matches!(step.wait.arm, Proof::BankItemId { .. })));
    }
}

#[test]
fn remaining_fighter_core_cases_register_melee_cycles() {
    let strength = Proof::StatXpGain {
        id: STRENGTH_STAT,
        min: 1,
    };
    let rock = get("rock_crab").expect("rock_crab");
    assert_eq!(rock.settings.start_script, Some("RockCrab"));
    assert_eq!(rock.settings.deadline, SCRIPT_GOLD_DEADLINE);
    let inject = settings_inject_map(rock.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("combatStyle"),
        Some(&Value::String("melee".into()))
    );
    assert_eq!(
        inject.get("meleeStyle"),
        Some(&Value::String("strength".into()))
    );
    assert_eq!(inject.get("solveClues"), Some(&Value::Bool(false)));
    assert_eq!(
        inject.get("bankStrategy"),
        Some(&Value::String("Off".into()))
    );
    let start = rock
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let seed = rock.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::ArrivedNear {
        x: 2712,
        z: 3707,
        level: 0,
        radius: 2,
    }));
    assert!(rock.steps[..start]
        .iter()
        .any(|step| step.name == "acknowledge dormant Rocks in the supported field before Start"));
    assert!(seed.contains(&Proof::ItemId {
        id: LOBSTER_ID,
        count: ROCK_CRAB_FOOD,
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: COMBAT_SCIMITAR_ID,
        count: 1,
    }));
    // The frozen RockCrab `GearEquip` refuses the carried melee fixture, so
    // the cell arrives wearing it: the wear step's own EquipmentId proof is
    // the pre-Start wear, and the catalog baseline requires the worn id.
    assert!(seed.contains(&Proof::EquipmentId {
        id: COMBAT_SCIMITAR_ID,
    }));
    assert!(rock.steps[..start].iter().any(|step| step.name
            == "wield and acknowledge the prepared Adamant scimitar before the hostile-field teleport"));
    assert_eq!(rock.steps[start + 1].wait.arm, strength);
    assert_eq!(rock.proof, strength);

    let dragon = get("green_dragon").expect("green_dragon");
    assert_eq!(dragon.settings.start_script, Some("GreenDragon"));
    let inject = settings_inject_map(dragon.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("useSpecial"), Some(&Value::Bool(false)));
    assert_eq!(inject.get("usePotions"), Some(&Value::Bool(false)));
    assert_eq!(
        inject.get("escape"),
        Some(&Value::String("Flee to bank".into()))
    );
    assert_eq!(
        inject.get("weapon"),
        Some(&Value::String("Rune scimitar".into()))
    );
    assert_eq!(
        inject.get("shield"),
        Some(&Value::String("Dragonfire shield".into()))
    );
    let start = dragon
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let seed = dragon.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::ArrivedNear {
        x: 3096,
        z: 3814,
        level: 0,
        radius: 22,
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: RUNE_SCIMITAR_ID,
        count: 1,
    }));
    assert!(seed.contains(&Proof::EquipmentId {
        id: DRAGONFIRE_SHIELD_ID,
    }));
    assert!(dragon.steps[..start]
        .iter()
        .any(|step| step.name
            == "wear and acknowledge Dragonfire shield before hostile-field teleport"));

    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: DRAGON_BONES_ID,
        count: 0,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: GREEN_DRAGONHIDE_ID,
        count: 0,
    }));
    assert_eq!(dragon.proof, strength);

    let giant = get("fire_giant").expect("fire_giant");
    assert_eq!(giant.settings.start_script, Some("FireGiant"));
    let inject = settings_inject_map(giant.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("escapeTele"),
        Some(&Value::String("Barrel (free)".into()))
    );
    assert_eq!(inject.get("buryBones"), Some(&Value::Bool(false)));
    assert!(giant.steps.iter().any(|step| matches!(
        step.wait.arm,
        Proof::QuestDone {
            name: "Waterfall Quest"
        }
    )));
    let start = giant
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let seed = giant.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::ArrivedNear {
        x: 2575,
        z: 9893,
        level: 0,
        radius: 10,
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: GLARIALS_AMULET_ID,
        count: 1,
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: ROPE_ID,
        count: 1,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: BIG_BONES_ID,
        count: 0,
    }));
    // Frozen FireGiant GearEquip refuses carried melee; wear + EquipmentId
    // before the hostile tele match rock_crab's pre-Start receipt.
    assert!(seed.contains(&Proof::EquipmentId {
        id: COMBAT_SCIMITAR_ID,
    }));
    assert!(giant.steps[..start].iter().any(|step| step.name
            == "wield and acknowledge the prepared Adamant scimitar before the hostile-field teleport"));
    assert!(seed.contains(&Proof::NoActiveContinue));
    let wear_i = giant.steps[..start]
            .iter()
            .position(|step| {
                step.name
                    == "wield and acknowledge the prepared Adamant scimitar before the hostile-field teleport"
            })
            .expect("fire_giant wear step");
    let drain_i = giant.steps[..start]
        .iter()
        .position(|step| {
            step.name == "drain setstat level-up dialogs before the hostile-field teleport"
        })
        .expect("fire_giant setstat drain");
    let tele_i = giant.steps[..start]
        .iter()
        .position(|step| {
            step.name == "teleport into the hostile field only after preparation is acknowledged"
        })
        .expect("fire_giant hostile tele");
    assert!(
            wear_i < drain_i && drain_i < tele_i && tele_i < start,
            "wear then setstat drain then tele then Start: wear={wear_i} drain={drain_i} tele={tele_i} start={start}"
        );
    assert!(matches!(
        giant.steps[drain_i].kind,
        StepKind::DrainDialogs { choice: 1 }
    ));
    assert_eq!(giant.proof, strength);

    let ardy = get("ardy_fighter").expect("ardy_fighter");
    assert_eq!(ardy.settings.start_script, Some("ArdyFighter"));
    let inject = settings_inject_map(ardy.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("target"), Some(&Value::String("Guard".into())));
    assert_eq!(
        inject.get("combatStyle"),
        Some(&Value::String("strength".into()))
    );
    assert_eq!(inject.get("solveClues"), Some(&Value::Bool(false)));
    assert_eq!(
        inject.get("bankStrategy"),
        Some(&Value::String("Off".into()))
    );
    assert_eq!(inject.get("foodTarget"), Some(&Value::from(1.0)));
    let start = ardy
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let seed = ardy.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::ArrivedNear {
        x: 2661,
        z: 3306,
        level: 0,
        radius: 12,
    }));
    assert!(seed.contains(&Proof::Stat {
        id: THIEVING_STAT,
        min: 5,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: CAKE_ID,
        count: 0,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: BREAD_ID,
        count: 0,
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: CHOCOLATE_SLICE_ID,
        count: 0,
    }));
    assert!(!seed.contains(&Proof::ItemId {
        id: CAKE_ID,
        count: 1,
    }));
    // Other combat cores stay unworn / undrained for setstat; only the
    // two FireGiant cells own that pre-Start hop.
    assert!(!seed.contains(&Proof::EquipmentId {
        id: COMBAT_SCIMITAR_ID,
    }));
    assert!(!seed.contains(&Proof::NoActiveContinue));
    assert_eq!(ardy.proof, strength);
}

#[test]
fn prepared_dragon_and_fire_fixtures_acknowledge_tier40_gear_before_teleport() {
    const DEFENCE_STAT: i32 = 1;
    const GREEN_FOOD: i32 = 20;
    const FIRE_FOOD: i32 = 12;
    const RUNE_ARMOUR: [i32; 3] = [1113, 1079, 1163];

    for (name, original, food, worn_weapon) in [
        ("green_dragon_prepared", "green_dragon", GREEN_FOOD, false),
        ("fire_giant_prepared", "fire_giant", FIRE_FOOD, true),
    ] {
        let scenario = get(name).unwrap_or_else(|| panic!("{name} is registered"));
        let original = get(original).unwrap();
        assert_eq!(
            scenario.settings.start_script,
            original.settings.start_script
        );
        let prepared_inject =
            settings_inject_map(scenario.settings.script_settings_inject).unwrap();
        let original_inject =
            settings_inject_map(original.settings.script_settings_inject).unwrap();
        for (id, value) in original_inject {
            assert_eq!(
                prepared_inject.get(&id),
                Some(&value),
                "{name} preserves existing setting {id}"
            );
        }
        if name == "fire_giant_prepared" {
            assert_eq!(
                prepared_inject.get("weapon"),
                Some(&Value::String("Rune scimitar".into()))
            );
        }
        assert_eq!(
            scenario.settings.deadline,
            Duration::from_secs(300),
            "{name} uses the 300s combat qualification wall"
        );
        assert_eq!(
            original.settings.deadline, SCRIPT_GOLD_DEADLINE,
            "{name}'s Defence-1 original keeps the gold wall"
        );
        assert_eq!(
            scenario.proof,
            Proof::StatXpGain {
                id: STRENGTH_STAT,
                min: 1
            }
        );

        let start = scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .expect("prepared combat Start");
        assert!(
            scenario.steps[start + 1..]
                .iter()
                .all(|step| step.wait.budget_ticks >= 750),
            "{name} post-Start dirty budgets must not pre-empt the 300s wall"
        );
        let before = &scenario.steps[..start];
        let arms = before.iter().map(|step| step.wait.arm).collect::<Vec<_>>();
        for id in [0, STRENGTH_STAT, DEFENCE_STAT, 3] {
            assert!(
                arms.contains(&Proof::Stat {
                    id,
                    min: COMBAT_ATTACK_LEVEL,
                }),
                "{name} acknowledges base stat {id} at 40"
            );
        }
        assert!(arms.contains(&Proof::ItemId {
            id: LOBSTER_ID,
            count: food,
        }));
        assert!(arms.contains(&Proof::ItemId {
            id: RUNE_SCIMITAR_ID,
            count: 1,
        }));
        for id in RUNE_ARMOUR {
            assert!(
                arms.contains(&Proof::EquipmentId { id }),
                "{name} acknowledges worn armour {id}"
            );
        }
        assert_eq!(
            arms.contains(&Proof::EquipmentId {
                id: RUNE_SCIMITAR_ID,
            }),
            worn_weapon,
            "{name} keeps its declared weapon preparation path"
        );

        let drain = before
            .iter()
            .position(|step| {
                step.name == "drain setstat level-up dialogs before the hostile-field teleport"
            })
            .expect("prepared setstat drain");
        let teleport = before
            .iter()
            .position(|step| {
                step.name
                    == "teleport into the hostile field only after preparation is acknowledged"
            })
            .expect("prepared hostile teleport");
        let last_wear = before
            .iter()
            .enumerate()
            .filter(|(_, step)| matches!(step.wait.arm, Proof::EquipmentId { .. }))
            .map(|(index, _)| index)
            .max()
            .expect("prepared worn equipment");
        assert!(
            last_wear < drain && drain < teleport,
            "{name}: armour/weapon wear, dialog drain, hostile teleport, Start"
        );
        assert!(matches!(
            before[drain].kind,
            StepKind::DrainDialogs { choice: 1 }
        ));
        assert!(before[..teleport]
            .iter()
            .any(|step| step.wait.arm == Proof::NoActiveContinue));

        let mut client = native_seed_client();
        let (_, written) = send_combat_seed(name, &mut client);
        assert!(written.contains("setstat defence 40"), "{name}: {written}");
        for alias in ["rune_chainbody", "rune_platelegs", "rune_full_helm"] {
            assert!(
                written.contains(&format!("give {alias} 1")),
                "{name} seeds {alias}: {written}"
            );
        }
    }

    let green = get("green_dragon_prepared").unwrap();
    let green_start = green
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let green_arms = green.steps[..green_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(green_arms.contains(&Proof::EquipmentId {
        id: DRAGONFIRE_SHIELD_ID,
    }));
    for id in [DRAGON_BONES_ID, GREEN_DRAGONHIDE_ID] {
        assert!(green_arms.contains(&Proof::ItemIdAtMost { id, count: 0 }));
    }

    let fire = get("fire_giant_prepared").unwrap();
    let fire_start = fire
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let fire_arms = fire.steps[..fire_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(fire_arms.contains(&Proof::QuestDone {
        name: "Waterfall Quest",
    }));
    for id in [GLARIALS_AMULET_ID, ROPE_ID] {
        assert!(fire_arms.contains(&Proof::ItemId { id, count: 1 }));
    }
    assert!(fire_arms.contains(&Proof::ItemIdAtMost {
        id: BIG_BONES_ID,
        count: 0,
    }));
}

#[test]
fn original_dragon_and_fire_fixtures_remain_defence1_profiles() {
    for name in ["green_dragon", "fire_giant"] {
        let scenario = get(name).unwrap();
        let start = scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        assert!(
            scenario.steps[..start].iter().all(|step| {
                step.wait.arm
                    != (Proof::Stat {
                        id: 1,
                        min: COMBAT_ATTACK_LEVEL,
                    })
            }),
            "{name} must not become a prepared Defence-40 fixture"
        );
        let mut client = native_seed_client();
        let (_, written) = send_combat_seed(name, &mut client);
        assert!(!written.contains("setstat defence"));
        assert!(!written.contains("give rune_chainbody"));
    }
}

#[test]
fn prepared_green_special_composes_exact_armour_stats_and_special_prerequisites() {
    const ATTACK_STAT: i32 = 0;
    const DEFENCE_STAT: i32 = 1;
    const HITPOINTS_STAT: i32 = 3;
    const DRAGON_DAGGER: i32 = 1215;
    const RUNE_ARMOUR: [i32; 3] = [1113, 1079, 1163];

    let prepared =
        get("green_dragon_special_prepared").expect("prepared Green special is registered");
    let original = get("green_dragon_special").expect("original Green special is registered");
    assert_eq!(
        prepared.settings.script_settings_inject,
        original.settings.script_settings_inject
    );
    assert_eq!(prepared.settings.start_script, Some("GreenDragon"));
    assert_eq!(prepared.settings.deadline, Duration::from_secs(300));
    assert_eq!(
        prepared.proof,
        Proof::StatXpGain {
            id: STRENGTH_STAT,
            min: 1,
        }
    );

    let start = prepared
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("prepared Green special Start");
    let before = &prepared.steps[..start];
    let arms = before.iter().map(|step| step.wait.arm).collect::<Vec<_>>();
    for (id, level) in [
        (ATTACK_STAT, 70),
        (STRENGTH_STAT, 70),
        (DEFENCE_STAT, 70),
        (HITPOINTS_STAT, 70),
    ] {
        assert!(
            arms.contains(&Proof::Stat { id, min: level }),
            "prepared Green special acknowledges exact base stat {id}={level}"
        );
    }
    assert!(arms.contains(&Proof::QuestDone { name: "Lost City" }));
    assert!(arms.contains(&Proof::ItemId {
        id: LOBSTER_ID,
        count: 12,
    }));
    assert!(arms.contains(&Proof::EquipmentId { id: DRAGON_DAGGER }));
    assert!(arms.contains(&Proof::EquipmentId {
        id: DRAGONFIRE_SHIELD_ID,
    }));
    for id in RUNE_ARMOUR {
        assert!(
            arms.contains(&Proof::EquipmentId { id }),
            "prepared Green special acknowledges worn armour {id}"
        );
    }
    assert!(arms.contains(&Proof::Varp { id: 300, min: 250 }));

    let position = |name: &str| {
        before
            .iter()
            .position(|step| step.name == name)
            .unwrap_or_else(|| panic!("missing preparation step {name}"))
    };
    let quest = before
        .iter()
        .position(|step| step.name.starts_with("open the native quest-journal"))
        .expect("Lost City preparation");
    let armour = before
        .iter()
        .enumerate()
        .filter(|(_, step)| {
            matches!(
                step.wait.arm,
                Proof::EquipmentId {
                    id: 1113 | 1079 | 1163
                }
            )
        })
        .map(|(index, _)| index)
        .max()
        .expect("tier-40 armour acknowledgement");
    let drain = position("drain setstat level-up dialogs before the hostile-field teleport");
    let dagger = position("wield and acknowledge Dragon dagger before hostile-field teleport");
    let energy = position("acknowledge the worn dagger's special cost is covered before Start");
    let teleport =
        position("teleport into the hostile field only after preparation is acknowledged");
    assert!(
        quest < armour && armour < drain && drain < dagger && dagger < energy && energy < teleport,
        "Lost City, armour, drain, dagger, special pool, hostile teleport, Start"
    );

    let attack_step =
        &before[position("prepare and acknowledge the Dragon-dagger Attack profile before Start")];
    let StepKind::Perform { send } = &attack_step.kind else {
        panic!("Dragon-dagger Attack preparation is a native fixture operation");
    };
    let mut attack_client = native_seed_client();
    let snapshot = GameSnapshot::new();
    let before_write = attack_client.out.pos;
    assert!(send(&mut attack_client, &snapshot));
    let attack_written =
        String::from_utf8_lossy(&attack_client.out.data()[before_write..attack_client.out.pos]);
    assert!(attack_written.contains("setstat attack 70"));

    let mut client = native_seed_client();
    let (_, written) = send_combat_seed("green_dragon_special_prepared", &mut client);
    for command in [
        "setstat attack 70",
        "setstat strength 70",
        "setstat defence 70",
        "setstat hitpoints 70",
        "give lobster 12",
        "give dragon_dagger 1",
        "give antidragonbreathshield 1",
        "give rune_chainbody 1",
        "give rune_platelegs 1",
        "give rune_full_helm 1",
    ] {
        assert!(
            written.contains(command),
            "prepared Green special seeds {command}: {written}"
        );
    }

    let original_start = original
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let original_arms = original.steps[..original_start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(!original_arms.contains(&Proof::Stat {
        id: DEFENCE_STAT,
        min: 40,
    }));
    assert!(RUNE_ARMOUR
        .into_iter()
        .all(|id| !original_arms.contains(&Proof::EquipmentId { id })));
    let mut client = native_seed_client();
    let (_, original_written) = send_combat_seed("green_dragon_special", &mut client);
    assert!(!original_written.contains("setstat defence"));
    assert!(!original_written.contains("give rune_chainbody"));
}

#[test]
fn auto_fighter_mage_prepares_and_observes_real_autocast_combat() {
    let mage = get("auto_fighter_mage").expect("auto_fighter_mage");
    assert_eq!(mage.settings.start_script, Some("AutoFighter"));
    assert_eq!(mage.settings.deadline, SCRIPT_GOLD_DEADLINE);

    let inject = settings_inject_map(mage.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("target"), Some(&Value::String("Guard".into())));
    assert_eq!(
        inject.get("spot"),
        Some(&Value::String("Start position".into()))
    );
    assert_eq!(
        inject.get("combatStyle"),
        Some(&Value::String("mage".into()))
    );
    assert_eq!(
        inject.get("spell"),
        Some(&Value::String("Fire Strike".into()))
    );
    assert_eq!(inject.get("runesWithdraw"), Some(&Value::from(150.0)));
    assert_eq!(inject.get("banking"), Some(&Value::String("None".into())));
    assert_eq!(inject.get("solveClues"), Some(&Value::Bool(false)));
    assert_eq!(inject.get("useSpecial"), Some(&Value::Bool(false)));

    let start = mage
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let before = mage.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(before.contains(&Proof::Stat { id: 6, min: 13 }));
    assert!(before.contains(&Proof::Stat { id: 3, min: 40 }));
    assert!(before.contains(&Proof::ItemId { id: 333, count: 8 }));
    assert!(before.contains(&Proof::ItemId {
        id: 558,
        count: 150,
    }));
    assert!(before.contains(&Proof::ItemId {
        id: 556,
        count: 300,
    }));
    assert!(before.contains(&Proof::EquipmentId { id: 1387 }));
    assert!(before.contains(&Proof::ArrivedNear {
        x: 2661,
        z: 3306,
        level: 0,
        radius: 8,
    }));

    let after = mage.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(after.contains(&Proof::Varp { id: 108, min: 3 }));
    assert!(after.contains(&Proof::StatXpGain { id: 6, min: 1 }));
    assert!(after.contains(&Proof::ItemIdAtMost {
        id: 558,
        count: 149,
    }));
    assert!(after.contains(&Proof::ItemIdAtMost {
        id: 556,
        count: 298,
    }));
    assert_eq!(mage.proof, Proof::StatXpGain { id: 6, min: 1 });
}

#[test]
fn ranged_and_consumable_options_prepare_the_exact_frozen_script_branches() {
    let ranged_xp = Proof::StatXpGain { id: 4, min: 1 };
    for (name, card, food_id, food_count) in [
        ("auto_fighter_range", "AutoFighter", 333, 8),
        ("rock_crab_range", "RockCrab", 379, 8),
    ] {
        let scenario = get(name).unwrap_or_else(|| panic!("{name} registered"));
        assert_eq!(scenario.settings.start_script, Some(card));
        if name == "rock_crab_range" {
            assert_eq!(
                scenario.settings.deadline,
                Duration::from_secs(300),
                "rock_crab_range uses the 300s combat qualification wall"
            );
            let start = scenario
                .steps
                .iter()
                .position(|step| matches!(step.kind, StepKind::StartScript))
                .unwrap();
            assert!(
                scenario.steps[start + 1..]
                    .iter()
                    .all(|step| step.wait.budget_ticks >= 750),
                "rock_crab_range post-Start dirty budgets must not pre-empt the 300s wall"
            );
        } else {
            assert_eq!(scenario.settings.deadline, SCRIPT_GOLD_DEADLINE);
        }
        let inject = settings_inject_map(scenario.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("combatStyle"),
            Some(&Value::String("range".into()))
        );
        assert_eq!(
            inject.get("rangeStyle"),
            Some(&Value::String("rapid".into()))
        );
        assert_eq!(
            inject.get("ammo"),
            Some(&Value::String("Bronze arrow".into()))
        );
        assert_eq!(inject.get("solveClues"), Some(&Value::Bool(false)));
        assert_eq!(
            inject.get("useSpecial"),
            (name == "auto_fighter_range").then_some(&Value::Bool(false))
        );
        assert!(!inject.contains_key("weapon"));

        let start = scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let before = scenario.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(before.contains(&Proof::Stat { id: 4, min: 40 }));
        assert!(before.contains(&Proof::ItemId {
            id: food_id,
            count: food_count
        }));
        assert!(before.contains(&Proof::EquipmentId { id: 853 }));
        assert!(before.contains(&Proof::EquipmentId { id: 882 }));
        assert!(scenario.steps[start + 1..]
            .iter()
            .any(|step| step.wait.arm == ranged_xp));
        assert_eq!(scenario.proof, ranged_xp);
    }

    let rock = get("rock_crab_range").expect("rock crab range");
    let rock_inject = settings_inject_map(rock.settings.script_settings_inject).unwrap();
    assert_eq!(
        rock_inject.get("loadout"),
        Some(&Value::String("Scenario Rock Crab food".into()))
    );
    let rock_loadouts = rock
        .settings
        .fixture_loadouts
        .expect("rock_crab_range pins scriptFood via fixture loadout");
    assert_eq!(rock_loadouts[0].name, "Scenario Rock Crab food");
    assert_eq!(rock_loadouts[0].carry, &[("Lobster", 8)]);
    assert_eq!(
        rock_inject.get("bow"),
        Some(&Value::String("Maple shortbow".into()))
    );
    let rock_start = rock
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert!(rock.steps[..rock_start]
        .iter()
        .any(|step| step.name == "acknowledge dormant Rocks in the supported field before Start"));

    let special = get("green_dragon_special").expect("green dragon special");
    let inject = settings_inject_map(special.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("useSpecial"), Some(&Value::Bool(true)));
    assert_eq!(inject.get("usePotions"), Some(&Value::Bool(false)));
    assert_eq!(
        inject.get("weapon"),
        Some(&Value::String("Dragon dagger".into()))
    );
    let start = special
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let before = special.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(before.contains(&Proof::Stat { id: 0, min: 60 }));
    assert!(before.contains(&Proof::EquipmentId { id: 1215 }));
    assert!(before.contains(&Proof::EquipmentId { id: 1540 }));
    // The card refuses to arm below the wielded weapon's cost, so the
    // prepared pool is an acknowledged prerequisite, not a post-Start fix.
    assert!(before.contains(&Proof::Varp { id: 300, min: 250 }));

    let potions = get("green_dragon_potions").expect("green dragon potions");
    let inject = settings_inject_map(potions.settings.script_settings_inject).unwrap();
    assert_eq!(inject.get("useSpecial"), Some(&Value::Bool(false)));
    assert_eq!(inject.get("usePotions"), Some(&Value::Bool(true)));
    let start = potions
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    let before = potions.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(before.contains(&Proof::ItemId { id: 145, count: 1 }));
    assert!(before.contains(&Proof::ItemId { id: 157, count: 1 }));
    assert!(before.contains(&Proof::EquipmentId { id: 1540 }));
    assert!(before.contains(&Proof::ItemIdAtMost { id: 147, count: 0 }));
    assert!(before.contains(&Proof::ItemIdAtMost { id: 159, count: 0 }));

    for name in [
        "auto_fighter_range",
        "rock_crab_range",
        "green_dragon_special",
        "green_dragon_potions",
    ] {
        assert!(names().contains(&name));
    }
}

#[test]
fn enabled_combat_option_cells_keep_explicit_preparation_and_old_failures() {
    let dart = get("moss_giant_dart").expect("moss_giant_dart registered");
    assert_eq!(dart.settings.start_script, Some("MossGiant"));
    assert_eq!(dart.settings.deadline, Duration::from_secs(300));
    assert!(
        dart.settings.fixture_loadouts.is_none(),
        "bank-only dart Start must not apply a packed food loadout"
    );
    let dart_inject = settings_inject_map(dart.settings.script_settings_inject).unwrap();
    assert_eq!(
        dart_inject.get("combatStyle"),
        Some(&Value::String("range".into()))
    );
    assert_eq!(
        dart_inject.get("bow"),
        Some(&Value::String("Bronze dart".into()))
    );
    assert_eq!(
        dart_inject.get("ammo"),
        Some(&Value::String("Rune arrow".into()))
    );
    assert_eq!(dart_inject.get("ammoWithdraw"), Some(&Value::from(80.0)));
    let dart_start = dart
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert!(
        dart.steps[..dart_start].iter().any(|step| step.wait.arm
            == Proof::ItemIdAtMost {
                id: BRONZE_DART_ID,
                count: 0
            }),
        "dart preStart must prove empty pack 806"
    );
    assert!(
        dart.steps[..dart_start].iter().any(|step| step.wait.arm
            == Proof::BankItemId {
                id: BRONZE_DART_ID,
                count: 80
            }),
        "dart preStart must prove open-bank 806x80"
    );
    assert!(
        dart.steps[..dart_start]
            .iter()
            .any(|step| step.wait.arm == Proof::BankItemIdAtMost { id: 892, count: 0 }),
        "dart preStart must prove unused Rune arrows absent"
    );
    assert!(
        !dart.steps[..dart_start]
            .iter()
            .any(|step| step.wait.arm == Proof::EquipmentId { id: BRONZE_DART_ID }),
        "bank-only dart Start must not require worn 806"
    );
    assert!(dart.steps[dart_start + 1..]
        .iter()
        .all(|step| step.wait.budget_ticks >= 750));
    assert!(dart.steps[dart_start + 1..]
        .iter()
        .any(|step| step.wait.arm == Proof::EquipmentId { id: BRONZE_DART_ID }));
    assert!(dart.steps[dart_start + 1..].iter().any(|step| {
        matches!(
            step.wait.arm,
            Proof::ArrivedNear {
                x: 2553,
                z: 3406,
                ..
            }
        )
    }));

    let moss_bank = get("moss_giant_bank").expect("moss fight-first bank cell remains");
    let moss_bank_start = moss_bank
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert!(
        moss_bank.steps[moss_bank_start + 1..]
            .iter()
            .any(|step| matches!(step.wait.arm, Proof::BankItemId { id: 532, .. })),
        "preserved moss_giant_bank still waits for earned Big bones"
    );

    let mage = get("green_dragon_mage_prepared").expect("mage option registered");
    assert_eq!(mage.settings.start_script, Some("GreenDragon"));
    assert_eq!(mage.settings.deadline, Duration::from_secs(300));
    let mage_inject = settings_inject_map(mage.settings.script_settings_inject).unwrap();
    assert_eq!(
        mage_inject.get("combatStyle"),
        Some(&Value::String("mage".into()))
    );
    assert_eq!(
        mage_inject.get("spell"),
        Some(&Value::String("Fire Strike".into()))
    );
    assert_eq!(
        mage_inject.get("staff"),
        Some(&Value::String("Staff of fire".into()))
    );
    let mage_start = mage
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert!(mage.steps[..mage_start]
        .iter()
        .any(|step| { step.wait.arm == Proof::Stat { id: 6, min: 70 } }));
    assert!(mage.steps[..mage_start].iter().any(|step| step.wait.arm
        == Proof::EquipmentId {
            id: STAFF_OF_FIRE_ID
        }));
    assert!(mage.steps[..mage_start].iter().any(|step| step.wait.arm
        == Proof::EquipmentId {
            id: DRAGONFIRE_SHIELD_ID
        }));
    assert!(
        !mage.steps[..mage_start].iter().any(|step| step.wait.arm
            == Proof::EquipmentId {
                id: RUNE_CHAINBODY_ID
            }),
        "mage prepared must not wear rune armour"
    );
    assert!(mage.steps[..mage_start].iter().any(|step| {
        step.wait.arm
            == Proof::ItemId {
                id: 558,
                count: 150,
            }
    }));
    assert!(mage.steps[mage_start + 1..]
        .iter()
        .all(|step| step.wait.budget_ticks >= 750));

    let camelot = get("fire_giant_camelot_prepared").expect("camelot option registered");
    assert_eq!(camelot.settings.start_script, Some("FireGiant"));
    assert_eq!(camelot.settings.deadline, Duration::from_secs(600));
    let camelot_inject = settings_inject_map(camelot.settings.script_settings_inject).unwrap();
    assert_eq!(
        camelot_inject.get("escapeTele"),
        Some(&Value::String("Camelot".into()))
    );
    assert_eq!(
        camelot_inject.get("foodWithdraw"),
        Some(&Value::from(FIRE_GIANT_CAMELOT_PREPARED_RESTOCK as f64))
    );
    let camelot_start = camelot
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert!(camelot.steps[camelot_start + 1..].iter().any(|step| {
        matches!(
            step.wait.arm,
            Proof::ArrivedNear {
                x: 2757,
                z: 3478,
                ..
            }
        )
    }));
    assert!(
        !camelot.steps.iter().any(|step| {
            matches!(
                step.wait.arm,
                Proof::ArrivedNear {
                    x: 2527,
                    z: 3413,
                    ..
                }
            )
        }),
        "Camelot cell must not watch the barrel wash-up"
    );
    assert!(camelot.steps[..camelot_start]
        .iter()
        .any(|step| step.wait.arm
            == Proof::ItemIdAtMost {
                id: BIG_BONES_ID,
                count: 0
            }));
    assert!(
        !camelot
            .steps
            .iter()
            .any(|step| step.name.contains("givebank") && step.name.contains("bone")),
        "never seed bones and call them earned"
    );
    assert!(camelot.steps[camelot_start + 1..]
        .iter()
        .all(|step| step.wait.budget_ticks >= 1500));
}

#[test]
fn green_dragon_potions_super_attack_boost_arm_tracks_acknowledged_base_level() {
    const ATTACK_STAT: i32 = 0;
    const PREPARED_ATTACK: i32 = 70;
    const NATIVE_SUPER_ATTACK_BOOST: i32 = 47;

    fn super_attack_boost_arm(scenario: &Scenario) -> Proof {
        scenario
            .steps
            .iter()
            .find(|step| step.name == "watch the native Super attack boost before further combat")
            .map(|step| step.wait.arm)
            .unwrap_or_else(|| panic!("missing Super attack boost arm on {}", scenario.name))
    }

    fn attack_effective_snapshot(level: i32) -> GameSnapshot {
        use client::io::ServerProt;
        let mut client = native_seed_client();
        client.stat_effective_level[ATTACK_STAT as usize] = level;
        client.bump_gens(ServerProt::UPDATE_STAT);
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(&client);
        snapshot
    }

    let original = get("green_dragon_potions").expect("green dragon potions");
    let prepared = get("green_dragon_potions_prepared").expect("prepared green dragon potions");
    let original_arm = super_attack_boost_arm(&original);
    let prepared_arm = super_attack_boost_arm(&prepared);

    assert_eq!(
        original_arm,
        Proof::Stat {
            id: ATTACK_STAT,
            min: COMBAT_ATTACK_LEVEL + 1,
        }
    );
    assert_eq!(
        prepared_arm,
        Proof::Stat {
            id: ATTACK_STAT,
            min: PREPARED_ATTACK + 1,
        }
    );

    assert!(
        !original_arm.check(&attack_effective_snapshot(COMBAT_ATTACK_LEVEL), None),
        "unboosted Attack {COMBAT_ATTACK_LEVEL} must not satisfy the original boost arm"
    );
    assert!(
        original_arm.check(&attack_effective_snapshot(NATIVE_SUPER_ATTACK_BOOST), None),
        "native Super attack boost must satisfy the original arm"
    );
    assert!(
        !prepared_arm.check(&attack_effective_snapshot(PREPARED_ATTACK), None),
        "unboosted Attack {PREPARED_ATTACK} must not satisfy the prepared boost arm"
    );
    assert!(
        prepared_arm.check(&attack_effective_snapshot(PREPARED_ATTACK + 1), None),
        "a native Super attack boost above the prepared base must satisfy the prepared arm"
    );
}

#[test]
fn bone_burier_requires_banking_between_burial_cycles() {
    let s = get("bone_burier").unwrap();
    assert_eq!(s.settings.start_script, Some("BoneBurier"));
    let start = s
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert!(s.steps[..start]
        .iter()
        .any(|step| matches!(step.kind, StepKind::Relog)));
    let arms: Vec<_> = s.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect();
    assert_eq!(
        arms,
        vec![
            Proof::StatXpGain { id: 5, min: 22 },
            Proof::ItemAtMost {
                name: "Bones",
                count: 0
            },
            Proof::BankItem {
                name: "Bones",
                count: 28
            },
            Proof::Item {
                name: "Bones",
                count: 28
            },
            Proof::BankItemAtMost {
                name: "Bones",
                count: 0
            },
            Proof::BankClosed,
            Proof::StatXpGain { id: 5, min: 27 },
        ]
    );
    assert_eq!(s.proof, Proof::StatXpGain { id: 5, min: 27 });
}

#[test]
fn bone_burier_v2_scenarios_are_exact_file_cards_not_catalog() {
    for (name, file) in [
        ("bone_burier_v2_ts", "bone_burier_v2.ts"),
        ("bone_burier_v2_js", "bone_burier_v2.js"),
    ] {
        let s = get(name).unwrap_or_else(|| panic!("{name} is registered"));
        assert_eq!(s.name, name);
        assert_eq!(s.settings.start_script, None);
        assert_eq!(s.settings.start_file, Some(file));
        assert_eq!(
            s.settings.wait_script_stop,
            Some("confirmed loaded current-generation bank exhaustion")
        );
        assert_eq!(s.settings.terminal_shot, Some(name));
        assert_eq!(s.settings.deadline, BONE_BURIER_V2_DEADLINE);
        assert_eq!(s.settings.fixture_prereqs, Some(BONE_BURIER_V2_PREREQS));
        assert!(!s
            .settings
            .fixture_prereqs
            .unwrap()
            .iter()
            .any(|p| matches!(p, Proof::BankItem { .. } | Proof::BankClosed)));
        let start = s
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        assert!(matches!(s.steps[start - 1].kind, StepKind::Relog));
        assert_eq!(
            s.steps[start - 1].wait.arm,
            Proof::SideTabAvailable { index: 3 }
        );
        assert_eq!(
            s.steps[start - 2].wait.arm,
            Proof::ArrivedNear {
                x: BONE_BURIER_V2_BANK.x,
                z: BONE_BURIER_V2_BANK.z,
                level: BONE_BURIER_V2_BANK.level,
                radius: 8,
            }
        );
        let arms: Vec<_> = s.steps[start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect();
        assert_eq!(
            arms,
            vec![
                Proof::StatXpGain { id: 5, min: 22 },
                Proof::ItemAtMost {
                    name: "Bones",
                    count: 0
                },
                Proof::BankItem {
                    name: "Bones",
                    count: 28
                },
                Proof::Item {
                    name: "Bones",
                    count: 28
                },
                Proof::BankItemAtMost {
                    name: "Bones",
                    count: 0
                },
                Proof::BankClosed,
                Proof::FreshStatXpGain { id: 5, min: 1 },
                Proof::ItemAtMost {
                    name: "Bones",
                    count: 0
                },
                Proof::BankItemAtMost {
                    name: "Bones",
                    count: 0
                },
            ]
        );
        let later = arms
            .iter()
            .rposition(|arm| {
                matches!(
                    arm,
                    Proof::BankItemAtMost {
                        name: "Bones",
                        count: 0
                    }
                )
            })
            .unwrap();
        let first_zero = arms
            .iter()
            .position(|arm| {
                matches!(
                    arm,
                    Proof::BankItemAtMost {
                        name: "Bones",
                        count: 0
                    }
                )
            })
            .unwrap();
        assert!(
            first_zero < later,
            "the post-withdraw zero row cannot satisfy the later exhaustion observation"
        );
        assert!(matches!(
            arms[later - 1],
            Proof::ItemAtMost {
                name: "Bones",
                count: 0
            }
        ));
        assert!(matches!(
            arms[later - 2],
            Proof::FreshStatXpGain { id: 5, min: 1 }
        ));
        assert_eq!(s.proof, arms[later]);
        assert!(names().contains(&name));
    }
    let v1 = get("bone_burier").unwrap();
    assert_eq!(v1.settings.start_script, Some("BoneBurier"));
    assert_eq!(v1.settings.start_file, None);
    assert_eq!(v1.settings.wait_script_stop, None);
}

#[test]
fn script_gold_watch_is_a_short_agentic_budget() {
    for name in [
        "bone_burier",
        "chicken_killer",
        "chicken_killer_bank",
        "thiever",
        "alcher",
        "alcher_custom_alias",
        "alcher_custom_name",
        "alcher_low",
        "alcher_fire_battlestaff",
        "bank_fletcher",
        "bank_fletcher_string",
        "bank_fletcher_cut_string",
        "dart_fletcher",
        "dart_fletcher_iron",
        "herb_cleaner",
        "herb_cleaner_named",
        "gem_cutter",
        "gem_cutter_named",
        "door_opener",
        "door_opener_gate",
        "gnome_course",
        "gnome_course_radius",
        "flax_picker",
        "superheater",
        "superheater_steel",
        "superheater_fire_battlestaff",
        "superheater_silver_low_natures",
        "superheater_mithril",
        "vial_filler",
        "vial_filler_east",
        "potion_maker",
        "potion_maker_named",
        "tanner_bot",
        "tanner_bot_hard",
        "rune_crafter",
        "rune_crafter_earth",
        "mule_crafter",
        "ardy_thiever",
        "ardy_thiever_knight",
        "gnome_chop",
        "coal_trucks",
        "cook_bot",
        "cook_bot_lobster",
        "smelter_bot",
        "smelter_bot_steel",
        "flax_spinner",
        "flax_aio",
        "flax_aio_pick",
        "flax_aio_spin",
        "herblore_secondaries_newt",
        "chaos_druid",
        "moss_giant",
        "hill_giant",
        "auto_fighter",
        "auto_fighter_mage",
        "rock_crab",
        "green_dragon",
        "fire_giant",
        "ardy_fighter",
        "script_trade",
    ] {
        let s = get(name).unwrap_or_else(|| panic!("{name} registered"));
        assert_eq!(s.settings.deadline, SCRIPT_GOLD_DEADLINE, "{name} deadline");
        let start = s
            .steps
            .iter()
            .position(|st| matches!(st.kind, StepKind::StartScript))
            .unwrap_or_else(|| panic!("{name} StartScript"));
        let watch = &s.steps[start + 1];
        assert_eq!(
            watch.wait.budget_ticks, SCRIPT_GOLD_WATCH_TICKS,
            "{name} watch ticks"
        );
        assert!(
            watch.name.starts_with("watch "),
            "{name} step after Start is the watch, got {}",
            watch.name
        );
    }
}

#[test]
fn nearest_door_loc_returns_the_offset_open_leaf_not_the_packed_at() {
    // Live Catherby: packed closed 1530 is (2816,3438); the open leaf
    // 1531 sits a tile off that `at`. The closer used to find the id
    // in radius 3 then still slam packed `at`, so interact_with_loc
    // looked up a typecode on an empty tile.
    let packed = WorldTile {
        x: 2816,
        z: 3438,
        level: 0,
    };
    let found = nearest_door_loc(packed, |x, z| (x == 2816 && z == 3439).then_some(OPEN_ID));
    assert_eq!(
        found,
        Some((
            WorldTile {
                x: 2816,
                z: 3439,
                level: 0
            },
            OPEN_ID
        )),
        "slam target is the live leaf tile, not packed at"
    );
}

#[test]
fn nearest_door_loc_keeps_the_packed_closed_leaf() {
    let packed = WorldTile {
        x: 2816,
        z: 3438,
        level: 0,
    };
    let found = nearest_door_loc(packed, |x, z| {
        (x == packed.x && z == packed.z).then_some(CLOSED_ID)
    });
    assert_eq!(found, Some((packed, CLOSED_ID)));
}

#[test]
fn nav_door_is_a_two_profile_fleet_with_a_door_closer_companion() {
    let s = get("nav_door").expect("nav_door is registered");
    assert_eq!(s.name, "nav_door");
    assert_eq!(s.seed.profiles, [("test", "test"), ("test2", "test2")]);
    assert!(
        s.seed.mainland,
        "the hop lands the walker before the Catherby tele"
    );
    assert_eq!(s.steps.len(), 2, "tele the walker, then follow the route");
    assert!(
        matches!(s.steps[0].kind, StepKind::Perform { .. }),
        "step 1 is the Catherby cheat-tele"
    );
    let (dest, arm) = match &s.steps[1].kind {
        StepKind::Follow { dest } => (
            *dest,
            match &s.steps[1].wait.arm {
                Proof::Arrived { x, z, level } => (*x, *z, *level),
                other => panic!("follow arm must be arrived, got {other:?}"),
            },
        ),
        _ => panic!("nav_door step 2 must be Follow"),
    };
    assert_eq!(
        dest,
        WorldTile {
            x: 2817,
            z: 3443,
            level: 0
        }
    );
    assert_eq!(arm, (2817, 3443, 0));
    assert_eq!(s.proof.name(), "arrived(2817,3443,0)");
    assert_eq!(s.companions.len(), 1, "the closer is the one companion");
    assert_eq!(
        s.companions[0].profile, 1,
        "profile 1 (test2) is the closer"
    );
    assert!(names().contains(&"nav_door"));
}

#[test]
fn nav_cart_is_a_mainland_follow_that_needs_the_cart_hop() {
    let s = get("nav_cart").expect("nav_cart is registered");
    assert_eq!(s.name, "nav_cart");
    assert_eq!(s.seed.profiles, [("test", "test")]);
    assert!(
        s.seed.mainland,
        "the hop lands the walker before the Shilo tele"
    );
    assert_eq!(s.steps.len(), 2, "give the fare + tele, then follow");
    let (tele, arm) = match &s.steps[0].kind {
        StepKind::Perform { .. } => (
            "perform",
            match &s.steps[0].wait.arm {
                Proof::Arrived { x, z, level } => (*x, *z, *level),
                other => panic!("tele arm must be arrived, got {other:?}"),
            },
        ),
        _ => panic!("nav_cart step 1 must be Perform"),
    };
    assert_eq!(tele, "perform");
    assert_eq!(arm, (2834, 2954, 0), "the tele targets the Shilo driver");
    let (dest, arm) = match &s.steps[1].kind {
        StepKind::Follow { dest } => (
            *dest,
            match &s.steps[1].wait.arm {
                Proof::Arrived { x, z, level } => (*x, *z, *level),
                other => panic!("follow arm must be arrived, got {other:?}"),
            },
        ),
        _ => panic!("nav_cart step 2 must be Follow"),
    };
    assert_eq!(
        dest,
        WorldTile {
            x: 2776,
            z: 3214,
            level: 0
        },
        "the destination is the Brimhaven cart landing"
    );
    assert_eq!(arm, (2776, 3214, 0));
    assert_eq!(s.proof.name(), "arrived(2776,3214,0)");
    assert!(s.companions.is_empty());
    assert!(names().contains(&"nav_cart"));
}

#[test]
fn nav_tele_gives_the_ring_and_follows_the_packed_rub_with_teleports_on() {
    let s = get("nav_tele").expect("nav_tele is registered");
    assert_eq!(s.name, "nav_tele");
    assert_eq!(s.seed.profiles, [("test", "test")]);
    assert!(s.seed.mainland);
    assert_eq!(
        s.steps.len(),
        2,
        "give the ring + follow with allow_teleports"
    );
    // Step 1 clears the backpack and cheats the charged ring (the
    // packed rub edge's item_req); the arm waits for the ring to land
    // in the inventory.
    match &s.steps[0].kind {
        StepKind::Perform { .. } => {}
        _ => panic!("nav_tele step 1 must be Perform"),
    }
    assert!(matches!(
        s.steps[0].wait.arm,
        Proof::Item {
            name: "Ring of dueling(8)",
            count: 1
        }
    ));
    // Step 2 is the teleport-layer follow: only a `FollowTele` step
    // arms `allow_teleports`, so the destination must be the packed
    // dueling-ring landing and the arm its scatter radius.
    let (dest, arm) = match &s.steps[1].kind {
        StepKind::FollowTele { dest } => (
            *dest,
            match &s.steps[1].wait.arm {
                Proof::ArrivedNear {
                    x,
                    z,
                    level,
                    radius,
                } => (*x, *z, *level, *radius),
                other => panic!("follow-tele arm must be arrivedNear, got {other:?}"),
            },
        ),
        _ => panic!("nav_tele step 2 must be FollowTele"),
    };
    assert_eq!(
        dest,
        WorldTile {
            x: 3315,
            z: 3235,
            level: 0
        },
        "the destination is the Al Kharid Duel Arena"
    );
    assert_eq!(arm, (3315, 3235, 0, 2));
    assert_eq!(s.proof.name(), "arrived_near(3315,3235,0,2)");
    assert!(s.companions.is_empty());
    assert!(names().contains(&"nav_tele"));
}

#[test]
fn nav_essence_follows_in_and_back_out_to_aubury() {
    let s = get("nav_essence").expect("nav_essence is registered");
    assert_eq!(s.name, "nav_essence");
    assert_eq!(s.seed.profiles, [("test", "test")]);
    assert!(s.seed.mainland);
    assert_eq!(
        s.steps.len(),
        5,
        "quest + dialog janitor + tele + entry follow + exit follow"
    );
    // Step 1 sends `~completequests`; the wait is the first
    // `p_choice` dialog it opens (never a dummy stat wait).
    assert!(matches!(s.steps[0].kind, StepKind::Perform { .. }));
    assert!(matches!(s.steps[0].wait.arm, Proof::ChatChoice));
    // Step 2 is the choice-answering janitor, waiting for the quest
    // journal to paint Rune Mysteries green.
    match &s.steps[1].kind {
        StepKind::DrainDialogs { choice } => assert_eq!(*choice, 1),
        _ => panic!("nav_essence step 2 must be DrainDialogs"),
    }
    assert!(matches!(
        s.steps[1].wait.arm,
        Proof::QuestDone {
            name: "Rune Mysteries Quest"
        }
    ));
    // Step 4 arms the entry follow to the mine pad and waits for any
    // mine landing (the landing is randomised, never the pad).
    let (dest, arm) = match &s.steps[3].kind {
        StepKind::Follow { dest } => (
            *dest,
            match &s.steps[3].wait.arm {
                Proof::EssenceMine => "in_essence_mine",
                other => panic!("entry arm must be EssenceMine, got {other:?}"),
            },
        ),
        _ => panic!("nav_essence step 4 must be Follow"),
    };
    assert_eq!(
        dest,
        WorldTile {
            x: 2912,
            z: 4833,
            level: 0
        },
        "the entry follow targets the mine pad"
    );
    assert_eq!(arm, "in_essence_mine");
    // Step 5 follows out through the exit portal to Aubury's anchor
    // (within the portal's randomised landing radius of 2).
    let (dest, arm) = match &s.steps[4].kind {
        StepKind::Follow { dest } => (
            *dest,
            match &s.steps[4].wait.arm {
                Proof::ArrivedNear {
                    x,
                    z,
                    level,
                    radius,
                } => (*x, *z, *level, *radius),
                other => panic!("exit arm must be ArrivedNear, got {other:?}"),
            },
        ),
        _ => panic!("nav_essence step 5 must be Follow"),
    };
    assert_eq!(
        dest,
        WorldTile {
            x: 3253,
            z: 3401,
            level: 0
        },
        "the exit follow targets Aubury's anchor"
    );
    assert_eq!(arm, (3253, 3401, 0, 2));
    assert_eq!(s.proof.name(), "arrived_near(3253,3401,0,2)");
    assert!(s.companions.is_empty());
    assert!(names().contains(&"nav_essence"));
}

#[test]
fn nav_elkoy_follows_into_the_village_through_the_maze_escort() {
    let s = get("nav_elkoy").expect("nav_elkoy is registered");
    assert_eq!(s.name, "nav_elkoy");
    assert_eq!(s.seed.profiles, [("test", "test")]);
    assert!(s.seed.mainland);
    assert_eq!(
        s.steps.len(),
        4,
        "quest-seed + dialog janitor + tele + escort follow"
    );
    // Step 1 sends `~completequests`; the wait is the first `p_choice`
    // dialog it opens (never a dummy stat wait).
    assert!(matches!(s.steps[0].kind, StepKind::Perform { .. }));
    assert!(matches!(s.steps[0].wait.arm, Proof::ChatChoice));
    // Step 2 is the choice-answering janitor, waiting for the journal
    // to paint Tree Gnome Village green.
    match &s.steps[1].kind {
        StepKind::DrainDialogs { choice } => assert_eq!(*choice, 1),
        _ => panic!("nav_elkoy step 2 must be DrainDialogs"),
    }
    assert!(matches!(
        s.steps[1].wait.arm,
        Proof::QuestDone {
            name: "Tree Gnome Village"
        }
    ));
    // Step 3 cheat-teles onto the maze-side Elkoy's tile (the packed
    // escort edge's `at`, one tile south of the entrance coord).
    match &s.steps[2].wait.arm {
        Proof::Arrived { x, z, level } => assert_eq!((*x, *z, *level), (2504, 3191, 0)),
        other => panic!("tele arm must be arrived, got {other:?}"),
    }
    // Step 4 follows into the village: the packed escort edge's `to`,
    // the hop the hedge maze forces.
    let (dest, arm) = match &s.steps[3].kind {
        StepKind::Follow { dest } => (
            *dest,
            match &s.steps[3].wait.arm {
                Proof::Arrived { x, z, level } => (*x, *z, *level),
                other => panic!("follow arm must be arrived, got {other:?}"),
            },
        ),
        _ => panic!("nav_elkoy step 4 must be Follow"),
    };
    assert_eq!(
        dest,
        WorldTile {
            x: 2515,
            z: 3159,
            level: 0
        },
        "the destination is the packed maze coord"
    );
    assert_eq!(arm, (2515, 3159, 0));
    assert_eq!(s.proof.name(), "arrived(2515,3159,0)");
    assert!(s.companions.is_empty());
    assert!(names().contains(&"nav_elkoy"));
}

#[test]
fn every_nav_scenario_uses_the_paint_preset() {
    for name in names() {
        if !name.starts_with("nav_") {
            continue;
        }
        let s = get(name).expect(name);
        let n = &s.settings.nav;
        assert!(
            n.show_nav_path
                && n.collision_fill
                && n.hop_labels
                && n.client_trail
                && n.camera_follow,
            "{name} missing a nav-test paint layer"
        );
        assert!(
            !n.nsew_labels && !n.component_flood,
            "{name} must not force NSEW / flood"
        );
        if name == "nav_door" {
            assert!(n.engine_speed_ms.is_none(), "door-troll stays 600ms ticks");
        } else {
            assert_eq!(n.engine_speed_ms, Some(300), "{name} halves the tickrate");
        }
    }
}

#[test]
fn scenario_settings_default_matches_the_bag() {
    let d = ScenarioSettings::default();
    assert!(d.renderer);
    assert!(d.only_render_selected);
    assert!(!d.capture);
    assert!(!d.full_rate);
    assert_eq!(
        d.nav,
        ScenarioNav::default(),
        "paint layers are opt-in per scenario"
    );
    assert_eq!(d.deadline, DEFAULT_DEADLINE);
    assert_eq!(d.terminal_shot, None);
    assert!(
        !d.require_mainland_base,
        "gate is opt-in for brand-new tutorial accounts"
    );
    assert!(d.sustains.is_empty());
    assert_eq!(d.start_script, None);
    assert_eq!(d.start_file, None);
    assert_eq!(d.wait_script_stop, None);
    assert_eq!(d.script_settings_inject, None);
    assert_eq!(d.inject_companion_as, None);
}

#[test]
fn nav_door_settings_are_full_rate_without_capture_or_sidecar() {
    let s = get("nav_door").expect("nav_door");
    assert!(s.settings.full_rate);
    assert!(!s.settings.only_render_selected);
    assert!(!s.settings.capture);
    assert!(s.settings.renderer);
    assert_eq!(s.settings.deadline, DEFAULT_DEADLINE);
    assert!(!s.settings.require_mainland_base);
}

#[test]
fn nav_door_settings_use_the_paint_preset_without_halved_ticks() {
    let s = nav_door_scenario();
    assert_eq!(s.settings.nav, nav_test_paints());
    assert!(s.settings.nav.engine_speed_ms.is_none());
    assert!(s.settings.full_rate);
}

#[test]
fn nav_paint_path_is_a_short_courtyard_walk_with_live_paint_layers() {
    let s = get("nav_paint_path").expect("nav_paint_path is registered");
    assert_eq!(s.name, "nav_paint_path");
    assert_eq!(s.seed.profiles, [("test", "test")]);
    assert!(s.seed.mainland, "the hop lands the courtyard walk");
    assert!(s.settings.require_mainland_base);
    assert_eq!(s.steps.len(), 1, "one short WalkTo step");
    let (dest, arm) = match &s.steps[0].kind {
        StepKind::Walk { dest } => (
            *dest,
            match &s.steps[0].wait.arm {
                Proof::Arrived { x, z, level } => (*x, *z, *level),
                other => panic!("walk arm must be arrived, got {other:?}"),
            },
        ),
        _ => panic!("nav_paint_path step must be Walk"),
    };
    // The courtyard walk spans ~8 tiles from the mainland landing
    // (3220,3220) or (3220,3222) down to (3220,3212).
    assert_eq!(
        dest,
        WorldTile {
            x: 3220,
            z: 3212,
            level: 0
        }
    );
    assert_eq!(arm, (3220, 3212, 0));
    assert_eq!(s.proof.name(), "arrived(3220,3212,0)");
    assert!(s.companions.is_empty(), "no closer for the paint path");
    assert!(s.settings.full_rate);
    assert_eq!(
        s.settings.nav,
        nav_test_paints().with_tick_ms(300),
        "nav tests use the paint preset and speed 300"
    );
    assert!(
        names().contains(&"nav_paint_path"),
        "registered for --live script_nav_paint_path"
    );
}

#[test]
fn nav_full_settings_carry_deadline_and_terminal_shot() {
    let s = get("nav_full").expect("nav_full");
    assert_eq!(s.settings.deadline, Duration::from_secs(360));
    assert_eq!(s.settings.terminal_shot, Some("nav_full terminal"));
    assert!(!s.settings.full_rate);
}

#[test]
fn render_smoke_settings_are_300s_and_gate_off() {
    let s = get("render_smoke").expect("render_smoke");
    assert_eq!(s.settings.deadline, Duration::from_secs(300));
    assert!(!s.settings.require_mainland_base);
}

#[test]
fn walk_settings_are_defaults() {
    let s = get("walk").expect("walk");
    assert_eq!(s.settings, ScenarioSettings::default());
}

#[test]
fn nav_routes_is_ten_borrowed_ods_with_item_seed() {
    let s = get("nav_routes").expect("nav_routes is registered");
    assert_eq!(s.name, "nav_routes");
    assert_eq!(s.seed.profiles, [("test", "test")]);
    assert!(s.seed.mainland, "unique live accounts spawn on tutorial");
    assert!(s.settings.require_mainland_base);
    assert_eq!(NAV_ROUTES.len(), 10);
    // tutorial + quest setvars + relog + journal + maxme + knife/coins/lean tele kit,
    // then tele+follow per OD.
    assert_eq!(s.steps.len(), 13 + NAV_ROUTES.len() * 2);
    assert!(matches!(s.steps[0].kind, StepKind::Repeat { .. }));
    assert!(matches!(s.steps[2].kind, StepKind::Relog));
    assert!(matches!(
        s.steps[2].wait.arm,
        Proof::SideTabAvailable { index: 3 }
    ));
    assert!(matches!(
        s.steps[4].wait.arm,
        Proof::Stat { id: 0, min: 99 }
    ));
    assert!(matches!(
        s.steps[5].wait.arm,
        Proof::Item {
            name: "Knife",
            count: 1
        }
    ));
    assert!(matches!(
        s.steps[6].wait.arm,
        Proof::Item {
            name: "Coins",
            count: 5000
        }
    ));
    assert!(matches!(
        s.steps[12].wait.arm,
        Proof::Item {
            name: "Games necklace(8)",
            count: 1
        }
    ));
    let kar_hewo = s
        .steps
        .iter()
        .find(|st| {
            matches!(
                st.kind,
                StepKind::Follow {
                    dest: WorldTile {
                        x: 3284,
                        z: 3211,
                        level: 0
                    }
                }
            )
        })
        .expect("Kar-Hewo follow");
    assert!(
        matches!(kar_hewo.wait.arm, Proof::ArrivedNear { radius: 1, .. }),
        "glider map_findsquare radius 1, not exact pad"
    );
    let last = NAV_ROUTES[9].2;
    let dest = match &s.steps[s.steps.len() - 1].kind {
        StepKind::Follow { dest } => *dest,
        _ => panic!("last step must be Follow"),
    };
    assert_eq!(dest, last);
    assert_eq!(s.proof.name(), "arrived(2817,3443,0)");
    assert!(s.settings.full_rate);
    assert_eq!(
        s.settings.nav,
        nav_test_paints().with_tick_ms(300),
        "seam run uses the paint preset and speed 300"
    );
    assert_eq!(s.settings.deadline, Duration::from_secs(3600));
    assert_eq!(s.settings.sustains, nav_energy_sustains());
    assert!(names().contains(&"nav_routes"));
}

#[test]
fn pair_cells_register_two_visible_actors_and_shared_start() {
    for (name, card, dest) in [
        (
            "nature_crafter_air",
            "NatureCrafter",
            WorldTile {
                x: 2983,
                z: 3288,
                level: 0,
            },
        ),
        (
            "mule_crafter_air",
            "MuleCrafter",
            WorldTile {
                x: 2983,
                z: 3288,
                level: 0,
            },
        ),
        (
            "flax_runner",
            "FlaxRunner",
            WorldTile {
                x: 2741,
                z: 3444,
                level: 0,
            },
        ),
        (
            "duel_arena",
            "Duel Arena Combat Trainer",
            WorldTile {
                x: 3368,
                z: 3274,
                level: 0,
            },
        ),
    ] {
        let s = get(name).unwrap_or_else(|| panic!("{name} must be registered"));
        assert_eq!(s.name, name);
        assert_eq!(s.seed.profiles.len(), 2, "{name} is a two-profile fleet");
        assert_ne!(
            s.seed.profiles[0].0, s.seed.profiles[1].0,
            "{name} actors must be distinct"
        );
        assert_eq!(s.settings.start_script, Some(card));
        assert_eq!(s.settings.inject_companion_as, None);
        assert_eq!(s.settings.deadline, SCRIPT_GOLD_DEADLINE);
        assert_eq!(s.settings.terminal_shot, Some(name));
        assert!(
            s.settings.full_rate,
            "{name} pair-watch keeps a full-rate headed cadence"
        );
        assert!(
            !s.settings.only_render_selected,
            "{name} pair-watch must draw both actors; ordinary panel default stays selected-only"
        );
        assert_eq!(s.companions.len(), 1);
        assert_eq!(s.companions[0].profile, 1);
        let start = s
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap_or_else(|| panic!("{name} must Start after prep"));
        assert!(
            start > 0,
            "{name} Start must follow native acknowledged prep"
        );
        assert!(s.steps[..start]
            .iter()
            .any(|step| matches!(step.kind, StepKind::Relog)));
        assert!(s.steps[..start].iter().any(|step| {
            matches!(
                step.wait.arm,
                Proof::ArrivedNear {
                    x,
                    z,
                    level,
                    radius: 8
                } if x == dest.x && z == dest.z && level == dest.level
            )
        }));
        assert!(names().contains(&name));
    }
    let mule = get("mule_crafter_air").expect("MuleCrafter pair is registered");
    assert!(mule
        .steps
        .iter()
        .any(|step| { step.name == "seed Mule crafter's exact noted essence bank before Start" }));
    assert!(mule.steps.iter().any(|step| {
        matches!(
            step.wait.arm,
            Proof::BankItemId {
                id: RUNE_ESSENCE_ID,
                count: RUNE_ESSENCE_SEED,
            }
        )
    }));
}

fn pair_companion_client() -> client::client::Client {
    use api::interact::CC_LOGOUT;
    use client::client::{Client, ClientConfig};
    use client::config::IfType;
    use client::dash3d::ClientPlayer;
    use client::io::ServerProt;
    use std::sync::Arc;

    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: false,
    });
    client.ingame = true;
    client.scene_state = 2;
    client.map_build_base_x = 3200;
    client.map_build_base_z = 3200;
    client.local_player = Some(ClientPlayer::at(20, 20));
    client.last_login_reconnect = Some(true);
    client.side_icon[3] = 1;
    let mut ifaces = vec![None; 16];
    ifaces[7] = Some(Box::new(IfType {
        client_code: CC_LOGOUT,
        ..Default::default()
    }));
    client.ifaces = Arc::new(ifaces);
    for prot in [
        ServerProt::PLAYER_INFO,
        ServerProt::REBUILD_NORMAL,
        ServerProt::UPDATE_STAT,
        ServerProt::IF_OPENMAIN,
    ] {
        client.bump_gens(prot);
    }
    client
}

fn pair_out_len(client: &client::client::Client) -> usize {
    client.out.pos
}

/// Host::run_client returns on !ingame before another observe, so the
/// companion callback never sees the off-world frame. A successful
/// session generation after intentional logout is the delivered
/// post-logout signal; last_login_reconnect may already be true from
/// an earlier grant and must not admit stale pre-logout scene2.
#[test]
fn pair_companion_seeds_after_reconnect_without_an_off_world_frame() {
    let scenario = get("nature_crafter_air").expect("nature_crafter_air is registered");
    let mut runner = crate::ScenarioRunner::new(scenario);
    let index = runner
        .companion_for("test2")
        .expect("air runner is profile 1");
    let mut client = pair_companion_client();
    assert_eq!(client.last_login_reconnect, Some(true));

    runner.companion_tick(index, &mut client);
    std::thread::sleep(Duration::from_millis(400));
    runner.companion_tick(index, &mut client);
    std::thread::sleep(Duration::from_millis(400));
    runner.companion_tick(index, &mut client);
    assert!(
        client.ingame,
        "the fixture keeps the in-game frame the live scheduler actually delivers"
    );

    let after_relog = pair_out_len(&client);
    runner.companion_tick(index, &mut client);
    std::thread::sleep(Duration::from_millis(400));
    runner.companion_tick(index, &mut client);
    assert_eq!(
        pair_out_len(&client),
        after_relog,
        "stale pre-logout scene2 must not seed even when last_login_reconnect is already true"
    );

    client.gens.session = client.gens.session.wrapping_add(1);
    runner.companion_tick(index, &mut client);
    std::thread::sleep(Duration::from_millis(400));
    runner.companion_tick(index, &mut client);
    assert!(
        pair_out_len(&client) > after_relog,
        "session generation change without a delivered !ingame frame must admit WaitRelog and seed"
    );
}

#[test]
fn pair_companion_close_bank_uses_native_modal_path() {
    use client::client::{Client, ClientConfig};
    use client::dash3d::ClientPlayer;
    use client::io::{ClientStream, ServerProt};
    use std::net::TcpListener;

    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: false,
    });
    client.ingame = true;
    client.scene_state = 2;
    client.map_build_base_x = 3200;
    client.map_build_base_z = 3200;
    client.local_player = Some(ClientPlayer::at(20, 20));
    client.main_modal_id = 100;
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    client.stream = Some(ClientStream::connect("127.0.0.1", port).unwrap());
    let _peer = listener.accept().unwrap();
    client.bump_gens(ServerProt::IF_OPENMAIN);
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(&client);

    match Interactions::new(&snapshot, &mut client).close_modal() {
        SendResult::Sent { .. } | SendResult::Refused { .. } => {}
    }
    assert_eq!(
        client.main_modal_id, -1,
        "native close clears local modal state"
    );
}

#[test]
fn combat_card_fixture_food_loadouts_align_with_seeded_inventory() {
    const GREEN_DRAGON_BASE_FOOD: u32 = 20;
    const GREEN_DRAGON_TRIP_FOOD: u32 = 12;
    const FIRE_GIANT_TRIP_FOOD: u32 = 12;
    for (name, card, loadout_name, food_name, carry_qty) in [
        (
            "moss_giant",
            "MossGiant",
            "Scenario Moss Giant food",
            "Lobster",
            MOSS_GIANT_FOOD as u32,
        ),
        (
            "moss_giant_prepared",
            "MossGiant",
            "Scenario Moss Giant food",
            "Lobster",
            MOSS_GIANT_FOOD as u32,
        ),
        (
            "moss_giant_bank",
            "MossGiant",
            "Scenario Moss Giant food",
            "Lobster",
            MOSS_GIANT_FOOD as u32,
        ),
        (
            "moss_giant_bank_start",
            "MossGiant",
            "Scenario Moss Giant food",
            "Lobster",
            MOSS_GIANT_FOOD as u32,
        ),
        (
            "hill_giant",
            "HillGiant",
            "Scenario Hill Giant food",
            "Trout",
            HILL_GIANT_FOOD as u32,
        ),
        (
            "hill_giant_bank",
            "HillGiant",
            "Scenario Hill Giant food",
            "Trout",
            HILL_GIANT_FOOD as u32,
        ),
        (
            "hill_giant_bank_prepared",
            "HillGiant",
            "Scenario Hill Giant food",
            "Trout",
            HILL_GIANT_FOOD as u32,
        ),
        (
            "hill_giant_loot_deposit",
            "HillGiant",
            "Scenario Hill Giant food",
            "Trout",
            HILL_GIANT_FOOD as u32,
        ),
        (
            "green_dragon",
            "GreenDragon",
            "Scenario Green Dragon food",
            "Lobster",
            GREEN_DRAGON_BASE_FOOD,
        ),
        (
            "green_dragon_special",
            "GreenDragon",
            "Scenario Green Dragon trip food",
            "Lobster",
            GREEN_DRAGON_TRIP_FOOD,
        ),
        (
            "green_dragon_potions",
            "GreenDragon",
            "Scenario Green Dragon trip food",
            "Lobster",
            GREEN_DRAGON_TRIP_FOOD,
        ),
        (
            "green_dragon_bank",
            "GreenDragon",
            "Scenario Green Dragon trip food",
            "Lobster",
            GREEN_DRAGON_TRIP_FOOD,
        ),
        (
            "green_dragon_tele",
            "GreenDragon",
            "Scenario Green Dragon trip food",
            "Lobster",
            GREEN_DRAGON_TRIP_FOOD,
        ),
        (
            "fire_giant",
            "FireGiant",
            "Scenario Fire Giant food",
            "Lobster",
            FIRE_GIANT_TRIP_FOOD,
        ),
        (
            "fire_giant_approach",
            "FireGiant",
            "Scenario Fire Giant food",
            "Lobster",
            FIRE_GIANT_TRIP_FOOD,
        ),
        (
            "fire_giant_bank",
            "FireGiant",
            "Scenario Fire Giant food",
            "Lobster",
            FIRE_GIANT_TRIP_FOOD,
        ),
        (
            "chaos_druid",
            "ChaosDruidKiller",
            "Scenario Chaos Druid food",
            "Lobster",
            CHAOS_DRUID_FOOD as u32,
        ),
        (
            "chaos_druid_bank",
            "ChaosDruidKiller",
            "Scenario Chaos Druid food",
            "Lobster",
            CHAOS_DRUID_FOOD as u32,
        ),
        (
            "rock_crab_bank",
            "RockCrab",
            "Scenario Rock Crab food",
            "Lobster",
            ROCK_CRAB_FOOD as u32,
        ),
        (
            "rock_crab_range",
            "RockCrab",
            "Scenario Rock Crab food",
            "Lobster",
            ROCK_CRAB_FOOD as u32,
        ),
    ] {
        let scenario = get(name).unwrap_or_else(|| panic!("{name} registered"));
        assert_eq!(scenario.settings.start_script, Some(card));
        let inject = settings_inject_map(scenario.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("loadout"),
            Some(&Value::String(loadout_name.into()))
        );
        assert!(
            !inject.contains_key("food"),
            "{name} must not inject a removed food setting id"
        );
        let loadouts = scenario
            .settings
            .fixture_loadouts
            .unwrap_or_else(|| panic!("{name} pins scriptFood via fixture loadout"));
        let selected = loadouts
            .iter()
            .find(|row| row.name == loadout_name)
            .unwrap_or_else(|| panic!("{name} posts loadout {loadout_name}"));
        assert_eq!(selected.carry, &[(food_name, carry_qty)]);
    }
    let green = get("green_dragon").expect("green_dragon");
    let green_loadouts = green
        .settings
        .fixture_loadouts
        .expect("green dragon loadouts");
    assert_eq!(green_loadouts.len(), 2);
    assert!(green_loadouts
        .iter()
        .any(|row| row.name == "Scenario Green Dragon trip food"));
}

// ---- P4 combat fixture repair: quest prerequisite + trip food ----

/// Whether the client's outbound buffer holds `needle` as plaintext —
/// `cheat()` writes the `::` command through `pjstr`, so the emitted
/// command stream is readable (only the opcode is ISAAC-encrypted).
fn emitted_has(client: &Client, needle: &str) -> bool {
    let bytes = &client.out.data()[..client.out.pos];
    bytes
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

/// Run the fixture's safe-tile preparation step against a synthetic
/// client so the emitted seed commands can be inspected.
fn run_prepare_step(scenario: &Scenario, client: &mut Client) -> bool {
    let step = scenario
        .steps
        .iter()
        .find(|step| step.name.starts_with("prepare melee stats"))
        .expect("the combat fixtures prepare on the safe tile");
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(client);
    match &step.kind {
        StepKind::Perform { send } => send(client, &snapshot),
        _ => panic!("the preparation step is a Perform"),
    }
}

/// A synthetic connected client (the api's sends require an attached
/// session) whose chat modal carries `options` as the native
/// `multi4`/`multi5` BUTTON_OK resume buttons.
fn chat_options_client(options: &[&str]) -> (Client, GameSnapshot) {
    let mut client = synthetic_client();
    set_chat_dialog(&mut client, options);
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(&client);
    (client, snapshot)
}

/// The connected ingame client the dialog helpers build on.
fn synthetic_client() -> Client {
    use client::client::{Client, ClientConfig};
    use client::dash3d::ClientPlayer;
    use client::io::ClientStream;
    use std::net::TcpListener;

    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: false,
    });
    client.ingame = true;
    client.scene_state = 2;
    client.map_build_base_x = 3200;
    client.map_build_base_z = 3200;
    client.local_player = Some(ClientPlayer::at(20, 20));
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    client.stream = Some(ClientStream::connect("127.0.0.1", port).unwrap());
    let _peer = listener.accept().unwrap();
    client
}

/// Re-render `client`'s chat modal as `options` — the native
/// `if_openchat(multiN)`: the same layer id and child component ids as
/// every other chat dialog, with the new question's texts and the
/// `IF_OPENCHAT` iface generation the snapshot's option gate tracks.
fn set_chat_dialog(client: &mut Client, options: &[&str]) {
    use client::config::if_type::{ButtonType, IfType, IfTypeMut};
    use client::io::ServerProt;

    for (i, text) in options.iter().enumerate() {
        let id = 101 + i;
        client.set_iface(
            id,
            IfType {
                id: id as i32,
                layer_id: 100,
                ..Default::default()
            },
        );
        client.set_iface_mut(
            id,
            IfTypeMut {
                button_type: ButtonType::BUTTON_OK,
                text: (*text).to_string(),
                ..Default::default()
            },
        );
    }
    client.set_iface(
        100,
        IfType {
            id: 100,
            layer_id: 100,
            children: Some((0..options.len() as i32).map(|i| 101 + i).collect()),
            ..Default::default()
        },
    );
    client.chat_modal_id = 100;
    client.bump_gens(ServerProt::IF_OPENCHAT);
}

/// The bytes the machine sends for `options` (empty when it refuses the
/// dialog instead of guessing a button).
fn machine_press(options: &[&str], prereq: NativeQuestPrereq) -> (bool, Vec<u8>) {
    let (mut client, snapshot) = chat_options_client(options);
    let mut state = QuestJournalState::default();
    let answered = answer_quest_journal_dialogs(&mut client, &snapshot, prereq, &mut state);
    (answered, client.out.data()[..client.out.pos].to_vec())
}

/// The bytes an equal synthetic client sends when it presses the
/// `option`-th (1-based) chat button directly.
fn reference_choice_bytes(options: &[&str], option: i32) -> Vec<u8> {
    let (mut client, snapshot) = chat_options_client(options);
    let mut ix = Interactions::new(&snapshot, &mut client);
    assert!(
        matches!(ix.answer_choice(option), SendResult::Sent { .. }),
        "reference option {option} is pressable"
    );
    client.out.data()[..client.out.pos].to_vec()
}

#[test]
fn zero_count_trip_food_is_not_emitted_and_positive_counts_still_are() {
    // ardy_fighter_bank declares food_count 0: the native `::give`
    // handler clamps to at least one (`Math.max(1, …)`), so `give cake
    // 0` seeded the Cake the empty-food baseline is supposed to lack.
    let bank = get("ardy_fighter_bank").expect("ardy_fighter_bank is registered");
    let mut client = native_seed_client();
    assert!(run_prepare_step(&bank, &mut client));
    assert!(
        !emitted_has(&client, "give cake"),
        "a zero-count trip food emits no give at all"
    );
    assert!(emitted_has(&client, "~clearinv"));
    assert!(emitted_has(&client, "give adamant_scimitar 1"));
    assert!(emitted_has(&client, "setstat attack 40"));
    assert!(emitted_has(&client, "setstat thieving 5"));

    // The positive-count cells keep their seeded trip food.
    let crab = get("rock_crab_bank").expect("rock_crab_bank is registered");
    let mut client = native_seed_client();
    assert!(run_prepare_step(&crab, &mut client));
    assert!(
        emitted_has(&client, &format!("give lobster {ROCK_CRAB_FOOD}")),
        "positive food counts keep their give"
    );
}

#[test]
fn quest_prereq_machine_presses_the_native_individual_quest_path() {
    // cheat_quest.rs2 @debug_quests: the first dialog is
    // Complete All Quests. / Reset All Quests. / Select Individual
    // Quest. / Cancel. — the machine takes the individual row, never
    // the complete-all branch whose queued completions leak past Start.
    let first = [
        "Complete All Quests.",
        "Reset All Quests.",
        "Select Individual Quest.",
        "Cancel.",
    ];
    let (answered, sent) = machine_press(&first, LOST_CITY_PREREQ);
    assert!(answered);
    assert_eq!(sent, reference_choice_bytes(&first, 3));

    // The paginated list: the quest's own row by its enum label, and
    // "Next." on a page that does not carry it.
    let page = [
        "Prev.",
        "33. Legends Quest",
        "34. Lost City",
        "35. Merlin's Crystal",
        "Next.",
    ];
    let (answered, sent) = machine_press(&page, LOST_CITY_PREREQ);
    assert!(answered);
    assert_eq!(sent, reference_choice_bytes(&page, 3));
    let (answered, sent) = machine_press(&page, WATERFALL_QUEST_PREREQ);
    assert!(answered);
    assert_eq!(sent, reference_choice_bytes(&page, 5));

    // The per-quest confirmation is Complete. (never Reset.).
    let confirm = ["Complete.", "Reset.", "Cancel."];
    let (answered, sent) = machine_press(&confirm, WATERFALL_QUEST_PREREQ);
    assert!(answered);
    assert_eq!(sent, reference_choice_bytes(&confirm, 1));

    // An unplaceable dialog fails the preparation explicitly.
    let strange = ["Bribe the guard.", "Walk away."];
    let (answered, sent) = machine_press(&strange, LOST_CITY_PREREQ);
    assert!(!answered, "an unknown dialog is never guessed");
    assert!(sent.is_empty());
}

#[test]
fn quest_prereq_machine_presses_each_native_dialog_once() {
    // The runner re-sends a `Repeat` step every tick, so the machine sees
    // the same dialog until the reply lands. `chat.rs2` re-registers the
    // same components as resume buttons for the question that follows
    // (`if_addresumebutton(multi5:com_5)`) and the engine resumes with the
    // clicked component, so a second press into the replaced dialog is not
    // inert: the page list's `Next.` would advance two pages at once.
    let page = [
        "Prev.",
        "33. Legends Quest",
        "34. Lost City",
        "35. Merlin's Crystal",
        "Next.",
    ];
    let (mut client, snapshot) = chat_options_client(&page);
    let mut state = QuestJournalState::default();
    assert!(answer_quest_journal_dialogs(
        &mut client,
        &snapshot,
        LOST_CITY_PREREQ,
        &mut state
    ));
    assert_eq!(
        client.out.data()[..client.out.pos],
        reference_choice_bytes(&page, 3)[..],
        "the required quest's own row is pressed"
    );
    // The identical dialog on the next tick sends nothing: the machine
    // waits for the dialog its press produces.
    let pressed = client.out.pos;
    assert!(answer_quest_journal_dialogs(
        &mut client,
        &snapshot,
        LOST_CITY_PREREQ,
        &mut state
    ));
    assert_eq!(client.out.pos, pressed, "the same dialog is pressed once");
    // A later page, then the confirmation, are answered.
    let next_page = [
        "Prev.",
        "36. Merlin's Crystal",
        "37. Murder Mystery",
        "38. Observatory Quest",
        "Next.",
    ];
    set_chat_dialog(&mut client, &next_page);
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(&client);
    assert!(answer_quest_journal_dialogs(
        &mut client,
        &snapshot,
        LOST_CITY_PREREQ,
        &mut state
    ));
    assert_eq!(
        client.out.data()[pressed..client.out.pos],
        reference_choice_bytes(&next_page, 5)[..],
        "a page without the quest advances with Next."
    );
}

#[test]
fn quest_prereq_settles_the_completion_modal_before_the_reset_and_start() {
    use client::config::if_type::{ComponentType, IfType, IfTypeMut};
    use client::io::ServerProt;

    // The native completion's own traffic — the reward messages, the
    // quest scroll `send_quest_complete` opens just before it paints the
    // row green, and the green itself — settles while the fixture is
    // still on the prerequisite step; the machine closes that scroll and
    // only then does the acknowledged run reach the stat/inventory reset
    // and Start. The demonstrated live failure started the script while
    // completion rewards and modal 297 were still arriving.
    const QUEST_SCROLL: i32 = 297;
    let kickoff = [
        "Complete All Quests.",
        "Reset All Quests.",
        "Select Individual Quest.",
        "Cancel.",
    ];
    let page = [
        "Prev.",
        "33. Legends Quest",
        "34. Lost City",
        "35. Merlin's Crystal",
        "Next.",
    ];
    let confirm = ["Complete.", "Reset.", "Cancel."];

    let mut scenario = get("green_dragon_special").expect("green_dragon_special is registered");
    let quest = scenario
        .steps
        .iter()
        .position(|step| step.name.starts_with("open the native quest-journal"))
        .expect("the fixture carries the native quest prerequisite");
    let prepare = scenario
        .steps
        .iter()
        .position(|step| step.name.starts_with("prepare melee stats"))
        .expect("the fixture resets stats and inventory");
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("the fixture starts the frozen script");
    scenario.steps.drain(..quest);
    scenario.steps.truncate(start - quest + 1);
    scenario.seed.mainland = false;
    scenario.settings.require_mainland_base = false;
    let (kickoff_step, prepare_step) = (0usize, prepare - quest);

    let mut client = synthetic_client();
    set_chat_dialog(&mut client, &kickoff);
    // The quest journal's Lost City row, red until the completion's
    // `if_setcolour(questlist:zanaris, ^green_rgb)`.
    client.side_icon[2] = 700;
    client.set_iface(
        700,
        IfType {
            children: Some(vec![701, 702]),
            ..Default::default()
        },
    );
    client.set_iface(
        702,
        IfType {
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    client.set_iface_mut(
        702,
        IfTypeMut {
            text: "Lost City".into(),
            colour: 0xF80000,
            ..Default::default()
        },
    );
    client.bump_gens(ServerProt::IF_OPENMAIN);

    let mut runner = ScenarioRunner::with_world(scenario, None);
    runner.set_scene_settle(Duration::ZERO);

    let mut phase = 0;
    let (mut rewards_tick, mut close_tick, mut green_tick, mut reset_tick) =
        (None, None, None, None);
    let mut step_at_rewards = None;
    for tick in 0..400 {
        let from = client.out.pos;
        runner.tick(&mut client);
        let sent = client.out.data()[from..client.out.pos].to_vec();
        let sent_has = |needle: &str| {
            sent.windows(needle.len())
                .any(|window| window == needle.as_bytes())
        };
        let step = match runner.status() {
            RunnerStatus::Running { step, .. } => Some(step),
            _ => None,
        };
        if reset_tick.is_none() && step == Some(prepare_step) && sent_has("~clearinv") {
            reset_tick = Some(tick);
        }
        if reset_tick.is_some() && green_tick.is_some() {
            break;
        }
        match phase {
            0 if sent == reference_choice_bytes(&kickoff, 3) => {
                set_chat_dialog(&mut client, &page);
                phase = 1;
            }
            1 if sent == reference_choice_bytes(&page, 3) => {
                set_chat_dialog(&mut client, &confirm);
                phase = 2;
            }
            2 if sent == reference_choice_bytes(&confirm, 1) => {
                // The completion ran: rewards, then the scroll (which
                // replaces the chat modal), then the green row.
                client.chat_text[0] = "You have completed the Lost City Of Zanaris Quest!".into();
                client.chat_modal_id = -1;
                client.main_modal_id = QUEST_SCROLL;
                client.bump_gens(ServerProt::IF_OPENMAIN);
                client.bump_gens(ServerProt::MESSAGE_GAME);
                rewards_tick = Some(tick);
                step_at_rewards = step;
                phase = 3;
            }
            3 if sent == reference_close_bytes(QUEST_SCROLL) => {
                client.main_modal_id = -1;
                client.set_iface_mut(
                    702,
                    IfTypeMut {
                        text: "Lost City".into(),
                        colour: 0xF800,
                        ..Default::default()
                    },
                );
                client.bump_gens(ServerProt::IF_SETCOLOUR);
                close_tick = Some(tick);
                green_tick = Some(tick);
                phase = 4;
            }
            _ => {}
        }
        assert!(
            step_at_rewards.is_none_or(|s| s == kickoff_step + 1),
            "step {step:?}: nothing advanced while the completion was live"
        );
    }

    assert_eq!(phase, 4, "the machine answered every native dialog in turn");
    assert_eq!(
        step_at_rewards,
        Some(kickoff_step + 1),
        "reward traffic and the completion scroll arrive on the prerequisite step"
    );
    let green_tick = green_tick.expect("the completion painted the row green");
    let close_tick = close_tick.expect("the machine closed the completion's scroll");
    let rewards_tick = rewards_tick.expect("the completion's scroll opened");
    assert!(
            close_tick > rewards_tick,
            "the scroll the completion opened is closed on the next tick (opened {rewards_tick}, closed {close_tick})"
        );
    assert_eq!(
        green_tick, close_tick,
        "the green follows the scroll's close, still on the prerequisite step"
    );
    let reset_tick = reset_tick.expect("the acknowledged run resets stats and inventory");
    assert!(
        reset_tick > green_tick,
        "the reset follows the journal acknowledgement (green {green_tick}, reset {reset_tick})"
    );
}

/// The bytes a client in the same state sends to close the open modal
/// (the native `CLOSE_MODAL`, no payload).
fn reference_close_bytes(main_modal: i32) -> Vec<u8> {
    let mut client = synthetic_client();
    client.main_modal_id = main_modal;
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(&client);
    let mut ix = Interactions::new(&snapshot, &mut client);
    assert!(
        matches!(ix.close_modal(), SendResult::Sent { .. }),
        "the open modal is closable"
    );
    client.out.data()[..client.out.pos].to_vec()
}

#[test]
fn quest_prereq_is_acknowledged_before_the_reset_teleport_and_start() {
    for (name, journal) in [
        ("fire_giant", "Waterfall Quest"),
        ("fire_giant_approach", "Waterfall Quest"),
        ("fire_giant_bank", "Waterfall Quest"),
        ("green_dragon_special", "Lost City"),
    ] {
        let scenario = get(name).expect("the combat fixture is registered");
        let index = |prefix: &str| {
            scenario
                .steps
                .iter()
                .position(|step| step.name.starts_with(prefix))
                .unwrap_or_else(|| panic!("{name}: no step starts with {prefix}"))
        };
        let load = index("open the native quest-journal prerequisite dialog");
        let ack = index("answer the quest-journal dialogs until the required quest");
        let prepare = index("prepare melee stats");
        let start = scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap_or_else(|| panic!("{name}: no Start step"));
        assert!(
            load + 1 == ack,
            "{name}: the kickoff precedes its dialog machine"
        );
        assert!(
            matches!(scenario.steps[ack].kind, StepKind::Repeat { .. }),
            "{name}: the dialog machine re-sends every tick"
        );
        assert!(
            matches!(scenario.steps[ack].wait.arm, Proof::QuestDone { name: row } if row == journal),
            "{name}: the boundary is the required quest's journal acknowledgement"
        );
        assert!(
            scenario.steps[ack].wait.budget_ticks > 0,
            "{name}: a preparation that never acknowledges times out"
        );
        assert!(
            load < ack && ack < prepare,
            "{name}: the quest acknowledgement precedes the stat/inventory reset"
        );
        assert!(prepare < start, "{name}: the reset precedes Start");
        // The kickoff sends the native individual-quest command, never
        // the complete-all debugproc whose queued completions leak.
        let mut client = native_seed_client();
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(&client);
        match &scenario.steps[load].kind {
            StepKind::Perform { send } => assert!(send(&mut client, &snapshot)),
            _ => panic!("{name}: the kickoff step is a Perform"),
        }
        assert!(emitted_has(&client, "~quest"), "{name}: native command");
        assert!(
            !emitted_has(&client, "~completequests"),
            "{name}: the complete-all debugproc is not used"
        );
    }
}

#[test]
fn unacknowledged_quest_prereq_fails_the_run_before_the_reset_and_start() {
    use client::io::ServerProt;
    // Late quest traffic keeps arriving (chat + rewards) while the
    // required quest is unacknowledged: the run must fail at the
    // preparation step, bounded, instead of resetting stats and
    // starting the frozen script without its prerequisite.
    let mut scenario = get("green_dragon_special").expect("green_dragon_special is registered");
    let quest = scenario
        .steps
        .iter()
        .position(|step| step.name.starts_with("open the native quest-journal"))
        .expect("the fixture carries the native quest prerequisite");
    scenario.steps.drain(..quest);
    scenario.steps.truncate(2);
    scenario.seed.mainland = false;
    scenario.settings.require_mainland_base = false;

    let (mut client, _) = chat_options_client(&[
        "Complete All Quests.",
        "Reset All Quests.",
        "Select Individual Quest.",
        "Cancel.",
    ]);
    let mut runner = ScenarioRunner::with_world(scenario, None);
    runner.set_scene_settle(Duration::ZERO);
    for tick in 0..700 {
        client.chat_text[0] = format!("Congratulations! Quest complete! {tick}");
        client.bump_gens(ServerProt::MESSAGE_GAME);
        runner.tick(&mut client);
        if matches!(runner.status(), RunnerStatus::Failed(_)) {
            break;
        }
    }
    match runner.status() {
        RunnerStatus::Failed(message) => {
            assert!(
                message.contains("answer the quest-journal dialogs"),
                "the failure names the preparation step: {message}"
            );
            assert!(
                message.contains("quest_done(Lost City)"),
                "the failure names the missing acknowledgement: {message}"
            );
        }
        RunnerStatus::Seeding => panic!("the run never left seeding"),
        RunnerStatus::Running { step, total } => {
            panic!("the run never failed the preparation step ({step}/{total})")
        }
        RunnerStatus::Passed => panic!("an unacknowledged prerequisite cannot pass"),
    }
}

#[test]
fn climbing_boots_variants_seed_complete_death_plateau_map_before_relog_and_start() {
    // Authentic completed Death Plateau retains death_map 8; primary 80
    // alone is incomplete. Both cells share the prep and keep their
    // walk/teleport differences. Readbacks are server Chat arms from
    // getvar, proven by the emitted CLIENT_CHEAT stream — not a client
    // varp snapshot (default 315 = 0 does not establish transmission).
    for (name, use_teleport, pack_coins, deadline) in [
        (
            "climbing_boots",
            false,
            CLIMBING_BOOTS_WALK_PACK_COINS,
            CLIMBING_BOOTS_WALK_DEADLINE,
        ),
        (
            "climbing_boots_teleport",
            true,
            CLIMBING_BOOTS_TELE_PACK_COINS,
            CLIMBING_BOOTS_TELE_DEADLINE,
        ),
    ] {
        let scenario = get(name).unwrap_or_else(|| panic!("{name} is registered"));
        assert_eq!(scenario.settings.start_script, Some("ClimbingBoots"));
        assert_eq!(scenario.settings.deadline, deadline);
        let inject = settings_inject_map(scenario.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("useTeleport"), Some(&Value::Bool(use_teleport)));
        assert_eq!(inject.get("runeStock"), Some(&Value::from(1.0)));

        let index = |needle: &str| {
            scenario
                .steps
                .iter()
                .position(|step| step.name == needle)
                .unwrap_or_else(|| panic!("{name}: missing step {needle}"))
        };
        let primary =
            index("complete Death Plateau primary by the authentic setvar and read it back");
        let map =
            index("retain Death Plateau map progress by the authentic setvar and read it back");
        let journal = index("acknowledge Death Plateau complete before Start");
        let start = scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap_or_else(|| panic!("{name}: no StartScript"));
        let relog = scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::Relog))
            .unwrap_or_else(|| panic!("{name}: seed relog missing"));

        assert!(
            primary + 1 == map,
            "{name}: map progress follows the primary setvar"
        );
        assert!(
                map < relog && relog < journal && journal < start,
                "{name}: both readbacks precede relog, QuestDone, and Start (primary={primary} map={map} relog={relog} journal={journal} start={start})"
            );
        assert_eq!(
            scenario.steps[primary].wait.arm,
            Proof::Chat {
                needle: "get death_equiproom: 80"
            }
        );
        assert_eq!(
            scenario.steps[map].wait.arm,
            Proof::Chat {
                needle: "get death_map: 8"
            }
        );
        assert_eq!(
            scenario.steps[journal].wait.arm,
            Proof::QuestDone {
                name: "Death Plateau"
            }
        );
        assert!(
            scenario.steps[primary].wait.budget_ticks > 0
                && scenario.steps[map].wait.budget_ticks > 0,
            "{name}: missing getvar receipts fail closed on budget"
        );

        let mut snapshot = GameSnapshot::new();
        let mut client = native_seed_client();
        snapshot.rebuild(&client);
        match &scenario.steps[primary].kind {
            StepKind::Perform { send } => assert!(send(&mut client, &snapshot)),
            _ => panic!("{name}: primary Death Plateau step is a Perform"),
        }
        assert!(
            emitted_has(&client, "setvar death_equiproom 80"),
            "{name}: primary setvar is emitted"
        );
        assert!(
            emitted_has(&client, "getvar death_equiproom"),
            "{name}: primary getvar is emitted"
        );
        assert!(
            !emitted_has(&client, "setvar death_map"),
            "{name}: map setvar is a separate step"
        );
        assert!(
            !emitted_has(&client, "give climbing") && !emitted_has(&client, "3105"),
            "{name}: preparation does not grant boots"
        );

        let mut client = native_seed_client();
        snapshot.rebuild(&client);
        match &scenario.steps[map].kind {
            StepKind::Perform { send } => assert!(send(&mut client, &snapshot)),
            _ => panic!("{name}: map progress step is a Perform"),
        }
        assert!(
            emitted_has(&client, "setvar death_map 8"),
            "{name}: map setvar is emitted"
        );
        assert!(
            emitted_has(&client, "getvar death_map"),
            "{name}: map getvar is emitted"
        );
        assert!(
            !emitted_has(&client, "setvar death_equiproom"),
            "{name}: primary setvar stays on its own step"
        );

        let seed = scenario.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::ArrivedNear {
            x: TENZING_DOOR.x,
            z: TENZING_DOOR.z,
            level: TENZING_DOOR.level,
            radius: 4,
        }));
        assert!(seed.contains(&Proof::NpcNameNear {
            name: TENZING_NAME,
            x: TENZING_INSIDE.x,
            z: TENZING_INSIDE.z,
            level: TENZING_INSIDE.level,
            radius: 12,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: CLIMBING_BOOTS_ID,
            count: 0,
        }));
        assert!(seed.contains(&Proof::BankItemIdAtMost {
            id: CLIMBING_BOOTS_ID,
            count: 0,
        }));
        assert!(seed.contains(&Proof::BankItemId {
            id: COINS_ID,
            count: CLIMBING_BOOTS_BANK_TRIPS * pack_coins,
        }));

        let watch_steps = &scenario.steps[start + 1..];
        let watches: Vec<_> = watch_steps.iter().map(|step| step.wait.arm).collect();
        let cast = Proof::ArrivedNear {
            x: FALADOR_TELE_LAND.x,
            z: FALADOR_TELE_LAND.z,
            level: FALADOR_TELE_LAND.level,
            radius: 8,
        };
        let position = |arm: &Proof| watches.iter().position(|w| w == arm);
        let spend = position(&Proof::ItemIdAtMost {
            id: COINS_ID,
            count: pack_coins - 24,
        })
        .unwrap_or_else(|| panic!("{name}: 2-pair spend arm"));
        let back = watch_steps
            .iter()
            .position(|step| step.name == "watch the return to the Falador West bank")
            .unwrap_or_else(|| panic!("{name}: return arm"));
        // The frozen card returns only after `tripComplete`, so the arm
        // after the 2-pair spend spans the rest of the pack. Every sherpa
        // pair is seven pause pages (`death_sherpa.rs2`), each a new chat
        // modal and so at least one dirty snapshot: the arm must allow at
        // least that many before any walking or casting.
        let pairs = pack_coins / 12;
        let trip_arm = &watch_steps[spend + 1];
        assert!(
            trip_arm.wait.budget_ticks > ((pairs - 2) * 7) as u32,
            "{name}: `{}` cannot outlast the remaining {} pairs",
            trip_arm.name,
            pairs - 2
        );
        if use_teleport {
            assert!(
                seed.iter().any(|p| matches!(
                    p,
                    Proof::BankItemId {
                        id: LAW_RUNE_ID,
                        count: 1
                    }
                )),
                "{name}: teleport bank restock Law is acknowledged"
            );
            assert!(
                seed.iter().any(|p| matches!(
                    p,
                    Proof::BankItemId {
                        id: AIR_RUNE_ID,
                        count: 3
                    }
                )),
                "{name}: teleport bank restock Air is acknowledged"
            );
            assert!(
                seed.iter().any(|p| matches!(
                    p,
                    Proof::BankItemId {
                        id: WATER_RUNE_ID,
                        count: 1
                    }
                )),
                "{name}: teleport bank restock Water is acknowledged"
            );
            let landed = position(&cast).unwrap_or_else(|| panic!("{name}: cast arm"));
            // The runner baselines `StatXpGain` when its first arm of that
            // shape begins and the proof reuses it. The proof must be that
            // arm, starting after the 2-pair spend and before the cast, or
            // its baseline is taken after the cycle and it waits for a
            // second full trip (live bce85f2df: 720s deadline mid-trip 2).
            assert_eq!(
                scenario.proof,
                Proof::StatXpGain {
                    id: MAGIC_STAT,
                    min: 1,
                }
            );
            let xp = position(&scenario.proof)
                .unwrap_or_else(|| panic!("{name}: the proof's XP arm is watched"));
            assert_eq!(
                (spend + 1, xp + 1, landed + 1),
                (xp, landed, back),
                "{name}: spend, cast XP, landing, then the bank return"
            );
        } else {
            assert!(
                !seed.iter().any(|p| matches!(
                    p,
                    Proof::BankItemId {
                        id: LAW_RUNE_ID,
                        ..
                    } | Proof::BankItemId {
                        id: AIR_RUNE_ID,
                        ..
                    } | Proof::BankItemId {
                        id: WATER_RUNE_ID,
                        ..
                    }
                )),
                "{name}: walking cell does not seed teleport runes"
            );
            assert_eq!(
                (position(&cast), spend + 1),
                (None, back),
                "{name}: walking cell returns without a Falador cast"
            );
            assert_eq!(
                scenario.proof,
                Proof::ItemId {
                    id: CLIMBING_BOOTS_ID,
                    count: 2,
                }
            );
        }
    }
}

#[test]
fn fightback_fixtures_acknowledge_strength_style_before_start() {
    assert!(is_aggressive_combat_style("(Aggressive)"));
    assert!(is_aggressive_combat_style("Aggressive"));
    assert!(!is_aggressive_combat_style("(Accurate)"));
    for name in ["ardy_cakes_fight", "ardy_thiever_fight"] {
        let scenario = get(name).expect("FightBack fixture is registered");
        let style = scenario
            .steps
            .iter()
            .position(|step| {
                step.name
                    .starts_with("select and acknowledge native Strength")
            })
            .expect("FightBack fixture selects the native Strength style");
        let start = scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .expect("FightBack fixture has a StartScript step");
        assert!(style < start, "{name}: style selection precedes Start");
        assert_eq!(
            scenario.steps[style].wait.arm,
            Proof::VarpExact { id: 43, value: 1 }
        );
    }
    for name in ["ardy_cakes", "ardy_thiever", "ardy_thiever_knight"] {
        let scenario = get(name).expect("unaffected fixture is registered");
        assert!(!scenario.steps.iter().any(|step| {
            step.name
                .starts_with("select and acknowledge native Strength")
        }));
    }
}

/// ShopBuyout Aemad (nonstackable vial), Aubury (stackable fire rune),
/// Lowe/Hickton (stackable bronze arrow), Harry (stackable fishing bait),
/// Betty (stackable fire rune), Gerrant (stackable feather): selected
/// buyItems only, budget>perTrip for resume, exact operable booth seed,
/// unseeded product proof order through bank/return/resume.
#[test]
fn shop_buyout_variants_prove_product_bank_and_resumed_purchase() {
    for (
        name,
        product_id,
        approach,
        booth,
        booth_id,
        keeper,
        stand,
        budget,
        per_trip,
        item,
        shop,
    ) in [
        (
            "shop_buyout",
            VIAL_OF_WATER_ID,
            AEMAD_BANK_APPROACH,
            AEMAD_BANK_BOOTH,
            AEMAD_BANK_BOOTH_ID,
            "Aemad",
            AEMAD_STAND,
            SHOP_BUYOUT_AEMAD_BUDGET_GP,
            SHOP_BUYOUT_AEMAD_PER_TRIP_GP,
            SHOP_BUYOUT_AEMAD_ITEM,
            SHOP_BUYOUT_AEMAD_LABEL,
        ),
        (
            "shop_buyout_aubury",
            FIRE_RUNE_ID,
            VARROCK_EAST_BANK_APPROACH,
            VARROCK_EAST_BANK_BOOTH,
            VARROCK_EAST_BANK_BOOTH_ID,
            "Aubury",
            AUBURY_STAND,
            SHOP_BUYOUT_AUBURY_BUDGET_GP,
            SHOP_BUYOUT_AUBURY_PER_TRIP_GP,
            SHOP_BUYOUT_AUBURY_ITEM,
            SHOP_BUYOUT_AUBURY_LABEL,
        ),
        (
            "shop_buyout_lowe",
            BRONZE_ARROW_ID,
            VARROCK_EAST_BANK_APPROACH,
            VARROCK_EAST_BANK_BOOTH,
            VARROCK_EAST_BANK_BOOTH_ID,
            "Lowe",
            LOWE_STAND,
            SHOP_BUYOUT_LOWE_BUDGET_GP,
            SHOP_BUYOUT_LOWE_PER_TRIP_GP,
            SHOP_BUYOUT_LOWE_ITEM,
            SHOP_BUYOUT_LOWE_LABEL,
        ),
        (
            "shop_buyout_hickton",
            BRONZE_ARROW_ID,
            CATHERBY_BANK_APPROACH,
            CATHERBY_BANK_BOOTH,
            CATHERBY_BANK_BOOTH_ID,
            "Hickton",
            HICKTON_STAND,
            SHOP_BUYOUT_HICKTON_BUDGET_GP,
            SHOP_BUYOUT_HICKTON_PER_TRIP_GP,
            SHOP_BUYOUT_HICKTON_ITEM,
            SHOP_BUYOUT_HICKTON_LABEL,
        ),
        (
            "shop_buyout_harry",
            FISHING_BAIT_ID,
            CATHERBY_BANK_APPROACH,
            CATHERBY_BANK_BOOTH,
            CATHERBY_BANK_BOOTH_ID,
            "Harry",
            HARRY_STAND,
            SHOP_BUYOUT_HARRY_BUDGET_GP,
            SHOP_BUYOUT_HARRY_PER_TRIP_GP,
            SHOP_BUYOUT_HARRY_ITEM,
            SHOP_BUYOUT_HARRY_LABEL,
        ),
        (
            "shop_buyout_betty",
            FIRE_RUNE_ID,
            FALADOR_WEST_BANK_APPROACH,
            FALADOR_WEST_BANK_BOOTH,
            FALADOR_WEST_BANK_BOOTH_ID,
            "Betty",
            BETTY_STAND,
            SHOP_BUYOUT_BETTY_BUDGET_GP,
            SHOP_BUYOUT_BETTY_PER_TRIP_GP,
            SHOP_BUYOUT_BETTY_ITEM,
            SHOP_BUYOUT_BETTY_LABEL,
        ),
        (
            "shop_buyout_gerrant",
            FEATHER_ID,
            DRAYNOR_BANK_APPROACH,
            DRAYNOR_BANK_BOOTH,
            DRAYNOR_BANK_BOOTH_ID,
            "Gerrant",
            GERRANT_STAND,
            SHOP_BUYOUT_GERRANT_BUDGET_GP,
            SHOP_BUYOUT_GERRANT_PER_TRIP_GP,
            SHOP_BUYOUT_GERRANT_ITEM,
            SHOP_BUYOUT_GERRANT_LABEL,
        ),
        (
            "shop_buyout_bob",
            STEEL_AXE_ID,
            DRAYNOR_BANK_APPROACH,
            DRAYNOR_BANK_BOOTH,
            DRAYNOR_BANK_BOOTH_ID,
            "Bob",
            BOB_STAND,
            SHOP_BUYOUT_BOB_BUDGET_GP,
            SHOP_BUYOUT_BOB_PER_TRIP_GP,
            SHOP_BUYOUT_BOB_ITEM,
            SHOP_BUYOUT_BOB_LABEL,
        ),
        (
            "shop_buyout_nurmof",
            IRON_PICKAXE_ID,
            FALADOR_EAST_BANK,
            FALADOR_EAST_BOOTH,
            2213,
            "Nurmof",
            NURMOF_STAND,
            SHOP_BUYOUT_NURMOF_BUDGET_GP,
            SHOP_BUYOUT_NURMOF_PER_TRIP_GP,
            SHOP_BUYOUT_NURMOF_ITEM,
            SHOP_BUYOUT_NURMOF_LABEL,
        ),
        (
            "shop_buyout_magic",
            BLOOD_RUNE_ID,
            YANILLE_BANK_APPROACH,
            YANILLE_BANK_BOOTH,
            YANILLE_BANK_BOOTH_ID,
            "Magic Store owner",
            MAGIC_STORE_STAND,
            SHOP_BUYOUT_MAGIC_BUDGET_GP,
            SHOP_BUYOUT_MAGIC_PER_TRIP_GP,
            SHOP_BUYOUT_MAGIC_ITEM,
            SHOP_BUYOUT_MAGIC_LABEL,
        ),
    ] {
        let scenario = get(name).unwrap_or_else(|| panic!("{name} is registered"));
        assert_eq!(scenario.settings.start_script, Some("ShopBuyout"));
        let expected_deadline = match name {
            "shop_buyout_betty" | "shop_buyout_bob" => SHOP_BUYOUT_BETTY_DEADLINE,
            "shop_buyout_gerrant" => SHOP_BUYOUT_GERRANT_DEADLINE,
            "shop_buyout_nurmof" => Duration::from_secs(450),
            _ => SCRIPT_GOLD_DEADLINE,
        };
        assert_eq!(
            scenario.settings.deadline, expected_deadline,
            "{name}: deadline matches the scoped observation budget"
        );
        assert_eq!(
            scenario.proof,
            Proof::ItemId {
                id: product_id,
                count: 1
            },
            "{name}: terminal proof is purchased product ≥1"
        );

        let inject = settings_inject_map(scenario.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("shop"),
            Some(&Value::String(shop.into())),
            "{name}: shop label is the named preset"
        );
        assert_eq!(
            inject.get("budgetGp"),
            Some(&Value::Number(
                serde_json::Number::from_f64(budget).unwrap()
            ))
        );
        assert_eq!(
            inject.get("perTripGp"),
            Some(&Value::Number(
                serde_json::Number::from_f64(per_trip).unwrap()
            ))
        );
        assert!(
            budget > per_trip,
            "{name}: budget must exceed perTrip so resume is not budget-spent Stop"
        );
        assert_eq!(
            inject.get("stopFloorGp"),
            Some(&Value::Number(serde_json::Number::from_f64(0.0).unwrap()))
        );
        assert_eq!(
            inject.get("buyItems"),
            Some(&Value::Array(vec![Value::String(item.into())])),
            "{name}: buyItems is the single selected product, not empty-all-stock"
        );

        let start = scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .expect("StartScript");
        let seed: Vec<_> = scenario.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect();
        assert!(
            seed.contains(&Proof::ArrivedNear {
                x: approach.x,
                z: approach.z,
                level: approach.level,
                radius: 4,
            }),
            "{name}: seed stands on the operable booth approach"
        );
        assert!(
            seed.contains(&Proof::LocActionNear {
                id: booth_id,
                x: booth.x,
                z: booth.z,
                level: booth.level,
                radius: 0,
                action: "Use-quickly",
                present: true,
            }),
            "{name}: seed acknowledges exact open booth Use-quickly"
        );
        assert_eq!(booth_id, 2213, "{name}: open booth id is 2213");
        assert!(
                scenario.steps[..start].iter().any(|step| {
                    step.name
                        == "acknowledge exact shop bank booth identity and Use-quickly action before bank send"
                }),
                "{name}: readiness step is present"
            );
        let open = scenario.steps[..start]
            .iter()
            .find(|step| {
                step.name == "open the exact named shop bank booth for the coin seed deposit"
            })
            .expect("exact booth open");
        assert!(
            matches!(open.kind, StepKind::Repeat { .. }),
            "{name}: open must Repeat exact booth, not one-shot nearest"
        );
        assert!(
            seed.contains(&Proof::BankItemId {
                id: COINS_ID,
                count: SHOP_BUYOUT_COIN_SEED,
            }),
            "{name}: exact coin seed banked"
        );
        assert!(
            seed.contains(&Proof::BankItemIdAtMost {
                id: product_id,
                count: 0,
            }),
            "{name}: product unseeded in bank"
        );
        assert!(
            seed.contains(&Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            }),
            "{name}: product unseeded in pack"
        );
        assert!(
            seed.contains(&Proof::NpcNameNear {
                name: keeper,
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 12,
            }),
            "{name}: keeper acknowledged before Start"
        );

        let watches: Vec<_> = scenario.steps[start + 1..]
            .iter()
            .map(|step| (step.name, step.wait.arm))
            .collect();
        let expected = [
            (
                "watch unseeded purchased product in pack after Start",
                Proof::ItemId {
                    id: product_id,
                    count: 1,
                },
            ),
            (
                "watch purchased product enter a fresh bank",
                Proof::BankItemId {
                    id: product_id,
                    count: 1,
                },
            ),
            (
                "watch the pack empty of product after deposit",
                Proof::ItemIdAtMost {
                    id: product_id,
                    count: 0,
                },
            ),
            (
                "watch the buyout bank close after deposit",
                Proof::BankClosed,
            ),
            (
                "watch return to the named shop after banking",
                Proof::NpcNameNear {
                    name: keeper,
                    x: stand.x,
                    z: stand.z,
                    level: stand.level,
                    radius: 12,
                },
            ),
            (
                "watch further purchased product after return",
                Proof::ItemId {
                    id: product_id,
                    count: 1,
                },
            ),
        ];
        assert_eq!(
            watches, expected,
            "{name}: post-Start proof order is product→bank→empty→close→shop→resume"
        );
        for (_, arm) in &watches {
            // No manufactured coin-only success arms after Start.
            assert!(
                !matches!(
                    arm,
                    Proof::ItemId { id: COINS_ID, .. } | Proof::ItemIdAtMost { id: COINS_ID, .. }
                ),
                "{name}: coins are not the primary post-Start contract"
            );
        }
    }

    // Distinct honest representatives: nonstackable vial vs stackable fire rune
    // vs stackable bronze arrow vs stackable fishing bait.
    assert_ne!(VIAL_OF_WATER_ID, FIRE_RUNE_ID);
    assert_ne!(FIRE_RUNE_ID, BRONZE_ARROW_ID);
    assert_ne!(BRONZE_ARROW_ID, FISHING_BAIT_ID);
    assert_eq!(VIAL_OF_WATER_ID, 227);
    assert_eq!(FIRE_RUNE_ID, 554);
    assert_eq!(BRONZE_ARROW_ID, 882);
    assert_eq!(FISHING_BAIT_ID, 313);
    assert_eq!(
        AEMAD_BANK_BOOTH,
        WorldTile {
            x: 2656,
            z: 3283,
            level: 0
        }
    );
    assert_eq!(
        VARROCK_EAST_BANK_BOOTH,
        WorldTile {
            x: 3253,
            z: 3419,
            level: 0
        }
    );
    assert_eq!(
        CATHERBY_BANK_BOOTH,
        WorldTile {
            x: 2809,
            z: 3442,
            level: 0
        }
    );
    assert_eq!(
        AEMAD_BANK_APPROACH,
        WorldTile {
            x: 2655,
            z: 3283,
            level: 0
        }
    );
    assert_eq!(
        VARROCK_EAST_BANK_APPROACH,
        WorldTile {
            x: 3253,
            z: 3420,
            level: 0
        }
    );
    assert_eq!(
        CATHERBY_BANK_APPROACH,
        WorldTile {
            x: 2809,
            z: 3441,
            level: 0
        }
    );
}

/// Betty-only long Falador West travel budgets: first purchase covers
/// initial withdraw round-trip, deposit/return cover post-buy legs;
/// empty/close/further and all other presets stay ordinary 150/180s.
/// Predicates and proof order are unchanged (see chain test).
#[test]
fn shop_buyout_betty_timing_covers_four_bank_travel_legs() {
    let betty = get("shop_buyout_betty").expect("shop_buyout_betty");
    assert_eq!(
        betty.settings.deadline, SHOP_BUYOUT_BETTY_DEADLINE,
        "Betty wall covers four Port Sarim↔Falador West legs"
    );
    let watch = |name: &str| {
        betty
            .steps
            .iter()
            .find(|step| step.name == name)
            .unwrap_or_else(|| panic!("betty has {name}"))
    };
    assert_eq!(
        watch("watch unseeded purchased product in pack after Start")
            .wait
            .budget_ticks,
        SHOP_BUYOUT_BETTY_FIRST_PURCHASE_WATCH_TICKS,
        "first purchase allows a bounded trial beyond the observed 150-dirty failure"
    );
    assert_eq!(
        watch("watch purchased product enter a fresh bank")
            .wait
            .budget_ticks,
        SHOP_BUYOUT_BETTY_DEPOSIT_WATCH_TICKS,
        "deposit dirty-budget covers post-buy shop→bank leg"
    );
    assert_eq!(
        watch("watch the pack empty of product after deposit")
            .wait
            .budget_ticks,
        SCRIPT_GOLD_WATCH_TICKS,
        "empty-pack arm is not loosened"
    );
    assert_eq!(
        watch("watch the buyout bank close after deposit")
            .wait
            .budget_ticks,
        SCRIPT_GOLD_WATCH_TICKS,
        "close arm is not loosened"
    );
    assert_eq!(
        watch("watch return to the named shop after banking")
            .wait
            .budget_ticks,
        SHOP_BUYOUT_BETTY_RETURN_WATCH_TICKS,
        "return dirty-budget covers bank→shop leg only"
    );
    assert_eq!(
        watch("watch further purchased product after return")
            .wait
            .budget_ticks,
        SCRIPT_GOLD_WATCH_TICKS,
        "further-purchase arm is not loosened"
    );
    // West bank geometry preserved (not East shortcut).
    assert_eq!(
        FALADOR_WEST_BANK_APPROACH,
        WorldTile {
            x: 2946,
            z: 3368,
            level: 0
        }
    );
    assert_eq!(
        BETTY_STAND,
        WorldTile {
            x: 3012,
            z: 3258,
            level: 0
        }
    );

    for name in [
        "shop_buyout",
        "shop_buyout_aubury",
        "shop_buyout_lowe",
        "shop_buyout_hickton",
        "shop_buyout_harry",
        "shop_buyout_magic",
        "shop_buyout_lundail",
        "shop_buyout_fernahei",
    ] {
        let s = get(name).unwrap_or_else(|| panic!("{name} registered"));
        assert_eq!(
            s.settings.deadline, SCRIPT_GOLD_DEADLINE,
            "{name}: short-route deadline stays 180s"
        );
        for step in s.steps.iter().filter(|step| {
            step.name.starts_with("watch unseeded purchased")
                || step.name.starts_with("watch purchased product enter")
                || step.name.starts_with("watch return to the named shop")
                || step.name.starts_with("watch further purchased")
                || step.name.starts_with("watch the pack empty")
                || step.name.starts_with("watch the buyout bank close")
        }) {
            assert_eq!(
                step.wait.budget_ticks, SCRIPT_GOLD_WATCH_TICKS,
                "{name}: {} stays ordinary gold watch",
                step.name
            );
        }
    }
}

/// Gerrant Draynor route: live7zpi0g mid-return + livexjav5j first-purchase
/// fails bound the four-leg dirty budgets and 390s wall (Betty unchanged).
#[test]
fn shop_buyout_gerrant_timing_covers_four_bank_travel_legs() {
    let gerrant = get("shop_buyout_gerrant").expect("shop_buyout_gerrant");
    assert_eq!(
        gerrant.settings.deadline, SHOP_BUYOUT_GERRANT_DEADLINE,
        "390s wall covers mid-return 180s fail plus depleted return trial margin"
    );
    let watch = |name: &str| {
        gerrant
            .steps
            .iter()
            .find(|step| step.name == name)
            .unwrap_or_else(|| panic!("gerrant has {name}"))
    };
    assert_eq!(
            watch("watch unseeded purchased product in pack after Start")
                .wait
                .budget_ticks,
            SHOP_BUYOUT_GERRANT_FIRST_PURCHASE_WATCH_TICKS,
            "first purchase covers two Draynor legs + withdraw + buy beyond livexjav5j 150-dirty arm fail"
        );
    assert_eq!(
        watch("watch purchased product enter a fresh bank")
            .wait
            .budget_ticks,
        SCRIPT_GOLD_WATCH_TICKS,
        "deposit arm: live7zpi0g measured pass at 150 dirty"
    );
    assert_eq!(
        watch("watch the pack empty of product after deposit")
            .wait
            .budget_ticks,
        SCRIPT_GOLD_WATCH_TICKS,
        "empty-pack arm is not loosened"
    );
    assert_eq!(
        watch("watch the buyout bank close after deposit")
            .wait
            .budget_ticks,
        SCRIPT_GOLD_WATCH_TICKS,
        "close arm is not loosened"
    );
    assert_eq!(
        watch("watch return to the named shop after banking")
            .wait
            .budget_ticks,
        SHOP_BUYOUT_GERRANT_RETURN_WATCH_TICKS,
        "return widened for Draynor→Gerrant depleted-energy leg"
    );
    assert_eq!(
        watch("watch further purchased product after return")
            .wait
            .budget_ticks,
        SCRIPT_GOLD_WATCH_TICKS,
        "further-purchase arm is not loosened"
    );
    assert_eq!(
        GERRANT_STAND,
        WorldTile {
            x: 3013,
            z: 3224,
            level: 0
        }
    );
    assert_eq!(
        DRAYNOR_BANK_APPROACH,
        WorldTile {
            x: 3092,
            z: 3243,
            level: 0
        }
    );
}

/// Lundail/Fernahei seed through the named teller, not a booth. Shared
/// post-Start proof order stays product→bank→empty→close→shop→resume.
#[test]
fn shop_buyout_npc_teller_variants_seed_through_named_dialog() {
    for (name, product_id, approach, banker, keeper, stand, item, shop, quest) in [
        (
            "shop_buyout_lundail",
            FIRE_RUNE_ID,
            GUNDAI_BANK_APPROACH,
            "Gundai",
            "Lundail",
            LUNDAIL_STAND,
            SHOP_BUYOUT_LUNDAIL_ITEM,
            SHOP_BUYOUT_LUNDAIL_LABEL,
            None,
        ),
        (
            "shop_buyout_fernahei",
            FEATHER_ID,
            SHILO_BANK_APPROACH,
            "Banker",
            "Fernahei",
            FERNAHEI_STAND,
            SHOP_BUYOUT_FERNAHEI_ITEM,
            SHOP_BUYOUT_FERNAHEI_LABEL,
            Some("Shilo Village"),
        ),
    ] {
        let scenario = get(name).unwrap_or_else(|| panic!("{name} is registered"));
        assert_eq!(scenario.settings.start_script, Some("ShopBuyout"));
        assert_eq!(scenario.settings.deadline, SCRIPT_GOLD_DEADLINE);
        assert_eq!(
            scenario.proof,
            Proof::ItemId {
                id: product_id,
                count: 1
            }
        );
        let inject = settings_inject_map(scenario.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("shop"), Some(&Value::String(shop.into())));
        assert_eq!(
            inject.get("buyItems"),
            Some(&Value::Array(vec![Value::String(item.into())]))
        );
        assert_eq!(
            inject.get("budgetGp"),
            Some(&Value::Number(
                serde_json::Number::from_f64(1500.0).unwrap()
            ))
        );
        assert_eq!(
            inject.get("perTripGp"),
            Some(&Value::Number(serde_json::Number::from_f64(500.0).unwrap()))
        );

        let start = scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .expect("StartScript");
        let seed: Vec<_> = scenario.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect();
        assert!(
            seed.contains(&Proof::ArrivedNear {
                x: approach.x,
                z: approach.z,
                level: approach.level,
                radius: 4,
            }),
            "{name}: seed stands on the teller approach"
        );
        assert!(
            seed.contains(&Proof::NpcNameNear {
                name: banker,
                x: approach.x,
                z: approach.z,
                level: approach.level,
                radius: 12,
            }),
            "{name}: seed acknowledges the named teller"
        );
        assert!(
            scenario.steps[..start].iter().any(|step| {
                step.name == "open the exact named shop banker for the coin seed deposit"
            }),
            "{name}: NPC teller open is present"
        );
        assert!(
            scenario.steps[..start].iter().all(|step| {
                step.name != "open the exact named shop bank booth for the coin seed deposit"
            }),
            "{name}: booth open is not used"
        );
        if let Some(journal) = quest {
            assert!(
                seed.contains(&Proof::QuestDone { name: journal }),
                "{name}: Shilo Village journal is acknowledged before Start"
            );
            let journal_at = scenario
                    .steps
                    .iter()
                    .position(|step| {
                        matches!(step.wait.arm, Proof::QuestDone { name: row } if row == journal)
                    })
                    .expect("quest ack");
            let coin_at = scenario
                    .steps
                    .iter()
                    .position(|step| {
                        step.name
                            == "seed stackable coins and stand at the operable shop bank approach before Start"
                    })
                    .expect("coin seed");
            assert!(
                journal_at < coin_at && coin_at < start,
                "{name}: quest ack precedes coin seed and Start"
            );
        } else {
            assert!(
                !seed
                    .iter()
                    .any(|arm| matches!(arm, Proof::QuestDone { .. })),
                "{name}: cellar teller does not invent a quest gate"
            );
        }
        assert!(
            seed.contains(&Proof::NpcNameNear {
                name: keeper,
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 12,
            }),
            "{name}: keeper acknowledged before Start"
        );

        let watches: Vec<_> = scenario.steps[start + 1..]
            .iter()
            .map(|step| (step.name, step.wait.arm))
            .collect();
        assert_eq!(
            watches,
            [
                (
                    "watch unseeded purchased product in pack after Start",
                    Proof::ItemId {
                        id: product_id,
                        count: 1,
                    },
                ),
                (
                    "watch purchased product enter a fresh bank",
                    Proof::BankItemId {
                        id: product_id,
                        count: 1,
                    },
                ),
                (
                    "watch the pack empty of product after deposit",
                    Proof::ItemIdAtMost {
                        id: product_id,
                        count: 0,
                    },
                ),
                (
                    "watch the buyout bank close after deposit",
                    Proof::BankClosed,
                ),
                (
                    "watch return to the named shop after banking",
                    Proof::NpcNameNear {
                        name: keeper,
                        x: stand.x,
                        z: stand.z,
                        level: stand.level,
                        radius: 12,
                    },
                ),
                (
                    "watch further purchased product after return",
                    Proof::ItemId {
                        id: product_id,
                        count: 1,
                    },
                ),
            ]
        );
    }
    assert_eq!(
        GUNDAI_BANK_APPROACH,
        WorldTile {
            x: 2533,
            z: 4714,
            level: 0
        }
    );
    assert_eq!(
        SHILO_BANK_APPROACH,
        WorldTile {
            x: 2852,
            z: 2954,
            level: 0
        }
    );
}

fn plant_named_banker(client: &mut Client, name: &str, local_x: i32, local_z: i32) {
    use client::client::ClientNpc;
    use client::config::NpcType;
    let cache = std::sync::Arc::get_mut(&mut client.cache).expect("sole cache owner");
    if cache.npcs.is_empty() {
        cache.npcs.push(NpcType::default());
    }
    cache.npcs[0].name = name.into();
    cache.npcs[0].op = vec![Some("Talk-to".into()), None, None, None, None];
    let mut npc = ClientNpc {
        r#type: Some(0),
        ..Default::default()
    };
    npc.entity.x = local_x * 128;
    npc.entity.z = local_z * 128;
    client.npc[1] = Some(Box::new(npc));
    client.npc_ids[0] = 1;
    client.npc_count = 1;
}

fn plant_latched_continue_chat(client: &mut Client) {
    use client::config::if_type::{ButtonType, IfType, IfTypeMut};
    use client::io::ServerProt;
    client.set_iface(
        968,
        IfType {
            id: 968,
            layer_id: 968,
            children: Some(vec![972]),
            ..Default::default()
        },
    );
    client.set_iface(
        972,
        IfType {
            id: 972,
            layer_id: 968,
            ..Default::default()
        },
    );
    client.set_iface_mut(
        972,
        IfTypeMut {
            button_type: ButtonType::BUTTON_CONTINUE,
            text: "Click here to continue".into(),
            ..Default::default()
        },
    );
    client.chat_modal_id = 968;
    client.resumed_pause_button = true;
    client.bump_gens(ServerProt::IF_OPENCHAT);
}

fn rebuild_npc_chat_snapshot(client: &mut Client) -> GameSnapshot {
    use client::io::ServerProt;
    for prot in [
        ServerProt::PLAYER_INFO,
        ServerProt::NPC_INFO,
        ServerProt::IF_OPENCHAT,
    ] {
        client.bump_gens(prot);
    }
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(client);
    snapshot
}

/// LIVE livemoj7nw / livevpttcg: Repeat re-Talk-to while the first
/// continue page is up (`chat_continue_component_id == -1` because the
/// pause latch hid it, chat modal still 968, "I can't reach that!").
/// The opener must Talk-to once, then wait/continue/choose — never
/// restart the conversation.
#[test]
fn shop_buyout_npc_opener_does_not_retalk_while_dialog_is_open() {
    use client::io::ServerProt;

    let approach = WorldTile {
        x: 3220,
        z: 3220,
        level: 0,
    };
    let step = shop_buyout_open_npc_bank(
        "open the exact named shop banker for the coin seed deposit",
        Proof::BankItemIdAtMost {
            id: COINS_ID,
            count: 0,
        },
        "Gundai",
        approach,
        "Talk-to",
        "Cool, I'd like to access my bank account please.",
    );
    let StepKind::Repeat { send } = &step.kind else {
        panic!("teller open must Repeat");
    };
    let mut client = native_seed_client();
    let _peer = attach_loopback(&mut client);
    plant_named_banker(&mut client, "Gundai", 20, 21);
    let snapshot = rebuild_npc_chat_snapshot(&mut client);
    assert!(
        snapshot
            .npcs()
            .iter()
            .any(|npc| npc.name.as_deref() == Some("Gundai")),
        "Gundai must be on the snapshot"
    );

    assert!(send(&mut client, &snapshot), "first Talk-to must send");
    let after_talk = client.out.pos;
    assert!(after_talk > 0, "Talk-to must emit an npc op");

    assert!(
        send(&mut client, &snapshot),
        "a second poll before chat lands must wait, not fail"
    );
    assert_eq!(
        client.out.pos, after_talk,
        "Repeat must not Talk-to again before the chat modal appears"
    );

    plant_latched_continue_chat(&mut client);
    let latched = rebuild_npc_chat_snapshot(&mut client);
    assert_eq!(latched.modals().chat, 968);
    assert_eq!(
        latched.chat_continue_component_id(),
        -1,
        "LIVE fail shape: continue widget present, pause latch hides the id"
    );
    assert!(
        send(&mut client, &latched),
        "latched continue must soft-wait"
    );
    assert_eq!(
        client.out.pos, after_talk,
        "latched continue must not Talk-to or walk the teller"
    );

    client.resumed_pause_button = false;
    client.bump_gens(ServerProt::IF_OPENCHAT);
    let mut live_continue = GameSnapshot::new();
    live_continue.rebuild(&client);
    assert_eq!(live_continue.chat_continue_component_id(), 972);
    assert!(
        send(&mut client, &live_continue),
        "visible continue must send"
    );
    assert!(
        client.out.pos > after_talk,
        "visible continue must emit PAUSE_BUTTON, not stall"
    );
    let after_continue = client.out.pos;
    assert!(send(&mut client, &live_continue));
    assert_eq!(
        client.out.pos, after_continue,
        "do not re-continue the same page"
    );
}

fn plant_open_unloaded_bank(client: &mut Client) {
    use api::snapshot::Family;
    use client::config::if_type::{ComponentType, IfType, IfTypeMut};
    use client::io::{Packet, ServerProt};

    client.set_iface(
        600,
        IfType {
            id: 600,
            layer_id: 600,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![601]),
            ..Default::default()
        },
    );
    client.set_iface(
        601,
        IfType {
            id: 601,
            layer_id: 600,
            r#type: ComponentType::TYPE_INV,
            iop: [Some("Withdraw 1".into()), None, None, None, None],
            ..Default::default()
        },
    );
    client.set_iface_mut(
        601,
        IfTypeMut {
            link_obj_type: Some(vec![0]),
            link_obj_number: Some(vec![0]),
            ..Default::default()
        },
    );
    let mut open = Packet::new(vec![2, 88]);
    client.handle_packet(ServerProt::IF_OPENMAIN, &mut open);
    let mut snapshot = GameSnapshot::new();
    assert!(snapshot.rebuild_family(client, Family::Bank));
    assert!(snapshot.bank_component_id() >= 0);
    assert!(!snapshot.bank_loaded());
}

fn plant_loaded_empty_bank(client: &mut Client) {
    use client::io::{Packet, ServerProt};

    plant_open_unloaded_bank(client);
    let mut full = Packet::new(vec![2, 89, 0]);
    client.handle_packet(ServerProt::UPDATE_INV_FULL, &mut full);
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(client);
    assert!(snapshot.bank_component_id() >= 0);
    assert!(snapshot.bank_loaded());
    assert!(
        Proof::BankItemIdAtMost {
            id: COINS_ID,
            count: 0,
        }
        .check(&snapshot, None),
        "loaded empty bank must satisfy the opener arm"
    );
}

fn close_chat_modal(client: &mut Client) {
    use client::io::ServerProt;
    client.chat_modal_id = -1;
    client.resumed_pause_button = false;
    client.bump_gens(ServerProt::IF_OPENCHAT);
}

/// LIVE livef0d8vo / livezb611z: opener arm passed (bank opened,
/// generation 2) then deposit rejected with the first dialog up and
/// bank closed. Repeat send runs before arm.check; after the bank
/// choice the chat closes while the bank is only opening
/// (`component >= 0`, `!loaded`) and the helper reset/Talk-to'd.
#[test]
fn shop_buyout_npc_opener_does_not_retalk_after_choice_before_bank_loaded() {
    use client::io::ServerProt;

    let approach = WorldTile {
        x: 3220,
        z: 3220,
        level: 0,
    };
    let opener = shop_buyout_open_npc_bank(
        "open the exact named shop banker for the coin seed deposit",
        Proof::BankItemIdAtMost {
            id: COINS_ID,
            count: 0,
        },
        "Gundai",
        approach,
        "Talk-to",
        "Cool, I'd like to access my bank account please.",
    );
    let deposit = native_bank_deposit(
        "deposit the coin seed through the bank window",
        vec![NativeSeed {
            unnoted_id: COINS_ID,
            debug_alias: "coins",
            note_alias: None,
            quantity: SHOP_BUYOUT_COIN_SEED,
            note_id: None,
        }],
    )
    .remove(0);
    let scenario = Scenario {
        name: "shop_buyout_npc_open_to_deposit",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![opener, deposit],
        proof: Proof::BankItemId {
            id: COINS_ID,
            count: SHOP_BUYOUT_COIN_SEED,
        },
        companions: vec![],
        settings: ScenarioSettings::default(),
    };
    let mut runner = ScenarioRunner::with_world(scenario, None);
    runner.set_scene_settle(Duration::ZERO);

    let mut client = native_seed_client();
    let _peer = attach_loopback(&mut client);
    plant_named_banker(&mut client, "Gundai", 20, 21);
    for prot in [ServerProt::PLAYER_INFO, ServerProt::NPC_INFO] {
        client.bump_gens(prot);
    }

    runner.tick(&mut client);
    assert_eq!(
        runner.status(),
        RunnerStatus::Running { step: 0, total: 2 },
        "first poll Talk-to's and stays on the opener"
    );
    let after_talk = client.out.pos;
    assert!(after_talk > 0, "opener must Talk-to once");

    set_chat_dialog(
        &mut client,
        &["Cool, I'd like to access my bank account please."],
    );
    runner.tick(&mut client);
    assert_eq!(runner.status(), RunnerStatus::Running { step: 0, total: 2 });
    let after_choice = client.out.pos;
    assert!(
        after_choice > after_talk,
        "opener must answer the exact bank-access choice"
    );

    close_chat_modal(&mut client);
    plant_open_unloaded_bank(&mut client);
    runner.tick(&mut client);
    assert_eq!(
        runner.status(),
        RunnerStatus::Running { step: 0, total: 2 },
        "partial bank load is not fresh_bank; stay on the opener"
    );
    assert_eq!(
        client.out.pos, after_choice,
        "must not queue Talk-to after the choice while the bank is only opening"
    );

    plant_loaded_empty_bank(&mut client);
    plant_bank_side(&mut client, COINS_ID, SHOP_BUYOUT_COIN_SEED);
    client.bump_gens(ServerProt::UPDATE_INV_FULL);
    runner.tick(&mut client);
    assert_eq!(
        runner.status(),
        RunnerStatus::Running { step: 1, total: 2 },
        "loaded empty bank advances the opener; Repeat send already ran"
    );
    assert_eq!(
        client.out.pos, after_choice,
        "must not Talk-to on the send-before-arm poll that proves the bank open"
    );

    runner.tick(&mut client);
    match runner.status() {
        RunnerStatus::Failed(message) => {
            panic!("deposit must stay ready after a stable opener, got {message}")
        }
        RunnerStatus::Running { step, .. } => {
            assert_eq!(step, 1, "deposit stays armed until coins land in bank")
        }
        other => panic!("deposit poll must not finish the run, got {other:?}"),
    }
    assert!(
        client.out.pos > after_choice,
        "deposit must emit from bank_side, not reject a closed bank"
    );
}

#[test]
fn ranging_guild_round_orders_fee_shot_payout_and_further_round() {
    // Stock 289 / frozen rangingguild-live.ts --phase round, plus a second
    // 200-coin enter (live full's coinsPerTrip=400 funds two rounds).
    const COINS: i32 = 995;
    const MAGIC_SHORTBOW: i32 = 861;
    const ARCHERY_TICKET: i32 = 1464;
    const RUNE_ARROW: i32 = 892;
    const TARGET_COUNT: i32 = 156;
    const RANGED_STAT: i32 = 4;
    const STAND_X: i32 = 2672;
    const STAND_Z: i32 = 3419;

    let round = get("ranging_guild_round").expect("ranging_guild_round");
    assert_eq!(round.settings.start_script, Some("RangingGuild"));
    assert_eq!(round.settings.deadline, Duration::from_secs(300));
    let inject = settings_inject_map(round.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("coinsPerTrip").and_then(Value::as_f64),
        Some(400.0)
    );

    let start = round
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("StartScript");
    let seed = round.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::Stat {
        id: RANGED_STAT,
        min: 70
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: MAGIC_SHORTBOW,
        count: 1
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: COINS,
        count: 400
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: ARCHERY_TICKET,
        count: 0
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: RUNE_ARROW,
        count: 0
    }));
    assert!(seed.contains(&Proof::ArrivedNear {
        x: STAND_X,
        z: STAND_Z,
        level: 0,
        radius: 2,
    }));
    assert!(
        !seed.iter().any(|proof| matches!(
            proof,
            Proof::ItemId {
                id: ARCHERY_TICKET,
                ..
            }
        )),
        "round must not seed tickets"
    );

    let watch = round.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert_eq!(
        watch,
        vec![
            Proof::ItemIdAtMost {
                id: COINS,
                count: 200
            },
            Proof::Varp {
                id: TARGET_COUNT,
                min: 1
            },
            Proof::Varp {
                id: TARGET_COUNT,
                min: 2
            },
            Proof::VarpExact {
                id: TARGET_COUNT,
                value: 0
            },
            Proof::ItemId {
                id: ARCHERY_TICKET,
                count: 1
            },
            Proof::ItemIdAtMost {
                id: COINS,
                count: 0
            },
            Proof::Varp {
                id: TARGET_COUNT,
                min: 1
            },
        ]
    );
    assert_eq!(
        round.proof,
        Proof::Varp {
            id: TARGET_COUNT,
            min: 1
        }
    );
    assert!(names().contains(&"ranging_guild_round"));
}

#[test]
fn ranging_guild_redeem_orders_seeded_ticket_spend_and_item_change() {
    // Frozen rangingguild-live.ts --phase redeem: tickets are seeded, not earned.
    const MAGIC_SHORTBOW: i32 = 861;
    const ARCHERY_TICKET: i32 = 1464;
    const RUNE_ARROW: i32 = 892;
    const RANGED_STAT: i32 = 4;
    const MERCHANT_X: i32 = 2659;
    const MERCHANT_Z: i32 = 3430;

    let redeem = get("ranging_guild_redeem").expect("ranging_guild_redeem");
    assert_eq!(redeem.settings.start_script, Some("RangingGuild"));
    assert_eq!(redeem.settings.deadline, SCRIPT_GOLD_DEADLINE);

    let start = redeem
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("StartScript");
    let seed = redeem.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::Stat {
        id: RANGED_STAT,
        min: 70
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: MAGIC_SHORTBOW,
        count: 1
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: ARCHERY_TICKET,
        count: 2000
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: RUNE_ARROW,
        count: 0
    }));
    assert!(seed.contains(&Proof::ArrivedNear {
        x: MERCHANT_X,
        z: MERCHANT_Z,
        level: 0,
        radius: 3,
    }));
    assert!(
        !seed
            .iter()
            .any(|proof| matches!(proof, Proof::ItemId { id: 995, .. })),
        "redeem must not seed coins that could fund a round"
    );
    assert!(
        redeem.steps[..start]
            .iter()
            .any(|step| step.name.contains("seeded") || step.name.contains("ticket")),
        "redemption tickets must be named as seeded, not earned"
    );

    let watch = redeem.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert_eq!(
        watch,
        vec![
            Proof::ItemIdAtMost {
                id: ARCHERY_TICKET,
                count: 0
            },
            Proof::ItemId {
                id: RUNE_ARROW,
                count: 50
            },
        ]
    );
    assert_eq!(
        redeem.proof,
        Proof::ItemId {
            id: RUNE_ARROW,
            count: 50
        }
    );
    assert!(names().contains(&"ranging_guild_redeem"));
}

#[test]
fn ranging_guild_bank_orders_keep_deposit_coin_withdraw_close_return_and_fee() {
    // Frozen rangingguild-live.ts --phase full Seers seed plus KEEP/deposit
    // identity. Seeded tickets/arrows are setup; the fee after STAND is the
    // subsequent work. Empty-coin stop is not this cell.
    const COINS: i32 = 995;
    const MAGIC_SHORTBOW: i32 = 861;
    const ARCHERY_TICKET: i32 = 1464;
    const RUNE_ARROW: i32 = 892;
    const TARGET_COUNT: i32 = 156;
    const RANGED_STAT: i32 = 4;
    const SEERS_X: i32 = 2725;
    const SEERS_Z: i32 = 3491;
    const STAND_X: i32 = 2672;
    const STAND_Z: i32 = 3419;

    let bank = get("ranging_guild_bank").expect("ranging_guild_bank");
    assert_eq!(bank.settings.start_script, Some("RangingGuild"));
    assert_eq!(bank.settings.deadline, SCRIPT_GOLD_DEADLINE);
    let inject = settings_inject_map(bank.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("coinsPerTrip").and_then(Value::as_f64),
        Some(400.0)
    );

    let start = bank
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("StartScript");
    let seed = bank.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::Stat {
        id: RANGED_STAT,
        min: 70
    }));
    assert!(seed.contains(&Proof::ArrivedNear {
        x: SEERS_X,
        z: SEERS_Z,
        level: 0,
        radius: 6,
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: ARCHERY_TICKET,
        count: 1999
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: RUNE_ARROW,
        count: 50
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: COINS,
        count: 0
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: MAGIC_SHORTBOW,
        count: 0
    }));
    assert!(
        !seed.iter().any(|proof| matches!(
            proof,
            Proof::ItemId {
                id: ARCHERY_TICKET,
                count: 2000
            }
        )),
        "bank must not seed a redeem stack"
    );
    assert!(
        !seed
            .iter()
            .any(|proof| matches!(proof, Proof::ItemId { id: COINS, .. })),
        "bank must not seed pack coins that could claim withdraw"
    );
    assert!(
        bank.steps[..start]
            .iter()
            .any(|step| step.name.contains("seeded") || step.name.contains("KEEP")),
        "KEEP tickets/arrows must be named as seeded, not earned"
    );

    let watch = bank.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert_eq!(
        watch,
        vec![
            Proof::BankItemId {
                id: RUNE_ARROW,
                count: 50
            },
            Proof::ItemId {
                id: ARCHERY_TICKET,
                count: 1999
            },
            Proof::ItemIdAtMost {
                id: RUNE_ARROW,
                count: 0
            },
            Proof::ItemId {
                id: COINS,
                count: 400
            },
            Proof::BankClosed,
            Proof::ArrivedNear {
                x: STAND_X,
                z: STAND_Z,
                level: 0,
                radius: 2,
            },
            Proof::ItemIdAtMost {
                id: COINS,
                count: 200
            },
            Proof::Varp {
                id: TARGET_COUNT,
                min: 1
            },
        ]
    );
    assert_eq!(
        bank.proof,
        Proof::Varp {
            id: TARGET_COUNT,
            min: 1
        }
    );
    assert!(names().contains(&"ranging_guild_bank"));
}

#[test]
fn ranging_guild_full_orders_bought_banked_continuation_and_empty_coin_rails() {
    // Frozen rangingguild-live.ts --phase full: 15-minute wall budget,
    // coinsPerTrip 400, seeded 1999 tickets, no seeded rune arrows. Modal
    // 446 and ScriptRunner.stop are Core witnesses, not scenario Proofs.
    const COINS: i32 = 995;
    const MAGIC_SHORTBOW: i32 = 861;
    const ARCHERY_TICKET: i32 = 1464;
    const RUNE_ARROW: i32 = 892;
    const TARGET_COUNT: i32 = 156;
    const RANGED_STAT: i32 = 4;
    const SEERS_X: i32 = 2725;
    const SEERS_Z: i32 = 3491;
    const STAND_X: i32 = 2672;
    const STAND_Z: i32 = 3419;

    let full = get("ranging_guild_full").expect("ranging_guild_full");
    assert_eq!(full.settings.start_script, Some("RangingGuild"));
    assert_eq!(full.settings.deadline, Duration::from_secs(15 * 60));
    let inject = settings_inject_map(full.settings.script_settings_inject).unwrap();
    assert_eq!(
        inject.get("coinsPerTrip").and_then(Value::as_f64),
        Some(400.0)
    );

    let start = full
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("StartScript");
    let seed = full.steps[..start]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert!(seed.contains(&Proof::Stat {
        id: RANGED_STAT,
        min: 70
    }));
    assert!(seed.contains(&Proof::ArrivedNear {
        x: SEERS_X,
        z: SEERS_Z,
        level: 0,
        radius: 6,
    }));
    assert!(seed.contains(&Proof::ItemId {
        id: ARCHERY_TICKET,
        count: 1999
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: RUNE_ARROW,
        count: 0
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: COINS,
        count: 0
    }));
    assert!(seed.contains(&Proof::ItemIdAtMost {
        id: MAGIC_SHORTBOW,
        count: 0
    }));
    assert!(
        !seed
            .iter()
            .any(|proof| matches!(proof, Proof::ItemId { id: RUNE_ARROW, .. })),
        "full must not seed pack rune arrows"
    );
    assert!(
        !seed.iter().any(|proof| matches!(
            proof,
            Proof::ItemId {
                id: ARCHERY_TICKET,
                count: 2000
            }
        )),
        "full must not seed a redeem stack"
    );
    assert!(
        full.steps[..start]
            .iter()
            .any(|step| step.name.contains("seeded") || step.name.contains("KEEP")),
        "KEEP tickets must be named as seeded, not earned"
    );

    let watch = full.steps[start + 1..]
        .iter()
        .map(|step| step.wait.arm)
        .collect::<Vec<_>>();
    assert_eq!(
        watch,
        vec![
            Proof::ItemId {
                id: COINS,
                count: 400
            },
            Proof::BankClosed,
            Proof::ArrivedNear {
                x: STAND_X,
                z: STAND_Z,
                level: 0,
                radius: 2,
            },
            Proof::ItemIdAtMost {
                id: COINS,
                count: 200
            },
            Proof::Varp {
                id: TARGET_COUNT,
                min: 1
            },
            Proof::Varp {
                id: TARGET_COUNT,
                min: 2
            },
            Proof::VarpExact {
                id: TARGET_COUNT,
                value: 0
            },
            Proof::ItemId {
                id: ARCHERY_TICKET,
                count: 2000
            },
            Proof::ItemIdAtMost {
                id: ARCHERY_TICKET,
                count: 1999
            },
            Proof::ItemId {
                id: RUNE_ARROW,
                count: 50
            },
            Proof::ItemIdAtMost {
                id: COINS,
                count: 0
            },
            Proof::Varp {
                id: TARGET_COUNT,
                min: 1
            },
            Proof::BankItemId {
                id: RUNE_ARROW,
                count: 50
            },
            Proof::ItemIdAtMost {
                id: RUNE_ARROW,
                count: 0
            },
            Proof::ItemIdAtMost {
                id: COINS,
                count: 199
            },
        ]
    );
    assert_eq!(
        full.proof,
        Proof::ItemIdAtMost {
            id: COINS,
            count: 199
        }
    );
    assert!(names().contains(&"ranging_guild_full"));
}

#[test]
fn sherlock_scenarios_start_the_compiled_card_with_one_seeded_clue() {
    let cases: [(&str, i32, &str, &[&str]); 4] = [
        ("sherlock_talk", 2681, "trail_clue_easy_simple005", &[]),
        ("sherlock_search", 2679, "trail_clue_easy_simple003", &[]),
        (
            "sherlock_dig",
            2827,
            "trail_clue_medium_map001",
            &["give spade 1"],
        ),
        (
            "sherlock_coord",
            2823,
            "trail_clue_medium_sextant012",
            &[
                "give spade 1",
                "give trail_sextant 1",
                "give trail_watch 1",
                "give trail_chart 1",
            ],
        ),
    ];
    for (name, clue_id, clue_alias, tools) in cases {
        let scenario = get(name).unwrap_or_else(|| panic!("{name} is registered"));
        assert_eq!(scenario.name, name);
        assert_eq!(
            scenario.settings.start_script,
            Some("Sherlock"),
            "{name} must start the compiled Sherlock card"
        );
        assert!(scenario.settings.start_file.is_none());
        assert_eq!(scenario.settings.terminal_shot, Some(name));
        assert_eq!(
            scenario.proof,
            Proof::ClueReplaced { seeded: clue_id },
            "{name} proof must name the seeded clue id"
        );
        assert!(
            scenario
                .steps
                .iter()
                .any(|step| matches!(step.kind, StepKind::StartScript)),
            "{name} starts Sherlock after seed"
        );
        let seed = scenario
            .steps
            .iter()
            .find(|step| {
                matches!(
                    step.wait.arm,
                    Proof::ItemId {
                        id,
                        count: 1
                    } if id == clue_id
                )
            })
            .unwrap_or_else(|| panic!("{name} must wait for the seeded clue"));
        let StepKind::Perform { send } = &seed.kind else {
            panic!("{name} clue seed must be a Perform give");
        };
        let mut client = native_seed_client();
        let snapshot = GameSnapshot::new();
        let before = client.out.pos;
        assert!(send(&mut client, &snapshot), "{name} seed send");
        let written =
            String::from_utf8_lossy(&client.out.data()[before..client.out.pos]).into_owned();
        let clue_give = format!("give {clue_alias} 1");
        assert!(
            written.contains(&clue_give),
            "{name} must seed {clue_give}: {written}"
        );
        for tool in tools {
            assert!(written.contains(tool), "{name} must seed {tool}: {written}");
        }
        assert!(
            scenario.steps.iter().any(|step| {
                matches!(
                    step.wait.arm,
                    Proof::ClueReplaced { seeded } if seeded == clue_id
                )
            }),
            "{name} watches the seeded clue leave after Start"
        );
        assert!(names().contains(&name));
    }
}
