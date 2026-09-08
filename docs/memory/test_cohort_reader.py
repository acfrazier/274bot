"""Focused fail-closed tests for the fixed-window cohort reader."""
from __future__ import annotations

import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

import cohort_reader as cr
import reference_metrics as rm

ROOT = pathlib.Path(__file__).resolve().parent

# Source-faithful fixed window: default harness observe=600s on process mono.
# Harness elapsed_s is independent (warmup 120 + observe 600).
START = 50_000_000_000
OBSERVE_S = 600
END = START + OBSERVE_S * 1_000_000_000
TAIL = 5_000_000_000
OBSERVE = END - START
HARNESS_OBS_START_S = 120.0
HARNESS_OBS_END_S = 720.0
# First observe ~1s after arm; final observe forced at mono END; drain after.
FIRST_OBS_MONO = START + 1_000_000_000
MID_OBS_MONO = START + 200_000_000_000
FINAL_OBS_MONO = END
DRAIN_MONO = END + 2_000_000_000
FIRST_OBS_ELAPSED_S = 121.0
MID_OBS_ELAPSED_S = 320.0
FINAL_OBS_ELAPSED_S = 720.0
DRAIN_ELAPSED_S = 722.0


def _fnv(name: str) -> int:
    return cr.slot_id_for(name)


def _names(n: int) -> list[str]:
    return [f"cohort-slot-{i:02d}" for i in range(n)]


def _ids(n: int) -> list[int]:
    return [_fnv(name) for name in _names(n)]


def _boundaries():
    return {"start_mono_ns": START, "end_mono_ns": END, "tail_ns": TAIL}


def _meta(n=1, frontend="panel", **extra):
    base = {
        "frontend": frontend,
        "n": n,
        "workload": "active",
        "responsiveness_profile": True,
        "responsiveness_fine": True,
        "scheduling_profile": False,
        "render_profile": False,
        "gpu_completion_profile": False,
        "render_policy": "focused-one",
        "started_unix": 1.0,
    }
    base.update(extra)
    return base


def _qual_slots(n: int) -> list[dict]:
    return [
        {
            "ordinal": i,
            "name": name,
            "responsiveness_slot_id": _fnv(name),
            "cadence_slot_id": _fnv(name) ^ 0x1111,
            "state": "ready",
            "error": None,
            "runtime": None,
            "client": None,
            "runtime_settings": None,
        }
        for i, name in enumerate(_names(n))
    ]


def _settings(n: int, frontend="panel", policy="focused-one"):
    return {
        "frontend": frontend,
        "n": n,
        "workload": "active",
        "render_policy_requested": policy,
        "responsiveness_profile_enabled": True,
        "responsiveness_fine_enabled": True,
        "scheduling_profile_enabled": False,
        "render_profile_enabled": False,
        "gpu_completion_profile_enabled": False,
    }


def _event(slot_id, generation, sequence, start_ns, surface, complete_ns, outcome="Completed"):
    return {
        "id": {
            "slot_id": slot_id,
            "generation": generation,
            "sequence": sequence,
            "start_mono_ns": start_ns,
            "surface": surface,
        },
        "complete_mono_ns": complete_ns,
        "outcome": outcome,
    }


def _clock(elapsed_mono_ns: int, *, slop: int = 50_000) -> dict:
    """Process-mono sample clock matching host-play serialization shape."""
    lo = max(0, elapsed_mono_ns - slop)
    hi = elapsed_mono_ns + slop
    return {
        "domain": "responsiveness_process_mono",
        "elapsed_mono_ns": elapsed_mono_ns,
        "elapsed_mono_ms_lower": elapsed_mono_ns // 1_000_000,
        "elapsed_mono_ms_upper": (elapsed_mono_ns + 999_999) // 1_000_000,
        "read_mono_ns_lower": lo,
        "read_mono_ns_upper": hi,
        "read_mono_ms_lower": lo // 1_000_000,
        "read_mono_ms_upper": (hi + 999_999) // 1_000_000,
        "sample_mono_ns_lower": lo,
        "sample_mono_ns_upper": hi,
        "sample_mono_ms_lower": lo // 1_000_000,
        "sample_mono_ms_upper": (hi + 999_999) // 1_000_000,
        "fine_latency_enabled": True,
        "means": (
            "process_local_mono_ns_enclosing_elapsed_capture_and_registry_read"
            "_floor_ceil_ms_siblings_not_cross_run"
        ),
    }


def _profile_row(sid: int, gen: int, sample_mono: int, *, ended: bool = False) -> dict:
    """Per-slot row with capture brackets enclosed by the sample mono."""
    # Capture ends slightly before sample read; starts inside the observe window.
    cap_lo = max(START, sample_mono - 500_000_000)
    cap_hi = max(cap_lo, sample_mono - 100_000)
    return {
        "slot_id": sid,
        "generation": gen,
        "ended": ended,
        "decode_capture_mono_ns_lower": cap_lo,
        "decode_capture_mono_ns_upper": cap_hi,
        "input_capture_mono_ns_lower": cap_lo,
        "input_capture_mono_ns_upper": cap_hi,
    }


