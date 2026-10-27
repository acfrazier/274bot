//! Bounded, stepped HTTP asset fetches over caller-resolved TCP addresses.
//!
//! Plain HTTP remains on the fetch owner. HTTPS delegates only connect and the
//! initial rustls handshake to the process-scoped `net-https-handshake` worker.

use std::io::{self, Read as IoRead, Write as IoWrite};
use std::net::{Shutdown, SocketAddr, TcpStream as StdTcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use client::client::client::AssetFetchError;
use client::io::client_stream::LoginCancel;
use client::io::host_network::HostHttpFetch;
use client::Transport;
use mio::net::TcpStream as MioTcpStream;
use rustls::ClientConnection;

use crate::https_handshake::{EstablishedTls, FetchClass, HttpsLane, HttpsTicket};

const CONNECTION_FAILED: AssetFetchError = AssetFetchError::Connection;
const PLAIN_QUIET_LIMIT: Duration = Duration::from_secs(2);
const STEPPED_HTTPS_QUIET_LIMIT: Duration = Duration::from_secs(8);
const BLOCKING_LOGIN_KEY_IO_LIMIT: Duration = Duration::from_secs(2);
const BLOCKING_HTTP_IO_LIMIT: Duration = Duration::from_secs(8);
const FETCH_STEP_BUDGET: Duration = Duration::from_millis(8);
const READ_CHUNK_BYTES: usize = 16 * 1024;
const IO_OPERATIONS_PER_STEP: usize = 32;
const MAX_ADDRESSES: usize = 64;
const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;
const MAX_CHUNK_LINE_BYTES: usize = 8 * 1024;

const REQUEST_PREFIX: &[u8] = b"GET ";
const REQUEST_MIDDLE: &[u8] = b" HTTP/1.0\r\nHost: ";
const REQUEST_SUFFIX: &[u8] = b"\r\n\r\n";

/// Start one HTTP or HTTPS GET using only the addresses resolved by the caller.
///
/// `Transport::Wss` selects HTTPS. It never retries the request over plaintext.
pub fn start_fetch(
    mode: Transport,
    host: &str,
    addrs: &[SocketAddr],
    path: &str,
    https: &Arc<HttpsLane>,
) -> Result<Box<dyn HostHttpFetch>, AssetFetchError> {
    let request = build_request(host, path)?;
    if addrs.is_empty() || addrs.len() > MAX_ADDRESSES {
        return Err(CONNECTION_FAILED);
    }

    let mut fetch = HttpFetch {
        addresses: Vec::new(),
        next_address: 0,
        socket: None,
        tls: None,
        https_ticket: None,
        phase: Phase::Connecting,
        request,
        sent: 0,
        response: ResponseParser::new(),
        progress_at: None,
        quiet_limit: PLAIN_QUIET_LIMIT,
    };

    match mode {
        Transport::Tcp => {
            fetch
                .addresses
                .try_reserve_exact(addrs.len())
                .map_err(|_| CONNECTION_FAILED)?;
            fetch.addresses.extend_from_slice(addrs);
            fetch.start_next_address()?;
        }
        Transport::Wss => {
            fetch.https_ticket = Some(
                https
                    .begin_fetch(host, addrs, FetchClass::Stepped)
                    .map_err(|_| CONNECTION_FAILED)?,
            );
            fetch.phase = Phase::WaitingTls;
            fetch.quiet_limit = STEPPED_HTTPS_QUIET_LIMIT;
        }
    }

    Ok(Box::new(fetch))
}

/// Drive a blocking fetch to completion.
///
/// `Transport::Tcp` treats `deadline` as an absolute cutoff. For WSS,
/// `Some(_)` selects login-key phase and per-operation budgets.
pub fn fetch_blocking(
    mode: Transport,
    host: &str,
    addrs: &[SocketAddr],
    path: &str,
    deadline: Option<Instant>,
    https: &Arc<HttpsLane>,
    cancel: Option<&LoginCancel>,
) -> Result<Vec<u8>, AssetFetchError> {
    check_cancel(cancel)?;
    if mode != Transport::Wss {
        check_blocking_deadline(deadline)?;
    }

    if mode == Transport::Wss {
        let request = build_request(host, path)?;
        if addrs.is_empty() || addrs.len() > MAX_ADDRESSES {
            return Err(CONNECTION_FAILED);
        }
        let class = if deadline.is_some() {
            FetchClass::BlockingLoginKey
        } else {
            FetchClass::Blocking
        };
        let mut ticket = https
            .begin_fetch(host, addrs, class)
            .map_err(|_| CONNECTION_FAILED)?;
        if let Some(cancel) = cancel {
            ticket.register_cancel_callback(cancel);
        }
        let established = ticket
            .wait_for_result(None)
            .map_err(|_| CONNECTION_FAILED)?;
        let io_limit = if deadline.is_some() {
            BLOCKING_LOGIN_KEY_IO_LIMIT
        } else {
            BLOCKING_HTTP_IO_LIMIT
        };
        return fetch_https_blocking(established, &request, io_limit, cancel);
    }

    let mut fetch = start_fetch(mode, host, addrs, path, https)?;
    loop {
        check_cancel(cancel)?;
        let now = Instant::now();
        if deadline.is_some_and(|deadline| now >= deadline) {
            return Err(CONNECTION_FAILED);
        }
        let step_until = now
            .checked_add(FETCH_STEP_BUDGET)
            .unwrap_or_else(|| deadline.unwrap_or(now));
        let until = deadline.map_or(step_until, |deadline| deadline.min(step_until));
        if let Some(body) = fetch.poll(until)? {
            check_cancel(cancel)?;
            check_blocking_deadline(deadline)?;
            return Ok(body);
        }
    }
}

struct HttpFetch {
    addresses: Vec<SocketAddr>,
    next_address: usize,
    socket: Option<MioTcpStream>,
    tls: Option<ClientConnection>,
    https_ticket: Option<HttpsTicket>,
    phase: Phase,
    request: Vec<u8>,
    sent: usize,
    response: ResponseParser,
    progress_at: Option<Instant>,
    quiet_limit: Duration,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Phase {
    WaitingTls,
    Connecting,
    Sending,
    Receiving,
    Complete,
    Failed,
}

enum Advance {
    Blocked,
    Complete,
    Yield,
}

impl HttpFetch {
    fn start_next_address(&mut self) -> Result<(), AssetFetchError> {
        self.drop_socket();
        self.phase = Phase::Connecting;
        self.progress_at = None;

        while let Some(&address) = self.addresses.get(self.next_address) {
            self.next_address += 1;
            let Ok(socket) = MioTcpStream::connect(address) else {
                continue;
            };
            self.socket = Some(socket);
            return Ok(());
        }

        self.phase = Phase::Failed;
        Err(CONNECTION_FAILED)
    }

    fn install_https(&mut self, established: EstablishedTls) -> Result<(), AssetFetchError> {
        let EstablishedTls { socket, connection } = established;
        socket
            .set_nonblocking(true)
            .map_err(|_| CONNECTION_FAILED)?;
        self.socket = Some(MioTcpStream::from_std(socket));
        self.tls = Some(connection);
        self.progress_at = Some(Instant::now());
        self.phase = Phase::Sending;
        Ok(())
    }

    fn drop_socket(&mut self) {
        drop(self.socket.take());
    }

    fn advance(&mut self) -> Result<Advance, AssetFetchError> {
        let mut chunk = [0u8; READ_CHUNK_BYTES];
        for _ in 0..IO_OPERATIONS_PER_STEP {
            match self.phase {
                Phase::Connecting => {
                    let connect_error = self.socket.as_mut().ok_or(CONNECTION_FAILED)?.take_error();
                    match connect_error {
                        Ok(Some(_)) | Err(_) => {
                            self.start_next_address()?;
                            continue;
                        }
                        Ok(None) => {}
                    }
                    let peer = self.socket.as_mut().ok_or(CONNECTION_FAILED)?.peer_addr();
                    match peer {
                        Ok(_) => {
                            self.progress_at = Some(Instant::now());
                            self.phase = Phase::Sending;
                        }
                        Err(error) if error.kind() == io::ErrorKind::NotConnected => {
                            return Ok(Advance::Blocked);
                        }
                        Err(_) => self.start_next_address()?,
                    }
                }
                Phase::Sending => {
                    if let Some(connection) = self.tls.as_mut() {
                        if self.sent < self.request.len() {
                            match connection.writer().write(&self.request[self.sent..]) {
                                Ok(0) => return Err(CONNECTION_FAILED),
                                Ok(written) => {
                                    self.sent += written;
                                    self.progress_at = Some(Instant::now());
                                }
                                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                                Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                                    continue;
                                }
                                Err(_) => return Err(CONNECTION_FAILED),
                            }
                        }
                        if connection.wants_write() {
                            let socket = self.socket.as_mut().ok_or(CONNECTION_FAILED)?;
                            match connection.write_tls(socket) {
                                Ok(0) => return Err(CONNECTION_FAILED),
                                Ok(_) => self.progress_at = Some(Instant::now()),
                                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                                    return Ok(Advance::Blocked);
                                }
                                Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                                    continue;
                                }
                                Err(_) => return Err(CONNECTION_FAILED),
                            }
                        }
                        if self.sent == self.request.len() && !connection.wants_write() {
                            self.phase = Phase::Receiving;
                            continue;
                        }
                    } else {
                        let socket = self.socket.as_mut().ok_or(CONNECTION_FAILED)?;
                        match socket.write(&self.request[self.sent..]) {
                            Ok(0) => return Err(CONNECTION_FAILED),
                            Ok(written) => {
                                self.sent += written;
                                self.progress_at = Some(Instant::now());
                                if self.sent == self.request.len() {
                                    self.phase = Phase::Receiving;
                                }
                            }
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                                return Ok(Advance::Blocked);
                            }
                            Err(_) => return Err(CONNECTION_FAILED),
                        }
                    }
                }
                Phase::Receiving => {
                    if let Some(connection) = self.tls.as_mut() {
                        match connection.reader().read(&mut chunk) {
                            Ok(0) => {
                                self.response.finish_eof(true)?;
                                return Ok(Advance::Complete);
                            }
                            Ok(read) => {
                                self.progress_at = Some(Instant::now());
                                self.response.append_network_bytes(&chunk[..read])?;
                                if self.response.is_complete() {
                                    return Ok(Advance::Complete);
                                }
                            }
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                                continue;
                            }
                            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                                self.response.finish_eof(false)?;
                                return Ok(Advance::Complete);
                            }
                            Err(_) => return Err(CONNECTION_FAILED),
                        }

                        if connection.wants_write() {
                            let socket = self.socket.as_mut().ok_or(CONNECTION_FAILED)?;
                            match connection.write_tls(socket) {
                                Ok(0) => return Err(CONNECTION_FAILED),
                                Ok(_) => {
                                    self.progress_at = Some(Instant::now());
                                    continue;
                                }
                                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                                    return Ok(Advance::Blocked);
                                }
                                Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                                    continue;
                                }
                                Err(_) => return Err(CONNECTION_FAILED),
                            }
                        }

                        let socket = self.socket.as_mut().ok_or(CONNECTION_FAILED)?;
                        match connection.read_tls(socket) {
                            Ok(0) => return Err(CONNECTION_FAILED),
                            Ok(_) => {
                                self.progress_at = Some(Instant::now());
                                connection
                                    .process_new_packets()
                                    .map_err(|_| CONNECTION_FAILED)?;
                                let _ = connection
                                    .writer()
                                    .write(&[])
                                    .map_err(|_| CONNECTION_FAILED)?;
                            }
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                                continue;
                            }
                            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                                return Ok(Advance::Blocked);
                            }
                            Err(_) => return Err(CONNECTION_FAILED),
                        }
                    } else {
                        let read_limit = self.response.read_limit(chunk.len());
                        if read_limit == 0 {
                            return Err(CONNECTION_FAILED);
                        }
                        let socket = self.socket.as_mut().ok_or(CONNECTION_FAILED)?;
                        match socket.read(&mut chunk[..read_limit]) {
                            Ok(0) => {
                                self.response.finish_eof(false)?;
                                return Ok(Advance::Complete);
                            }
                            Ok(read) => {
                                self.progress_at = Some(Instant::now());
                                self.response.append_network_bytes(&chunk[..read])?;
                                if self.response.is_complete() {
                                    return Ok(Advance::Complete);
                                }
                            }
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                                return Ok(Advance::Blocked);
                            }
                            Err(_) => return Err(CONNECTION_FAILED),
                        }
                    }
                }
                Phase::WaitingTls | Phase::Complete | Phase::Failed => {
                    return Err(CONNECTION_FAILED);
                }
            }
        }
        Ok(Advance::Yield)
    }

    fn fail(&mut self) -> AssetFetchError {
        self.phase = Phase::Failed;
        self.drop_socket();
        drop(self.tls.take());
        drop(self.https_ticket.take());
        CONNECTION_FAILED
    }

    fn quiet_deadline(&self) -> Option<Instant> {
        self.progress_at
            .and_then(|at| at.checked_add(self.quiet_limit))
    }
}

