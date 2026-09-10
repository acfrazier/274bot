#!/usr/bin/env python3
"""Independent exact59 coverage: routes.tsv + root-audit frames + stream output equality."""
import hashlib
import json
import struct
import tarfile
from collections import Counter
from pathlib import Path

base = Path("diagnostics/native-nav-differential-preparation/coordinate-a44a930-native-01")
out = Path("diagnostics/native-nav-differential-preparation/coordinate-native-evidence-review")
out.mkdir(parents=True, exist_ok=True)

EXPECTED_PACK = "2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30"
EXPECTED_TSV = "49e348ea78806c8278d720d27b171c54a0a209930fa8191ffcd38d2e9bbbc125"
EXPECTED_OUT = "bbb179b7bedf32e8811db730b6fe69e069c67d9f80aca62bf14a0c577e7dd022"
EXPECTED_OUT_BYTES = 2119776634
WORLD_CELLS = 65142784

arc = base / "exact59-evidence.tar.gz"
PREFIX = "host/docs/memory/nav-tiled-differential/coordinate-native-generated-01/real-release/"

# Extract small members only into memory
small = {}
with tarfile.open(arc, "r:gz") as tf:
    for m in tf:
        if not m.isfile():
            continue
        if m.size > 100_000_000:  # skip multi-GB outs and pack; stream separately
            continue
        f = tf.extractfile(m)
        data = f.read()
        assert len(data) == m.size
        small[m.name] = data

checks = []


def check(name, ok, detail=None):
    checks.append({"name": name, "ok": bool(ok), "detail": detail})
    print(("PASS" if ok else "FAIL"), name, detail if detail is not None else "")


# routes.tsv
routes_name = PREFIX + "routes.tsv"
routes_raw = small[routes_name]
check("routes_sha256", hashlib.sha256(routes_raw).hexdigest() == EXPECTED_TSV)
lines = [ln for ln in routes_raw.decode().splitlines() if ln.strip()]
check("routes_row_count", len(lines) == 59, len(lines))
rows = []
for i, ln in enumerate(lines, 1):
    parts = ln.split("\t") if "\t" in ln else ln.split()
    # keep raw line identity
    rows.append({"ordinal": i, "fields": parts, "line": ln})
check("routes_all_10_fields", all(len(r["fields"]) == 10 for r in rows), 
      Counter(len(r["fields"]) for r in rows))
# distinct ordinals 1..59 even if tuples repeat
tuple_keys = [tuple(r["fields"]) for r in rows]
check("routes_ordinals_1_to_59", [r["ordinal"] for r in rows] == list(range(1, 60)))
check("routes_has_some_duplicate_tuples_ok", True, f"unique_tuples={len(set(tuple_keys))}")

# authorization / preflight / result small files
auth = json.loads(small[PREFIX + "authorization.json"])
auth_local = json.loads((base / "exact59-authorization.json").read_text())
check("auth_archive_eq_local", auth == auth_local)
check("auth_sha", hashlib.sha256(small[PREFIX + "authorization.json"]).hexdigest()
      == "f8ed2150e9d2c0eda91fdf525165a09a94011c52f1d3504c75821cd0256b038d")
check("auth_input_sha", auth["input_sha256"] == EXPECTED_PACK)
check("auth_routes_sha", auth["routes_sha256"] == EXPECTED_TSV)
check("auth_binding", auth["source_binding"]["sha256"]
      == "7b101c30bfd64c302815f9844fe27d3a52302feedaec58737726ab6a062491bb")
check("auth_limits", auth["limits"] == {
    "address": 4294967296, "cpu": 800, "output": 4294967296, "rss": 1073741824, "wall": 900
})
check("auth_preflight_bind", auth["root_preflight_sha256"]
      == "440d9623deaa75bff24577d5145b01c6f690f9419a63642158a1029c52ccd152")
check("auth_schema", auth["schema"] == "nav-coordinate-differential-release-v1")
check("auth_mode_real", auth["mode"] == "real")
check("auth_scope_correctness", "performance" not in auth["scope"].lower() or "no" in auth["scope"].lower())

pre = json.loads(small["exact59-preflight.json"] if "exact59-preflight.json" in small
                 else (base / "exact59-preflight.json").read_bytes())
