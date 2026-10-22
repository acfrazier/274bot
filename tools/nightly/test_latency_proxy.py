#!/usr/bin/env python3
"""Tests for `latency_proxy.py`.

Each test runs on loopback with the standard library only. The band test
prints one `BAND ...` line per band. Reports quote those measured lines.

Run on Linux, macOS or Windows:

  python3 -B tools/nightly/test_latency_proxy.py
"""

import contextlib
import json
import os
import socket
import statistics
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import latency_proxy as lp

GAME = bytes([lp.GAME_FIRST_BYTE])
ONDEMAND = bytes([lp.ONDEMAND_FIRST_BYTE])


class EchoServer:
    """Threaded TCP echo server on a free loopback port."""

    def __init__(self) -> None:
        self._listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self._listener.bind(("127.0.0.1", 0))
        self._listener.listen(64)
        self._listener.settimeout(0.25)
        self.port = self._listener.getsockname()[1]
        self._stopping = threading.Event()
        self._thread = threading.Thread(target=self._accept, daemon=True)

    def start(self) -> "EchoServer":
        self._thread.start()
        return self

    def stop(self) -> None:
        self._stopping.set()
        self._listener.close()
        self._thread.join(timeout=5)

    def _accept(self) -> None:
        while not self._stopping.is_set():
            try:
                conn, _ = self._listener.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            threading.Thread(target=self._echo, args=(conn,), daemon=True).start()

    @staticmethod
    def _echo(conn: socket.socket) -> None:
        try:
            while True:
                data = conn.recv(65536)
                if not data:
                    return
                conn.sendall(data)
        except OSError:
            return
        finally:
            conn.close()


class SinkServer:
    """Accepts connections and reads nothing. It only holds sockets open."""

    def __init__(self) -> None:
        self._listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self._listener.bind(("127.0.0.1", 0))
        self._listener.listen(64)
        self._listener.settimeout(0.25)
        self.port = self._listener.getsockname()[1]
        self._stopping = threading.Event()
        self._held: list[socket.socket] = []
        self._thread = threading.Thread(target=self._accept, daemon=True)

    def start(self) -> "SinkServer":
        self._thread.start()
        return self

    def stop(self) -> None:
        self._stopping.set()
        self._listener.close()
        for conn in self._held:
            conn.close()
        self._thread.join(timeout=5)

    def _accept(self) -> None:
        while not self._stopping.is_set():
            try:
                conn, _ = self._listener.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            self._held.append(conn)


@contextlib.contextmanager
def running_proxy(upstream_port: int, delay_ms: float, stats_path: Path, **kwargs):
    server = lp.ProxyServer("127.0.0.1", 0, "127.0.0.1", upstream_port, delay_ms,
                            stats_path, **kwargs)
    server.start()
    try:
        yield server
    finally:
        server.stop()


def recv_exact(sock: socket.socket, count: int) -> bytes:
    chunks = []
    remaining = count
    while remaining > 0:
        chunk = sock.recv(remaining)
        if not chunk:
            raise ConnectionError(f"closed with {remaining} bytes still expected")
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


def ping_times(proxy_port: int, first_byte: bytes, count: int, payload: bytes = b"ping") -> list[float]:
    """Round-trip times in ms for `count` pings on one connection."""
    times = []
    with socket.create_connection(("127.0.0.1", proxy_port), timeout=10) as sock:
        message = first_byte + payload
        for _ in range(count):
            started = time.monotonic()
            sock.sendall(message)
            echoed = recv_exact(sock, len(message))
            times.append((time.monotonic() - started) * 1000.0)
            if echoed != message:
                raise AssertionError("echo bytes differ from sent bytes")
            message = payload  # only the first send carries the first byte
    return times


def wait_for_records(stats_path: Path, minimum: int, timeout: float = 10.0) -> list[dict]:
    deadline = time.monotonic() + timeout
    rows: list[dict] = []
    while time.monotonic() < deadline:
        if stats_path.is_file():
            rows = [json.loads(line) for line in
                    stats_path.read_text(encoding="utf-8").splitlines() if line.strip()]
            if len(rows) >= minimum:
                return rows
        time.sleep(0.05)
    raise AssertionError(f"stats file holds {len(rows)} records, expected at least {minimum}")


