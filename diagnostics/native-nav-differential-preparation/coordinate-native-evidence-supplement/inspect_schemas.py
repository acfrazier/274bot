#!/usr/bin/env python3
"""Schema peek for corrective supplement — no multi-GB extract."""
import json
from pathlib import Path

OUT = Path("diagnostics/native-nav-differential-preparation/coordinate-native-evidence-supplement")
OUT.mkdir(parents=True, exist_ok=True)

def peek_json(path, max_keys=40):
    p = Path(path)
    raw = p.read_bytes()
    obj = json.loads(raw)
    info = {"path": str(p), "bytes": len(raw), "type": type(obj).__name__}
    if isinstance(obj, dict):
        info["keys"] = list(obj.keys())[:max_keys]
        for k, v in list(obj.items())[:max_keys]:
            if isinstance(v, list):
                info[f"len_{k}"] = len(v)
                if v and isinstance(v[0], dict):
                    info[f"sample0_{k}_keys"] = list(v[0].keys())
                    info[f"sample0_{k}"] = {kk: v[0][kk] for kk in list(v[0].keys())[:12]}
            elif isinstance(v, dict):
                info[f"keys_{k}"] = list(v.keys())[:30]
            else:
                info[f"val_{k}"] = v if not isinstance(v, str) or len(v) < 200 else v[:200]
    elif isinstance(obj, list):
        info["len"] = len(obj)
        if obj and isinstance(obj[0], dict):
            info["sample0_keys"] = list(obj[0].keys())
            info["sample0"] = {kk: obj[0][kk] for kk in list(obj[0].keys())[:12]}
    return info

paths = [
    "diagnostics/native-nav-differential-preparation/coordinate-a44a930-native-01/coordinate-native-generated-01-archive.json",
    "diagnostics/native-nav-differential-preparation/coordinate-a44a930-native-01/manifest.json",
    "diagnostics/nav-stage-a-native-preparation/scheduler-fixture-correction-native-01/coordinate-concord-qualified-03.manifest.json",
    "diagnostics/nav-stage-a-native-preparation/scheduler-fixture-correction-native-01/package-manifest.json",
    "diagnostics/nav-stage-a-native-preparation/scheduler-fixture-correction-native-01/review.json",
    "diagnostics/nav-stage-a-native-preparation/scheduler-fixture-correction-native-01/package-receipt.json",
]

report = []
for p in paths:
    try:
        report.append(peek_json(p))
    except Exception as e:
        report.append({"path": p, "error": str(e)})

(OUT / "inspect-schemas.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2)[:12000])
