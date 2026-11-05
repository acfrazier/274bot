#!/usr/bin/env python3
"""Guard the Engine A evaluator against false passes and false failures."""

import contextlib
import hashlib
import io
import json
from pathlib import Path
import tempfile
import sys
import unittest
from unittest import mock

import t14_soak_summary as summary


class SceneSamplesTests(unittest.TestCase):
    @staticmethod
    def sample(at, *, state=2, epoch=3, connected=True, ingame=None):
        return {"event": "sample", "elapsed_seconds": at // 1_000_000_000,
                "sample_monotonic_ns": at,
                "slots": [{"account": "slot", "connected": connected,
                           "stream_epoch": epoch, "ingame": state == 2 if ingame is None else ingame,
                           "scene_state": state}]}

    @staticmethod
    def scene2(at):
        return {"event": "scene2", "account": "slot", "scene2_monotonic_ns": at}

    def check(self, events, accounts=("slot",)):
        errors = []
        summary.check_scene_samples(Path("run"), events, list(accounts),
                                    {account: 0 for account in accounts}, 10, errors)
        return errors

    def test_connected_map_load_recovers_at_next_sample(self):
        self.assertEqual(self.check([self.sample(0), self.sample(10_000_000_000, state=1),
                                     self.scene2(10_045_000_000), self.sample(20_000_000_000)]), [])

    def test_play_sampler_gap_cannot_hide_a_long_load(self):
        errors = self.check([self.sample(0), self.sample(60_000_000_000, state=1),
                             self.scene2(60_045_000_000), self.sample(70_000_000_000)])
        self.assertTrue(any("Play sample gap exceeds 12s" in error for error in errors))

    def test_play_gap_must_be_positive_and_within_one_interval_plus_jitter(self):
        for at in (0, -1_000_000_000, 13_000_000_000):
            with self.subTest(at=at):
                errors = self.check([self.sample(0), self.sample(at)])
                self.assertTrue(any("Play sample gap exceeds 12s" in error for error in errors))

    def test_two_consecutive_loading_samples_fail_on_recovery(self):
        errors = self.check([self.sample(0), self.sample(10_000_000_000, state=1),
                             self.sample(20_000_000_000, state=1),
                             self.scene2(20_045_000_000), self.sample(30_000_000_000)])
        self.assertTrue(any("did not return to scene2 within 12s" in error for error in errors))

    def test_slow_scene2_event_fails_even_when_next_sample_is_ready(self):
        errors = self.check([self.sample(0), self.sample(10_000_000_000, state=1),
                             self.scene2(12_000_000_000), self.sample(20_000_000_000)])
        self.assertTrue(any("no scene2 return within 1s" in error for error in errors))

    def test_loading_sample_needs_its_own_later_scene2_event(self):
        for event in (None, {"event": "scene2", "account": "other",
                             "scene2_monotonic_ns": 10_045_000_000}):
            with self.subTest(event=event):
                events = [self.sample(0), self.sample(10_000_000_000, state=1)]
                if event is not None:
                    events.append(event)
                events.append(self.sample(20_000_000_000))
                errors = self.check(events)
                self.assertTrue(any("no scene2 return within 1s" in error for error in errors))

    def test_other_connected_account_scene2_does_not_finish_load(self):
        samples = [self.sample(0), self.sample(10_000_000_000, state=1),
                   self.sample(20_000_000_000)]
        for sample in samples:
            sample["slots"].append({"account": "other", "connected": True, "stream_epoch": 3,
                                    "ingame": True, "scene_state": 2})
        events = [samples[0], samples[1],
                  {"event": "scene2", "account": "other", "scene2_monotonic_ns": 10_045_000_000},
                  samples[2]]
        errors = self.check(events, ("slot", "other"))
        self.assertTrue(any("slot map load has no scene2 return" in error for error in errors))

    def test_long_map_load_fails(self):
        errors = self.check([self.sample(0), self.sample(10_000_000_000, state=1),
                             self.sample(20_000_000_000, state=1),
                             self.sample(30_000_000_000, state=1),
                             self.scene2(30_045_000_000), self.sample(40_000_000_000)])
        self.assertTrue(any("stayed in scene1 beyond 12s" in error for error in errors))

    def test_changed_epoch_and_disconnect_fail_during_loading(self):
        for changed, expected in (
            (self.sample(10_000_000_000, state=1, epoch=4), "changed stream epoch"),
            (self.sample(10_000_000_000, state=1, connected=False), "disconnected"),
        ):
            with self.subTest(changed=changed):
                errors = self.check([self.sample(0), changed, self.scene2(10_045_000_000),
                                     self.sample(20_000_000_000)])
                self.assertTrue(any(expected in error for error in errors))

    def test_unrecovered_and_nonloading_scene_fail(self):
        self.assertTrue(any("never returned" in error for error in
                            self.check([self.sample(0), self.sample(10_000_000_000, state=1)])))
        self.assertTrue(any("outside map loading" in error for error in
                            self.check([self.sample(0), self.sample(10_000_000_000, state=0)])))
        self.assertTrue(any("outside map loading" in error for error in
                            self.check([self.sample(0), self.sample(10_000_000_000,
                                                    state=1, ingame=True)])))

    def test_missing_epoch_fails_without_a_second_sample(self):
        errors = self.check([self.sample(0, epoch=None)])
        self.assertTrue(any("lacks a live stream epoch" in error for error in errors))


