#!/usr/bin/env python3
"""Run the local three-stage WSS soak fault path without logging payloads.

Non-upgrade HTTPS responses end with TLS close_notify after upstream completion;
upgraded WebSockets keep their TLS decoder alive for the peer's Close alert.
"""
from __future__ import annotations

import argparse
import collections
import datetime as dt
import hashlib
import json
import math
import os
import random
import selectors
import signal
import socket
import ssl
import sys
import threading
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable


MAX_HTTP_HEADER_BYTES = 64 * 1024
MAX_WS_FRAME_BYTES = 64 * 1024
TCP_READ_BYTES = 16 * 1024
HASH_CHUNK_BYTES = 4096
CHECKPOINT_SECONDS = 60.0
LISTEN_BACKLOG = 128
INTERNAL_HOST = "127.0.0.1"
PHASES = (
    (1800, 0, 0),
    (1800, 40, 10),
    (1800, 120, 40),
    (1800, 40, 10),
)


class ConfigError(ValueError):
    pass


class FrameError(ValueError):
    pass


@dataclass(frozen=True)
class FaultConfig:
    seed: int
    latency_phases: tuple[tuple[int, int, int], ...]
    stall_one_in: int
    stall_ms: int
    segment_first_bytes: int
    segment_first_write_bytes: int
    segment_small_write_probability: float
    segment_min_bytes: int
    segment_max_bytes: int
    ws_fragment_probability: float
    ws_fragment_min_frames: int
    ws_fragment_max_frames: int
    ws_ping_one_in: int
    diagnostic_disable_ws_faults: bool


@dataclass(frozen=True)
class ProxyConfig:
    listen_host: str
    listen_port: int
    upstream_host: str
    upstream_port: int
    cert_pem: Path
    key_pem: Path
    local_login_modulus: str
    local_login_key_source_sha256: str
    tls_context: ssl.SSLContext
    faults: FaultConfig
    fingerprint: str


def _unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ConfigError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def _required(mapping: Any, key: str, where: str) -> Any:
    if not isinstance(mapping, dict) or key not in mapping:
        raise ConfigError(f"missing required config field {where}.{key}")
    return mapping[key]


def _string(mapping: Any, key: str, where: str) -> str:
    value = _required(mapping, key, where)
    if not isinstance(value, str) or not value:
        raise ConfigError(f"{where}.{key} must be a non-empty string")
    return value


def _integer(mapping: Any, key: str, where: str, minimum: int, maximum: int) -> int:
    value = _required(mapping, key, where)
    if type(value) is not int or value < minimum or value > maximum:
        raise ConfigError(f"{where}.{key} must be an integer from {minimum} through {maximum}")
    return value


def _probability(mapping: Any, key: str, where: str, expected: float) -> float:
    value = _required(mapping, key, where)
    if type(value) not in (int, float):
        raise ConfigError(f"{where}.{key} must be a finite number")
    try:
        finite = math.isfinite(float(value))
    except (OverflowError, ValueError):
        finite = False
    if not finite:
        raise ConfigError(f"{where}.{key} must be a finite number")
    if not math.isclose(float(value), expected, rel_tol=0.0, abs_tol=1e-12):
        raise ConfigError(f"{where}.{key} must equal {expected}")
    return float(value)


def _parse_faults(mapping: Any) -> FaultConfig:
    where = "fault"
    seed = _integer(mapping, "seed", where, 0, (1 << 64) - 1)
    raw_phases = _required(mapping, "latency_phases", where)
    if not isinstance(raw_phases, list) or len(raw_phases) != len(PHASES):
        raise ConfigError("fault.latency_phases must contain the four approved phases")
    phases: list[tuple[int, int, int]] = []
    for index, (entry, expected) in enumerate(zip(raw_phases, PHASES, strict=True)):
        prefix = f"fault.latency_phases[{index}]"
        phase = (
            _integer(entry, "duration_s", prefix, 1, 86400),
            _integer(entry, "latency_ms", prefix, 0, 60000),
            _integer(entry, "jitter_ms", prefix, 0, 60000),
        )
        if phase != expected:
            raise ConfigError(f"{prefix} must equal the approved phase {expected}")
        phases.append(phase)

    stall_one_in = _integer(mapping, "stall_one_in", where, 1, 1000000)
    stall_ms = _integer(mapping, "stall_ms", where, 1, 60000)
    segment_first_bytes = _integer(mapping, "segment_first_bytes", where, 1, 1 << 20)
    segment_first_write_bytes = _integer(mapping, "segment_first_write_bytes", where, 1, 64)
    segment_small_probability = _probability(mapping, "segment_small_write_probability", where, 0.5)
    segment_min_bytes = _integer(mapping, "segment_min_bytes", where, 1, 64)
    segment_max_bytes = _integer(mapping, "segment_max_bytes", where, 1, 64)
    ws_fragment_probability = _probability(mapping, "ws_fragment_probability", where, 0.30)
    ws_fragment_min_frames = _integer(mapping, "ws_fragment_min_frames", where, 2, 4)
    ws_fragment_max_frames = _integer(mapping, "ws_fragment_max_frames", where, 2, 4)
    ws_ping_one_in = _integer(mapping, "ws_ping_one_in", where, 1, 1000000)

    approved = {
        "stall_one_in": (stall_one_in, 500),
        "stall_ms": (stall_ms, 750),
        "segment_first_bytes": (segment_first_bytes, 4096),
        "segment_first_write_bytes": (segment_first_write_bytes, 1),
        "segment_min_bytes": (segment_min_bytes, 1),
        "segment_max_bytes": (segment_max_bytes, 64),
        "ws_fragment_min_frames": (ws_fragment_min_frames, 2),
        "ws_fragment_max_frames": (ws_fragment_max_frames, 4),
        "ws_ping_one_in": (ws_ping_one_in, 100),
    }
    for key, (actual, expected) in approved.items():
        if actual != expected:
            raise ConfigError(f"fault.{key} must equal the approved value {expected}")

    diagnostic = mapping.get("diagnostic_disable_ws_faults", False)
    if type(diagnostic) is not bool:
        raise ConfigError("fault.diagnostic_disable_ws_faults must be a boolean")
    return FaultConfig(
        seed=seed,
        latency_phases=tuple(phases),
        stall_one_in=stall_one_in,
        stall_ms=stall_ms,
        segment_first_bytes=segment_first_bytes,
        segment_first_write_bytes=segment_first_write_bytes,
        segment_small_write_probability=segment_small_probability,
        segment_min_bytes=segment_min_bytes,
        segment_max_bytes=segment_max_bytes,
        ws_fragment_probability=ws_fragment_probability,
        ws_fragment_min_frames=ws_fragment_min_frames,
        ws_fragment_max_frames=ws_fragment_max_frames,
        ws_ping_one_in=ws_ping_one_in,
        diagnostic_disable_ws_faults=diagnostic,
    )


def load_config(path: Path) -> ProxyConfig:
    try:
        raw = path.read_bytes()
    except OSError as exc:
        raise ConfigError(f"cannot read config file: {exc}") from exc
    try:
        document = json.loads(raw, object_pairs_hook=_unique_object)
    except (json.JSONDecodeError, UnicodeDecodeError) as exc:
        raise ConfigError(f"invalid JSON config: {exc}") from exc
    if not isinstance(document, dict):
        raise ConfigError("config root must be a JSON object")

    proxy = _required(document, "proxy", "config")
    listen_host = _string(proxy, "listen_host", "proxy")
    listen_port = _integer(proxy, "listen_port", "proxy", 0, 0)
    upstream_host = _string(proxy, "upstream_host", "proxy")
    upstream_port = _integer(proxy, "upstream_port", "proxy", 1, 65535)
    if listen_host != INTERNAL_HOST:
        raise ConfigError("proxy.listen_host must be 127.0.0.1")
    if upstream_host != INTERNAL_HOST:
        raise ConfigError("proxy.upstream_host must be 127.0.0.1")
    local_login_modulus = _string(proxy, "local_login_modulus", "proxy")
    if (not 250 <= len(local_login_modulus) <= 4096
            or not local_login_modulus.isascii() or not local_login_modulus.isdigit()):
        raise ConfigError("proxy.local_login_modulus must be an Engine RSA decimal modulus")
    source_sha256 = _string(proxy, "local_login_key_source_sha256", "proxy")
    if len(source_sha256) != 64 or any(char not in "0123456789abcdef" for char in source_sha256):
        raise ConfigError("proxy.local_login_key_source_sha256 must be a SHA-256 hex digest")

    config_dir = path.resolve().parent
    cert_path = Path(_string(proxy, "cert_pem", "proxy"))
    key_path = Path(_string(proxy, "key_pem", "proxy"))
    if not cert_path.is_absolute():
        cert_path = config_dir / cert_path
    if not key_path.is_absolute():
        key_path = config_dir / key_path
    for label, candidate in (("proxy.cert_pem", cert_path), ("proxy.key_pem", key_path)):
        if not candidate.is_file():
            raise ConfigError(f"{label} must name an existing regular file")

    faults = _parse_faults(_required(document, "fault", "config"))
    if faults.diagnostic_disable_ws_faults and document.get("run", {}).get("qualification") is not False:
        raise ConfigError("WS fault isolation is only permitted for a nonqualifying smoke")
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    try:
        context.load_cert_chain(certfile=str(cert_path), keyfile=str(key_path))
        # The capture can identify the encrypted Close only with per-run TLS keys.
        context.keylog_filename = str(path.parent / "tls.keys.log")
    except (OSError, ssl.SSLError) as exc:
        raise ConfigError(f"cannot load TLS server identity: {exc}") from exc

    return ProxyConfig(
        listen_host=listen_host,
        listen_port=listen_port,
        upstream_host=upstream_host,
        upstream_port=upstream_port,
        cert_pem=cert_path,
        key_pem=key_path,
        tls_context=context,
        local_login_modulus=local_login_modulus,
        local_login_key_source_sha256=source_sha256,
        faults=faults,
        fingerprint=hashlib.sha256(raw).hexdigest(),
    )


