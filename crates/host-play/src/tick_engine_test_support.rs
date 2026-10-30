//! Test-support-only observations and mutations for the real Gatherer tick
//! engine harness. The packet observer delegates every callback to the live
//! slot observer; it changes no bytes, cipher state, or authority rules.

use api::interact::server_ordered::{RowDispatch, ServerOrderedDispatch};
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::{Client, LocalAnimationUpdate};
use client::io::PacketObserver;
use host::SlotInput;
use script::native::HostAuthority;
use script::surface::ServerOrderedEnvelope;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, LazyLock, Mutex, Weak};
use std::time::Duration;

thread_local! {
    static CURRENT_HOOK: RefCell<Option<Arc<TickEngineTestHook>>> = const { RefCell::new(None) };
}

static REGISTERED_HOOKS: LazyLock<Mutex<HashMap<String, Weak<TickEngineTestHook>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// A real transport mutation performed by the loopback test proxy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportMutation {
    /// Hold the encoded request until one host frame after the next observed
    /// PLAYER_INFO edge.
    HoldThroughNextEdge,
    /// Hold encoded bytes until the next real content dispatch begins.
    HoldUntilNextDispatch,
    /// Join bytes from a later real dispatch to a held multi-request gate.
    JoinActiveGate,
    /// Forward the first packet, then hold the remaining stream bytes through
    /// the next observed PLAYER_INFO edge.
    SplitAfterFirstPacket,
    /// Forward the first `packet_count` encoded packets and drop the
    /// remaining bytes at the proxy. The host dispatcher still completes.
    DropSuffixAfterPacket(usize),
}

/// Controls a byte-preserving proxy between the real encrypted Client and the
/// configured loopback engine. Implementations never decode or rewrite bytes.
pub trait OrderedTransportController: Send + Sync {
    /// Begin gating the client-to-engine stream before the real dispatcher emits.
    fn begin_dispatch(
        &self,
        request_id: u64,
        mutation: TransportMutation,
        game_tick: u32,
        host_frame: u64,
    );

    /// Report one packet that the real Client encoded into its outgoing stream.
    fn packet_committed(
        &self,
        request_id: u64,
        packet_index: usize,
        opcode: u8,
        encoded_length: usize,
    );

    /// Advance the release condition using an observed host frame.
    fn observed_frame(&self, game_tick: u32, tick_edge: bool, host_frame: u64);
    /// Finish request-scoped gating when the real dispatcher returns.
    fn end_dispatch(&self, request_id: u64, host_frame: u64);
}

/// A snapshot change authorized only for a named live test case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchMutation {
    /// Remove one real scene loc immediately before the API's normal preflight.
    RemoveLoc { id: i32, tile: WorldTile },
    /// Revoke the real native request after this many packets were encoded.
    CancelAfterPackets(usize),
    /// Gate actual encoded bytes through the installed loopback proxy.
    Transport(TransportMutation),
}

/// Whether a frame was first observed on a PLAYER_INFO edge or a wake poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollKind {
    /// The frame carried a new game tick.
    Edge,
    /// The frame observed a family change without a new game tick.
    Wake,
}

/// One newly first-seen chat line, attributed to its actual host poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatObservation {
    /// Snapshot game tick when the line was first visible.
    pub game_tick: u32,
    /// Client chat-ring sequence assigned by the real Client.
    pub sequence: i32,
    /// Text from the real snapshot.
    pub text: String,
    /// Poll kind which first observed this sequence.
    pub poll: PollKind,
}

