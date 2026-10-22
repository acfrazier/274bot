#!/usr/bin/env python3
"""Userspace TCP delay proxy for the nightly latency lane.

The proxy sits between one cell process and the engine game port. It delays
each direction of a game connection by one fixed one-way delay, in FIFO
order, so byte order cannot change.

First-byte rule: a game login writes byte 14 first. The OnDemand update
socket writes byte 15 first and shares the game port. Byte 15 takes the
direct path with no queue delay. Byte 14 takes the delayed path. Any other
first byte takes the delayed path. The stats record names the byte value.

Fail-closed rules: a bad argument, a failed bind, or an unwritable stats
file exits nonzero before the ready file exists. A dead upstream closes the
client connection at once and writes an `upstream_unavailable` stats
record. A full delay queue closes that one connection and writes a
`queue_overflow` record. The proxy never drops bytes and continues.

The runner starts this file as a child process and owns its life cycle.
Exit paths: SIGTERM/SIGINT, or stdin close when stdin is a pipe (the runner
holds the pipe open; `/dev/null` and a terminal do not trigger this exit).

Standard library only (Python 3.10+).
"""

from __future__ import annotations

import argparse
import collections
import json
import os
import signal
import socket
import stat
import sys
import threading
import time
from pathlib import Path

GAME_FIRST_BYTE = 14
ONDEMAND_FIRST_BYTE = 15
DEFAULT_QUEUE_MAX_BYTES = 4 << 20  # 4 MiB per direction queue
DEFAULT_CHUNK_BYTES = 64 << 10  # 64 KiB per read
SAMPLE_CAP = 16384  # stored queue-delay samples per direction


def parse_endpoint(text: str) -> tuple[str, int]:
    """Parse `host:port`. Port 0 asks the OS for a free port."""
    host, sep, port_text = text.rpartition(":")
    if not sep or not host:
        raise ValueError(f"endpoint must be host:port, got {text!r}")
    try:
        port = int(port_text)
    except ValueError:
        raise ValueError(f"endpoint port is not a whole number: {text!r}") from None
    if not 0 <= port <= 65535:
        raise ValueError(f"endpoint port out of range: {text!r}")
    return host, port


def percentile(sorted_values: list[float], q: float) -> float:
    """Nearest-rank percentile of an already sorted, non-empty list."""
    index = max(0, min(len(sorted_values) - 1, int(q * len(sorted_values) + 0.5) - 1))
    if q <= 0:
        index = 0
    return sorted_values[index]


class Samples:
    """Queue-delay samples in ms with bounded storage.

    Storage halves (every second stored sample) each time it passes the
    cap, and only every `stride`-th new sample is stored after that. The
    total count and sum stay exact, so the mean stays exact. Percentiles
    come from the stored subset.
    """

    def __init__(self) -> None:
        self._stored: list[float] = []
        self._stride = 1
        self.count = 0
        self._sum = 0.0

    def add(self, value: float) -> None:
        self.count += 1
        self._sum += value
        if (self.count - 1) % self._stride:
            return
        self._stored.append(value)
        if len(self._stored) > SAMPLE_CAP:
            self._stored = self._stored[::2]
            self._stride *= 2

    def summary(self) -> dict | None:
        if not self._stored:
            return None
        ordered = sorted(self._stored)
        return {
            "p50": round(percentile(ordered, 0.50), 3),
            "p95": round(percentile(ordered, 0.95), 3),
            "mean": round(self._sum / self.count, 3),
            "samples": self.count,
        }


class DirectionQueue:
    """One FIFO queue of chunks, each with a release time.

    `push` returns False on overflow or after close; the caller then
    aborts the connection. `finish` marks end of input: the sender drains
    the queue, then `pop_wait` returns None. `abort` discards the rest.
    """

    def __init__(self, max_bytes: int) -> None:
        self._max_bytes = max_bytes
        self._cond = threading.Condition()
        self._items: collections.deque[tuple[float, float, bytes]] = collections.deque()
        self._bytes = 0
        self.high_water_bytes = 0
        self._eof = False
        self._aborted = False

    def push(self, arrival: float, release: float, data: bytes) -> bool:
        with self._cond:
            if self._aborted or self._eof:
                return False
            if self._bytes + len(data) > self._max_bytes:
                return False
            self._items.append((release, arrival, data))
            self._bytes += len(data)
            self.high_water_bytes = max(self.high_water_bytes, self._bytes)
            self._cond.notify()
            return True

    def finish(self) -> None:
        with self._cond:
            self._eof = True
            self._cond.notify_all()

    def abort(self) -> None:
        with self._cond:
            self._aborted = True
            self._items.clear()
            self._bytes = 0
            self._cond.notify_all()

    def pop_wait(self) -> tuple[float, bytes] | None:
        """Next (arrival, data) once its release time passes; None at end."""
        with self._cond:
            while True:
                if self._aborted:
                    return None
                if self._items:
                    release, arrival, data = self._items[0]
                    wait = release - time.monotonic()
                    if wait <= 0:
                        self._items.popleft()
                        self._bytes -= len(data)
                        return arrival, data
                    self._cond.wait(timeout=wait)
                elif self._eof:
                    return None
                else:
                    self._cond.wait(timeout=0.5)


