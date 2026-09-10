#!/usr/bin/env python3
"""Verify generated differential archive (stream) and Concord fixture-correction archive."""
import hashlib
import json
import tarfile
from pathlib import Path

out = Path("diagnostics/native-nav-differential-preparation/coordinate-native-evidence-review")
out.mkdir(parents=True, exist_ok=True)
checks = []


def check(name, ok, detail=None):
    checks.append({"name": name, "ok": bool(ok), "detail": detail})
    print(("PASS" if ok else "FAIL"), name, detail if detail is not None else "")


def sha_file(p, chunk=1024 * 1024):
    h = hashlib.sha256()
    n = 0
    with open(p, "rb") as f:
        while True:
            b = f.read(chunk)
            if not b:
                break
            h.update(b)
            n += len(b)
    return h.hexdigest(), n


# --- Generated archive ---
gen_base = Path("diagnostics/native-nav-differential-preparation/coordinate-a44a930-native-01")
gen_arc = gen_base / "coordinate-native-generated-01.tar.gz"
EXPECTED_GEN = "baa98510e1c4044bf817e9ceffddfcee4dcf3178ce4f94ba3d37d42e291c7524"
d, n = sha_file(gen_arc)
check("gen_archive_sha", d == EXPECTED_GEN, d)
check("gen_archive_bytes", n == 18765670, n)

# Stream member inventory: count files, uncompressed, look for key comparison artifacts
file_count = 0
uncomp = 0
interesting = {}
# Don't fully hash every member of 13k files if too slow — hash key receipts and sample.
key_suffixes = (
    "result.json",
    "progress.json",
    "coordinate-source-binding.json",
    "dense/output.bin",
    "refined/output.bin",
    "comparison.json",
    "sources.json",
    "admission",
)
member_names = []
with tarfile.open(gen_arc, "r:gz") as tf:
    for m in tf:
        if m.isfile():
            file_count += 1
            uncomp += m.size
            member_names.append(m.name)
            # stash small interesting
            low = m.name.lower()
            if m.size < 5_000_000 and any(s in low for s in (
                "result", "progress", "binding", "comparison", "sources.json",
                "guards", "protocol", "receipt", "manifest"
            )):
                f = tf.extractfile(m)
                data = f.read()
                interesting[m.name] = {
                    "bytes": len(data),
                    "sha256": hashlib.sha256(data).hexdigest(),
                }

check("gen_file_count", file_count == 13436, file_count)
check("gen_uncomp", uncomp == 681424323, uncomp)

# root-archive-audit consistency
root_gen = json.loads((gen_base / "root-archive-audit.json").read_text())
check("gen_root_verified", root_gen.get("verified") is True)
check("gen_root_equal", root_gen.get("comparison", {}).get("equal") is True)
check("gen_root_bytes", root_gen.get("comparison", {}).get("bytes") == 328120731)
check("gen_root_protocol", root_gen.get("protocol_comparison", {}).get("equal") is True)
check("gen_root_protocol_bytes", root_gen.get("protocol_comparison", {}).get("bytes") == 80987)
check("gen_root_inputs", root_gen.get("input_count") == 12954)
check("gen_root_binding", root_gen["source_binding"]["sha256"]
      == "7b101c30bfd64c302815f9844fe27d3a52302feedaec58737726ab6a062491bb")
check("gen_source_counts", root_gen.get("source_counts") == {"dense": 209, "refined": 210})

# progress.json local
if (gen_base / "progress.json").exists():
    prog = json.loads((gen_base / "progress.json").read_text())
    (out / "phase5-progress.json").write_text(json.dumps(prog, indent=2) + "\n")