def free_port() -> int:
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.bind(("127.0.0.1", 0))
    port = sock.getsockname()[1]
    sock.close()
    return port


class BandDelayTests(unittest.TestCase):
    def test_each_band_round_trip_within_tolerance(self):
        with tempfile.TemporaryDirectory() as tmp:
            stats = Path(tmp) / "stats.jsonl"
            echo = EchoServer().start()
            try:
                for rtt, one_way in ((50, 25.0), (100, 50.0), (150, 75.0)):
                    with running_proxy(echo.port, one_way, stats) as proxy:
                        times = ping_times(proxy.port, GAME, 5)
                    median = statistics.median(times)
                    print(f"BAND rtt{rtt}: median {median:.1f} ms, min {min(times):.1f} ms, "
                          f"max {max(times):.1f} ms, one-way {one_way:.0f} ms")
                    self.assertLessEqual(abs(median - rtt), 20.0,
                                         f"rtt{rtt} median {median:.1f} ms outside ±20 ms")
                    self.assertGreaterEqual(median, rtt - 5.0,
                                            f"rtt{rtt} median {median:.1f} ms arrived early")
            finally:
                echo.stop()

    def test_byte15_is_undelayed_while_byte14_and_other_bytes_are_delayed(self):
        with tempfile.TemporaryDirectory() as tmp:
            stats = Path(tmp) / "stats.jsonl"
            echo = EchoServer().start()
            try:
                with running_proxy(echo.port, 75.0, stats) as proxy:
                    direct = ping_times(proxy.port, ONDEMAND, 5)
                    game = ping_times(proxy.port, GAME, 3)
                    other = ping_times(proxy.port, b"\x63", 3)
                print(f"BYTE15 direct: min {min(direct):.2f} ms, median "
                      f"{statistics.median(direct):.2f} ms at one-way 75 ms setting")
                self.assertLess(min(direct), 10.0, "byte 15 must not wait in a delay queue")
                self.assertGreaterEqual(statistics.median(game), 130.0)
                self.assertGreaterEqual(statistics.median(other), 130.0)
                rows = wait_for_records(stats, 3)
                by_byte = {row["first_byte"]: row for row in rows}
                self.assertEqual(by_byte[15]["path"], "direct")
                self.assertEqual(by_byte[14]["path"], "delayed")
                self.assertEqual(by_byte[0x63]["path"], "delayed")
            finally:
                echo.stop()