/// One host observation of the real encrypted Client session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameObservation {
    /// Monotonic host-observation index since this hook was created.
    pub host_frame: u64,
    /// Snapshot tick, advanced only by observed PLAYER_INFO.
    pub game_tick: u32,
    /// Whether this observation is the PLAYER_INFO edge.
    pub tick_edge: bool,
    /// The Client's actual animation update, including its reception serial.
    pub animation: Option<LocalAnimationUpdate>,
    /// Newly first-seen chat rows on this poll.
    pub chat: Vec<ChatObservation>,
    /// Inventory counts requested by the live test.
    pub inventory_counts: Vec<(i32, i32)>,
    /// Count of the configured log item in the real Client inventory.
    pub log_count: i32,
    /// XP for every skill slot in the real Client snapshot.
    pub skill_xp: Vec<(i32, i32)>,
    /// Real local-player tile from this snapshot, when present.
    pub player_tile: Option<WorldTile>,
    /// The chat modal root visible to the script after test-only filtering.
    pub chat_modal_root: i32,
    /// The original Client chat root before the test-only snapshot filter.
    pub actual_chat_modal_root: i32,
    /// Actual inbound IF_CLOSE packet count for this Client session.
    pub server_if_close_packets: u64,
    /// Actual main-modal open observation stamp for this Client session.
    pub main_modal_opened_observation: u64,
    /// Actual main-modal close observation stamp for this Client session.
    pub main_modal_closed_observation: u64,
    /// True only when the test hook suppressed a real IF_OPENCHAT snapshot.
    pub suppressed_openchat: bool,
}

/// A dispatch result captured from the real ServerOrderedInputs dispatcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchObservation {
    /// Receipt returned by the real API planner and encrypted Client driver.
    pub receipt: ServerOrderedDispatch,
    /// Debug rendering of the real typed request at dispatch time.
    pub request_debug: Option<String>,
    /// Whether the requested test-only snapshot mutation actually changed it.
    pub mutation_applied: bool,
}
/// One packet emitted by the real encrypted Client during measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireObservation {
    /// Host observation index active when the Client committed the packet.
    pub host_frame: u64,
    /// Snapshot tick active when the Client committed the packet.
    pub game_tick: u32,
    /// Native request which owned this packet, if it was inside a dispatch.
    pub request_id: Option<u64>,
    /// Revision-specific encrypted packet opcode.
    pub opcode: u8,
    /// Encoded bytes, including opcode and any variable-length prefix.
    pub encoded_length: usize,
}

/// Bounded trace for one real-engine live case.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TickEngineTrace {
    /// Host observations in arrival order.
    pub frames: Vec<FrameObservation>,
    /// Receipts in actual dispatcher completion order.
    pub dispatches: Vec<DispatchObservation>,
    /// Actual encrypted Client packets emitted during measurement.
    pub wire: Vec<WireObservation>,
}

struct ActiveDispatch {
    request_id: u64,
    authority: HostAuthority,
    packets: usize,
    cancel_after: Option<usize>,
    transport: Option<TransportMutation>,
}

struct HookState {
    trace: TickEngineTrace,
    next_host_frame: u64,
    last_game_tick: u32,
    last_chat_sequence: i32,
    inventory_ids: Vec<i32>,
    log_id: i32,
    measurement_armed: bool,
    pending_mutations: VecDeque<DispatchMutation>,
    active_dispatch: Option<ActiveDispatch>,
    transport: Option<Arc<dyn OrderedTransportController>>,
    observer_session: Option<u64>,
    suppress_openchat_through_tick: Option<u32>,
}

/// A session-local test hook. It is never installed by a product path.
pub struct TickEngineTestHook {
    account: String,
    state: Mutex<HookState>,
    changed: Condvar,
}

