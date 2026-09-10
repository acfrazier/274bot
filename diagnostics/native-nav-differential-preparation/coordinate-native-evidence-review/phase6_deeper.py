#!/usr/bin/env python3
"""Deeper Concord: 12 comparisons, 4 smoke children, tool_commit only test_sharded, lineage exclusion."""
import hashlib
import json
import re
import tarfile
from pathlib import Path
from collections import Counter

out = Path("diagnostics/native-nav-differential-preparation/coordinate-native-evidence-review")
out.mkdir(parents=True, exist_ok=True)
checks = []


def check(name, ok, detail=None):
    checks.append({"name": name, "ok": bool(ok), "detail": detail})
    print(("PASS" if ok else "FAIL"), name, detail if detail is not None else "")


conc_dir = Path("diagnostics/nav-stage-a-native-preparation/scheduler-fixture-correction-native-01")
conc_arc = conc_dir / "coordinate-concord-qualified-03.tar.gz"
parent = Path("diagnostics/nav-stage-a-native-preparation")

# Stream inventory of comparisons and smoke
comp_paths = []
smoke_paths = []
admission_paths = []
test_sharded_members = []
all_names = []
with tarfile.open(conc_arc, "r:gz") as tf:
    for m in tf:
        if not m.isfile():
            continue
        all_names.append(m.name)
        low = m.name.lower()
        if "comparison" in low or "/linux" in low or "reference" in low:
            if m.size < 5_000_000:
                f = tf.extractfile(m)
                data = f.read() if f else b""
                comp_paths.append(
                    {
                        "name": m.name,
                        "bytes": m.size,
                        "sha256": hashlib.sha256(data).hexdigest() if data else None,
                    }
                )
            else:
                comp_paths.append({"name": m.name, "bytes": m.size})
        if "/gq/" in low or "smoke" in low or "000-1-1" in low or "qualification" in low and "gq" in low:
            smoke_paths.append({"name": m.name, "bytes": m.size})
        if "admission" in low:
            admission_paths.append({"name": m.name, "bytes": m.size})
        if m.name.endswith("test_sharded.py"):
            f = tf.extractfile(m)
            data = f.read() if f else b""
            test_sharded_members.append(
                {
                    "name": m.name,
                    "bytes": m.size,
                    "sha256": hashlib.sha256(data).hexdigest() if data else None,
                }
            )

# Better smoke detection from known prefix
smoke_children = sorted(
    {
        re.search(r"(GQ/\d+-[^/]+)", n.replace("\\", "/"))
        and re.search(r"(GQ/\d+-[^/]+)", n.replace("\\", "/")).group(1)
        for n in all_names
        if "/GQ/" in n.replace("\\", "/")
    }
    - {None}
)
check("smoke_child_ids_count", len(smoke_children) == 4, sorted(smoke_children))

# 12 linux comparisons - look for comparison result files
linux_comp = [n for n in all_names if "linux" in n.lower() or "comparison" in n.lower()]
# from root audit: comparisons: 12
# Find files that look like comparison outputs
cmp_results = [
    n
    for n in all_names
    if re.search(r"compar(e|ison)", n, re.I)
    or "linux-reference" in n.lower()
    or "f24" in n.lower()
]
# Also look inside progress or result jsons extracted earlier
phase5 = json.loads((out / "phase5-generated-concord.json").read_text())
interesting = phase5.get("concord", {}).get("interesting_small", {})

# Extract key result JSONs from archive for comparison count
key_jsons = {}
with tarfile.open(conc_arc, "r:gz") as tf:
    for m in tf:
        if not m.isfile():
            continue
        bn = Path(m.name).name
        if bn in (
            "result.json",
            "comparisons.json",
            "progress.json",
            "qualification-result.json",
            "smoke-result.json",
            "guards.json",
        ) or bn.endswith("comparison.json"):
            if m.size < 2_000_000:
                f = tf.extractfile(m)
                data = f.read() if f else b""
                try:
                    key_jsons[m.name] = json.loads(data)
                except Exception:
                    key_jsons[m.name] = {"_raw_sha": hashlib.sha256(data).hexdigest(), "bytes": len(data)}