def _build_records(
    claimed_path: str,
    *,
    n: int = 1,
    frontend: str = "panel",
    input_kind: str = "focused-one",
    decode_events=None,
    input_events=None,
    losses=None,
    loss_count=0,
    overflow=0,
    complete=True,
    terminal_available=True,
    records_n=None,
    losses_n=None,
    pending_n=0,
    producers_joined=True,
    observe_end_elapsed_s=HARNESS_OBS_END_S,
    trailing_after_terminal=False,
):
    b = _boundaries()
    ids = _ids(n)
    if input_kind == "focused-one":
        input_pop = {"kind": "focused-one", "slots": [0]}
    elif input_kind == "tui-endpoint":
        input_pop = {
            "kind": "tui-endpoint",
            "note": "TUI flush endpoint; not panel texture present",
        }
    else:
        input_pop = {"kind": "all-run-slots", "n": n}

    if decode_events is None:
        decode_events = [
            _event(ids[i], 3, 100 + i, START + 10_000 + i, "Decode", START + 50_000 + i)
            for i in range(n)
        ]
    if input_events is None:
        surface = "Tui" if frontend == "tui" else "Panel"
        if input_kind == "focused-one":
            input_events = [_event(ids[0], 3, 1, START + 20_000, surface, START + 40_000)]
        elif input_kind in ("all-run-slots", "tui-endpoint"):
            input_events = [
                _event(ids[i], 3, 200 + i, START + 20_000 + i, surface, START + 40_000 + i)
                for i in range(n)
            ]
        else:
            input_events = []
    if losses is None:
        losses = []

    all_recs = list(decode_events) + list(input_events)
    retained = len(all_recs) + len(losses)
    next_cursor = retained

    header = {
        "record": "cohort-header",
        "schema_version": 1,
        "tail_name": "DEFAULT_TAIL_NS",
        "tail_ns": TAIL,
        "observe_ns": OBSERVE,
        "capacity": 512,
        "frontend": frontend,
        "input_population": input_pop,
        "decode_population": {"kind": "all-run-slots", "n": n},
        "boundaries": b,
        "clock_domain": "responsiveness_process_mono",
        "sidecar_path": claimed_path,
    }
    batch = {
        "record": "cohort-batch",
        "schema_version": 1,
        "boundaries": b,
        "records": all_recs,
        "losses": losses,
        "next_cursor": next_cursor,
        "complete": complete,
        "loss_count": loss_count,
        "journal_overflow_n": overflow,
    }
    if overflow:
        batch["available"] = False
        batch["unavailable_reason"] = "journal_overflow"
    terminal = {
        "record": "cohort-terminal",
        "schema_version": 1,
        "boundaries": b,
        "terminal": True,
        "available": terminal_available,
        "records_n": len(all_recs) if records_n is None else records_n,
        "losses_n": loss_count if losses_n is None else losses_n,
        "pending_n": pending_n,
        "tail_name": "DEFAULT_TAIL_NS",
        "producers_joined": producers_joined,
        "observe_end_elapsed_s": observe_end_elapsed_s,
        "frontend": frontend,
    }
    rows = [header, batch, terminal]
    if trailing_after_terminal:
        rows.append(dict(batch))
    return rows


def _win_path(run_name: str, leaf: str = "samples.cohort.jsonl") -> str:
    return f"C:\\ProgramData\\274bot-Test\\archives\\{run_name}\\{leaf}"