impl HostHttpFetch for HttpFetch {
    fn poll(&mut self, until: Instant) -> Result<Option<Vec<u8>>, AssetFetchError> {
        if matches!(self.phase, Phase::Complete | Phase::Failed) {
            return Err(CONNECTION_FAILED);
        }

        let step_deadline = Instant::now()
            .checked_add(FETCH_STEP_BUDGET)
            .map_or(until, |limit| until.min(limit));
        loop {
            let now = Instant::now();
            if now >= step_deadline {
                return Ok(None);
            }
            if self.phase == Phase::WaitingTls {
                let Some(ticket) = self.https_ticket.as_mut() else {
                    return Err(self.fail());
                };
                match ticket.wait_step(step_deadline) {
                    Ok(Some(established)) => {
                        if self.install_https(established).is_err() {
                            return Err(self.fail());
                        }
                        continue;
                    }
                    Ok(None) => return Ok(None),
                    Err(_) => return Err(self.fail()),
                }
            }
            if self.quiet_deadline().is_some_and(|quiet| now >= quiet) {
                return Err(self.fail());
            }

            match self.advance() {
                Ok(Advance::Complete) => {
                    self.phase = Phase::Complete;
                    self.drop_socket();
                    drop(self.tls.take());
                    drop(self.https_ticket.take());
                    return Ok(Some(std::mem::take(&mut self.response.body)));
                }
                Ok(Advance::Yield) => return Ok(None),
                Ok(Advance::Blocked) => {
                    let now = Instant::now();
                    if now >= step_deadline {
                        return Ok(None);
                    }
                    let quiet_deadline = self.quiet_deadline();
                    if quiet_deadline.is_some_and(|quiet| now >= quiet) {
                        return Err(self.fail());
                    }
                    let wake_at =
                        quiet_deadline.map_or(step_deadline, |quiet| quiet.min(step_deadline));
                    std::thread::park_timeout(wake_at.saturating_duration_since(now));
                    if self
                        .quiet_deadline()
                        .is_some_and(|quiet| Instant::now() >= quiet)
                    {
                        return Err(self.fail());
                    }
                    return Ok(None);
                }
                Err(_) => return Err(self.fail()),
            }
        }
    }
}

