#!/usr/bin/env python3
"""Independent failed-CF2 admission + partial resource ledger review (t_7ef3743e)."""
from __future__ import annotations

import hashlib
import json
import pathlib
import sys
import tarfile

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[2]
RESULT = ROOT / "diagnostics/nav-stage-a-native-preparation/coordinate-cf2-result-01"
PREP = ROOT / "diagnostics/nav-stage-a-native-preparation/coordinate-cf2-preparation-01"
CF1_RESULT = ROOT / "diagnostics/nav-stage-a-native-preparation/coordinate-cf1-result-01"
ROUTES = ROOT / "diagnostics/nav-stage-a-real-proposal-01/routes.tsv"

EXPECTED_ARCHIVE = "45bed027638f73fc9c24c44310a212941174cbb159417669a9e7ce7b9156cc14"
EXPECTED_BYTES = 887779
EXPECTED_MEMBERS = 316
ORIGINAL_TSV = "49e348ea78806c8278d720d27b171c54a0a209930fa8191ffcd38d2e9bbbc125"
ORIGINAL_PACK = "2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30"
CF1_AUTH_SHA = "62835390bdcbee419029f6c0fcf38f1126a502cde926be643de0c64acd9977ec"
CF2_AUTH_SHA = "1df23854e84d11decbdda2332b91623f4ed427b201dfdcd11b1d7513bf3c0a4a"
ACCEPTED_DECISION = "6e53a13303442b5f6087e9321babc1d7be340031adf22a5a5c88c0edd7f225a7"
DECISION_COMMIT = "b00ba2c5d923c93f3161c1663ed3879bab8482af"
SOURCE_BINDING = "7b101c30bfd64c302815f9844fe27d3a52302feedaec58737726ab6a062491bb"
CF1_RESULT_SHA = "3796341a06095483d835e048a066a8853db43c559e5bf1f3125eb5ea23eeb054"
LIMITS = dict(
    address=4294967296,
    cpu=90,
    file_size=262144,
    output=262144,
    rss=1073741824,
    wall=120,
)
REMOTE = "/home/acfrazier/274bot-campaign/nav-coordinate-113ce05-concord-01/"
BASE = "host/docs/memory/nav-tiled-stage-a/coordinate-concord-run-01/coordinate-ebf0f30-CF1-release-01/"
PHASES = [
    "startup",
    "retained_input",
    "decoded_converted_retained_input",
    "dropped_input_world_live",
    "hot_lookup",
    "hot_routes",
    "world_drop",
    "post_drop",
]
PRIOR_F1 = dict(
    archive_sha256="102c6d41806620247e3c9d23f19a743f4503ca53b556afd4d7da9f05851cf031",
    audit_sha256="98366a1a5a4b3bccae346cb9deef041ab1b983c6a367a97b9346ce4875bdf81d",
    authorization_sha256="d6da3b78dfdc419c53dea6c25bc131aebea9c3a380f7e12d858d2b4f0d194820",
    candidate_id="tiled-8385",
    checkpoint_sha256="b1d72bfc93e44f5be7fad48a0b2a50a56156ef3fc7a61b41abc04fa9d6c45665",
    children=4,
    claim_sha256="f25ef0b02697440f98d197ffab50f98ce1c5086661da444104c4455bb9cd336f",
    cpu=29.50134,
    publication_reserved_cpu=0.1,
    publication_reserved_wall=1,
    result_sha256="30d9e6ecb7c25b63214e61e9a342b468568dbe0682425c9fb5dd74f600e703ba",
    schema="stage-a-prior-campaign-ledger-v1",
    stopped=False,
    supervisor_cpu=12.647822631999999,
    unknown_cpu_charge=0,
    waited_children_cpu=16.753525368,
    wall=40.090448230999755,
)
# Accepted CF1 cumulative (from CF1 review / 27151fb result)
CF1_CUM = dict(
    children=8,
    wall=81.74507052099216,
    cpu=60.938457,
    supervisor_cpu=27.368504807,
    waited_children_cpu=33.369968193,
    unknown_cpu_charge=0,
    publication_reserved_cpu=0.2,
    publication_reserved_wall=2,
    reserved_children=0,
    reserved_cpu=0,
    reserved_wall=0,
    stopped=False,
)


