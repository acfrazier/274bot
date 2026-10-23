//! Ordered requests through the production script pump and encrypted Client sink.
use super::*;
use api::interact::server_ordered::*;
use api::interact::Driver;
use api::prot::Send;
use script::native::{
    ActionContext, ActionError, ActionHandle, NativeMachine, NativeTick, Script, ScriptFailure,
    ScriptFlow,
};
use std::collections::HashSet;
use std::task::Poll;

const CIPHER_SEED: [i32; 4] = [17, 29, 41, 53];

enum Emission {
    Content(ServerOrderedRequest),
    ContentThenDrop(ServerOrderedRequest),
    Advanced {
        operations: Vec<Send>,
        debug: Option<Arc<api::debug_commands::DebugCatalog>>,
    },
}

struct OrderedMachine {
    request_id: u64,
    follow_drop: bool,
    drop_request: Option<u64>,
}

impl NativeMachine for OrderedMachine {
    type Args = Emission;
    type Output = ServerOrderedDispatch;

    fn begin(request: Emission, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let follow_drop = matches!(&request, Emission::ContentThenDrop(_));
        let request_id = match request {
            Emission::Content(request) | Emission::ContentThenDrop(request) => {
                cx.emit_server_ordered(request)?
            }
            Emission::Advanced { operations, debug } => match debug {
                Some(debug) => script::surface::packet_context_for_test_with_debug(cx, debug)
                    .send_batch(operations)?,
                None => script::surface::packet_context_for_test(cx).send_batch(operations)?,
            },
        };
        Ok(Self {
            request_id,
            follow_drop,
            drop_request: None,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        let Some(receipt) = cx.server_ordered_receipt(self.request_id).copied() else {
            return Poll::Pending;
        };
        if self.follow_drop {
            let Some(request) = self.drop_request else {
                self.drop_request = Some(cx.emit(script::shim::InteractReq::Held {
                    name: "Logs".into(),
                    action: "Drop".into(),
                    slot: Some(2),
                    target_item_id: Some(1511),
                })?);
                return Poll::Pending;
            };
            if cx.interaction_receipt(request).is_none() {
                return Poll::Pending;
            }
        }
        Poll::Ready(Ok(receipt))
    }

    fn cancel(&mut self) {}
}

#[derive(Default)]
struct Results {
    receipts: Vec<ServerOrderedDispatch>,
    errors: Vec<ActionError>,
}

struct OrderedScript {
    pending: VecDeque<Emission>,
    handle: Option<ActionHandle<OrderedMachine>>,
    results: Arc<Mutex<Results>>,
}

impl Script for OrderedScript {
    fn configure(
        &mut self,
        _: Arc<script::native::PreparedConfig>,
    ) -> Result<script::native::SettingsApply, script::native::ConfigError> {
        Ok(script::native::SettingsApply::Applied)
    }

    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        if let Some(handle) = self.handle.as_ref() {
            if let Poll::Ready(result) = tick.actions.poll(handle, &mut tick.cx) {
                match result {
                    Ok(receipt) => self.results.lock().unwrap().receipts.push(receipt),
                    Err(error) => self.results.lock().unwrap().errors.push(error),
                }
                self.handle = None;
            }
        }
        if self.handle.is_none() {
            if let Some(request) = self.pending.pop_front() {
                match tick.actions.begin::<OrderedMachine>(request, &mut tick.cx) {
                    Ok(handle) => self.handle = Some(handle),
                    Err(error) => self.results.lock().unwrap().errors.push(error),
                }
            }
        }
        Ok(ScriptFlow::Continue)
    }
}

struct Rig {
    client: Client,
    snapshot: GameSnapshot,
    input: Arc<SlotInput>,
    names: Arc<api::obj_names::ObjNames>,
    scripts: ScriptWall,
    cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    navs: Arc<Mutex<HashMap<String, NavBot>>>,
    results: Arc<Mutex<Results>>,
    _listener: TcpListener,
}

impl Rig {
    fn new() -> Self {
        Self::new_with_revision(client::client::ClientRevision::R289)
    }

    fn new_with_revision(revision: client::client::ClientRevision) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let mut client = Client::new_with_revision(
            ClientConfig {
                host: addr.ip().to_string(),
                port: addr.port(),
                cache_dir: String::new(),
                members: true,
                lowmem: true,
            },
            revision,
        );
        client.stream =
            Some(client::io::ClientStream::connect(&addr.ip().to_string(), addr.port()).unwrap());
        client.ingame = true;
        client.scene_state = 2;
        client.local_player = Some(client::dash3d::ClientPlayer::at(5, 5));
        client.out.random = Some(client::io::Isaac::new(&CIPHER_SEED));
        client.gens.player_info = 1;
        client.gens.scene = 1;
        client.gens.inv = 1;
        client.gens.iface = 1;
        {
            let cache = Arc::get_mut(&mut client.cache).unwrap();
            cache.objs.resize(1512, client::config::ObjType::default());
            for (id, name) in [(946, "Knife"), (1511, "Logs")] {
                cache.objs[id].id = id as i32;
                cache.objs[id].name = name.into();
                cache.objs[id].iop[4] = Some("Drop".into());
            }
        }
        let names = Arc::new(api::obj_names::ObjNames::from_objs(&client.cache.objs));
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(&client);
        let input = SlotInput::new();
        input.bind_outbound_observer(&mut client);
        input.observe_input_tick(
            client.revision(),
            client.gens.player_info,
            client.gens.session,
        );
        Self {
            client,
            snapshot,
            input,
            names,
            scripts: Arc::new(Mutex::new(HashMap::new())),
            cheats: Arc::new(Mutex::new(HashMap::new())),
            navs: test_navs("so3-wire"),
            results: Arc::new(Mutex::new(Results::default())),
            _listener: listener,
        }
    }

    fn start(&self, requests: impl IntoIterator<Item = Emission>) {
        let slot = script_slot_or_insert(&self.scripts, "so3-wire");
        slot.lock()
            .unwrap()
            .start_test_script(
                Box::new(OrderedScript {
                    pending: requests.into_iter().collect(),
                    handle: None,
                    results: Arc::clone(&self.results),
                }),
                Some(api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap()),
            )
            .unwrap();
    }

    fn pump(&mut self, tick: u64) -> crate::script_runtime::ScriptObservation {
        self.snapshot.seed_tick(u32::try_from(tick).unwrap());
        self.input
            .observe_input_tick(self.client.revision(), tick, self.client.gens.session);
        let cache = Arc::clone(&self.client.cache);
        crate::script_runtime::script_observe_cached_with_channels(
            &mut self.client,
            "so3-wire",
            true,
            true,
            true,
            tick,
            self.snapshot.tile(),
            None,
            None,
            Some(&self.snapshot),
            None,
            None,
            Some(&self.names),
            &self.scripts,
            &self.cheats,
            &self.navs,
            &None,
            false,
            false,
            false,
            None,
            Some(&self.input),
            Some(cache),
            Some(Arc::clone(&self.names)),
            None,
            crate::script_channels::BrokerWorld::Local,
            None,
            None,
            None,
        )
    }

    fn receipt(&self) -> ServerOrderedDispatch {
        let results = self.results.lock().unwrap();
        assert!(
            results.errors.is_empty(),
            "submission failed: {:?}",
            results.errors
        );
        assert_eq!(results.receipts.len(), 1);
        results.receipts[0]
    }

    fn decoded(&self) -> Vec<(u8, Vec<u8>)> {
        let mut cipher = client::io::Isaac::new(&CIPHER_SEED);
        let bytes = &self.client.out.data()[..self.client.out.pos];
        let mut cursor = 0;
        let mut result = Vec::new();
        while cursor < bytes.len() {
            let opcode = bytes[cursor].wrapping_sub(cipher.next_int() as u8);
            cursor += 1;
            let metadata = api::prot::packet_metadata(self.client.revision(), opcode).unwrap();
            let length = match metadata.length {
                n if n >= 0 => n as usize,
                -1 => {
                    let n = bytes[cursor] as usize;
                    cursor += 1;
                    n
                }
                -2 => {
                    let n = u16::from_be_bytes([bytes[cursor], bytes[cursor + 1]]) as usize;
                    cursor += 2;
                    n
                }
                n => panic!("invalid packet descriptor {n}"),
            };
            let end = cursor + length;
            assert!(end <= bytes.len(), "truncated committed packet {opcode}");
            result.push((opcode, bytes[cursor..end].to_vec()));
            cursor = end;
        }
        result
    }
}

