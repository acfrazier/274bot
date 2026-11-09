use host_play::tick_engine_test_support::{OrderedTransportController, TransportMutation};
use std::cmp::min;
use std::collections::{HashMap, HashSet};
use std::io::{self, ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
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
    let mut relays = Vec::new();
    let result = loop {
        if stop.load(Ordering::Acquire) {
            break Ok(());
        }
        match listener.accept() {
            Ok((client, _)) => {
                let relay_state = Arc::clone(&state);
                let relay_stop = Arc::clone(&stop);
                relays.push(thread::spawn(move || {
                    let result =
                        TcpStream::connect_timeout(&upstream, CONNECT_TIMEOUT).and_then(|server| {
                            relay_connection(client, server, &relay_state, &relay_stop)
                        });
                    if let Err(error) = result {
                        set_error(&relay_state, error.to_string());
                        relay_stop.store(true, Ordering::Release);
                    }
                }));
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(POLL_PAUSE);
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => break Err(error),
        }
        reap_finished_relays(&mut relays, &state, &stop);
    };
    if result.is_err() {
        stop.store(true, Ordering::Release);
    }
    for relay in relays {
        if relay.join().is_err() {
            set_error(&state, "loopback proxy relay thread panicked".to_owned());
            stop.store(true, Ordering::Release);
        }
    }
    result
}

fn reap_finished_relays(
    relays: &mut Vec<JoinHandle<()>>,
    state: &Arc<Mutex<ProxyState>>,
    stop: &AtomicBool,
) {
    let mut index = 0;
    while index < relays.len() {
        if relays[index].is_finished() {
            let relay = relays.swap_remove(index);
            if relay.join().is_err() {
                set_error(state, "loopback proxy relay thread panicked".to_owned());
                stop.store(true, Ordering::Release);
            }
        } else {
            index += 1;
        }
    }
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
    let mut game_stream = false;
    let mut stream_kind_known = false;
    while !stop.load(Ordering::Acquire) {
        let mut activity = false;
        if !client_eof {
            match client.read(&mut bytes) {
                Ok(0) => client_eof = true,
                Ok(read) => {
                    if !stream_kind_known {
                        // JS5 opens with opcode 15; the game login stream's
                        // first packet starts with opcode 14.
                        game_stream = bytes[0] != 15;
                        stream_kind_known = true;
                    }
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
        activity |= flush_upstream(&mut client_to_server, &mut server, game_stream, state)?;
        activity |= flush_downstream(&mut server_to_client, &mut client)?;
        if client_eof || server_eof {
            break;
        }
        if !activity {
            thread::sleep(POLL_PAUSE);
        }
    }
    let _ = client.shutdown(Shutdown::Both);
    let _ = server.shutdown(Shutdown::Both);
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
    game_stream: bool,
    state: &Arc<Mutex<ProxyState>>,
) -> io::Result<bool> {
    let mut activity = false;
    loop {
        // Record completion even when the last write drained the whole buffer.
        // A later client packet must not be needed to publish that receipt.
        let action = {
            let mut state = state
                .lock()
                .map_err(|_| io::Error::other("loopback proxy state poisoned"))?;
            if let Some(error) = &state.error {
                return Err(io::Error::other(error.clone()));
            }
            if !game_stream || state.gate.is_none() {
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
        if buffer.is_empty() {
            return Ok(activity);
        }
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
                        let mut state = state
                            .lock()
                            .map_err(|_| io::Error::other("loopback proxy state poisoned"))?;
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
                let mut state = state
                    .lock()
                    .map_err(|_| io::Error::other("loopback proxy state poisoned"))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::time::Instant;

    const IO_TIMEOUT: Duration = Duration::from_secs(5);

    fn accept_with_timeout(listener: &TcpListener) -> io::Result<(TcpStream, SocketAddr)> {
        let deadline = Instant::now() + IO_TIMEOUT;
        loop {
            match listener.accept() {
                Err(error)
                    if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    thread::sleep(POLL_PAUSE);
                }
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                result => return result,
            }
        }
    }

    #[test]
    fn game_and_js5_connections_are_serviced_concurrently() {
        let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        upstream.set_nonblocking(true).unwrap();
        let proxy = LoopbackProxy::start(upstream.local_addr().unwrap()).unwrap();
        let server = thread::spawn(move || -> io::Result<()> {
            let (mut game, _) = accept_with_timeout(&upstream)?;
            game.set_nonblocking(false)?;
            game.set_read_timeout(Some(IO_TIMEOUT))?;
            let mut login_hello = [0_u8; 2];
            game.read_exact(&mut login_hello)?;
            if login_hello != [14, 0] {
                return Err(io::Error::other(
                    "game stream did not start with login opcode 14",
                ));
            }
            game.write_all(&[1, 2, 3, 4, 5, 6, 7, 8])?;

            let (mut js5, _) = accept_with_timeout(&upstream)?;
            js5.set_nonblocking(false)?;
            js5.set_read_timeout(Some(IO_TIMEOUT))?;
            let mut js5_hello = [0_u8; 1];
            js5.read_exact(&mut js5_hello)?;
            if js5_hello != [15] {
                return Err(io::Error::other(
                    "update stream did not start with opcode 15",
                ));
            }
            js5.write_all(&[8, 7, 6, 5, 4, 3, 2, 1])?;
            let mut request = [0_u8; 4];
            js5.read_exact(&mut request)?;
            if request != [0, 0, 1, 2] {
                return Err(io::Error::other("proxy changed the update request bytes"));
            }
            js5.write_all(b"map-data")?;
            let mut js5_tail = Vec::new();
            js5.read_to_end(&mut js5_tail)?;
            let mut game_tail = Vec::new();
            game.read_to_end(&mut game_tail)?;
            Ok(())
        });

        let mut game = TcpStream::connect(proxy.address()).unwrap();
        game.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
        game.write_all(&[14, 0]).unwrap();
        let mut game_reply = [0_u8; 8];
        game.read_exact(&mut game_reply).unwrap();
        assert_eq!(game_reply, [1, 2, 3, 4, 5, 6, 7, 8]);

        proxy.begin_dispatch(1, TransportMutation::HoldThroughNextEdge, 1, 1);
        let mut js5 = TcpStream::connect(proxy.address()).unwrap();
        js5.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
        js5.write_all(&[15]).unwrap();
        let js5_exchange = (|| -> io::Result<([u8; 8], [u8; 8])> {
            let mut hello = [0_u8; 8];
            js5.read_exact(&mut hello)?;
            js5.write_all(&[0, 0, 1, 2])?;
            let mut map_data = [0_u8; 8];
            js5.read_exact(&mut map_data)?;
            Ok((hello, map_data))
        })();
        let _ = game.shutdown(Shutdown::Write);
        let _ = js5.shutdown(Shutdown::Write);
        let server_result = server.join().expect("upstream thread panicked");

        assert_eq!(
            js5_exchange.unwrap(),
            ([8, 7, 6, 5, 4, 3, 2, 1], *b"map-data")
        );
        assert!(
            server_result.is_ok(),
            "upstream could not finish both live streams: {server_result:?}"
        );
        proxy.ensure_healthy().unwrap();
    }

    fn connected_game_proxy() -> (LoopbackProxy, TcpStream, TcpStream) {
        let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        upstream.set_nonblocking(true).unwrap();
        let proxy = LoopbackProxy::start(upstream.local_addr().unwrap()).unwrap();
        let mut client = TcpStream::connect(proxy.address()).unwrap();
        let (mut server, _) = accept_with_timeout(&upstream).unwrap();
        client.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
        server.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
        client.write_all(&[14, 0]).unwrap();
        let mut hello = [0; 2];
        server.read_exact(&mut hello).unwrap();
        assert_eq!(hello, [14, 0]);
        (proxy, client, server)
    }

    fn assert_no_upstream_bytes(server: &mut TcpStream) {
        server.set_nonblocking(true).unwrap();
        let mut byte = [0];
        let result = server.read(&mut byte);
        server.set_nonblocking(false).unwrap();
        assert!(
            matches!(&result, Err(error) if error.kind() == ErrorKind::WouldBlock),
            "unexpected upstream bytes: {result:?}"
        );
    }

    #[test]
    fn ordinary_split_after_first_packet_still_holds_only_the_suffix() {
        let (proxy, mut client, mut server) = connected_game_proxy();
        let first = [0x31; 3];
        let suffix = [0x42; 5];

        proxy.begin_dispatch(51, TransportMutation::SplitAfterFirstPacket, 12, 200);
        proxy.packet_committed(51, 1, 27, first.len());
        proxy.packet_committed(51, 2, 28, suffix.len());
        proxy.end_dispatch(51, 200);
        client.write_all(&first).unwrap();
        client.write_all(&suffix).unwrap();

        let mut received_first = [0; 3];
        server.read_exact(&mut received_first).unwrap();
        assert_eq!(received_first, first);
        assert_no_upstream_bytes(&mut server);
        proxy.observed_frame(13, true, 201);
        assert_no_upstream_bytes(&mut server);
        proxy.observed_frame(13, false, 202);
        let mut received_suffix = [0; 5];
        server.read_exact(&mut received_suffix).unwrap();
        assert_eq!(received_suffix, suffix);

        client.write_all(&[0x53]).unwrap();
        let mut tail = [0];
        server.read_exact(&mut tail).unwrap();
        assert_eq!(tail, [0x53]);
        let observation = proxy.observations().pop().unwrap();
        assert_eq!(observation.packet_lengths, [first.len(), suffix.len()]);
        assert_eq!(observation.forwarded_bytes, first.len() + suffix.len());
        assert_eq!(observation.dropped_bytes, 0);
        proxy.ensure_healthy().unwrap();
    }

    #[test]
    fn ordinary_hold_through_next_edge_still_holds_every_byte() {
        let (proxy, mut client, mut server) = connected_game_proxy();
        let packet = [0x64; 9];

        proxy.begin_dispatch(61, TransportMutation::HoldThroughNextEdge, 20, 300);
        proxy.packet_committed(61, 1, 29, packet.len());
        proxy.end_dispatch(61, 300);
        client.write_all(&packet).unwrap();
        assert_no_upstream_bytes(&mut server);
        proxy.observed_frame(21, true, 301);
        assert_no_upstream_bytes(&mut server);
        proxy.observed_frame(21, false, 302);
        let mut received = [0; 9];
        server.read_exact(&mut received).unwrap();
        assert_eq!(received, packet);

        client.write_all(&[0x75]).unwrap();
        let mut tail = [0];
        server.read_exact(&mut tail).unwrap();
        assert_eq!(tail, [0x75]);
        let observation = proxy.observations().pop().unwrap();
        assert_eq!(observation.packet_lengths, [packet.len()]);
        assert_eq!(observation.forwarded_bytes, packet.len());
        assert_eq!(observation.dropped_bytes, 0);
        proxy.ensure_healthy().unwrap();
    }

    #[test]
    fn final_drained_byte_completes_the_gate_without_another_client_packet() {
        for mutation in [
            TransportMutation::HoldUntilNextDispatch,
            TransportMutation::DropSuffixAfterPacket(1),
        ] {
            let (proxy, _client, mut server) = connected_game_proxy();
            proxy.begin_dispatch(71, mutation, 30, 400);
            proxy.packet_committed(71, 1, 31, 3);
            if mutation == TransportMutation::HoldUntilNextDispatch {
                proxy.end_dispatch(71, 400);
                proxy.begin_dispatch(72, TransportMutation::JoinActiveGate, 31, 401);
                proxy.packet_committed(72, 1, 32, 5);
                proxy.end_dispatch(72, 401);
            } else {
                proxy.packet_committed(71, 2, 32, 5);
                proxy.end_dispatch(71, 400);
            }
            let mut buffer = vec![0x31; 8];
            assert!(flush_upstream(&mut buffer, &mut server, true, &proxy.state).unwrap());
            assert!(buffer.is_empty());
            let observations = proxy.observations();
            assert_eq!(observations.len(), 1, "mutation={mutation:?}");
            assert_eq!(observations[0].packet_lengths, [3, 5]);
            if mutation == TransportMutation::HoldUntilNextDispatch {
                assert_eq!(observations[0].request_ids, [71, 72]);
                assert_eq!(observations[0].forwarded_bytes, 8);
                assert_eq!(observations[0].dropped_bytes, 0);
            } else {
                assert_eq!(observations[0].request_ids, [71]);
                assert_eq!(observations[0].forwarded_bytes, 3);
                assert_eq!(observations[0].dropped_bytes, 5);
            }
            proxy.ensure_healthy().unwrap();
        }
    }
}