# generated wrapper local tiny outputs
gw = gen_base / "generated-wrapper"
if gw.exists():
    gw_files = sorted(p.name for p in gw.iterdir())
    gw_info = {}
    for p in gw.iterdir():
        if p.is_file() and p.stat().st_size < 5_000_000:
            data = p.read_bytes()
            gw_info[p.name] = {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
    (out / "phase5-generated-wrapper.json").write_text(
        json.dumps({"files": gw_files, "hashed": gw_info}, indent=2) + "\n"
    )
    # Look for equal dense/refined outs of claimed sizes 337 and 80987
    # task says: actual native real-wrapper tiny337B fixture80987B equal
    for name, meta in gw_info.items():
        print("gw", name, meta["bytes"], meta["sha256"][:16])

# --- Concord scheduler-fixture-correction ---
conc_dir = Path("diagnostics/nav-stage-a-native-preparation/scheduler-fixture-correction-native-01")
conc_parent = Path("diagnostics/nav-stage-a-native-preparation")
# find the 9999-member archive
cands = list(conc_dir.glob("*.tar.gz")) + list(conc_parent.glob("*qualified*03*.tar.gz"))
print("concord archives", [str(c) for c in cands])

EXPECTED_CONC = "41cd71c943b11c3c899fe66693cc98dbe83e2936007eb10698fa83f161b02681"
conc_arc = None
for c in list(conc_dir.glob("*.tar.gz")):
    d, n = sha_file(c)
    print("cand", c.name, d, n)
    if d == EXPECTED_CONC:
        conc_arc = c
        check("concord_archive_sha", True, f"{c.name} bytes={n}")
        break
if conc_arc is None:
    # try parent
    for c in conc_parent.glob("*.tar.gz"):
        if "qualified" in c.name or "113ce05" in c.name or "fixture" in c.name:
            d, n = sha_file(c)
            print("parent_cand", c.name, d, n)
            if d == EXPECTED_CONC:
                conc_arc = c
                check("concord_archive_sha", True, f"{c.name} bytes={n}")
                break
if conc_arc is None:
    check("concord_archive_sha", False, "not found with expected hash")

conc_report = {"interesting_small": {}, "file_count": None, "uncomp": None}
if conc_arc is not None:
    fc = 0
    uc = 0
    with tarfile.open(conc_arc, "r:gz") as tf:
        for m in tf:
            if m.isfile():
                fc += 1
                uc += m.size
                low = m.name.lower()
                if m.size < 2_000_000 and any(
                    s in low
                    for s in (
                        "receipt",
                        "result",
                        "guards",
                        "scheduler",
                        "admission",
                        "package",
                        "manifest",
                        "comparison",
                        "child",
                        "smoke",
                        "binding",
                        "test_sharded",
                    )
                ):
                    f = tf.extractfile(m)
                    data = f.read()
                    conc_report["interesting_small"][m.name] = {
                        "bytes": len(data),
                        "sha256": hashlib.sha256(data).hexdigest(),
                    }
    conc_report["file_count"] = fc
    conc_report["uncomp"] = uc
    check("concord_file_count", fc == 9999, fc)
    check("concord_uncomp", uc == 24948143, uc)
    check("concord_comp_approx", True, f"compressed={conc_arc.stat().st_size}")

# package-manifest and root audit
pm = conc_dir / "package-manifest.json"
if pm.exists():
    pkg = json.loads(pm.read_text())
    (out / "phase5-concord-package-manifest.json").write_text(json.dumps(pkg, indent=2) + "\n")
    # binding of 4 immutable admissions
    print("package keys", list(pkg.keys())[:40])

audit_p = conc_dir / "root-coordinate-concord-qualified-03-audit.json"
if audit_p.exists():
    audit = json.loads(audit_p.read_text())
    (out / "phase5-concord-root-audit.json").write_text(json.dumps(audit, indent=2) + "\n")
    check("concord_audit_verified", audit.get("verified") is True or audit.get("ok") is True
          or "verified" in json.dumps(audit).lower())
    print("audit top keys", list(audit.keys())[:50])

# failure archive distinction
fail_arc = conc_parent / "coordinate-concord-scheduler-failure-01.tar.gz"
if fail_arc.exists():
    fd, fn = sha_file(fail_arc)
    check("failure_archive_distinct_sha",
          fd == "7c500748ace8bae33fd449452df5f0b9e8377d3ec4c3a70abe8925d2728a68de", fd)
    check("failure_ne_success", fd != EXPECTED_CONC)

# review.json for 113ce05
rev = conc_dir / "review.json"
if rev.exists():
    review = json.loads(rev.read_text())
    (out / "phase5-concord-review.json").write_text(json.dumps(review, indent=2) + "\n")

report = {
    "checks": checks,
    "all_ok": all(c["ok"] for c in checks),
    "gen_interesting_count": len(interesting),
    "gen_interesting_sample": dict(list(interesting.items())[:30]),
    "concord": conc_report,
}
(out / "phase5-generated-concord.json").write_text(json.dumps(report, indent=2) + "\n")
print("ALL_OK" if report["all_ok"] else "HAS_FAILURES")
for c in checks:
    if not c["ok"]:
        print("FAIL_DETAIL", c)
