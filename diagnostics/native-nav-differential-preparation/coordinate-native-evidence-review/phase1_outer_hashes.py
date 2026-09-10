#!/usr/bin/env python3
import hashlib
import json
from pathlib import Path

base = Path("diagnostics/native-nav-differential-preparation/coordinate-a44a930-native-01")
out = Path("diagnostics/native-nav-differential-preparation/coordinate-native-evidence-review")
out.mkdir(parents=True, exist_ok=True)


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


results = {}
for name in [
    "exact59-evidence.tar.gz",
    "coordinate-native-generated-01.tar.gz",
    "source.tar.gz",
]:
    p = base / name
    if p.exists():
        d, n = sha_file(p)
        results[name] = {"sha256": d, "bytes": n, "exists": True}
    else:
        results[name] = {"exists": False}

for name in [
    "exact59-authorization.json",
    "exact59-preflight.json",
    "exact59-evidence.manifest.json",
    "exact59-evidence.receipt.json",
    "exact59-root-archive-audit.json",
    "exact59-root-audit.json",
    "manifest.json",
    "root-archive-audit.json",
    "prepare_exact59_release.py",
    "audit_exact59.py",
    "audit_exact59_archive.py",
]:
    p = base / name
    if p.exists():
        d, n = sha_file(p)
        results[f"local:{name}"] = {"sha256": d, "bytes": n}

(out / "phase1-outer-hashes.json").write_text(json.dumps(results, indent=2) + "\n")
print(json.dumps(results, indent=2))
