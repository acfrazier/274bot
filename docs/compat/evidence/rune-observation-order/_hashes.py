import hashlib
import json
from pathlib import Path

root = Path("/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision")
man = json.loads((root / ".superpowers/review-exports/t67dc0fab-rune-obs-manifest.json").read_text())
print("host", man["host_commit"])
print("client", man["client_commit"])
print("overlays", json.dumps(man["overlays"], indent=2))
owned = [
    "crates/scenario/src/lib.rs",
    "crates/scenario/src/runner.rs",
    "crates/host-play/tests/catalog_boundary_live.rs",
]
for name in owned:
    data = (root / name).read_bytes()
    digest = hashlib.sha256(data).hexdigest()
    print(name, digest, "manifest", man["files"].get(name), "match", digest == man["files"].get(name))
brief = root / "docs/compat/briefs/86-rune-observation-order.md"
print("brief", hashlib.sha256(brief.read_bytes()).hexdigest())