def _utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat(timespec="milliseconds")


def _endpoint_key(address: tuple[Any, ...]) -> tuple[str, int]:
    return str(address[0]), int(address[1])


def _nodelay(sock: socket.socket) -> None:
    sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)


def _safe_shutdown(sock: socket.socket | ssl.SSLSocket) -> None:
    try:
        # SSLSocket.shutdown() discards its TLS state; the other relay worker
        # could then read a raw encrypted record as application data.
        socket.socket.shutdown(sock, socket.SHUT_RDWR)
    except OSError:
        pass

def _close(sock: socket.socket | ssl.SSLSocket | None) -> None:
    if sock is not None:
        try:
            sock.close()
        except OSError:
            pass


def _send_counted(
    sock: socket.socket | ssl.SSLSocket,
    data: bytes | bytearray | memoryview,
    callback: Callable[[int, int], None] | None = None,
) -> tuple[int, int]:
    view = memoryview(data)
    sent = 0
    calls = 0
    while sent < len(view):
        n = sock.send(view[sent:])
        if n <= 0:
            raise ConnectionError("socket send returned zero bytes")
        sent += n
        calls += 1
        if callback is not None:
            callback(n, 1)
    return sent, calls


def _recv_exact(
    sock: socket.socket | ssl.SSLSocket,
    length: int,
    on_bytes: Callable[[int], None] | None = None,
    allow_clean_eof: bool = False,
) -> bytearray | None:
    buffer = bytearray(length)
    view = memoryview(buffer)
    offset = 0
    while offset < length:
        n = sock.recv_into(view[offset:])
        if n == 0:
            if offset == 0 and allow_clean_eof:
                return None
            raise FrameError(f"incomplete frame: expected {length} bytes, received {offset}")
        offset += n
        if on_bytes is not None:
            on_bytes(n)
    return buffer

def _peek_raw(sock: socket.socket | ssl.SSLSocket, limit: int = 64) -> dict[str, Any]:
    try:
        data = sock.recv(limit, socket.MSG_PEEK | socket.MSG_DONTWAIT)
    except (OSError, ValueError) as exc:
        return {"hex": None, "bytes": None, "error": f"{type(exc).__name__}: {exc}"}
    return {"hex": data.hex(), "bytes": len(data), "error": None}


def _record_ws_unparsed_tail(
    context: "ConnectionState",
    client: socket.socket,
    engine: socket.socket,
    direction: str,
    error: BaseException,
) -> None:
    with context.lock:
        if context.unparsed_tail_recorded:
            return
        context.unparsed_tail_recorded = True
    context.runtime.write_event({
        "type": "websocket_unparsed_tail",
        "connection_id": context.identifier,
        "failure_direction": direction,
        "error": f"{type(error).__name__}: {error}",
        "client_to_engine": _peek_raw(client),
        "engine_to_client": _peek_raw(engine),
    })

def _send_half_close(
    sock: socket.socket | ssl.SSLSocket, *, http_response_complete: bool = False
) -> None:
    if isinstance(sock, ssl.SSLSocket):
        if not http_response_complete:
            # A WebSocket can still send a Close or close_notify; retain its decoder.
            return
        # A completed HTTP response needs close_notify, but a blocking
        # SSL_shutdown waits for the peer and can deadlock with proxy A.
        sock.setblocking(False)
        try:
            with selectors.DefaultSelector() as ready:
                ready.register(sock, selectors.EVENT_WRITE)
                deadline = time.monotonic() + 0.5
                while True:
                    try:
                        sock.unwrap()
                        break
                    except ssl.SSLWantReadError:
                        # SSL_shutdown sent the alert. Do not wait for the peer.
                        break
                    except ssl.SSLWantWriteError:
                        remaining = deadline - time.monotonic()
                        if remaining <= 0 or not ready.select(remaining):
                            raise TimeoutError("TLS close_notify write timed out")
                    except ssl.SSLError:
                        # A peer that closed while the complete response was
                        # in flight cannot receive another TLS alert.
                        break
        finally:
            try:
                socket.socket.shutdown(sock, socket.SHUT_WR)
            except OSError:
                pass
        return
    try:
        sock.shutdown(socket.SHUT_WR)
    except OSError:
        pass

def _http_response_complete(context: "ConnectionState") -> bool:
    with context.lock:
        status = context.handshake.get("upgrade_status")
    return type(status) is int and status != 101


def _recv_tls(
    sock: ssl.SSLSocket, context: "ConnectionState", runtime: "Runtime"
) -> bytes | None:
    """Poll for the upstream EOF without holding an SSL_read across SSL_write."""
    with selectors.DefaultSelector() as ready:
        ready.register(sock, selectors.EVENT_READ)
        while True:
            if context.tls_upstream_eof.is_set() and _http_response_complete(context):
                return None
            if runtime.stop.is_set():
                return b""
            with context.tls_io_lock:
                buffered = sock.pending()
            if not buffered and not ready.select(0.05):
                continue
            with context.tls_io_lock:
                if context.tls_upstream_eof.is_set() and _http_response_complete(context):
                    return None
                timeout = sock.gettimeout()
                sock.settimeout(0.05)
                try:
                    return sock.recv(TCP_READ_BYTES)
                except socket.timeout:
                    pass
                finally:
                    sock.settimeout(timeout)


def _send_tls_counted(
    sock: ssl.SSLSocket,
    data: bytes,
    callback: Callable[[int, int], None],
    context: "ConnectionState",
    runtime: "Runtime",
) -> None:
    """Keep SSL_write bounded while the read worker shares the TLS lock."""
    view = memoryview(data)
    sent = 0
    want = selectors.EVENT_WRITE
    with selectors.DefaultSelector() as ready:
        ready.register(sock, want)
        while sent < len(view):
            if runtime.stop.is_set():
                raise ConnectionError("TLS relay stopped during write")
            if not ready.select(0.05):
                continue
            next_want = selectors.EVENT_WRITE
            with context.tls_io_lock:
                timeout = sock.gettimeout()
                sock.settimeout(0.05)
                try:
                    count = sock.send(view[sent:])
                except ssl.SSLWantReadError:
                    next_want = selectors.EVENT_READ
                    count = None
                except (ssl.SSLWantWriteError, socket.timeout):
                    count = None
                finally:
                    sock.settimeout(timeout)
            if next_want != want:
                ready.modify(sock, next_want)
                want = next_want
            if count is None:
                continue
            if count <= 0:
                raise ConnectionError("TLS socket send returned zero bytes")
            sent += count
            callback(count, 1)



