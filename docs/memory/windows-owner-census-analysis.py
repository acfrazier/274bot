#!/usr/bin/env python3
"""Analyze the two qualification-boundary native renderer owner snapshots."""
from __future__ import annotations
import hashlib, json, statistics, tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RAW = ROOT / "diagnostics/windows-owner-census-20260908/native-render-owner-census-focused-plus-background-20260908-0150/raw-run-01"
QUAL = RAW / "samples.qualification.jsonl"
RAW_ARCHIVE = ROOT / "diagnostics/windows-owner-census-20260908/native-render-owner-census-focused-plus-background-20260908-0150.tar.gz"
FIELDS = [
    ("tile_container_bytes", "tile containers", "renderer world tile/container ownership; lifetime follows the renderer world"),
    ("linked_container_bytes", "linked containers", "linked container backing storage; lifetime follows linked world/container state"),
    ("sprite_container_bytes", "sprite containers", "sprite/cache container backing storage; lifetime follows sprite cache state"),
    ("nested_model_bytes", "nested model bytes", "model bytes counted through nested ownership; nested occurrence, not an extra unique allocation"),
    ("unique_arc_model_bytes", "unique Arc model bytes", "unique Arc-owned model pointees in this renderer census; aggregate identity is renderer-local"),
    ("shared_arc_pointees", "shared Arc pointees", "count of shared Arc pointees represented by the unique-model accounting"),
    ("render_world_scratch_bytes", "render-world scratch", "temporary renderer-world scratch storage; lifetime follows render/update work"),
    ("pix3d_scratch_bytes", "Pix3D scratch", "Pix3D temporary scratch storage; lifetime follows 3D raster work"),
    ("pix3d_texel_active_bytes", "Pix3D active texels", "active Pix3D texel backing storage; lifetime follows active texture use"),
    ("pix3d_texel_free_bytes", "Pix3D free texels", "free/reusable Pix3D texel backing storage; retained until allocator/cache release"),
    ("pixmap_bytes", "pixmaps", "software pixmap backing storage; lifetime follows pixmap owners"),
    ("minimap_bytes", "minimap", "minimap backing storage; lifetime follows minimap state"),
    ("overlay_materialized", "materialized overlays", "count of materialized overlay objects, not bytes"),
    ("transient_mesh_bytes", "transient meshes", "currently-live transient mesh bytes; short-lived render work"),
    ("transient_mesh_high_water_bytes", "transient mesh high-water", "observed transient mesh peak, not current residency"),
]

def read_jsonl(path):
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]

def main():
    boundaries = read_jsonl(QUAL)
    assert [r["phase"] for r in boundaries] == ["observe-start", "observe-end"]
    assert all(len(r["slots"]) == 16 for r in boundaries)
    rows = []
    for bi, boundary in enumerate(boundaries):
        for slot in boundary["slots"]:
            renderer = slot["renderer"]
            owner = renderer["owner_census"]
            row = {"boundary": boundary["phase"], "boundary_index": bi, "elapsed_s": boundary["elapsed_s"],
                   "ordinal": slot["ordinal"], "name": slot["name"], "renderer_generation": renderer["generation"],
                   "renderer_updated_ms": renderer["updated_ms"], **owner}
            rows.append(row)
    first = rows[:16]
    second = rows[16:]
    fields = [f[0] for f in FIELDS]
    by_slot = []
    for a, b in zip(first, second):
        assert a["ordinal"] == b["ordinal"] and a["name"] == b["name"]
        by_slot.append({"ordinal": a["ordinal"], "name": a["name"],
                        "start": {k: a.get(k) for k in ["sampled_ms", "build_generation", "phase", "duration_ns", "unmeasured_mask"] + fields},
                        "end": {k: b.get(k) for k in ["sampled_ms", "build_generation", "phase", "duration_ns", "unmeasured_mask"] + fields},
                        "delta": {k: b.get(k) - a.get(k) for k in fields if isinstance(a.get(k), (int,float)) and isinstance(b.get(k), (int,float))}})
    manifest = ROOT / "diagnostics/windows-owner-census-20260908/native-render-owner-census-focused-plus-background-20260908-0150/archive-manifest.json"
    manifest_obj = json.loads(manifest.read_text(encoding="utf-8-sig"))
    with tarfile.open(RAW_ARCHIVE, "r:gz") as archive:
        members = {m.name.replace("\\", "/"): m for m in archive.getmembers() if m.isfile()}
        manifest_checks = []
        for entry in manifest_obj["files"]:
            name = entry["path"].replace("\\", "/")
            member = members.get(name)
            if member is None:
                matches = [m for m in members.values() if m.name.endswith("/" + name)]
                assert matches, (name, [m.name for m in matches])
                member = min(matches, key=lambda m: len(m.name))
            extracted = archive.extractfile(member)
            assert extracted is not None
            payload = extracted.read()
            manifest_checks.append({"path": name, "ok": hashlib.sha256(payload).hexdigest() == entry["sha256"], "length_ok": len(payload) == entry["length"]})
    result = {
        "schema": "windows-owner-census-analysis-v1",
        "archive_sha256": hashlib.sha256(RAW_ARCHIVE.read_bytes()).hexdigest(),
        "qualification": {"phases": [r["phase"] for r in boundaries], "elapsed_s": [r["elapsed_s"] for r in boundaries], "slot_count": 16,
                          "renderer_generations": [[s["renderer"]["generation"] for s in r["slots"]] for r in boundaries],
                          "owner_build_generations": [[s["renderer"]["owner_census"]["build_generation"] for s in r["slots"]] for r in boundaries]},
        "field_definitions": [{"field": f, "category": c, "purpose_lifetime": p} for f,c,p in FIELDS],
        "rows": by_slot,
        "fleet_aggregate": [{"boundary": r["boundary"], "sum": {k: sum(x.get(k, 0) for x in r["rows"]) if False else None for k in fields}} for r in []],
        "archive_manifest_file_count": len(manifest_obj["files"]),
        "archive_manifest_verification": {"all_hashes_ok": all(x["ok"] for x in manifest_checks), "all_lengths_ok": all(x["length_ok"] for x in manifest_checks), "checked": len(manifest_checks)},
        "unmeasured_mask": sorted(set(x["unmeasured_mask"] for x in first + second)),
        "duration_ns": {boundary_name: {"min": min(x["duration_ns"] for x in source), "max": max(x["duration_ns"] for x in source)}
                        for boundary_name, source in [("observe-start", first), ("observe-end", second)]},
        "caveats": ["Owner census appears only at qualification boundaries, not periodic rawrenderer samples.", "Absent fields are not treated as zero.", "Nested model bytes and unique Arc model bytes are distinct views; do not add them.", "Per-renderer unique Arc identity cannot establish fleet-wide deduplication.", "These attribution samples do not explain total RSS and do not accept performance overhead."],
    }
    for boundary_name, source in [("observe-start", first), ("observe-end", second)]:
        result["fleet_aggregate"].append({"boundary": boundary_name, "sum": {k: sum(x[k] for x in source if isinstance(x.get(k), (int,float))) for k in fields},
                                           "max": {k: max(x[k] for x in source if isinstance(x.get(k), (int,float))) for k in fields}})
    Path(__file__).with_name("windows-owner-census-table.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"archive_sha256": result["archive_sha256"], "slot_count": 16, "boundaries": result["qualification"], "fleet_aggregate": result["fleet_aggregate"]}, indent=2))

if __name__ == "__main__":
    main()