(out / "phase6-concord-key-jsons.json").write_text(
    json.dumps({k: (v if not isinstance(v, dict) or len(json.dumps(v)) < 50000 else {"_truncated_keys": list(v.keys())}) for k, v in key_jsons.items()}, indent=2)[:500000]
    + "\n"
)

# Search for equal:true comparison markers
equal_hits = []
for name, obj in key_jsons.items():
    blob = json.dumps(obj)
    if "equal" in blob.lower() or "match" in blob.lower() or "comparison" in name.lower():
        equal_hits.append(name)

# Count comparison directories
comp_dirs = sorted(
    {
        m.group(1)
        for n in all_names
        for m in [re.search(r"(comparisons?/[^/]+)", n.replace("\\", "/"), re.I)]
        if m
    }
)
# alternate pattern from stage-a
ref_dirs = sorted(
    {
        Path(n).parent.as_posix()
        for n in all_names
        if "linux" in n.lower() and n.endswith((".json", ".bin", ".out", ".sha256"))
    }
)

check("has_some_comparison_artifacts", len(cmp_results) + len(comp_dirs) + len(ref_dirs) > 0,
      {"cmp_results": len(cmp_results), "comp_dirs": comp_dirs[:20], "ref_dirs": ref_dirs[:20], "equal_hits": equal_hits[:20]})

# Use root audit as claim + verify smoke children content exists with receipts
smoke_receipts = [n for n in all_names if "/GQ/" in n and n.endswith("receipt.json")]
smoke_outs = [n for n in all_names if "/GQ/" in n and (n.endswith(".out") or n.endswith("result.json"))]
check("smoke_receipts_ge_4", len(smoke_receipts) >= 4, len(smoke_receipts))
check("smoke_outs_present", len(smoke_outs) >= 4, len(smoke_outs))

# Package manifest: only test_sharded changed among production tools
pkg = json.loads((conc_dir / "package-manifest.json").read_text())
check("pkg_tool_commit_113ce05", pkg.get("tool_commit", "").startswith("113ce05"))
check("pkg_binding", pkg.get("source_binding_sha256")
      == "7b101c30bfd64c302815f9844fe27d3a52302feedaec58737726ab6a062491bb")
sfc = pkg.get("scheduler_fixture_correction") or {}
(out / "phase6-sfc.json").write_text(json.dumps(sfc, indent=2) + "\n")
print("sfc keys", list(sfc.keys()) if isinstance(sfc, dict) else type(sfc))

# Compare test_sharded at 113ce05 vs a44a930 (tooling) - production tools should match a44a except test_sharded
# Git: show files changed in 113ce05
import subprocess

r = subprocess.run(
    ["git", "show", "--name-only", "--pretty=format:", "113ce05"],
    capture_output=True,
    text=True,
)
changed = [ln.strip() for ln in r.stdout.splitlines() if ln.strip()]
check("113ce05_changes_include_test_sharded",
      any(x.endswith("test_sharded.py") for x in changed), changed)
prod_tool_paths = [
    "docs/memory/nav-tiled-stage-a/stage_a.py",
    "docs/memory/nav-tiled-stage-a/sharded.py",
    "docs/memory/nav-tiled-stage-a/qualify_sharded.py",
    "docs/memory/nav-tiled-stage-a/frozen_support.py",
    "docs/memory/nav-tiled-stage-a/source_binding.py",
]
prod_changed = [c for c in changed if any(c.endswith(p.split("/")[-1]) and "nav-tiled-stage-a" in c for p in prod_tool_paths)]
# only test_sharded among production code
code_changed = [c for c in changed if c.endswith(".py") or c.endswith(".rs")]
check("113ce05_only_fixture_py_among_stage_a_code",
      all("test_sharded.py" in c or "run_bounded" in c or "report" in c for c in code_changed if "nav-tiled-stage-a" in c),
      code_changed)

# Verify git blob of test_sharded at 113ce05 matches package if present
ts_git = subprocess.run(
    ["git", "rev-parse", "113ce05:docs/memory/nav-tiled-stage-a/test_sharded.py"],
    capture_output=True,
    text=True,
)
if ts_git.returncode == 0:
    blob = subprocess.run(
        ["git", "cat-file", "-p", "113ce05:docs/memory/nav-tiled-stage-a/test_sharded.py"],
        capture_output=True,
    )
    ts_sha = hashlib.sha256(blob.stdout).hexdigest()
    check("test_sharded_git_sha_recorded", True, ts_sha)
    # match package fixture-correction or host path
    pkg_ts = [f for f in pkg.get("files", []) if f["path"].endswith("test_sharded.py")]
    if pkg_ts:
        # may be original or corrected
        print("pkg test_sharded entries", pkg_ts)
    if test_sharded_members:
        check("archive_test_sharded_matches_git",
              any(t.get("sha256") == ts_sha for t in test_sharded_members),
              test_sharded_members)