class Runtime:
    def __init__(self, config: ProxyConfig, output_dir: Path):
        self.config = config
        self.output_dir = output_dir
        if output_dir.exists() and not output_dir.is_dir():
            raise ConfigError("--output must be a directory")
        output_dir.mkdir(parents=True, exist_ok=True)
        self.log_path = output_dir / "connections.jsonl"
        self.ready_path = output_dir / "ready.json"
        self.start_path = output_dir / "soak-start.json"
        if self.log_path.exists() or self.ready_path.exists() or self.start_path.exists():
            raise ConfigError("output directory already contains proxy receipt files")
        try:
            self.log = self.log_path.open("x", encoding="utf-8", buffering=1)
        except OSError as exc:
            raise ConfigError(f"cannot create JSONL receipt: {exc}") from exc

        self.stop = threading.Event()
        self.started = time.monotonic()
        self.phase_lock = threading.Lock()
        self.phase_zero_monotonic: float | None = None
        self.phase_zero_utc: str | None = None
        self.phase_zero_monotonic_ns: int | None = None
        self.log_lock = threading.Lock()
        self.socket_lock = threading.Lock()
        self.sockets: set[socket.socket | ssl.SSLSocket] = set()
        self.listener_lock = threading.Lock()
        self.listeners: set[socket.socket] = set()
        self.worker_lock = threading.Lock()
        self.workers: set[threading.Thread] = set()
        self.route_condition = threading.Condition()
        self.term_routes: dict[tuple[str, int], ConnectionState] = {}
        self.ws_routes: dict[tuple[str, int], ConnectionState] = {}
        self.context_lock = threading.Lock()
        self.contexts: dict[str, ConnectionState] = {}
        self.next_id = 1
        self.log_failed = False

    def write_event(self, record: dict[str, Any]) -> None:
        encoded = json.dumps({"recorded_at_utc": _utc_now(), **record}, sort_keys=True, separators=(",", ":"))
        try:
            with self.log_lock:
                self.log.write(encoded + "\n")
                self.log.flush()
        except OSError as exc:
            if not self.log_failed:
                self.log_failed = True
                print(f"proxy receipt write failed: {exc}", file=sys.stderr, flush=True)
            self.request_stop()

    def track_socket(self, sock: socket.socket | ssl.SSLSocket) -> None:
        with self.socket_lock:
            self.sockets.add(sock)

    def untrack_socket(self, sock: socket.socket | ssl.SSLSocket) -> None:
        with self.socket_lock:
            self.sockets.discard(sock)

    def track_listener(self, sock: socket.socket) -> None:
        with self.listener_lock:
            self.listeners.add(sock)

    def untrack_listener(self, sock: socket.socket) -> None:
        with self.listener_lock:
            self.listeners.discard(sock)

    def spawn(self, target: Callable[..., None], name: str, *args: Any) -> threading.Thread:
        self.reap_finished()
        thread = threading.Thread(target=target, args=args, name=name, daemon=False)
        with self.worker_lock:
            thread.start()
            self.workers.add(thread)
        return thread

    def reap_finished(self) -> None:
        with self.worker_lock:
            finished = tuple(worker for worker in self.workers if not worker.is_alive())
        for worker in finished:
            worker.join()
        if finished:
            with self.worker_lock:
                self.workers.difference_update(finished)

    def join_pair(self, workers: tuple[threading.Thread, ...]) -> None:
        for worker in workers:
            worker.join()
        with self.worker_lock:
            self.workers.difference_update(workers)

    def join_workers(self) -> None:
        while True:
            with self.worker_lock:
                workers = tuple(self.workers)
            if not workers:
                return
            for worker in workers:
                worker.join()
            with self.worker_lock:
                self.workers.difference_update(worker for worker in workers if not worker.is_alive())
                if not self.workers:
                    return

    def register_route(self, kind: str, address: tuple[Any, ...], context: ConnectionState) -> None:
        key = _endpoint_key(address)
        with self.route_condition:
            routes = self.term_routes if kind == "term" else self.ws_routes
            routes[key] = context
            self.route_condition.notify_all()

    def take_route(self, kind: str, address: tuple[Any, ...]) -> ConnectionState | None:
        key = _endpoint_key(address)
        with self.route_condition:
            routes = self.term_routes if kind == "term" else self.ws_routes
            while key not in routes and not self.stop.is_set():
                self.route_condition.wait(timeout=0.25)
            return routes.pop(key, None)

    def new_context(self, peer: tuple[Any, ...]) -> "ConnectionState":
        with self.context_lock:
            identifier = f"c{self.next_id:06d}"
            self.next_id += 1
            context = ConnectionState(self, identifier, peer)
            self.contexts[identifier] = context
        self.write_event(
            {
                "type": "connection_open",
                "connection_id": identifier,
                "client_peer_host": str(peer[0]),
                "client_peer_port": int(peer[1]),
            }
        )
        return context

    def maybe_checkpoints(self) -> None:
        self.phase_started()
        now = time.monotonic()
        with self.context_lock:
            contexts = tuple(self.contexts.values())
        for context in contexts:
            if context.take_checkpoint_due(now):
                self.write_event({"type": "connection_checkpoint", **context.snapshot()})
        self.reap_finished()

    def phase_started(self) -> bool:
        with self.phase_lock:
            if self.phase_zero_monotonic is not None:
                return True
            try:
                marker = json.loads(self.start_path.read_bytes(), object_pairs_hook=_unique_object)
            except FileNotFoundError:
                return False
            except (OSError, ValueError, UnicodeDecodeError) as exc:
                raise ConfigError(f"invalid atomic soak-start marker: {exc}") from exc
            if (
                not isinstance(marker, dict)
                or marker.get("schema_version") != 1
                or type(marker.get("seed")) is not int
                or marker["seed"] != self.config.faults.seed
                or type(marker.get("monotonic_ns")) is not int
                or marker["monotonic_ns"] <= 0
                or marker["monotonic_ns"] > time.monotonic_ns()
                or not isinstance(marker.get("account"), str)
                or not marker["account"]
                or type(marker.get("wall_clock_unix_ms")) is not int
            ):
                raise ConfigError("soak-start marker does not identify an observed script Start")
            self.phase_zero_monotonic_ns = marker["monotonic_ns"]
            self.phase_zero_monotonic = marker["monotonic_ns"] / 1_000_000_000
            self.phase_zero_utc = dt.datetime.fromtimestamp(
                marker["wall_clock_unix_ms"] / 1000, dt.timezone.utc
            ).isoformat()
            record = {
                "type": "fault_phase_zero",
                "trigger": "first_script_start_after_setup",
                "account": marker["account"],
                "seed": self.config.faults.seed,
                "phase_zero_at_utc": self.phase_zero_utc,
                "phase_zero_monotonic_ns": self.phase_zero_monotonic_ns,
                "phase_zero_elapsed_seconds": round(self.phase_zero_monotonic - self.started, 6),
            }
        self.write_event(record)
        return True
    def finalize_unfinished(self) -> None:
        with self.context_lock:
            contexts = tuple(self.contexts.values())
        for context in contexts:
            for stage in ("proxy_a", "tls_terminator", "proxy_b"):
                context.complete_stage(stage, "stopped")

    def request_stop(self) -> None:
        self.stop.set()
        with self.route_condition:
            self.route_condition.notify_all()
        with self.listener_lock:
            listeners = tuple(self.listeners)
        for listener in listeners:
            try:
                listener.close()
            except OSError:
                pass

    def listener_failed(self, name: str, exc: BaseException) -> None:
        self.write_event({"type": "listener_failure", "listener": name, "error": f"{type(exc).__name__}: {exc}"})
        self.request_stop()

    def close_log(self) -> None:
        try:
            with self.log_lock:
                self.log.flush()
                os.fsync(self.log.fileno())
                self.log.close()
        except OSError as exc:
            print(f"proxy receipt finalization failed: {exc}", file=sys.stderr, flush=True)