struct ResponseParser {
    header: Vec<u8>,
    body: Vec<u8>,
    framing: Option<Framing>,
    chunk_state: ChunkState,
    chunk_line: Vec<u8>,
    trailer_bytes: usize,
}

#[derive(Clone, Copy)]
enum Framing {
    ContentLength(usize),
    Chunked,
    CloseDelimited,
}

#[derive(Clone, Copy)]
enum ChunkState {
    SizeLine,
    Data(usize),
    DataCr,
    DataLf,
    Trailers,
    Complete,
}

impl ResponseParser {
    fn new() -> Self {
        Self {
            header: Vec::new(),
            body: Vec::new(),
            framing: None,
            chunk_state: ChunkState::SizeLine,
            chunk_line: Vec::new(),
            trailer_bytes: 0,
        }
    }

    fn read_limit(&self, chunk_capacity: usize) -> usize {
        if self.framing.is_some() {
            chunk_capacity
        } else {
            (MAX_HEADER_BYTES - self.header.len()).min(chunk_capacity)
        }
    }

    fn append_network_bytes(&mut self, bytes: &[u8]) -> Result<(), AssetFetchError> {
        if self.framing.is_some() {
            return self.append_body_bytes(bytes);
        }
        if bytes.len() > MAX_HEADER_BYTES.saturating_sub(self.header.len())
            || self.header.try_reserve(bytes.len()).is_err()
        {
            return Err(CONNECTION_FAILED);
        }
        self.header.extend_from_slice(bytes);
        let Some(split) = self
            .header
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
        else {
            return Ok(());
        };
        // The delimiter starts with the final header line's CRLF. Keep that
        // CRLF in the parsed head; only the second CRLF separates the body.
        let framing = parse_response_headers(&self.header[..split + 2])?;
        let body_start = split + 4;
        let header = std::mem::take(&mut self.header);
        self.framing = Some(framing);
        self.append_body_bytes(&header[body_start..])
    }

