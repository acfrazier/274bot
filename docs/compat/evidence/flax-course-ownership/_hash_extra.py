#!/usr/bin/env python3
from pathlib import Path
import hashlib, json
ROOT = Path("/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision")
OUT = ROOT / "docs/compat/evidence/flax-course-ownership"
files = [
    ROOT / "vendor/fr-client-rust/crates/client/src/core/world.rs",
    ROOT / ".superpowers/review-exports/catalog-root-b1cff8a7/crates/api/src/interact.rs",
    ROOT / ".superpowers/review-exports/catalog-root-b1cff8a7/crates/script/src/shim/mod.rs",
    ROOT / "docs/compat/evidence/catalog-headed/r274-flax-picker-100adccc-b1cff8a7-gpu-diagnostic/shots/2026-09-11T06-21-11_29226/2026-09-11T06-22-46_flax_picker.png",
    ROOT / "docs/compat/evidence/catalog-headed/r274-flax-picker-100adccc-b1cff8a7-gpu-diagnostic/shots/2026-09-11T06-21-11_29226/2026-09-11T06-22-46_flax_picker.json",
]
rows = []
for p in files:
    b = p.read_bytes()
    rows.append({"path": str(p.relative_to(ROOT)), "sha256": hashlib.sha256(b).hexdigest(), "bytes": len(b)})
(OUT / "extra-hashes.json").write_text(json.dumps(rows, indent=2) + "\n")
for r in rows:
    print(r["sha256"][:16], r["bytes"], r["path"])
