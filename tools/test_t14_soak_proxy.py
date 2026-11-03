#!/usr/bin/env python3
"""The live final Close barrier needs the proxy's monotonic receipt time."""

import socket
import threading
from types import SimpleNamespace
import unittest

import t14_soak_proxy as proxy


class CloseReceiptTests(unittest.TestCase):
    def test_connection_close_records_precise_monotonic_time(self):
        events = []
        runtime = SimpleNamespace(
            config=SimpleNamespace(faults=SimpleNamespace(seed=1027)),
            context_lock=threading.Lock(),
            contexts={},
            write_event=events.append,
        )
        connection = proxy.ConnectionState(runtime, "c1", ("127.0.0.1", 1234))
        runtime.contexts["c1"] = connection
        connection.handshake["upgrade_status"] = 101

        for stage in ("proxy_a", "tls_terminator", "proxy_b"):
            connection.complete_stage(stage, "closed")

        self.assertEqual(len(events), 1)
        receipt = events[0]
        self.assertEqual(receipt["type"], "connection_close")
        self.assertEqual(receipt["connection_id"], "c1")
        self.assertIs(type(receipt["closed_monotonic_ns"]), int)
        self.assertGreaterEqual(receipt["closed_monotonic_ns"], connection.started_monotonic_ns)
        self.assertNotIn("c1", runtime.contexts)

    def test_websocket_close_records_unmasked_payload_length(self):
        for direction, masked, payload in (
            ("client_to_engine", True, b""),
            ("engine_to_client", False, b"\x03\xe8"),
        ):
            with self.subTest(direction=direction, payload=payload):
                events = []
                runtime = SimpleNamespace(
                    config=SimpleNamespace(faults=SimpleNamespace(seed=1027)),
                    context_lock=threading.Lock(),
                    contexts={},
                    stop=threading.Event(),
                    phase_started=lambda: False,
                    write_event=events.append,
                )
                connection = proxy.ConnectionState(runtime, "c1", ("127.0.0.1", 1234))
                source, writer = socket.socketpair()
                destination, receiver = socket.socketpair()
                try:
                    mask_key = b"\x01\x02\x03\x04"
                    wire_payload = (
                        bytes(value ^ mask_key[index & 3] for index, value in enumerate(payload))
                        if masked else payload
                    )
                    frame = (
                        bytes((0x88, len(payload) | (0x80 if masked else 0)))
                        + (mask_key if masked else b"")
                        + wire_payload
                    )
                    writer.sendall(frame)
                    writer.shutdown(socket.SHUT_WR)
                    proxy._relay_ws_direction(
                        runtime, connection, source, destination, direction, masked
                    )
                    receipt = next(
                        event for event in events if event["type"] == "websocket_close_frame"
                    )
                    self.assertEqual(receipt["payload_len"], len(payload))
                    self.assertEqual(receipt["status_code"], 1000 if payload else None)
                    self.assertEqual(receiver.recv(len(frame) + 1), frame)
                finally:
                    for sock in (source, writer, destination, receiver):
                        sock.close()


if __name__ == "__main__":
    unittest.main()