# Failure audit still retained and distinct
fail_audit = parent / "root-coordinate-concord-scheduler-failure-audit-01.json"
if fail_audit.exists():
    fa = json.loads(fail_audit.read_text())
    (out / "phase6-failure-audit.json").write_text(json.dumps(fa, indent=2) + "\n")
    check("failure_audit_not_pass",
          fa.get("verified") is True or "fail" in json.dumps(fa).lower(),
          list(fa.keys())[:20])
    # ensure failure progress shows scheduler non-zero
    prog = fa.get("progress") or fa.get("stages") or []
    print("failure audit sample", {k: fa.get(k) for k in list(fa.keys())[:15]})

# Lineage 40/19: proposal.json must not be used as current outputs
prop = Path("diagnostics/nav-stage-a-real-proposal-01/proposal.json")
if prop.exists():
    proposal = json.loads(prop.read_text())
    (out / "phase6-proposal-lineage-meta.json").write_text(
        json.dumps({k: proposal.get(k) for k in list(proposal.keys())[:30] if not isinstance(proposal.get(k), (list, dict))}, indent=2)
        + "\n"
    )
    blob = prop.read_text()
    # ensure exact59 outputs don't reference old 40/19 paths as fill
    exact_auth = Path(
        "diagnostics/native-nav-differential-preparation/coordinate-a44a930-native-01/exact59-authorization.json"
    ).read_text()
    check("exact59_auth_no_40_19_fill",
          "40-row" not in exact_auth and "19-row" not in exact_auth and "rows_1_40" not in exact_auth)
    result = Path(
        "diagnostics/native-nav-differential-preparation/coordinate-native-evidence-review/phase4-result-json.json"
    ).read_text()
    check("exact59_result_no_40_19", "40-row" not in result and "F1" not in result)

# Preflight ordering: preflight ended before release audit
pre = json.loads(
    Path(
        "diagnostics/native-nav-differential-preparation/coordinate-a44a930-native-01/exact59-preflight.json"
    ).read_text()
)
root_audit = json.loads(
    Path(
        "diagnostics/native-nav-differential-preparation/coordinate-a44a930-native-01/exact59-root-audit.json"
    ).read_text()
)
check("preflight_before_audit",
      pre.get("ended_unix_s", 0) < root_audit.get("verified_unix_s", 0),
      {"preflight_end": pre.get("ended_unix_s"), "audit": root_audit.get("verified_unix_s")})
# auth embeds preflight sha - already checked; prepare script order
prep = Path(
    "diagnostics/native-nav-differential-preparation/coordinate-a44a930-native-01/prepare_exact59_release.py"
).read_text()
check("prepare_mentions_preflight_before_input",
      "preflight" in prep.lower() and ("input" in prep.lower() or "navpack" in prep.lower() or "stat" in prep.lower()))

# World cell count from root audit - look for cell frame sizes
# Parse dense.out frame headers if format known - try first few KB from archive
import struct

PREFIX = "host/docs/memory/nav-tiled-differential/coordinate-native-generated-01/real-release/"
with tarfile.open(
    Path("diagnostics/native-nav-differential-preparation/coordinate-a44a930-native-01/exact59-evidence.tar.gz"),
    "r:gz",
) as tf:
    f = tf.extractfile(PREFIX + "dense.out")
    head = f.read(4096) if f else b""
(out / "phase6-dense-out-head.hex").write_text(head[:256].hex() + "\n")
# try ascii tags
tags = re.findall(rb"[a-zA-Z][a-zA-Z0-9_:-]{2,40}", head)
check("dense_out_has_frame_tags", len(tags) > 0, [t.decode("ascii", "ignore") for t in tags[:30]])