fn close_request() -> ServerOrderedRequest {
    ServerOrderedRequest::CloseModal {
        expected_root: ExpectedModalRoot::Bank,
        reason: CloseModalReason::TerminalBankExit,
    }
}

#[test]
fn ordered_native_content_needs_no_external_declaration() {
    let mut rig = Rig::new();
    rig.start([Emission::Content(close_request())]);
    rig.pump(1);
    rig.pump(2);
    let receipt = rig.receipt();
    assert_eq!(receipt.row_count, 1);
    assert_eq!(receipt.dispatched_prefix, 1);
    assert_eq!(receipt.wire_events, 1);
    assert!(receipt.terminal.is_none());
    let decoded = rig.decoded();
    assert_eq!(decoded.len(), 1);
    assert!(decoded[0].1.is_empty());
}

fn debug_catalog() -> Arc<api::debug_commands::DebugCatalog> {
    static CATALOG: std::sync::LazyLock<Arc<api::debug_commands::DebugCatalog>> =
        std::sync::LazyLock::new(|| {
            Arc::new(
                api::debug_commands::DebugCatalog::decode(
                    include_bytes!("../../api/data/game-data/289/debug.json"),
                    api::selected::ClientRevision::R289,
                    api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap(),
                )
                .unwrap(),
            )
        });
    Arc::clone(&CATALOG)
}

fn assert_no_cipher_movement(rig: &Rig) {
    assert_eq!(rig.client.out.pos, 0);
    let mut actual = rig.client.out.random.clone().unwrap();
    let mut expected = client::io::Isaac::new(&CIPHER_SEED);
    for _ in 0..16 {
        assert_eq!(actual.next_int(), expected.next_int());
    }
    assert!(rig.decoded().is_empty());
}

#[test]
fn ordered_advanced_dispatch_ignores_loading_scene_and_absent_prompt_target() {
    let mut rig = Rig::new();
    rig.client.scene_state = 0;
    rig.snapshot.rebuild(&rig.client);
    rig.start([Emission::Advanced {
        operations: vec![
            Send::if_button(8889).unwrap(),
            Send::opnpc2(1234).unwrap(),
            Send::count_dialog(-7),
            Send::event_mouse_click(3, false, 100, 200).unwrap(),
        ],
        debug: None,
    }]);
    rig.pump(1);
    rig.pump(2);
    let receipt = rig.receipt();
    assert_eq!(receipt.dispatched_prefix, 4);
    assert_eq!(receipt.wire_events, 4);
    assert!(receipt.terminal.is_none());
    let packets = rig.decoded();
    assert_eq!(packets.len(), 4);
    assert_eq!(packets[0].1, 8889u16.to_be_bytes());
    assert_eq!(packets[1].1, 1234u16.to_be_bytes());
    assert_eq!(packets[2].1, (-7i32).to_be_bytes());
}

#[test]
fn ordered_host_queued_receipt_does_not_claim_server_acceptance() {
    let mut rig = Rig::new();
    rig.start([Emission::Advanced {
        operations: vec![Send::if_button(u16::MAX.into()).unwrap()],
        debug: None,
    }]);
    rig.pump(1);
    rig.pump(1);
    let receipt = rig.receipt();
    assert_eq!(receipt.dispatched_prefix, 1);
    assert_eq!(receipt.wire_events, 1);
    assert_eq!(receipt.rows[0], RowDispatch::HostQueued { wire_events: 1 });
    assert!(receipt.terminal.is_none());
    rig.input.with_server_ordered_input(|state| {
        assert_eq!(state.snapshot().packet_count(), 1);
        assert_eq!(state.snapshot().category_counts(), [1, 0, 0]);
    });
    println!(
        "engine-refused-ciphertext: {}",
        serde_json::json!({
            "seed": CIPHER_SEED,
            "bytes": &rig.client.out.data()[..rig.client.out.pos],
            "host_queued": receipt.wire_events,
        })
    );
}

#[test]
fn ordered_advanced_unauthorized_cheat_refuses_before_every_packet_and_cipher_change() {
    let mut rig = Rig::new();
    let debug = debug_catalog();
    rig.start([Emission::Advanced {
        operations: vec![
            Send::if_button(8889).unwrap(),
            Send::client_cheat(&debug, "~cat").unwrap(),
        ],
        debug: Some(debug),
    }]);
    rig.pump(1);
    rig.pump(2);
    let receipt = rig.receipt();
    assert_eq!(receipt.row_count, 2);
    assert_eq!(receipt.dispatched_prefix, 0);
    assert_eq!(receipt.wire_events, 0);
    assert_eq!(receipt.terminal, Some(ServerOrderedError::PermissionDenied));
    assert_no_cipher_movement(&rig);
}

#[test]
fn ordered_advanced_cheat_uses_shared_catalog_arguments_and_authorization() {
    let mut rig = Rig::new();
    let debug = debug_catalog();
    for command in [
        "no-such-selected-command",
        "~addxp attack not-an-int",
        "~addxp attack 42 extra",
    ] {
        assert!(Send::client_cheat(&debug, command).is_err(), "{command}");
        assert_no_cipher_movement(&rig);
    }
    rig.client
        .set_cheat_admission(client::CheatAdmission::Granted);
    rig.start([Emission::Advanced {
        operations: vec![Send::client_cheat(&debug, "~addxp attack 42").unwrap()],
        debug: Some(debug),
    }]);
    rig.pump(1);
    rig.pump(2);
    let receipt = rig.receipt();
    assert!(receipt.terminal.is_none());
    assert_eq!(receipt.wire_events, 1);
    assert_eq!(rig.decoded()[0].1, b"~addxp attack 42\n");
}

#[test]
fn ordered_every_advanced_destructive_cheat_refuses_without_confirmation_state() {
    let debug = debug_catalog();
    let mut tested = 0;
    for command in debug
        .commands()
        .iter()
        .filter(|command| command.destructive)
    {
        let values: Vec<_> = command
            .args
            .iter()
            .take_while(|argument| !argument.optional)
            .map(|_| "1".to_owned())
            .collect();
        let formatted = command.format_command(&values).unwrap();
        let mut rig = Rig::new();
        rig.client
            .set_cheat_admission(client::CheatAdmission::Granted);
        rig.start([Emission::Advanced {
            operations: vec![
                Send::if_button(8889).unwrap(),
                Send::client_cheat(&debug, &formatted).unwrap(),
            ],
            debug: Some(Arc::clone(&debug)),
        }]);
        rig.pump(1);
        rig.pump(2);
        let receipt = rig.receipt();
        assert_eq!(
            receipt.terminal,
            Some(ServerOrderedError::DestructiveCheatConfirmationUnavailable),
            "{formatted}",
        );
        assert_eq!(receipt.dispatched_prefix, 0);
        assert_eq!(receipt.wire_events, 0);
        assert_no_cipher_movement(&rig);
        tested += 1;
    }
    assert!(
        tested > 0,
        "selected debug catalog must exercise destructive policy"
    );
}

#[test]
fn ordered_single_sink_counts_host_and_manual_packets_once_not_trace_reads() {
    let mut rig = Rig::new();
    let checkpoint = rig.client.packet_checkpoint().unwrap();
    let packet = Send::if_button(3651).unwrap();
    let opcode = packet
        .validate_for_revision(rig.client.revision())
        .unwrap()
        .opcode;
    rig.client.send_packet(&packet).unwrap();
    rig.client.out.p1_enc(i32::from(opcode));
    rig.client.out.p2(3651);
    assert_eq!(rig.client.out.committed_packet_count(), Some(2));
    let mut attributed = Vec::new();
    assert!(rig
        .client
        .trace_packets(*checkpoint, &mut |opcode| attributed.push(opcode)));
    assert_eq!(attributed, [opcode, opcode]);
    assert_eq!(rig.client.out.committed_packet_count(), Some(2));
    rig.input.with_server_ordered_input(|state| {
        let input = state.snapshot();
        assert_eq!(input.packet_count(), 2);
        assert_eq!(input.category_counts(), [2, 0, 0]);
        assert_eq!(input.input_epoch(), 2);
    });
    assert_eq!(rig.decoded().len(), 2);
}

