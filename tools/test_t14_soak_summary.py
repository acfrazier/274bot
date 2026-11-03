#!/usr/bin/env python3
"""Guard the Engine A evaluator against false passes and false failures."""

import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import t14_soak_summary as summary


class SceneSamplesTests(unittest.TestCase):
    @staticmethod
    def sample(at, *, state=2, epoch=3, connected=True):
        return {"event": "sample", "elapsed_seconds": at // 1_000_000_000,
                "sample_monotonic_ns": at,
                "slots": [{"account": "slot", "connected": connected,
                           "stream_epoch": epoch, "ingame": state == 2,
                           "scene_state": state}]}

    def check(self, events):
        errors = []
        summary.check_scene_samples(Path("run"), events, ["slot"], {"slot": 0}, 10, errors)
        return errors

    def test_connected_map_load_recovers_at_next_sample(self):
        self.assertEqual(self.check([self.sample(0), self.sample(10_000_000_000, state=1),
                                     self.sample(20_000_000_000)]), [])

    def test_long_map_load_fails(self):
        errors = self.check([self.sample(0), self.sample(10_000_000_000, state=1),
                             self.sample(20_000_000_000, state=1),
                             self.sample(30_000_000_000, state=1), self.sample(40_000_000_000)])
        self.assertTrue(any("stayed in scene1 beyond 12s" in error for error in errors))

    def test_changed_epoch_and_disconnect_fail_during_loading(self):
        for changed in (self.sample(10_000_000_000, state=1, epoch=4),
                        self.sample(10_000_000_000, state=1, connected=False)):
            with self.subTest(changed=changed):
                self.assertTrue(self.check([self.sample(0), changed, self.sample(20_000_000_000)]))

    def test_unrecovered_and_nonloading_scene_fail(self):
        self.assertTrue(any("never returned" in error for error in
                            self.check([self.sample(0), self.sample(10_000_000_000, state=1)])))
        self.assertTrue(any("outside map loading" in error for error in
                            self.check([self.sample(0), self.sample(10_000_000_000, state=0)])))


class TeardownTests(unittest.TestCase):
    @staticmethod
    def event(failure, *, started=10, closed=20, tls_out=0):
        counters = {"bytes_read": 0, "bytes_written": 0}
        return {"connection_id": "c000060", "started_monotonic_ns": started,
                "closed_monotonic_ns": closed,
                "stage_status": {"proxy_a": "failed" if failure["stage"] == "proxy_a" else "closed",
                                 "proxy_b": "closed", "tls_terminator": "closed"},
                "websocket_handshake": {"upgrade_status": 101},
                "proxy_a_ciphertext": {"client_to_terminator_ciphertext": counters,
                                       "terminator_to_client_ciphertext": counters},
                "tls_terminator": {"handshake": {"completed": 1, "tls_version": "TLSv1.3"},
                                   "client_to_proxy_b_plaintext": counters,
                                   "proxy_b_to_client_plaintext": {"bytes_read": tls_out,
                                                                   "bytes_written": 0}},
                "proxy_b_websocket": {
                    direction: {"opcodes_read": {"8": 1}, "opcodes_written": {"8": 1}}
                    for direction in ("client_to_engine", "engine_to_client")},
                "binary_payload_hashes_match_at_proxy_b": {"client_to_engine": True,
                                                              "engine_to_client": True},
                "failures": [failure]}

    @staticmethod
    def close_frames():
        return {direction: [{"status_code": None, "reason": ""}]
                for direction in ("client_to_engine", "engine_to_client")}

    def check(self, event, phase_zero=30):
        errors = []
        summary.check_transport_teardown(Path("run"), event, self.close_frames(),
                                         phase_zero, errors)
        return errors

    def test_ssl_eof_accepts_changed_source_line_not_changed_class_or_message(self):
        original = {"stage": "tls_terminator",
                    "error": "SSLEOFError: EOF occurred in violation of protocol (_ssl.c:2557)"}
        self.assertEqual(self.check(self.event(original, tls_out=2)), [])
        for bad in ("SSLError: EOF occurred in violation of protocol (_ssl.c:2557)",
                    "SSLEOFError: handshake failed (_ssl.c:2557)",
                    "SSLEOFError: EOF occurred in violation of protocol (other.c:2557)"):
            with self.subTest(error=bad):
                failure = {"stage": "tls_terminator", "error": bad}
                self.assertTrue(any("unexpected TLS/segmentation error" in reason for reason in
                                    self.check(self.event(failure, tls_out=2))))

    def test_setup_reset_accepts_only_complete_prephase_relay(self):
        reset = {"stage": "proxy_a",
                 "error": "ConnectionResetError: [Errno 104] Connection reset by peer"}
        self.assertEqual(self.check(self.event(reset)), [])
        self.assertTrue(any("unexpected TLS/segmentation error" in reason for reason in
                            self.check(self.event(reset, closed=40))))
        corrupt = self.event(reset)
        corrupt["proxy_a_ciphertext"]["client_to_terminator_ciphertext"] = {
            "bytes_read": 4, "bytes_written": 0}
        self.assertTrue(any("unexpected TLS/segmentation error" in reason for reason in
                            self.check(corrupt)))
        self.assertTrue(any("unexpected TLS/segmentation error" in reason for reason in
                            self.check(self.event(reset), phase_zero=None)))


