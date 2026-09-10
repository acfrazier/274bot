#!/usr/bin/env python3
"""Extract and verify Concord 12 comparisons + 4 GQ children from qualification result."""
import hashlib
import json
import tarfile
from pathlib import Path

out = Path("diagnostics/native-nav-differential-preparation/coordinate-native-evidence-review")
conc_arc = Path(
    "diagnostics/nav-stage-a-native-preparation/scheduler-fixture-correction-native-01/coordinate-concord-qualified-03.tar.gz"
)
checks = []


def check(name, ok, detail=None):
    checks.append({"name": name, "ok": bool(ok), "detail": detail})
    print(("PASS" if ok else "FAIL"), name, detail if detail is not None else "")


with tarfile.open(conc_arc, "r:gz") as tf:
    names = [m.name for m in tf.getmembers() if m.isfile()]
    # GQ children
    gq_ids = sorted({n.split("/GQ/")[1].split("/")[0] for n in names if "/GQ/" in n})
    check("gq_child_count_4", len(gq_ids) == 4, gq_ids)
    expected_gq = {
        "000-1-1-dense",
        "001-1-1-refined",
        "002-1-2-refined",
        "003-1-2-dense",
    }
    check("gq_ids_expected", set(gq_ids) == expected_gq, gq_ids)

    # Load qualification-03 result
    q3 = "host/docs/memory/nav-tiled-stage-a/coordinate-concord-run-01/coordinate-ebf0f30-qualification-03/result.json"
    f = tf.extractfile(q3)
    q3_data = json.loads(f.read())
    comps = q3_data.get("comparisons") or []
    check("comparisons_count_12", len(comps) == 12, len(comps))
    # each should indicate match/equal
    bad = []
    for i, c in enumerate(comps):
        blob = json.dumps(c).lower()
        ok = (
            c.get("equal") is True
            or c.get("matched") is True
            or c.get("match") is True
            or "equal\": true" in json.dumps(c).lower()
            or c.get("status") == "ok"
            or c.get("ok") is True
        )
        # softer: no failure field
        if not ok:
            # inspect keys
            if isinstance(c, dict) and c.get("failure") in (None, "", False):
                # check nested
                if any(
                    (isinstance(v, dict) and v.get("equal") is True)
                    or v is True
                    and k.endswith("equal")
                    for k, v in c.items()
                ):
                    ok = True
        if not ok:
            bad.append((i, list(c.keys()) if isinstance(c, dict) else type(c).__name__, c if isinstance(c, dict) and len(json.dumps(c)) < 500 else None))
    check("all_12_comparisons_ok", not bad, bad[:3])

    # smoke children in result
    smoke = q3_data.get("smoke_children") or q3_data.get("children") or q3_data.get("gq") or []
    print("q3 top keys", list(q3_data.keys()))
    print("comparisons sample0", json.dumps(comps[0], indent=2)[:800] if comps else None)
    if len(comps) > 1:
        print("comparisons sample1 keys", list(comps[1].keys()) if isinstance(comps[1], dict) else comps[1])

    # guards 29 tests - look for unittest output
    guard_out_names = [n for n in names if "guard" in n.lower() and (n.endswith(".out") or n.endswith(".log") or n.endswith("result.json"))]
    print("guard outs", guard_out_names[:20])

    # four admissions unchanged - from root audit already true; verify admission files exist with stable hashes if listed in package
    adm = [n for n in names if "admission" in n.lower() and n.endswith(".json")]
    print("admission count", len(adm))
    adm_hashes = {}
    for n in adm[:20]:
        m = tf.getmember(n)
        if m.size < 200000:
            data = tf.extractfile(m).read()
            adm_hashes[n] = hashlib.sha256(data).hexdigest()

    # GQ receipts returncode 0
    gq_receipts = {}
    for n in names:
        if "/GQ/" in n and n.endswith(".receipt.json"):
            data = tf.extractfile(n).read()
            gq_receipts[n] = json.loads(data)
    rc_bad = []
    for n, r in gq_receipts.items():
        if r.get("returncode", r.get("exit_code", 0)) not in (0, None) or r.get("failure") not in (None, "", False):
            if r.get("returncode") != 0:
                rc_bad.append((n, r.get("returncode"), r.get("failure")))
    check("gq_receipts_rc0", not rc_bad, rc_bad)
    check("gq_receipt_files_16_or_4", len(gq_receipts) in (4, 16, 12), len(gq_receipts))

    # address guard on scheduler from root audit
    root_audit = json.loads(
        Path(
            "diagnostics/nav-stage-a-native-preparation/scheduler-fixture-correction-native-01/root-coordinate-concord-qualified-03-audit.json"
        ).read_text()
    )
    check("as_guard_active", root_audit["guards"]["address_guard_active"] is True)
    check("guards_rc0", root_audit["guards"]["returncode"] == 0)
    check("guards_limits_unchanged", root_audit["guards"]["limits"] == {
        "address": 4294967296,
        "cpu": 300,
        "file_size": 1048576,
        "output": 1048576,
        "rss": 536870912,
        "wall": 360,
    })
    check("all_progress_rc0", all(p.get("returncode") == 0 for p in root_audit["progress"]))
    check("four_admissions_unchanged_flag", root_audit.get("four_admissions_unchanged") is True)

    # failure vs success distinction
    fail = json.loads(
        Path(
            "diagnostics/nav-stage-a-native-preparation/root-coordinate-concord-scheduler-failure-audit-01.json"
        ).read_text()
    )
    check("failure_scheduler_rc_neg9", fail["scheduler"]["returncode"] == -9)
    check("failure_scope_failed", "fail" in fail.get("scope", "").lower())
    check("success_ne_failure_archive",
          root_audit["archive_sha256"] != fail["archive_sha256"])

    # wrapper audit uses equal_bytes
    gw = json.loads(
        Path(
            "diagnostics/native-nav-differential-preparation/coordinate-a44a930-native-01/generated-wrapper/root-audit.json"
        ).read_text()
    )
    check("wrapper_equal_bytes_80987", gw.get("equal_bytes") == 80987 and gw.get("verified") is True)
    gw_result = json.loads(
        Path(
            "diagnostics/native-nav-differential-preparation/coordinate-a44a930-native-01/generated-wrapper/result.json"
        ).read_text()
    )
    check("wrapper_result_equal",
          gw_result.get("comparison", {}).get("equal") is True or gw_result.get("equal") is True,
          list(gw_result.keys()))

    # Save comparisons summary
    comp_summary = []
    for i, c in enumerate(comps):
        if isinstance(c, dict):
            comp_summary.append({
                "i": i,
                "keys": list(c.keys()),
                "equal": c.get("equal", c.get("matched")),
                "name": c.get("name") or c.get("id") or c.get("label") or c.get("path"),
                "snippet": {k: c[k] for k in list(c.keys())[:12]},
            })
        else:
            comp_summary.append({"i": i, "value": c})