class ConnectionState:
    def __init__(self, runtime: Runtime, identifier: str, peer: tuple[Any, ...]):
        self.runtime = runtime
        self.identifier = identifier
        self.peer_host = str(peer[0])
        self.peer_port = int(peer[1])
        self.started = time.monotonic()
        self.started_monotonic_ns = time.monotonic_ns()
        self.started_utc = _utc_now()
        self.next_checkpoint: float | None = None
        self.lock = threading.RLock()
        # Only one worker may enter this TLS object at a time. The read worker
        # owns close_notify after the upstream HTTP response has ended.
        self.tls_io_lock = threading.Lock()
        self.tls_upstream_eof = threading.Event()
        self.stage_status: dict[str, str] = {}
        self.failures: list[dict[str, str]] = []
        self.finalized = False
        self.unparsed_tail_recorded = False
        self.a: dict[str, collections.Counter[str]] = {
            "client_to_terminator_ciphertext": collections.Counter(),
            "terminator_to_client_ciphertext": collections.Counter(),
        }
        self.tls: dict[str, Any] = {
            "handshake": collections.Counter(),
            "client_to_proxy_b_plaintext": collections.Counter(),
            "proxy_b_to_client_plaintext": collections.Counter(),
        }
        self.ws: dict[str, dict[str, Any]] = {
            "client_to_engine": self._new_ws_direction(),
            "engine_to_client": self._new_ws_direction(),
        }
        self.hashes = {
            direction: {"read": hashlib.sha256(), "written": hashlib.sha256()}
            for direction in self.ws
        }
        self.hash_bytes = {
            direction: {"read": 0, "written": 0}
            for direction in self.ws
        }
        self.binary_messages = {
            direction: {
                flow: {
                    "digest": hashlib.sha256(),
                    "rolling_digest": self.hashes[direction][flow].copy(),
                    "bytes": 0,
                }
                for flow in ("read", "written")
            }
            for direction in self.ws
        }
        self.binary_message_indices = {direction: 0 for direction in self.ws}
        self.a_rng = {
            direction: random.Random(self._derived_seed("tcp", direction))
            for direction in self.a
        }
        self.ws_rng = random.Random(self._derived_seed("ws", "server_to_client"))
        self.pending_proxy_ping: bytes | None = None
        self.proxy_ping_serial = 0
        self.handshake: dict[str, Any] = {"request_bytes": 0, "response_bytes": 0, "upgrade_status": None}
        self._fault_phase_chunks: dict[str, collections.Counter[str]] = {
            direction: collections.Counter() for direction in self.a
        }

    @staticmethod
    def _new_ws_direction() -> dict[str, Any]:
        return {
            "counters": collections.Counter(),
            "opcodes_read": collections.Counter(),
            "opcodes_written": collections.Counter(),
            "min_payload_length_read": None,
            "max_payload_length_read": 0,
            "min_payload_length_written": None,
            "max_payload_length_written": 0,
        }

    def _derived_seed(self, lane: str, direction: str) -> int:
        material = f"{self.runtime.config.faults.seed}:{self.identifier}:{lane}:{direction}".encode("ascii")
        return int.from_bytes(hashlib.sha256(material).digest()[:8], "big")

    def begin_proxy_ping(self) -> bytes | None:
        with self.lock:
            if self.pending_proxy_ping is not None:
                return None
            self.proxy_ping_serial += 1
            payload = f"t14:{self.identifier}:{self.proxy_ping_serial:016x}".encode("ascii")
            self.pending_proxy_ping = payload
            return payload

    def consume_proxy_pong(self, payload: bytes) -> bool:
        with self.lock:
            if payload != self.pending_proxy_ping:
                return False
            self.pending_proxy_ping = None
            self.ws["client_to_engine"]["counters"]["injected_pong_frames_consumed"] += 1
            return True

    def inc(self, stage: str, direction: str, key: str, amount: int = 1) -> None:
        with self.lock:
            if stage == "a":
                self.a[direction][key] += amount
            elif stage == "tls":
                self.tls[direction][key] += amount
            elif stage == "ws":
                self.ws[direction]["counters"][key] += amount
            else:
                self.ws[direction]["counters"][f"{stage}_{key}"] += amount

    def inc_ws_opcode(self, direction: str, flow: str, opcode: int) -> None:
        key = "opcodes_read" if flow == "read" else "opcodes_written"
        with self.lock:
            self.ws[direction][key][str(opcode)] += 1

    def ws_payload_length(self, direction: str, flow: str, length: int) -> None:
        with self.lock:
            ws_dir = self.ws[direction]
            min_key = f"min_payload_length_{flow}"
            max_key = f"max_payload_length_{flow}"
            old_min = ws_dir[min_key]
            ws_dir[min_key] = length if old_min is None else min(old_min, length)
            ws_dir[max_key] = max(ws_dir[max_key], length)

    def update_binary_payload(self, direction: str, flow: str, data: bytes | bytearray | memoryview) -> None:
        with self.lock:
            message = self.binary_messages[direction][flow]
            message["digest"].update(data)
            message["rolling_digest"].update(data)
            message["bytes"] += len(data)

    def finish_binary_message(self, direction: str) -> None:
        with self.lock:
            read_message = self.binary_messages[direction]["read"]
            written_message = self.binary_messages[direction]["written"]
            self.binary_message_indices[direction] += 1
            index = self.binary_message_indices[direction]
            read_bytes = read_message["bytes"]
            written_bytes = written_message["bytes"]
            read_digest = read_message["digest"].hexdigest()
            written_digest = written_message["digest"].hexdigest()
            for flow, message in (("read", read_message), ("written", written_message)):
                self.hashes[direction][flow] = message["rolling_digest"]
                self.hash_bytes[direction][flow] += message["bytes"]
            rolling_read = self.hashes[direction]["read"].copy().hexdigest()
            rolling_written = self.hashes[direction]["written"].copy().hexdigest()
            rolling_read_bytes = self.hash_bytes[direction]["read"]
            rolling_written_bytes = self.hash_bytes[direction]["written"]
            for flow in ("read", "written"):
                self.binary_messages[direction][flow] = {
                    "digest": hashlib.sha256(),
                    "rolling_digest": self.hashes[direction][flow].copy(),
                    "bytes": 0,
                }
        self.runtime.write_event(
            {
                "type": "websocket_binary_message",
                "connection_id": self.identifier,
                "client_peer_host": self.peer_host,
                "client_peer_port": self.peer_port,
                "direction": direction,
                "message_index": index,
                "message_bytes_read": read_bytes,
                "message_bytes_written": written_bytes,
                "message_sha256_read": read_digest,
                "message_sha256_written": written_digest,
                "message_hashes_match": read_bytes == written_bytes and read_digest == written_digest,
                "rolling_payload_bytes_read": rolling_read_bytes,
                "rolling_payload_bytes_written": rolling_written_bytes,
                "rolling_payload_sha256_read": rolling_read,
                "rolling_payload_sha256_written": rolling_written,
            }
        )

    def snapshot(self) -> dict[str, Any]:
        with self.lock:
            a = {direction: dict(counters) for direction, counters in self.a.items()}
            tls = {
                key: dict(value) if isinstance(value, collections.Counter) else value
                for key, value in self.tls.items()
            }
            ws: dict[str, Any] = {}
            for direction, state in self.ws.items():
                ws[direction] = {
                    **dict(state["counters"]),
                    "opcodes_read": dict(state["opcodes_read"]),
                    "opcodes_written": dict(state["opcodes_written"]),
                    "min_payload_length_read": state["min_payload_length_read"],
                    "max_payload_length_read": state["max_payload_length_read"],
                    "min_payload_length_written": state["min_payload_length_written"],
                    "max_payload_length_written": state["max_payload_length_written"],
                }
            hashes = {
                direction: {flow: digest.copy().hexdigest() for flow, digest in pair.items()}
                for direction, pair in self.hashes.items()
            }
            hash_bytes = {direction: dict(counts) for direction, counts in self.hash_bytes.items()}
            message_counts = dict(self.binary_message_indices)
            statuses = dict(self.stage_status)
            failures = list(self.failures)
            handshake = dict(self.handshake)
            phase_chunks = {
                direction: dict(counts) for direction, counts in self._fault_phase_chunks.items()
            }
        return {
            "connection_id": self.identifier,
            "client_peer_host": self.peer_host,
            "client_peer_port": self.peer_port,
            "started_at_utc": self.started_utc,
            "elapsed_seconds": round(time.monotonic() - self.started, 3),
            "stage_status": statuses,
            "failures": failures,
            "proxy_a_ciphertext": a,
            "tls_terminator": tls,
            "websocket_handshake": handshake,
            "proxy_b_websocket": ws,
            "binary_payload_sha256": hashes,
            "binary_payload_bytes": hash_bytes,
            "binary_messages_completed": message_counts,
            "binary_payload_hashes_match_at_proxy_b": {
                direction: pair["read"] == pair["written"] for direction, pair in hashes.items()
            },
            "latency_phase_chunk_counts": phase_chunks,
        }

    def fault_random(self, direction: str) -> random.Random:
        return self.a_rng[direction]

    def next_ws_random(self) -> random.Random:
        return self.ws_rng

    def record_phase_chunk(self, direction: str, phase_index: int) -> None:
        key = "pre_phase_zero" if phase_index < 0 else str(phase_index)
        with self.lock:
            self._fault_phase_chunks[direction][key] += 1

    def set_handshake(self, key: str, value: Any) -> None:
        with self.lock:
            self.handshake[key] = value

    def take_checkpoint_due(self, now: float) -> bool:
        with self.runtime.phase_lock:
            phase_zero = self.runtime.phase_zero_monotonic
        if phase_zero is None:
            return False
        with self.lock:
            if self.finalized:
                return False
            if self.next_checkpoint is None:
                elapsed = max(0.0, self.started - phase_zero)
                self.next_checkpoint = phase_zero + (math.floor(elapsed / CHECKPOINT_SECONDS) + 1) * CHECKPOINT_SECONDS
            if now < self.next_checkpoint:
                return False
            while self.next_checkpoint <= now:
                self.next_checkpoint += CHECKPOINT_SECONDS
            return True

    def add_failure(self, stage: str, exc: BaseException) -> None:
        with self.lock:
            self.failures.append({"stage": stage, "error": f"{type(exc).__name__}: {exc}"})

    def complete_stage(self, stage: str, status: str) -> None:
        final_record: dict[str, Any] | None = None
        with self.lock:
            if status not in ("stopped", "not_started") and any(
                failure["stage"] == stage or failure["stage"].startswith(f"{stage}_")
                for failure in self.failures
            ):
                status = "failed"
            self.stage_status.setdefault(stage, status)
            expected = {"proxy_a", "tls_terminator", "proxy_b"}
            if expected.issubset(self.stage_status) and not self.finalized:
                self.finalized = True
                closed_monotonic_ns = time.monotonic_ns()
                final_record = {
                    "connection_id": self.identifier,
                    "client_peer_host": self.peer_host,
                    "client_peer_port": self.peer_port,
                    "started_at_utc": self.started_utc,
                    "started_monotonic_ns": self.started_monotonic_ns,
                    "closed_monotonic_ns": closed_monotonic_ns,
                    "elapsed_seconds": round(time.monotonic() - self.started, 3),
                    "stage_status": dict(self.stage_status),
                    "failures": list(self.failures),
                    "proxy_a_ciphertext": {
                        direction: dict(counters) for direction, counters in self.a.items()
                    },
                    "tls_terminator": {
                        key: dict(value) if isinstance(value, collections.Counter) else value
                        for key, value in self.tls.items()
                    },
                    "websocket_handshake": dict(self.handshake),
                    "proxy_b_websocket": {
                        direction: {
                            **dict(state["counters"]),
                            "opcodes_read": dict(state["opcodes_read"]),
                            "opcodes_written": dict(state["opcodes_written"]),
                            "min_payload_length_read": state["min_payload_length_read"],
                            "max_payload_length_read": state["max_payload_length_read"],
                            "min_payload_length_written": state["min_payload_length_written"],
                            "max_payload_length_written": state["max_payload_length_written"],
                        }
                        for direction, state in self.ws.items()
                    },
                    "binary_payload_sha256": {
                        direction: {flow: digest.copy().hexdigest() for flow, digest in pair.items()}
                        for direction, pair in self.hashes.items()
                    },
                    "binary_payload_bytes": {
                        direction: dict(counts) for direction, counts in self.hash_bytes.items()
                    },
                    "binary_messages_completed": dict(self.binary_message_indices),
                    "binary_payload_hashes_match_at_proxy_b": {
                        direction: pair["read"].copy().hexdigest() == pair["written"].copy().hexdigest()
                        for direction, pair in self.hashes.items()
                    },
                    "latency_phase_chunk_counts": {
                        direction: dict(counts) for direction, counts in self._fault_phase_chunks.items()
                    },
                }
        if final_record is not None:
            self.runtime.write_event({"type": "connection_close", **final_record})
            with self.runtime.context_lock:
                self.runtime.contexts.pop(self.identifier, None)

