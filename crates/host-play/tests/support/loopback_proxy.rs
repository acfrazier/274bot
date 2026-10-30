use host_play::tick_engine_test_support::{OrderedTransportController, TransportMutation};
use std::collections::{HashMap, HashSet};
use std::io::{self, ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const POLL_PAUSE: Duration = Duration::from_millis(2);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const RELAY_BUFFER: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyObservation {
    pub request_id: u64,
    pub request_ids: Vec<u64>,
    pub mutation: String,
    pub start_game_tick: u32,
    pub start_host_frame: u64,
    pub packet_requests: Vec<(u64, usize)>,
    pub packet_opcodes: Vec<u8>,
    pub packet_lengths: Vec<usize>,
    pub forwarded_bytes: usize,
    pub dropped_bytes: usize,
    pub release_edge_tick: Option<u32>,
    pub release_trigger_tick: Option<u32>,
    pub release_edge_frame: Option<u64>,
    pub released_host_frame: Option<u64>,
}

#[derive(Default)]
struct ProxyState {
    gate: Option<Gate>,
    completed: Vec<ProxyObservation>,
    error: Option<String>,
}

struct Gate {
    mutation: TransportMutation,
    expected_bytes: usize,
    processed_bytes: usize,
    release_prefix_bytes: usize,
    release_all: bool,
    next_edge_frame: Option<u64>,
    request_packets: HashMap<u64, usize>,
    active_requests: HashSet<u64>,
    observation: ProxyObservation,
}

pub struct LoopbackProxy {
    address: SocketAddr,
    state: Arc<Mutex<ProxyState>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl LoopbackProxy {
    pub fn start(upstream: SocketAddr) -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let state = Arc::new(Mutex::new(ProxyState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_state = Arc::clone(&state);
        let worker_stop = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            if let Err(error) = accept_loop(listener, upstream, worker_state.clone(), worker_stop) {
                set_error(&worker_state, error.to_string());
            }
        });
        Ok(Self {
            address,
            state,
            stop,
            worker: Some(worker),
        })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn observations(&self) -> Vec<ProxyObservation> {
        self.state
            .lock()
            .expect("loopback proxy state poisoned")
            .completed
            .clone()
    }

    pub fn ensure_healthy(&self) -> Result<(), String> {
        self.state
            .lock()
            .map_err(|_| "loopback proxy state poisoned".to_owned())?
            .error
            .clone()
            .map_or(Ok(()), Err)
    }
}

impl OrderedTransportController for LoopbackProxy {
    fn begin_dispatch(
        &self,
        request_id: u64,
        mutation: TransportMutation,
        game_tick: u32,
        host_frame: u64,
    ) {
        let mut state = self.state.lock().expect("loopback proxy state poisoned");
        if mutation == TransportMutation::JoinActiveGate {
            if let Some(gate) = state.gate.as_mut().filter(|gate| {
                matches!(
                    gate.mutation,
                    TransportMutation::HoldThroughNextEdge
                        | TransportMutation::HoldUntilNextDispatch
                )
            }) {
                if gate.active_requests.insert(request_id) {
                    gate.observation.request_ids.push(request_id);
                    gate.request_packets.insert(request_id, 0);
                    if gate.mutation == TransportMutation::HoldUntilNextDispatch {
                        gate.observation.release_trigger_tick = Some(game_tick);
                    }
                }
            }
            return;
        }
        if state.gate.is_some() {
            state.error = Some(format!(
                "request {request_id} began while another transport gate was active"
            ));
            return;
        }
        if matches!(mutation, TransportMutation::DropSuffixAfterPacket(0)) {
            state.error = Some(format!(
                "request {request_id} requested a zero-packet forwarding prefix"
            ));
            return;
        }
        let mut request_packets = HashMap::new();
        request_packets.insert(request_id, 0);
        let mut active_requests = HashSet::new();
        active_requests.insert(request_id);
        state.gate = Some(Gate {
            mutation,
            expected_bytes: 0,
            processed_bytes: 0,
            release_prefix_bytes: 0,
            release_all: false,
            next_edge_frame: None,
            request_packets,
            active_requests,
            observation: ProxyObservation {
                request_id,
                request_ids: vec![request_id],
                mutation: format!("{mutation:?}"),
                start_game_tick: game_tick,
                start_host_frame: host_frame,
                packet_requests: Vec::new(),
                packet_opcodes: Vec::new(),
                packet_lengths: Vec::new(),
                forwarded_bytes: 0,
                dropped_bytes: 0,
                release_edge_tick: None,
                release_trigger_tick: None,
                release_edge_frame: None,
                released_host_frame: None,
            },
        });
    }

    fn packet_committed(
        &self,
        request_id: u64,
        packet_index: usize,
        opcode: u8,
        encoded_length: usize,
    ) {
        let mut state = self.state.lock().expect("loopback proxy state poisoned");
        let accounting = state.gate.as_ref().and_then(|gate| {
            gate.request_packets.get(&request_id).map(|committed| {
                (
                    committed.saturating_add(1),
                    gate.active_requests.contains(&request_id),
                )
            })
        });
        let Some((expected_index, active)) = accounting else {
            return;
        };
        if !active || packet_index != expected_index {
            state.error = Some(format!(
                "unexpected packet {packet_index} for transport request {request_id}"
            ));
            return;
        }
        let gate = state
            .gate
            .as_mut()
            .expect("gate ownership was just checked");
        gate.request_packets.insert(request_id, packet_index);
        gate.expected_bytes = gate.expected_bytes.saturating_add(encoded_length);
        gate.observation
            .packet_requests
            .push((request_id, packet_index));
        gate.observation.packet_opcodes.push(opcode);
        gate.observation.packet_lengths.push(encoded_length);
        match gate.mutation {
            TransportMutation::SplitAfterFirstPacket
                if request_id == gate.observation.request_id && packet_index == 1 =>
            {
                gate.release_prefix_bytes = encoded_length;
            }
            TransportMutation::DropSuffixAfterPacket(count)
                if request_id == gate.observation.request_id && packet_index <= count =>
            {
                gate.release_prefix_bytes = gate.expected_bytes;
            }
            _ => {}
        }
    }

    fn observed_frame(&self, game_tick: u32, tick_edge: bool, host_frame: u64) {
        let mut state = self.state.lock().expect("loopback proxy state poisoned");
        let Some(gate) = state.gate.as_mut() else {
            return;
        };
        if matches!(
            gate.mutation,
            TransportMutation::HoldThroughNextEdge | TransportMutation::SplitAfterFirstPacket
        ) && tick_edge
            && game_tick > gate.observation.start_game_tick
            && gate.next_edge_frame.is_none()
        {
            gate.next_edge_frame = Some(host_frame);
            gate.observation.release_edge_tick = Some(game_tick);
            gate.observation.release_edge_frame = Some(host_frame);
        }
        if gate
            .next_edge_frame
            .is_some_and(|edge_frame| host_frame > edge_frame)
        {
            gate.release_all = true;
            gate.observation.released_host_frame = Some(host_frame);
        }
    }

    fn end_dispatch(&self, request_id: u64, host_frame: u64) {
        let mut state = self.state.lock().expect("loopback proxy state poisoned");
        if let Some(gate) = state.gate.as_mut() {
            if gate.active_requests.remove(&request_id)
                && gate.mutation == TransportMutation::HoldUntilNextDispatch
                && gate.observation.release_trigger_tick.is_some()
                && request_id != gate.observation.request_id
            {
                gate.release_all = true;
                gate.observation.released_host_frame = Some(host_frame);
            }
        }
    }
}

impl Drop for LoopbackProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn accept_loop(
    listener: TcpListener,
    upstream: SocketAddr,
    state: Arc<Mutex<ProxyState>>,
    stop: Arc<AtomicBool>,
) -> io::Result<()> {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((client, _)) => {
                let server = TcpStream::connect_timeout(&upstream, CONNECT_TIMEOUT)?;
                relay_connection(client, server, &state, &stop)?;
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(POLL_PAUSE);
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn relay_connection(
    mut client: TcpStream,
    mut server: TcpStream,
    state: &Arc<Mutex<ProxyState>>,
    stop: &AtomicBool,
) -> io::Result<()> {
    client.set_nonblocking(true)?;
    server.set_nonblocking(true)?;
    let mut client_to_server = Vec::new();
    let mut server_to_client = Vec::new();
    let mut bytes = [0_u8; RELAY_BUFFER];
    let mut client_eof = false;
    let mut server_eof = false;
    while !stop.load(Ordering::Acquire) {
        let mut activity = false;
        if !client_eof {
            match client.read(&mut bytes) {
                Ok(0) => client_eof = true,
                Ok(read) => {
                    client_to_server.extend_from_slice(&bytes[..read]);
                    activity = true;
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        if !server_eof {
            match server.read(&mut bytes) {
                Ok(0) => server_eof = true,
                Ok(read) => {
                    server_to_client.extend_from_slice(&bytes[..read]);
                    activity = true;
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        activity |= flush_upstream(&mut client_to_server, &mut server, state)?;
        activity |= flush_downstream(&mut server_to_client, &mut client)?;
        if client_eof || server_eof {
            break;
        }
        if !activity {
            thread::sleep(POLL_PAUSE);
        }
    }
    let _ = client.shutdown(std::net::Shutdown::Both);
    let _ = server.shutdown(std::net::Shutdown::Both);
    Ok(())
}

#[derive(Clone, Copy)]
enum UpstreamAction {
    Forward(usize),
    Drop(usize),
    Wait,
    Pass,
}

fn flush_upstream(
    buffer: &mut Vec<u8>,
    server: &mut TcpStream,
    state: &Arc<Mutex<ProxyState>>,
) -> io::Result<bool> {
    let mut activity = false;
    loop {
        if buffer.is_empty() {
            return Ok(activity);
        }
        let action = {
            let mut state = state
                .lock()
                .map_err(|_| io::Error::new(ErrorKind::Other, "loopback proxy state poisoned"))?;
            if let Some(error) = &state.error {
                return Err(io::Error::new(ErrorKind::Other, error.clone()));
            }
            if state.gate.is_none() {
                UpstreamAction::Pass
            } else if state.gate.as_ref().is_some_and(|gate| {
                gate.active_requests.is_empty() && gate.processed_bytes >= gate.expected_bytes
            }) {
                let completed = state.gate.take().expect("gate was present");
                state.completed.push(completed.observation);
                UpstreamAction::Pass
            } else {
                let gate = state.gate.as_ref().expect("gate was present");
                let remaining = gate.expected_bytes.saturating_sub(gate.processed_bytes);
                let available = min(buffer.len(), remaining);
                let prefix = gate
                    .release_prefix_bytes
                    .saturating_sub(gate.processed_bytes);
                match gate.mutation {
                    TransportMutation::HoldThroughNextEdge
                    | TransportMutation::HoldUntilNextDispatch => {
                        if gate.release_all {
                            UpstreamAction::Forward(available)
                        } else {
                            UpstreamAction::Wait
                        }
                    }
                    TransportMutation::SplitAfterFirstPacket => {
                        if gate.release_all {
                            UpstreamAction::Forward(available)
                        } else if prefix > 0 {
                            UpstreamAction::Forward(min(available, prefix))
                        } else {
                            UpstreamAction::Wait
                        }
                    }
                    TransportMutation::DropSuffixAfterPacket(_) => {
                        if prefix > 0 {
                            UpstreamAction::Forward(min(available, prefix))
                        } else {
                            UpstreamAction::Drop(available)
                        }
                    }
                    TransportMutation::JoinActiveGate => UpstreamAction::Pass,
                }
            }
        };
        match action {
            UpstreamAction::Wait => return Ok(activity),
            UpstreamAction::Pass => match server.write(buffer) {
                Ok(0) => return Ok(activity),
                Ok(written) => {
                    buffer.drain(..written);
                    activity = true;
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(activity),
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            },
            UpstreamAction::Forward(length) => {
                if length == 0 {
                    return Ok(activity);
                }
                match server.write(&buffer[..length]) {
                    Ok(0) => return Ok(activity),
                    Ok(written) => {
                        buffer.drain(..written);
                        let mut state = state.lock().map_err(|_| {
                            io::Error::new(ErrorKind::Other, "loopback proxy state poisoned")
                        })?;
                        if let Some(gate) = state.gate.as_mut() {
                            gate.processed_bytes = gate.processed_bytes.saturating_add(written);
                            gate.observation.forwarded_bytes =
                                gate.observation.forwarded_bytes.saturating_add(written);
                        }
                        activity = true;
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(activity),
                    Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error),
                }
            }
            UpstreamAction::Drop(length) => {
                if length == 0 {
                    return Ok(activity);
                }
                buffer.drain(..length);
                let mut state = state.lock().map_err(|_| {
                    io::Error::new(ErrorKind::Other, "loopback proxy state poisoned")
                })?;
                if let Some(gate) = state.gate.as_mut() {
                    gate.processed_bytes = gate.processed_bytes.saturating_add(length);
                    gate.observation.dropped_bytes =
                        gate.observation.dropped_bytes.saturating_add(length);
                }
                activity = true;
            }
        }
    }
}

fn flush_downstream(buffer: &mut Vec<u8>, client: &mut TcpStream) -> io::Result<bool> {
    if buffer.is_empty() {
        return Ok(false);
    }
    match client.write(buffer) {
        Ok(0) => Ok(false),
        Ok(written) => {
            buffer.drain(..written);
            Ok(true)
        }
        Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(false),
        Err(error) if error.kind() == ErrorKind::Interrupted => Ok(false),
        Err(error) => Err(error),
    }
}

fn set_error(state: &Arc<Mutex<ProxyState>>, error: String) {
    if let Ok(mut state) = state.lock() {
        state.error.get_or_insert(error);
    }
}