impl TickEngineTestHook {
    /// Create a hook scoped to one exact Play slot name.
    pub fn new(account: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            account: account.into(),
            state: Mutex::new(HookState {
                trace: TickEngineTrace::default(),
                next_host_frame: 0,
                last_game_tick: 0,
                last_chat_sequence: 0,
                inventory_ids: Vec::new(),
                log_id: 1511,
                measurement_armed: false,
                pending_mutations: VecDeque::new(),
                active_dispatch: None,
                transport: None,
                observer_session: None,
                suppress_openchat_through_tick: None,
            }),
            changed: Condvar::new(),
        })
    }

    /// Register this hook for the named slot before starting the live Play.
    pub fn install(self: &Arc<Self>) {
        REGISTERED_HOOKS
            .lock()
            .expect("tick-engine hook registry poisoned")
            .insert(self.account.clone(), Arc::downgrade(self));
    }

    /// Remove this exact hook from future slot frames.
    pub fn uninstall(&self) {
        let mut hooks = REGISTERED_HOOKS
            .lock()
            .expect("tick-engine hook registry poisoned");
        if hooks
            .get(&self.account)
            .and_then(Weak::upgrade)
            .is_some_and(|hook| std::ptr::eq(Arc::as_ptr(&hook), self))
        {
            hooks.remove(&self.account);
        }
    }

    /// Bind the current slot worker to its registered live test hook.
    pub(crate) fn bind_current_for_account(account: &str) -> Option<Arc<Self>> {
        let hook = {
            let mut hooks = REGISTERED_HOOKS
                .lock()
                .expect("tick-engine hook registry poisoned");
            let hook = hooks.get(account).and_then(Weak::upgrade);
            if hook.is_none() {
                hooks.remove(account);
            }
            hook
        };
        CURRENT_HOOK.with(|current| {
            *current.borrow_mut() = hook.clone();
        });
        hook
    }

    /// Bind this hook to the current slot worker and install a packet observer
    /// that preserves the host's real SlotInput accounting.
    pub fn bind_client(
        self: &Arc<Self>,
        client: &mut Client,
        account: &str,
        input: Arc<SlotInput>,
    ) {
        if account != self.account {
            return;
        }
        CURRENT_HOOK.with(|current| {
            *current.borrow_mut() = Some(Arc::clone(self));
        });
        let session = client.gens.session;
        let should_install = {
            let mut state = self.state.lock().expect("tick-engine hook poisoned");
            if state.observer_session == Some(session) {
                false
            } else {
                state.observer_session = Some(session);
                true
            }
        };
        if should_install {
            let observer: Arc<dyn PacketObserver> = Arc::new(ObservedPackets {
                input,
                hook: Arc::downgrade(self),
            });
            client.out.set_observer(Some(observer));
        }
    }

    /// Return the hook bound by the current Play slot worker, if any.
    pub(crate) fn current() -> Option<Arc<Self>> {
        CURRENT_HOOK.with(|current| current.borrow().clone())
    }

    /// Install the byte-preserving live proxy controller for this case.
    pub fn set_transport_controller(&self, controller: Arc<dyn OrderedTransportController>) {
        self.state
            .lock()
            .expect("tick-engine hook poisoned")
            .transport = Some(controller);
    }

    /// Queue one test-only mutation for the next real ordered dispatch.
    pub fn queue_dispatch_mutation(&self, mutation: DispatchMutation) {
        self.state
            .lock()
            .expect("tick-engine hook poisoned")
            .pending_mutations
            .push_back(mutation);
    }

    /// Suppress the root of a real IF_OPENCHAT observation through `tick`.
    /// Chat lines and the underlying Client state remain untouched.
    pub fn suppress_openchat_through_tick(&self, tick: u32) {
        self.state
            .lock()
            .expect("tick-engine hook poisoned")
            .suppress_openchat_through_tick = Some(tick);
    }

    /// Return the most recent game tick observed by the live slot.
    pub fn observed_game_tick(&self) -> u32 {
        self.state
            .lock()
            .expect("tick-engine hook poisoned")
            .last_game_tick
    }
    /// Return the most recent host frame observed by the live slot.
    pub fn observed_host_frame(&self) -> u64 {
        self.state
            .lock()
            .expect("tick-engine hook poisoned")
            .next_host_frame
    }


    /// Start evidence capture after per-account setup. Existing chat sequences
    /// remain the watermark, so setup messages are never reported as proof.
    pub fn arm_measurement(&self, log_id: i32, inventory_ids: &[i32]) {
        let mut state = self.state.lock().expect("tick-engine hook poisoned");
        state.log_id = log_id;
        state.inventory_ids.clear();
        state.inventory_ids.extend_from_slice(inventory_ids);
        state.trace = TickEngineTrace::default();
        state.measurement_armed = true;
    }

    /// Clone the current bounded evidence trace.
    pub fn trace(&self) -> TickEngineTrace {
        self.state
            .lock()
            .expect("tick-engine hook poisoned")
            .trace
            .clone()
    }

    /// Wait until a later host observation is recorded. The timeout is a
    /// hang guard only; assertions use the trace's game-tick budget.
    pub fn wait_for_frame(&self, after: u64, timeout: Duration) -> Option<u64> {
        let state = self.state.lock().ok()?;
        let (state, _) = self
            .changed
            .wait_timeout_while(state, timeout, |state| state.next_host_frame <= after)
            .ok()?;
        (state.next_host_frame > after).then_some(state.next_host_frame)
    }

    /// Filter one real chat-open root from the mutable slot snapshot. Called
    /// after snapshot publication and before the script's observation.
    pub(crate) fn before_snapshot_observe(
        &self,
        account: &str,
        snapshot: &mut GameSnapshot,
    ) -> bool {
        if account != self.account || snapshot.modals().chat != 8880 {
            return false;
        }
        let mut state = self.state.lock().expect("tick-engine hook poisoned");
        let Some(through) = state.suppress_openchat_through_tick else {
            return false;
        };
        if snapshot.tick() > through {
            state.suppress_openchat_through_tick = None;
            return false;
        }
        snapshot.seed_chat_modal(-1, Vec::new());
        snapshot.seed_chat_options(Vec::new(), -1);
        true
    }

    /// Record one host poll and advance any transport release condition.
    pub(crate) fn after_frame(
        &self,
        client: &Client,
        account: &str,
        snapshot: &GameSnapshot,
        tick_edge: bool,
        suppressed_openchat: bool,
    ) {
        if account != self.account {
            return;
        }
        let (controller, host_frame, game_tick) = {
            let mut state = self.state.lock().expect("tick-engine hook poisoned");
            state.next_host_frame = state.next_host_frame.wrapping_add(1);
            let host_frame = state.next_host_frame;
            let game_tick = snapshot.tick();
            state.last_game_tick = game_tick;
            if state.measurement_armed {
                let poll = if tick_edge {
                    PollKind::Edge
                } else {
                    PollKind::Wake
                };
                let mut chat = snapshot
                    .chat_lines()
                    .iter()
                    .filter(|line| line.sequence > state.last_chat_sequence)
                    .map(|line| ChatObservation {
                        game_tick,
                        sequence: line.sequence,
                        text: line.text.clone(),
                        poll,
                    })
                    .collect::<Vec<_>>();
                chat.sort_by_key(|line| line.sequence);
                if let Some(latest) = chat.iter().map(|line| line.sequence).max() {
                    state.last_chat_sequence = latest;
                }
                let inventory_counts = state
                    .inventory_ids
                    .iter()
                    .map(|id| (*id, snapshot.inv_count(*id)))
                    .collect();
                let skill_xp = snapshot
                    .stats()
                    .iter()
                    .map(|stat| (stat.index, stat.xp))
                    .collect();
                let player_tile = snapshot.tile().map(|(x, z, level)| WorldTile { x, z, level });
                let log_count = snapshot.inv_count(state.log_id);
                let modal_packets = client.main_modal_packet_state();
                let chat_modal_root = snapshot.modals().chat;
                let actual_chat_modal_root = if suppressed_openchat {
                    8880
                } else {
                    chat_modal_root
                };
                state.trace.frames.push(FrameObservation {
                    host_frame,
                    game_tick,
                    tick_edge,
                    animation: client.local_animation_update(),
                    chat,
                    inventory_counts,
                    log_count,
                    skill_xp,
                    player_tile,
                    chat_modal_root,
                    actual_chat_modal_root,
                    server_if_close_packets: modal_packets.close_packets,
                    main_modal_opened_observation: modal_packets.opened_observation,
                    main_modal_closed_observation: modal_packets.closed_observation,
                    suppressed_openchat,
                });
            } else if let Some(latest) = snapshot
                .chat_lines()
                .iter()
                .map(|line| line.sequence)
                .max()
            {
                state.last_chat_sequence = state.last_chat_sequence.max(latest);
            }
            (state.transport.clone(), host_frame, game_tick)
        };
        if let Some(controller) = controller {
            controller.observed_frame(game_tick, tick_edge, host_frame);
        }
        self.changed.notify_all();
    }

    /// Begin a real ordered dispatch and apply its one queued mutation, if any.
    pub(crate) fn begin_dispatch(
        self: &Arc<Self>,
        snapshot: Option<&mut GameSnapshot>,
        authority: &HostAuthority,
        envelope: &ServerOrderedEnvelope,
    ) -> DispatchGuard {
        let mutation = self
            .state
            .lock()
            .expect("tick-engine hook poisoned")
            .pending_mutations
            .pop_front();
        let mut mutation_applied = false;
        let game_tick = snapshot.as_deref().map_or(0, GameSnapshot::tick);
        let request_id = authority.request_id().get();
        let request_debug = match envelope.payload() {
            script::surface::ServerOrderedPayload::Content(request) => Some(format!("{request:?}")),
            script::surface::ServerOrderedPayload::Advanced(_) => None,
        };
        let mut cancel_after = None;
        let mut transport = None;
        match mutation {
            Some(DispatchMutation::RemoveLoc { id, tile }) => {
                if let Some(snapshot) = snapshot {
                    let mut kept = Vec::with_capacity(snapshot.locs().len());
                    let before = snapshot.locs().len();
                    kept.extend(
                        snapshot
                            .locs()
                            .iter()
                            .filter(|loc| !(loc.id == id && loc.tile == tile))
                            .cloned(),
                    );
                    mutation_applied = kept.len() != before;
                    snapshot.seed_locs(kept);
                }
            }
            Some(DispatchMutation::CancelAfterPackets(count)) => {
                cancel_after = Some(count);
            }
            Some(DispatchMutation::Transport(mode)) => {
                transport = Some(mode);
            }
            None => {}
        }
        let content_request = matches!(
            envelope.payload(),
            script::surface::ServerOrderedPayload::Content(_)
        );
        let (controller, host_frame, active_transport) = {
            let mut state = self.state.lock().expect("tick-engine hook poisoned");
            let host_frame = state.next_host_frame;
            let controller = state.transport.clone();
            let is_knife_arm = request_debug
                .as_deref()
                .is_some_and(|request| request.contains("KnifeArm"));
            let active_transport = if content_request {
                controller.as_ref().and_then(|_| match transport {
                    Some(TransportMutation::JoinActiveGate) if !is_knife_arm => None,
                    Some(mutation) => Some(mutation),
                    None if is_knife_arm => Some(TransportMutation::JoinActiveGate),
                    None => None,
                })
            } else {
                None
            };
            if content_request {
                state.active_dispatch = Some(ActiveDispatch {
                    request_id,
                    authority: authority.clone(),
                    packets: 0,
                    cancel_after,
                    transport: active_transport,
                });
            }
            (controller, host_frame, active_transport)
        };
        if let (Some(controller), Some(mode)) = (controller, active_transport) {
            controller.begin_dispatch(request_id, mode, game_tick, host_frame);
        }
        DispatchGuard {
            hook: Arc::clone(self),
            request_id,
            mutation_applied,
            request_debug,
        }
    }

    /// Record the exact receipt produced by the real API planner/driver.
    pub(crate) fn record_dispatch(
        &self,
        receipt: ServerOrderedDispatch,
        request_debug: Option<&str>,
        mutation_applied: bool,
    ) {
        self.state
            .lock()
            .expect("tick-engine hook poisoned")
            .trace
            .dispatches
            .push(DispatchObservation {
                receipt,
                request_debug: request_debug.map(str::to_owned),
                mutation_applied,
            });
    }

    fn packet_committed(&self, opcode: u8, encoded_length: usize) {
        let (controller, packet, revoke) = {
            let mut state = self.state.lock().expect("tick-engine hook poisoned");
            let host_frame = state.next_host_frame;
            let game_tick = state.last_game_tick;
            let mut packet = None;
            let revoke = if let Some(active) = state.active_dispatch.as_mut() {
                active.packets = active.packets.saturating_add(1);
                let packet_index = active.packets;
                packet = Some((active.request_id, packet_index, active.transport.is_some()));
                if active.cancel_after == Some(packet_index) {
                    active.cancel_after = None;
                    Some(active.authority.clone())
                } else {
                    None
                }
            } else {
                None
            };
            if state.measurement_armed {
                state.trace.wire.push(WireObservation {
                    host_frame,
                    game_tick,
                    request_id: packet.map(|(request_id, _, _)| request_id),
                    opcode,
                    encoded_length,
                });
            }
            (state.transport.clone(), packet, revoke)
        };
        if let (Some(controller), Some((request_id, packet_index, true))) = (controller, packet) {
            controller.packet_committed(request_id, packet_index, opcode, encoded_length);
        }
        if let Some(authority) = revoke {
            authority.cancel_request_for_test();
        }
    }

    fn end_dispatch(&self, request_id: u64) {
        let (controller, ended, host_frame) = {
            let mut state = self.state.lock().expect("tick-engine hook poisoned");
            let host_frame = state.next_host_frame;
            let active = state
                .active_dispatch
                .as_ref()
                .filter(|active| active.request_id == request_id);
            let end_transport = active.is_some_and(|active| active.transport.is_some());
            if active.is_some() {
                state.active_dispatch = None;
            }
            (state.transport.clone(), end_transport, host_frame)
        };
        if ended {
            if let Some(controller) = controller {
                controller.end_dispatch(request_id, host_frame);
            }
        }
    }
}