def _phase(runtime: Runtime) -> tuple[int, int, int, int]:
    with runtime.phase_lock:
        phase_zero = runtime.phase_zero_monotonic
    if phase_zero is None:
        return -1, 0, 0, 0
    elapsed_s = max(0, int(time.monotonic() - phase_zero))
    phases = runtime.config.faults.latency_phases
    offset = 0
    for index, (duration, latency, jitter) in enumerate(phases):
        if elapsed_s < offset + duration:
            return index, latency, jitter, offset + duration - elapsed_s
        offset += duration
    index, latency, jitter = len(phases) - 1, *phases[-1][1:]
    return index, latency, jitter, 1


def _tcp_fault_send(
    runtime: Runtime,
    context: ConnectionState,
    direction: str,
    destination: socket.socket,
    data: bytes,
) -> None:
    faults = runtime.config.faults
    rng = context.fault_random(direction)
    phase_index, latency_ms, jitter_ms, _ = _phase(runtime)
    context.record_phase_chunk(direction, phase_index)
    delay_ms = latency_ms + (rng.randint(-jitter_ms, jitter_ms) if jitter_ms else 0)
    if delay_ms > 0:
        with context.lock:
            counters = context.a[direction]
            counters["latency_delay_ms_total"] += delay_ms
            counters["latency_delay_ms_max"] = max(counters["latency_delay_ms_max"], delay_ms)
        time.sleep(delay_ms / 1000.0)
    if phase_index == 3 and rng.randrange(faults.stall_one_in) == 0:
        context.inc("a", direction, "stalls")
        context.inc("a", direction, "stall_ms_total", faults.stall_ms)
        time.sleep(faults.stall_ms / 1000.0)

    sent_this_chunk = 0

    def on_sent(n: int, calls: int) -> None:
        nonlocal sent_this_chunk
        sent_this_chunk += n
        context.inc("a", direction, "bytes_written", n)
        context.inc("a", direction, "tcp_write_calls", calls)

    initial_remaining = max(0, faults.segment_first_bytes - context.a[direction]["bytes_written"])
    initial_count = min(initial_remaining, len(data))
    if initial_count:
        unit = faults.segment_first_write_bytes
        offset = 0
        while offset < initial_count:
            take = min(unit, initial_count - offset)
            _, calls = _send_counted(destination, memoryview(data)[offset : offset + take], on_sent)
            context.inc("a", direction, "one_byte_writes", calls)
            offset += take

    offset = initial_count
    remaining = len(data) - offset
    if remaining:
        if rng.random() < faults.segment_small_write_probability:
            context.inc("a", direction, "small_write_chunks")
            while offset < len(data):
                size = rng.randint(faults.segment_min_bytes, faults.segment_max_bytes)
                take = min(size, len(data) - offset)
                _, calls = _send_counted(destination, memoryview(data)[offset : offset + take], on_sent)
                context.inc("a", direction, "small_segment_write_calls", calls)
                offset += take
        else:
            _send_counted(destination, memoryview(data)[offset:], on_sent)
            context.inc("a", direction, "normal_write_chunks")
    if sent_this_chunk != len(data):
        raise ConnectionError("TCP proxy did not forward the complete chunk")


def _tcp_copy(
    runtime: Runtime,
    context: ConnectionState,
    source: socket.socket | ssl.SSLSocket,
    destination: socket.socket | ssl.SSLSocket,
    stage: str,
    direction: str,
    use_faults: bool,
) -> None:
    counter_stage = {"proxy_a": "a", "tls_terminator": "tls"}.get(stage, stage)
    try:
        while True:
            tls_reader = stage == "tls_terminator" and direction == "client_to_proxy_b_plaintext"
            tls_writer = stage == "tls_terminator" and direction == "proxy_b_to_client_plaintext"
            data = _recv_tls(source, context, runtime) if tls_reader else source.recv(TCP_READ_BYTES)
            if data is None or not data:
                if tls_writer:
                    # The read owner sends the TLS alert. This worker must
                    # never call unwrap while the other worker is in SSL_read.
                    context.tls_upstream_eof.set()
                    return
                if tls_reader:
                    _send_half_close(destination)
                    # A client EOF observed first means the client cannot
                    # receive close_notify. Only half-close proxy B's read.
                    if data is None and not runtime.stop.is_set() and _http_response_complete(context):
                        with context.tls_io_lock:
                            _send_half_close(source, http_response_complete=True)
                    return
                if stage == "proxy_a":
                    runtime.write_event({
                        "type": "tcp_eof", "connection_id": context.identifier,
                        "direction": direction, "monotonic_ns": time.monotonic_ns(),
                    })
                _send_half_close(destination)
                if stage == "proxy_a":
                    runtime.write_event({
                        "type": "tcp_half_close", "connection_id": context.identifier,
                        "direction": direction, "monotonic_ns": time.monotonic_ns(),
                    })
                return
            context.inc(counter_stage, direction, "bytes_read", len(data))
            context.inc(counter_stage, direction, "chunks_read")
            if use_faults and runtime.phase_started():
                _tcp_fault_send(runtime, context, direction, destination, data)
            else:
                if use_faults:
                    context.record_phase_chunk(direction, -1)
                def on_sent(n: int, calls: int) -> None:
                    context.inc(counter_stage, direction, "bytes_written", n)
                    context.inc(counter_stage, direction, "write_calls" if not use_faults else "tcp_write_calls", calls)

                if tls_writer:
                    _send_tls_counted(destination, data, on_sent, context, runtime)
                else:
                    _send_counted(destination, data, on_sent)
    except (OSError, ssl.SSLError, ConnectionError, ConfigError) as exc:
        pending_client_bytes = 0
        pending_server_bytes = 0
        if stage == "proxy_a":
            with context.lock:
                client = context.a["client_to_terminator_ciphertext"]
                server = context.a["terminator_to_client_ciphertext"]
                pending_client_bytes = client["bytes_read"] - client["bytes_written"]
                pending_server_bytes = server["bytes_read"] - server["bytes_written"]
            runtime.write_event({
                "type": "tcp_copy_error", "connection_id": context.identifier,
                "direction": direction, "monotonic_ns": time.monotonic_ns(),
                "error": repr(exc), "pending_client_bytes": pending_client_bytes,
                "pending_server_bytes": pending_server_bytes,
            })
        if not runtime.stop.is_set():
            context.add_failure(stage, exc)
        pending_opposite = (
            pending_client_bytes if direction == "terminator_to_client_ciphertext"
            else pending_server_bytes
        )
        if stage == "proxy_a" and pending_opposite > 0:
            # The other worker owns bytes it already read. Let it finish its
            # delayed write before the connection owner closes both sockets.
            runtime.write_event({
                "type": ("tcp_reverse_error_draining_client_tail"
                         if direction == "terminator_to_client_ciphertext"
                         else "tcp_forward_error_draining_server_tail"),
                "connection_id": context.identifier,
                "monotonic_ns": time.monotonic_ns(),
                "pending_client_bytes": pending_client_bytes,
                "pending_server_bytes": pending_server_bytes,
            })
            return
        _safe_shutdown(source)
        _safe_shutdown(destination)
        if stage == "proxy_a":
            runtime.write_event({
                "type": "tcp_shutdown_complete", "connection_id": context.identifier,
                "direction": direction, "monotonic_ns": time.monotonic_ns(),
            })


