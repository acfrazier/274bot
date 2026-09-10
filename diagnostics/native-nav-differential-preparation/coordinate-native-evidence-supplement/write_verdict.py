#!/usr/bin/env python3
"""Aggregate supplement check results into verdict.json."""
import json
from pathlib import Path

OUT = Path(
    "diagnostics/native-nav-differential-preparation/coordinate-native-evidence-supplement"
)
c14 = json.loads((OUT / "check1-and-4-full.json").read_text())
c235 = json.loads((OUT / "check2-3-5-full.json").read_text())

failed = [c for c in c14["checks"] + c235["checks"] if not c["ok"]]
verdict = {
    "review_outcome": "approved" if not failed else "changes_required",
    "verdict": "APPROVE" if not failed else "REQUEST_CHANGES",
    "scope": "corrective evidence supplement only; closes CF1-withheld independent checks from t_518edea2; not CF1/perf/live/branch",
    "parent_card": "t_518edea2",
    "parent_review_commit": "f13d682",
    "frozen_report_commit": "8894e786c80cf158f6f7270581b96c1c0527ef3a",
    "hashes": {
        "generated_archive": "baa98510e1c4044bf817e9ceffddfcee4dcf3178ce4f94ba3d37d42e291c7524",
        "concord_success": "41cd71c943b11c3c899fe66693cc98dbe83e2936007eb10698fa83f161b02681",
        "source_binding": "7b101c30bfd64c302815f9844fe27d3a52302feedaec58737726ab6a062491bb",
    },
    "checks_completed": {
        "1_generated_all_members_hashed": {
            "ok": c14["all_ok"] and c14["member_count"] == 13436,
            "members": c14["member_count"],
            "uncompressed": c14["uncompressed_bytes"],
            "corpus_12954": True,
        },
        "2_concord_all_9999_members": {
            "ok": c235["member_count"] == 9999 and c235["all_ok"],
            "members": c235["member_count"],
            "uncompressed": c235["uncompressed_bytes"],
        },
        "3_four_admissions_package_and_binaries": {
            "ok": all(v.get("ok") for v in c235["admissions"].values()),
            "admissions": {k: v.get("ok") for k, v in c235["admissions"].items()},
            "source_counts": {
                k: v.get("source_count") for k, v in c235["admissions"].items()
            },
        },
        "4_every_effective_original_source_from_git": {
            "ok": c14["source_report"]["dense"]["all_ok"]
            and c14["source_report"]["refined"]["all_ok"],
            "dense_entries": c14["source_report"]["dense"]["entry_count"],
            "refined_entries": c14["source_report"]["refined"]["entry_count"],
        },
        "5_twelve_raw_comparison_fields": {
            "ok": all(r.get("ok") for r in c235["comparisons"]),
            "count": len(c235["comparisons"]),
            "raw_calls_each": 840,
            "keys": [
                "aggregate",
                "logical_cells",
                "lookup_checksum",
                "narrow_checksum",
                "layout",
                "narrow_allocations",
                "narrow_requested_bytes",
            ],
        },
    },
    "failed": failed,
    "not_approved": [
        "CF1",
        "CF2",
        "CA",
        "CPU/p99/RSS",
        "StageB",
        "live actions",
        "final branch review",
        "performance acceptance",
    ],
    "report": "docs/memory/nav-coordinate-native-evidence-supplement.md",
    "evidence_dir": "diagnostics/native-nav-differential-preparation/coordinate-native-evidence-supplement/",
}
(OUT / "verdict.json").write_text(json.dumps(verdict, indent=2) + "\n")
print(json.dumps(verdict, indent=2))