#[test]
fn ordered_housekeeping_is_not_counted_as_dispatched_input() {
    let mut rig = Rig::new();
    for packet in [
        Send::no_timeout(),
        Send::map_build_complete(),
        Send::anticheat_oplogic1(42),
    ] {
        rig.client.send_packet(&packet).unwrap();
    }
    assert_eq!(rig.client.out.committed_packet_count(), Some(3));
    assert_eq!(rig.decoded().len(), 3);
    rig.input.with_server_ordered_input(|state| {
        let input = state.snapshot();
        assert_eq!(input.packet_count(), 0);
        assert_eq!(input.category_counts(), [0, 0, 0]);
        assert_eq!(input.input_epoch(), 0);
    });
}

#[test]
fn ordered_foreign_emission_tick_traffic_never_charges_native_batch_allowance_a2_r4() {
    let mut rig = Rig::new();
    for _ in 0..4 {
        rig.client
            .send_packet(&Send::if_button(3651).unwrap())
            .unwrap();
    }
    rig.start([Emission::Content(close_request())]);
    rig.pump(1);
    rig.pump(2);
    let receipt = rig.receipt();
    assert!(receipt.terminal.is_none());
    assert_eq!(receipt.wire_events, 1);
    assert_eq!(rig.decoded().len(), 5);
    assert_eq!(rig.client.out.committed_packet_count(), Some(5));
}

#[test]
fn ordered_mixed_producers_send_and_count_every_packet_beyond_five() {
    let mut rig = Rig::new();
    rig.client.send_packet(&Send::close_modal()).unwrap();
    let opcode = Send::close_modal()
        .validate_for_revision(rig.client.revision())
        .unwrap()
        .opcode;
    rig.client.out.p1_enc(i32::from(opcode));
    rig.start([Emission::Advanced {
        operations: (0..5).map(|_| Send::close_modal()).collect(),
        debug: None,
    }]);
    rig.pump(1);
    rig.input.with_server_ordered_input(|state| {
        let input = state.snapshot();
        assert_eq!(input.packet_count(), 7);
        assert_eq!(input.category_counts(), [7, 0, 0]);
        assert_eq!(input.input_epoch(), 7);
    });
    rig.pump(2);
    let receipt = rig.receipt();
    assert_eq!(receipt.dispatched_prefix, 5);
    assert_eq!(receipt.wire_events, 5);
    assert!(receipt.terminal.is_none());
    assert_eq!(rig.decoded().len(), 7);
    eprintln!(
        "engine-deferral-ciphertext: {}",
        serde_json::json!({
            "seed": CIPHER_SEED,
            "bytes": &rig.client.out.data()[..rig.client.out.pos],
            "committed": 7,
        })
    );
}

#[test]
fn ordered_five_drop_packets_dispatch_in_four_consecutive_observed_ticks() {
    let mut rig = Rig::new();
    rig.start((0..4).map(|_| {
        Emission::Advanced {
            operations: (0..5)
                .map(|slot| Send::opheld5(1511, slot, 3214).unwrap())
                .collect(),
            debug: None,
        }
    }));
    for tick in 1..=4 {
        rig.pump(tick);
        rig.input.with_server_ordered_input(|state| {
            let input = state.snapshot();
            assert_eq!(input.category_counts(), [5, 0, 0]);
            assert_eq!(input.packet_count(), tick * 5);
        });
    }
    rig.pump(5);
    let results = rig.results.lock().unwrap();
    assert!(results.errors.is_empty());
    assert_eq!(results.receipts.len(), 4);
    for receipt in &results.receipts {
        assert_eq!(receipt.dispatched_prefix, 5);
        assert_eq!(receipt.wire_events, 5);
        assert!(receipt.terminal.is_none());
    }
    assert_eq!(rig.decoded().len(), 20);
}

#[test]
fn ordered_counters_reset_only_on_observed_tick_not_owner_restart_or_reconnect() {
    let mut rig = Rig::new();
    rig.client
        .send_packet(&Send::if_button(3651).unwrap())
        .unwrap();
    rig.client
        .send_packet(&Send::event_mouse_click(1, false, 10, 20).unwrap())
        .unwrap();
    let before = rig
        .input
        .with_server_ordered_input(|state| state.snapshot());
    assert_eq!(before.category_counts(), [1, 1, 0]);
    for session in [rig.client.gens.session, rig.client.gens.session + 1] {
        rig.input
            .observe_input_tick(rig.client.revision(), 1, session);
        let after = rig
            .input
            .with_server_ordered_input(|state| state.snapshot());
        assert_eq!(after.category_counts(), before.category_counts());
        assert_eq!(after.packet_count(), before.packet_count());
        assert_eq!(after.input_epoch(), before.input_epoch());
    }
    rig.start([]);
    script_slot(&rig.scripts, "so3-wire")
        .unwrap()
        .lock()
        .unwrap()
        .stop();
    rig.input.bind_outbound_observer(&mut rig.client);
    let restarted = rig
        .input
        .with_server_ordered_input(|state| state.snapshot());
    assert_eq!(restarted.category_counts(), before.category_counts());
    assert_eq!(restarted.packet_count(), before.packet_count());
    rig.input
        .observe_input_tick(rig.client.revision(), 2, rig.client.gens.session);
    let later = rig
        .input
        .with_server_ordered_input(|state| state.snapshot());
    assert_eq!(later.category_counts(), [0, 0, 0]);
    assert_eq!(later.packet_count(), 2);
    assert_eq!(later.input_epoch(), 1);
    assert_eq!(rig.decoded().len(), 2);
}

fn inventory_item(id: i32, slot: i32, name: &str) -> api::snapshot::ItemView {
    api::snapshot::ItemView {
        def: api::obj_names::ItemDefView {
            id,
            name: Some(name.into()),
            stackable: false,
            members: false,
            base_value: 1,
            noted: false,
            certificate_link: -1,
            certificate_template: -1,
        },
        container: api::snapshot::ItemContainer::Inventory,
        action_family: api::snapshot::ItemActionFamily::Held,
        slot,
        count: 1,
        actions: vec![None, None, None, None, Some("Drop".into())],
        component_id: 3214,
    }
}

struct RealDropScript {
    args: Option<script::gatherer::DropBatchArgs>,
    handle: Option<ActionHandle<script::gatherer::DropBatch>>,
    result: Arc<Mutex<Option<Result<script::gatherer::DropResult, ActionError>>>>,
}

impl Script for RealDropScript {
    fn configure(
        &mut self,
        _: Arc<script::native::PreparedConfig>,
    ) -> Result<script::native::SettingsApply, script::native::ConfigError> {
        Ok(script::native::SettingsApply::Applied)
    }

    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        if let Some(args) = self.args.take() {
            self.handle = Some(
                tick.actions
                    .begin::<script::gatherer::DropBatch>(args, &mut tick.cx)
                    .expect("real DropBatch begins with an admitted fixture owner"),
            );
        }
        if let Some(handle) = &self.handle {
            if let Poll::Ready(result) = tick.actions.poll(handle, &mut tick.cx) {
                *self.result.lock().unwrap() = Some(result);
                self.handle = None;
            }
        }
        Ok(ScriptFlow::Continue)
    }
}

fn start_drop(
    rig: &Rig,
    protected: bool,
) -> Arc<Mutex<Option<Result<script::gatherer::DropResult, ActionError>>>> {
    let result = Arc::new(Mutex::new(None));
    let slot = script_slot_or_insert(&rig.scripts, "so3-wire");
    slot.lock()
        .unwrap()
        .start_test_script(
            Box::new(RealDropScript {
                args: Some(script::gatherer::DropBatchArgs {
                    products: Arc::from([1511]),
                    protected: [946, 0, 0, 0, 0, 0, 0, 0],
                    protected_len: u8::from(protected),
                }),
                handle: None,
                result: Arc::clone(&result),
            }),
            Some(api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap()),
        )
        .unwrap();
    result
}