def _relay_pair(
    runtime: Runtime,
    context: ConnectionState,
    left: socket.socket | ssl.SSLSocket,
    right: socket.socket | ssl.SSLSocket,
    stage: str,
    left_to_right: str,
    right_to_left: str,
    use_faults: bool = False,
) -> None:
    workers = (
        runtime.spawn(
            _tcp_copy,
            f"{context.identifier}-{stage}-lr",
            runtime,
            context,
            left,
            right,
            stage,
            left_to_right,
            use_faults,
        ),
        runtime.spawn(
            _tcp_copy,
            f"{context.identifier}-{stage}-rl",
            runtime,
            context,
            right,
            left,
            stage,
            right_to_left,
            use_faults,
        ),
    )
    runtime.join_pair(workers)


def _listener(host: str, port: int, runtime: Runtime) -> socket.socket:
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        sock.bind((host, port))
        sock.listen(LISTEN_BACKLOG)
        sock.settimeout(0.25)
        runtime.track_listener(sock)
        return sock
    except BaseException:
        sock.close()
        raise


def _connection_status(runtime: Runtime, failed: bool) -> str:
    if runtime.stop.is_set():
        return "stopped"
    return "failed" if failed else "closed"


def _a_connection(runtime: Runtime, client: socket.socket, peer: tuple[Any, ...], term_address: tuple[str, int]) -> None:
    context = runtime.new_context(peer)
    term: socket.socket | None = None
    failed = False
    runtime.track_socket(client)
    try:
        _nodelay(client)
        term = socket.create_connection(term_address)
        _nodelay(term)
        runtime.track_socket(term)
        runtime.register_route("term", term.getsockname(), context)
        runtime.write_event({
            "type": "proxy_a_route", "connection_id": context.identifier,
            "client_peer_port": peer[1], "term_peer_port": term.getsockname()[1],
            "a_port": client.getsockname()[1],
            "terminator_port": term.getpeername()[1],
            "monotonic_ns": time.monotonic_ns(),
        })
        _relay_pair(
            runtime,
            context,
            client,
            term,
            "proxy_a",
            "client_to_terminator_ciphertext",
            "terminator_to_client_ciphertext",
            use_faults=True,
        )
    except (OSError, ssl.SSLError, ConnectionError) as exc:
        failed = True
        context.add_failure("proxy_a", exc)
        _safe_shutdown(client)
        _safe_shutdown(term) if term is not None else None
        if term is None:
            context.complete_stage("tls_terminator", "not_started")
            context.complete_stage("proxy_b", "not_started")
    finally:
        _close(client)
        runtime.untrack_socket(client)
        _close(term)
        if term is not None:
            runtime.untrack_socket(term)
        context.complete_stage("proxy_a", _connection_status(runtime, failed))


def _terminator_connection(
    runtime: Runtime,
    raw: socket.socket,
    peer: tuple[Any, ...],
    ws_address: tuple[str, int],
) -> None:
    context = runtime.take_route("term", peer)
    if context is None:
        _close(raw)
        runtime.untrack_socket(raw)
        return
    runtime.track_socket(raw)
    secure: ssl.SSLSocket | None = None
    plain: socket.socket | None = None
    failed = False
    try:
        _nodelay(raw)
        secure = runtime.config.tls_context.wrap_socket(raw, server_side=True)
        runtime.untrack_socket(raw)
        runtime.track_socket(secure)
        with context.lock:
            context.tls["handshake"]["completed"] = 1
            context.tls["handshake"]["tls_version"] = secure.version()
            cipher = secure.cipher()
            context.tls["handshake"]["cipher"] = cipher[0] if cipher else None
        plain = socket.create_connection(ws_address)
        _nodelay(plain)
        runtime.track_socket(plain)
        runtime.register_route("ws", plain.getsockname(), context)
        _relay_pair(
            runtime,
            context,
            secure,
            plain,
            "tls_terminator",
            "client_to_proxy_b_plaintext",
            "proxy_b_to_client_plaintext",
        )
    except (OSError, ssl.SSLError, ConnectionError) as exc:
        failed = True
        context.add_failure("tls_terminator", exc)
        _safe_shutdown(secure) if secure is not None else _safe_shutdown(raw)
        _safe_shutdown(plain) if plain is not None else None
        if plain is None:
            context.complete_stage("proxy_b", "not_started")
    finally:
        _close(secure)
        _close(plain)
        if secure is not None:
            runtime.untrack_socket(secure)
        if plain is not None:
            runtime.untrack_socket(plain)
        if secure is None:
            _close(raw)
            runtime.untrack_socket(raw)
        context.complete_stage("tls_terminator", _connection_status(runtime, failed))


def _read_http_headers(
    source: socket.socket,
    context: ConnectionState,
    direction: str,
    kind: str,
) -> bytes | None:
    data = bytearray()
    while len(data) < MAX_HTTP_HEADER_BYTES:
        chunk = source.recv(1)
        if not chunk:
            if not data:
                return None
            raise FrameError(f"incomplete {kind} HTTP headers")
        data.extend(chunk)
        context.inc("ws", direction, "http_header_bytes_read")
        if data.endswith(b"\r\n\r\n"):
            return bytes(data)
    raise FrameError(f"{kind} HTTP headers exceed {MAX_HTTP_HEADER_BYTES} bytes")


def _http_status(response: bytes) -> int:
    line = response.split(b"\r\n", 1)[0]
    parts = line.split(b" ")
    if len(parts) < 2 or not parts[0].startswith(b"HTTP/") or not parts[1].isdigit():
        raise FrameError("invalid HTTP response status line")
    return int(parts[1])


def _is_websocket_request(request: bytes) -> bool:
    lines = request.split(b"\r\n")
    if not lines or not lines[0].startswith(b"GET ") or not lines[0].endswith(b" HTTP/1.1"):
        return False
    return any(
        line.partition(b":")[0].strip().lower() == b"upgrade"
        and b"websocket" in line.partition(b":")[2].strip().lower()
        for line in lines[1:]
    )


def _send_ws_wire(
    destination: socket.socket,
    context: ConnectionState,
    direction: str,
    data: bytes | bytearray | memoryview,
) -> None:
    def on_sent(n: int, calls: int) -> None:
        context.inc("ws", direction, "wire_bytes_written", n)
        context.inc("ws", direction, "write_calls", calls)

    _send_counted(destination, data, on_sent)

def _send_http_headers(
    destination: socket.socket,
    context: ConnectionState,
    direction: str,
    data: bytes,
) -> None:
    def on_sent(n: int, calls: int) -> None:
        context.inc("ws", direction, "http_header_bytes_written", n)
        context.inc("ws", direction, "http_header_write_calls", calls)

    _send_counted(destination, data, on_sent)


def _read_frame(
    source: socket.socket,
    context: ConnectionState,
    direction: str,
    expect_masked: bool,
) -> dict[str, Any] | None:
    def received(n: int) -> None:
        context.inc("ws", direction, "wire_bytes_read", n)
    with context.lock:
        stream_byte_offset = context.ws[direction]["counters"]["wire_bytes_read"]


    first = _recv_exact(source, 2, received, allow_clean_eof=True)
    if first is None:
        return None
    b0, b1 = first
    fin = bool(b0 & 0x80)
    rsv = (b0 >> 4) & 0x07
    opcode = b0 & 0x0F
    masked = bool(b1 & 0x80)
    length_code = b1 & 0x7F
    length: int | None = length_code if length_code < 126 else None

    def fail(message: str) -> None:
        next_bytes = _peek_raw(source)
        decoded_length = length
        if decoded_length is None and next_bytes["hex"] is not None:
            extension_size = 2 if length_code == 126 else 8
            extension = bytes.fromhex(next_bytes["hex"])[:extension_size]
            if len(extension) == extension_size:
                decoded_length = int.from_bytes(extension, "big")
        context.runtime.write_event({
            "type": "websocket_frame_error",
            "connection_id": context.identifier,
            "direction": direction,
            "source_endpoint": "client" if expect_masked else "engine",
            "stream_byte_offset": stream_byte_offset,
            "header_hex": bytes(first).hex(),
            "opcode": opcode,
            "masked": masked,
            "length_code": length_code,
            "decoded_length": decoded_length,
            "next_64_raw_bytes": next_bytes,
            "error": message,
        })
        raise FrameError(message)

    if rsv & 0x06:
        fail("RSV2 or RSV3 bit is set without a supported extension")
    if masked != expect_masked:
        fail("client frames must be masked and server frames must not be masked")
    if opcode not in (0, 1, 2, 8, 9, 10):
        fail(f"reserved WebSocket opcode {opcode}")
    if opcode == 0 and rsv:
        fail("continuation frame has a reserved bit set")
    if opcode >= 8 and rsv:
        fail("control frame has a reserved bit set")
    if opcode >= 8 and not fin:
        fail("control frame is fragmented")

    header_parts = [bytes(first)]
    if length_code == 126:
        extended = _recv_exact(source, 2, received)
        assert extended is not None
        header_parts.append(bytes(extended))
        length = int.from_bytes(extended, "big")
        if length < 126:
            fail("non-minimal 16-bit payload length")
    elif length_code == 127:
        extended = _recv_exact(source, 8, received)
        assert extended is not None
        header_parts.append(bytes(extended))
        length = int.from_bytes(extended, "big")
        if extended[0] & 0x80:
            fail("invalid 64-bit payload length high bit")
        if length < 65536:
            fail("non-minimal 64-bit payload length")
    else:
        length = length_code
    if opcode >= 8 and length > 125:
        fail("control frame payload exceeds 125 bytes")
    if length > MAX_WS_FRAME_BYTES:
        fail(f"WebSocket frame length {length} exceeds maximum {MAX_WS_FRAME_BYTES}")

    mask_key: bytes | None = None
    if masked:
        key = _recv_exact(source, 4, received)
        assert key is not None
        mask_key = bytes(key)
        header_parts.append(mask_key)
    payload = _recv_exact(source, length, received)
    assert payload is not None
    raw_header = b"".join(header_parts)
    if rsv & 1 and opcode not in (1, 2):
        fail("RSV1 is only valid on an initial data frame")
    return {
        "fin": fin,
        "opcode": opcode,
        "rsv1": bool(rsv & 1),
        "masked": masked,
        "mask_key": mask_key,
        "header": raw_header,
        "payload": payload,
        "length": length,
    }