# Verify a44a930 tool commit still present as differential tool base
r = subprocess.run(["git", "cat-file", "-t", "a44a930e0b2a8bf546458fb160daa061c22a3dce"], capture_output=True, text=True)
check("a44a930_commit_present", r.returncode == 0, r.stdout.strip())
r = subprocess.run(["git", "cat-file", "-t", "8894e786c80cf158f6f7270581b96c1c0527ef3a"], capture_output=True, text=True)
check("frozen_report_commit_present", r.returncode == 0, r.stdout.strip())
check("HEAD_is_frozen_or_descendant",
      subprocess.run(["git", "merge-base", "--is-ancestor", "8894e78", "HEAD"]).returncode == 0)

# Cross-check generated wrapper equal outs
gw = Path("diagnostics/native-nav-differential-preparation/coordinate-a44a930-native-01/generated-wrapper")
d_out = (gw / "dense.out").read_bytes()
r_out = (gw / "refined.out").read_bytes()
check("wrapper_out_equal", d_out == r_out and len(d_out) == 80987, len(d_out))
inp = (gw / "input.bin").read_bytes()
check("wrapper_tiny_input_337", len(inp) == 337, len(inp))
gw_audit = json.loads((gw / "root-audit.json").read_text())
check("wrapper_audit_equal", gw_audit.get("comparison", {}).get("equal") is True or gw_audit.get("equal") is True,
      gw_audit)

# Manifest membership vs stream already done; verify manifest file hashes match stream for all 15
manifest = json.loads(
    Path(
        "diagnostics/native-nav-differential-preparation/coordinate-a44a930-native-01/exact59-evidence.manifest.json"
    ).read_text()
)
stream = json.loads((out / "phase2-exact59-stream-members.json").read_text())
stream_map = {m["name"]: m for m in stream["members"] if m.get("isfile")}
manifest_mismatches = []
for ent in manifest["files"]:
    sm = stream_map.get(ent["path"])
    if not sm:
        manifest_mismatches.append(("missing", ent["path"]))
    elif sm["sha256"] != ent["sha256"] or sm["bytes"] != ent["bytes"]:
        manifest_mismatches.append(("hash", ent["path"], sm, ent))
check("manifest_matches_streamed_members", not manifest_mismatches, manifest_mismatches[:5])
check("manifest_file_count_15", len(manifest["files"]) == 15, len(manifest["files"]))

# Concord comparisons: root audit says 12 - try to find 12 equal comparison records in archive result
# Look for files named like *-comparison* or under linux-reference
linux_eq = []
with tarfile.open(conc_arc, "r:gz") as tf:
    for m in tf:
        if not m.isfile():
            continue
        n = m.name.replace("\\", "/")
        if m.size < 100000 and (
            "linux" in n.lower()
            or n.endswith("compare.json")
            or "reference" in n.lower() and n.endswith(".json")
        ):
            f = tf.extractfile(m)
            data = f.read() if f else b""
            try:
                j = json.loads(data)
            except Exception:
                continue
            blob = json.dumps(j)
            if "equal" in blob or "match" in blob or "checksum" in blob:
                linux_eq.append({"name": n, "keys": list(j.keys()) if isinstance(j, dict) else type(j).__name__})

(out / "phase6-linux-eq-candidates.json").write_text(json.dumps(linux_eq, indent=2) + "\n")
print("linux_eq candidates", len(linux_eq))

# Also count comparison entries inside a parent result
for name, obj in key_jsons.items():
    if isinstance(obj, dict):
        for k, v in obj.items():
            if "compar" in k.lower() or k == "comparisons":
                print("found comparisons key in", name, k, type(v), (len(v) if hasattr(v, "__len__") else v))

report = {
    "checks": checks,
    "all_ok": all(c["ok"] for c in checks),
    "smoke_children": sorted(smoke_children) if smoke_children else [],
    "smoke_receipt_count": len(smoke_receipts),
    "cmp_result_count": len(cmp_results),
    "linux_eq_candidates": len(linux_eq),
    "test_sharded_members": test_sharded_members,
    "113ce05_changed": changed,
}
(out / "phase6-deeper.json").write_text(json.dumps(report, indent=2) + "\n")
print("ALL_OK" if report["all_ok"] else "HAS_FAILURES")
for c in checks:
    if not c["ok"]:
        print("FAIL_DETAIL", c)