#[test]
fn ordered_real_drop_batch_preserves_28_protected_tool_and_full_pack_pacing() {
    for (disposable, protected, full_modal, foreign) in [
        (28, false, false, false),
        (27, true, false, false),
        (28, false, true, false),
        (28, false, false, true),
    ] {
        let mut rig = Rig::new();
        let mut inventory: Vec<_> = (0..disposable)
            .map(|slot| inventory_item(1511, slot, "Logs"))
            .collect();
        if protected {
            inventory.push(inventory_item(946, 27, "Knife"));
        }
        rig.snapshot.seed_inventory(inventory.clone(), 28);
        if full_modal {
            rig.snapshot
                .seed_chat_modal(100, vec!["Your inventory is full.".into()]);
        }
        let result = start_drop(&rig, protected);
        let opcode = Send::opheld5(1511, 0, 3214)
            .unwrap()
            .validate_for_revision(rig.client.revision())
            .unwrap()
            .opcode;
        let close_opcode = Send::close_modal()
            .validate_for_revision(rig.client.revision())
            .unwrap()
            .opcode;
        let mut previous_packets = 0;
        let mut dropped_slots = HashSet::new();
        let mut by_tick = Vec::new();
        let mut done_tick = None;
        for tick in 1..=10 {
            if foreign && tick == 1 {
                for _ in 0..2 {
                    rig.client
                        .send_packet(&Send::if_button(3651).unwrap())
                        .unwrap();
                }
            }
            rig.pump(tick);
            let packets = rig.decoded();
            let newly_sent = &packets[previous_packets..];
            let drops: Vec<i32> = newly_sent
                .iter()
                .filter(|(sent, _)| *sent == opcode)
                .map(|(_, body)| {
                    assert_eq!(u16::from_be_bytes([body[0], body[1]]), 1511);
                    assert_eq!(u16::from_be_bytes([body[4], body[5]]), 3214);
                    i32::from(u16::from_be_bytes([body[2], body[3]]))
                })
                .collect();
            for slot in &drops {
                assert!(
                    dropped_slots.insert(*slot),
                    "duplicate Drop at slot {slot}, tick {tick}"
                );
            }
            let closes = newly_sent
                .iter()
                .filter(|(sent, _)| *sent == close_opcode)
                .count();
            assert!(
                drops.len() + closes <= 5,
                "consumer pacing changed at tick {tick}"
            );
            if tick == 1 {
                assert_eq!(drops.len(), if full_modal { 4 } else { 5 });
                if foreign {
                    rig.input.with_server_ordered_input(|state| {
                        assert_eq!(state.snapshot().category_counts()[0], 7);
                    });
                }
            }
            by_tick.push((tick, drops.clone(), closes));
            inventory.retain(|row| !drops.contains(&row.slot));
            rig.snapshot.seed_inventory(inventory.clone(), 28);
            if closes != 0 {
                rig.snapshot.seed_chat_modal(-1, Vec::new());
            }
            previous_packets = packets.len();
            if result.lock().unwrap().is_some() {
                done_tick = Some(tick);
                break;
            }
        }
        let done_tick = done_tick.expect("DropBatch did not settle inside ten observed ticks");
        let result = result.lock().unwrap().take().unwrap().unwrap();
        assert_eq!(result.end, script::gatherer::DropEnd::Cleared);
        assert_eq!(result.dropped, disposable as u32);
        assert_eq!(dropped_slots.len(), disposable as usize);
        assert_eq!(inventory.len(), usize::from(protected));
        if protected {
            assert_eq!((inventory[0].def.id, inventory[0].slot), (946, 27));
        }
        assert_eq!(
            done_tick, 7,
            "six dispatch ticks then one inventory settlement"
        );
        eprintln!("DropBatch disposable={disposable} protected={protected} modal={full_modal} foreign={foreign}: {by_tick:?}; final={inventory:?}; done={done_tick}; result={result:?}");
    }
}

#[test]
fn ordered_real_drop_batch_cancellation_retains_committed_prefix_without_replay() {
    let mut rig = Rig::new();
    rig.snapshot.seed_inventory(
        (0..28)
            .map(|slot| inventory_item(1511, slot, "Logs"))
            .collect(),
        28,
    );
    let result = start_drop(&rig, false);
    rig.pump(1);
    let prefix = rig.decoded();
    assert_eq!(prefix.len(), 5);
    script_slot(&rig.scripts, "so3-wire")
        .unwrap()
        .lock()
        .unwrap()
        .stop();
    for tick in 2..=4 {
        rig.pump(tick);
    }
    assert_eq!(rig.decoded(), prefix);
    assert!(result.lock().unwrap().is_none());
    assert_eq!(rig.client.out.committed_packet_count(), Some(5));
}

fn bind_waiting_bank(rig: &mut Rig) -> ServerOrderedRequest {
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let pin = selected.selected_pin().unwrap();
    let catalog = selected.server_ordered_catalog(&pin).unwrap();
    rig.snapshot.seed_bank_observation(
        catalog.bank().main_component().id(),
        1,
        Some(Vec::new()),
        Vec::new(),
    );
    rig.snapshot.seed_main_modal(
        catalog.component("bank_main:inv").unwrap().root_id(),
        Vec::new(),
    );
    rig.snapshot
        .seed_side_modal(catalog.component("bank_side:inv").unwrap().root_id());
    rig.input.with_server_ordered_input(|state| {
        state
            .bind_expectation(42, 900, ContractKey::BankDepositX, BANK_COUNT_PHASE, 8)
            .unwrap();
    });
    ServerOrderedRequest::CountResume {
        contract: CountResumeContract::BankDepositX,
        predecessor_request_id: 900,
        run_generation: 42,
        bank_session_generation: rig.snapshot.bank_session_generation(),
        value: 7,
    }
}

#[test]
fn ordered_owned_count_resumes_without_a_visible_prompt_and_consumes_its_expectation() {
    let mut rig = Rig::new();
    let request = bind_waiting_bank(&mut rig);
    assert!(!rig.client.dialog_input_open);
    rig.start([Emission::Content(request)]);
    rig.pump(1);
    rig.pump(2);
    let receipt = rig.receipt();
    assert!(receipt.terminal.is_none());
    assert_eq!(
        (
            receipt.row_count,
            receipt.dispatched_prefix,
            receipt.wire_events
        ),
        (1, 1, 1)
    );
    assert_eq!(rig.decoded().len(), 1);
    rig.input.with_server_ordered_input(|state| {
        assert!(state
            .require_expectation(42, ContractKey::BankDepositX, BANK_COUNT_PHASE, 900, 2)
            .is_err());
    });
}

#[test]
fn ordered_advanced_close_and_non_user_mouse_invalidate_bank_wait_grok_a2() {
    for mouse in [false, true] {
        let mut rig = Rig::new();
        let dependent = bind_waiting_bank(&mut rig);
        let operation = if mouse {
            Send::event_mouse_click(1, false, 10, 20).unwrap()
        } else {
            Send::close_modal()
        };
        rig.start([
            Emission::Advanced {
                operations: vec![operation],
                debug: None,
            },
            Emission::Content(dependent),
        ]);
        rig.pump(1);
        rig.input.with_server_ordered_input(|state| {
            let input = state.snapshot();
            assert_eq!(input.packet_count(), 1);
            assert_eq!(input.input_epoch(), u64::from(!mouse));
            assert_eq!(
                input.category_counts(),
                if mouse { [0, 1, 0] } else { [1, 0, 0] }
            );
            assert!(state
                .require_expectation(42, ContractKey::BankDepositX, BANK_COUNT_PHASE, 900, 1)
                .is_err());
        });
        rig.pump(2);
        rig.pump(3);
        rig.pump(4);
        let results = rig.results.lock().unwrap();
        assert!(results.errors.is_empty());
        assert_eq!(results.receipts.len(), 2);
        assert!(results.receipts[0].terminal.is_none());
        let refused = &results.receipts[1];
        assert_eq!((refused.dispatched_prefix, refused.wire_events), (0, 0));
        assert!(refused.terminal.is_some());
        assert_eq!(
            rig.decoded().len(),
            1,
            "no stale CountResume or orphan CLOSE_MODAL"
        );
    }
}