class CohortReaderTests(unittest.TestCase):
    def _materialize(self, meta, records, *, n=None, generations=None):
        """Write a complete archived run. ``records[0].sidecar_path`` is rewritten
        to a native Windows path ending in this run directory name.

        Positive fixtures use source-faithful observe (first+final) + drain
        samples with process-mono clocks, capture brackets, and independent
        harness elapsed inside the qualification interval.
        """
        td = tempfile.TemporaryDirectory()
        run = pathlib.Path(td.name)
        n = int(n if n is not None else meta.get("n", 1))
        ids = _ids(n)
        gens = generations or {sid: 3 for sid in ids}
        claimed = _win_path(run.name)

        records = [json.loads(json.dumps(r)) for r in records]  # deep copy
        records[0]["sidecar_path"] = claimed
        (run / "metadata.json").write_text(json.dumps(meta), encoding="utf-8")
        (run / "samples.cohort.jsonl").write_text(
            "\n".join(json.dumps(x) for x in records) + "\n", encoding="utf-8"
        )

        b = records[0]["boundaries"]
        final_cursor = 0
        for rec in records:
            if isinstance(rec, dict) and rec.get("record") == "cohort-batch":
                final_cursor = rec.get("next_cursor", final_cursor)
        ref_sample = {
            "present": True,
            "schema_version": 1,
            "sidecar_path": claimed,
            "cursor": final_cursor,
            "tail_name": "DEFAULT_TAIL_NS",
            "boundaries": b,
            "terminal": None,
        }
        ref_qual_start = {
            "present": True,
            "schema_version": 1,
            "phase_tag": "observe-start",
            "sidecar_path": claimed,
            "tail_name": "DEFAULT_TAIL_NS",
            "boundaries": b,
            "observe_end_elapsed_s": None,
            "qualification_elapsed_s_is_harness_not_cohort_mono": True,
        }
        ref_qual_end = dict(ref_qual_start)
        ref_qual_end["phase_tag"] = "observe-end"
        ref_qual_end["observe_end_elapsed_s"] = HARNESS_OBS_END_S

        def _rows_at(mono: int):
            return [_profile_row(sid, gens[sid], mono) for sid in ids]

        # first observe, mid observe, final observe at END, drain after END.
        samples = [
            {
                "phase": "observe",
                "elapsed_s": FIRST_OBS_ELAPSED_S,
                "frontend": meta.get("frontend"),
                "n": n,
                "responsiveness_profile": _rows_at(FIRST_OBS_MONO),
                "responsiveness_clock": _clock(FIRST_OBS_MONO),
                "cohort": dict(ref_sample),
            },
            {
                "phase": "observe",
                "elapsed_s": MID_OBS_ELAPSED_S,
                "frontend": meta.get("frontend"),
                "n": n,
                "responsiveness_profile": _rows_at(MID_OBS_MONO),
                "responsiveness_clock": _clock(MID_OBS_MONO),
                "cohort": dict(ref_sample),
            },
            {
                "phase": "observe",
                "elapsed_s": FINAL_OBS_ELAPSED_S,
                "frontend": meta.get("frontend"),
                "n": n,
                "responsiveness_profile": _rows_at(FINAL_OBS_MONO),
                "responsiveness_clock": _clock(FINAL_OBS_MONO),
                "cohort": dict(ref_sample),
            },
            {
                "phase": "drain",
                "elapsed_s": DRAIN_ELAPSED_S,
                "frontend": meta.get("frontend"),
                "n": n,
                "responsiveness_profile": _rows_at(DRAIN_MONO),
                "responsiveness_clock": _clock(DRAIN_MONO),
                "cohort": dict(ref_sample),
            },
        ]
        (run / "samples.jsonl").write_text(
            "\n".join(json.dumps(x) for x in samples) + "\n", encoding="utf-8"
        )

        policy = meta.get("render_policy", "focused-one")
        qualification = [
            {
                "phase": "observe-start",
                "elapsed_s": HARNESS_OBS_START_S,
                "slots": _qual_slots(n),
                "settings": _settings(n, meta.get("frontend", "panel"), policy),
                "cohort": ref_qual_start,
            },
            {
                "phase": "observe-end",
                "elapsed_s": HARNESS_OBS_END_S,
                "slots": _qual_slots(n),
                "settings": _settings(n, meta.get("frontend", "panel"), policy),
                "cohort": ref_qual_end,
            },
        ]
        (run / "samples.qualification.jsonl").write_text(
            "\n".join(json.dumps(x) for x in qualification) + "\n", encoding="utf-8"
        )
        return td, run, samples

    def _samples(self, run):
        return [
            json.loads(line)
            for line in (run / "samples.jsonl").read_text(encoding="utf-8").splitlines()
            if line.strip()
        ]

    # --- Happy path / CLI ---

    def test_full_archived_run_analyze_and_require(self):
        n = 16
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n, frontend="panel", input_kind="focused-one")
        td, run, _ = self._materialize(meta, records, n=n)
        try:
            result = rm.analyze_run(run)
            self.assertEqual(
                result["gates"]["decode_cohort"]["target_verdict"],
                "meet",
                result["gates"]["decode_cohort"],
            )
            self.assertEqual(
                result["gates"]["input_cohort"]["target_verdict"],
                "meet",
                result["gates"]["input_cohort"],
            )
            self.assertEqual(result["gates"]["decode_cohort"]["population_n"], 16)
            self.assertEqual(result["gates"]["input_cohort"]["population_n"], 1)
            self.assertIn("decode", result["gates"])
            self.assertIn("input", result["gates"])
            self.assertIs(result["final_acceptance_claim"], False)
            proc = subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "reference_metrics.py"),
                    str(run),
                    "--require",
                    "decode_cohort,input_cohort",
                ],
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)
        finally:
            td.cleanup()

    def test_old_archive_is_legacy_visible_and_cohort_unavailable(self):
        with tempfile.TemporaryDirectory() as td:
            run = pathlib.Path(td)
            (run / "metadata.json").write_text(
                json.dumps({"frontend": "tui", "n": 1, "responsiveness_profile": False})
            )
            (run / "samples.jsonl").write_text(
                json.dumps({"phase": "observe", "elapsed_s": 1}) + "\n"
            )
            result = rm.analyze_run(run)
            self.assertEqual(result["gates"]["decode_cohort"]["status"], "unavailable")
            self.assertEqual(result["gates"]["decode_cohort"]["reason"], "cohort_schema_missing")
            self.assertIn("decode", result["gates"])

    def test_ns_boundary_and_tail_rules(self):
        self.assertEqual(cr.p99_ns([1_100_000], cr.FINE_BOUNDS_MS)["upper_ms"], 2)
        self.assertEqual(cr.p99_ns([100_000_000])["upper_ms"], 100)
        self.assertEqual(
            cr.p99_ns([100_000_001], cr.FINE_BOUNDS_MS)["reason"], "overflow_bucket"
        )
        self.assertEqual(cr.p99_ns([1_000_000], cr.FINE_BOUNDS_MS)["upper_ms"], 1)
        self.assertEqual(cr.p99_ns([1_000_001], cr.FINE_BOUNDS_MS)["upper_ms"], 2)

    # --- Root-reproduced blockers ---

    def test_decode_requires_all_declared_slots_n16(self):
        n = 16
        meta = _meta(n=n)
        ids = _ids(n)
        panel = [_event(ids[0], 3, 1, START + 20_000, "Panel", START + 40_000)]
        dec = [_event(ids[0], 3, 100, START + 10, "Decode", START + 50)]
        records = _build_records(
            "PLACEHOLDER", n=n, decode_events=dec, input_events=panel
        )
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["status"], "unavailable")
            self.assertEqual(result["reason"], "declared_population_incomplete")
        finally:
            td.cleanup()

    def test_counter_only_loss_unavailable(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records(
            "PLACEHOLDER", n=n, loss_count=7, losses_n=7, losses=[]
        )
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["status"], "unavailable")
            self.assertEqual(result["reason"], "cohort_accounting_unavailable")
        finally:
            td.cleanup()

    def test_trailing_batch_after_terminal_unavailable(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n, trailing_after_terminal=True)
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "input")
            self.assertEqual(result["reason"], "record_after_terminal")
        finally:
            td.cleanup()

    def test_malformed_surface_is_unavailable_not_exception(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n)
        records[1]["records"][0]["id"]["surface"] = []
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["status"], "unavailable")
        finally:
            td.cleanup()

    def test_fuzz_malformed_scalar_types_no_exception(self):
        n = 1
        meta = _meta(n=n)
        for bad in (True, False, 1.5, [], {}, "x", None, -1):
            records = _build_records("PLACEHOLDER", n=n)
            records[1]["records"][0]["id"]["slot_id"] = bad
            td, run, samples = self._materialize(meta, records, n=n)
            try:
                result = cr.read_cohort(run, meta, samples, "decode")
                self.assertEqual(result["status"], "unavailable", msg=repr(bad))
            finally:
                td.cleanup()

    # --- Membership / boundaries ---

    def test_completion_in_tail_included(self):
        n = 1
        meta = _meta(n=n)
        sid = _ids(n)[0]
        # Start inside [START,END); complete after END within tail; short duration.
        dec = [_event(sid, 3, 1, END - 500_000, "Decode", END + 500_000)]
        inp = [_event(sid, 3, 2, END - 400_000, "Panel", END + 400_000)]
        records = _build_records(
            "PLACEHOLDER", n=n, decode_events=dec, input_events=inp
        )
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            self.assertEqual(
                cr.read_cohort(run, meta, samples, "decode")["target_verdict"], "meet"
            )
            self.assertEqual(
                cr.read_cohort(run, meta, samples, "input")["target_verdict"], "meet"
            )
        finally:
            td.cleanup()
        # Equality at END+TAIL is a valid completion timestamp (membership).
        dec2 = [_event(sid, 3, 1, END - 1_000_000, "Decode", END + TAIL)]
        # duration is huge → p99 overflow, but must not be malformed_completion
        records = _build_records(
            "PLACEHOLDER", n=n, decode_events=dec2, input_events=inp
        )
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertNotEqual(result["reason"], "malformed_completion_timestamp")
            self.assertEqual(result["reason"], "overflow_bucket")
        finally:
            td.cleanup()

    def test_start_at_end_excluded_and_pre_start_malformed(self):
        n = 1
        meta = _meta(n=n)
        sid = _ids(n)[0]
        for start_ns in (END, START - 1):
            dec = [_event(sid, 3, 1, start_ns, "Decode", END + 10)]
            records = _build_records(
                "PLACEHOLDER", n=n, decode_events=dec, input_events=[]
            )
            td, run, samples = self._materialize(meta, records, n=n)
            try:
                result = cr.read_cohort(run, meta, samples, "decode")
                self.assertEqual(result["reason"], "event_start_out_of_window")
            finally:
                td.cleanup()

    def test_late_completion_beyond_tail_unavailable(self):
        n = 1
        meta = _meta(n=n)
        sid = _ids(n)[0]
        dec = [_event(sid, 3, 1, START + 1, "Decode", END + TAIL + 1)]
        records = _build_records(
            "PLACEHOLDER", n=n, decode_events=dec, input_events=[]
        )
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["reason"], "malformed_completion_timestamp")
        finally:
            td.cleanup()

    def test_start_ids_out_of_order_valid(self):
        n = 1
        meta = _meta(n=n)
        sid = _ids(n)[0]
        dec = [
            _event(sid, 3, 99, START + 50, "Decode", START + 60),
            _event(sid, 3, 1, START + 10, "Decode", START + 20),
        ]
        inp = [_event(sid, 3, 2, START + 30, "Panel", START + 40)]
        records = _build_records(
            "PLACEHOLDER", n=n, decode_events=dec, input_events=inp
        )
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["target_verdict"], "meet", result)
        finally:
            td.cleanup()

    def test_canceled_member_unavailable(self):
        n = 1
        meta = _meta(n=n)
        sid = _ids(n)[0]
        dec = [_event(sid, 3, 1, START + 1, "Decode", None, outcome="Canceled")]
        records = _build_records(
            "PLACEHOLDER", n=n, decode_events=dec, input_events=[]
        )
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["status"], "unavailable")
            self.assertIn(
                result["reason"],
                {"noncompleted_member", "declared_population_incomplete"},
            )
        finally:
            td.cleanup()

    def test_overflow_batch_unavailable(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records(
            "PLACEHOLDER", n=n, overflow=3, terminal_available=False
        )
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["reason"], "cohort_accounting_unavailable")
        finally:
            td.cleanup()

    def test_missing_terminal_unavailable(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n)[:2]
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["reason"], "cohort_terminal_missing")
        finally:
            td.cleanup()

    def test_finalized_unavailable_pending0(self):
        n = 1
        meta = _meta(n=n)
        sid = _ids(n)[0]
        losses = [
            {
                "sequence": 9,
                "slot_id": sid,
                "generation": 3,
                "start_mono_ns": START + 1,
                "surface": "Decode",
                "outcome": "Lost",
                "reason": "Incomplete",
            },
            {
                "sequence": 10,
                "slot_id": sid,
                "generation": 3,
                "start_mono_ns": START + 2,
                "surface": "Decode",
                "outcome": "Lost",
                "reason": "Incomplete",
            },
        ]
        records = _build_records(
            "PLACEHOLDER",
            n=n,
            terminal_available=False,
            pending_n=0,
            loss_count=2,
            losses_n=2,
            losses=losses,
        )
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["status"], "unavailable")
            self.assertEqual(result["reason"], "cohort_accounting_unavailable")
        finally:
            td.cleanup()

    def test_duplicate_batch_identities_unavailable(self):
        n = 1
        meta = _meta(n=n)
        sid = _ids(n)[0]
        e = _event(sid, 3, 1, START + 1, "Decode", START + 10)
        records = _build_records(
            "PLACEHOLDER", n=n, decode_events=[e, e], input_events=[]
        )
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["reason"], "duplicate_event_identity")
        finally:
            td.cleanup()

    def test_cursor_gap_and_stale_cursor_unavailable(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n)
        records[1]["next_cursor"] = 999
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["reason"], "cursor_accounting_mismatch")
        finally:
            td.cleanup()

        records = _build_records("PLACEHOLDER", n=n)
        records[1]["next_cursor"] = 0
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["reason"], "cursor_accounting_mismatch")
        finally:
            td.cleanup()

    def test_missing_generation(self):
        n = 2
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n)
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            # Cohort-bearing profiles present but generation field stripped →
            # not invented; empty profiles are separately sample_slot_disappeared.
            for s in samples:
                for row in s["responsiveness_profile"]:
                    del row["generation"]
            (run / "samples.jsonl").write_text(
                "\n".join(json.dumps(x) for x in samples) + "\n"
            )
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["reason"], "sample_slot_generation_malformed")
        finally:
            td.cleanup()

    def test_empty_profile_all_samples_disappeared(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n)
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            for s in samples:
                s["responsiveness_profile"] = []
            (run / "samples.jsonl").write_text(
                "\n".join(json.dumps(x) for x in samples) + "\n"
            )
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["status"], "unavailable")
            self.assertIn(
                result["reason"],
                ("sample_slot_disappeared", "sample_generations_missing"),
            )
        finally:
            td.cleanup()

    def test_no_input_samples_population_incomplete(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n, input_events=[])
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "input")
            self.assertEqual(result["reason"], "declared_population_incomplete")
        finally:
            td.cleanup()

    def test_basename_only_path_rejected(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n)
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            # Overwrite with basename-only claims
            records = json.loads(
                "[" + ",".join(
                    (run / "samples.cohort.jsonl").read_text().splitlines()
                ) + "]"
            ) if False else None
            raw = [
                json.loads(line)
                for line in (run / "samples.cohort.jsonl").read_text().splitlines()
                if line.strip()
            ]
            raw[0]["sidecar_path"] = "samples.cohort.jsonl"
            (run / "samples.cohort.jsonl").write_text(
                "\n".join(json.dumps(x) for x in raw) + "\n"
            )
            for s in samples:
                s["cohort"]["sidecar_path"] = "samples.cohort.jsonl"
            (run / "samples.jsonl").write_text(
                "\n".join(json.dumps(x) for x in samples) + "\n"
            )
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["reason"], "sidecar_provenance_mismatch")
        finally:
            td.cleanup()

    def test_relative_path_rejected(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n)
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            claimed = f"{run.name}/samples.cohort.jsonl"
            raw = [
                json.loads(line)
                for line in (run / "samples.cohort.jsonl").read_text().splitlines()
                if line.strip()
            ]
            raw[0]["sidecar_path"] = claimed
            (run / "samples.cohort.jsonl").write_text(
                "\n".join(json.dumps(x) for x in raw) + "\n"
            )
            for s in samples:
                s["cohort"]["sidecar_path"] = claimed
            (run / "samples.jsonl").write_text(
                "\n".join(json.dumps(x) for x in samples) + "\n"
            )
            quals = [
                json.loads(line)
                for line in (run / "samples.qualification.jsonl").read_text().splitlines()
                if line.strip()
            ]
            for q in quals:
                q["cohort"]["sidecar_path"] = claimed
            (run / "samples.qualification.jsonl").write_text(
                "\n".join(json.dumps(x) for x in quals) + "\n"
            )
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["reason"], "sidecar_provenance_mismatch")
        finally:
            td.cleanup()

    def test_profile_fine_required(self):
        n = 1
        meta = _meta(n=n, responsiveness_fine=False)
        records = _build_records("PLACEHOLDER", n=n)
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["reason"], "responsiveness_fine_disabled")
        finally:
            td.cleanup()

    def test_slow_slot_not_pooled_away(self):
        n = 2
        meta = _meta(n=n)
        ids = _ids(n)
        dec = [
            _event(ids[0], 3, 1, START + 1, "Decode", START + 1_000_000),
            _event(ids[1], 3, 2, START + 2, "Decode", START + 2 + 150_000_000),
        ]
        inp = [_event(ids[0], 3, 3, START + 3, "Panel", START + 4_000_000)]
        records = _build_records(
            "PLACEHOLDER", n=n, decode_events=dec, input_events=inp
        )
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["status"], "unavailable")
            self.assertEqual(result["reason"], "overflow_bucket")
        finally:
            td.cleanup()

    def test_header_only_unavailable(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n)[:1]
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["reason"], "cohort_terminal_missing")
        finally:
            td.cleanup()

    def test_producers_not_joined(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n, producers_joined=False)
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["reason"], "producers_not_joined")
        finally:
            td.cleanup()

    def test_final_batch_must_be_complete(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n, complete=False)
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["reason"], "final_batch_incomplete")
        finally:
            td.cleanup()

    def test_focused_one_maps_ordinal_zero_not_slot_id_zero(self):
        n = 16
        meta = _meta(n=n)
        ids = _ids(n)
        self.assertNotEqual(ids[0], 0)
        records = _build_records("PLACEHOLDER", n=n)
        panel = [r for r in records[1]["records"] if r["id"]["surface"] == "Panel"]
        self.assertEqual(panel[0]["id"]["slot_id"], ids[0])
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "input")
            self.assertEqual(result["target_verdict"], "meet", result)
            self.assertEqual(result["population_n"], 1)
            dec = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(dec["target_verdict"], "meet", dec)
            self.assertEqual(dec["population_n"], 16)
        finally:
            td.cleanup()

    def test_qualification_shape_is_slots_not_qualification_slot_rows(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n)
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            quals = [
                json.loads(line)
                for line in (run / "samples.qualification.jsonl").read_text().splitlines()
                if line.strip()
            ]
            for q in quals:
                for s in q["slots"]:
                    s["generation"] = 3
            (run / "samples.qualification.jsonl").write_text(
                "\n".join(json.dumps(x) for x in quals) + "\n"
            )
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["reason"], "qualification_unexpected_generation")
        finally:
            td.cleanup()

    def test_fnv_slot_id_matches_rust(self):
        self.assertEqual(cr.slot_id_for(""), 0xCBF29CE484222325)
        self.assertNotEqual(cr.slot_id_for("cohort-slot-00"), 0)

    def test_bool_not_accepted_as_uint_in_boundaries(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n)
        records[0]["boundaries"]["start_mono_ns"] = True
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["status"], "unavailable")
        finally:
            td.cleanup()

    def test_pre_arm_seed_generation_ignored_observe_generation_used(self):
        """Seed gen1 ended then observe gen2 with cohort refs must not reset-fail."""
        n = 1
        meta = _meta(n=n)
        sid = _ids(n)[0]
        dec = [_event(sid, 2, 100, START + 10_000, "Decode", START + 50_000)]
        inp = [_event(sid, 2, 1, START + 20_000, "Panel", START + 40_000)]
        records = _build_records(
            "PLACEHOLDER", n=n, decode_events=dec, input_events=inp
        )
        td, run, samples = self._materialize(
            meta, records, n=n, generations={sid: 2}
        )
        try:
            # Prepend seed/warmup samples WITHOUT cohort refs (pre-arm lifecycle).
            seed_rows = [
                {
                    "phase": "seed",
                    "elapsed_s": 1.0,
                    "frontend": "panel",
                    "n": n,
                    "responsiveness_profile": [
                        {"slot_id": sid, "generation": 1, "ended": False}
                    ],
                },
                {
                    "phase": "seed",
                    "elapsed_s": 4.0,
                    "frontend": "panel",
                    "n": n,
                    "responsiveness_profile": [
                        {"slot_id": sid, "generation": 1, "ended": True}
                    ],
                },
                {
                    "phase": "warmup",
                    "elapsed_s": 25.0,
                    "frontend": "panel",
                    "n": n,
                    "responsiveness_profile": [
                        {"slot_id": sid, "generation": 2, "ended": False}
                    ],
                },
            ]
            # Existing observe sample already has cohort + gen 2.
            full = seed_rows + samples
            (run / "samples.jsonl").write_text(
                "\n".join(json.dumps(x) for x in full) + "\n", encoding="utf-8"
            )
            result = cr.read_cohort(run, meta, full, "decode")
            self.assertEqual(result.get("target_verdict"), "meet", result)
            result_in = cr.read_cohort(run, meta, full, "input")
            self.assertEqual(result_in.get("target_verdict"), "meet", result_in)
        finally:
            td.cleanup()

    def test_in_cohort_generation_reset_unavailable(self):
        """Two generations on cohort-bearing samples fail closed."""
        n = 1
        meta = _meta(n=n)
        sid = _ids(n)[0]
        dec = [_event(sid, 2, 100, START + 10_000, "Decode", START + 50_000)]
        inp = [_event(sid, 2, 1, START + 20_000, "Panel", START + 40_000)]
        records = _build_records(
            "PLACEHOLDER", n=n, decode_events=dec, input_events=inp
        )
        td, run, samples = self._materialize(
            meta, records, n=n, generations={sid: 2}
        )
        try:
            # Flip generation on an existing mid-window observe sample.
            full = json.loads(json.dumps(samples))
            full[1]["responsiveness_profile"] = [
                _profile_row(sid, 3, MID_OBS_MONO)
            ]
            (run / "samples.jsonl").write_text(
                "\n".join(json.dumps(x) for x in full) + "\n", encoding="utf-8"
            )
            result = cr.read_cohort(run, meta, full, "decode")
            self.assertEqual(result["reason"], "sample_generation_reset")
        finally:
            td.cleanup()

    def test_quiet_inner_span_not_selected(self):
        """Fixed header boundaries are authoritative; a slow start-of-window
        member cannot be dropped to invent a quieter interior span."""
        n = 1
        meta = _meta(n=n)
        sid = _ids(n)[0]
        # One slow event right after START (would be outside a quieter interior).
        dec = [
            _event(sid, 3, 1, START + 1, "Decode", START + 1 + 150_000_000),  # 150ms
            _event(sid, 3, 2, START + 500_000_000, "Decode", START + 500_000_000 + 1_000_000),
        ]
        inp = [_event(sid, 3, 3, START + 20_000, "Panel", START + 40_000)]
        records = _build_records(
            "PLACEHOLDER", n=n, decode_events=dec, input_events=inp
        )
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "decode")
            # Slow member retained → overflow / unavailable, not meet via quiet trim.
            self.assertNotEqual(result.get("target_verdict"), "meet")
            self.assertEqual(result["status"], "unavailable")
            self.assertIn(result.get("reason"), ("overflow_bucket", "slot_p99_unavailable"))
        finally:
            td.cleanup()

    # --- R2 provenance false-meet probes (must stay unavailable) ---

    def test_r2_qual_fine_false_while_meta_true(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n)
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            qual_path = run / "samples.qualification.jsonl"
            rows = [json.loads(line) for line in qual_path.read_text().splitlines() if line.strip()]
            for row in rows:
                row["settings"]["responsiveness_fine_enabled"] = False
            qual_path.write_text("\n".join(json.dumps(x) for x in rows) + "\n")
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["status"], "unavailable")
            self.assertEqual(result["reason"], "qualification_fine_disabled")
            self.assertNotEqual(result.get("target_verdict"), "meet")
        finally:
            td.cleanup()

    def test_r2_observe_end_qualification_missing(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n)
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            qual_path = run / "samples.qualification.jsonl"
            rows = [json.loads(line) for line in qual_path.read_text().splitlines() if line.strip()]
            rows = [r for r in rows if r.get("phase") != "observe-end"]
            qual_path.write_text("\n".join(json.dumps(x) for x in rows) + "\n")
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["status"], "unavailable")
            self.assertEqual(result["reason"], "qualification_observe_end_missing")
            self.assertNotEqual(result.get("target_verdict"), "meet")
        finally:
            td.cleanup()

    def test_r2_cohort_bearing_sample_ended_true(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n)
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            samples = json.loads(json.dumps(samples))
            samples[0]["responsiveness_profile"][0]["ended"] = True
            (run / "samples.jsonl").write_text(
                "\n".join(json.dumps(x) for x in samples) + "\n"
            )
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["status"], "unavailable")
            self.assertEqual(result["reason"], "sample_slot_ended")
            self.assertNotEqual(result.get("target_verdict"), "meet")
        finally:
            td.cleanup()

    def test_r2_slot_disappearance_empty_profile(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n)
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            samples = json.loads(json.dumps(samples))
            # Empty profile on an existing cohort-bearing observe sample.
            samples[1]["responsiveness_profile"] = []
            (run / "samples.jsonl").write_text(
                "\n".join(json.dumps(x) for x in samples) + "\n"
            )
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["status"], "unavailable")
            self.assertEqual(result["reason"], "sample_slot_disappeared")
            self.assertNotEqual(result.get("target_verdict"), "meet")
        finally:
            td.cleanup()

    def test_r2_forged_sample_cohort_cursor(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n)
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            samples = json.loads(json.dumps(samples))
            samples[0]["cohort"]["cursor"] = 99999
            (run / "samples.jsonl").write_text(
                "\n".join(json.dumps(x) for x in samples) + "\n"
            )
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["status"], "unavailable")
            self.assertIn(
                result["reason"],
                ("sample_cursor_mismatch", "sample_cursor_hwm_mismatch"),
            )
            self.assertNotEqual(result.get("target_verdict"), "meet")
        finally:
            td.cleanup()

    def test_r2_observe_end_profile_disabled(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n)
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            qual_path = run / "samples.qualification.jsonl"
            rows = [json.loads(line) for line in qual_path.read_text().splitlines() if line.strip()]
            for row in rows:
                if row.get("phase") == "observe-end":
                    row["settings"]["responsiveness_profile_enabled"] = False
            qual_path.write_text("\n".join(json.dumps(x) for x in rows) + "\n")
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["status"], "unavailable")
            # Settings disagree across boundaries, or explicit profile disabled.
            self.assertIn(
                result["reason"],
                ("qualification_settings_disagree", "qualification_profile_disabled"),
            )
            self.assertNotEqual(result.get("target_verdict"), "meet")
        finally:
            td.cleanup()

    def test_r2_interior_observe_missing_cohort_ref(self):
        """Missing cohort on an interior observe row must not hide lifetime faults."""
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n)
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            samples = json.loads(json.dumps(samples))
            # Strip cohort from mid observe (between first and final).
            bare = dict(samples[1])
            bare.pop("cohort", None)
            full = [samples[0], bare, samples[2], samples[3]]
            (run / "samples.jsonl").write_text(
                "\n".join(json.dumps(x) for x in full) + "\n"
            )
            result = cr.read_cohort(run, meta, full, "decode")
            self.assertEqual(result["status"], "unavailable")
            self.assertEqual(result["reason"], "cohort_sample_ref_missing_interior")
        finally:
            td.cleanup()

    # --- Root gap probes (endpoint / u64 / clock) ---

    def test_headless_frontend_unavailable(self):
        """Unsupported frontend must not meet with Panel events."""
        n = 2
        meta = _meta(n=n, frontend="headless")
        records = _build_records("PLACEHOLDER", n=n, frontend="headless")
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            for gate in ("input", "decode"):
                result = cr.read_cohort(run, meta, samples, gate)
                self.assertEqual(result["status"], "unavailable", gate)
                self.assertEqual(result["reason"], "unsupported_frontend", gate)
                self.assertNotEqual(result.get("target_verdict"), "meet", gate)
        finally:
            td.cleanup()

    def test_focused_wrong_ordinal_unavailable(self):
        """pins_focus always ordinal 0; slots:[1] is forged."""
        n = 2
        meta = _meta(n=n)
        ids = _ids(n)
        records = _build_records("PLACEHOLDER", n=n)
        records[0]["input_population"] = {"kind": "focused-one", "slots": [1]}
        for batch in records:
            if batch.get("record") == "cohort-batch":
                for ev in batch["records"]:
                    if ev["id"]["surface"] == "Panel":
                        ev["id"]["slot_id"] = ids[1]
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "input")
            self.assertEqual(result["status"], "unavailable")
            self.assertEqual(result["reason"], "input_population_ordinal_mismatch")
            self.assertNotEqual(result.get("target_verdict"), "meet")
        finally:
            td.cleanup()

    def test_sequence_over_u64_unavailable(self):
        """Rust u64 cannot emit sequence=2**64."""
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n)
        for batch in records:
            if batch.get("record") == "cohort-batch":
                for ev in batch["records"]:
                    ev["id"]["sequence"] = 2**64
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "input")
            self.assertEqual(result["status"], "unavailable")
            self.assertIn(
                result["reason"],
                ("malformed_event_identity", "malformed_cohort_input"),
            )
            self.assertNotEqual(result.get("target_verdict"), "meet")
        finally:
            td.cleanup()

    def test_sample_clock_missing_unavailable(self):
        """Cohort-bearing samples without responsiveness_clock fail closed."""
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n)
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            samples = json.loads(json.dumps(samples))
            for s in samples:
                s.pop("responsiveness_clock", None)
            (run / "samples.jsonl").write_text(
                "\n".join(json.dumps(x) for x in samples) + "\n"
            )
            result = cr.read_cohort(run, meta, samples, "input")
            self.assertEqual(result["status"], "unavailable")
            self.assertEqual(result["reason"], "sample_clock_missing")
            self.assertNotEqual(result.get("target_verdict"), "meet")
        finally:
            td.cleanup()

    def test_inverted_clock_bracket_unavailable(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n)
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            samples = json.loads(json.dumps(samples))
            clk = samples[0]["responsiveness_clock"]
            clk["sample_mono_ns_lower"], clk["sample_mono_ns_upper"] = (
                clk["sample_mono_ns_upper"],
                clk["sample_mono_ns_lower"],
            )
            (run / "samples.jsonl").write_text(
                "\n".join(json.dumps(x) for x in samples) + "\n"
            )
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["status"], "unavailable")
            self.assertEqual(result["reason"], "sample_clock_bracket_inverted")
        finally:
            td.cleanup()

    def test_clock_domain_mismatch_unavailable(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n)
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            samples = json.loads(json.dumps(samples))
            samples[0]["responsiveness_clock"]["domain"] = "wall_clock"
            (run / "samples.jsonl").write_text(
                "\n".join(json.dumps(x) for x in samples) + "\n"
            )
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["status"], "unavailable")
            self.assertEqual(result["reason"], "sample_clock_domain_mismatch")
        finally:
            td.cleanup()

    def test_pins_focus_fixed_one_and_background_meet(self):
        """fixed-one / focused-plus-background emit focused-one slots:[0]."""
        for policy in ("fixed-one", "focused-plus-background"):
            n = 2
            meta = _meta(n=n, render_policy=policy)
            records = _build_records(
                "PLACEHOLDER", n=n, frontend="panel", input_kind="focused-one"
            )
            td, run, samples = self._materialize(meta, records, n=n)
            try:
                result = cr.read_cohort(run, meta, samples, "input")
                self.assertEqual(result.get("target_verdict"), "meet", (policy, result))
                self.assertEqual(result.get("population_n"), 1, policy)
            finally:
                td.cleanup()

    def test_tui_endpoint_distinct_from_panel(self):
        n = 2
        meta = _meta(n=n, frontend="tui", render_policy="rotating-all")
        records = _build_records(
            "PLACEHOLDER", n=n, frontend="tui", input_kind="tui-endpoint"
        )
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            result = cr.read_cohort(run, meta, samples, "input")
            self.assertEqual(result.get("target_verdict"), "meet", result)
            self.assertEqual(result.get("population_n"), 2)
            # Panel surface with tui frontend must not meet input.
            bad = _build_records(
                "PLACEHOLDER", n=n, frontend="tui", input_kind="tui-endpoint"
            )
            for batch in bad:
                if batch.get("record") == "cohort-batch":
                    for ev in batch["records"]:
                        if ev["id"]["surface"] == "Tui":
                            ev["id"]["surface"] = "Panel"
            td2, run2, s2 = self._materialize(meta, bad, n=n)
            try:
                r2 = cr.read_cohort(run2, meta, s2, "input")
                self.assertEqual(r2["status"], "unavailable")
                self.assertNotEqual(r2.get("target_verdict"), "meet")
            finally:
                td2.cleanup()
        finally:
            td.cleanup()

    def test_capture_bracket_missing_unavailable(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n)
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            samples = json.loads(json.dumps(samples))
            for row in samples[0]["responsiveness_profile"]:
                row.pop("decode_capture_mono_ns_lower", None)
                row.pop("decode_capture_mono_ns_upper", None)
            (run / "samples.jsonl").write_text(
                "\n".join(json.dumps(x) for x in samples) + "\n"
            )
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["status"], "unavailable")
            self.assertEqual(result["reason"], "capture_bracket_missing")
        finally:
            td.cleanup()

    def test_observe_final_misaligned_unavailable(self):
        n = 1
        meta = _meta(n=n)
        records = _build_records("PLACEHOLDER", n=n)
        td, run, samples = self._materialize(meta, records, n=n)
        try:
            samples = json.loads(json.dumps(samples))
            # Move final observe mono away from END so it no longer encloses END.
            bad_mono = END - 10_000_000_000
            samples[2]["responsiveness_clock"] = _clock(bad_mono)
            samples[2]["responsiveness_profile"] = [
                _profile_row(_ids(n)[0], 3, bad_mono)
            ]
            (run / "samples.jsonl").write_text(
                "\n".join(json.dumps(x) for x in samples) + "\n"
            )
            result = cr.read_cohort(run, meta, samples, "decode")
            self.assertEqual(result["status"], "unavailable")
            self.assertEqual(result["reason"], "observe_final_missing_or_misaligned")
        finally:
            td.cleanup()


if __name__ == "__main__":
    unittest.main()
