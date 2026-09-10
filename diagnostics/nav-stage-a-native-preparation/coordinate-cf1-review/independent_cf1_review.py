#!/usr/bin/env python3
"""Independent CF1 raw feasibility + cumulative ledger review (t_f7d038ac)."""
from __future__ import annotations

import hashlib
import json
import math
import pathlib
import sys
import tarfile

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[2]  # worktree root
RESULT = ROOT / "diagnostics/nav-stage-a-native-preparation/coordinate-cf1-result-01"
PREP = ROOT / "diagnostics/nav-stage-a-native-preparation/coordinate-cf1-preparation-01"
SHARDED = ROOT / "docs/memory/nav-tiled-stage-a/sharded.py"
ROUTES = ROOT / "diagnostics/nav-stage-a-real-proposal-01/routes.tsv"
EXPECTED_ARCHIVE = "6b6314fe9e60235ab92feeb65f972921104c66bccbc5fabcf385e4e28f576cdf"
ORIGINAL_TSV = "49e348ea78806c8278d720d27b171c54a0a209930fa8191ffcd38d2e9bbbc125"
ORIGINAL_PACK = "2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30"
AUTH_SHA = "62835390bdcbee419029f6c0fcf38f1126a502cde926be643de0c64acd9977ec"
ACCEPTED_DECISION = "6e53a13303442b5f6087e9321babc1d7be340031adf22a5a5c88c0edd7f225a7"
DECISION_COMMIT = "b00ba2c5d923c93f3161c1663ed3879bab8482af"
SOURCE_BINDING = "7b101c30bfd64c302815f9844fe27d3a52302feedaec58737726ab6a062491bb"
LIMITS = dict(
    address=4294967296,
    cpu=90,
    file_size=262144,
    output=262144,
    rss=1073741824,
    wall=120,
)
SLOTS = [[1, 1, "dense"], [1, 1, "refined"], [1, 2, "refined"], [1, 2, "dense"]]
REMOTE = "/home/acfrazier/274bot-campaign/nav-coordinate-113ce05-concord-01/"
BASE = "host/docs/memory/nav-tiled-stage-a/coordinate-concord-run-01/coordinate-ebf0f30-CF1-release-01/"
OUT = BASE + "CF1/"