#[test]
fn ordered_foreign_movement_checked_owner_manual_and_zero_wire_edits_fence_dependents() {
    for producer in [
        "movement",
        "checked",
        "native-owner",
        "manual",
        "amount-edit",
    ] {
        let mut rig = Rig::new();
        let dependent = bind_waiting_bank(&mut rig);
        match producer {
            "movement" => rig
                .client
                .send_packet(&Send::move_gameclick(false, 6, 6, &[]).unwrap())
                .unwrap(),
            "checked" => {
                assert!(matches!(
                    api::interact::Interactions::new(&rig.snapshot, &mut rig.client).close_modal(),
                    api::interact::SendResult::Sent { .. }
                ));
            }
            "native-owner" => {
                rig.start([Emission::Content(close_request())]);
                rig.pump(1);
                rig.pump(2);
                assert!(rig.receipt().terminal.is_none());
                rig.results.lock().unwrap().receipts.clear();
                crate::script_runtime::script_slot(&rig.scripts, "so3-wire")
                    .unwrap()
                    .lock()
                    .unwrap()
                    .stop();
            }
            "manual" => {
                rig.input.set_enabled(true);
                let (tx, rx) = std::sync::mpsc::channel();
                rig.input.connect_rx(rx);
                tx.send(host::InputEv::Key { down: true, ch: 55 }).unwrap();
                rig.input.drain(&mut rig.client.shell);
            }
            "amount-edit" => {
                rig.client.dialog_input_open = true;
                assert!(
                    api::interact::Interactions::new(&rig.snapshot, &mut rig.client)
                        .apply_amount_key(true, "7")
                );
            }
            _ => unreachable!(),
        }
        rig.input.with_server_ordered_input(|state| {
            assert!(state
                .require_expectation(42, ContractKey::BankDepositX, BANK_COUNT_PHASE, 900, 2)
                .is_err());
        });
        let before = rig.decoded();
        rig.start([Emission::Content(dependent)]);
        rig.pump(3);
        rig.pump(4);
        let receipt = rig.receipt();
        assert_eq!(
            (receipt.dispatched_prefix, receipt.wire_events),
            (0, 0),
            "{producer}"
        );
        assert!(receipt.terminal.is_some(), "{producer}");
        assert_eq!(
            rig.decoded(),
            before,
            "{producer} must not authorize a stale dependent"
        );
        if matches!(producer, "manual" | "amount-edit") {
            assert!(
                before.is_empty(),
                "local input invalidates with zero wire events"
            );
        }
    }
}

fn knife_request(rig: &mut Rig, continuing: bool) -> ServerOrderedRequest {
    rig.client.map_build_base_x = 3200;
    rig.client.map_build_base_z = 3200;
    for row in &mut rig.client.collision[0].flags {
        row.fill(0);
    }
    rig.client.collision[0].flags[8][5] = client::dash3d::CollisionFlag::SQ_BLOCKED;
    rig.snapshot.rebuild(&rig.client);
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let pin = selected.selected_pin().unwrap();
    let catalog = selected.server_ordered_catalog(&pin).unwrap();
    let knife_id = catalog.knife().knife().id();
    let logs_id = catalog.knife().logs()[0].id();
    let rows = vec![
        inventory_item(knife_id, 0, "Knife"),
        inventory_item(logs_id, 1, "Logs"),
        inventory_item(logs_id, 2, "Logs"),
    ];
    rig.snapshot.seed_inventory(rows.clone(), 28);
    rig.snapshot
        .seed_inventory_packet_generations(Some(1), None);
    rig.snapshot.seed_scene(api::snapshot::SceneView {
        available: true,
        base_x: 3200,
        base_z: 3200,
        level: 0,
        width: 104,
        height: 104,
        collision_flags: vec![0; 104 * 104],
    });
    rig.snapshot.seed_locs(vec![api::snapshot::LocView {
        typecode: (1276 << 14) | (5 << 7) | 8,
        info: 0,
        id: 1276,
        name: Some("Tree".into()),
        description: None,
        actions: vec![Some("Chop down".into())],
        tile: api::snapshot::WorldTile {
            x: 3208,
            z: 3205,
            level: 0,
        },
        distance: 3,
        layer: api::snapshot::LocLayer::Ground,
        shape: 10,
        angle: 0,
        width: 1,
        length: 1,
        footprint_width: 1,
        footprint_length: 1,
        block_walk: true,
        block_range: false,
        active: true,
        animation: -1,
        map_function: -1,
        map_scene: -1,
        force_approach: 0,
    }]);
    ServerOrderedRequest::KnifeArm {
        knife: InventoryItemReference::from_snapshot(&rig.snapshot, &rows[0]).unwrap(),
        logs: InventoryItemReference::from_snapshot(&rig.snapshot, &rows[1]).unwrap(),
        shape: if continuing {
            KnifeArmShape::Continuing {
                click: ExactLocOrNpcRef::Loc(
                    ExactLocReference::from_snapshot(&rig.snapshot, &rig.snapshot.locs()[0], 1)
                        .unwrap(),
                ),
            }
        } else {
            KnifeArmShape::Ending
        },
    }
}

#[test]
fn ordered_knife_ending_and_continuing_use_exact_unseen_make_and_move_op_order() {
    for continuing in [false, true] {
        let mut rig = Rig::new();
        let request = knife_request(&mut rig, continuing);
        assert_ne!(rig.snapshot.modals().chat, 8880);
        assert_ne!(rig.snapshot.modals().main, 8880);
        rig.start([Emission::Content(request)]);
        rig.pump(1);
        rig.pump(2);
        let receipt = rig.receipt();
        assert!(receipt.terminal.is_none(), "{receipt:?}");
        assert_eq!(receipt.row_count, if continuing { 3 } else { 2 });
        assert_eq!(receipt.dispatched_prefix, receipt.row_count);
        assert_eq!(receipt.wire_events, if continuing { 4 } else { 2 });
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let pin = selected.selected_pin().unwrap();
        let catalog = selected.server_ordered_catalog(&pin).unwrap();
        let mut expected = vec![
            Send::opheldu(
                catalog.knife().logs()[0].id(),
                1,
                3214,
                catalog.knife().knife().id(),
                0,
                3214,
            )
            .unwrap(),
            Send::if_button(catalog.component("skill_multi3:make1_a").unwrap().id()).unwrap(),
        ];
        if continuing {
            expected.push(Send::move_gameclick(false, 3207, 3205, &[]).unwrap());
            expected.push(Send::oploc1(3208, 3205, 1276).unwrap());
        }
        let expected: Vec<_> = expected
            .iter()
            .map(|packet| {
                (
                    packet
                        .validate_for_revision(rig.client.revision())
                        .unwrap()
                        .opcode,
                    packet.body().to_vec(),
                )
            })
            .collect();
        assert_eq!(rig.decoded(), expected);
    }
}

#[test]
fn ordered_knife_last_row_static_failure_has_zero_prefix_bytes_and_cipher_movement() {
    let mut rig = Rig::new();
    let request = knife_request(&mut rig, true);
    rig.snapshot.seed_locs(Vec::new());
    let mut before = rig.client.out.random.clone().unwrap();
    rig.start([Emission::Content(request)]);
    rig.pump(1);
    rig.pump(2);
    let receipt = rig.receipt();
    assert_eq!((receipt.dispatched_prefix, receipt.wire_events), (0, 0));
    assert_eq!(receipt.rows[0], RowDispatch::NotAttempted);
    assert_eq!(receipt.rows[1], RowDispatch::NotAttempted);
    assert!(matches!(receipt.rows[2], RowDispatch::Refused { .. }));
    assert!(rig.decoded().is_empty());
    let mut after = rig.client.out.random.clone().unwrap();
    assert_eq!(before.next_int(), after.next_int());
}

