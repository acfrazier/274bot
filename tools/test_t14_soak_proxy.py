#!/usr/bin/env python3
"""The live final Close barrier needs the proxy's monotonic receipt time."""

import socket
import threading
from types import SimpleNamespace
from unittest.mock import patch
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
                    self.assertFalse(
                        any(event["type"] == "websocket_frame_error" for event in events)
                    )
                finally:
                    for sock in (source, writer, destination, receiver):
                        sock.close()

    def test_masking_error_names_source_and_preserves_both_unparsed_tails(self):
        for direction, expected_masked, actual_masked in (
            ("client_to_engine", True, False),
            ("engine_to_client", False, True),
        ):
            with self.subTest(direction=direction):
                events = []
                runtime = SimpleNamespace(
                    config=SimpleNamespace(faults=SimpleNamespace(seed=1027)),
                    stop=threading.Event(),
                    write_event=events.append,
                )
                connection = proxy.ConnectionState(runtime, "c1", ("127.0.0.1", 1234))
                source, writer = socket.socketpair()
                destination, receiver = socket.socketpair()
                try:
                    payload = b"\x03\xe8"
                    mask_key = b"\x01\x02\x03\x04"
                    wire_payload = (
                        bytes(value ^ mask_key[index & 3] for index, value in enumerate(payload))
                        if actual_masked else payload
                    )
                    header = bytes((0x88, len(payload) | (0x80 if actual_masked else 0)))
                    remainder = (mask_key if actual_masked else b"") + wire_payload + b"\xde\xad"
                    writer.sendall(header + remainder)
                    receiver.sendall(b"\x89\x00")
                    proxy._relay_ws_direction(
                        runtime, connection, source, destination, direction, expected_masked
                    )
                    failure = next(event for event in events if event["type"] == "websocket_frame_error")
                    self.assertEqual(failure["direction"], direction)
                    self.assertEqual(
                        failure["source_endpoint"],
                        "client" if expected_masked else "engine",
                    )
                    self.assertEqual(failure["header_hex"], header.hex())
                    self.assertEqual(failure["opcode"], 8)
                    self.assertEqual(failure["masked"], actual_masked)
                    self.assertEqual(failure["decoded_length"], 2)
                    self.assertEqual(failure["stream_byte_offset"], 0)
                    self.assertEqual(failure["next_64_raw_bytes"]["hex"], remainder.hex())
                    tails = next(event for event in events if event["type"] == "websocket_unparsed_tail")
                    self.assertEqual(tails["failure_direction"], direction)
                    self.assertEqual(tails[direction]["hex"], remainder.hex())
                    other = "engine_to_client" if expected_masked else "client_to_engine"
                    self.assertEqual(tails[other]["hex"], "8900")
                    self.assertEqual(len(connection.failures), 1)
                finally:
                    for sock in (source, writer, destination, receiver):
                        sock.close()

    def test_masking_error_decodes_extended_length_without_consuming_tail(self):
        events = []
        runtime = SimpleNamespace(
            config=SimpleNamespace(faults=SimpleNamespace(seed=1027)),
            write_event=events.append,
        )
        connection = proxy.ConnectionState(runtime, "c1", ("127.0.0.1", 1234))
        source, writer = socket.socketpair()
        try:
            writer.sendall(
                b"\x82\x81\x01\x02\x03\x04\x79" + b"\x82\x7e\x00\x7e" + b"x" * 126
            )
            frame = proxy._read_frame(source, connection, "client_to_engine", True)
            self.assertEqual(frame["payload"], b"y")
            with self.assertRaisesRegex(proxy.FrameError, "client frames must be masked"):
                proxy._read_frame(source, connection, "client_to_engine", True)
            failure = events[0]
            self.assertEqual(failure["decoded_length"], 126)
            self.assertEqual(failure["stream_byte_offset"], 7)
            self.assertEqual(failure["next_64_raw_bytes"]["hex"], (b"\x00\x7e" + b"x" * 62).hex())
        finally:
            source.close()
            writer.close()


    def test_reverse_write_failure_drains_client_close_already_read(self):
        events = []
        runtime = SimpleNamespace(
            config=SimpleNamespace(faults=SimpleNamespace(seed=1027)),
            stop=threading.Event(),
            phase_started=lambda: True,
            write_event=events.append,
        )
        connection = proxy.ConnectionState(runtime, "c1", ("127.0.0.1", 1234))
        client, client_peer = socket.socketpair()
        terminator, terminator_peer = socket.socketpair()
        client_read = threading.Event()
        allow_client_write = threading.Event()
        close_record = b"\x17\x03\x03\x00\x06\x88\x80\x01\x02\x03\x04"

        def fault_send(_runtime, context, direction, destination, data):
            if direction == "terminator_to_client_ciphertext":
                self.assertTrue(client_read.wait(2))
                raise BrokenPipeError("reverse write failed")
            client_read.set()
            self.assertTrue(allow_client_write.wait(2))
            destination.sendall(data)
            context.inc("a", direction, "bytes_written", len(data))

        try:
            client_peer.sendall(close_record)
            client_peer.shutdown(socket.SHUT_WR)
            terminator_peer.sendall(b"server")
            with patch.object(proxy, "_tcp_fault_send", side_effect=fault_send):
                forward = threading.Thread(target=proxy._tcp_copy, args=(
                    runtime, connection, client, terminator, "proxy_a",
                    "client_to_terminator_ciphertext", True,
                ))
                reverse = threading.Thread(target=proxy._tcp_copy, args=(
                    runtime, connection, terminator, client, "proxy_a",
                    "terminator_to_client_ciphertext", True,
                ))
                forward.start()
                reverse.start()
                self.assertTrue(client_read.wait(2))
                reverse.join(2)
                self.assertFalse(reverse.is_alive())
                allow_client_write.set()
                forward.join(2)
                self.assertFalse(forward.is_alive())
            self.assertEqual(terminator_peer.recv(100), close_record)
            self.assertEqual(terminator_peer.recv(1), b"")
            self.assertEqual(
                connection.a["client_to_terminator_ciphertext"]["bytes_read"],
                connection.a["client_to_terminator_ciphertext"]["bytes_written"],
            )
            self.assertEqual(len(connection.failures), 1)
            self.assertTrue(any(event["type"] == "tcp_reverse_error_draining_client_tail"
                                and event["pending_client_bytes"] == len(close_record)
                                for event in events))
            self.assertFalse(any(event["type"] == "tcp_shutdown_complete" for event in events))
        finally:
            allow_client_write.set()
            for sock in (client, client_peer, terminator, terminator_peer):
                sock.close()

    def test_reverse_failure_without_pending_client_bytes_shuts_down_and_records_failure(self):
        events = []
        runtime = SimpleNamespace(
            config=SimpleNamespace(faults=SimpleNamespace(seed=1027)),
            stop=threading.Event(),
            phase_started=lambda: True,
            write_event=events.append,
        )
        connection = proxy.ConnectionState(runtime, "c1", ("127.0.0.1", 1234))
        client, client_peer = socket.socketpair()
        terminator, terminator_peer = socket.socketpair()
        try:
            terminator_peer.sendall(b"server")
            with patch.object(proxy, "_tcp_fault_send", side_effect=BrokenPipeError("reverse write failed")):
                proxy._tcp_copy(runtime, connection, terminator, client, "proxy_a",
                                "terminator_to_client_ciphertext", True)
            self.assertEqual(len(connection.failures), 1)
            self.assertEqual(client_peer.recv(1), b"")
            self.assertEqual(terminator_peer.recv(1), b"")
            self.assertTrue(any(event["type"] == "tcp_copy_error"
                                and event["direction"] == "terminator_to_client_ciphertext"
                                and event["pending_client_bytes"] == 0 for event in events))
            self.assertTrue(any(event["type"] == "tcp_shutdown_complete" for event in events))
            self.assertFalse(any(event["type"] == "tcp_reverse_error_draining_client_tail"
                                 for event in events))
        finally:
            for sock in (client, client_peer, terminator, terminator_peer):
                sock.close()

if __name__ == "__main__":
    unittest.main()
