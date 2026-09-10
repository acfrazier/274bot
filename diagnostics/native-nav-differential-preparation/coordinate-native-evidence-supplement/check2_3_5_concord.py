#!/usr/bin/env python3
"""Supplement checks 2+3+5: Concord 9999 members vs manifest; four admissions
byte-identical to package-manifest; every admission source/executable hash vs
archived bytes; all 12 raw comparison field pairs (not flag-only).
"""
from __future__ import annotations

import hashlib
import json
import subprocess
import tarfile
import time
from pathlib import Path

OUT = Path(
    "diagnostics/native-nav-differential-preparation/coordinate-native-evidence-supplement"
)
BASE = Path(
    "diagnostics/nav-stage-a-native-preparation/scheduler-fixture-correction-native-01"
)
ARC = BASE / "coordinate-concord-qualified-03.tar.gz"
MANIFEST = BASE / "coordinate-concord-qualified-03.manifest.json"
RECEIPT = BASE / "coordinate-concord-qualified-03.receipt.json"
PACKAGE = BASE / "package-manifest.json"
EXPECTED_OUTER = (
    "41cd71c943b11c3c899fe66693cc98dbe83e2936007eb10698fa83f161b02681"
)
BINDING = "7b101c30bfd64c302815f9844fe27d3a52302feedaec58737726ab6a062491bb"
TOOL_COMMIT = "113ce05525cd2bd27128bd3f420cafaad76c529c"
ROOT_PREFIX = "/home/acfrazier/274bot-campaign/nav-coordinate-113ce05-concord-01/"
TOOL = "host/docs/memory/nav-tiled-stage-a/"
RUN = TOOL + "coordinate-concord-run-01/"
PROOF = RUN + "coordinate-ebf0f30-qualification-03/"
COMPARE_KEYS = (
    "aggregate",
    "logical_cells",
    "lookup_checksum",
    "narrow_checksum",
    "layout",
    "narrow_allocations",
    "narrow_requested_bytes",
)