class LedgerTests(unittest.TestCase):
    def make_folder(self, root, *, connected):
        folder = Path(root)
        (folder / "proxy").mkdir()
        (folder / "summary.json").write_text(json.dumps({"run": {"warmup_end_monotonic_ns": 0}}))
        (folder / "events.jsonl").write_text(json.dumps({
            "event": "sample", "sample_monotonic_ns": 10_000_000_000, "elapsed_seconds": 10,
            "slots": [{"connected": connected, "ingame": False, "scene_state": 1}]}) + "\n")
        sizes = {"received_plaintext": 16384, "pending_tls_output": 131072,
                 "handshake_deframer": 65536, "outbound_ring": 5000, "inbound_ring": 32768}
        ledger = {"client_local_port": 123, "elapsed_seconds": 10,
                  "reserved_total_bytes": 250760, "overrun": False,
                  "actual_whole_connection_bytes": None}
        ledger.update({key: {"reserved_bytes": size, "observed_bytes": 0}
                       for key, size in sizes.items()})
        (folder / "candidate-five-compartment-ledger.jsonl").write_text(json.dumps(ledger) + "\n")
        empty = {"bytes": 0, "sha256": hashlib.sha256(b"").hexdigest()}
        (folder / "candidate-wss-payloads.jsonl").write_text(json.dumps({
            "client_local_port": 123, "elapsed_seconds": 10,
            "outbound": empty, "inbound": empty}) + "\n")
        (folder / "proxy" / "connections.jsonl").write_text('{"type":"proxy_stopped"}\n')
        proxy = {123: {"binary_payload_sha256": {
            "client_to_engine": {"read": empty["sha256"]},
            "engine_to_client": {"written": empty["sha256"]}},
            "binary_payload_bytes": {"client_to_engine": {"read": 0},
                                     "engine_to_client": {"written": 0}}}}
        return folder, proxy

    def test_loading_connected_account_retains_wss_port(self):
        with tempfile.TemporaryDirectory() as temporary:
            folder, proxy = self.make_folder(temporary, connected=True)
            errors = []
            summary.check_candidate(folder, proxy, 250760, errors)
            self.assertEqual(errors, [])

    def test_open_port_without_connected_session_fails(self):
        with tempfile.TemporaryDirectory() as temporary:
            folder, proxy = self.make_folder(temporary, connected=False)
            errors = []
            summary.check_candidate(folder, proxy, 250760, errors)
            self.assertTrue(any("active WSS ports" in error for error in errors))


class ArmReasonTests(unittest.TestCase):
    def test_same_folder_basename_produces_distinct_manifest_arm_reasons(self):
        with tempfile.TemporaryDirectory() as temporary:
            folders = [Path(temporary) / arm / "run" for arm in ("baseline", "candidate")]
            for folder, arm in zip(folders, ("baseline", "candidate")):
                folder.mkdir(parents=True)
                (folder / "run-manifest.json").write_text(json.dumps({
                    "build": arm, "commit": arm, "shared_harness_sha256": "x",
                    "proxy_sha256": "p", "seed": 1027, "profile": "release",
                    "mode": "smoke", "nav_build_mode": "skip", "nav_build_snapshot_root": "r"}))
                (folder / "nav-identity.json").write_text(json.dumps({
                    key: "x" for key in ("source_pack_sha256", "selected_pack_sha256",
                                           "source_flags_sha256", "selected_flags_sha256")}))
                (folder / "effective-config.json").write_text(json.dumps({
                    "fault": {"seed": 1027}, "workload": {},
                    "limits": {"connection_bytes": 250760},
                    "run": {"slots": 2, "duration_seconds": 180, "warmup_min_seconds": 0,
                            "sample_seconds": 10, "hash_seconds": 60}}))
                (folder / "cohort-exit.json").write_text('{"code":0}')
                (folder / "summary.json").write_text('{}')

            def fail_workload(folder, _summary, _config, _smoke, errors):
                errors.append(f"{folder.name}: scene error")
                return {"accounts": 2}

            with mock.patch.object(summary, "check_workload", side_effect=fail_workload), \
                    mock.patch.object(summary, "check_samples", return_value=[]), \
                    mock.patch.object(summary, "check_census", return_value={}), \
                    mock.patch.object(summary, "check_proxy", return_value={}), \
                    mock.patch.object(summary, "check_candidate", return_value={}):
                result = summary.evaluate(*folders, smoke=True)
            self.assertEqual(result["verdict"], "FAIL")
            self.assertIn("baseline: run: scene error", result["reasons"])
            self.assertIn("candidate: run: scene error", result["reasons"])


if __name__ == "__main__":
    unittest.main()