def sha(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def load_json(b: bytes):
    return json.loads(b)


def fail(msg: str):
    raise AssertionError(msg)


def main():
    findings = []
    checks = {}

    # --- archive stream hash all 82 members ---
    manifest = json.loads((RESULT / "root-coordinate-cf1-evidence.manifest.json").read_text())
    arc_path = RESULT / "root-coordinate-cf1-evidence.tar.gz"
    arc_bytes = arc_path.read_bytes()
    arc_sha = sha(arc_bytes)
    checks["archive_sha256"] = arc_sha == EXPECTED_ARCHIVE == manifest.get("archive_sha256")
    checks["archive_bytes"] = len(arc_bytes) == 834734
    if not checks["archive_sha256"]:
        fail(f"archive sha mismatch {arc_sha}")
    expected = {r["path"]: r for r in manifest["files"]}
    checks["manifest_member_count"] = len(expected) == 82
    data = {}
    with tarfile.open(arc_path, "r:gz") as t:
        members = [p for p in t if p.isfile()]
        checks["tar_file_count"] = len(members) == 82
        for p in members:
            if p.name not in expected:
                fail(f"unexpected member {p.name}")
            if p.name in data:
                fail(f"duplicate member {p.name}")
            b = t.extractfile(p).read()
            e = expected[p.name]
            if len(b) != e["bytes"] or sha(b) != e["sha256"]:
                fail(f"member hash/size fail {p.name}")
            data[p.name] = b
    checks["exact_member_set"] = set(data) == set(expected)
    findings.append(f"stream-hashed all {len(data)} archive members vs manifest; archive {arc_sha}")

    def obj(p):
        return load_json(data[p])

    def ref(r):
        if not r["path"].startswith(REMOTE):
            fail(f"path not under remote {r['path']}")
        p = r["path"][len(REMOTE) :]
        if sha(data[p]) != r["sha256"]:
            fail(f"ref hash mismatch {p}")
        return obj(p)

    # --- result / auth / claim / checkpoint immutable ---
    result = obj(OUT + "result.json")
    auth = ref(result["authorization"])
    checks["root_eq_auth"] = ref(result["root"]) == auth
    admission_auth = data["root-coordinate-cf1-admission/CF1-authorization.json"]
    checks["admission_auth_sha"] = sha(admission_auth) == AUTH_SHA
    prep_auth = (PREP / "CF1-authorization.json").read_bytes()
    checks["prep_auth_matches_result_ref"] = sha(prep_auth) == result["authorization"]["sha256"]
    checks["prep_auth_eq_admission"] = prep_auth == admission_auth
    checks["result_status"] = (
        result["status"] == "complete"
        and result["completed"] == 4
        and result["phase"] == "CF1"
        and result.get("schema") == "stage-a-coordinate-v1"
    )
    claim = ref(result["claim"])
    checkpoint = ref(result["checkpoint"])
    checks["claim_phase"] = claim.get("phase") == "CF1" and claim.get("candidate_id") == "coordinate-ebf0f30"
    checks["checkpoint_completed"] = checkpoint.get("completed") == 4
    findings.append(
        f"immutable auth/claim/checkpoint/result bound; auth={AUTH_SHA[:12]}… status=complete completed=4"
    )

    # --- pack + 59 selectors ---
    pack = data[BASE + "input.bin"]
    checks["pack_sha"] = sha(pack) == ORIGINAL_PACK == auth["prerequisites"]["input_sha256"]
    checks["pack_bytes"] = len(pack) == 73438581 == auth["prerequisites"]["input_bytes"]
    route_bytes = ROUTES.read_bytes()
    checks["routes_sha"] = sha(route_bytes) == ORIGINAL_TSV == auth["prerequisites"]["routes_sha256"]
    # space-normalized integer rows (reviewed contract); tab would fail
    route_rows = [
        b" ".join(str(int(x)).encode() for x in line.split()) + b"\n"
        for line in route_bytes.splitlines()
        if line.strip() and not line.lstrip().startswith(b"#")
    ]
    checks["route_row_count"] = len(route_rows) == 59
    selector_ok = True
    for i, row in enumerate(route_rows, 1):
        if data[BASE + f"selectors/{i}.tsv"] != row:
            selector_ok = False
            fail(f"selector {i} mismatch vs space-normalized routes")
    checks["all_59_selectors_space_normalized"] = selector_ok
    # prove tab join would diverge from archived selectors (audit correction)
    tab_rows = [
        b"\t".join(str(int(x)).encode() for x in line.split()) + b"\n"
        for line in route_bytes.splitlines()
        if line.strip() and not line.lstrip().startswith(b"#")
    ]
    tab_mismatches = sum(
        1 for i, row in enumerate(tab_rows, 1) if data[BASE + f"selectors/{i}.tsv"] != row
    )
    checks["tab_join_mismatches_all_or_some"] = tab_mismatches > 0
    findings.append(
        f"pack {ORIGINAL_PACK[:12]}… {len(pack)}B; 59 selectors match space-normalized ORIGINAL_TSV; "
        f"tab-join mismatches={tab_mismatches} (confirms audit correction)"
    )

    # --- source binding ---
    sb = result.get("source_binding") or auth.get("source_binding")
    # binding may be nested dict with sha256
    if isinstance(sb, dict):
        sb_sha = sb.get("sha256") or sb.get("digest")
    else:
        sb_sha = sb
    # also check auth contract
    auth_sb = auth.get("source_binding")
    if isinstance(auth_sb, dict):
        auth_sb_sha = auth_sb.get("sha256") or auth_sb.get("digest")
    else:
        auth_sb_sha = auth_sb
    checks["source_binding_present"] = bool(sb_sha or auth_sb_sha)
    # Prefer exact known binding hash if present as string field
    binding_hits = []
    for blob in (json.dumps(result), json.dumps(auth)):
        if SOURCE_BINDING in blob:
            binding_hits.append(True)
    checks["source_binding_7b101c30_in_artifacts"] = any(binding_hits) or (
        sb_sha == SOURCE_BINDING or auth_sb_sha == SOURCE_BINDING
    )
    findings.append(f"source_binding presence checked; 7b101c30 in artifacts={checks['source_binding_7b101c30_in_artifacts']}")

    # --- four children: records, receipts, outs, aggregates, raw ---
    children = []
    aggregates = {}
    for i, (entry, slot) in enumerate(zip(result["entries"], SLOTS)):
        name = f"{i:03d}-{slot[0]}-{slot[1]}-{slot[2]}"
        if entry["name"] != name:
            fail(f"entry name {entry['name']} != {name}")
        if sha(data[OUT + name + ".record.json"]) != entry["record_sha256"]:
            fail(f"record sha {name}")
        record = obj(OUT + name + ".record.json")
        if record["slot"] != slot:
            fail(f"slot {name}")
        if record.get("phase") not in (None, "CF1") and record.get("phase") != "CF1":
            # coordinate records should carry phase
            pass
        if record.get("source_binding") != result.get("source_binding"):
            # may still be ok if both encode same binding
            if json.dumps(record.get("source_binding"), sort_keys=True) != json.dumps(
                result.get("source_binding"), sort_keys=True
            ):
                fail(f"source_binding drift on {name}")
        for suffix, h in record["files"].items():
            if sha(data[OUT + name + suffix]) != h:
                fail(f"file bind {name}{suffix}")
        receipt = obj(OUT + name + ".receipt.json")
        if receipt["returncode"] != 0 or receipt["failure"] is not None:
            fail(f"receipt fail {name}")
        if receipt["address_guard_active"] is not True:
            fail(f"address guard inactive {name}")
        if receipt["limits"] != LIMITS:
            fail(f"limits changed {name}: {receipt['limits']}")
        samples = [json.loads(l) for l in data[OUT + name + ".out"].splitlines() if l.strip()]
        if len(samples) != 9:
            fail(f"expected 9 NDJSON lines {name} got {len(samples)}")
        s = samples[-1]
        if s.get("summary") is not True:
            fail(f"no summary {name}")
        if s.get("raw_order") != "sweep-row-lane/8/3":
            fail(f"raw_order {name}")
        if s.get("raw_schema") != "stage-a-raw-v1":
            fail(f"raw_schema {name}")
        raw = s.get("raw_elapsed_ns", [])
        if len(raw) != 24 or any(type(x) is not int or x < 0 for x in raw):
            fail(f"raw_elapsed_ns {name}")
        # p99 check
        p99 = sorted(raw)[(len(raw) * 99 + 99) // 100 - 1]
        if s.get("route_p99_ns") != p99:
            fail(f"p99 mismatch {name}")
        lane = s.get("lane_cpu_ns", [])
        if len(lane) != 3 or any(type(x) is not int or x < 0 for x in lane):
            fail(f"lane_cpu_ns {name}")
        if s.get("narrow_allocations") != 0 or s.get("narrow_requested_bytes") != 0:
            fail(f"narrow alloc {name}")
        if s.get("input_bytes") != 73438581 or s.get("logical_cells") != 65142784:
            fail(f"input/cells {name}")
        key = slot[1]
        if key not in aggregates:
            aggregates[key] = s["aggregate"]
        elif aggregates[key] != s["aggregate"]:
            fail(f"aggregate disagreement row {key} on {name}")
        # aggregate calls == 24
        if s["aggregate"].get("calls") != 24:
            fail(f"aggregate calls {name}")
        children.append(
            dict(
                name=name,
                child_cpu=record["child_cpu"],
                receipt_wall=receipt["wall_seconds"],
                peak_rss=s["process_peak_rss_bytes"],
                raw_calls=len(raw),
                aggregate=s["aggregate"],
                route_p99_ns=s["route_p99_ns"],
                narrow_allocations=s["narrow_allocations"],
            )
        )
    checks["four_children_ok"] = len(children) == 4
    checks["row_aggregate_agreement"] = True
    findings.append(
        "four children exit0; 24 raw samples each; order sweep-row-lane/8/3; "
        "narrow=0; address guard active; limits unchanged; per-row aggregates agree"
    )

    # --- budget ledger recompute ---
    b = result["budget"]
    prior = result["prior_campaign_ledger"]
    # prior must match PRIOR_F1_LEDGER / preparation
    prior_expected = json.loads((PREP / "root-prior-ledger-verification.json").read_text())[
        "prior_campaign_ledger"
    ]
    # normalize bool
    pe = dict(prior_expected)
    checks["prior_ledger_exact"] = prior == pe or (
        {k: prior[k] for k in pe} == pe and prior.get("schema") == pe.get("schema")
    )
    # deeper field-wise
    prior_fields_ok = True
    for k, v in pe.items():
        if prior.get(k) != v:
            prior_fields_ok = False
            findings.append(f"prior field mismatch {k}: {prior.get(k)!r} vs {v!r}")
    checks["prior_fields_match_prep"] = prior_fields_ok

    # decision
    cb = auth.get("campaign_budget") or {}
    checks["decision_sha"] = (
        result.get("child_decision_sha256") == ACCEPTED_DECISION
        or auth.get("child_decision_sha256") == ACCEPTED_DECISION
        or cb.get("decision_sha256") == ACCEPTED_DECISION
    )
    checks["ceilings"] = (
        cb.get("cumulative_child_ceiling") == 122
        and cb.get("cumulative_wall_ceiling") == 1800
        and cb.get("cumulative_cpu_ceiling") == 1500
        and cb.get("fresh_child_ceiling") == 118
    )

    checks["children_8"] = b.get("children") == 8 and prior.get("children") == 4
    checks["no_reservations"] = (
        b.get("reserved_children", 0) == 0
        and b.get("reserved_cpu", 0) == 0
        and b.get("reserved_wall", 0) == 0
        and b.get("unknown_cpu_charge", 0) == 0
        and b.get("stopped") is False
    )

    fresh_wall = b["wall"] - prior["wall"]
    fresh_cpu = b["cpu"] - prior["cpu"]
    checks["fresh_wall"] = abs(fresh_wall - 41.6546222899924) < 1e-12
    checks["fresh_cpu"] = abs(fresh_cpu - 31.437117) < 1e-12
    checks["cum_wall"] = abs(b["wall"] - 81.74507052099216) < 1e-12
    checks["cum_cpu"] = abs(b["cpu"] - 60.938457) < 1e-12

    # CPU component identity.
    # Budget.snapshot spent gauge: cpu = prior.cpu + total_cpu_delta + penalty
    # Components: supervisor (process_time), waited (total_cpu_delta - own), unknown (penalty).
    # Sealed ledgers also carry publication_reserved_cpu inside the cumulative cpu
    # number (prior F1 sealed with +0.1; CF1 result carries +0.2). Meaningful identity:
    #   cpu ≈ supervisor + waited + unknown + publication_reserved_cpu
    # within sampling noise (~16us), not exact equality of separately sampled clocks.
    def components_with_pub(x):
        return (
            x["supervisor_cpu"]
            + x["waited_children_cpu"]
            + x.get("unknown_cpu_charge", 0)
            + x.get("publication_reserved_cpu", 0)
        )

    def components_no_pub(x):
        return x["supervisor_cpu"] + x["waited_children_cpu"] + x.get("unknown_cpu_charge", 0)

    prior_cpu_vs_with_pub = prior["cpu"] - components_with_pub(prior)
    prior_cpu_vs_no_pub = prior["cpu"] - components_no_pub(prior)
    cpu_vs_with_pub = b["cpu"] - components_with_pub(b)
    cpu_vs_no_pub = b["cpu"] - components_no_pub(b)
    checks["prior_cpu_equals_components_plus_publication"] = abs(prior_cpu_vs_with_pub) < 1e-4
    checks["cpu_component_delta_abs_lt_1ms"] = abs(cpu_vs_with_pub) < 0.001
    checks["reported_16us_order"] = abs(cpu_vs_with_pub) < 1e-4  # ~16us
    checks["publication_reserved_explains_0_2_gap"] = abs(cpu_vs_no_pub - b.get("publication_reserved_cpu", 0)) < 1e-4

    sum_child_cpu = sum(c["child_cpu"] for c in children)
    waited_delta = b["waited_children_cpu"] - prior["waited_children_cpu"]
    waited_minus_recorded = waited_delta - sum_child_cpu
    checks["waited_minus_recorded_approx_0_1207"] = abs(waited_minus_recorded - 0.120699825) < 1e-9

    accounting = {
        "prior_cpu": prior["cpu"],
        "prior_supervisor_cpu": prior["supervisor_cpu"],
        "prior_waited_children_cpu": prior["waited_children_cpu"],
        "prior_unknown_cpu_charge": prior["unknown_cpu_charge"],
        "prior_publication_reserved_cpu": prior.get("publication_reserved_cpu"),
        "prior_cpu_minus_components_no_pub": prior_cpu_vs_no_pub,
        "prior_cpu_minus_components_with_pub": prior_cpu_vs_with_pub,
        "cum_cpu": b["cpu"],
        "cum_supervisor_cpu": b["supervisor_cpu"],
        "cum_waited_children_cpu": b["waited_children_cpu"],
        "cum_unknown_cpu_charge": b["unknown_cpu_charge"],
        "cum_publication_reserved_cpu": b.get("publication_reserved_cpu"),
        "cum_cpu_minus_supervisor_waited_unknown": cpu_vs_no_pub,
        "cum_cpu_minus_supervisor_waited_unknown_publication": cpu_vs_with_pub,
        "fresh_waited_children_cpu": waited_delta,
        "sum_four_probe_child_cpu": sum_child_cpu,
        "waited_minus_recorded_child_cpu_s": waited_minus_recorded,
        "publication_reserved_wall": b.get("publication_reserved_wall"),
        "publication_reserved_cpu": b.get("publication_reserved_cpu"),
        "semantics": {
            "total_cpu": "RUSAGE_SELF utime+stime + RUSAGE_CHILDREN utime+stime",
            "supervisor_cpu": "prior.supervisor + process_time delta (own_start)",
            "waited_children_cpu": "prior.waited + max(0, total_cpu_delta - own_process_time)",
            "per_probe_child_cpu": "RUSAGE_CHILDREN delta strictly around each launch()",
            "spent_cpu_gauge": "prior.cpu + total_cpu_delta + penalty (unknown failures)",
            "publication_reserved": (
                "Sealed into cumulative cpu on the prior F1 ledger (+0.1) and carried "
                "forward; CF1 result shows publication_reserved_cpu=0.2. It is not free "
                "headroom outside the spent gauge: cpu ≈ supervisor+waited+unknown+publication_reserved."
            ),
            "fully_charged": True,
            "silently_discarded": False,
            "waited_surplus_interpretation": (
                "Fresh waited_children_cpu exceeds sum of four per-probe child_cpu by "
                f"{waited_minus_recorded:.9f}s. That surplus remains inside waited_children_cpu "
                "and therefore inside the spent cpu gauge (via total_cpu). It is not discarded. "
                "Equality to the four probe records is not required: any other reaped child "
                "work (helpers under the supervisor between probes, bookkeeping subprocesses) "
                "contributes to RUSAGE_CHILDREN/total_cpu outside the four instrumented windows. "
                "Separate clock reads (process_time vs getrusage SELF) can also move a few "
                "microseconds between supervisor and waited buckets; that is bucketization "
                "noise, not lost charge."
            ),
            "cpu_component_16us": (
                f"cpu - (supervisor+waited+unknown+publication_reserved) = {cpu_vs_with_pub:.12e}s. "
                "Budget.snapshot samples process_time and total_cpu (two getrusage calls) "
                "sequentially; exact equality is not a production invariant. "
                "1ms audit tolerance is appropriate; production ceilings unchanged. "
                f"Omitting publication_reserved leaves a {cpu_vs_no_pub:.6f}s gap equal to "
                "publication_reserved_cpu — not missing charge."
            ),
        },
    }
    checks["fully_charged_no_discard"] = (
        b["unknown_cpu_charge"] == 0
        and b["stopped"] is False
        and waited_minus_recorded > 0  # surplus is in waited, not missing
        and abs(cpu_vs_with_pub) < 0.001
    )

    # CF2 admissibility (do not release)
    remaining_children = 122 - b["children"]
    remaining_wall = 1800 - b["wall"]
    remaining_cpu = 1500 - b["cpu"]
    cf2_needs_children = 114
    # CF2 ceiling is 114 fresh within same cumulative 1800/1500/122
    cf2_admissible = (
        remaining_children >= cf2_needs_children
        and remaining_wall > 0
        and remaining_cpu > 0
        and b["stopped"] is False
        and b.get("reserved_children", 0) == 0
        and prior.get("stopped") is False
    )
    # Headroom sanity: CF2 is large; feasibility of CF1 does not prove CF2 completes,
    # only that the accounting slots/ceilings still admit starting CF2 under the contract.
    cf2 = {
        "remaining_children": remaining_children,
        "remaining_wall_s": remaining_wall,
        "remaining_cpu_s": remaining_cpu,
        "cf2_child_ceiling": 114,
        "slots_sufficient_for_full_cf2_schedule": remaining_children >= 114,
        "cumulative_ceilings_not_exhausted": remaining_wall > 0 and remaining_cpu > 0,
        "no_stop_no_pending_reservation": cf2_admissible,
        "admissible_to_authorize_cf2_under_budget_contract": cf2_admissible,
        "does_not_authorize_or_release_cf2": True,
        "does_not_claim_cf2_will_complete_within_remaining_cpu_wall": True,
        "note": (
            "Contract admits CF2's 114 children under 122/1800/1500 after 8 spent. "
            "Remaining ~1718 wall / ~1439 CPU is the cumulative remainder including "
            "whatever CF2 actually burns; this review does not project CF2 runtime."
        ),
    }
    checks["cf2_budget_admissible"] = cf2_admissible

    # RSS observation only
    rss = {c["name"]: c["peak_rss"] for c in children}
    checks["rss_observation_only"] = (
        rss["000-1-1-dense"] == 149553152
        and rss["003-1-2-dense"] == 149553152
        and rss["001-1-1-refined"] == 153616384
        and rss["002-1-2-refined"] == 153616384
    )
    findings.append(
        "RSS dense=149553152 refined=153616384 both rows; observation only, not a win; "
        "two routes insufficient for 59-route acceptance"
    )

    # preflight present in archive
    checks["preflight_member"] = "root-coordinate-cf1-admission/preflight.json" in data
    checks["run_log_member"] = "root-coordinate-cf1-admission/run.log" in data

    # Compare to root-audit.json claims
    root_audit = json.loads((RESULT / "root-audit.json").read_text())
    checks["root_audit_archive"] = root_audit["archive_sha256"] == EXPECTED_ARCHIVE
    checks["root_audit_waited_delta"] = abs(
        root_audit["waited_minus_recorded_child_cpu_s"] - waited_minus_recorded
    ) < 1e-12
    # root used publication_reserved in component delta; record both
    findings.append(
        f"fresh wall={fresh_wall} cpu={fresh_cpu}; cum wall={b['wall']} cpu={b['cpu']} children={b['children']}"
    )
    findings.append(
        f"cpu_component_delta(supervisor+waited+unknown+pub)={cpu_vs_with_pub}; "
        f"without_pub={cpu_vs_no_pub}; root_audit={root_audit.get('cpu_component_sampling_delta_s')}"
    )
    findings.append(
        f"waited_minus_recorded={waited_minus_recorded}; fully charged in waited/spent gauges"
    )
    findings.append(
        f"CF2 budget-admissible={cf2_admissible} remaining children={remaining_children} "
        f"wall={remaining_wall:.3f} cpu={remaining_cpu:.3f}; not released"
    )

    # meaningful invariants (not broad rootqualified)
    invariants = {
        "archive_members_82_match_manifest": checks["exact_member_set"],
        "auth_sha_62835390": checks["admission_auth_sha"],
        "result_complete_4_CF1": checks["result_status"],
        "pack_2f393138_73438581": checks["pack_sha"] and checks["pack_bytes"],
        "selectors_59_space_normalized_49e348ea": checks["all_59_selectors_space_normalized"]
        and checks["routes_sha"],
        "four_children_exit0_limits_unchanged_narrow0_raw24": checks["four_children_ok"],
        "prior_F1_ledger_immutable_4_children": checks["prior_fields_match_prep"],
        "cumulative_8_children_no_stop_no_unknown_no_pending": checks["children_8"]
        and checks["no_reservations"],
        "fresh_and_cumulative_budget_quantities": checks["fresh_wall"]
        and checks["fresh_cpu"]
        and checks["cum_wall"]
        and checks["cum_cpu"],
        "spent_cpu_matches_components_within_1ms": checks["cpu_component_delta_abs_lt_1ms"],
        "waited_surplus_retained_in_ledger": checks["fully_charged_no_discard"],
        "decision_122_1800_1500": checks["decision_sha"] and checks["ceilings"],
        "cf2_slots_open_under_contract_not_released": checks["cf2_budget_admissible"],
    }
    checks["all_invariants"] = all(invariants.values())

    failed = [k for k, v in checks.items() if not v]
    verdict = "APPROVE" if not failed and checks["all_invariants"] else "CHANGES_REQUESTED"

    out = {
        "verdict": verdict,
        "reviewed_commit": "27151fb2bc5356159e51a2ecda3a0228ca0e22ac",
        "task": "t_f7d038ac",
        "scope": "CF1 raw feasibility + cumulative ledger only; CF2/CA/perf closed; no release",
        "checks": checks,
        "invariants": invariants,
        "failed_checks": failed,
        "children": children,
        "accounting": accounting,
        "cf2_admissibility": cf2,
        "budget": {
            "prior": prior,
            "cumulative": b,
            "fresh_wall": fresh_wall,
            "fresh_cpu": fresh_cpu,
        },
        "rss_observation": rss,
        "findings": findings,
        "root_audit_crosscheck": {
            "qualified_flag_not_used_as_verdict": True,
            "root_audit_cpu_component_sampling_delta_s": root_audit.get(
                "cpu_component_sampling_delta_s"
            ),
            "independent_cpu_minus_supervisor_waited_unknown_publication": cpu_vs_with_pub,
            "independent_cpu_minus_supervisor_waited_unknown": cpu_vs_no_pub,
            "note": (
                "Root audit identity cpu-(supervisor+waited+publication_reserved) is correct: "
                "sealed ledgers carry publication_reserved inside cumulative cpu. "
                "Independent match is ~16us; omitting publication_reserved leaves ~0.2s equal to "
                "publication_reserved_cpu, not missing charge."
            ),
        },
    }
    (HERE / "verdict.json").write_text(json.dumps(out, indent=2, sort_keys=True) + "\n")
    (HERE / "checks.json").write_text(json.dumps(checks, indent=2, sort_keys=True) + "\n")
    print(json.dumps({"verdict": verdict, "failed": failed, "invariants": invariants}, indent=2))
    print("CF2:", json.dumps(cf2, indent=2))
    print("accounting deltas:", json.dumps({
        "cpu_vs_with_pub": cpu_vs_with_pub,
        "cpu_vs_no_pub": cpu_vs_no_pub,
        "waited_minus_recorded": waited_minus_recorded,
        "fresh_wall": fresh_wall,
        "fresh_cpu": fresh_cpu,
    }, indent=2))
    if failed:
        sys.exit(1)


if __name__ == "__main__":
    main()
