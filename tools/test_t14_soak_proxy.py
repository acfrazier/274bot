#!/usr/bin/env python3
"""The live final Close barrier needs the proxy's monotonic receipt time."""

from pathlib import Path
import socket
import ssl
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

    def test_forward_write_failure_drains_server_alert_already_read(self):
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
        server_read = threading.Event()
        allow_server_write = threading.Event()
        alert_record = b"\x17\x03\x03\x00\x13" + b"x" * 19

        def fault_send(_runtime, context, direction, destination, data):
            if direction == "client_to_terminator_ciphertext":
                self.assertTrue(server_read.wait(2))
                raise BrokenPipeError("forward write failed")
            server_read.set()
            self.assertTrue(allow_server_write.wait(2))
            destination.sendall(data)
            context.inc("a", direction, "bytes_written", len(data))

        try:
            terminator_peer.sendall(alert_record)
            terminator_peer.shutdown(socket.SHUT_WR)
            client_peer.sendall(b"client")
            with patch.object(proxy, "_tcp_fault_send", side_effect=fault_send):
                forward = threading.Thread(target=proxy._tcp_copy, args=(
                    runtime, connection, client, terminator, "proxy_a",
                    "client_to_terminator_ciphertext", True,
                ))
                reverse = threading.Thread(target=proxy._tcp_copy, args=(
                    runtime, connection, terminator, client, "proxy_a",
                    "terminator_to_client_ciphertext", True,
                ))
                reverse.start()
                forward.start()
                self.assertTrue(server_read.wait(2))
                forward.join(2)
                self.assertFalse(forward.is_alive())
                allow_server_write.set()
                reverse.join(2)
                self.assertFalse(reverse.is_alive())
            self.assertEqual(client_peer.recv(100), alert_record)
            self.assertEqual(client_peer.recv(1), b"")
            self.assertEqual(
                connection.a["terminator_to_client_ciphertext"]["bytes_read"],
                connection.a["terminator_to_client_ciphertext"]["bytes_written"],
            )
            self.assertEqual(len(connection.failures), 1)
            self.assertTrue(any(event["type"] == "tcp_forward_error_draining_server_tail"
                                and event["pending_server_bytes"] == len(alert_record)
                                for event in events))
            self.assertFalse(any(event["type"] == "tcp_shutdown_complete" for event in events))
        finally:
            allow_server_write.set()
            for sock in (client, client_peer, terminator, terminator_peer):
                sock.close()

    def test_forward_failure_without_pending_server_bytes_shuts_down(self):
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
            client_peer.sendall(b"client")
            with patch.object(proxy, "_tcp_fault_send",
                              side_effect=BrokenPipeError("forward write failed")):
                proxy._tcp_copy(runtime, connection, client, terminator, "proxy_a",
                                "client_to_terminator_ciphertext", True)
            self.assertEqual(len(connection.failures), 1)
            self.assertEqual(client_peer.recv(1), b"")
            self.assertEqual(terminator_peer.recv(1), b"")
            self.assertTrue(any(event["type"] == "tcp_copy_error"
                                and event["direction"] == "client_to_terminator_ciphertext"
                                and event["pending_server_bytes"] == 0 for event in events))
            self.assertTrue(any(event["type"] == "tcp_shutdown_complete" for event in events))
        finally:
            for sock in (client, client_peer, terminator, terminator_peer):
                sock.close()

    def test_tls_close_notify_after_ws_close_never_enters_plaintext_proxy(self):
        events = []
        runtime = SimpleNamespace(
            config=SimpleNamespace(faults=SimpleNamespace(seed=1027)),
            stop=threading.Event(),
            phase_started=lambda: False,
            write_event=events.append,
            spawn=lambda target, name, *args: threading.Thread(
                target=target, args=args, name=name),
        )
        def spawn(target, name, *args):
            worker = threading.Thread(target=target, args=args, name=name)
            worker.start()
            return worker
        runtime.spawn = spawn
        runtime.join_pair = lambda workers: [worker.join(3) for worker in workers]
        connection = proxy.ConnectionState(runtime, "c1", ("127.0.0.1", 1234))
        connection.set_handshake("upgrade_status", 101)
        fixture = (Path(__file__).resolve().parents[1] / "vendor" / "fr-client-rust"
                   / "crates" / "client" / "tests" / "fixtures" / "tls")
        server_context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        server_context.load_cert_chain(str(fixture / "localhost.pem"),
                                       str(fixture / "localhost.key"))
        client_context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        client_context.check_hostname = False
        client_context.verify_mode = ssl.CERT_NONE
        server_raw, client_raw = socket.socketpair()
        server_holder = []
        server_handshake = spawn(
            lambda: server_holder.append(server_context.wrap_socket(server_raw, server_side=True)),
            "server-handshake",
        )
        client = client_context.wrap_socket(client_raw, server_hostname="localhost")
        server_handshake.join(3)
        self.assertFalse(server_handshake.is_alive())
        secure = server_holder[0]
        plain, proxy_b = socket.socketpair()
        engine, engine_peer = socket.socketpair()
        relay = spawn(lambda: proxy._relay_pair(
            runtime, connection, secure, plain, "tls_terminator",
            "client_to_proxy_b_plaintext", "proxy_b_to_client_plaintext",
        ), "tls-relay")
        websocket = spawn(lambda: proxy._relay_ws_direction(
            runtime, connection, proxy_b, engine, "client_to_engine", True,
        ), "proxy-b-frames")
        masked_close = b"\x88\x80\x01\x02\x03\x04"
        try:
            # Proxy B finishes writing the server Close first. Its write EOF
            # must not remove the TLS decoder while the client is still live.
            proxy_b.sendall(b"\x88\x00")
            proxy_b.shutdown(socket.SHUT_WR)
            self.assertEqual(client.recv(2), b"\x88\x00")
            # HTTP-only TLS shutdown must never run for an upgraded socket.
            client.settimeout(0.2)
            with self.assertRaises(socket.timeout):
                client.recv(1)
            client.settimeout(2)
            client.sendall(masked_close)
            client.settimeout(2)
            client_shutdown = spawn(lambda: self._unwrap_client(client), "client-close-notify")
            engine_peer.settimeout(2)
            self.assertEqual(engine_peer.recv(6), masked_close)
            self.assertEqual(engine_peer.recv(1), b"")
            relay.join(3)
            websocket.join(3)
            self.assertFalse(relay.is_alive())
            self.assertFalse(websocket.is_alive())
            self.assertFalse(any(event["type"] == "websocket_frame_error" for event in events))
            self.assertEqual(connection.tls["client_to_proxy_b_plaintext"]["bytes_read"],
                             len(masked_close))
            client_shutdown.join(3)
        finally:
            for sock in (secure, client, plain, proxy_b, engine, engine_peer):
                sock.close()

    @staticmethod
    def _unwrap_client(client):
        try:
            client.unwrap()
        except (OSError, ssl.SSLError):
            # The proxy closes TCP after the relay; it does not send a server
            # close_notify. The client alert still reached the TLS decoder.
            pass

    def test_https_response_half_closes_after_complete_body(self):
        self._check_https_response_half_close(partial_body=False)

    def test_https_partial_body_waits_for_upstream_eof(self):
        self._check_https_response_half_close(partial_body=True)

    def _check_https_response_half_close(self, *, partial_body):
        events = []
        def spawn(target, name, *args):
            worker = threading.Thread(target=target, args=args, name=name)
            worker.start()
            return worker

        runtime = SimpleNamespace(
            config=SimpleNamespace(faults=SimpleNamespace(seed=1027)),
            stop=threading.Event(),
            phase_started=lambda: False,
            write_event=events.append,
            spawn=spawn,
            join_pair=lambda workers: [worker.join(3) for worker in workers],
        )
        connection = proxy.ConnectionState(runtime, "http", ("127.0.0.1", 1234))
        fixture = (Path(__file__).resolve().parents[1] / "vendor" / "fr-client-rust"
                   / "crates" / "client" / "tests" / "fixtures" / "tls")
        server_context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        server_context.load_cert_chain(str(fixture / "localhost.pem"),
                                       str(fixture / "localhost.key"))
        client_context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        client_context.check_hostname = False
        client_context.verify_mode = ssl.CERT_NONE
        server_raw, client_raw = socket.socketpair()
        server_holder = []
        handshake = spawn(
            lambda: server_holder.append(server_context.wrap_socket(server_raw, server_side=True)),
            "http-server-handshake",
        )
        client = client_context.wrap_socket(
            client_raw, server_hostname="localhost", suppress_ragged_eofs=False)
        handshake.join(3)
        self.assertFalse(handshake.is_alive())
        secure = server_holder[0]
        plain, proxy_b = socket.socketpair()
        relay = spawn(lambda: proxy._relay_pair(
            runtime, connection, secure, plain, "tls_terminator",
            "client_to_proxy_b_plaintext", "proxy_b_to_client_plaintext",
        ), "http-tls-relay")
        request = b"GET /crc HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
        response = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbody"
        try:
            client.sendall(request)
            proxy_b.settimeout(2)
            received_request = bytearray()
            while len(received_request) < len(request):
                chunk = proxy_b.recv(len(request) - len(received_request))
                self.assertTrue(chunk)
                received_request.extend(chunk)
            self.assertEqual(received_request, request)
            connection.set_handshake("upgrade_status", 200)
            client.settimeout(1)
            received = bytearray()
            if partial_body:
                proxy_b.sendall(response[:-2])
                while len(received) < len(response) - 2:
                    received.extend(client.recv(len(response) - 2 - len(received)))
                with self.assertRaises(socket.timeout):
                    client.recv(1)
                proxy_b.sendall(response[-2:])
            else:
                proxy_b.sendall(response)
            proxy_b.shutdown(socket.SHUT_WR)
            while chunk := client.recv(4096):
                received.extend(chunk)
            self.assertEqual(received, response)
            client.close()
            relay.join(3)
            self.assertFalse(relay.is_alive())
            self.assertFalse(any(event["type"] == "tcp_copy_error" for event in events))
        finally:
            for sock in (secure, client, plain, proxy_b):
                sock.close()

    def test_plaintext_tls_record_is_still_a_websocket_frame_error(self):
        events = []
        runtime = SimpleNamespace(
            config=SimpleNamespace(faults=SimpleNamespace(seed=1027)),
            stop=threading.Event(), write_event=events.append,
        )
        connection = proxy.ConnectionState(runtime, "c1", ("127.0.0.1", 1234))
        source, writer = socket.socketpair()
        destination, receiver = socket.socketpair()
        try:
            writer.sendall(b"\x17\x03\x03\x00\x13" + b"x" * 19)
            writer.shutdown(socket.SHUT_WR)
            proxy._relay_ws_direction(
                runtime, connection, source, destination, "client_to_engine", True,
            )
            self.assertEqual(len(connection.failures), 1)
            self.assertTrue(any(event["type"] == "websocket_frame_error" for event in events))
        finally:
            for sock in (source, writer, destination, receiver):
                sock.close()

if __name__ == "__main__":
    unittest.main()