/// RAII fence for the packet observer's request-specific authority.
pub(crate) struct DispatchGuard {
    hook: Arc<TickEngineTestHook>,
    request_id: u64,
    mutation_applied: bool,
    request_debug: Option<String>,
}

impl DispatchGuard {
    /// Whether the requested snapshot edit removed the named live object.
    pub(crate) fn mutation_applied(&self) -> bool {
        self.mutation_applied
    }

    /// Return the real request's typed debug representation.
    pub(crate) fn request_debug(&self) -> Option<&str> {
        self.request_debug.as_deref()
    }
}

impl Drop for DispatchGuard {
    fn drop(&mut self) {
        self.hook.end_dispatch(self.request_id);
    }
}

struct ObservedPackets {
    input: Arc<SlotInput>,
    hook: Weak<TickEngineTestHook>,
}

impl PacketObserver for ObservedPackets {
    fn packet_body_length(&self, opcode: u8) -> i16 {
        PacketObserver::packet_body_length(self.input.as_ref(), opcode)
    }

    fn packet_committed(&self, opcode: u8, encoded_length: usize) {
        PacketObserver::packet_committed(self.input.as_ref(), opcode, encoded_length);
        if let Some(hook) = self.hook.upgrade() {
            hook.packet_committed(opcode, encoded_length);
        }
    }

    fn observed_tick(&self, tick: u64, session: u64) {
        PacketObserver::observed_tick(self.input.as_ref(), tick, session);
    }

    fn foreign_local_input(&self) {
        PacketObserver::foreign_local_input(self.input.as_ref());
    }

    fn committed_packet_count(&self) -> Option<u64> {
        PacketObserver::committed_packet_count(self.input.as_ref())
    }
}