#[test]
fn ordered_knife_stale_modal_stale_reference_and_each_wrong_item_role_emit_nothing() {
    for failure in ["modal", "inventory-stamp", "knife-role", "logs-role"] {
        let mut rig = Rig::new();
        let mut request = knife_request(&mut rig, false);
        match failure {
            "modal" => rig.snapshot.seed_chat_modal(8880, Vec::new()),
            "inventory-stamp" => rig
                .snapshot
                .seed_inventory_packet_generations(Some(2), None),
            "knife-role" | "logs-role" => {
                let mut rows = rig.snapshot.inventory().to_vec();
                let role = usize::from(failure == "logs-role");
                rows[role] = inventory_item(1, role as i32, "Wrong item");
                rig.snapshot.seed_inventory(rows.clone(), 28);
                request = ServerOrderedRequest::KnifeArm {
                    knife: InventoryItemReference::from_snapshot(&rig.snapshot, &rows[0]).unwrap(),
                    logs: InventoryItemReference::from_snapshot(&rig.snapshot, &rows[1]).unwrap(),
                    shape: KnifeArmShape::Ending,
                };
            }
            _ => unreachable!(),
        }
        let mut before = rig.client.out.random.clone().unwrap();
        rig.start([Emission::Content(request)]);
        rig.pump(1);
        rig.pump(2);
        let receipt = rig.receipt();
        assert_eq!(
            (receipt.dispatched_prefix, receipt.wire_events),
            (0, 0),
            "{failure}"
        );
        if failure == "modal" {
            assert_eq!(receipt.terminal, Some(ServerOrderedError::StaleModal));
        } else {
            assert!(receipt.terminal.is_some(), "{failure}");
        }
        assert!(rig.decoded().is_empty(), "{failure}");
        let mut after = rig.client.out.random.clone().unwrap();
        assert_eq!(before.next_int(), after.next_int(), "{failure}");
    }
}

#[test]
fn ordered_delayed_knife_does_not_charge_later_drain_tick_checked_drop() {
    for continuing in [false, true] {
        let mut rig = Rig::new();
        let request = knife_request(&mut rig, continuing);
        rig.start([Emission::ContentThenDrop(request)]);
        crate::script_runtime::delay_ordered_drain_for_test();
        rig.pump(1);
        assert!(rig.decoded().is_empty());
        rig.pump(2);
        assert_eq!(rig.decoded().len(), if continuing { 4 } else { 2 });
        rig.pump(2);
        let decoded = rig.decoded();
        assert_eq!(decoded.len(), if continuing { 5 } else { 3 });
        assert_eq!(
            decoded.last().unwrap().0,
            Send::opheld5(1511, 2, 3214)
                .unwrap()
                .validate_for_revision(rig.client.revision())
                .unwrap()
                .opcode
        );
        rig.input.with_server_ordered_input(|state| {
            assert_eq!(state.snapshot().category_counts()[0], decoded.len() as u32);
        });
    }
}

struct CancelAfterPacket {
    input: Arc<SlotInput>,
    authority: Mutex<Option<script::native::HostAuthority>>,
    remaining: std::sync::atomic::AtomicUsize,
}

impl client::io::PacketObserver for CancelAfterPacket {
    fn packet_body_length(&self, opcode: u8) -> i16 {
        client::io::PacketObserver::packet_body_length(self.input.as_ref(), opcode)
    }

    fn packet_committed(&self, opcode: u8, encoded_length: usize) {
        client::io::PacketObserver::packet_committed(self.input.as_ref(), opcode, encoded_length);
        if self
            .remaining
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst)
            == 1
        {
            self.authority
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .cancel_request_for_test();
        }
    }

    fn committed_packet_count(&self) -> Option<u64> {
        client::io::PacketObserver::committed_packet_count(self.input.as_ref())
    }

    fn observed_tick(&self, tick: u64, session: u64) {
        client::io::PacketObserver::observed_tick(self.input.as_ref(), tick, session);
    }

    fn foreign_local_input(&self) {
        self.input.foreign_local_input();
    }
}

#[test]
fn ordered_cancellation_before_row_zero_between_rows_and_inside_move_op_keeps_exact_prefix() {
    for committed in 0..=3 {
        let mut rig = Rig::new();
        let request = knife_request(&mut rig, true);
        if committed == 0 {
            crate::script_runtime::install_ordered_dispatch_hook(Box::new(|authority| {
                authority.cancel_request_for_test();
            }));
        } else {
            let observer = Arc::new(CancelAfterPacket {
                input: Arc::clone(&rig.input),
                authority: Mutex::new(None),
                remaining: std::sync::atomic::AtomicUsize::new(committed),
            });
            let hook_observer = Arc::clone(&observer);
            crate::script_runtime::install_ordered_dispatch_hook(Box::new(move |authority| {
                *hook_observer.authority.lock().unwrap() = Some(authority.clone());
            }));
            rig.client.out.set_observer(Some(observer));
        }
        rig.start([Emission::Content(request)]);
        rig.pump(1);
        rig.pump(1);
        let receipt = rig.receipt();
        assert_eq!(
            receipt.terminal,
            Some(ServerOrderedError::Cancelled),
            "{receipt:?}"
        );
        assert_eq!(receipt.wire_events as usize, committed, "{receipt:?}");
        let completed_rows = committed.min(2);
        assert_eq!(receipt.dispatched_prefix as usize, completed_rows);
        assert_eq!(
            receipt.rows[completed_rows],
            RowDispatch::Refused {
                reason: ServerOrderedError::Cancelled
            }
        );
        for row in &receipt.rows[completed_rows + 1..receipt.row_count as usize] {
            assert_eq!(*row, RowDispatch::SkippedDependency);
        }
        assert_eq!(rig.decoded().len(), committed);
        rig.input.with_server_ordered_input(|state| {
            assert_eq!(state.snapshot().packet_count(), committed as u64);
        });
        // Later pumps must not replay either the prefix or the suffix.
        rig.pump(3);
        assert_eq!(rig.decoded().len(), committed);
    }
}

#[test]
fn ordered_four_packet_requests_repeat_on_consecutive_observed_ticks() {
    let mut rig = Rig::new();
    let requests = (0..4)
        .map(|_| Emission::Content(knife_request(&mut rig, true)))
        .collect::<Vec<_>>();
    rig.start(requests);
    for tick in 1..=5 {
        rig.pump(tick);
        if tick <= 4 {
            rig.input.with_server_ordered_input(|state| {
                assert_eq!(state.snapshot().category_counts()[0], 4, "tick {tick}");
            });
        }
    }
    let results = rig.results.lock().unwrap();
    assert!(results.errors.is_empty(), "{:?}", results.errors);
    assert_eq!(results.receipts.len(), 4);
    for (index, receipt) in results.receipts.iter().enumerate() {
        assert_eq!(receipt.dispatch_tick, index as u64 + 1);
        assert_eq!((receipt.dispatched_prefix, receipt.wire_events), (3, 4));
        assert!(receipt.terminal.is_none());
    }
    assert_eq!(rig.decoded().len(), 16);
}

#[test]
fn ordered_delayed_request_counts_later_foreign_traffic_without_rate_admission() {
    let mut rig = Rig::new();
    let request = knife_request(&mut rig, true);
    rig.start([Emission::Content(request)]);
    crate::script_runtime::delay_ordered_drain_for_test();
    rig.pump(1);
    assert!(rig.decoded().is_empty());
    rig.input
        .observe_input_tick(rig.client.revision(), 2, rig.client.gens.session);
    for _ in 0..3 {
        rig.client.send_packet(&Send::close_modal()).unwrap();
    }
    rig.pump(2);
    rig.pump(3);
    let receipt = rig.receipt();
    assert_eq!(
        (
            receipt.dispatch_tick,
            receipt.dispatched_prefix,
            receipt.wire_events
        ),
        (2, 3, 4)
    );
    assert!(receipt.terminal.is_none());
    let decoded = rig.decoded();
    assert_eq!(decoded.len(), 7);
    let opcode = Send::close_modal()
        .validate_for_revision(rig.client.revision())
        .unwrap()
        .opcode;
    assert!(decoded[..3].iter().all(|packet| packet.0 == opcode));
    rig.input.with_server_ordered_input(|state| {
        assert_eq!(state.snapshot().packet_count(), 7);
    });
}