class OrderAndStatsTests(unittest.TestCase):
    def test_byte_order_is_exact_for_a_1mib_stream_at_top_delay(self):
        pattern = GAME + bytes(range(256)) * 4096  # 1 MiB plus the first byte
        with tempfile.TemporaryDirectory() as tmp:
            echo = EchoServer().start()
            try:
                with running_proxy(echo.port, 75.0, Path(tmp) / "stats.jsonl") as proxy:
                    with socket.create_connection(("127.0.0.1", proxy.port), timeout=30) as sock:
                        received = bytearray()

                        def collect() -> None:
                            while len(received) < len(pattern):
                                chunk = sock.recv(65536)
                                if not chunk:
                                    return
                                received.extend(chunk)

                        collector = threading.Thread(target=collect, daemon=True)
                        collector.start()
                        sock.sendall(pattern)
                        collector.join(timeout=30)
                    self.assertEqual(bytes(received), pattern)
            finally:
                echo.stop()

    def test_stats_record_holds_the_contract_fields(self):
        with tempfile.TemporaryDirectory() as tmp:
            stats = Path(tmp) / "stats.jsonl"
            echo = EchoServer().start()
            try:
                with running_proxy(echo.port, 50.0, stats) as proxy:
                    ping_times(proxy.port, GAME, 3)
                row = wait_for_records(stats, 1)[0]
                self.assertEqual(row["first_byte"], 14)
                self.assertEqual(row["path"], "delayed")
                self.assertEqual(row["delay_ms"], 50.0)
                self.assertIsNone(row["error"])
                self.assertGreaterEqual(row["bytes_client_to_server"], 13)
                self.assertEqual(row["bytes_client_to_server"], row["bytes_server_to_client"])
                self.assertGreaterEqual(row["chunks_client_to_server"], 1)
                c2s = row["queue_delay_ms_client_to_server"]
                s2c = row["queue_delay_ms_server_to_client"]
                for summary in (c2s, s2c):
                    self.assertLessEqual(abs(summary["p50"] - 50.0), 10.0)
                self.assertGreaterEqual(row["login_first_response_ms"], 90.0)
                self.assertLessEqual(row["login_first_response_ms"], 150.0)
                self.assertGreater(row["closed_at_epoch"], row["opened_at_epoch"])
            finally:
                echo.stop()

    def test_half_close_still_drains_the_echo(self):
        with tempfile.TemporaryDirectory() as tmp:
            echo = EchoServer().start()
            try:
                with running_proxy(echo.port, 25.0, Path(tmp) / "stats.jsonl") as proxy:
                    with socket.create_connection(("127.0.0.1", proxy.port), timeout=10) as sock:
                        sock.sendall(GAME + b"drain-me")
                        sock.shutdown(socket.SHUT_WR)
                        echoed = b""
                        while True:
                            chunk = sock.recv(4096)
                            if not chunk:
                                break
                            echoed += chunk
                    self.assertEqual(echoed, GAME + b"drain-me")
            finally:
                echo.stop()

    def test_eight_concurrent_connections(self):
        with tempfile.TemporaryDirectory() as tmp:
            echo = EchoServer().start()
            errors: list[BaseException] = []
            try:
                with running_proxy(echo.port, 25.0, Path(tmp) / "stats.jsonl") as proxy:
                    def work(index: int) -> None:
                        try:
                            ping_times(proxy.port, GAME if index % 2 == 0 else ONDEMAND, 3,
                                       payload=f"conn{index}".encode())
                        except BaseException as error:  # reported below
                            errors.append(error)

                    threads = [threading.Thread(target=work, args=(i,)) for i in range(8)]
                    for thread in threads:
                        thread.start()
                    for thread in threads:
                        thread.join(timeout=30)
                self.assertEqual(errors, [])
                rows = wait_for_records(Path(tmp) / "stats.jsonl", 8)
                self.assertTrue(all(row["error"] is None for row in rows))
            finally:
                echo.stop()


class OverflowAndUpstreamTests(unittest.TestCase):
    def test_queue_overflow_closes_the_connection_and_records_the_error(self):
        queue_max = 64 << 10
        with tempfile.TemporaryDirectory() as tmp:
            stats = Path(tmp) / "stats.jsonl"
            sink = SinkServer().start()
            try:
                with running_proxy(sink.port, 1000.0, stats,
                                   queue_max_bytes=queue_max) as proxy:
                    with socket.create_connection(("127.0.0.1", proxy.port), timeout=15) as sock:
                        closed = False
                        try:
                            sock.sendall(GAME + b"x" * (1 << 20))
                            closed = sock.recv(1) == b""
                        except OSError:
                            closed = True
                    self.assertTrue(closed, "an overflowing connection must close, not drop bytes")
                rows = wait_for_records(stats, 1)
                row = rows[0]
                self.assertIn("queue_overflow", row["error"] or "")
                self.assertLessEqual(row["queue_high_water_bytes_client_to_server"], queue_max)
            finally:
                sink.stop()

    def test_dead_upstream_is_reported_unavailable_and_closes_fast(self):
        with tempfile.TemporaryDirectory() as tmp:
            stats = Path(tmp) / "stats.jsonl"
            with running_proxy(free_port(), 25.0, stats) as proxy:
                started = time.monotonic()
                with socket.create_connection(("127.0.0.1", proxy.port), timeout=10) as sock:
                    sock.sendall(GAME + b"hello")
                    self.assertEqual(sock.recv(1), b"")
                self.assertLess(time.monotonic() - started, 5.0)
            row = wait_for_records(stats, 1)[0]
            self.assertTrue((row["error"] or "").startswith("upstream_unavailable"), row)
            self.assertEqual(row["close_reason"], "upstream_unavailable")


