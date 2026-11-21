use api::interact::{
    ActionSpec, Driver, Interactions, NativeMenuTarget, OpTarget, SendReason, SendResult,
    WireCommand, SCENE_READY,
};
use api::prot::packet_metadata;
use api::snapshot::{GameSnapshot, WidgetView};
use client::client::{
    Client, ClientConfig, ClientNpc, ClientPlayer, ClientRevision, MiniMenuAction,
};
use client::config::if_type::{ButtonType, ComponentType, IfType, IfTypeMut};
use client::config::runtime_377::{
    CacheFields377, InterfaceFields377, MorphTable377, NpcFields377,
};
use client::config::{Cache, CacheRevision377, NpcType};
use client::io::{ClientProt, ClientStream, Isaac};
use std::net::TcpListener;
use std::sync::Arc;

const ISAAC_SEED: [i32; 4] = [17, 29, 41, 53];
const NPC_TYPE: i32 = 9;
const NPC_SLOT: usize = 7;
const CHAT_ROOT: usize = 200;
const CHOICE_ONE: usize = 201;
const CHOICE_TWO: usize = 202;
const CONTINUE_BUTTON: usize = 203;

fn config() -> ClientConfig {
    ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: String::new(),
        members: true,
        lowmem: false,
    }
}

struct Scene {
    _listener: TcpListener,
    client: Client,
}
fn npc_fields(morph: Option<MorphTable377>) -> NpcFields377 {
    NpcFields377 {
        field_1451: -1,
        field_1430: -1,
        field_1440: -1,
        field_1434: true,
        morph,
    }
}

fn scene(revision: ClientRevision) -> Scene {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local test socket");
    let addr = listener.local_addr().expect("local socket address");
    let stream = ClientStream::connect(&addr.ip().to_string(), addr.port()).expect("connect");
    let mut client = Client::from_shared_with_revision(
        config(),
        Arc::new(Cache::default()),
        Arc::new(Vec::new()),
        Arc::new(Vec::new()),
        revision,
    );
    client.ingame = true;
    client.scene_state = SCENE_READY;
    client.map_build_base_x = 3200;
    client.map_build_base_z = 3200;
    client.stream = Some(stream);
    client.out.random = Some(Isaac::new(&ISAAC_SEED));
    client.local_player = Some(ClientPlayer::at(50, 50));
    client.shell.mouse_x = 120;
    client.shell.mouse_y = 240;
    client.shell.mouse_click_x = 23;
    client.shell.mouse_click_y = 45;
    client.shell.mouse_click_time = 6789;
    if revision == ClientRevision::R377 {
        Arc::get_mut(&mut client.cache)
            .expect("test owns the client cache")
            .revision_377 = Some(Box::new(CacheRevision377 {
            config: CacheFields377 {
                npcs: (0..=NPC_TYPE).map(|_| npc_fields(None)).collect(),
                ..Default::default()
            },
            interfaces: InterfaceFields377 {
                client_code_layers: Default::default(),
                type1_fields: Vec::new(),
                null_string_masks: Vec::new(),
                initial_overlays: Vec::new(),
            },
        }));
    }
    Scene {
        _listener: listener,
        client,
    }
}

fn rebuild(client: &mut Client) -> GameSnapshot {
    // Publish the received scene, actor and interface fixture together.
    client.gens.scene += 1;
    client.gens.npc += 1;
    client.gens.npc_info += 1;
    client.gens.iface += 1;
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(client);
    snapshot
}

fn plant_npc(client: &mut Client) {
    let cache = Arc::get_mut(&mut client.cache).expect("test owns the client cache");
    while cache.npcs.len() <= NPC_TYPE as usize {
        cache.npcs.push(NpcType::default());
    }
    cache.npcs[NPC_TYPE as usize] = NpcType {
        id: NPC_TYPE,
        name: "Mercenary Captain".into(),
        op: vec![Some("Talk-to".into())],
        ..Default::default()
    };
    let mut entity = client::dash3d::ClientEntity {
        x: 53 * 128,
        z: 50 * 128,
        ..Default::default()
    };
    entity.route_x[0] = 53;
    entity.route_z[0] = 50;
    let npc = ClientNpc {
        r#type: Some(NPC_TYPE as usize),
        entity,
        ..Default::default()
    };
    client.npc[NPC_SLOT] = Some(Box::new(npc));
    client.npc_ids = vec![NPC_SLOT as i32];
    client.npc_count = 1;
    client.pick_count = 1;
    client.pick_typecodes[0] =
        (1 << 29) | (((NPC_SLOT as i32) & 0x7fff) << 14) | ((50 & 0x7f) << 7) | (53 & 0x7f);
}