def _validate_message_frame(frame: dict[str, Any], fragmented_opcode: int | None) -> int | None:
    opcode = frame["opcode"]
    fin = frame["fin"]
    if opcode >= 8:
        return fragmented_opcode
    if opcode == 0:
        if fragmented_opcode is None:
            raise FrameError("continuation frame has no open fragmented message")
        return None if fin else fragmented_opcode
    if fragmented_opcode is not None:
        raise FrameError("new data frame arrived before the fragmented message ended")
    return None if fin else opcode


def _unmask_hash(
    payload: bytearray,
    mask_key: bytes,
    context: ConnectionState,
    direction: str,
    flow: str,
) -> None:
    scratch = bytearray(min(HASH_CHUNK_BYTES, len(payload)))
    offset = 0
    while offset < len(payload):
        size = min(len(scratch), len(payload) - offset)
        for index in range(size):
            scratch[index] = payload[offset + index] ^ mask_key[(offset + index) & 3]
        context.update_binary_payload(direction, flow, memoryview(scratch)[:size])
        offset += size


def _payload_hash(
    context: ConnectionState,
    direction: str,
    flow: str,
    payload: bytearray,
    mask_key: bytes | None,
) -> None:
    if mask_key is None:
        context.update_binary_payload(direction, flow, payload)
    else:
        _unmask_hash(payload, mask_key, context, direction, flow)


def _record_frame_read(context: ConnectionState, direction: str, frame: dict[str, Any]) -> None:
    context.inc("ws", direction, "frames_read")
    context.inc("ws", direction, "payload_bytes_read", frame["length"])
    context.inc_ws_opcode(direction, "read", frame["opcode"])
    context.ws_payload_length(direction, "read", frame["length"])


def _record_frame_written(context: ConnectionState, direction: str, opcode: int, payload_length: int) -> None:
    context.inc("ws", direction, "frames_written")
    context.inc("ws", direction, "payload_bytes_written", payload_length)
    context.inc_ws_opcode(direction, "written", opcode)
    context.ws_payload_length(direction, "written", payload_length)


def _emit_raw_frame(
    destination: socket.socket,
    context: ConnectionState,
    direction: str,
    frame: dict[str, Any],
) -> None:
    _send_ws_wire(destination, context, direction, frame["header"])
    _send_ws_wire(destination, context, direction, frame["payload"])
    _record_frame_written(context, direction, frame["opcode"], frame["length"])


def _frame_header(fin: bool, opcode: int, payload_length: int, rsv1: bool = False) -> bytes:
    first = (0x80 if fin else 0) | (0x40 if rsv1 else 0) | opcode
    if payload_length < 126:
        return bytes((first, payload_length))
    if payload_length < 65536:
        return bytes((first, 126)) + payload_length.to_bytes(2, "big")
    return bytes((first, 127)) + payload_length.to_bytes(8, "big")


def _emit_frame_parts(
    destination: socket.socket,
    context: ConnectionState,
    direction: str,
    opcode: int,
    fin: bool,
    payload: memoryview,
    rsv1: bool = False,
    hash_data: bool = False,
) -> None:
    header = _frame_header(fin, opcode, len(payload), rsv1)
    _send_ws_wire(destination, context, direction, header)
    _send_ws_wire(destination, context, direction, payload)
    _record_frame_written(context, direction, opcode, len(payload))
    if hash_data and opcode in (0, 2):
        context.update_binary_payload(direction, "written", payload)


def _fragment_server_frame(
    destination: socket.socket,
    context: ConnectionState,
    direction: str,
    frame: dict[str, Any],
) -> None:
    rng = context.next_ws_random()
    piece_count = rng.randint(
        context.runtime.config.faults.ws_fragment_min_frames,
        context.runtime.config.faults.ws_fragment_max_frames,
    )
    payload: bytearray = frame["payload"]
    cuts = sorted(rng.randint(0, len(payload)) for _ in range(piece_count - 1))
    edges = [0, *cuts, len(payload)]
    inject_ping = rng.randrange(context.runtime.config.faults.ws_ping_one_in) == 0
    for index in range(piece_count):
        start, end = edges[index], edges[index + 1]
        _emit_frame_parts(
            destination,
            context,
            direction,
            opcode=2 if index == 0 else 0,
            fin=index == piece_count - 1,
            payload=memoryview(payload)[start:end],
            rsv1=frame["rsv1"] if index == 0 else False,
            hash_data=True,
        )
        context.inc("ws", direction, "fault_fragment_frames")
        if index == 0 and inject_ping:
            ping_payload = context.begin_proxy_ping()
            if ping_payload is not None:
                _emit_frame_parts(destination, context, direction, 9, True, memoryview(ping_payload))
                context.inc("ws", direction, "injected_ping_frames")
    context.inc("ws", direction, "fragmented_messages")


def _relay_ws_direction(
    runtime: Runtime,
    context: ConnectionState,
    source: socket.socket,
    destination: socket.socket,
    direction: str,
    expect_masked: bool,
) -> None:
    fragmented_opcode: int | None = None
    try:
        while True:
            frame = _read_frame(source, context, direction, expect_masked)
            if frame is None:
                _send_half_close(destination)
                return
            previous_fragmented_opcode = fragmented_opcode
            binary_message = frame["opcode"] == 2 or (
                frame["opcode"] == 0 and previous_fragmented_opcode == 2
            )
            fragmented_opcode = _validate_message_frame(frame, fragmented_opcode)
            _record_frame_read(context, direction, frame)
            if expect_masked and frame["opcode"] == 10:
                mask_key = frame["mask_key"]
                assert mask_key is not None
                pong = bytes(byte ^ mask_key[index & 3]
                             for index, byte in enumerate(frame["payload"]))
                if context.consume_proxy_pong(pong):
                    runtime.write_event({
                        "type": "proxy_injected_pong_consumed",
                        "connection_id": context.identifier,
                        "payload_sha256": hashlib.sha256(pong).hexdigest(),
                    })
                    continue
            if frame["opcode"] == 8:
                payload = frame["payload"]
                mask_key = frame["mask_key"]
                if mask_key is not None:
                    payload = bytes(byte ^ mask_key[index & 3] for index, byte in enumerate(payload))
                runtime.write_event({
                    "type": "websocket_close_frame",
                    "connection_id": context.identifier,
                    "direction": direction,
                    "payload_len": len(payload),
                    "status_code": int.from_bytes(payload[:2], "big") if len(payload) >= 2 else None,
                    "reason": payload[2:].decode("utf-8", errors="replace") if len(payload) > 2 else "",
                })
            if binary_message:
                _payload_hash(context, direction, "read", frame["payload"], frame["mask_key"])

            if expect_masked:
                _emit_raw_frame(destination, context, direction, frame)
                if binary_message:
                    _payload_hash(context, direction, "written", frame["payload"], frame["mask_key"])
            elif (
                runtime.phase_started()
                and not runtime.config.faults.diagnostic_disable_ws_faults
                and frame["opcode"] == 2
                and frame["fin"]
                and context.next_ws_random().random() < runtime.config.faults.ws_fragment_probability
            ):
                _fragment_server_frame(destination, context, direction, frame)
            else:
                _emit_raw_frame(destination, context, direction, frame)
                if binary_message:
                    _payload_hash(context, direction, "written", frame["payload"], frame["mask_key"])

            if frame["opcode"] < 8 and frame["fin"]:
                context.inc("ws", direction, "data_messages_read")
                context.inc("ws", direction, "data_messages_written")
                if binary_message:
                    context.inc("ws", direction, "binary_messages_read")
                    context.inc("ws", direction, "binary_messages_written")
                    context.finish_binary_message(direction)
    except (OSError, ssl.SSLError, FrameError, ConnectionError, ConfigError) as exc:
        _record_ws_unparsed_tail(
            context,
            source if expect_masked else destination,
            destination if expect_masked else source,
            direction,
            exc,
        )
        if not runtime.stop.is_set():
            context.add_failure("proxy_b_websocket", exc)
        _safe_shutdown(source)
        _safe_shutdown(destination)