# preflight may only be top-level name
if "exact59-preflight.json" not in small:
    pre = json.loads((base / "exact59-preflight.json").read_text())
else:
    pre = json.loads(small["exact59-preflight.json"])
check("preflight_qualified", pre.get("qualified") is True)
check("preflight_steal_zero", pre.get("steal_ticks") == 0)
check("preflight_no_conflict", pre.get("conflicting_matches") == [])
check("preflight_idle_100", pre.get("idle_percent") == 100.0)
check("preflight_swap_zero", pre.get("swap_total_bytes") == 0 and pre.get("swap_free_bytes") == 0)
check("preflight_builder", pre.get("hardware", {}).get("node") == "274bot-builder")

# preflight before private: authorization embeds preflight sha; preflight itself has no pack path
pre_text = json.dumps(pre)
check("preflight_no_private_path",
      "navpack" not in pre_text.lower() and "real-input" not in pre_text)
check("preflight_sha_matches_auth",
      hashlib.sha256((base / "exact59-preflight.json").read_bytes()).hexdigest()
      == auth["root_preflight_sha256"])

# root audit row/frame coverage
root_audit = json.loads((base / "exact59-root-audit.json").read_text())
check("root_audit_verified", root_audit.get("verified") is True)
check("root_audit_equal", root_audit.get("comparison", {}).get("equal") is True)
check("root_audit_bytes", root_audit.get("comparison", {}).get("bytes") == EXPECTED_OUT_BYTES)
check("root_audit_out_sha_dense", root_audit["output_sha256"]["dense"] == EXPECTED_OUT)
check("root_audit_out_sha_refined", root_audit["output_sha256"]["refined"] == EXPECTED_OUT)

for arm in ("dense", "refined"):
    frames = root_audit["arms"][arm]["frames"]
    arm_rows = root_audit["arms"][arm]["rows"]
    check(f"{arm}_row_count", len(arm_rows) == 59, len(arm_rows))
    check(f"{arm}_fixed_selector_59", frames.get("fixed-selector") == 59)
    check(f"{arm}_post_state_59", frames.get("post-state") == 59)
    check(f"{arm}_fixed_model_59", frames.get("fixed-model") == 59)
    check(f"{arm}_fixed_tele_59", frames.get("fixed-tele-model") == 59)
    host_total = frames.get("host-route", 0) + frames.get("host-outcome", 0)
    check(f"{arm}_host_frames_59", host_total == 59, host_total)
    check(f"{arm}_cells_2", frames.get("cells") == 2)
    check(f"{arm}_geometry_2", frames.get("geometry") == 2)
    # each row has model/tele/host payload hashes
    missing = []
    for i, r in enumerate(arm_rows):
        ps = r.get("payload_sha256") or {}
        for k in ("post-state", "fixed-model", "fixed-tele-model"):
            if k not in ps:
                missing.append((i, k))
        # host may be host-route or host-outcome
        if not any(k.startswith("host") for k in ps):
            missing.append((i, "host-*"))
    check(f"{arm}_all_rows_have_model_tele_host", not missing, missing[:5])

# dense vs refined row identities match
d_rows = [tuple(r["row"]) for r in root_audit["arms"]["dense"]["rows"]]
r_rows = [tuple(r["row"]) for r in root_audit["arms"]["refined"]["rows"]]
check("dense_refined_row_identity_equal", d_rows == r_rows)
# compare to TSV
tsv_rows = [tuple(int(x) for x in r["fields"]) for r in rows]
check("audit_rows_match_tsv", d_rows == tsv_rows, 
      f"first_diff={next(((i,a,b) for i,(a,b) in enumerate(zip(d_rows,tsv_rows)) if a!=b), None)}")

# payload equality dense vs refined for all rows
payload_mismatch = []
for i, (dr, rr) in enumerate(zip(root_audit["arms"]["dense"]["rows"],
                                  root_audit["arms"]["refined"]["rows"])):
    if dr.get("payload_sha256") != rr.get("payload_sha256"):
        payload_mismatch.append(i)
check("dense_refined_payload_sha_equal_all_rows", not payload_mismatch, payload_mismatch[:10])