def sha256(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def sha_file(p: Path) -> tuple[str, int]:
    h = hashlib.sha256()
    n = 0
    with open(p, "rb") as f:
        while True:
            c = f.read(1024 * 1024)
            if not c:
                break
            h.update(c)
            n += len(c)
    return h.hexdigest(), n


def check(checks, name, ok, detail=None):
    checks.append({"name": name, "ok": bool(ok), "detail": detail})
    print(("PASS" if ok else "FAIL"), name, "" if detail is None else detail)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    t0 = time.time()
    checks = []

    outer, outer_n = sha_file(ARC)
    check(checks, "concord_outer_sha", outer == EXPECTED_OUTER, outer)
    receipt = json.loads(RECEIPT.read_text())
    check(checks, "receipt_sha", receipt.get("sha256") == EXPECTED_OUTER)

    man = json.loads(MANIFEST.read_text())
    entries = {e["path"]: e for e in man["files"]}
    check(
        checks,
        "manifest_unique_9999",
        len(entries) == len(man["files"]) == 9999,
        len(man["files"]),
    )
    check(
        checks,
        "receipt_files_count",
        receipt.get("files") == 9999,
        receipt.get("files"),
    )

    data = {}
    mismatches = []
    with tarfile.open(ARC, "r:gz") as tf:
        for m in tf:
            if not m.isfile():
                continue
            if m.name not in entries:
                mismatches.append({"path": m.name, "err": "extra"})
                continue
            if m.name in data:
                mismatches.append({"path": m.name, "err": "duplicate"})
                continue
            b = tf.extractfile(m).read()
            e = entries[m.name]
            if len(b) != e["bytes"] or sha256(b) != e["sha256"]:
                mismatches.append(
                    {
                        "path": m.name,
                        "err": "hash_or_size",
                        "got_sha": sha256(b),
                        "got_bytes": len(b),
                        "exp_sha": e["sha256"],
                        "exp_bytes": e["bytes"],
                    }
                )
            data[m.name] = b

    missing = sorted(set(entries) - set(data))
    check(
        checks,
        "all_9999_members_hashed",
        not missing and not mismatches and len(data) == 9999,
        {
            "present": len(data),
            "missing": len(missing),
            "mismatch": len(mismatches),
            "mismatch_sample": mismatches[:5],
            "missing_sample": missing[:5],
        },
    )
    uncomp = sum(len(v) for v in data.values())
    check(checks, "concord_uncomp_24948143", uncomp == 24948143, uncomp)

    def obj(p):
        return json.loads(data[p])

    def ref(x):
        path = x["path"]
        assert path.startswith(ROOT_PREFIX), path
        rel = path[len(ROOT_PREFIX) :]
        assert sha256(data[rel]) == x["sha256"], rel
        return rel

    q = obj(PROOF + "result.json")
    check(checks, "q_qualified", q.get("qualified") is True)
    check(checks, "q_native_hard_as", q.get("native_hard_as_qualified") is True)
    check(checks, "q_smoke_executed", q.get("generated_smoke_executed") is True)
    check(checks, "q_candidate", q.get("candidate_id") == "coordinate-ebf0f30")
    check(checks, "q_phase_GQ", q.get("phase") == "GQ")
    bind_path = TOOL + "coordinate-source-binding.json"
    check(
        checks,
        "binding_in_archive",
        sha256(data[bind_path]) == BINDING == q["source_binding"]["sha256"],
    )

    # Tools match git 113ce05
    tool_bad = []
    for group in q["tools"].values():
        for n, h in group.items():
            b = data[TOOL + n]
            if sha256(b) != h:
                tool_bad.append(n + ":archive_hash")
                continue
            try:
                original = subprocess.check_output(
                    ["git", "show", f"{TOOL_COMMIT}:docs/memory/nav-tiled-stage-a/{n}"]
                )
                if b != original:
                    tool_bad.append(n + ":git_bytes")
            except subprocess.CalledProcessError as exc:
                tool_bad.append(n + f":git_err:{exc.returncode}")
    check(checks, "tools_vs_git_113ce05", not tool_bad, tool_bad)

    # Guards receipt + raw stderr
    for name, x in q["guards"].items():
        try:
            ref(x)
            check(checks, f"guard_ref_{name}", True)
        except Exception as exc:  # noqa: BLE001
            check(checks, f"guard_ref_{name}", False, str(exc))

    g = obj(PROOF + "scheduler-guards.receipt.json")
    check(checks, "guards_rc0", g.get("returncode") == 0 and g.get("failure") is None)
    check(checks, "guards_as_active", g.get("address_guard_active") is True)
    check(
        checks,
        "guards_limits",
        g.get("limits")
        == dict(
            address=4294967296,
            cpu=300,
            file_size=1048576,
            output=1048576,
            rss=536870912,
            wall=360,
        ),
        g.get("limits"),
    )
    err = data[PROOF + "scheduler-guards.err"]
    check(checks, "guards_ran_29", b"Ran 29 tests" in err)
    check(checks, "guards_OK", b"\nOK\n" in err)
    check(checks, "guards_no_skipped", b"skipped" not in err)

    # Smoke 4 children order + receipts
    smoke_path = ref(q["smoke"])
    smoke = obj(smoke_path)
    check(
        checks,
        "smoke_completed_4",
        smoke["completed"] == smoke["budget"]["children"] == 4
        and smoke["status"] == "complete",
        {"completed": smoke.get("completed"), "status": smoke.get("status")},
    )
    expected_order = [
        "000-1-1-dense",
        "001-1-1-refined",
        "002-1-2-refined",
        "003-1-2-dense",
    ]
    names = [x["name"] for x in smoke["entries"]]
    check(checks, "smoke_order", names == expected_order, names)
    smoke_rec_bad = []
    for x in smoke["entries"]:
        p = str(Path(smoke_path).parent / (x["name"] + ".record.json"))
        # pathlib may use OS sep; archive uses posix
        p = str(Path(smoke_path).as_posix().rsplit("/", 1)[0] + "/" + x["name"] + ".record.json")
        if sha256(data[p]) != x["record_sha256"]:
            smoke_rec_bad.append(x["name"])
    check(checks, "smoke_record_hashes", not smoke_rec_bad, smoke_rec_bad)

    # GQ child receipts rc=0 (native guards / 4 smoke receipts)
    gq_rc_bad = []
    gq_receipts = []
    for path in data:
        if "/GQ/" in path and path.endswith(".receipt.json"):
            r = json.loads(data[path])
            gq_receipts.append(path)
            if r.get("returncode", 0) != 0:
                gq_rc_bad.append((path, r.get("returncode")))
    check(checks, "gq_receipts_rc0", not gq_rc_bad, gq_rc_bad)
    check(
        checks,
        "gq_receipt_count_nonzero",
        len(gq_receipts) >= 4,
        len(gq_receipts),
    )

    # --- Check 3: four admissions vs package-manifest + source/executable ---
    package = json.loads(PACKAGE.read_text())
    original_files = {e["path"]: e for e in package["files"]}
    check(checks, "package_files_774", len(original_files) == 774, len(original_files))
    check(
        checks,
        "package_binding",
        package.get("source_binding_sha256") == BINDING,
    )
    check(checks, "package_tool_commit", package.get("tool_commit") == TOOL_COMMIT)

    admissions = q["admissions"]
    check(checks, "admissions_count_4", len(admissions) == 4, list(admissions))
    adm_detail = {}
    for key, x in admissions.items():
        try:
            p = ref(x)
            pkg_sha = original_files[p]["sha256"]
            arc_sha = sha256(data[p])
            match_pkg = arc_sha == pkg_sha == x["sha256"]
            a = obj(p)
            arm, variant = key.split("-")
            bind_ok = a["source_binding"] == q["source_binding"]
            src_bad = []
            for rel, h in a["source"].items():
                src_path = RUN + arm + "/" + rel
                if sha256(data[src_path]) != h:
                    src_bad.append(rel)
            candidates = [
                path
                for path in data
                if path.startswith(RUN + arm + "/")
                and path.endswith("/stage-a-probe")
                and f"target-{variant}/" in path
            ]
            exe_ok = (
                len(candidates) == 1
                and sha256(data[candidates[0]]) == a["executable_sha256"]
            )
            adm_detail[key] = {
                "path": p,
                "match_package_and_q_ref": match_pkg,
                "binding_ok": bind_ok,
                "source_count": len(a["source"]),
                "source_hash_failures": src_bad[:10],
                "source_hash_fail_count": len(src_bad),
                "executable_path": candidates[0] if candidates else None,
                "executable_ok": exe_ok,
                "ok": match_pkg
                and bind_ok
                and not src_bad
                and exe_ok,
            }
            check(checks, f"admission_{key}_package_bytes", match_pkg, {
                "pkg": pkg_sha,
                "arc": arc_sha,
                "q": x["sha256"],
            })
            check(checks, f"admission_{key}_binding", bind_ok)
            check(
                checks,
                f"admission_{key}_all_source_hashes",
                not src_bad,
                {"count": len(a["source"]), "bad": len(src_bad)},
            )
            check(checks, f"admission_{key}_executable", exe_ok, candidates)
        except Exception as exc:  # noqa: BLE001
            adm_detail[key] = {"ok": False, "error": f"{type(exc).__name__}: {exc}"}
            check(checks, f"admission_{key}", False, str(exc))

    check(
        checks,
        "all_four_admissions_independent",
        all(d.get("ok") for d in adm_detail.values()),
        {k: d.get("ok") for k, d in adm_detail.items()},
    )

    # --- Check 5: 12 raw comparison pairs field-by-field ---
    comps = q["comparisons"]
    check(checks, "comparisons_count_12", len(comps) == 12, len(comps))
    comp_results = []
    for i, c in enumerate(comps):
        try:
            new_path = (
                RUN
                + "qualification-"
                + c["variant"]
                + "/"
                + c["arm"]
                + "-"
                + c["fixture"]
                + ".out"
            )
            old_path = (
                TOOL
                + "run-05/qualification-"
                + c["variant"]
                + "/"
                + c["reference_arm"]
                + "-"
                + c["fixture"]
                + ".out"
            )
            new_b = data[new_path]
            old_b = data[old_path]
            sha_new_ok = sha256(new_b) == c["new_sha256"]
            sha_old_ok = sha256(old_b) == c["old_sha256"]
            n = json.loads(new_b.splitlines()[-1])
            o = json.loads(old_b.splitlines()[-1])
            field_eq = {k: n.get(k) == o.get(k) for k in COMPARE_KEYS}
            raw_n = n.get("raw_elapsed_ns")
            raw_ok = (
                isinstance(raw_n, list)
                and len(raw_n) == 840 == c["raw_calls"]
            )
            narrow_zero = (
                n.get("narrow_allocations") == 0
                and n.get("narrow_requested_bytes") == 0
            )
            ok = (
                sha_new_ok
                and sha_old_ok
                and all(field_eq.values())
                and raw_ok
                and narrow_zero
            )
            comp_results.append(
                {
                    "i": i,
                    "variant": c.get("variant"),
                    "arm": c.get("arm"),
                    "fixture": c.get("fixture"),
                    "reference_arm": c.get("reference_arm"),
                    "sha_new_ok": sha_new_ok,
                    "sha_old_ok": sha_old_ok,
                    "field_eq": field_eq,
                    "raw_calls": c.get("raw_calls"),
                    "raw_len": len(raw_n) if isinstance(raw_n, list) else None,
                    "narrow_zero": narrow_zero,
                    "ok": ok,
                }
            )
            check(
                checks,
                f"comp_{i}_{c.get('variant')}_{c.get('arm')}_{c.get('fixture')}",
                ok,
                {
                    "sha_new_ok": sha_new_ok,
                    "sha_old_ok": sha_old_ok,
                    "fields": field_eq,
                    "raw_ok": raw_ok,
                    "narrow_zero": narrow_zero,
                },
            )
        except Exception as exc:  # noqa: BLE001
            comp_results.append({"i": i, "ok": False, "error": str(exc), "c": c})
            check(checks, f"comp_{i}", False, str(exc))

    check(
        checks,
        "all_12_comparisons_fields",
        len(comp_results) == 12 and all(r.get("ok") for r in comp_results),
        {"ok_count": sum(1 for r in comp_results if r.get("ok"))},
    )

    # Progress 5 stages rc0
    progress = obj("root-concord-qualification-progress.json")
    check(
        checks,
        "progress_5_rc0",
        len(progress) == 5 and all(x.get("returncode") == 0 for x in progress),
        [{"name": x.get("name"), "rc": x.get("returncode")} for x in progress],
    )

    # q3 matches receipt qualification if present
    if "qualification" in receipt:
        check(
            checks,
            "receipt_qualification_eq_q3",
            receipt["qualification"] == q,
        )

    report = {
        "scope": "supplement checks 2+3+5 concord members, admissions, 12 comparisons",
        "archive": str(ARC),
        "outer_sha256": outer,
        "member_count": len(data),
        "uncompressed_bytes": uncomp,
        "admissions": adm_detail,
        "comparisons": comp_results,
        "checks": checks,
        "all_ok": all(c["ok"] for c in checks),
        "elapsed_s": time.time() - t0,
    }
    (OUT / "check2-concord-members.json").write_text(
        json.dumps(
            {
                "outer_sha256": outer,
                "member_count": len(data),
                "uncompressed_bytes": uncomp,
                "checks": [c for c in checks if c["name"].startswith(("concord_", "all_9999", "receipt_"))],
                "all_ok": all(
                    c["ok"]
                    for c in checks
                    if c["name"].startswith(("concord_", "all_9999", "receipt_"))
                ),
            },
            indent=2,
        )
        + "\n"
    )
    (OUT / "check3-admissions.json").write_text(
        json.dumps({"admissions": adm_detail, "all_ok": all(d.get("ok") for d in adm_detail.values())}, indent=2)
        + "\n"
    )
    (OUT / "check5-comparisons.json").write_text(
        json.dumps(
            {
                "comparisons": comp_results,
                "compare_keys": list(COMPARE_KEYS),
                "all_ok": all(r.get("ok") for r in comp_results),
                "count": len(comp_results),
            },
            indent=2,
        )
        + "\n"
    )
    (OUT / "check2-3-5-full.json").write_text(json.dumps(report, indent=2) + "\n")
    print("ALL_OK" if report["all_ok"] else "HAS_FAILURES")
    for c in checks:
        if not c["ok"]:
            print("FAIL_DETAIL", json.dumps(c)[:600])


if __name__ == "__main__":
    from pathlib import Path  # ensure Path in smoke path helper

    main()
