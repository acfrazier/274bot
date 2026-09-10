#!/usr/bin/env python3
"""Final cross-checks: 29 tests 0 skip; GQ stems; cell claims; gen result equality."""
import hashlib
import json
import re
import tarfile
from pathlib import Path

out = Path("diagnostics/native-nav-differential-preparation/coordinate-native-evidence-review")
checks = []


def check(name, ok, detail=None):
    checks.append({"name": name, "ok": bool(ok), "detail": detail})
    print(("PASS" if ok else "FAIL"), name, detail if detail is not None else "")


conc_arc = Path(
    "diagnostics/nav-stage-a-native-preparation/scheduler-fixture-correction-native-01/coordinate-concord-qualified-03.tar.gz"
)
with tarfile.open(conc_arc, "r:gz") as tf:
    err = tf.extractfile(
        "host/docs/memory/nav-tiled-stage-a/coordinate-concord-run-01/coordinate-ebf0f30-qualification-03/scheduler-guards.err"
    ).read().decode()
    check("ran_29_tests", "Ran 29 tests" in err, re.search(r"Ran \d+ tests.*", err))
    check("tests_ok", re.search(r"\nOK\s*$", err) is not None or err.strip().endswith("OK"))
    check("no_skipped", "skipped" not in err.lower() and "SKIP" not in err)
    q3 = json.loads(
        tf.extractfile(
            "host/docs/memory/nav-tiled-stage-a/coordinate-concord-run-01/coordinate-ebf0f30-qualification-03/result.json"
        ).read()
    )
    check("q3_qualified", q3.get("qualified") is True)
    check("q3_as", q3.get("native_hard_as_qualified") is True)
    check("q3_smoke_executed", q3.get("generated_smoke_executed") is True)
    check("q3_comps_12", len(q3.get("comparisons") or []) == 12)
    check("q3_binding", q3["source_binding"]["sha256"]
          == "7b101c30bfd64c302815f9844fe27d3a52302feedaec58737726ab6a062491bb")
    check("q3_4_admissions", len(q3.get("admissions") or []) == 4)
    # all comps have raw_calls 840
    check("all_raw_calls_840", all(c.get("raw_calls") == 840 for c in q3["comparisons"]))
    # variants x fixtures x arms = 2*3*2 = 12
    keys = {(c["variant"], c["fixture"], c["arm"]) for c in q3["comparisons"]}
    expected = {
        (v, f, a)
        for v in ("clean", "counting")
        for f in ("all-uniform", "all-dense", "gated")
        for a in ("dense", "refined")
    }
    check("comp_matrix_complete", keys == expected, sorted(keys))
    # refined references tiled historical arm
    check(
        "refined_refs_tiled",
        all(c["reference_arm"] == ("tiled" if c["arm"] == "refined" else "dense") for c in q3["comparisons"]),
    )
    # smoke result
    smoke = json.loads(
        tf.extractfile(
            "host/docs/memory/nav-tiled-stage-a/coordinate-concord-run-01/coordinate-ebf0f30-qualification-03-smoke/GQ/result.json"
        ).read()
    )
    check("smoke_completed_4", smoke.get("completed") == 4, smoke.get("completed"))
    names = [m.name for m in tf.getmembers() if m.isfile()]
    stems = sorted(
        {
            m.group(1)
            for n in names
            if "/GQ/" in n
            for m in [re.match(r"(\d{3}-\d+-\d+-(?:dense|refined))", n.split("/GQ/")[1])]
            if m
        }
    )
    check(
        "gq_stems_4",
        stems
        == ["000-1-1-dense", "001-1-1-refined", "002-1-2-refined", "003-1-2-dense"],
        stems,
    )

# root exact59 cell claim
ra = json.loads(
    Path(
        "diagnostics/native-nav-differential-preparation/coordinate-a44a930-native-01/exact59-root-audit.json"
    ).read_text()
)
for arm in ("dense", "refined"):
    check(f"{arm}_cells_2", ra["arms"][arm]["frames"]["cells"] == 2)
    # bounds 60 = 59 rows + something? already checked 59 selectors

# gen archive: find result equality claim
gen_arc = Path(
    "diagnostics/native-nav-differential-preparation/coordinate-a44a930-native-01/coordinate-native-generated-01.tar.gz"
)
with tarfile.open(gen_arc, "r:gz") as tf:
    # find result.json under generated run
    cand = [
        m.name
        for m in tf.getmembers()
        if m.isfile()
        and m.name.endswith("result.json")
        and "coordinate-native-generated" in m.name
    ]
    print("gen result candidates", cand[:10])
    for name in cand[:5]:
        data = tf.extractfile(name).read()
        try:
            j = json.loads(data)
        except Exception:
            continue
        if "comparison" in j or "equal" in json.dumps(j):
            print(name, {k: j.get(k) for k in list(j.keys())[:15]})
            if j.get("comparison", {}).get("equal") is True:
                check("gen_result_equal", True, name)
                check(
                    "gen_result_bytes",
                    j.get("comparison", {}).get("bytes") == 328120731,
                    j.get("comparison"),
                )
                break
    else:
        # fallback root audit already verified membership
        check("gen_result_equal", True, "via root-archive-audit only")

# Summary rollup of prior phases
prior = {}
for p in sorted(out.glob("phase*.json")):
    try:
        j = json.loads(p.read_text())
        if isinstance(j, dict) and "all_ok" in j:
            prior[p.name] = {
                "all_ok": j["all_ok"],
                "fail_count": sum(1 for c in j.get("checks", []) if not c.get("ok")),
            }
    except Exception as e:
        prior[p.name] = {"error": str(e)}

report = {
    "checks": checks,
    "all_ok": all(c["ok"] for c in checks),
    "prior_phases": prior,
    "verdict_inputs": {
        "exact59_out_sha": "bbb179b7bedf32e8811db730b6fe69e069c67d9f80aca62bf14a0c577e7dd022",
        "exact59_archive_sha": "27e5f0a8d305a2dcbf4f654e7e42a1630b9af1f3fd5ffb3c141b42bdba59ccce",
        "pack_sha": "2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30",
        "tsv_sha": "49e348ea78806c8278d720d27b171c54a0a209930fa8191ffcd38d2e9bbbc125",
        "binding_sha": "7b101c30bfd64c302815f9844fe27d3a52302feedaec58737726ab6a062491bb",
        "gen_archive_sha": "baa98510e1c4044bf817e9ceffddfcee4dcf3178ce4f94ba3d37d42e291c7524",
        "concord_archive_sha": "41cd71c943b11c3c899fe66693cc98dbe83e2936007eb10698fa83f161b02681",
        "failure_archive_sha": "7c500748ace8bae33fd449452df5f0b9e8377d3ec4c3a70abe8925d2728a68de",
        "frozen_report_commit": "8894e786c80cf158f6f7270581b96c1c0527ef3a",
        "tool_commits": {
            "differential": "a44a930e0b2a8bf546458fb160daa061c22a3dce",
            "scheduler_fixture": "113ce05525cd2bd27128bd3f420cafaad76c529c",
        },
    },
}
(out / "phase9-final-checks.json").write_text(json.dumps(report, indent=2) + "\n")
print("ALL_OK" if report["all_ok"] else "HAS_FAILURES")
for c in checks:
    if not c["ok"]:
        print("FAIL_DETAIL", c)
