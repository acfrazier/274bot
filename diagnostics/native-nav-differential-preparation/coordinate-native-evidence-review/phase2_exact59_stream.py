#!/usr/bin/env python3
"""Stream exact59 archive members; hash without retaining full bodies."""
import hashlib
import json
import tarfile
from pathlib import Path

base = Path("diagnostics/native-nav-differential-preparation/coordinate-a44a930-native-01")
out = Path("diagnostics/native-nav-differential-preparation/coordinate-native-evidence-review")
out.mkdir(parents=True, exist_ok=True)
arc = base / "exact59-evidence.tar.gz"
members = []
total_uncomp = 0
with tarfile.open(arc, "r:gz") as tf:
    for m in tf:
        if not m.isfile():
            members.append(
                {"name": m.name, "type": str(m.type), "size": m.size, "isfile": False}
            )
            continue
        f = tf.extractfile(m)
        h = hashlib.sha256()
        n = 0
        while True:
            b = f.read(1024 * 1024)
            if not b:
                break
            h.update(b)
            n += len(b)
        assert n == m.size, (m.name, n, m.size)
        total_uncomp += n
        members.append(
            {"name": m.name, "bytes": n, "sha256": h.hexdigest(), "isfile": True}
        )

report = {
    "archive": "exact59-evidence.tar.gz",
    "member_count": len(members),
    "file_count": sum(1 for m in members if m.get("isfile")),
    "uncompressed_bytes": total_uncomp,
    "members": members,
}
(out / "phase2-exact59-stream-members.json").write_text(
    json.dumps(report, indent=2) + "\n"
)
print("files", report["file_count"], "uncomp", total_uncomp)
for m in members:
    if m.get("isfile"):
        print(f"{m['bytes']:12d}  {m['sha256']}  {m['name']}")