    fn append_body_bytes(&mut self, bytes: &[u8]) -> Result<(), AssetFetchError> {
        let framing = self.framing.ok_or(CONNECTION_FAILED)?;
        match framing {
            Framing::ContentLength(expected) => {
                if bytes.len() > expected.saturating_sub(self.body.len()) {
                    return Err(CONNECTION_FAILED);
                }
                self.append_body(bytes)
            }
            Framing::CloseDelimited => self.append_body(bytes),
            Framing::Chunked => self.append_chunked(bytes),
        }
    }

    fn append_body(&mut self, bytes: &[u8]) -> Result<(), AssetFetchError> {
        if bytes.len() > MAX_BODY_BYTES.saturating_sub(self.body.len())
            || self.body.try_reserve(bytes.len()).is_err()
        {
            return Err(CONNECTION_FAILED);
        }
        self.body.extend_from_slice(bytes);
        Ok(())
    }

    fn append_chunked(&mut self, bytes: &[u8]) -> Result<(), AssetFetchError> {
        let mut index = 0;
        while index < bytes.len() {
            match self.chunk_state {
                ChunkState::SizeLine => {
                    let byte = bytes[index];
                    index += 1;
                    self.push_chunk_line(byte)?;
                    if self.chunk_line.ends_with(b"\r\n") {
                        self.chunk_line.truncate(self.chunk_line.len() - 2);
                        let size = parse_chunk_size(&self.chunk_line)?;
                        self.chunk_line.clear();
                        if size > MAX_BODY_BYTES.saturating_sub(self.body.len()) {
                            return Err(CONNECTION_FAILED);
                        }
                        self.chunk_state = if size == 0 {
                            ChunkState::Trailers
                        } else {
                            ChunkState::Data(size)
                        };
                    } else if byte == b'\n' {
                        return Err(CONNECTION_FAILED);
                    }
                }
                ChunkState::Data(remaining) => {
                    let take = remaining.min(bytes.len() - index);
                    self.append_body(&bytes[index..index + take])?;
                    index += take;
                    self.chunk_state = if take == remaining {
                        ChunkState::DataCr
                    } else {
                        ChunkState::Data(remaining - take)
                    };
                }
                ChunkState::DataCr => {
                    if bytes[index] != b'\r' {
                        return Err(CONNECTION_FAILED);
                    }
                    index += 1;
                    self.chunk_state = ChunkState::DataLf;
                }
                ChunkState::DataLf => {
                    if bytes[index] != b'\n' {
                        return Err(CONNECTION_FAILED);
                    }
                    index += 1;
                    self.chunk_state = ChunkState::SizeLine;
                }
                ChunkState::Trailers => {
                    let byte = bytes[index];
                    index += 1;
                    self.trailer_bytes =
                        self.trailer_bytes.checked_add(1).ok_or(CONNECTION_FAILED)?;
                    if self.trailer_bytes > MAX_HEADER_BYTES {
                        return Err(CONNECTION_FAILED);
                    }
                    self.push_chunk_line(byte)?;
                    if self.chunk_line.ends_with(b"\r\n") {
                        self.chunk_line.truncate(self.chunk_line.len() - 2);
                        if self.chunk_line.is_empty() {
                            self.chunk_state = ChunkState::Complete;
                        } else {
                            validate_header_field(&self.chunk_line)?;
                            self.chunk_line.clear();
                        }
                    } else if byte == b'\n' {
                        return Err(CONNECTION_FAILED);
                    }
                }
                ChunkState::Complete => return Err(CONNECTION_FAILED),
            }
        }
        Ok(())
    }