def _relay_ws_pair(runtime: Runtime, context: ConnectionState, client: socket.socket, engine: socket.socket) -> None:
    workers = (
        runtime.spawn(
            _relay_ws_direction,
            f"{context.identifier}-proxy_b-client-ws",
            runtime,
            context,
            client,
            engine,
            "client_to_engine",
            True,
        ),
        runtime.spawn(
            _relay_ws_direction,
            f"{context.identifier}-proxy_b-server-ws",
            runtime,
            context,
            engine,
            client,
            "engine_to_client",
            False,
        ),
    )
    runtime.join_pair(workers)


def _proxy_b_connection(runtime: Runtime, client: socket.socket, peer: tuple[Any, ...]) -> None:
    context = runtime.take_route("ws", peer)
    if context is None:
        _close(client)
        runtime.untrack_socket(client)
        return
    runtime.track_socket(client)
    engine: socket.socket | None = None
    failed = False
    try:
        _nodelay(client)
        request = _read_http_headers(client, context, "client_to_engine", "request")
        if request is None:
            context.set_handshake("upgrade_status", "client_closed_before_request")
            _send_half_close(client)
            return
        context.set_handshake("request_bytes", len(request))
        if request.split(b"\r\n", 1)[0] in (
            b"GET /client/client.js HTTP/1.0",
            b"GET /client/client.js HTTP/1.1",
        ):
            # Engine A's bundled client.js lacks the local private key's
            # modulus. Serve only this read-only test asset; never rewrite a
            # game packet, WebSocket upgrade, or any other HTTP resource.
            body = f'const loginRsaN = "{runtime.config.local_login_modulus}";\n'.encode("ascii")
            headers = (
                b"HTTP/1.0 200 OK\r\nContent-Type: application/javascript\r\n"
                + f"Content-Length: {len(body)}\r\nConnection: close\r\n\r\n".encode("ascii")
            )
            context.set_handshake("upgrade_status", 200)
            context.set_handshake("response_bytes", len(headers))
            context.set_handshake("local_login_key_fixture", {
                "rsa_source_sha256": runtime.config.local_login_key_source_sha256,
                "served_body_sha256": hashlib.sha256(body).hexdigest(),
                "body_bytes": len(body),
            })
            _send_http_headers(client, context, "engine_to_client", headers)
            _send_ws_wire(client, context, "engine_to_client", body)
            return
        engine = socket.create_connection((runtime.config.upstream_host, runtime.config.upstream_port))
        _nodelay(engine)
        runtime.track_socket(engine)
        _send_http_headers(engine, context, "client_to_engine", request)
        websocket_request = _is_websocket_request(request)
        response = _read_http_headers(engine, context, "engine_to_client", "response")
        if response is None:
            context.set_handshake("upgrade_status", "engine_closed_before_response")
            _send_half_close(client)
            return
        status = _http_status(response)
        context.set_handshake("response_bytes", len(response))
        context.set_handshake("upgrade_status", status)
        _send_http_headers(client, context, "engine_to_client", response)
        if status == 101:
            if not websocket_request:
                raise FrameError("Engine returned 101 for a non-WebSocket request")
            # The first 101 belongs to unfaulted setup; the Play witness
            # publishes phase zero only after every slot finishes its Relog.
            _relay_ws_pair(runtime, context, client, engine)
        else:
            _relay_pair(
                runtime,
                context,
                client,
                engine,
                "proxy_b_http_fallback",
                "client_to_engine",
                "engine_to_client",
            )
    except (OSError, ssl.SSLError, FrameError, ConnectionError) as exc:
        failed = True
        context.add_failure("proxy_b", exc)
        _safe_shutdown(client)
        _safe_shutdown(engine) if engine is not None else None
    finally:
        _close(client)
        _close(engine)
        runtime.untrack_socket(client)
        if engine is not None:
            runtime.untrack_socket(engine)
        context.complete_stage("proxy_b", _connection_status(runtime, failed))

def _accept_loop(runtime: Runtime, listener: socket.socket, name: str, ws_address: tuple[str, int] | None = None) -> None:
    try:
        while not runtime.stop.is_set():
            try:
                connection, peer = listener.accept()
            except socket.timeout:
                continue
            except OSError:
                if runtime.stop.is_set():
                    return
                raise
            try:
                _nodelay(connection)
                runtime.track_socket(connection)
                if name == "terminator":
                    assert ws_address is not None
                    runtime.spawn(_terminator_connection, f"terminator-{peer[1]}", runtime, connection, peer, ws_address)
                elif name == "proxy_b":
                    runtime.spawn(_proxy_b_connection, f"proxy-b-{peer[1]}", runtime, connection, peer)
                else:
                    raise ValueError(f"unknown listener role: {name}")
            except BaseException:
                _close(connection)
                runtime.untrack_socket(connection)
                raise
    except BaseException as exc:
        if not runtime.stop.is_set():
            runtime.listener_failed(name, exc)
    finally:
        runtime.untrack_listener(listener)
        _close(listener)


def _atomic_ready(runtime: Runtime, ready: dict[str, Any]) -> None:
    temporary = runtime.output_dir / f".ready.json.{os.getpid()}.tmp"
    data = json.dumps(ready, sort_keys=True, separators=(",", ":")) + "\n"
    try:
        with temporary.open("x", encoding="utf-8") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, runtime.ready_path)
        directory_fd = os.open(runtime.output_dir, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    except OSError:
        try:
            temporary.unlink(missing_ok=True)
        except OSError:
            pass
        raise




def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        config = load_config(args.config)
        runtime = Runtime(config, args.output)

        def request_stop(_signum: int, _frame: Any) -> None:
            runtime.request_stop()

        for signum in (signal.SIGINT, signal.SIGTERM):
            signal.signal(signum, request_stop)
        return _run_runtime(runtime)
    except (ConfigError, OSError) as exc:
        print(f"configuration error: {exc}", file=sys.stderr)
        return 2

def _run_runtime(runtime: Runtime) -> int:
    config = runtime.config
    b_listener: socket.socket | None = None
    term_listener: socket.socket | None = None
    a_listener: socket.socket | None = None
    try:
        b_listener = _listener(INTERNAL_HOST, 0, runtime)
        term_listener = _listener(INTERNAL_HOST, 0, runtime)
        a_listener = _listener(config.listen_host, config.listen_port, runtime)
        b_address = (INTERNAL_HOST, int(b_listener.getsockname()[1]))
        term_address = (INTERNAL_HOST, int(term_listener.getsockname()[1]))
        a_address = (config.listen_host, int(a_listener.getsockname()[1]))

        runtime.spawn(_accept_loop, "terminator-accept", runtime, term_listener, "terminator", b_address)
        runtime.spawn(_accept_loop, "proxy-b-accept", runtime, b_listener, "proxy_b")
        ready = {
            "a_host": a_address[0],
            "a_port": a_address[1],
            "endpoint": f"wss://{a_address[0]}:{a_address[1]}",
            "asset_endpoint": f"https://{a_address[0]}:{a_address[1]}",
            "terminator_host": term_address[0],
            "terminator_port": term_address[1],
            "reframer_host": b_address[0],
            "reframer_port": b_address[1],
            "upstream_host": config.upstream_host,
            "upstream_port": config.upstream_port,
            "seed": config.faults.seed,
            "config_sha256": config.fingerprint,
            "jsonl": "connections.jsonl",
        }
        _atomic_ready(runtime, ready)
        print("READY " + json.dumps(ready, sort_keys=True, separators=(",", ":")), flush=True)

        runtime.track_listener(a_listener)
        while not runtime.stop.is_set():
            try:
                connection, peer = a_listener.accept()
            except socket.timeout:
                runtime.maybe_checkpoints()
                continue
            except OSError:
                if runtime.stop.is_set():
                    break
                raise
            _nodelay(connection)
            runtime.track_socket(connection)
            runtime.spawn(
                _a_connection,
                f"proxy-a-{peer[1]}",
                runtime,
                connection,
                peer,
                term_address,
            )
            runtime.maybe_checkpoints()
    except BaseException as exc:
        if not runtime.stop.is_set():
            print(f"proxy failed: {type(exc).__name__}: {exc}", file=sys.stderr, flush=True)
            runtime.request_stop()
        return 1
    finally:
        runtime.request_stop()
        for listener in (a_listener, term_listener, b_listener):
            if listener is not None:
                runtime.untrack_listener(listener)
                _close(listener)
        runtime.join_workers()
        runtime.finalize_unfinished()
        runtime.maybe_checkpoints()
        runtime.write_event({"type": "proxy_stopped", "elapsed_seconds": round(time.monotonic() - runtime.started, 3)})
        runtime.close_log()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