def _nodelay(sock: socket.socket) -> None:
    try:
        sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    except OSError:
        pass


def _close(sock: socket.socket | None) -> None:
    if sock is None:
        return
    try:
        sock.shutdown(socket.SHUT_RDWR)
    except OSError:
        pass
    try:
        sock.close()
    except OSError:
        pass


class _DirectionStats:
    def __init__(self) -> None:
        self.bytes = 0
        self.chunks = 0
        self.samples = Samples()


class ProxyServer:
    """A listening delay proxy. One instance serves many connections."""

    def __init__(self, listen_host: str, listen_port: int, upstream_host: str, upstream_port: int,
                 delay_ms: float, stats_path: Path | None,
                 queue_max_bytes: int = DEFAULT_QUEUE_MAX_BYTES,
                 chunk_bytes: int = DEFAULT_CHUNK_BYTES) -> None:
        self.listen_host = listen_host
        self.listen_port = listen_port
        self.upstream_host = upstream_host
        self.upstream_port = upstream_port
        self.delay_ms = delay_ms
        self.delay_s = delay_ms / 1000.0
        self.stats_path = stats_path
        self.queue_max_bytes = queue_max_bytes
        self.chunk_bytes = chunk_bytes
        self.port: int | None = None
        self._listener: socket.socket | None = None
        self._stats_file = None
        self._stats_lock = threading.Lock()
        self._stopping = threading.Event()
        self._accept_thread: threading.Thread | None = None
        self._conn_id_lock = threading.Lock()
        self._next_id = 0
        self._active_lock = threading.Lock()
        self._active_sockets: set[socket.socket] = set()
        self._threads: list[threading.Thread] = []

    # ------------------------------------------------------------ lifecycle

    def start(self) -> int:
        """Bind, open the stats file, and start accepting. Return the port."""
        if self.stats_path is not None:
            # Open now so an unwritable stats file fails before listen().
            self._stats_file = self.stats_path.open("a", encoding="utf-8")
        listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            listener.bind((self.listen_host, self.listen_port))
            listener.listen(64)
        except OSError:
            listener.close()
            if self._stats_file is not None:
                self._stats_file.close()
                self._stats_file = None
            raise
        listener.settimeout(0.25)
        self._listener = listener
        self.port = listener.getsockname()[1]
        self._accept_thread = threading.Thread(target=self._accept_loop, daemon=True,
                                               name="latency-proxy-accept")
        self._accept_thread.start()
        return self.port

    def stop(self) -> None:
        self._stopping.set()
        if self._listener is not None:
            _close(self._listener)
            self._listener = None
        if self._accept_thread is not None:
            self._accept_thread.join(timeout=5)
            self._accept_thread = None
        with self._active_lock:
            sockets = list(self._active_sockets)
        for sock in sockets:
            _close(sock)
        for thread in self._threads:
            thread.join(timeout=5)
        if self._stats_file is not None:
            self._stats_file.close()
            self._stats_file = None

    # ------------------------------------------------------------ accepting

    def _accept_loop(self) -> None:
        assert self._listener is not None
        listener = self._listener
        while not self._stopping.is_set():
            try:
                client, addr = listener.accept()
            except socket.timeout:
                continue
            except OSError:
                return  # listener closed by stop()
            with self._conn_id_lock:
                self._next_id += 1
                conn_id = self._next_id
            thread = threading.Thread(target=self._handle, args=(client, addr, conn_id),
                                      daemon=True, name=f"latency-proxy-{conn_id}")
            self._threads.append(thread)
            thread.start()

    def _register(self, sock: socket.socket) -> None:
        with self._active_lock:
            self._active_sockets.add(sock)

    def _unregister(self, sock: socket.socket) -> None:
        with self._active_lock:
            self._active_sockets.discard(sock)

    # ------------------------------------------------------------ one connection

    def _handle(self, client: socket.socket, addr, conn_id: int) -> None:
        opened_epoch = time.time()
        record: dict = {
            "connection": conn_id,
            "client": f"{addr[0]}:{addr[1]}",
            "upstream": f"{self.upstream_host}:{self.upstream_port}",
            "first_byte": None,
            "path": "none",
            "delay_ms": self.delay_ms,
            "opened_at_epoch": round(opened_epoch, 3),
            "bytes_client_to_server": 0,
            "bytes_server_to_client": 0,
            "chunks_client_to_server": 0,
            "chunks_server_to_client": 0,
            "queue_delay_ms_client_to_server": None,
            "queue_delay_ms_server_to_client": None,
            "queue_high_water_bytes_client_to_server": 0,
            "queue_high_water_bytes_server_to_client": 0,
            "login_first_response_ms": None,
            "error": None,
            "close_reason": "client_closed_before_data",
        }
        _nodelay(client)
        self._register(client)
        upstream: socket.socket | None = None
        try:
            try:
                first = client.recv(self.chunk_bytes)
            except OSError as error:
                if not self._stopping.is_set():
                    record["error"] = f"client_read_failed: {error}"
                    record["close_reason"] = "client_read_failed"
                return
            first_client_at = time.monotonic()
            if not first:
                return  # probe-style connect-and-close; defaults above stand
            first_byte = first[0]
            record["first_byte"] = first_byte
            record["path"] = "direct" if first_byte == ONDEMAND_FIRST_BYTE else "delayed"
            delay_s = 0.0 if record["path"] == "direct" else self.delay_s
            try:
                upstream = socket.create_connection(
                    (self.upstream_host, self.upstream_port), timeout=10)
            except OSError as error:
                record["error"] = f"upstream_unavailable: {error}"
                record["close_reason"] = "upstream_unavailable"
                return
            upstream.settimeout(None)
            _nodelay(upstream)
            self._register(upstream)

            c2s_queue = DirectionQueue(self.queue_max_bytes)
            s2c_queue = DirectionQueue(self.queue_max_bytes)
            c2s_stats, s2c_stats = _DirectionStats(), _DirectionStats()
            state: dict = {"error": None, "close_reason": "eof",
                           "login_first_response_ms": None}
            state_lock = threading.Lock()

            def fail(reason: str) -> None:
                with state_lock:
                    if state["error"] is None:
                        state["error"] = reason
                        state["close_reason"] = reason.split(":", 1)[0]
                c2s_queue.abort()
                s2c_queue.abort()
                _close(client)
                _close(upstream)

            def reader(src: socket.socket, queue: DirectionQueue, stats: _DirectionStats,
                       first_chunk: bytes | None, first_arrival: float) -> None:
                pending, pending_arrival = first_chunk, first_arrival
                try:
                    while True:
                        if pending is not None:
                            data, arrival = pending, pending_arrival
                            pending = None
                        else:
                            try:
                                data = src.recv(self.chunk_bytes)
                            except OSError as error:
                                if not self._stopping.is_set():
                                    fail(f"read_failed: {error}")
                                else:
                                    queue.finish()
                                return
                            arrival = time.monotonic()
                            if not data:
                                queue.finish()
                                return
                        stats.bytes += len(data)
                        stats.chunks += 1
                        if not queue.push(arrival, arrival + delay_s, data):
                            fail(f"queue_overflow: queue over {self.queue_max_bytes} bytes")
                            return
                except Exception as error:  # keep the process alive; record it
                    fail(f"reader_failed: {error!r}")

            def sender(dst: socket.socket, queue: DirectionQueue, stats: _DirectionStats,
                       is_server_to_client: bool) -> None:
                try:
                    while True:
                        item = queue.pop_wait()
                        if item is None:
                            try:
                                dst.shutdown(socket.SHUT_WR)
                            except OSError:
                                pass
                            return
                        arrival, data = item
                        send_at = time.monotonic()
                        try:
                            dst.sendall(data)
                        except OSError as error:
                            if not self._stopping.is_set():
                                fail(f"write_failed: {error}")
                            return
                        stats.samples.add((send_at - arrival) * 1000.0)
                        if (is_server_to_client and first_byte == GAME_FIRST_BYTE
                                and state["login_first_response_ms"] is None):
                            # First client byte arrival -> first server byte
                            # handed to the client socket. Both queue delays
                            # are inside this span.
                            state["login_first_response_ms"] = round(
                                (send_at - first_client_at) * 1000.0, 3)
                except Exception as error:
                    fail(f"sender_failed: {error!r}")

            threads = [
                threading.Thread(target=reader, args=(client, c2s_queue, c2s_stats,
                                                      first, first_client_at), daemon=True),
                threading.Thread(target=sender, args=(upstream, c2s_queue, c2s_stats, False),
                                 daemon=True),
                threading.Thread(target=reader, args=(upstream, s2c_queue, s2c_stats, None, 0.0),
                                 daemon=True),
                threading.Thread(target=sender, args=(client, s2c_queue, s2c_stats, True),
                                 daemon=True),
            ]
            for thread in threads:
                thread.start()
            for thread in threads:
                thread.join()
            record.update({
                "bytes_client_to_server": c2s_stats.bytes,
                "bytes_server_to_client": s2c_stats.bytes,
                "chunks_client_to_server": c2s_stats.chunks,
                "chunks_server_to_client": s2c_stats.chunks,
                "queue_delay_ms_client_to_server": c2s_stats.samples.summary(),
                "queue_delay_ms_server_to_client": s2c_stats.samples.summary(),
                "queue_high_water_bytes_client_to_server": c2s_queue.high_water_bytes,
                "queue_high_water_bytes_server_to_client": s2c_queue.high_water_bytes,
                "login_first_response_ms": state["login_first_response_ms"],
                "error": state["error"],
                "close_reason": state["close_reason"],
            })
        finally:
            record["closed_at_epoch"] = round(time.time(), 3)
            _close(client)
            self._unregister(client)
            if upstream is not None:
                _close(upstream)
                self._unregister(upstream)
            self._write_record(record)

    def _write_record(self, record: dict) -> None:
        if self._stats_file is None:
            return
        line = json.dumps(record, separators=(",", ":")) + "\n"
        try:
            with self._stats_lock:
                self._stats_file.write(line)
                self._stats_file.flush()
        except (OSError, ValueError) as error:
            # The runner fails closed on missing stats. Keep serving.
            print(f"latency_proxy: stats write failed: {error}", file=sys.stderr, flush=True)