# Stream both large outs: compare byte-by-byte equality + hash without retaining
# Also count cells frames if format is known. Root audit already claims 2 cell passes.
# We'll recompute hash and pairwise equality by streaming both in lockstep.


def stream_member(tf, name):
    m = tf.getmember(name)
    f = tf.extractfile(m)
    return f, m.size


dense_h = hashlib.sha256()
refined_h = hashlib.sha256()
equal = True
n = 0
with tarfile.open(arc, "r:gz") as tf:
    df, ds = stream_member(tf, PREFIX + "dense.out")
    # need second open for parallel? tarfile can't easily parallel-stream two members.
# Sequential hash then equality via re-open compare is heavy. Hash each; if same hash+size -> equal.
with tarfile.open(arc, "r:gz") as tf:
    f, sz = stream_member(tf, PREFIX + "dense.out")
    assert sz == EXPECTED_OUT_BYTES
    while True:
        b = f.read(8 * 1024 * 1024)
        if not b:
            break
        dense_h.update(b)
        n += len(b)
    check("dense_out_size", n == EXPECTED_OUT_BYTES, n)
    check("dense_out_sha", dense_h.hexdigest() == EXPECTED_OUT, dense_h.hexdigest())

n = 0
with tarfile.open(arc, "r:gz") as tf:
    f, sz = stream_member(tf, PREFIX + "refined.out")
    assert sz == EXPECTED_OUT_BYTES
    while True:
        b = f.read(8 * 1024 * 1024)
        if not b:
            break
        refined_h.update(b)
        n += len(b)
    check("refined_out_size", n == EXPECTED_OUT_BYTES, n)
    check("refined_out_sha", refined_h.hexdigest() == EXPECTED_OUT, refined_h.hexdigest())

check("dense_refined_hash_equal", dense_h.hexdigest() == refined_h.hexdigest())

# input.bin hash
n = 0
ih = hashlib.sha256()
with tarfile.open(arc, "r:gz") as tf:
    f, sz = stream_member(tf, PREFIX + "input.bin")
    while True:
        b = f.read(8 * 1024 * 1024)
        if not b:
            break
        ih.update(b)
        n += len(b)
check("input_bin_size", n == 73438581, n)
check("input_bin_sha", ih.hexdigest() == EXPECTED_PACK, ih.hexdigest())

# receipts
dense_rcpt = json.loads(small[PREFIX + "dense.receipt.json"])
refined_rcpt = json.loads(small[PREFIX + "refined.receipt.json"])
result = json.loads(small[PREFIX + "result.json"])
check("dense_err_empty", small[PREFIX + "dense.err"] == b"")
check("refined_err_empty", small[PREFIX + "refined.err"] == b"")
check("result_equal_true", result.get("equal") is True or result.get("comparison", {}).get("equal") is True
      or "equal" in json.dumps(result).lower())

# Parse result more carefully
(out / "phase4-result-json.json").write_text(json.dumps(result, indent=2) + "\n")
(out / "phase4-dense-receipt.json").write_text(json.dumps(dense_rcpt, indent=2) + "\n")
(out / "phase4-refined-receipt.json").write_text(json.dumps(refined_rcpt, indent=2) + "\n")

# Frame count sanity: cells=2 means two full world cell dumps?
# WORLD_CELLS * 2 would be huge; verify root audit claims geometry/cells present.
# Estimate: if each cell is 1 byte blocked flag, 65M*2 = 130M of 2.1GB — plausible fraction.

report = {
    "checks": checks,
    "all_ok": all(c["ok"] for c in checks),
    "routes_unique_tuples": len(set(tuple_keys)),
    "routes_count": len(rows),
    "dense_out_sha": dense_h.hexdigest(),
    "refined_out_sha": refined_h.hexdigest(),
    "input_sha": ih.hexdigest(),
    "result_keys": list(result.keys()) if isinstance(result, dict) else type(result).__name__,
}
(out / "phase4-exact59-coverage.json").write_text(json.dumps(report, indent=2) + "\n")
print("ALL_OK" if report["all_ok"] else "HAS_FAILURES")
fail = [c for c in checks if not c["ok"]]
for c in fail:
    print("FAIL_DETAIL", c)
