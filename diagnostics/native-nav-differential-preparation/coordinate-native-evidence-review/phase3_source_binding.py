#!/usr/bin/env python3
"""Verify source binding record against Git objects and claimed identities."""
import hashlib
import json
import subprocess
from pathlib import Path

out = Path("diagnostics/native-nav-differential-preparation/coordinate-native-evidence-review")
out.mkdir(parents=True, exist_ok=True)

binding_path = Path("docs/memory/nav-tiled-stage-a/coordinate-source-binding.json")
raw = binding_path.read_bytes()
binding_sha = hashlib.sha256(raw).hexdigest()
binding = json.loads(raw)

EXPECTED = {
    "schema": "nav-coordinate-source-v1",
    "candidate_id": "coordinate-ebf0f30",
    "dense_base": "29b7aea779322c8611f83dc193e939ca7d756f75",
    "refined_base": "8385babb23fd15b876506d4a3f6154984a6b2df1",
    "client": "3456edc8dabf7b25ada78110ffa56327af9f67a4",
    "overlay_path": "crates/nav/src/collision.rs",
    "overlay_source_commit": "ebf0f30f0422229ea75de39db06944091d6497a5",
    "old_git_blob": "7446a044e12cddf4bd2d8ff0f7093cbaea4d7af1",
    "old_sha256": "688713da89546ab61755f6cdb95714f466ff270657b5a936e53a5d2e4702b8ef",
    "new_git_blob": "07e0b215aa1fd21df87f308f0a889aa4ce8bb4a7",
    "new_sha256": "760ae7c88c35fac6183f2db51af1f69af8d5e3225110889bc84a5a4944720b0b",
    "binding_sha256": "7b101c30bfd64c302815f9844fe27d3a52302feedaec58737726ab6a062491bb",
}


def git(*args):
    r = subprocess.run(
        ["git", *args], capture_output=True, text=True, check=False
    )
    return r.returncode, r.stdout.strip(), r.stderr.strip()


def cat_blob(oid):
    r = subprocess.run(
        ["git", "cat-file", "-p", oid], capture_output=True, check=False
    )
    return r.returncode, r.stdout


checks = []


def check(name, ok, detail=None):
    checks.append({"name": name, "ok": bool(ok), "detail": detail})
    print(("PASS" if ok else "FAIL"), name, detail or "")


check("binding_sha256", binding_sha == EXPECTED["binding_sha256"], binding_sha)
check("schema", binding.get("schema") == EXPECTED["schema"])
check("candidate_id", binding.get("candidate_id") == EXPECTED["candidate_id"])
check("dense_base", binding.get("dense_base") == EXPECTED["dense_base"])
check("refined_base", binding.get("refined_base") == EXPECTED["refined_base"])
check("client", binding.get("client") == EXPECTED["client"])
check("exactly_one_overlay", len(binding.get("overlays") or []) == 1)
ov = (binding.get("overlays") or [{}])[0]
check("overlay_path", ov.get("path") == EXPECTED["overlay_path"])
check("overlay_source_commit", ov.get("source_commit") == EXPECTED["overlay_source_commit"])
check("old_git_blob", ov.get("old_git_blob") == EXPECTED["old_git_blob"])
check("new_git_blob", ov.get("new_git_blob") == EXPECTED["new_git_blob"])
check("old_sha256_field", ov.get("old_sha256") == EXPECTED["old_sha256"])
check("new_sha256_field", ov.get("new_sha256") == EXPECTED["new_sha256"])

# Git object presence
for label, oid in [
    ("dense_base_commit", EXPECTED["dense_base"]),
    ("refined_base_commit", EXPECTED["refined_base"]),
    ("client_commit", EXPECTED["client"]),
    ("overlay_source_commit_obj", EXPECTED["overlay_source_commit"]),
    ("old_git_blob_obj", EXPECTED["old_git_blob"]),
    ("new_git_blob_obj", EXPECTED["new_git_blob"]),
]:
    rc, typ, err = git("cat-file", "-t", oid)
    check(f"git_type_{label}", rc == 0, typ or err)

# Verify blob content hashes for old/new collision blobs if present
for label, oid, expected_sha in [
    ("old_collision_blob_bytes", EXPECTED["old_git_blob"], EXPECTED["old_sha256"]),
    ("new_collision_blob_bytes", EXPECTED["new_git_blob"], EXPECTED["new_sha256"]),
]:
    rc, data = cat_blob(oid)
    if rc != 0:
        check(label, False, "missing blob")
        continue
    got = hashlib.sha256(data).hexdigest()
    check(label, got == expected_sha, f"bytes={len(data)} sha={got}")

# Client commit may live in submodule — note if missing
rc, typ, err = git("cat-file", "-t", EXPECTED["client"])
if rc != 0:
    # try submodule
    sub = Path("vendor/fr-client-rust")
    if sub.exists():
        r = subprocess.run(
            ["git", "-C", str(sub), "cat-file", "-t", EXPECTED["client"]],
            capture_output=True,
            text=True,
        )
        check(
            "client_in_submodule",
            r.returncode == 0,
            r.stdout.strip() or r.stderr.strip(),
        )
    else:
        check("client_in_submodule", False, "no submodule path")

report = {
    "binding_path": str(binding_path),
    "binding_sha256": binding_sha,
    "binding": binding,
    "checks": checks,
    "all_ok": all(c["ok"] for c in checks),
}
(out / "phase3-source-binding.json").write_text(json.dumps(report, indent=2) + "\n")
print("ALL_OK" if report["all_ok"] else "HAS_FAILURES")