def _stdin_is_pipe() -> bool:
    try:
        return stat.S_ISFIFO(os.fstat(0).st_mode)
    except (OSError, ValueError):
        return False


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--listen", required=True, help="listen endpoint host:port (port 0 = free port)")
    parser.add_argument("--upstream", required=True, help="engine game endpoint host:port")
    parser.add_argument("--delay-ms", required=True, type=float,
                        help="one-way delay in ms for delayed (non-byte-15) connections")
    parser.add_argument("--stats", required=True, type=Path,
                        help="JSON Lines stats file, one record per connection close")
    parser.add_argument("--ready-file", type=Path,
                        help="write the bound port to this file after listen() succeeds")
    parser.add_argument("--queue-max-bytes", type=int, default=DEFAULT_QUEUE_MAX_BYTES,
                        help=f"per-direction queue bound (default {DEFAULT_QUEUE_MAX_BYTES})")
    parser.add_argument("--chunk-bytes", type=int, default=DEFAULT_CHUNK_BYTES,
                        help=f"max bytes per read (default {DEFAULT_CHUNK_BYTES})")
    args = parser.parse_args(argv)
    try:
        listen_host, listen_port = parse_endpoint(args.listen)
        upstream_host, upstream_port = parse_endpoint(args.upstream)
    except ValueError as error:
        parser.error(str(error))
    if args.delay_ms < 0:
        parser.error("--delay-ms must be >= 0")
    if args.queue_max_bytes < 1024:
        parser.error("--queue-max-bytes must be >= 1024")
    if not 1 <= args.chunk_bytes <= (1 << 20):
        parser.error("--chunk-bytes must be between 1 and 1048576")
    if upstream_port == 0:
        parser.error("--upstream port must not be 0")

    server = ProxyServer(listen_host, listen_port, upstream_host, upstream_port,
                         args.delay_ms, args.stats, args.queue_max_bytes, args.chunk_bytes)
    try:
        port = server.start()
    except OSError as error:
        print(f"latency_proxy: startup failed: {error}", file=sys.stderr, flush=True)
        return 1
    if args.ready_file is not None:
        tmp = args.ready_file.with_suffix(args.ready_file.suffix + ".tmp")
        try:
            tmp.write_text(f"{port}\n", encoding="utf-8")
            tmp.replace(args.ready_file)
        except OSError as error:
            print(f"latency_proxy: ready file write failed: {error}", file=sys.stderr, flush=True)
            server.stop()
            return 1

    stopping = threading.Event()

    def on_signal(_signum, _frame) -> None:
        stopping.set()

    for name in ("SIGTERM", "SIGINT"):
        if hasattr(signal, name):
            try:
                signal.signal(getattr(signal, name), on_signal)
            except (ValueError, OSError):
                pass
    if _stdin_is_pipe():
        def watch_stdin() -> None:
            try:
                while sys.stdin.buffer.read(1):
                    pass
            except (OSError, ValueError):
                pass
            stopping.set()

        threading.Thread(target=watch_stdin, daemon=True, name="latency-proxy-stdin").start()
    stopping.wait()
    server.stop()
    return 0


if __name__ == "__main__":
    sys.exit(main())