class SubprocessTests(unittest.TestCase):
    def start_proxy_process(self, root: Path, upstream_port: int):
        ready = root / "ready.txt"
        stats = root / "stats.jsonl"
        proc = subprocess.Popen(
            [sys.executable, str(HERE / "latency_proxy.py"),
             "--listen", "127.0.0.1:0", "--upstream", f"127.0.0.1:{upstream_port}",
             "--delay-ms", "25", "--stats", str(stats), "--ready-file", str(ready)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if ready.is_file():
                return proc, int(ready.read_text(encoding="utf-8").strip()), stats
            if proc.poll() is not None:
                raise AssertionError(f"proxy exited early: {proc.stderr.read() if proc.stderr else ''}")
            time.sleep(0.05)
        proc.kill()
        raise AssertionError("ready file did not appear within 10 s")

    def test_ready_file_then_clean_shutdown_on_stdin_close(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            echo = EchoServer().start()
            try:
                proc, port, stats = self.start_proxy_process(root, echo.port)
                try:
                    self.assertGreater(port, 0)
                    ping_times(port, GAME, 1)
                    self.assertIsNotNone(proc.stdin)
                    proc.stdin.close()
                    proc.wait(timeout=10)
                    out = proc.stdout.read() if proc.stdout else ""
                    self.assertEqual(proc.returncode, 0)
                    self.assertEqual(out, "", "the proxy prints nothing to stdout")
                    self.assertEqual(len(wait_for_records(stats, 1)), 1)
                finally:
                    if proc.poll() is None:
                        proc.kill()
            finally:
                echo.stop()

    @unittest.skipIf(os.name == "nt", "SIGTERM exit code is a POSIX contract")
    def test_clean_shutdown_on_sigterm(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            echo = EchoServer().start()
            try:
                proc, port, _stats = self.start_proxy_process(root, echo.port)
                try:
                    ping_times(port, GAME, 1)
                    proc.terminate()
                    proc.communicate(timeout=10)
                    self.assertEqual(proc.returncode, 0)
                finally:
                    if proc.poll() is None:
                        proc.kill()
            finally:
                echo.stop()

    def test_bad_arguments_exit_nonzero_without_a_ready_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ready = root / "ready.txt"
            result = subprocess.run(
                [sys.executable, str(HERE / "latency_proxy.py"),
                 "--listen", "127.0.0.1:notaport", "--upstream", "127.0.0.1:9",
                 "--delay-ms", "25", "--stats", str(root / "stats.jsonl"),
                 "--ready-file", str(ready)],
                capture_output=True, text=True, timeout=30)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(ready.exists())


class UnitTests(unittest.TestCase):
    def test_parse_endpoint(self):
        self.assertEqual(lp.parse_endpoint("127.0.0.1:0"), ("127.0.0.1", 0))
        self.assertEqual(lp.parse_endpoint("engine.example:44594"), ("engine.example", 44594))
        for bad in ("127.0.0.1", ":1234", "127.0.0.1:x", "127.0.0.1:65536", "127.0.0.1:-1"):
            with self.assertRaises(ValueError):
                lp.parse_endpoint(bad)

    def test_samples_storage_stays_bounded_and_mean_stays_exact(self):
        samples = lp.Samples()
        for value in range(100_000):
            samples.add(float(value % 100))
        self.assertLessEqual(len(samples._stored), lp.SAMPLE_CAP)
        summary = samples.summary()
        self.assertEqual(summary["samples"], 100_000)
        self.assertAlmostEqual(summary["mean"], 49.5, places=3)


if __name__ == "__main__":
    unittest.main(verbosity=2)