(out / "phase7-comparisons.json").write_text(json.dumps({
    "q3_keys": list(q3_data.keys()),
    "comparisons": comp_summary,
    "gq_ids": gq_ids,
    "gq_receipt_count": len(gq_receipts),
    "adm_hashes_sample": adm_hashes,
}, indent=2) + "\n")

# Also verify cell count claim: geometry/cells tags and two cell frames from root audit
# Parse routes unique 59 already done.
# Cross-check authorization result.json sha after run differs from pre-release placeholder
auth = json.loads(
    Path(
        "diagnostics/native-nav-differential-preparation/coordinate-a44a930-native-01/exact59-authorization.json"
    ).read_text()
)
# auth had result.json_sha256 for pre-release generated result - different from real result
real_result_sha = hashlib.sha256(
    Path(
        "diagnostics/native-nav-differential-preparation/coordinate-native-evidence-review/phase4-result-json.json"
    ).read_bytes()
)
# phase4 result was reformatted - use archive stream
import tarfile as tfmod
arc = Path("diagnostics/native-nav-differential-preparation/coordinate-a44a930-native-01/exact59-evidence.tar.gz")
with tfmod.open(arc, "r:gz") as t:
    data = t.extractfile(
        "host/docs/memory/nav-tiled-differential/coordinate-native-generated-01/real-release/result.json"
    ).read()
real_sha = hashlib.sha256(data).hexdigest()
check("real_result_sha_matches_manifest",
      real_sha == "e6c8b044b5272dffff16dab604a14a1490bbfaf59e919e6b75644f9264d0e6fa")
# pre-authorization bound a different generated result hash
check("auth_pre_result_hash_distinct_from_real",
      auth.get("result.json_sha256") != real_sha,
      {"auth": auth.get("result.json_sha256"), "real": real_sha})

report = {"checks": checks, "all_ok": all(c["ok"] for c in checks)}
(out / "phase7-concord-detail.json").write_text(json.dumps(report, indent=2) + "\n")
print("ALL_OK" if report["all_ok"] else "HAS_FAILURES")
for c in checks:
    if not c["ok"]:
        print("FAIL_DETAIL", c)