#[test]
fn ordered_dispatch_does_not_relax_ordinary_invisible_ui_or_target_mask_checks() {
    use api::interact::{Interactions, OpTarget, SendReason, SendResult};
    use client::config::if_type::{ButtonType, ComponentType, IfType, IfTypeMut};
    let mut rig = Rig::new();
    let _ = knife_request(&mut rig, false);
    let locs = rig.snapshot.locs().to_vec();
    rig.client.set_iface(
        100,
        IfType {
            id: 100,
            layer_id: 100,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![101, 102]),
            ..Default::default()
        },
    );
    for (id, button_type) in [
        (101, ButtonType::BUTTON_TARGET),
        (102, ButtonType::BUTTON_OK),
    ] {
        rig.client.set_iface(
            id,
            IfType {
                id: id as i32,
                layer_id: 100,
                r#type: ComponentType::TYPE_TEXT,
                target_mask: 0x8, // Only players, not the exact loc fixture.
                ..Default::default()
            },
        );
        rig.client.set_iface_mut(
            id,
            IfTypeMut {
                button_type,
                ..Default::default()
            },
        );
    }
    rig.client.main_modal_id = 100;
    rig.client.gens.iface += 1;
    rig.snapshot.rebuild(&rig.client);
    rig.snapshot.seed_locs(locs.clone());
    let target = rig
        .snapshot
        .widgets()
        .iter()
        .find(|widget| widget.component_id == 101)
        .unwrap()
        .clone();
    let button = rig
        .snapshot
        .widgets()
        .iter()
        .find(|widget| widget.component_id == 102)
        .unwrap()
        .clone();
    assert!(matches!(
        Interactions::new(&rig.snapshot, &mut rig.client)
            .use_widget_on(&target, OpTarget::Loc(&rig.snapshot.locs()[0])),
        SendResult::Refused {
            reason: SendReason::TargetMaskMismatch,
            ..
        }
    ));
    assert!(!dispatch_script_interact_cached(
        &mut rig.client,
        &rig.snapshot,
        None,
        None,
        &rig.navs,
        &None,
        None,
        None,
        "so3-wire",
        vec![script::shim::InteractReq::UseWidgetOn {
            component_id: 101,
            kind: "loc".into(),
            target_name: None,
            x: 3208,
            z: 3205,
            level: 0,
            index: None,
        }],
        None,
        None,
    ));
    rig.client.main_modal_id = -1;
    rig.client.gens.iface += 1;
    rig.snapshot.rebuild(&rig.client);
    rig.snapshot.seed_locs(locs);
    let mut checked = Interactions::new(&rig.snapshot, &mut rig.client);
    assert!(matches!(checked.press(&button), SendResult::Refused { .. }));
    assert!(matches!(
        checked.if_button(&button),
        SendResult::Refused { .. }
    ));
    assert!(matches!(
        checked.answer_choice(1),
        SendResult::Refused { .. }
    ));
    assert!(matches!(
        checked.use_widget_on(&target, OpTarget::Loc(&rig.snapshot.locs()[0])),
        SendResult::Refused { .. }
    ));
    assert!(!dispatch_script_interact_cached(
        &mut rig.client,
        &rig.snapshot,
        None,
        None,
        &rig.navs,
        &None,
        None,
        None,
        "so3-wire",
        vec![
            script::shim::InteractReq::IfButton { component_id: 102 },
            script::shim::InteractReq::Answer { option: 1 },
            script::shim::InteractReq::UseWidgetOn {
                component_id: 101,
                kind: "loc".into(),
                target_name: None,
                x: 3208,
                z: 3205,
                level: 0,
                index: None,
            },
        ],
        None,
        None,
    ));
    assert_no_cipher_movement(&rig);
    rig.input.with_server_ordered_input(|state| {
        assert_eq!(state.snapshot().packet_count(), 0);
    });
}

#[test]
fn ordered_observation_survives_real_client_socket_adoption_without_reset() {
    let mut receiving = Rig::new();
    let mut prepared = Rig::new();
    receiving.client.send_packet(&Send::close_modal()).unwrap();
    let before = receiving
        .input
        .with_server_ordered_input(|state| state.snapshot());
    assert_eq!(before.packet_count(), 1);
    assert_eq!(receiving.client.adopt_from(&mut prepared.client), Some(()));
    receiving.client.send_packet(&Send::close_modal()).unwrap();
    let after = receiving
        .input
        .with_server_ordered_input(|state| state.snapshot());
    assert_eq!(after.packet_count(), 2);
    assert_eq!(after.category_counts(), [2, 0, 0]);
    assert_eq!(receiving.client.out.committed_packet_count(), Some(2));
    assert_eq!(
        prepared
            .input
            .with_server_ordered_input(|state| state.snapshot().packet_count()),
        0
    );
    assert_eq!(
        receiving.decoded().len(),
        1,
        "the adopted sink owns only its new committed bytes"
    );
}

#[test]
fn ordered_existing_compat_loop_keeps_twenty_eight_immediate_true_drops_without_host_rate_refusal()
{
    let mut rig = Rig::new();
    {
        let cache = Arc::get_mut(&mut rig.client.cache).unwrap();
        cache.objs.resize(1539, client::config::ObjType::default());
        for slot in 0..28 {
            let id = 1511 + slot;
            cache.objs[id].id = id as i32;
            cache.objs[id].name = format!("Disposable {slot}");
            cache.objs[id].iop[4] = Some("Drop".into());
        }
    }
    rig.names = Arc::new(api::obj_names::ObjNames::from_objs(&rig.client.cache.objs));
    rig.snapshot.seed_inventory(
        (0..28)
            .map(|slot| inventory_item(1511 + slot, slot, &format!("Disposable {slot}")))
            .collect(),
        28,
    );
    t23_bank_start_compat_slot(
        &rig.scripts,
        "so3-wire",
        r#"
import { Input } from '../../input/Input.js';
export default class T extends LoopingBot {
    loop() {
        if (globalThis.__sent) return;
        globalThis.__sent = true;
        globalThis.__returns = [];
        for (let slot = 0; slot < 28; slot++) {
            globalThis.__returns.push(Input.heldOp(1511 + slot, slot, 3214, 1));
        }
    }
}
"#,
    );
    rig.pump(1);
    let slot = script_slot(&rig.scripts, "so3-wire").unwrap();
    let immediate = slot.lock().unwrap().probe("globalThis.__returns").unwrap();
    assert_eq!(immediate, serde_json::json!(vec![true; 28]));
    // A second same-tick pump consumes the asynchronous isolate's original queue.
    rig.pump(1);
    let decoded = rig.decoded();
    assert_eq!(decoded.len(), 28);
    let opcode = Send::opheld5(1511, 0, 3214)
        .unwrap()
        .validate_for_revision(rig.client.revision())
        .unwrap()
        .opcode;
    for (slot, (sent, body)) in decoded.iter().enumerate() {
        assert_eq!(*sent, opcode);
        assert_eq!(u16::from_be_bytes([body[0], body[1]]), 1511 + slot as u16);
        assert_eq!(u16::from_be_bytes([body[2], body[3]]), slot as u16);
        assert_eq!(u16::from_be_bytes([body[4], body[5]]), 3214);
    }
    rig.input.with_server_ordered_input(|state| {
        assert_eq!(state.snapshot().category_counts(), [28, 0, 0]);
        assert_eq!(state.snapshot().packet_count(), 28);
    });
    rig.pump(2);
    assert_eq!(
        rig.decoded(),
        decoded,
        "the host does not replay deferred compat input"
    );
    slot.lock().unwrap().stop();
}

#[test]
fn ordered_existing_shop_move_op_npc_spills_to_engine_without_host_deferral_or_replay() {
    let mut rig = Rig::new();
    let _ = knife_request(&mut rig, false);
    {
        let cache = Arc::get_mut(&mut rig.client.cache).unwrap();
        cache.npcs.resize_with(10, client::config::NpcType::default);
        cache.npcs[9] = client::config::NpcType {
            id: 9,
            name: "Shopkeeper".into(),
            op: vec![None, Some("Trade".into())],
            ..Default::default()
        };
    }
    let mut npc = client::dash3d::ClientNpc::default();
    npc.entity.x = 8 * 128;
    npc.entity.z = 5 * 128;
    npc.r#type = Some(9);
    rig.client.npc[7] = Some(Box::new(npc));
    rig.client.npc_ids = vec![7];
    rig.client.npc_count = 1;
    rig.client.gens.npc += 1;
    rig.snapshot.rebuild(&rig.client);
    for _ in 0..4 {
        rig.client.send_packet(&Send::close_modal()).unwrap();
    }
    assert!(dispatch_script_interact_cached(
        &mut rig.client,
        &rig.snapshot,
        None,
        None,
        &rig.navs,
        &None,
        None,
        None,
        "so3-wire",
        vec![script::shim::InteractReq::Npc {
            name: "Shopkeeper".into(),
            action: "Trade".into(),
            index: Some(7),
        }],
        None,
        None,
    ));
    let decoded = rig.decoded();
    assert_eq!(decoded.len(), 6);
    let movement = Send::move_opclick(false, 3207, 3205, &[])
        .unwrap()
        .validate_for_revision(rig.client.revision())
        .unwrap()
        .opcode;
    let operation = Send::opnpc2(7)
        .unwrap()
        .validate_for_revision(rig.client.revision())
        .unwrap()
        .opcode;
    assert_eq!((decoded[4].0, decoded[5].0), (movement, operation));
    rig.input.with_server_ordered_input(|state| {
        assert_eq!(state.snapshot().category_counts(), [6, 0, 0]);
    });
    rig.pump(2);
    assert_eq!(rig.decoded(), decoded);
}

