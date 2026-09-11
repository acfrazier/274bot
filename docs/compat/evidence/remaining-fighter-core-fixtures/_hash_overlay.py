import hashlib
from pathlib import Path

root = Path("/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision")
files = [
    "crates/scenario/src/lib.rs",
    "crates/host-play/tests/catalog_boundary_live.rs",
    "docs/compat/05u-remaining-fighter-core-fixtures.md",
    "docs/compat/briefs/109-remaining-fighter-core-fixtures.md",
]
for rel in files:
    path = root / rel
    print(hashlib.sha256(path.read_bytes()).hexdigest(), rel)