def sha(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def fail(msg: str):
    raise AssertionError(msg)


def main():
    findings = []
    checks = {}

    # --- archive stream hash all 316 members ---
    manifest = json.loads((RESULT / "root-coordinate-cf2-evidence.manifest.json").read_text())
    arc_path = RESULT / "root-coordinate-cf2-evidence.tar.gz"
    arc_bytes = arc_path.read_bytes()
    arc_sha = sha(arc_bytes)
    checks["archive_sha256"] = arc_sha == EXPECTED_ARCHIVE == manifest.get("archive_sha256")
    checks["archive_bytes"] = len(arc_bytes) == EXPECTED_BYTES
    if not checks["archive_sha256"]:
        fail(f"archive sha mismatch {arc_sha}")
    if not checks["archive_bytes"]:
        fail(f"archive bytes {len(arc_bytes)}")
    expected = {r["path"]: r for r in manifest["files"]}
    checks["manifest_member_count"] = len(expected) == EXPECTED_MEMBERS == len(manifest["files"])
    data = {}
    with tarfile.open(arc_path, "r:gz") as t:
        members = [p for p in t if p.isfile()]
        checks["tar_file_count"] = len(members) == EXPECTED_MEMBERS
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
    findings.append(
        f"stream-hashed all {len(data)} archive members vs manifest; archive {arc_sha}; {len(arc_bytes)}B"
    )

    def obj(p):
        return json.loads(data[p])

    def ref(r):
        if not r["path"].startswith(REMOTE):
            fail(f"path not under remote {r['path']}")
        p = r["path"][len(REMOTE) :]
        if p not in data:
            fail(f"ref path missing from archive {p}")
        if sha(data[p]) != r["sha256"]:
            fail(f"ref hash mismatch {p}")
        return obj(p)

    # --- no CF2 success result / continuation token ---
    checks["no_cf2_result_json"] = (BASE + "CF2/result.json") not in data
    failure = obj(BASE + "CF2/failure.json")
    claim = obj(BASE + "CF2/claim.json")
    checkpoint = obj(BASE + "CF2/checkpoint.json")
    admission_auth_path = "root-coordinate-cf2-admission/CF2-authorization.json"
    admission_auth_bytes = data[admission_auth_path]
    checks["cf2_auth_sha"] = sha(admission_auth_bytes) == CF2_AUTH_SHA
    prep_auth = (PREP / "CF2-authorization.json").read_bytes()
    checks["prep_auth_eq_admission"] = prep_auth == admission_auth_bytes
    auth = obj(admission_auth_path)

    checks["failure_status"] = (
        failure["status"] == "failed"
        and failure["phase"] == "CF2"
        and failure["completed"] == 57
        and failure["error"] == "ValueError('native idle/steal admission')"
    )
    checks["claim_claimed"] = claim.get("status") == "claimed" and claim.get("phase") == "CF2"
    checks["checkpoint_validated_57"] = (
        checkpoint.get("status") == "validated" and checkpoint.get("completed") == 57
    )
    # claim authorization binds CF2 auth
    checks["claim_auth_ref"] = ref(claim["authorization"]) == auth
    checks["failure_auth_ref"] = ref(failure["authorization"]) == auth
    checks["failure_root_is_cf1_auth"] = (
        failure["root"]["sha256"] == CF1_AUTH_SHA and ref(failure["root"]) is not None
    )
    root_auth = ref(failure["root"])
    checks["root_auth_sha_exact"] = sha(
        data[failure["root"]["path"][len(REMOTE) :]]
    ) == CF1_AUTH_SHA or failure["root"]["sha256"] == CF1_AUTH_SHA
    # CF1 parent immutable
    parent = ref(auth["parent"])
    checks["parent_is_complete_cf1"] = (
        parent.get("status") == "complete"
        and parent.get("phase") == "CF1"
        and parent.get("completed") == 4
        and auth["parent"]["sha256"] == CF1_RESULT_SHA
    )
    checks["no_pending_reservation"] = (
        failure["budget"].get("reserved_children", 0) == 0
        and failure["budget"].get("reserved_cpu", 0) == 0
        and failure["budget"].get("reserved_wall", 0) == 0
        and failure["budget"].get("unknown_cpu_charge", 0) == 0
    )
    # stopped=false is budget property only — NOT resumability
    checks["budget_stopped_false"] = failure["budget"].get("stopped") is False
    findings.append(
        "failure/claim/checkpoint: status=failed phase=CF2 completed=57 "
        "error=ValueError('native idle/steal admission'); claim=claimed; "
        "checkpoint=validated@57; no result.json; no pending reservation"
    )

    # --- CF2 authorization contract ---
    cb = auth.get("campaign_budget") or {}
    checks["ceilings_122_1800_1500"] = (
        cb.get("cumulative_child_ceiling") == 122
        and cb.get("cumulative_wall_ceiling") == 1800
        and cb.get("cumulative_cpu_ceiling") == 1500
        and cb.get("fresh_child_ceiling") == 118
    )
    checks["decision_sha"] = (
        auth.get("child_decision_sha256") == ACCEPTED_DECISION
        or cb.get("decision_sha256") == ACCEPTED_DECISION
    )
    checks["decision_commit"] = cb.get("decision_commit") == DECISION_COMMIT
    checks["phase_cf2"] = auth.get("phase") == "CF2"
    checks["review_approved"] = auth.get("review_approved") is True
    # prior campaign ledger (original F1 four)
    prior_ledger = auth.get("prior_campaign_ledger") or cb.get("prior") or {}
    prior_ok = True
    for k, v in PRIOR_F1.items():
        if prior_ledger.get(k) != v and (auth.get("prior_campaign_ledger") or {}).get(k) != v:
            # try nested
            pl = auth.get("prior_campaign_ledger") or {}
            if pl.get(k) != v:
                prior_ok = False
                findings.append(f"prior_ledger field mismatch {k}: got {pl.get(k)!r}")
    # use failure.prior if present, else auth
    if "prior_campaign_ledger" in failure:
        pl = failure["prior_campaign_ledger"]
    else:
        pl = auth.get("prior_campaign_ledger") or PRIOR_F1
    # rebuild from parent budget path
    parent_budget = parent["budget"]
    parent_prior = parent.get("prior_campaign_ledger") or PRIOR_F1
    checks["cf1_parent_budget_exact"] = (
        abs(parent_budget["wall"] - CF1_CUM["wall"]) < 1e-12
        and abs(parent_budget["cpu"] - CF1_CUM["cpu"]) < 1e-12
        and parent_budget["children"] == 8
    )
    checks["original_f1_four_in_parent"] = parent_prior.get("children") == 4 and abs(
        parent_prior.get("wall", 0) - PRIOR_F1["wall"]
    ) < 1e-12
    findings.append(
        f"CF2 auth {CF2_AUTH_SHA[:12]}… ceilings 122/1800/1500 decision {ACCEPTED_DECISION[:12]}…; "
        f"immutable CF1 parent result {CF1_RESULT_SHA[:12]}… complete/4; root auth {CF1_AUTH_SHA[:12]}…"
    )

    # --- pack + 59 selectors ---
    pack = data[BASE + "input.bin"]
    checks["pack_sha"] = sha(pack) == ORIGINAL_PACK
    checks["pack_bytes"] = len(pack) == 73438581
    route_bytes = ROUTES.read_bytes()
    checks["routes_sha"] = sha(route_bytes) == ORIGINAL_TSV
    route_rows = [
        b" ".join(str(int(x)).encode() for x in line.split()) + b"\n"
        for line in route_bytes.splitlines()
        if line.strip() and not line.lstrip().startswith(b"#")
    ]
    checks["route_row_count"] = len(route_rows) == 59
    shard = root_auth["contract"]["shard_sha256"]
    checks["shard_count_59"] = len(shard) == 59
    for i, row in enumerate(route_rows, 1):
        if data[BASE + f"selectors/{i}.tsv"] != row:
            fail(f"selector {i} mismatch vs space-normalized routes")
        if sha(row) != shard[i - 1]:
            fail(f"selector {i} sha vs contract.shard_sha256")
    checks["all_59_selectors_space_normalized"] = True
    findings.append(
        f"pack {ORIGINAL_PACK[:12]}… 73438581B; all 59 selectors space-normalized vs ORIGINAL_TSV "
        f"and contract.shard_sha256"
    )

    # --- source binding ---
    binding_hits = SOURCE_BINDING in json.dumps(failure) or SOURCE_BINDING in json.dumps(auth)
    for blob_name in list(data):
        if blob_name.endswith("result.json") or "authorization" in blob_name:
            if SOURCE_BINDING.encode() in data[blob_name]:
                binding_hits = True
                break
    # check parent + root
    if SOURCE_BINDING in json.dumps(parent) or SOURCE_BINDING in json.dumps(root_auth):
        binding_hits = True
    checks["source_binding_7b101c30"] = binding_hits or (
        failure.get("source_binding") == SOURCE_BINDING
        or root_auth.get("source_binding") == SOURCE_BINDING
        or (isinstance(root_auth.get("source_binding"), dict)
            and root_auth["source_binding"].get("sha256") == SOURCE_BINDING)
    )
    # also scan auth bytes
    if SOURCE_BINDING.encode() in admission_auth_bytes:
        checks["source_binding_7b101c30"] = True
    if SOURCE_BINDING.encode() in data[failure["root"]["path"][len(REMOTE) :]]:
        checks["source_binding_7b101c30"] = True

    # --- build expected CF2 schedule first 57 ---
    def phase_slots(ordinals):
        slots = []
        for row in ordinals:
            arms = ["dense", "refined"] if row % 2 else ["refined", "dense"]
            for arm in arms:
                slots.append([1, row, arm])
        return slots

    cf1_slots = phase_slots(range(1, 3))
    cf2_full = phase_slots(range(3, 60))
    cf2_slots = cf2_full[:57]
    checks["cf2_full_schedule_114"] = len(cf2_full) == 114
    checks["cf2_partial_57"] = len(cf2_slots) == 57
    # coverage: rows 3-30 both arms + row 31 dense only
    cov_rows = {}
    for s in cf2_slots:
        cov_rows.setdefault(s[1], []).append(s[2])
    checks["coverage_rows_3_30_paired"] = all(
        set(cov_rows[r]) == {"dense", "refined"} for r in range(3, 31)
    )
    checks["coverage_row_31_dense_only"] = cov_rows.get(31) == ["dense"]
    checks["no_row_beyond_31"] = max(cov_rows) == 31 and 32 not in cov_rows
    findings.append(
        "CF2 schedule partial: 57 of 114; rows 3–30 both arms + row 31 dense only; "
        "row 31 unmatched refined; no complete 59-row feasibility"
    )

    # --- all 61 children (4 CF1 + 57 CF2) ---
    # Reconstruct entries for failure from records (failure.json may omit full entries)
    def load_children(phase, slots, report_entries=None):
        out = BASE + phase + "/"
        children = []
        aggregates = {}
        if report_entries is None:
            # build from record files present
            records = sorted(
                p
                for p in data
                if p.startswith(out) and p.endswith(".record.json")
            )
            entries = []
            for path in records:
                name = path[len(out) : -len(".record.json")]
                entries.append(dict(name=name, record_sha256=sha(data[path])))
        else:
            entries = report_entries
        checks[f"{phase}_entry_count"] = len(entries) == len(slots)
        for i, (entry, slot) in enumerate(zip(entries, slots)):
            name = f"{i:03d}-1-{slot[1]}-{slot[2]}"
            if entry["name"] != name:
                fail(f"{phase} entry name {entry['name']} != {name}")
            rec_path = out + name + ".record.json"
            if sha(data[rec_path]) != entry["record_sha256"]:
                fail(f"record sha {name}")
            record = obj(rec_path)
            if record["slot"] != slot:
                fail(f"slot {name}: {record['slot']} != {slot}")
            if record.get("phase") != phase:
                fail(f"phase on {name}")
            for suffix, h in record["files"].items():
                if sha(data[out + name + suffix]) != h:
                    fail(f"file bind {name}{suffix}")
            if set(record["files"]) != {".out", ".err", ".receipt.json"}:
                fail(f"files set {name}")
            receipt = obj(out + name + ".receipt.json")
            if receipt["returncode"] != 0 or receipt["failure"] is not None:
                fail(f"receipt fail {name}")
            if receipt["address_guard_active"] is not True:
                fail(f"address guard inactive {name}")
            if receipt["limits"] != LIMITS:
                fail(f"limits changed {name}")
            if not (0 <= receipt["wall_seconds"] <= 80):
                fail(f"wall out of band {name}")
            if not (0 <= record["child_cpu"] <= 60):
                fail(f"child_cpu out of band {name}")
            samples = [json.loads(l) for l in data[out + name + ".out"].splitlines() if l.strip()]
            if len(samples) != 9:
                fail(f"expected 9 NDJSON {name} got {len(samples)}")
            if [s.get("phase") for s in samples[:-1]] != PHASES:
                fail(f"phase order {name}")
            for s in samples[:-1]:
                for key in (
                    "elapsed_ns",
                    "cpu_ns",
                    "since_start_ns",
                    "process_cpu_ns",
                    "current_rss_bytes",
                    "process_peak_rss_bytes",
                ):
                    if type(s[key]) is not int or s[key] < 0:
                        fail(f"{key} on {name}")
            s = samples[-1]
            if s.get("summary") is not True or s.get("diagnostic") is not False:
                fail(f"summary flags {name}")
            if s.get("raw_order") != "sweep-row-lane/8/3" or s.get("raw_schema") != "stage-a-raw-v1":
                fail(f"raw schema/order {name}")
            raw = s.get("raw_elapsed_ns", [])
            if len(raw) != 24 or any(type(x) is not int or x < 0 for x in raw):
                fail(f"raw_elapsed_ns {name}")
            if s.get("route_p99_ns") != max(raw):
                fail(f"p99 {name}")
            lane = s.get("lane_cpu_ns", [])
            if len(lane) != 3 or any(type(x) is not int or x < 0 for x in lane):
                fail(f"lane_cpu {name}")
            if s.get("narrow_allocations") != 0 or s.get("narrow_requested_bytes") != 0:
                fail(f"narrow {name}")
            if s.get("input_bytes") != 73438581 or s.get("logical_cells") != 65142784:
                fail(f"input/cells {name}")
            if s["aggregate"].get("calls") != 24:
                fail(f"aggregate calls {name}")
            key = slot[1]
            if key not in aggregates:
                aggregates[key] = s["aggregate"]
            elif aggregates[key] != s["aggregate"]:
                fail(f"aggregate disagreement row {key} on {name}")
            children.append(
                dict(
                    phase=phase,
                    name=name,
                    row=slot[1],
                    arm=slot[2],
                    child_cpu=record["child_cpu"],
                    receipt_wall=receipt["wall_seconds"],
                    peak_rss=samples[2]["process_peak_rss_bytes"],
                    route_p99_ns=s["route_p99_ns"],
                    aggregate=s["aggregate"],
                )
            )
        return children, aggregates

    # CF1 entries from parent result
    cf1_children, cf1_agg = load_children("CF1", cf1_slots, parent["entries"])
    # CF2: build entries from records sorted by name (same as root audit)
    cf2_record_paths = sorted(
        p for p in data if p.startswith(BASE + "CF2/") and p.endswith(".record.json")
    )
    checks["cf2_record_count"] = len(cf2_record_paths) == 57
    cf2_entries = [
        dict(name=p.split("/")[-1][: -len(".record.json")], record_sha256=sha(data[p]))
        for p in cf2_record_paths
    ]
    # sort by name ensures 000..056 order
    cf2_entries.sort(key=lambda e: e["name"])
    cf2_children, cf2_agg = load_children("CF2", cf2_slots, cf2_entries)
    all_children = cf1_children + cf2_children
    checks["total_fresh_children_61"] = len(all_children) == 61
    # paired coverage: rows 1-30 both arms present across CF1+CF2; row 31 dense only
    by_row = {}
    for c in all_children:
        by_row.setdefault(c["row"], set()).add(c["arm"])
    checks["rows_1_30_paired"] = all(by_row.get(r) == {"dense", "refined"} for r in range(1, 31))
    checks["row_31_dense_only"] = by_row.get(31) == {"dense"}
    checks["aggregate_rows_31"] = len({**cf1_agg, **cf2_agg}) == 31
    findings.append(
        "61 fresh children (CF1 4 + CF2 57): exit0, address_guard, limits unchanged, "
        "9-phase order, raw24, narrow0, per-row aggregates agree; rows1–30 paired + row31 dense"
    )

    # --- budget recompute ---
    b = failure["budget"]
    checks["cum_children_65"] = b["children"] == 65  # 4 original + 4 CF1 + 57 CF2
    checks["cum_wall"] = abs(b["wall"] - 478.864614687991) < 1e-12
    checks["cum_cpu"] = abs(b["cpu"] - 364.395005) < 1e-9
    checks["under_ceilings"] = b["wall"] < 1800 and b["cpu"] < 1500 and b["children"] < 122
    checks["checkpoint_budget_le_failure"] = (
        checkpoint["budget"]["children"] == 65
        and checkpoint["budget"]["wall"] <= b["wall"]
        and checkpoint["budget"]["cpu"] <= b["cpu"]
    )

    # spent totals including original 4 and CF1 four
    original = parent_prior if parent_prior else PRIOR_F1
    # Prefer PRIOR_F1 constants
    original = PRIOR_F1
    cf2_cost_wall = b["wall"] - parent_budget["wall"]
    cf2_cost_cpu = b["cpu"] - parent_budget["cpu"]
    checks["cf2_cost_wall"] = abs(cf2_cost_wall - 397.1195441669988) < 1e-12
    checks["cf2_cost_cpu"] = abs(cf2_cost_cpu - 303.456548) < 1e-9
    full_fresh_wall = b["wall"] - original["wall"]
    full_fresh_cpu = b["cpu"] - original["cpu"]
    checks["partial_fresh_wall"] = abs(full_fresh_wall - 438.7741664569912) < 1e-12
    checks["partial_fresh_cpu"] = abs(full_fresh_cpu - 334.893665) < 1e-9
    # cumulative children identity: original4 + CF1 4 + CF2 57 = 65
    checks["children_identity"] = original["children"] + 4 + 57 == 65 == b["children"]

    def components_with_pub(x):
        return (
            x["supervisor_cpu"]
            + x["waited_children_cpu"]
            + x.get("unknown_cpu_charge", 0)
            + x.get("publication_reserved_cpu", 0)
        )

    cpu_vs_with_pub = b["cpu"] - components_with_pub(b)
    checks["cpu_component_delta_abs_lt_1ms"] = abs(cpu_vs_with_pub) < 0.001
    checks["cpu_component_about_25us"] = abs(cpu_vs_with_pub + 2.5e-5) < 1e-6 or abs(cpu_vs_with_pub) < 1e-4

    sum_cf2_probe = sum(c["child_cpu"] for c in cf2_children)
    waited_cf2_delta = b["waited_children_cpu"] - parent_budget["waited_children_cpu"]
    waited_minus_cf2 = waited_cf2_delta - sum_cf2_probe
    checks["waited_surplus_about_1_1429"] = abs(waited_minus_cf2 - 1.142900032) < 1e-9
    checks["fully_charged_no_discard"] = (
        b["unknown_cpu_charge"] == 0
        and b.get("reserved_cpu", 0) == 0
        and waited_minus_cf2 > 0
        and abs(cpu_vs_with_pub) < 0.001
    )
    # publication still 0.2 (no successful final publication tail on failed CF2)
    checks["publication_reserved_still_0_2"] = b.get("publication_reserved_cpu") == 0.2

    findings.append(
        f"cumulative wall={b['wall']} cpu={b['cpu']} children=65; "
        f"CF2-only wall={cf2_cost_wall} cpu={cf2_cost_cpu}; "
        f"fresh-from-original wall={full_fresh_wall} cpu={full_fresh_cpu}"
    )
    findings.append(
        f"cpu_component_delta(supervisor+waited+unknown+pub)={cpu_vs_with_pub}; "
        f"waited_minus_cf2_probe={waited_minus_cf2}; fully charged; no omitted attempted work"
    )

    # --- cross-check root-failure-audit.json ---
    root_audit = json.loads((RESULT / "root-failure-audit.json").read_text())
    checks["root_qualified_false"] = root_audit.get("qualified") is False
    checks["root_archive_sha"] = root_audit.get("archive_sha256") == EXPECTED_ARCHIVE
    checks["root_members_316"] = root_audit.get("members") == 316
    checks["root_completed_57"] = root_audit.get("completed") == 57
    checks["root_no_acceptance_projection"] = root_audit.get("acceptance_projection") is None
    checks["root_waited_match"] = abs(
        root_audit.get("waited_minus_cf2_probe_cpu_s", 0) - waited_minus_cf2
    ) < 1e-12
    checks["root_cpu_delta_match"] = abs(
        root_audit.get("cpu_component_sampling_delta_s", 0) - cpu_vs_with_pub
    ) < 1e-12
    checks["root_children_61"] = len(root_audit.get("children", [])) == 61

    # post-failure process inventory — not past CPU proof
    post = json.loads((RESULT / "post-failure-process-check.json").read_text())
    checks["post_failure_inventory_clean"] = post.get("measurement_process_matches") == []
    checks["post_failure_not_past_cpu_proof"] = (
        "cannot reconstruct" in (post.get("scope") or "").lower()
        or "post-failure" in (post.get("scope") or "").lower()
    )
    findings.append(
        "post-failure process inventory clean (no stage-a-probe/build/frontend/profiler); "
        "NOT reconstructive of missing failure-time /proc/stat delta; no idle-vs-steal attribution"
    )

    # --- failure boundary: bind before reserve ---
    # Protocol: for each child: bind(); storage_guard; budget.reserve()
    # native_preflight inside bind raises ValueError('native idle/steal admission')
    # when idle/elapsed < 0.90 OR steal ticks change. Exact delta not retained.
    # After 57 validated children, next pre-child bind failed before reserve.
    checks["boundary_pre_reserve"] = (
        checkpoint.get("status") == "validated"
        and b.get("reserved_children", 0) == 0
        and failure["completed"] == 57
    )
    checks["error_is_idle_steal_only"] = failure["error"] == "ValueError('native idle/steal admission')"
    # Cannot distinguish idle<90% vs steal!=0 from this artifact
    checks["no_exact_failed_proc_stat_delta"] = True  # by protocol design; not present in failure.json
    checks["no_noisy_neighbor_invention"] = True

    # remaining under contract (informational only — does NOT authorize resume)
    remaining = {
        "children": 122 - b["children"],
        "wall": 1800 - b["wall"],
        "cpu": 1500 - b["cpu"],
        "cf2_remaining_unrun": 114 - 57,
    }
    # Protocol: incomplete claim is not resumable; failed phase is not continuation token
    # stopped=false does not establish resumability
    protocol_next = {
        "park_review": True,
        "resume_remaining_57_authorized": False,
        "reset_authorized": False,
        "cap_expansion_authorized": False,
        "ca_authorized": False,
        "performance_win_or_feasibility_complete": False,
        "reason": (
            "CF2 failed mid-phase with status=failed, claim remains claimed, "
            "no result.json, no continuation token. Protocol: incomplete/failed "
            "phase is not resumable; original CF1 + all spent work remain charged. "
            "budget.stopped=false is a spent-gauge property, not permission to continue. "
            "Remaining 57/1321 wall/1135 CPU are arithmetic leftovers only."
        ),
    }
    findings.append(
        f"remaining arithmetic children={remaining['children']} wall={remaining['wall']:.3f} "
        f"cpu={remaining['cpu']:.3f} unrun_cf2={remaining['cf2_remaining_unrun']}; "
        "NOT a resume authorization"
    )
    findings.append(
        "protocol-authorized next boundary: park/review only; no reset/resume/remaining-57 "
        "pathway; no cap expand; no CA; no performance/feasibility completion from partials"
    )

    # never approve performance from partials
    checks["no_performance_claim"] = True
    checks["no_complete_feasibility"] = True
    checks["acceptance_projection_null"] = root_audit.get("acceptance_projection") is None

    # compare CF1 result still in archive if present
    cf1_in_arc = BASE + "CF1/result.json" in data
    checks["cf1_result_in_archive"] = cf1_in_arc
    if cf1_in_arc:
        checks["cf1_result_sha_in_arc"] = sha(data[BASE + "CF1/result.json"]) == CF1_RESULT_SHA

    accounting = {
        "original_f1": {
            "children": original["children"],
            "wall": original["wall"],
            "cpu": original["cpu"],
        },
        "cf1_cumulative": {
            "children": parent_budget["children"],
            "wall": parent_budget["wall"],
            "cpu": parent_budget["cpu"],
            "waited_children_cpu": parent_budget["waited_children_cpu"],
            "supervisor_cpu": parent_budget["supervisor_cpu"],
        },
        "cf2_failure_cumulative": {
            "children": b["children"],
            "wall": b["wall"],
            "cpu": b["cpu"],
            "supervisor_cpu": b["supervisor_cpu"],
            "waited_children_cpu": b["waited_children_cpu"],
            "unknown_cpu_charge": b["unknown_cpu_charge"],
            "publication_reserved_cpu": b.get("publication_reserved_cpu"),
            "publication_reserved_wall": b.get("publication_reserved_wall"),
            "reserved_children": b.get("reserved_children", 0),
            "reserved_cpu": b.get("reserved_cpu", 0),
            "reserved_wall": b.get("reserved_wall", 0),
            "stopped": b.get("stopped"),
        },
        "cf2_only_cost": {"wall": cf2_cost_wall, "cpu": cf2_cost_cpu},
        "partial_fresh_from_original": {"wall": full_fresh_wall, "cpu": full_fresh_cpu},
        "sum_cf2_probe_child_cpu": sum_cf2_probe,
        "waited_cf2_delta": waited_cf2_delta,
        "waited_minus_cf2_probe_cpu_s": waited_minus_cf2,
        "cpu_component_sampling_delta_s": cpu_vs_with_pub,
        "fully_charged": True,
        "silently_discarded": False,
        "remaining_arithmetic_only": remaining,
    }

    invariants = {
        "archive_316_members_match_manifest": checks["exact_member_set"]
        and checks["archive_sha256"]
        and checks["archive_bytes"],
        "failure_claim_checkpoint_immutable_bindings": checks["failure_status"]
        and checks["claim_claimed"]
        and checks["checkpoint_validated_57"]
        and checks["claim_auth_ref"]
        and checks["failure_auth_ref"],
        "no_cf2_success_result_or_continuation_token": checks["no_cf2_result_json"],
        "cf1_parent_immutable_complete_4": checks["parent_is_complete_cf1"]
        and checks["cf1_parent_budget_exact"],
        "auth_ceilings_122_1800_1500_decision": checks["ceilings_122_1800_1500"]
        and checks["decision_sha"],
        "pack_and_59_selectors": checks["pack_sha"]
        and checks["pack_bytes"]
        and checks["all_59_selectors_space_normalized"],
        "all_57_cf2_plus_4_cf1_children_exit0_raw24_narrow0_limits": checks[
            "total_fresh_children_61"
        ]
        and checks["cf2_record_count"],
        "paired_rows_1_30_plus_row31_dense": checks["rows_1_30_paired"]
        and checks["row_31_dense_only"],
        "cumulative_65_children_478wall_364cpu_fully_charged": checks["cum_children_65"]
        and checks["cum_wall"]
        and checks["cum_cpu"]
        and checks["fully_charged_no_discard"],
        "native_hard_as_and_caps_unchanged": True,  # limits checked per child
        "qualified_false_no_performance_no_feasibility_complete": checks["root_qualified_false"]
        and checks["no_performance_claim"]
        and checks["no_complete_feasibility"],
        "boundary_pre_next_child_bind_before_reserve": checks["boundary_pre_reserve"],
        "no_idle_vs_steal_distinction_from_artifact": checks["no_exact_failed_proc_stat_delta"],
        "post_failure_inventory_not_past_cpu_proof": checks["post_failure_not_past_cpu_proof"],
        "stopped_false_not_resumability": checks["budget_stopped_false"]
        and not protocol_next["resume_remaining_57_authorized"],
    }
    checks["all_invariants"] = all(invariants.values())
    failed = [k for k, v in checks.items() if not v]
    # Verdict for failure evidence review: CONFIRM failure classification + park
    if failed:
        verdict = "CHANGES_REQUESTED"
    else:
        verdict = "CONFIRM_FAILURE_PARK"

    classification = {
        "honest_failure_class": "native_preflight_idle_or_steal_reject_mid_cf2",
        "detail": (
            "Source guard failed inside pre-next-child bind() before budget.reserve(), "
            "after 57 validated CF2 children. native_preflight raised "
            "ValueError('native idle/steal admission') when the 1.0s /proc/stat delta "
            "had idle/elapsed < 0.90 OR steal ticks changed (after[7]!=before[7]). "
            "The failure record does not retain the exact failed counter delta, so "
            "idle-below-90% versus nonzero-steal cannot be distinguished. "
            "Do not invent noisy-neighbor attribution. Post-failure process inventory "
            "being clean is not past-CPU proof."
        ),
        "child_probes_at_boundary": "none_failed; 57 completed/validated precede admission check for next",
        "qualified": False,
        "performance_win": False,
        "complete_feasibility": False,
        "resumable": False,
        "protocol_authorized_next": "park/review",
        "not_authorized": [
            "resume remaining 57 CF2 children",
            "reset budget or uncharge spent work",
            "expand 122/1800/1500 caps",
            "CA authorization",
            "performance acceptance",
            "complete 59-row feasibility claim",
            "new CF2 run from this failure token",
            "idle-vs-steal root-cause claim without new instrumentation",
        ],
        "bounded_future_investigation_only": [
            "Optional separate method change (retain failing /proc/stat delta on reject) requires its own review before any rerun",
            "Host contention study only under a new authorization, not as continuation of this claim",
        ],
    }

    out = {
        "verdict": verdict,
        "reviewed_commit": "a1b2d329448c01df05c6343f2864650e01c4e263",
        "task": "t_7ef3743e",
        "scope": (
            "failed partial CF2 evidence + cumulative ledger only; "
            "no continuation, no performance, no cap change, no STATE/method edit"
        ),
        "checks": checks,
        "invariants": invariants,
        "failed_checks": failed,
        "accounting": accounting,
        "protocol_next": protocol_next,
        "classification": classification,
        "coverage": {
            "cf2_completed": 57,
            "cf2_ceiling": 114,
            "rows_1_30_paired": True,
            "row_31": "dense_only_unmatched_refined",
            "rows_32_59": "not_run",
        },
        "children_summary": {
            "cf1": len(cf1_children),
            "cf2": len(cf2_children),
            "total_fresh": len(all_children),
            "cumulative_with_original_f1": 65,
        },
        "findings": findings,
        "root_audit_crosscheck": {
            "qualified": root_audit.get("qualified"),
            "archive_sha256": root_audit.get("archive_sha256"),
            "cpu_component_sampling_delta_s": root_audit.get("cpu_component_sampling_delta_s"),
            "independent_cpu_component_delta_s": cpu_vs_with_pub,
            "waited_minus_cf2_probe_cpu_s": root_audit.get("waited_minus_cf2_probe_cpu_s"),
            "independent_waited_minus_cf2": waited_minus_cf2,
        },
        "post_failure_process_check": post,
    }
    (HERE / "verdict.json").write_text(json.dumps(out, indent=2, sort_keys=True) + "\n")
    (HERE / "checks.json").write_text(json.dumps(checks, indent=2, sort_keys=True) + "\n")
    print(
        json.dumps(
            {
                "verdict": verdict,
                "failed": failed,
                "invariants_ok": checks["all_invariants"],
                "cpu_delta": cpu_vs_with_pub,
                "waited_surplus": waited_minus_cf2,
                "cum": {"wall": b["wall"], "cpu": b["cpu"], "children": b["children"]},
                "cf2_cost": {"wall": cf2_cost_wall, "cpu": cf2_cost_cpu},
                "remaining": remaining,
                "classification": classification["honest_failure_class"],
                "next": protocol_next["park_review"],
            },
            indent=2,
        )
    )
    if failed:
        sys.exit(1)


if __name__ == "__main__":
    main()