    fn push_chunk_line(&mut self, byte: u8) -> Result<(), AssetFetchError> {
        if self.chunk_line.len() >= MAX_CHUNK_LINE_BYTES || self.chunk_line.try_reserve(1).is_err()
        {
            return Err(CONNECTION_FAILED);
        }
        self.chunk_line.push(byte);
        Ok(())
    }

    fn is_complete(&self) -> bool {
        match self.framing {
            Some(Framing::ContentLength(expected)) => self.body.len() == expected,
            Some(Framing::Chunked) => matches!(self.chunk_state, ChunkState::Complete),
            Some(Framing::CloseDelimited) | None => false,
        }
    }

    fn finish_eof(&self, close_notify: bool) -> Result<(), AssetFetchError> {
        if self.is_complete() {
            return Ok(());
        }
        match self.framing {
            Some(Framing::CloseDelimited) if close_notify => Ok(()),
            _ => Err(CONNECTION_FAILED),
        }
    }
}

fn parse_response_headers(bytes: &[u8]) -> Result<Framing, AssetFetchError> {
    let head = bytes.strip_suffix(b"\n").ok_or(CONNECTION_FAILED)?;
    let mut lines = head.split(|byte| *byte == b'\n');
    let status = lines.next().ok_or(CONNECTION_FAILED)?;
    let status = status.strip_suffix(b"\r").ok_or(CONNECTION_FAILED)?;
    if !status.starts_with(b"HTTP/1.0 200 ") && !status.starts_with(b"HTTP/1.1 200 ") {
        return Err(CONNECTION_FAILED);
    }

    let mut content_length = None;
    let mut transfer_encoding = false;
    for line in lines {
        let line = line.strip_suffix(b"\r").ok_or(CONNECTION_FAILED)?;
        if line.is_empty() {
            return Err(CONNECTION_FAILED);
        }
        let (name, value) = validate_header_field(line)?;
        if name.eq_ignore_ascii_case(b"content-length") {
            let parsed = parse_content_length(trim_ows(value))?;
            if content_length.is_some_and(|old| old != parsed) {
                return Err(CONNECTION_FAILED);
            }
            content_length = Some(parsed);
        } else if name.eq_ignore_ascii_case(b"transfer-encoding") {
            if transfer_encoding || !trim_ows(value).eq_ignore_ascii_case(b"chunked") {
                return Err(CONNECTION_FAILED);
            }
            transfer_encoding = true;
        }
    }

    if transfer_encoding && content_length.is_some() {
        return Err(CONNECTION_FAILED);
    }
    if transfer_encoding {
        Ok(Framing::Chunked)
    } else if let Some(length) = content_length {
        if length > MAX_BODY_BYTES {
            return Err(CONNECTION_FAILED);
        }
        Ok(Framing::ContentLength(length))
    } else {
        Ok(Framing::CloseDelimited)
    }
}

fn validate_header_field(line: &[u8]) -> Result<(&[u8], &[u8]), AssetFetchError> {
    let colon = line
        .iter()
        .position(|byte| *byte == b':')
        .ok_or(CONNECTION_FAILED)?;
    let name = &line[..colon];
    if name.is_empty() || !name.iter().copied().all(is_header_token) {
        return Err(CONNECTION_FAILED);
    }
    let value = &line[colon + 1..];
    if value
        .iter()
        .any(|byte| (*byte < 0x20 && *byte != b'\t') || *byte == 0x7f)
    {
        return Err(CONNECTION_FAILED);
    }
    Ok((name, value))
}

fn is_header_token(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

fn trim_ows(mut value: &[u8]) -> &[u8] {
    while value
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        value = &value[1..];
    }
    while value
        .last()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        value = &value[..value.len() - 1];
    }
    value
}

fn parse_content_length(bytes: &[u8]) -> Result<usize, AssetFetchError> {
    if bytes.is_empty() {
        return Err(CONNECTION_FAILED);
    }
    let mut value = 0usize;
    for byte in bytes {
        if !byte.is_ascii_digit() {
            return Err(CONNECTION_FAILED);
        }
        value = value
            .checked_mul(10)
            .and_then(|value| value.checked_add(usize::from(*byte - b'0')))
            .ok_or(CONNECTION_FAILED)?;
    }
    Ok(value)
}