#[test]
fn ordered_real_drain_releases_one_owner_and_one_completion_for_every_terminal_outcome() {
    let mut rig = Rig::new();
    let requests = (0..40)
        .map(|index| match index % 4 {
            0 | 3 => Emission::Content(close_request()),
            1 => Emission::Advanced {
                operations: vec![Send::close_modal()],
                debug: None,
            },
            2 => Emission::Content(knife_request(&mut rig, false)),
            _ => unreachable!(),
        })
        .collect::<Vec<_>>();
    rig.snapshot
        .seed_inventory_packet_generations(Some(2), None);
    rig.start(requests);
    for tick in 1..=41 {
        if tick <= 40 && tick % 4 == 0 {
            crate::script_runtime::install_ordered_dispatch_hook(Box::new(|authority| {
                authority.cancel_request_for_test();
            }));
        }
        rig.pump(tick);
    }
    let results = rig.results.lock().unwrap();
    assert!(results.errors.is_empty(), "{:?}", results.errors);
    assert_eq!(results.receipts.len(), 40);
    assert_eq!(
        results
            .receipts
            .iter()
            .map(|receipt| receipt.request_id)
            .collect::<HashSet<_>>()
            .len(),
        40
    );
    for (index, receipt) in results.receipts.iter().enumerate() {
        let expected = match index % 4 {
            0 | 1 => None,
            2 => Some(ServerOrderedError::StaleTarget),
            3 => Some(ServerOrderedError::Cancelled),
            _ => unreachable!(),
        };
        assert_eq!(receipt.terminal, expected, "request {index}: {receipt:?}");
        assert_eq!(receipt.wire_events, u8::from(expected.is_none()));
        assert_eq!(receipt.dispatched_prefix, u8::from(expected.is_none()));
    }
    assert_eq!(rig.decoded().len(), 20);
    assert_eq!(rig.client.out.committed_packet_count(), Some(20));
}

#[test]
fn ordered_missing_sink_observation_refuses_before_cipher_or_payload_movement() {
    let mut rig = Rig::new();
    rig.client.out.set_observer(None);
    rig.start([Emission::Content(close_request())]);
    rig.pump(1);
    rig.pump(2);
    let receipt = rig.receipt();
    assert_eq!(
        receipt.terminal,
        Some(ServerOrderedError::ObservationUnavailable)
    );
    assert_eq!((receipt.dispatched_prefix, receipt.wire_events), (0, 0));
    assert_no_cipher_movement(&rig);
}

#[test]
fn ordered_mismatched_selected_revision_refuses_before_packet_or_cipher_movement() {
    let mut rig = Rig::new_with_revision(client::client::ClientRevision::R274);
    rig.start([Emission::Content(close_request())]);
    rig.pump(1);
    rig.pump(2);
    let receipt = rig.receipt();
    assert_eq!(receipt.terminal, Some(ServerOrderedError::RevisionMismatch));
    assert_eq!((receipt.dispatched_prefix, receipt.wire_events), (0, 0));
    assert_no_cipher_movement(&rig);
}

#[test]
fn ordered_invalid_bank_expectations_and_conflicting_cleanup_emit_no_prefix() {
    for failure in [
        "missing",
        "expired",
        "predecessor",
        "bank-session",
        "prompt",
    ] {
        let mut rig = Rig::new();
        let mut request = bind_waiting_bank(&mut rig);
        let tick = match failure {
            "missing" => {
                rig.input
                    .with_server_ordered_input(|state| state.invalidate_content_expectation());
                1
            }
            "expired" => 9,
            "predecessor" => {
                if let ServerOrderedRequest::CountResume {
                    predecessor_request_id,
                    ..
                } = &mut request
                {
                    *predecessor_request_id += 1;
                }
                1
            }
            "bank-session" => {
                if let ServerOrderedRequest::CountResume {
                    bank_session_generation,
                    ..
                } = &mut request
                {
                    *bank_session_generation += 1;
                }
                1
            }
            "prompt" => {
                rig.snapshot
                    .seed_chat_modal(4882, vec!["Unrelated chat".into()]);
                request = close_request();
                1
            }
            _ => unreachable!(),
        };
        rig.start([Emission::Content(request)]);
        rig.pump(tick);
        rig.pump(tick + 1);
        let receipt = rig.receipt();
        let expected = match failure {
            "missing" | "expired" => ServerOrderedError::MissingExpectation,
            "predecessor" => ServerOrderedError::InvalidatedExpectation,
            "bank-session" => ServerOrderedError::StaleTarget,
            "prompt" => ServerOrderedError::ConflictingPrompt,
            _ => unreachable!(),
        };
        assert_eq!(receipt.terminal, Some(expected), "{failure}: {receipt:?}");
        assert_eq!((receipt.dispatched_prefix, receipt.wire_events), (0, 0));
        assert_no_cipher_movement(&rig);
    }
}

#[test]
fn ordered_invalid_request_shapes_fail_at_admission_without_cipher_or_payload_movement() {
    for rows in [0, 6] {
        let mut rig = Rig::new();
        rig.start([Emission::Advanced {
            operations: vec![Send::close_modal(); rows],
            debug: None,
        }]);
        rig.pump(1);
        let results = rig.results.lock().unwrap();
        assert!(results.receipts.is_empty());
        assert!(matches!(
            results.errors.as_slice(),
            [ActionError::ServerOrdered(ServerOrderedError::InvalidShape)]
        ));
        assert_no_cipher_movement(&rig);
    }
    let mut rig = Rig::new();
    let mut request = bind_waiting_bank(&mut rig);
    if let ServerOrderedRequest::CountResume { value, .. } = &mut request {
        *value = 0;
    }
    rig.start([Emission::Content(request)]);
    rig.pump(1);
    assert!(matches!(
        rig.results.lock().unwrap().errors.as_slice(),
        [ActionError::ServerOrdered(ServerOrderedError::InvalidShape)]
    ));
    assert_no_cipher_movement(&rig);
}

#[test]
fn ordered_stale_dispatch_observation_refuses_before_cipher_or_payload_movement() {
    let mut rig = Rig::new();
    let input = Arc::clone(&rig.input);
    crate::script_runtime::install_ordered_dispatch_hook(Box::new(move |_| {
        input.with_server_ordered_input(|state| state.observe_tick(2));
    }));
    rig.start([Emission::Content(close_request())]);
    rig.pump(1);
    rig.pump(2);
    let receipt = rig.receipt();
    assert_eq!(receipt.terminal, Some(ServerOrderedError::StaleOwner));
    assert_eq!((receipt.dispatched_prefix, receipt.wire_events), (0, 0));
    assert_no_cipher_movement(&rig);
}

#[test]
fn ordered_advanced_cheat_without_shared_catalog_refuses_the_whole_request() {
    let mut rig = Rig::new();
    rig.client
        .set_cheat_admission(client::CheatAdmission::Granted);
    let debug = debug_catalog();
    rig.start([Emission::Advanced {
        operations: vec![
            Send::close_modal(),
            Send::client_cheat(&debug, "~cat").unwrap(),
        ],
        debug: None,
    }]);
    rig.pump(1);
    rig.pump(2);
    let receipt = rig.receipt();
    assert_eq!(receipt.terminal, Some(ServerOrderedError::PermissionDenied));
    assert_eq!((receipt.dispatched_prefix, receipt.wire_events), (0, 0));
    assert_no_cipher_movement(&rig);
}