fn plant_chat(client: &mut Client, buttons: &[(usize, i32, &str)], children: Option<Vec<usize>>) {
    let child_ids = children.unwrap_or_else(|| buttons.iter().map(|(id, _, _)| *id).collect());
    let child_ids_for_client: Vec<i32> = child_ids.iter().map(|id| *id as i32).collect();
    let child_x = vec![24; child_ids.len()];
    let child_y = vec![20; child_ids.len()];
    client.set_iface(
        CHAT_ROOT,
        IfType {
            id: CHAT_ROOT as i32,
            layer_id: CHAT_ROOT as i32,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(child_ids_for_client),
            child_x: Some(child_x),
            child_y: Some(child_y),
            width: 400,
            height: 100,
            ..Default::default()
        },
    );
    for (id, button_type, label) in buttons {
        client.set_iface(
            *id,
            IfType {
                id: *id as i32,
                layer_id: CHAT_ROOT as i32,
                r#type: ComponentType::TYPE_TEXT,
                width: 140,
                height: 24,
                button_text: (*label).into(),
                ..Default::default()
            },
        );
        client.set_iface_mut(
            *id,
            IfTypeMut {
                button_type: *button_type,
                text: (*label).into(),
                ..Default::default()
            },
        );
    }
    client.chat_modal_id = CHAT_ROOT as i32;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Frame {
    protocol: ClientProt,
    body: Vec<u8>,
}

fn frames(client: &Client) -> Vec<Frame> {
    let bytes = &client.out.data()[..client.out.pos];
    let mut cipher = Isaac::new(&ISAAC_SEED);
    let mut offset = 0;
    let mut result = Vec::new();
    while offset < bytes.len() {
        let opcode = bytes[offset].wrapping_sub(cipher.next_int() as u8);
        offset += 1;
        let metadata = packet_metadata(client.revision(), opcode)
            .unwrap_or_else(|| panic!("unmapped outbound opcode {opcode}"));
        let body_len = match metadata.length {
            length if length >= 0 => length as usize,
            -1 => {
                let length = *bytes.get(offset).expect("one-byte packet length");
                offset += 1;
                usize::from(length)
            }
            -2 => {
                let length = u16::from_be_bytes([
                    *bytes.get(offset).expect("two-byte packet length high byte"),
                    *bytes
                        .get(offset + 1)
                        .expect("two-byte packet length low byte"),
                ]);
                offset += 2;
                usize::from(length)
            }
            length => panic!("unsupported packet length {length}"),
        };
        let end = offset.checked_add(body_len).expect("packet end");
        let body = bytes
            .get(offset..end)
            .expect("complete outbound packet")
            .to_vec();
        offset = end;
        result.push(Frame {
            protocol: metadata.named,
            body,
        });
    }
    result
}

fn assert_packet(client: &Client, protocol: ClientProt, body: &[u8]) {
    let output = frames(client);
    assert!(
        output
            .iter()
            .any(|frame| frame.protocol == protocol && frame.body == body),
        "missing {protocol:?} {body:02x?} in {output:?}"
    );
    assert!(
        output
            .iter()
            .all(|frame| frame.protocol != ClientProt::EVENT_MOUSE_CLICK),
        "native menu dispatch must not emit click telemetry: {output:?}"
    );
}

fn mouse_state(client: &Client) -> (i32, i32, i32, i32, i32, i32, i64) {
    (
        client.shell.mouse_x,
        client.shell.mouse_y,
        client.shell.mouse_button,
        client.shell.mouse_click_button,
        client.shell.mouse_click_x,
        client.shell.mouse_click_y,
        client.shell.mouse_click_time,
    )
}

fn run_native_npc_talk(revision: ClientRevision) -> Client {
    let mut scene = scene(revision);
    plant_npc(&mut scene.client);
    let snapshot = rebuild(&mut scene.client);
    let before_mouse = mouse_state(&scene.client);
    let npc = snapshot
        .npcs()
        .iter()
        .find(|npc| npc.actions.first().and_then(|action| action.as_deref()) == Some("Talk-to"))
        .expect("planted NPC in snapshot");
    let result = Interactions::new(&snapshot, &mut scene.client)
        .interact(OpTarget::Npc(npc), ActionSpec::Label("Talk-to".into()));
    assert!(
        matches!(
            &result,
            SendResult::Sent {
                command: WireCommand::Op { operation: 1, .. },
                ..
            }
        ),
        "Talk-to was not dispatched: {result:?}"
    );
    assert_eq!(mouse_state(&scene.client), before_mouse);
    scene.client
}

fn run_synthetic_npc_talk(revision: ClientRevision) -> Client {
    let mut scene = scene(revision);
    plant_npc(&mut scene.client);
    let _snapshot = rebuild(&mut scene.client);
    Driver::set_menu(
        &mut scene.client,
        0,
        MiniMenuAction::OP_NPC1,
        NPC_SLOT as i32,
        0,
        0,
    );
    assert!(Driver::do_action(&mut scene.client, 0));
    scene.client
}

fn choice_scene(revision: ClientRevision) -> (Scene, GameSnapshot) {
    let mut scene = scene(revision);
    plant_chat(
        &mut scene.client,
        &[
            (CHOICE_ONE, ButtonType::BUTTON_OK, "First choice"),
            (CHOICE_TWO, ButtonType::BUTTON_OK, "Second choice"),
        ],
        None,
    );
    let snapshot = rebuild(&mut scene.client);
    (scene, snapshot)
}

fn continue_scene(revision: ClientRevision) -> (Scene, GameSnapshot) {
    let mut scene = scene(revision);
    plant_chat(
        &mut scene.client,
        &[(CONTINUE_BUTTON, ButtonType::BUTTON_CONTINUE, "Continue")],
        None,
    );
    let snapshot = rebuild(&mut scene.client);
    (scene, snapshot)
}

fn run_native_choice(revision: ClientRevision) -> Client {
    let (mut scene, snapshot) = choice_scene(revision);
    let before_mouse = mouse_state(&scene.client);
    let result = Interactions::new(&snapshot, &mut scene.client).answer_choice(1);
    assert!(
        matches!(
            &result,
            SendResult::Sent {
                command: WireCommand::Button {
                    component_id,
                    button_type: ButtonType::BUTTON_OK,
                },
                ..
            } if *component_id == CHOICE_ONE as i32
        ),
        "first choice was not dispatched: {result:?}"
    );
    assert_eq!(mouse_state(&scene.client), before_mouse);
    scene.client
}

fn run_synthetic_choice(revision: ClientRevision) -> Client {
    let (mut scene, _) = choice_scene(revision);
    Driver::set_menu(
        &mut scene.client,
        0,
        MiniMenuAction::IF_BUTTON,
        0,
        0,
        CHOICE_ONE as i32,
    );
    assert!(Driver::do_action(&mut scene.client, 0));
    scene.client
}

fn run_native_continue(revision: ClientRevision) -> Client {
    let (mut scene, snapshot) = continue_scene(revision);
    let before_mouse = mouse_state(&scene.client);
    let result = Interactions::new(&snapshot, &mut scene.client).continue_dialog(None);
    assert!(
        matches!(
            &result,
            SendResult::Sent {
                command: WireCommand::Continue {
                    component_id,
                },
                ..
            } if *component_id == CONTINUE_BUTTON as i32
        ),
        "Continue was not dispatched: {result:?}"
    );
    assert_eq!(mouse_state(&scene.client), before_mouse);
    scene.client
}

fn run_synthetic_continue(revision: ClientRevision) -> Client {
    let (mut scene, _) = continue_scene(revision);
    Driver::set_menu(
        &mut scene.client,
        0,
        MiniMenuAction::PAUSE_BUTTON,
        0,
        0,
        CONTINUE_BUTTON as i32,
    );
    assert!(Driver::do_action(&mut scene.client, 0));
    scene.client
}

#[test]
fn native_npc_talk_wire_matches_synthetic_menu_on_r289_and_r377() {
    for revision in [ClientRevision::R289, ClientRevision::R377] {
        let native = run_native_npc_talk(revision);
        let synthetic = run_synthetic_npc_talk(revision);
        let native_frames = frames(&native);
        assert_eq!(native_frames, frames(&synthetic));
        let body = match revision {
            ClientRevision::R274 | ClientRevision::R289 => [0, NPC_SLOT as u8],
            ClientRevision::R377 => [NPC_SLOT as u8, 0],
        };
        assert_packet(&native, ClientProt::OPNPC1, &body);
    }
}

#[test]
fn native_choice_and_continue_wire_matches_synthetic_menu_on_r289_and_r377() {
    for revision in [ClientRevision::R289, ClientRevision::R377] {
        let native_choice = run_native_choice(revision);
        let synthetic_choice = run_synthetic_choice(revision);
        assert_eq!(frames(&native_choice), frames(&synthetic_choice));
        assert_packet(
            &native_choice,
            ClientProt::IF_BUTTON,
            &(CHOICE_ONE as u16).to_be_bytes(),
        );

        let native_continue = run_native_continue(revision);
        let synthetic_continue = run_synthetic_continue(revision);
        assert_eq!(frames(&native_continue), frames(&synthetic_continue));
        assert_packet(
            &native_continue,
            ClientProt::RESUME_PAUSEBUTTON,
            &(CONTINUE_BUTTON as u16).to_be_bytes(),
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UiState {
    menu_num_entries: i32,
    menu_option: Vec<String>,
    menu_action: Vec<i32>,
    menu_param_a: Vec<i32>,
    menu_param_b: Vec<i32>,
    menu_param_c: Vec<i32>,
    is_menu_open: bool,
    menu_area: i32,
    menu_x: i32,
    menu_y: i32,
    menu_width: i32,
    menu_height: i32,
    use_mode: i32,
    target_mode: i32,
    obj_selected_name: String,
    target_op: String,
    target_mask: i32,
    target_com_id: i32,
    obj_com_id: i32,
    obj_selected_slot: i32,
    obj_selected_com_id: i32,
    selected_area: i32,
    selected_com_id: i32,
    selected_item: i32,
    selected_cycle: i32,
    resumed_pause_button: bool,
    over_main_com_id: i32,
    over_side_com_id: i32,
    over_chat_com_id: i32,
    last_over_com_id: i32,
    hovered_slot: i32,
    hovered_slot_com_id: i32,
}

fn ui_state(client: &Client) -> UiState {
    UiState {
        menu_num_entries: client.menu_num_entries,
        menu_option: client.menu_option.clone(),
        menu_action: client.menu_action.clone(),
        menu_param_a: client.menu_param_a.clone(),
        menu_param_b: client.menu_param_b.clone(),
        menu_param_c: client.menu_param_c.clone(),
        is_menu_open: client.is_menu_open,
        menu_area: client.menu_area,
        menu_x: client.menu_x,
        menu_y: client.menu_y,
        menu_width: client.menu_width,
        menu_height: client.menu_height,
        use_mode: client.use_mode,
        target_mode: client.target_mode,
        obj_selected_name: client.obj_selected_name.clone(),
        target_op: client.target_op.clone(),
        target_mask: client.target_mask,
        target_com_id: client.target_com_id,
        obj_com_id: client.obj_com_id,
        obj_selected_slot: client.obj_selected_slot,
        obj_selected_com_id: client.obj_selected_com_id,
        selected_area: client.selected_area,
        selected_com_id: client.selected_com_id,
        selected_item: client.selected_item,
        selected_cycle: client.selected_cycle,
        resumed_pause_button: client.resumed_pause_button,
        over_main_com_id: client.over_main_com_id,
        over_side_com_id: client.over_side_com_id,
        over_chat_com_id: client.over_chat_com_id,
        last_over_com_id: client.last_over_com_id,
        hovered_slot: client.hovered_slot,
        hovered_slot_com_id: client.hovered_slot_com_id,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WireState {
    bytes: Vec<u8>,
    cipher_next: [i32; 4],
}

fn wire_state(client: &Client) -> WireState {
    let mut cipher = client.out.random.clone().expect("seeded packet cipher");
    let mut cipher_next = [0; 4];
    for value in &mut cipher_next {
        *value = cipher.next_int();
    }
    WireState {
        bytes: client.out.data()[..client.out.pos].to_vec(),
        cipher_next,
    }
}

fn seed_menu_sentinel(client: &mut Client) {
    client.menu_num_entries = 1;
    client.menu_option[0] = "Existing menu row".into();
    client.menu_action[0] = MiniMenuAction::CANCEL;
    client.menu_param_a[0] = 11;
    client.menu_param_b[0] = 12;
    client.menu_param_c[0] = 13;
    client.use_mode = 1;
    client.target_mode = 1;
    client.obj_selected_name = "Existing item".into();
    client.target_op = "Cast".into();
    client.target_mask = 0x10;
    client.target_com_id = 44;
    client.obj_com_id = 45;
    client.obj_selected_slot = 4;
    client.obj_selected_com_id = 46;
    client.selected_area = 1;
    client.selected_com_id = 47;
    client.selected_item = 3;
    client.selected_cycle = 9;
}

fn assert_native_refusal_unchanged(
    client: &mut Client,
    snapshot: &GameSnapshot,
    widget: &WidgetView,
    action: i32,
    expected: SendReason,
) {
    let before_ui = ui_state(client);
    let before_wire = wire_state(client);
    assert_eq!(
        Driver::dispatch_native_menu(
            client,
            snapshot,
            NativeMenuTarget::Component { widget, action },
        ),
        Err(expected)
    );
    assert_eq!(
        ui_state(client),
        before_ui,
        "refusal changed menu or selection"
    );
    assert_eq!(
        wire_state(client),
        before_wire,
        "refusal changed output or cipher"
    );
}

#[test]
fn native_component_refusals_preserve_menu_selection_and_cipher() {
    for revision in [ClientRevision::R289, ClientRevision::R377] {
        for button_type in [ButtonType::BUTTON_OK, ButtonType::BUTTON_CONTINUE] {
            let mut scene = scene(revision);
            plant_chat(
                &mut scene.client,
                &[(CHOICE_ONE, button_type, "Action")],
                None,
            );
            seed_menu_sentinel(&mut scene.client);
            let snapshot = rebuild(&mut scene.client);
            let widget = snapshot
                .widgets()
                .iter()
                .find(|widget| widget.component_id == CHOICE_ONE as i32)
                .expect("planted chat button");
            let expected_action = if button_type == ButtonType::BUTTON_OK {
                MiniMenuAction::PAUSE_BUTTON
            } else {
                MiniMenuAction::IF_BUTTON
            };
            assert_native_refusal_unchanged(
                &mut scene.client,
                &snapshot,
                widget,
                expected_action,
                SendReason::MenuIneligible,
            );

            let before_ui = ui_state(&scene.client);
            let before_wire = wire_state(&scene.client);
            let missing =
                Interactions::new(&snapshot, &mut scene.client).continue_dialog(Some(9999));
            assert!(matches!(
                missing,
                SendResult::Refused {
                    reason: SendReason::StaleTarget,
                    ..
                }
            ));
            assert_eq!(ui_state(&scene.client), before_ui);
            assert_eq!(wire_state(&scene.client), before_wire);

            if button_type == ButtonType::BUTTON_OK {
                let before_ui = ui_state(&scene.client);
                let before_wire = wire_state(&scene.client);
                let ineligible = Interactions::new(&snapshot, &mut scene.client)
                    .continue_dialog(Some(CHOICE_ONE as i32));
                assert!(matches!(
                    ineligible,
                    SendResult::Refused {
                        reason: SendReason::InvalidAction,
                        ..
                    }
                ));
                assert_eq!(ui_state(&scene.client), before_ui);
                assert_eq!(wire_state(&scene.client), before_wire);
            }
        }
    }
}

#[test]
fn duplicate_native_component_row_is_ambiguous_without_mutation() {
    for revision in [ClientRevision::R289, ClientRevision::R377] {
        let mut scene = scene(revision);
        plant_chat(
            &mut scene.client,
            &[(CHOICE_ONE, ButtonType::BUTTON_OK, "Action")],
            Some(vec![CHOICE_ONE, CHOICE_ONE]),
        );
        seed_menu_sentinel(&mut scene.client);
        let snapshot = rebuild(&mut scene.client);
        let widget = snapshot
            .widgets()
            .iter()
            .find(|widget| widget.component_id == CHOICE_ONE as i32)
            .expect("planted chat button");
        assert_native_refusal_unchanged(
            &mut scene.client,
            &snapshot,
            widget,
            MiniMenuAction::IF_BUTTON,
            SendReason::MenuAmbiguous,
        );
    }
}

#[test]
fn stale_native_component_scope_refuses_without_mutation() {
    for revision in [ClientRevision::R289, ClientRevision::R377] {
        for expected in [SendReason::StaleSession, SendReason::StaleScene] {
            let mut scene = scene(revision);
            plant_chat(
                &mut scene.client,
                &[(CHOICE_ONE, ButtonType::BUTTON_OK, "Action")],
                None,
            );
            seed_menu_sentinel(&mut scene.client);
            let snapshot = rebuild(&mut scene.client);
            let widget = snapshot
                .widgets()
                .iter()
                .find(|widget| widget.component_id == CHOICE_ONE as i32)
                .expect("planted chat button");
            match expected {
                SendReason::StaleSession => {
                    scene.client.gens.session = scene.client.gens.session.wrapping_add(1)
                }
                SendReason::StaleScene => {
                    scene.client.gens.scene = scene.client.gens.scene.wrapping_add(1)
                }
                _ => unreachable!(),
            }
            assert_native_refusal_unchanged(
                &mut scene.client,
                &snapshot,
                widget,
                MiniMenuAction::IF_BUTTON,
                expected,
            );
        }
    }
}

#[test]
fn repeated_continue_refuses_after_one_wire_send_without_further_mutation() {
    for revision in [ClientRevision::R289, ClientRevision::R377] {
        let (mut scene, snapshot) = continue_scene(revision);
        {
            let first = Interactions::new(&snapshot, &mut scene.client).continue_dialog(None);
            assert!(matches!(
                &first,
                SendResult::Sent {
                    command: WireCommand::Continue {
                        component_id,
                    },
                    ..
                } if *component_id == CONTINUE_BUTTON as i32
            ));
        }

        let after_first_ui = ui_state(&scene.client);
        let after_first_wire = wire_state(&scene.client);
        let second = Interactions::new(&snapshot, &mut scene.client).continue_dialog(None);
        assert!(matches!(
            second,
            SendResult::Refused {
                reason: SendReason::MenuIneligible,
                ..
            }
        ));
        assert_eq!(ui_state(&scene.client), after_first_ui);
        assert_eq!(wire_state(&scene.client), after_first_wire);
        assert_packet(
            &scene.client,
            ClientProt::RESUME_PAUSEBUTTON,
            &(CONTINUE_BUTTON as u16).to_be_bytes(),
        );
        assert_eq!(
            frames(&scene.client)
                .iter()
                .filter(|frame| frame.protocol == ClientProt::RESUME_PAUSEBUTTON)
                .count(),
            1
        );
    }
}

#[derive(Clone, Copy, Debug)]
enum NpcChange {
    Session,
    Scene,
    Removed,
    Unpublished,
    Reused,
    WrongKind,
    Moved,
    ReceivedNpcInfo,
    Ineligible,
    MenuOpen,
}

#[test]
fn stale_and_ineligible_npc_rows_refuse_without_mutation() {
    for revision in [ClientRevision::R289, ClientRevision::R377] {
        for (change, expected) in [
            (NpcChange::Session, SendReason::StaleSession),
            (NpcChange::Scene, SendReason::StaleScene),
            (NpcChange::Removed, SendReason::StaleTarget),
            (NpcChange::Unpublished, SendReason::StaleTarget),
            (NpcChange::Reused, SendReason::StaleTarget),
            (NpcChange::WrongKind, SendReason::StaleTarget),
            (NpcChange::Moved, SendReason::StaleTarget),
            (NpcChange::ReceivedNpcInfo, SendReason::StaleTarget),
            (NpcChange::Ineligible, SendReason::MenuIneligible),
            (NpcChange::MenuOpen, SendReason::MenuIneligible),
        ] {
            let mut scene = scene(revision);
            plant_npc(&mut scene.client);
            if matches!(change, NpcChange::Ineligible) {
                Arc::get_mut(&mut scene.client.cache)
                    .expect("test owns the unpublished cache")
                    .npcs[NPC_TYPE as usize]
                    .op[0] = Some("Attack".into());
            }
            seed_menu_sentinel(&mut scene.client);
            scene.client.use_mode = 0;
            scene.client.target_mode = 0;
            let snapshot = rebuild(&mut scene.client);
            let npc = snapshot.npcs().first().expect("published NPC");
            match change {
                NpcChange::Session => scene.client.gens.session += 1,
                NpcChange::Scene => scene.client.gens.scene += 1,
                NpcChange::Removed => scene.client.npc[NPC_SLOT] = None,
                NpcChange::Unpublished => scene.client.npc_count = 0,
                NpcChange::Reused => {
                    scene.client.npc[NPC_SLOT].as_mut().unwrap().r#type =
                        Some(NPC_TYPE as usize + 1);
                }
                NpcChange::WrongKind => {
                    scene.client.npc[NPC_SLOT] = None;
                    scene.client.players[NPC_SLOT] = Some(Box::new(ClientPlayer::at(53, 50)));
                }
                NpcChange::Moved => {
                    scene.client.npc[NPC_SLOT].as_mut().unwrap().route_x[0] += 1;
                }
                NpcChange::ReceivedNpcInfo => scene.client.gens.npc_info += 1,
                NpcChange::Ineligible => {
                    assert_eq!(npc.actions[0].as_deref(), Some("Attack"));
                }
                NpcChange::MenuOpen => scene.client.is_menu_open = true,
            }
            let before_ui = ui_state(&scene.client);
            let before_wire = wire_state(&scene.client);
            assert_eq!(
                Driver::dispatch_native_menu(
                    &mut scene.client,
                    &snapshot,
                    NativeMenuTarget::NpcTalk(npc),
                ),
                Err(expected),
                "{revision:?} {change:?}",
            );
            assert_eq!(
                ui_state(&scene.client),
                before_ui,
                "{revision:?} {change:?}"
            );
            assert_eq!(
                wire_state(&scene.client),
                before_wire,
                "{revision:?} {change:?}"
            );
        }
    }
}

#[test]
fn native_377_npc_morph_resolves_talk_and_refuses_changed_or_inactive_forms() {
    for change in [None, Some(false), Some(true)] {
        let mut scene = scene(ClientRevision::R377);
        plant_npc(&mut scene.client);
        let cache = Arc::get_mut(&mut scene.client.cache).expect("test owns the client cache");
        let form_id = cache.npcs.len();
        cache.npcs.push(NpcType {
            id: form_id as i32,
            name: "Current form".into(),
            op: vec![Some("Talk-to".into())],
            ..Default::default()
        });
        let fields = &mut cache.revision_377.as_mut().unwrap().config.npcs;
        fields.push(npc_fields(None));
        fields[form_id].field_1434 = change != Some(false);
        fields[NPC_TYPE as usize].morph = Some(MorphTable377 {
            varbit: -1,
            varp: 0,
            targets: vec![form_id as i32, -1],
        });
        scene.client.var = vec![0];
        let snapshot = rebuild(&mut scene.client);
        let npc = snapshot.npcs().first().expect("published morph NPC");
        assert_eq!(npc.r#type, Some(NPC_TYPE as usize));
        assert_eq!(
            npc.name.as_deref(),
            if change == Some(false) {
                None
            } else {
                Some("Current form")
            }
        );
        match change {
            None => {
                assert_eq!(
                    Driver::dispatch_native_menu(
                        &mut scene.client,
                        &snapshot,
                        NativeMenuTarget::NpcTalk(npc),
                    ),
                    Ok(()),
                );
                assert_packet(&scene.client, ClientProt::OPNPC1, &[NPC_SLOT as u8, 0]);
            }
            Some(changed_varp) => {
                let expected = if changed_varp {
                    scene.client.var[0] = 1;
                    scene.client.gens.varp += 1;
                    SendReason::StaleTarget
                } else {
                    SendReason::MenuIneligible
                };
                let before_ui = ui_state(&scene.client);
                let before_wire = wire_state(&scene.client);
                assert_eq!(
                    Driver::dispatch_native_menu(
                        &mut scene.client,
                        &snapshot,
                        NativeMenuTarget::NpcTalk(npc),
                    ),
                    Err(expected),
                );
                assert_eq!(ui_state(&scene.client), before_ui);
                assert_eq!(wire_state(&scene.client), before_wire);
            }
        }
    }
}