fn parse_chunk_size(line: &[u8]) -> Result<usize, AssetFetchError> {
    let (digits, extension) = match line.iter().position(|byte| *byte == b';') {
        Some(split) => (&line[..split], Some(&line[split + 1..])),
        None => (line, None),
    };
    if digits.is_empty()
        || !digits.iter().all(u8::is_ascii_hexdigit)
        || extension.is_some_and(|bytes| bytes.iter().any(|byte| !(0x20..=0x7e).contains(byte)))
    {
        return Err(CONNECTION_FAILED);
    }
    let mut size = 0usize;
    for byte in digits {
        let digit = match byte {
            b'0'..=b'9' => usize::from(*byte - b'0'),
            b'a'..=b'f' => usize::from(*byte - b'a' + 10),
            b'A'..=b'F' => usize::from(*byte - b'A' + 10),
            _ => return Err(CONNECTION_FAILED),
        };
        size = size
            .checked_mul(16)
            .and_then(|size| size.checked_add(digit))
            .ok_or(CONNECTION_FAILED)?;
    }
    Ok(size)
}

fn fetch_https_blocking(
    established: EstablishedTls,
    request: &[u8],
    io_limit: Duration,
    cancel: Option<&LoginCancel>,
) -> Result<Vec<u8>, AssetFetchError> {
    let EstablishedTls {
        mut socket,
        mut connection,
    } = established;
    let _cancel_socket = register_cancel_socket(cancel, &socket)?;
    let mut sent = 0;
    while sent < request.len() {
        check_cancel(cancel)?;
        match connection.writer().write(&request[sent..]) {
            Ok(0) => return Err(CONNECTION_FAILED),
            Ok(written) => sent += written,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(CONNECTION_FAILED),
        }
        flush_tls_blocking(&mut connection, &mut socket, io_limit, cancel)?;
    }
    flush_tls_blocking(&mut connection, &mut socket, io_limit, cancel)?;
    let mut response = ResponseParser::new();
    let mut chunk = [0u8; READ_CHUNK_BYTES];
    loop {
        check_cancel(cancel)?;
        match connection.reader().read(&mut chunk) {
            Ok(0) => {
                check_cancel(cancel)?;
                response.finish_eof(true)?;
                return Ok(response.body);
            }
            Ok(read) => {
                response.append_network_bytes(&chunk[..read])?;
                if response.is_complete() {
                    check_cancel(cancel)?;
                    return Ok(response.body);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                response.finish_eof(false)?;
                check_cancel(cancel)?;
                return Ok(response.body);
            }
            Err(_) => return Err(CONNECTION_FAILED),
        }

        flush_tls_blocking(&mut connection, &mut socket, io_limit, cancel)?;
        set_read_timeout(&socket, io_limit)?;
        match connection.read_tls(&mut socket) {
            Ok(0) => return Err(CONNECTION_FAILED),
            Ok(_) => {
                connection
                    .process_new_packets()
                    .map_err(|_| CONNECTION_FAILED)?;
                let _ = connection
                    .writer()
                    .write(&[])
                    .map_err(|_| CONNECTION_FAILED)?;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Err(CONNECTION_FAILED);
            }
            Err(_) => return Err(CONNECTION_FAILED),
        }
        if response.is_complete() {
            check_cancel(cancel)?;
            return Ok(response.body);
        }
    }
}

fn flush_tls_blocking(
    connection: &mut ClientConnection,
    socket: &mut StdTcpStream,
    io_limit: Duration,
    cancel: Option<&LoginCancel>,
) -> Result<(), AssetFetchError> {
    while connection.wants_write() {
        check_cancel(cancel)?;
        set_write_timeout(socket, io_limit)?;
        match connection.write_tls(socket) {
            Ok(0) => return Err(CONNECTION_FAILED),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Err(CONNECTION_FAILED);
            }
            Err(_) => return Err(CONNECTION_FAILED),
        }
    }
    Ok(())
}

fn set_read_timeout(socket: &StdTcpStream, io_limit: Duration) -> Result<(), AssetFetchError> {
    socket
        .set_read_timeout(Some(io_limit))
        .map_err(|_| CONNECTION_FAILED)
}

fn set_write_timeout(socket: &StdTcpStream, io_limit: Duration) -> Result<(), AssetFetchError> {
    socket
        .set_write_timeout(Some(io_limit))
        .map_err(|_| CONNECTION_FAILED)
}

fn check_cancel(cancel: Option<&LoginCancel>) -> Result<(), AssetFetchError> {
    if cancel.is_some_and(LoginCancel::is_cancelled) {
        Err(CONNECTION_FAILED)
    } else {
        Ok(())
    }
}

struct CancelSocketGuard(Arc<Mutex<Option<StdTcpStream>>>);

impl Drop for CancelSocketGuard {
    fn drop(&mut self) {
        drop(lock_cancel_socket(&self.0).take());
    }
}

fn register_cancel_socket(
    cancel: Option<&LoginCancel>,
    socket: &StdTcpStream,
) -> Result<Option<CancelSocketGuard>, AssetFetchError> {
    let Some(cancel) = cancel else {
        return Ok(None);
    };
    check_cancel(Some(cancel))?;

    let shared = Arc::new(Mutex::new(Some(
        socket.try_clone().map_err(|_| CONNECTION_FAILED)?,
    )));
    let callback_shared = Arc::downgrade(&shared);
    cancel.register_cancel_callback(move || {
        if let Some(shared) = callback_shared.upgrade() {
            let socket = lock_cancel_socket(&shared).take();
            if let Some(socket) = socket {
                let _ = socket.shutdown(Shutdown::Both);
            }
        }
    });
    Ok(Some(CancelSocketGuard(shared)))
}