class TeardownTests(unittest.TestCase):
    @staticmethod
    def event(failure, *, started=10, closed=20, tls_out=0):
        counters = {"bytes_read": 0, "bytes_written": 0}
        return {"connection_id": "c000060", "started_monotonic_ns": started,
                "closed_monotonic_ns": closed,
                "stage_status": {"proxy_a": "failed" if failure["stage"] == "proxy_a" else "closed",
                                 "proxy_b": "closed",
                                 "tls_terminator": "failed" if failure["stage"] == "tls_terminator" else "closed"},
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
    def http_event(failure):
        event = TeardownTests.event(failure)
        event["websocket_handshake"] = {
            "upgrade_status": 200,
            "response_bytes": 37,
            "local_login_key_fixture": {"body_bytes": 7},
        }
        event["proxy_b_websocket"]["engine_to_client"].update({
            "wire_bytes_written": 7,
            "http_header_bytes_written": 37,
        })
        event["tls_terminator"]["proxy_b_to_client_plaintext"] = {
            "bytes_read": 44,
            "bytes_written": 44,
        }
        event["proxy_a_ciphertext"]["terminator_to_client_ciphertext"] = {
            "bytes_read": 44,
            "bytes_written": 44,
        }
        return event


    @staticmethod
    def close_frames():
        return {direction: [{"status_code": None, "reason": ""}]
                for direction in ("client_to_engine", "engine_to_client")}

    def check(self, event, phase_zero=30, arm="candidate"):
        errors = []
        summary.check_transport_teardown(Path("run"), arm, event, self.close_frames(),
                                         phase_zero, errors)
        return errors

    def test_baseline_tls12_is_observed_but_candidate_tls12_fails(self):
        event = self.event({"stage": "proxy_a", "error": "BrokenPipeError: [Errno 32] Broken pipe"})
        for version in ("TLSv1.2", "TLSv1.3"):
            with self.subTest(arm="baseline", version=version):
                event["tls_terminator"]["handshake"]["tls_version"] = version
                self.assertEqual(self.check(event, arm="baseline"), [])
        event["tls_terminator"]["handshake"]["tls_version"] = "TLSv1.2"
        self.assertTrue(any("unexpected TLS/segmentation error" in reason for reason in
                            self.check(event, arm="candidate")))
        event["tls_terminator"]["handshake"]["tls_version"] = "TLSv1.0"
        self.assertTrue(any("unexpected TLS/segmentation error" in reason for reason in
                            self.check(event, arm="baseline")))
        event["tls_terminator"]["handshake"]["tls_version"] = "TLSv1.3"
        self.assertEqual(self.check(event, arm="candidate"), [])

    def test_baseline_tls12_does_not_mask_real_teardown_failure(self):
        event = self.event({"stage": "proxy_b_websocket", "error": "FrameError: bad frame"})
        event["tls_terminator"]["handshake"]["tls_version"] = "TLSv1.2"
        self.assertTrue(any("unexpected TLS/segmentation error" in reason
                            for reason in self.check(event, arm="baseline")))

    def test_old_proxy_a_broken_pipe_acceptance_retains_13_and_16_byte_window(self):
        for connection_id, deficit in (("old-zero", 0), ("c000053", 13), ("c000057", 16)):
            for arm in ("baseline", "candidate"):
                with self.subTest(connection_id=connection_id, arm=arm):
                    event = self.event({
                        "stage": "proxy_a",
                        "error": "BrokenPipeError: [Errno 32] Broken pipe",
                    })
                    event["connection_id"] = connection_id
                    event["proxy_a_ciphertext"]["terminator_to_client_ciphertext"] = {
                        "bytes_read": deficit, "bytes_written": 0,
                    }
                    observations = []
                    errors = []
                    summary.check_transport_teardown(
                        Path(arm), arm, event, self.close_frames(), 30, errors,
                        observations if arm == "baseline" else None)
                    self.assertEqual(errors, [])
                    if arm == "baseline":
                        self.assertFalse(observations[0]["non_gating"])

    def test_reason_lists_match_the_old_rules_exactly_on_decided_edges(self):
        # The old rules decide FAIL for a proxy-A BrokenPipe outside the
        # 24-byte window with only the deficit reason, for both arms.
        for arm in ("baseline", "candidate"):
            with self.subTest(arm=arm):
                out_of_window = self.event(
                    {"stage": "proxy_a", "error": "BrokenPipeError: [Errno 32] Broken pipe"})
                out_of_window["proxy_a_ciphertext"]["terminator_to_client_ciphertext"] = {
                    "bytes_read": 30, "bytes_written": 0}
                self.assertEqual(self.check(out_of_window, arm=arm), [
                    "run: connection c000060: ciphertext/plaintext deficit extends "
                    "beyond terminal WS Close"])
        # The old rules decide FAIL for a failed proxy B with an incomplete
        # handshake with the one combined reason, never the same reason twice.
        incomplete = self.event(
            {"stage": "proxy_a",
             "error": "ConnectionResetError: [Errno 54] Connection reset by peer"})
        incomplete["failures"] = []
        incomplete["stage_status"]["proxy_a"] = "closed"
        incomplete["stage_status"]["proxy_b"] = "failed"
        incomplete["tls_terminator"]["handshake"]["completed"] = 0
        for arm in ("baseline", "candidate"):
            with self.subTest(arm=arm):
                self.assertEqual(self.check(incomplete, arm=arm), [
                    "run: connection c000060: application relay or TLS handshake "
                    "did not complete"])

    def test_new_baseline_rule_requires_zero_deficit(self):
        reset = {"stage": "proxy_a",
                 "error": "ConnectionResetError: [Errno 54] Connection reset by peer"}
        for deficit in (13, 16):
            with self.subTest(deficit=deficit):
                event = self.event(reset)
                event["proxy_a_ciphertext"]["terminator_to_client_ciphertext"] = {
                    "bytes_read": deficit, "bytes_written": 0,
                }
                observations = []
                errors = []
                summary.check_transport_teardown(
                    Path("baseline"), "baseline", event, self.close_frames(), 30,
                    errors, observations)
                self.assertTrue(any("unexpected TLS/segmentation error" in reason
                                    for reason in errors))
                self.assertFalse(observations[0]["non_gating"])

    def test_proxy_b_stage_failure_gates_both_arms_and_observation(self):
        fault = {"stage": "proxy_a",
                 "error": "ConnectionResetError: [Errno 54] Connection reset by peer"}
        for arm in ("baseline", "candidate"):
            with self.subTest(arm=arm):
                stage_only = self.event(fault)
                stage_only["failures"] = []
                stage_only["stage_status"]["proxy_a"] = "closed"
                stage_only["stage_status"]["proxy_b"] = "failed"
                errors = []
                summary.check_transport_teardown(
                    Path(arm), arm, stage_only, self.close_frames(), 30, errors)
                self.assertTrue(any("application relay or TLS handshake did not complete" in reason
                                    for reason in errors))

                if arm == "baseline":
                    event = self.event(fault)
                    event["stage_status"]["proxy_b"] = "failed"
                    observations = []
                    errors = []
                    summary.check_transport_teardown(
                        Path(arm), arm, event, self.close_frames(), 30, errors, observations)
                    self.assertFalse(observations[0]["non_gating"])

    def test_baseline_teardown_is_observed_only_after_lossless_close(self):
        failures = (
            {"stage": "proxy_a", "error": "ConnectionResetError: [Errno 54] Connection reset by peer"},
            {"stage": "tls_terminator", "error": "BrokenPipeError: [Errno 32] Broken pipe"},
        )
        for failure in failures:
            with self.subTest(failure=failure):
                event = self.event(failure, started=35, closed=40)
                observations = []
                errors = []
                summary.check_transport_teardown(Path("baseline"), "baseline", event,
                                                 self.close_frames(), 30, errors, observations)
                self.assertEqual(errors, [])
                self.assertEqual(observations[0]["failure"], failure)
                self.assertTrue(observations[0]["non_gating"])
                self.assertTrue(any("unexpected TLS/segmentation error" in reason
                                    for reason in self.check(event, phase_zero=30, arm="candidate")))

                missing = self.close_frames()
                missing["engine_to_client"] = []
                observations = []
                errors = []
                summary.check_transport_teardown(Path("baseline"), "baseline", event,
                                                 missing, 30, errors, observations)
                self.assertTrue(any("unexpected TLS/segmentation error" in reason for reason in errors))
                self.assertFalse(observations[0]["non_gating"])

                partial = self.event(failure, started=35, closed=40)
                partial["proxy_a_ciphertext"]["client_to_terminator_ciphertext"] = {
                    "bytes_read": 1, "bytes_written": 0}
                observations = []
                errors = []
                summary.check_transport_teardown(Path("baseline"), "baseline", partial,
                                                 self.close_frames(), 30, errors, observations)
                self.assertTrue(any("unexpected TLS/segmentation error" in reason for reason in errors))
                self.assertFalse(observations[0]["non_gating"])

                mismatched = self.event(failure, started=35, closed=40)
                mismatched["binary_payload_hashes_match_at_proxy_b"]["engine_to_client"] = False
                observations = []
                errors = []
                summary.check_transport_teardown(Path("baseline"), "baseline", mismatched,
                                                 self.close_frames(), 30, errors, observations)
                self.assertTrue(any("unexpected TLS/segmentation error" in reason for reason in errors))
                self.assertFalse(observations[0]["non_gating"])

    def test_unknown_baseline_failure_still_gates(self):
        failure = {"stage": "tls_terminator", "error": "SSLError: certificate verify failed"}
        self.assertTrue(any("unexpected TLS/segmentation error" in reason
                            for reason in self.check(self.event(failure), arm="baseline")))
        self.assertIs(summary.baseline_teardown_fault(
            failure, {"tls_terminator": "failed"}), False)

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
        self.assertEqual(self.check(self.event(
            {"stage": "tls_terminator",
             "error": "SSLEOFError: EOF occurred in violation of protocol (_ssl.c:2556)"},
            tls_out=2)), [])
        read_site = self.event(original)
        self.assertTrue(any("unexpected TLS/segmentation error" in reason for reason in
                            self.check(read_site)))
        self.assertFalse(summary.accepted_tls_close_error(
            {"stage": "proxy_a", "error": original["error"]},
            {"proxy_a": "closed", "tls_terminator": "failed"}, (0, 0, 0, 2), True))

    def test_ssl_eof_rejects_invalid_stage_deficit_or_phase(self):
        failure = {"stage": "tls_terminator",
                   "error": "SSLEOFError: EOF occurred in violation of protocol (_ssl.c:2557)"}
        for stage, status in (("tls_terminator", "closed"), ("proxy_a", "failed")):
            with self.subTest(stage=stage):
                invalid = self.event(failure, tls_out=2)
                invalid["stage_status"][stage] = status
                self.assertTrue(any("unexpected TLS/segmentation error" in reason for reason in
                                    self.check(invalid)))
        partial = self.event(failure, tls_out=2)
        partial["proxy_a_ciphertext"]["terminator_to_client_ciphertext"] = {
            "bytes_read": 10, "bytes_written": 0}
        self.assertTrue(any("unexpected TLS/segmentation error" in reason for reason in
                            self.check(partial)))
        self.assertTrue(any("unexpected TLS/segmentation error" in reason for reason in
                            self.check(self.event(failure, started=35, closed=40, tls_out=2))))
        self.assertTrue(any("unexpected TLS/segmentation error" in reason for reason in
                            self.check(self.event(failure, tls_out=2), phase_zero=None)))


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
        partial = self.event(reset)
        partial["proxy_a_ciphertext"]["terminator_to_client_ciphertext"] = {
            "bytes_read": 10, "bytes_written": 0}
        self.assertTrue(any("unexpected TLS/segmentation error" in reason for reason in
                            self.check(partial)))
        self.assertTrue(any("unexpected TLS/segmentation error" in reason for reason in
                            self.check(self.event(reset, tls_out=2))))
        wrong_stage = self.event(reset)
        wrong_stage["stage_status"]["proxy_a"] = "closed"
        self.assertTrue(any("unexpected TLS/segmentation error" in reason for reason in
                            self.check(wrong_stage)))
        wrong_errno = {"stage": "proxy_a",
                       "error": "ConnectionResetError: [Errno 54] Connection reset by peer"}
        self.assertTrue(any("unexpected TLS/segmentation error" in reason for reason in
                            self.check(self.event(wrong_errno))))
        incomplete = self.event(reset)
        incomplete["proxy_b_websocket"]["engine_to_client"]["opcodes_written"]["8"] = 0
        self.assertTrue(any("unexpected TLS/segmentation error" in reason for reason in
                            self.check(incomplete)))
        hash_mismatch = self.event(reset)
        hash_mismatch["binary_payload_hashes_match_at_proxy_b"]["engine_to_client"] = False
        self.assertTrue(any("unexpected TLS/segmentation error" in reason for reason in
                            self.check(hash_mismatch)))

    def test_post_response_proxy_a_resets_are_observed_non_gating_for_both_arms(self):
        reset_errors = (
            "ConnectionResetError: [Errno 54] Connection reset by peer",
            "ConnectionResetError: [Errno 104] Connection reset by peer",
            "ConnectionResetError: [WinError 10054] "
            "An existing connection was forcibly closed by the remote host",
        )
        for error in reset_errors:
            observations_by_arm = {}
            for arm in ("baseline", "candidate"):
                with self.subTest(arm=arm, error=error):
                    failure = {"stage": "proxy_a", "error": error}
                    event = self.http_event(failure)
                    observations = []
                    errors = []
                    summary.check_transport_teardown(
                        Path(arm), arm, event, self.close_frames(), 30, errors, observations)
                    self.assertEqual(errors, [])
                    self.assertEqual(observations, [{
                        "connection_id": event["connection_id"],
                        "failure": failure,
                        "reason": (
                            "post-response proxy-A reset after a complete status-200 HTTPS "
                            "body and header, zero transport deficits, and matching binary hashes"
                        ),
                        "non_gating": True,
                        "byte_deficits": [0, 0, 0, 0],
                    }])
                    observations_by_arm[arm] = observations
            self.assertEqual(observations_by_arm["baseline"],
                             observations_by_arm["candidate"])

    def test_http_reset_requires_failed_proxy_a_stage_and_status_200(self):
        reset = {"stage": "proxy_a",
                 "error": "ConnectionResetError: [Errno 54] Connection reset by peer"}
        for arm in ("baseline", "candidate"):
            with self.subTest(arm=arm, reason="proxy A stage is not failed"):
                event = self.http_event(reset)
                event["stage_status"]["proxy_a"] = "closed"
                observations = []
                errors = []
                summary.check_transport_teardown(
                    Path(arm), arm, event, self.close_frames(), 30, errors, observations)
                self.assertEqual(errors, [
                    f"{arm}: connection {event['connection_id']}: "
                    "HTTPS teardown error is not the witnessed terminal TLS write"])
                self.assertFalse(observations)

            with self.subTest(arm=arm, reason="non-200 non-101 status"):
                event = self.http_event(reset)
                event["websocket_handshake"]["upgrade_status"] = 204
                observations = []
                errors = []
                summary.check_transport_teardown(
                    Path(arm), arm, event, self.close_frames(), 30, errors, observations)
                self.assertEqual(errors, [
                    f"{arm}: connection {event['connection_id']}: "
                    "incomplete non-200/non-101 HTTP transfer"])
                self.assertFalse(observations)

    def test_post_response_reset_requires_complete_lossless_matching_evidence(self):
        reset = {"stage": "proxy_a",
                 "error": "ConnectionResetError: [Errno 54] Connection reset by peer"}

        def incomplete_body(event):
            event["proxy_b_websocket"]["engine_to_client"]["wire_bytes_written"] = 6

        def incomplete_header(event):
            event["proxy_b_websocket"]["engine_to_client"]["http_header_bytes_written"] = 36

        def byte_deficit(event):
            event["tls_terminator"]["proxy_b_to_client_plaintext"] = {
                "bytes_read": 1, "bytes_written": 0}

        def unmatched_hash(event):
            event["binary_payload_hashes_match_at_proxy_b"]["engine_to_client"] = False


        def missing_body_counters(event):
            event["websocket_handshake"].pop("local_login_key_fixture")

        def missing_transport_counters(event):
            event["tls_terminator"]["proxy_b_to_client_plaintext"] = {}

        def missing_proxy_a_transport_counters(event):
            event["proxy_a_ciphertext"]["terminator_to_client_ciphertext"] = {}

        def short_terminator_receipt(event):
            counters = event["tls_terminator"]["proxy_b_to_client_plaintext"]
            counters["bytes_read"] = 43
            counters["bytes_written"] = 43

        def zero_terminator_receipt(event):
            counters = event["tls_terminator"]["proxy_b_to_client_plaintext"]
            counters["bytes_read"] = 0
            counters["bytes_written"] = 0

        def short_proxy_a_receipt(event):
            counters = event["proxy_a_ciphertext"]["terminator_to_client_ciphertext"]
            counters["bytes_read"] = 43
            counters["bytes_written"] = 43

        def zero_proxy_a_receipt(event):
            counters = event["proxy_a_ciphertext"]["terminator_to_client_ciphertext"]
            counters["bytes_read"] = 0
            counters["bytes_written"] = 0

        def noninteger_terminator_receipt(event):
            counters = event["tls_terminator"]["proxy_b_to_client_plaintext"]
            counters["bytes_read"] = True
            counters["bytes_written"] = True

        def negative_proxy_a_receipt(event):
            counters = event["proxy_a_ciphertext"]["terminator_to_client_ciphertext"]
            counters["bytes_read"] = -1
            counters["bytes_written"] = -1

        def missing_hash_match(event):
            event["binary_payload_hashes_match_at_proxy_b"].pop("engine_to_client")

        def incomplete_handshake(event):
            event["tls_terminator"]["handshake"]["completed"] = 0

        def proxy_b_not_closed(event):
            event["stage_status"]["proxy_b"] = "failed"


        invalid_evidence = (
            ("incomplete body", incomplete_body),
            ("incomplete header", incomplete_header),
            ("byte deficit", byte_deficit),
            ("unmatched hash", unmatched_hash),
            ("missing body counters", missing_body_counters),
            ("missing transport counters", missing_transport_counters),
            ("missing hash match", missing_hash_match),
            ("incomplete TLS handshake", incomplete_handshake),
            ("proxy B not closed", proxy_b_not_closed),
            ("missing proxy-A transport counters", missing_proxy_a_transport_counters),
            ("short terminator receipt", short_terminator_receipt),
            ("zero terminator receipt", zero_terminator_receipt),
            ("short proxy-A receipt", short_proxy_a_receipt),
            ("zero proxy-A receipt", zero_proxy_a_receipt),
            ("noninteger terminator receipt", noninteger_terminator_receipt),
            ("negative proxy-A receipt", negative_proxy_a_receipt),
        )
        for arm in ("baseline", "candidate"):
            for name, invalidate in invalid_evidence:
                with self.subTest(arm=arm, reason=name):
                    event = self.http_event(reset)
                    invalidate(event)
                    observations = []
                    errors = []
                    summary.check_transport_teardown(
                        Path(arm), arm, event, self.close_frames(), 30, errors, observations)
                    self.assertTrue(errors)
                    self.assertFalse(observations)

        invalid_failures = (
            {"stage": "proxy_a", "error": "BrokenPipeError: [Errno 32] Broken pipe"},
            {"stage": "proxy_a",
             "error": "wrapped ConnectionResetError: [Errno 54] Connection reset by peer"},
            {"stage": "proxy_a",
             "error": "ConnectionResetError: [Errno 54] Connection reset by peer",
             "detail": "unrecognized"},
            {"stage": "proxy_b",
             "error": "ConnectionResetError: [Errno 54] Connection reset by peer"},
        )
        for arm in ("baseline", "candidate"):
            for failure in invalid_failures:
                with self.subTest(arm=arm, failure=failure):
                    event = self.http_event(reset)
                    event["failures"] = [failure]
                    if failure["stage"] != "proxy_a":
                        event["stage_status"]["proxy_a"] = "closed"
                        event["stage_status"][failure["stage"]] = "failed"
                    observations = []
                    errors = []
                    summary.check_transport_teardown(
                        Path(arm), arm, event, self.close_frames(), 30, errors, observations)
                    self.assertTrue(errors)
                    self.assertFalse(observations)

        for arm in ("baseline", "candidate"):
            with self.subTest(arm=arm, failure="reset plus unrelated failure"):
                event = self.http_event(reset)
                event["failures"].append({
                    "stage": "tls_terminator", "error": "UnexpectedError: unrelated"}
                )
                event["stage_status"]["tls_terminator"] = "failed"
                observations = []
                errors = []
                summary.check_transport_teardown(
                    Path(arm), arm, event, self.close_frames(), 30, errors, observations)
                self.assertTrue(errors)
                self.assertFalse(observations)


    def test_http_reset_allowance_does_not_apply_to_completed_websocket(self):
        failure = {"stage": "proxy_a",
                   "error": "ConnectionResetError: [Errno 54] Connection reset by peer"}
        # Only the upgrade status changes from the accepted HTTPS observation.
        event = self.http_event(failure)
        event["websocket_handshake"]["upgrade_status"] = 101
        self.assertTrue(all(
            event["proxy_b_websocket"][direction]["opcodes_read"].get("8") == 1
            and event["proxy_b_websocket"][direction]["opcodes_written"].get("8") == 1
            for direction in ("client_to_engine", "engine_to_client")
        ))
        self.assertEqual(self.check(event, arm="candidate")[-1],
                         "run: connection c000060: unexpected TLS/segmentation error")

    def test_post_response_reset_accepts_complete_http_fallback_body(self):
        failure = {"stage": "proxy_a",
                   "error": "ConnectionResetError: [Errno 104] Connection reset by peer"}
        for arm in ("baseline", "candidate"):
            with self.subTest(arm=arm):
                event = self.http_event(failure)
                event["websocket_handshake"].pop("local_login_key_fixture")
                event["proxy_b_websocket"]["engine_to_client"].update({
                    "proxy_b_http_fallback_bytes_read": 7,
                    "proxy_b_http_fallback_bytes_written": 7,
                })
                observations = []
                errors = []
                summary.check_transport_teardown(
                    Path(arm), arm, event, self.close_frames(), 30, errors, observations)
                self.assertEqual(errors, [])
                self.assertTrue(observations[0]["non_gating"])

    def test_http_tls_terminator_broken_pipe_remains_allowed(self):
        failure = {"stage": "tls_terminator",
                   "error": "BrokenPipeError: [Errno 32] Broken pipe"}
        for arm in ("baseline", "candidate"):
            with self.subTest(arm=arm):
                event = self.http_event(failure)
                self.assertEqual(self.check(event, arm=arm), [])





class LedgerTests(unittest.TestCase):
    def make_folder(self, root, *, connected, extra_connected=False):
        folder = Path(root)
        (folder / "proxy").mkdir()
        (folder / "summary.json").write_text(json.dumps({"run": {"warmup_end_monotonic_ns": 0}}))
        slots = [{"connected": connected, "ingame": False, "scene_state": 1}]
        if extra_connected:
            slots.append({"connected": True, "ingame": True, "scene_state": 2})
        (folder / "events.jsonl").write_text(json.dumps({
            "event": "sample", "sample_monotonic_ns": 10_000_000_000,
            "elapsed_seconds": 10, "slots": slots}) + "\n")
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

    def test_connected_session_without_a_port_fails(self):
        with tempfile.TemporaryDirectory() as temporary:
            folder, proxy = self.make_folder(temporary, connected=True, extra_connected=True)
            errors = []
            summary.check_candidate(folder, proxy, 250760, errors)
            self.assertTrue(any("active WSS ports" in error for error in errors))


class ArmReasonTests(unittest.TestCase):
    @staticmethod
    def folders(root, *, smoke=True):
        folders = [Path(root) / arm / "run" for arm in ("baseline", "candidate")]
        for folder, arm in zip(folders, ("baseline", "candidate")):
            folder.mkdir(parents=True)
            (folder / "run-manifest.json").write_text(json.dumps({
                "build": arm, "commit": arm, "shared_harness_sha256": "x",
                "proxy_sha256": "p", "seed": 1027, "profile": "release",
                "mode": "smoke" if smoke else "qualification",
                "nav_build_mode": "skip", "nav_build_snapshot_root": "r"}))
            (folder / "nav-identity.json").write_text(json.dumps({
                key: "x" for key in ("source_pack_sha256", "selected_pack_sha256",
                                     "source_flags_sha256", "selected_flags_sha256")}))
            (folder / "effective-config.json").write_text(json.dumps({
                "fault": {"seed": 1027}, "workload": {},
                "limits": {"connection_bytes": 250760,
                           "rss_final_baseline_extra_bytes_per_connection": 250760,
                           "rss_final_baseline_extra_percent": 5,
                           "rss_slope_baseline_slack_bytes_per_hour": 1048576},
                "run": {"slots": 2 if smoke else 20,
                        "duration_seconds": 180 if smoke else 7200,
                        "warmup_min_seconds": 0, "sample_seconds": 10, "hash_seconds": 60}}))
            (folder / "cohort-exit.json").write_text('{"code":0}')
            (folder / "summary.json").write_text('{"run":{"warmup_end_monotonic_ns":0}}')
        return folders

    def test_same_folder_basename_produces_distinct_manifest_arm_reasons(self):
        with tempfile.TemporaryDirectory() as temporary:
            folders = self.folders(temporary)

            def fail_workload(folder, _summary, _config, _smoke, errors):
                errors.append(f"{folder.name}: scene error")
                return {"accounts": 2}

            def fail_proxy(folder, _arm, _full, errors, _observations=None):
                errors.append(f"{folder.name}: proxy error")
                return {}

            def fail_candidate(_folder, _proxy, _limit, errors):
                errors.append("ledger error")
                return {}

            with mock.patch.object(summary, "check_workload", side_effect=fail_workload), \
                    mock.patch.object(summary, "check_samples", return_value=[]), \
                    mock.patch.object(summary, "check_census", return_value={}), \
                    mock.patch.object(summary, "check_proxy", side_effect=fail_proxy), \
                    mock.patch.object(summary, "check_candidate", side_effect=fail_candidate):
                result = summary.evaluate(*folders, smoke=True)
            self.assertEqual(result["verdict"], "FAIL")
            self.assertIn("baseline: run: scene error", result["reasons"])
            self.assertIn("candidate: run: scene error", result["reasons"])
            self.assertIn("baseline: run: proxy error", result["reasons"])
            self.assertIn("candidate: run: proxy error", result["reasons"])
            self.assertIn("candidate: ledger error", result["reasons"])

    def test_pair_verdict_retains_candidate_gate_and_baseline_observation(self):
        fault = {"stage": "proxy_a",
                 "error": "ConnectionResetError: [Errno 54] Connection reset by peer"}
        for faulty_arm, expected in (("baseline", "PASS"), ("candidate", "FAIL")):
            with self.subTest(faulty_arm=faulty_arm), tempfile.TemporaryDirectory() as temporary:
                folders = self.folders(temporary, smoke=False)
                events = {}
                for arm in ("baseline", "candidate"):
                    event = TeardownTests.event(fault, started=35, closed=40)
                    if arm != faulty_arm:
                        event["failures"] = []
                        event["stage_status"]["proxy_a"] = "closed"
                    events[arm] = event

                def proxy_receipt(folder, arm, _full, errors, observations=None):
                    summary.check_transport_teardown(folder, arm, events[arm],
                                                     TeardownTests.close_frames(), 30,
                                                     errors, observations)
                    return {123: events[arm]}

                with mock.patch.object(summary, "check_nav_identity"), \
                        mock.patch.object(summary, "check_workload", return_value={}), \
                        mock.patch.object(summary, "check_samples", return_value=[]), \
                        mock.patch.object(summary, "check_census", return_value={}), \
                        mock.patch.object(summary, "check_proxy", side_effect=proxy_receipt), \
                        mock.patch.object(summary, "check_candidate", return_value={}), \
                        mock.patch.object(summary, "rss_arm_summary", return_value=(0, 3600, 100)) as rss, \
                        mock.patch.object(summary, "rss_thresholds", return_value=(200, 1)):
                    result = summary.evaluate(*folders, smoke=False)
                    if faulty_arm == "baseline":
                        (folders[0] / "cohort-exit.json").write_text('{"code":1}')
                        self.assertTrue(any("baseline: cohort process failed" in reason
                                            for reason in summary.evaluate(*folders, smoke=False)["reasons"]))
                        (folders[0] / "cohort-exit.json").write_text('{"code":0}')
                        hashes = events["baseline"]["binary_payload_hashes_match_at_proxy_b"]
                        hashes["engine_to_client"] = False
                        self.assertTrue(any("baseline: " in reason and "hash diverged" in reason
                                            for reason in summary.evaluate(*folders, smoke=False)["reasons"]))
                        hashes["engine_to_client"] = True
                        rss.side_effect = [(0, 3600, 100), (2, 3600, 100)]
                        self.assertTrue(any("candidate post-warmup RSS slope exceeded baseline" in reason
                                            for reason in summary.evaluate(*folders, smoke=False)["reasons"]))
                self.assertEqual(result["verdict"], expected, result["reasons"])
                if faulty_arm == "baseline":
                    self.assertEqual(result["baseline"]["teardown_observations"][0]["failure"], fault)
                    self.assertTrue(result["baseline"]["teardown_observations"][0]["non_gating"])
                    self.assertEqual(result["reasons"], [])
                else:
                    self.assertTrue(any(reason.startswith("candidate: ") for reason in result["reasons"]))


    def test_http_reset_observation_is_reported_for_both_arms(self):
        with tempfile.TemporaryDirectory() as temporary:
            folders = self.folders(temporary)
            failure = {"stage": "proxy_a",
                       "error": "ConnectionResetError: [Errno 54] Connection reset by peer"}
            events = {
                arm: TeardownTests.http_event(failure)
                for arm in ("baseline", "candidate")
            }

            def proxy_receipt(folder, arm, _full, errors, observations=None):
                summary.check_transport_teardown(
                    folder, arm, events[arm], TeardownTests.close_frames(), 30,
                    errors, observations)
                return {123: events[arm]}

            with mock.patch.object(summary, "check_nav_identity"), \
                    mock.patch.object(summary, "check_workload", return_value={}), \
                    mock.patch.object(summary, "check_samples", return_value=[]), \
                    mock.patch.object(summary, "check_census", return_value={}), \
                    mock.patch.object(summary, "check_proxy", side_effect=proxy_receipt), \
                    mock.patch.object(summary, "check_candidate", return_value={}):
                result = summary.evaluate(*folders, smoke=True)

            self.assertEqual(result["verdict"], "NONQUALIFYING_SMOKE_OK",
                             result["reasons"])
            self.assertEqual(result["baseline"]["teardown_observations"],
                             result["candidate"]["teardown_observations"])
            observation = result["baseline"]["teardown_observations"][0]
            self.assertEqual(observation["connection_id"], "c000060")
            self.assertTrue(observation["non_gating"])
            self.assertIn("complete status-200 HTTPS body and header", observation["reason"])

    def test_baseline_tls_version_is_recorded_in_result(self):
        with tempfile.TemporaryDirectory() as temporary:
            folders = self.folders(temporary)

            def proxy_receipt(_folder, arm, _full, _errors, _observations=None):
                version = "TLSv1.2" if arm == "baseline" else "TLSv1.3"
                return {123: {"tls_terminator": {"handshake": {"tls_version": version}}}}

            with mock.patch.object(summary, "check_workload", return_value={}), \
                    mock.patch.object(summary, "check_samples", return_value=[]), \
                    mock.patch.object(summary, "check_census", return_value={}), \
                    mock.patch.object(summary, "check_proxy", side_effect=proxy_receipt), \
                    mock.patch.object(summary, "check_candidate", return_value={}):
                result = summary.evaluate(*folders, smoke=True)
            self.assertEqual(result["baseline"]["tls_versions_observed"], ["TLSv1.2"])

    def test_missing_evidence_exception_names_the_arm(self):
        for missing in ("baseline", "candidate"):
            with self.subTest(missing=missing), tempfile.TemporaryDirectory() as temporary:
                folders = self.folders(temporary)
                (folders[0 if missing == "baseline" else 1] / "run-manifest.json").unlink()
                output = Path(temporary) / "failure.json"
                argv = ["t14_soak_summary.py", "--baseline", str(folders[0]),
                        "--candidate", str(folders[1]), "--output", str(output)]
                with mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(summary.main(), 1)
                reason = json.loads(output.read_text())["reasons"][0]
                self.assertTrue(reason.startswith(f"{missing}: missing or invalid evidence:"), reason)

    def test_candidate_workload_exception_names_candidate(self):
        with tempfile.TemporaryDirectory() as temporary:
            folders = self.folders(temporary)

            def fail_candidate(folder, *_args):
                if folder == folders[1]:
                    raise KeyError("candidate workload")
                return {"accounts": 2}

            with mock.patch.object(summary, "check_workload", side_effect=fail_candidate):
                with self.assertRaisesRegex(summary.EvidenceError, "^candidate: missing or invalid evidence:"):
                    summary.evaluate(*folders, smoke=True)

    def test_manifest_label_must_match_arm_folder(self):
        for arm in ("baseline", "candidate"):
            with self.subTest(arm=arm), tempfile.TemporaryDirectory() as temporary:
                folders = self.folders(temporary)
                manifest = folders[0 if arm == "baseline" else 1] / "run-manifest.json"
                receipt = json.loads(manifest.read_text())
                receipt["build"] = "candidate" if arm == "baseline" else "baseline"
                manifest.write_text(json.dumps(receipt))
                with self.assertRaisesRegex(summary.EvidenceError, f"^{arm}: run manifest build"):
                    summary.evaluate(*folders, smoke=True)

    def test_missing_nav_key_names_exact_arm(self):
        for arm in ("baseline", "candidate"):
            for missing in ("source_pack_sha256", "selected_flags_sha256"):
                with self.subTest(arm=arm, missing=missing), tempfile.TemporaryDirectory() as temporary:
                    folders = self.folders(temporary)
                    nav = folders[0 if arm == "baseline" else 1] / "nav-identity.json"
                    receipt = json.loads(nav.read_text())
                    receipt.pop(missing)
                    if missing == "selected_flags_sha256":
                        receipt["source_pack_sha256"] = "mismatch"
                    nav.write_text(json.dumps(receipt))
                    with self.assertRaisesRegex(summary.EvidenceError, f"^{arm}: missing or invalid evidence:"):
                        summary.evaluate(*folders, smoke=True)

    def test_missing_candidate_connection_limit_names_candidate(self):
        with tempfile.TemporaryDirectory() as temporary:
            folders = self.folders(temporary)
            for folder in folders:
                config = folder / "effective-config.json"
                receipt = json.loads(config.read_text())
                receipt["limits"].pop("connection_bytes")
                config.write_text(json.dumps(receipt))
            with mock.patch.object(summary, "check_workload", return_value={}), \
                    mock.patch.object(summary, "check_samples", return_value=[]), \
                    mock.patch.object(summary, "check_census", return_value={}), \
                    mock.patch.object(summary, "check_proxy", return_value={}):
                with self.assertRaisesRegex(summary.EvidenceError, "^candidate: missing or invalid evidence:"):
                    summary.evaluate(*folders, smoke=True)

    def test_rss_exception_names_arm(self):
        samples = [{"sample_monotonic_ns": 0, "rss_bytes": 100},
                   {"sample_monotonic_ns": 10_000_000_000, "rss_bytes": 110}]
        for arm in ("baseline", "candidate"):
            with self.subTest(arm=arm), tempfile.TemporaryDirectory() as temporary:
                folders = self.folders(temporary, smoke=False)
                (folders[0 if arm == "baseline" else 1] / "summary.json").write_text('{"run":{}}')
                with mock.patch.object(summary, "check_workload", return_value={}), \
                        mock.patch.object(summary, "check_samples", return_value=samples), \
                        mock.patch.object(summary, "check_census", return_value={}), \
                        mock.patch.object(summary, "check_proxy", return_value={}), \
                        mock.patch.object(summary, "check_candidate", return_value={}):
                    with self.assertRaisesRegex(summary.EvidenceError, f"^{arm}: missing or invalid evidence:"):
                        summary.evaluate(*folders, smoke=False)

    def test_missing_candidate_rss_limit_names_candidate(self):
        samples = [{"sample_monotonic_ns": 0, "rss_bytes": 100},
                   {"sample_monotonic_ns": 10_000_000_000, "rss_bytes": 110}]
        for key in ("rss_final_baseline_extra_percent", "rss_slope_baseline_slack_bytes_per_hour"):
            with self.subTest(key=key), tempfile.TemporaryDirectory() as temporary:
                folders = self.folders(temporary, smoke=False)
                for folder in folders:
                    config = folder / "effective-config.json"
                    receipt = json.loads(config.read_text())
                    receipt["limits"].pop(key)
                    config.write_text(json.dumps(receipt))
                with mock.patch.object(summary, "check_workload", return_value={}), \
                        mock.patch.object(summary, "check_samples", return_value=samples), \
                        mock.patch.object(summary, "check_census", return_value={}), \
                        mock.patch.object(summary, "check_proxy", return_value={}), \
                        mock.patch.object(summary, "check_candidate", return_value={}):
                    with self.assertRaisesRegex(summary.EvidenceError, "^candidate: missing or invalid evidence:"):
                        summary.evaluate(*folders, smoke=False)


if __name__ == "__main__":
    unittest.main()