fn lock_cancel_socket(
    socket: &Mutex<Option<StdTcpStream>>,
) -> std::sync::MutexGuard<'_, Option<StdTcpStream>> {
    socket
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn check_blocking_deadline(deadline: Option<Instant>) -> Result<(), AssetFetchError> {
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        Err(CONNECTION_FAILED)
    } else {
        Ok(())
    }
}

fn build_request(host: &str, path: &str) -> Result<Vec<u8>, AssetFetchError> {
    let length = REQUEST_PREFIX
        .len()
        .checked_add(path.len())
        .and_then(|length| length.checked_add(REQUEST_MIDDLE.len()))
        .and_then(|length| length.checked_add(host.len()))
        .and_then(|length| length.checked_add(REQUEST_SUFFIX.len()))
        .ok_or(CONNECTION_FAILED)?;
    if length > MAX_REQUEST_BYTES {
        return Err(CONNECTION_FAILED);
    }

    let mut request = Vec::new();
    request
        .try_reserve_exact(length)
        .map_err(|_| CONNECTION_FAILED)?;
    request.extend_from_slice(REQUEST_PREFIX);
    request.extend_from_slice(path.as_bytes());
    request.extend_from_slice(REQUEST_MIDDLE);
    request.extend_from_slice(host.as_bytes());
    request.extend_from_slice(REQUEST_SUFFIX);
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::ReactorHealth;
    use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    const STEP_BOUND: Duration = Duration::from_millis(100);
    /// How long the silent peer in the cadence test holds its response.
    const PEER_DELAY: Duration = Duration::from_millis(120);
    /// The step budget the cadence test holds the product to. It is fixed here,
    /// not read from `FETCH_STEP_BUDGET`, so that raising the product budget
    /// lowers the poll count and fails the test instead of moving the bound.
    const EXPECTED_STEP_BUDGET: Duration = Duration::from_millis(8);

    /// How long a park of `EXPECTED_STEP_BUDGET` really takes on this host.
    /// A blocked poll parks for at most one step budget, but the OS rounds the
    /// wait up to its timer tick (about 15.6 ms on Windows, where an 8 ms park
    /// measures 15.1-18.9 ms), so the cadence the test can expect is per real
    /// slice, not per nominal budget. Uses the median of several parks and
    /// never reports less than the nominal budget.
    fn observed_step_slice() -> Duration {
        let mut parks: Vec<Duration> = (0..9)
            .map(|_| {
                let started = Instant::now();
                thread::park_timeout(EXPECTED_STEP_BUDGET);
                started.elapsed()
            })
            .collect();
        parks.sort();
        parks[parks.len() / 2].max(EXPECTED_STEP_BUDGET)
    }

    fn test_health() -> Arc<ReactorHealth> {
        Arc::new(ReactorHealth::new(Instant::now()))
    }

    fn read_request_head(socket: &mut impl Read) -> Vec<u8> {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
        }
        head
    }

    fn response(body: &[u8]) -> Vec<u8> {
        let mut response =
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
        response.extend_from_slice(body);
        response
    }

    fn loopback(port: u16) -> [SocketAddr; 1] {
        [SocketAddr::from(([127, 0, 0, 1], port))]
    }

    fn client_config() -> Arc<rustls::ClientConfig> {
        let root = CertificateDer::from_pem_slice(include_bytes!(
            "../../../vendor/fr-client-rust/crates/client/tests/fixtures/tls/ca.pem"
        ))
        .unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(root).unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        Arc::new(
            rustls::ClientConfig::builder_with_provider(provider)
                .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
                .unwrap()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        )
    }

    fn server_config() -> Arc<rustls::ServerConfig> {
        let certificate = CertificateDer::from_pem_slice(include_bytes!(
            "../../../vendor/fr-client-rust/crates/client/tests/fixtures/tls/localhost.pem"
        ))
        .unwrap();
        let key = PrivateKeyDer::from_pem_slice(include_bytes!(
            "../../../vendor/fr-client-rust/crates/client/tests/fixtures/tls/localhost.key"
        ))
        .unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        Arc::new(
            rustls::ServerConfig::builder_with_provider(provider)
                .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(vec![certificate], key)
                .unwrap(),
        )
    }

    fn tls_server_response(socket: StdTcpStream, config: Arc<rustls::ServerConfig>, body: &[u8]) {
        let mut connection = rustls::ServerConnection::new(config).unwrap();
        let mut socket = socket;
        while connection.is_handshaking() {
            connection.complete_io(&mut socket).unwrap();
        }
        let mut tls = rustls::StreamOwned::new(connection, socket);
        let _ = read_request_head(&mut tls);
        tls.write_all(&response(body)).unwrap();
        tls.flush().unwrap();
    }

    fn poll_steps(fetch: &mut dyn HostHttpFetch) -> (Vec<u8>, usize, Duration) {
        let mut polls = 0;
        let mut longest = Duration::ZERO;
        loop {
            let started = Instant::now();
            let result = fetch.poll(started + Duration::from_secs(1));
            longest = longest.max(started.elapsed());
            polls += 1;
            match result {
                Ok(None) => {}
                Ok(Some(body)) => return (body, polls, longest),
                Err(error) => panic!("HTTP fetch failed: {error:?}"),
            }
        }
    }

    #[test]
    fn plain_http_fetch_sends_request_returns_body_and_respects_steps() {
        let observed_slice = observed_step_slice();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (head_tx, head_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            head_tx.send(read_request_head(&mut socket)).unwrap();
            thread::sleep(PEER_DELAY);
            socket.write_all(&response(b"crc-body")).unwrap();
        });

        let https = HttpsLane::start(None, test_health()).unwrap();
        let mut fetch =
            start_fetch(Transport::Tcp, "127.0.0.1", &loopback(port), "/crc", &https).unwrap();
        let (body, polls, longest) = poll_steps(fetch.as_mut());
        server.join().unwrap();
        https.shutdown();

        assert_eq!(body, b"crc-body");
        assert_eq!(
            head_rx.recv().unwrap(),
            b"GET /crc HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n"
        );
        // A blocked poll must hand control back after one step slice. During
        // the silent PEER_DELAY, a poll that waited for the peer instead (or
        // used a slice many times the budget) returns only once or twice. The
        // threshold is two thirds of the polls that fit in the delay at the
        // measured slice: at an exact 8 ms slice that is the original ten.
        let expected_polls = PEER_DELAY.as_secs_f64() / observed_slice.as_secs_f64();
        assert!(
            polls as f64 > expected_polls * (2.0 / 3.0),
            "{polls} polls during a {PEER_DELAY:?} peer delay at a {observed_slice:?} step slice"
        );
        assert!(longest < STEP_BOUND, "longest poll {longest:?}");
    }

    #[test]
    fn blocking_http_fetch_honors_absolute_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (head_tx, head_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            head_tx.send(read_request_head(&mut socket)).unwrap();
            let _ = release_rx.recv_timeout(Duration::from_secs(2));
        });

        let https = HttpsLane::start(None, test_health()).unwrap();
        let started = Instant::now();
        let deadline = started + Duration::from_millis(80);
        let result = fetch_blocking(
            Transport::Tcp,
            "127.0.0.1",
            &loopback(port),
            "/crc",
            Some(deadline),
            &https,
            None,
        );
        let elapsed = started.elapsed();
        assert_eq!(result, Err(AssetFetchError::Connection));
        assert!(
            elapsed < Duration::from_millis(250),
            "fetch took {elapsed:?}"
        );
        assert_eq!(
            head_rx.recv().unwrap(),
            b"GET /crc HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n"
        );
        release_tx.send(()).unwrap();
        server.join().unwrap();
        https.shutdown();
    }

    #[test]
    fn https_fetch_uses_rustls_and_returns_framed_body() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            tls_server_response(socket, server_config(), b"tls-body");
        });
        let https = HttpsLane::start(Some(client_config()), test_health()).unwrap();
        let mut fetch =
            start_fetch(Transport::Wss, "localhost", &loopback(port), "/crc", &https).unwrap();
        let (body, _, _) = poll_steps(fetch.as_mut());
        server.join().unwrap();
        https.shutdown();
        assert_eq!(body, b"tls-body");
    }

    #[test]
    fn raw_eof_requires_complete_explicit_framing() {
        let mut close_delimited = ResponseParser::new();
        close_delimited
            .append_network_bytes(b"HTTP/1.1 200 OK\r\n\r\nbody")
            .unwrap();
        assert!(close_delimited.finish_eof(false).is_err());
        assert!(close_delimited.finish_eof(true).is_ok());

        let mut incomplete = ResponseParser::new();
        incomplete
            .append_network_bytes(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nabc")
            .unwrap();
        assert!(incomplete.finish_eof(false).is_err());

        let mut chunked = ResponseParser::new();
        chunked
            .append_network_bytes(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n",
            )
            .unwrap();
        assert!(chunked.is_complete());
        assert_eq!(chunked.body, b"abc");

        let mut conflicting = ResponseParser::new();
        assert!(conflicting
            .append_network_bytes(
                b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nTransfer-Encoding: chunked\r\n\r\n",
            )
            .is_err());
    }

    #[test]
    fn wss_without_tls_config_fails_closed() {
        let address = SocketAddr::from(([127, 0, 0, 1], 9));
        let https = HttpsLane::start(None, test_health()).unwrap();
        let mut fetch =
            start_fetch(Transport::Wss, "localhost", &[address], "/crc", &https).unwrap();
        assert_eq!(
            fetch.poll(Instant::now() + FETCH_STEP_BUDGET),
            Err(AssetFetchError::Connection)
        );
        assert_eq!(
            fetch_blocking(
                Transport::Wss,
                "localhost",
                &[address],
                "/crc",
                None,
                &https,
                None,
            ),
            Err(AssetFetchError::Connection)
        );
        https.shutdown();
    }
}
