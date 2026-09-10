#!/usr/bin/env python3
"""Supplement check 1+4: stream-hash every generated archive member; reconstruct
effective original-source from git blobs (dense 29b7 / refined 8385 / client 3456
+ sole 07e0 overlay). Independent of root audit flags.
"""
from __future__ import annotations

import hashlib
import json
import subprocess
import tarfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
OUT = Path(
    "diagnostics/native-nav-differential-preparation/coordinate-native-evidence-supplement"
)
GEN = Path(
    "diagnostics/native-nav-differential-preparation/coordinate-a44a930-native-01"
)
ARC = GEN / "coordinate-native-generated-01.tar.gz"
MANIFEST = GEN / "coordinate-native-generated-01-archive.json"
EXPECTED_OUTER = (
    "baa98510e1c4044bf817e9ceffddfcee4dcf3178ce4f94ba3d37d42e291c7524"
)
BINDING_SHA = (
    "7b101c30bfd64c302815f9844fe27d3a52302feedaec58737726ab6a062491bb"
)
DENSE = "29b7aea779322c8611f83dc193e939ca7d756f75"
REFINED = "8385babb23fd15b876506d4a3f6154984a6b2df1"
CLIENT = "3456edc8dabf7b25ada78110ffa56327af9f67a4"
OVERLAY_BLOB = "07e0b215aa1fd21df87f308f0a889aa4ce8bb4a7"
# Adapter-only suffixes applied after effective original bytes (production harness).
SUFFIXES = {
    "crates/nav/Cargo.toml": (
        b'\n[[bin]]\nname="differential-probe"\npath="src/differential.rs"\n'
    ),
    "crates/nav/src/router.rs": (
        b'\n#[path="router-access.rs"]\npub mod differential_access;\n'
    ),
}
KEEP_MAX = 16 * 1024 * 1024  # retain for later logical checks only


def sha256(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def sha_file(p: Path) -> tuple[str, int]:
    h = hashlib.sha256()
    n = 0
    with open(p, "rb") as f:
        while True:
            chunk = f.read(1024 * 1024)
            if not chunk:
                break
            h.update(chunk)
            n += len(chunk)
    return h.hexdigest(), n


def git_out(where: Path, *args: str) -> bytes:
    return subprocess.check_output(["git", "-C", str(where), *args])


def check(checks: list, name: str, ok: bool, detail=None):
    checks.append({"name": name, "ok": bool(ok), "detail": detail})
    print(("PASS" if ok else "FAIL"), name, "" if detail is None else detail)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    t0 = time.time()
    checks = []
    progress = {"phase": "start", "t0": t0}

    outer, outer_n = sha_file(ARC)
    check(checks, "gen_outer_sha", outer == EXPECTED_OUTER, outer)
    check(checks, "gen_outer_bytes", outer_n == 18765670, outer_n)

    man = json.loads(MANIFEST.read_text())
    check(
        checks,
        "manifest_outer_sha",
        man.get("archive_sha256") == EXPECTED_OUTER,
        man.get("archive_sha256"),
    )
    expected = {e["path"]: e for e in man["files"]}
    check(
        checks,
        "manifest_unique_paths",
        len(expected) == len(man["files"]) == 13436,
        len(man["files"]),
    )

    seen = {}
    payload = {}
    uncomp = 0
    mismatches = []
    extra = []
    progress["phase"] = "stream_members"
    with tarfile.open(ARC, "r|gz") as tf:
        for m in tf:
            if not m.isfile():
                continue
            p = Path(m.name)
            if p.is_absolute() or ".." in p.parts:
                mismatches.append({"path": m.name, "err": "unsafe_path"})
                continue
            if p.parts[0] != "coordinate-native-generated-01":
                mismatches.append({"path": m.name, "err": "bad_prefix"})
                continue
            rel = str(Path(*p.parts[1:]))
            if rel not in expected:
                extra.append(rel)
                continue
            if rel in seen:
                mismatches.append({"path": rel, "err": "duplicate"})
                continue
            e = expected[rel]
            if m.size != e["bytes"]:
                mismatches.append(
                    {
                        "path": rel,
                        "err": "size",
                        "tar": m.size,
                        "manifest": e["bytes"],
                    }
                )
            f = tf.extractfile(m)
            h = hashlib.sha256()
            chunks = []
            keep = m.size < KEEP_MAX
            while True:
                b = f.read(1024 * 1024)
                if not b:
                    break
                h.update(b)
                if keep:
                    chunks.append(b)
            got = h.hexdigest()
            if got != e["sha256"]:
                mismatches.append(
                    {
                        "path": rel,
                        "err": "sha256",
                        "got": got,
                        "expected": e["sha256"],
                    }
                )
            seen[rel] = {"bytes": m.size, "sha256": got}
            uncomp += m.size
            if keep and chunks:
                payload[rel] = b"".join(chunks)

    missing = sorted(set(expected) - set(seen))
    check(checks, "all_members_present", not missing and not extra, {
        "missing_count": len(missing),
        "extra_count": len(extra),
        "missing_sample": missing[:5],
        "extra_sample": extra[:5],
    })
    check(checks, "member_hash_mismatches", not mismatches, mismatches[:10])
    check(checks, "member_count", len(seen) == 13436, len(seen))
    check(checks, "uncompressed_bytes", uncomp == 681424323, uncomp)

    # Corpus / input / output hashes from retained small JSON + stream hashes
    progress["phase"] = "corpus_outputs"
    result = json.loads(payload["result.json"])
    check(checks, "result_qualified", result.get("qualified") is True)
    check(
        checks,
        "result_input_count",
        result.get("input_count") == 12954,
        result.get("input_count"),
    )
    check(
        checks,
        "result_comparison",
        result.get("comparison") == {"equal": True, "bytes": 328120731},
        result.get("comparison"),
    )
    check(
        checks,
        "result_protocol",
        result.get("generated_protocol_comparison")
        == {"equal": True, "bytes": 80987},
        result.get("generated_protocol_comparison"),
    )
    out_hash_bad = []
    for p, h in (result.get("output_hashes") or {}).items():
        if seen.get(p, {}).get("sha256") != h:
            out_hash_bad.append(p)
    check(checks, "result_output_hashes_vs_stream", not out_hash_bad, out_hash_bad[:10])
    check(
        checks,
        "dense_eq_refined_probe",
        seen["dense-probe.out"]["sha256"] == seen["refined-probe.out"]["sha256"],
    )
    check(
        checks,
        "dense_eq_refined_protocol",
        seen["protocol-fixture/dense.out"]["sha256"]
        == seen["protocol-fixture/refined.out"]["sha256"],
    )

    corpus = json.loads(payload["corpus.json"])
    check(checks, "corpus_len_12954", len(corpus) == 12954, len(corpus))
    corpus_bad = []
    for e in corpus:
        s = seen.get(e["path"])
        if not s or s["sha256"] != e["sha256"] or s["bytes"] != e["bytes"]:
            corpus_bad.append(e["path"])
    check(checks, "corpus_all_hashes", not corpus_bad, {
        "bad": len(corpus_bad),
        "sample": corpus_bad[:5],
    })

    binding_bytes = payload["tools/coordinate-source-binding.json"]
    check(checks, "binding_sha_in_archive", sha256(binding_bytes) == BINDING_SHA)
    check(
        checks,
        "result_binding_sha",
        result["source_binding"]["sha256"] == BINDING_SHA,
    )

    launch = json.loads(payload["launch.json"])
    tool_bad = []
    for name, h in launch["tools"].items():
        path = "tools/" + name
        if seen.get(path, {}).get("sha256") != h:
            tool_bad.append(name)
        else:
            try:
                git_bytes = git_out(
                    ROOT, "show", f"a44a930:docs/memory/nav-tiled-differential/{name}"
                )
                if payload.get(path) != git_bytes:
                    tool_bad.append(name + ":git_bytes")
            except subprocess.CalledProcessError as exc:
                tool_bad.append(name + f":git_err:{exc.returncode}")
    check(checks, "launch_tools_vs_stream_and_git_a44a930", not tool_bad, tool_bad)

    # --- Check 4: every original/effective source entry from git ---
    progress["phase"] = "source_reconstruction"
    source_report = {}
    for arm, rev in (("dense", DENSE), ("refined", REFINED)):
        original = json.loads(payload[f"{arm}/original-source-manifest.json"])
        effective = json.loads(payload[f"{arm}/effective-source-manifest.json"])
        admission = json.loads(payload[f"{arm}-admission.json"])
        arm_checks = []
        arm_checks.append(
            {
                "name": "commit_align",
                "ok": original["commit"]
                == effective["base_commit"]
                == admission["base_commit"]
                == rev,
            }
        )
        arm_checks.append(
            {
                "name": "admission_eff_manifest_sha",
                "ok": admission["effective_source_manifest_sha256"]
                == seen[f"{arm}/effective-source-manifest.json"]["sha256"],
            }
        )
        adm_src_bad = []
        for rel, h in admission["source"].items():
            key = f"{arm}/{rel}"
            if seen.get(key, {}).get("sha256") != h:
                adm_src_bad.append(rel)
        arm_checks.append(
            {"name": "admission_source_vs_archive", "ok": not adm_src_bad, "bad": adm_src_bad[:10], "count": len(admission["source"])}
        )

        orig = {e["path"]: e for e in original["files"]}
        eff = {e["path"]: e for e in effective["files"]}
        arm_checks.append(
            {
                "name": "orig_eff_path_sets_equal",
                "ok": set(orig) == set(eff),
                "orig": len(orig),
                "eff": len(eff),
            }
        )

        entry_failures = []
        overlays = []
        for p, e in orig.items():
            try:
                client = p.startswith("vendor/fr-client-rust/")
                where = ROOT / "vendor/fr-client-rust" if client else ROOT
                commit = original["client"] if client else rev
                gp = p[len("vendor/fr-client-rust/") :] if client else p
                oid = git_out(where, "rev-parse", f"{commit}:{gp}").decode().strip()
                if oid != e["git_blob"]:
                    entry_failures.append(
                        {"path": p, "err": "git_blob", "oid": oid, "expected": e["git_blob"]}
                    )
                    continue
                base = git_out(where, "cat-file", "blob", oid)
                if sha256(base) != e["sha256"] or len(base) != e["size"]:
                    entry_failures.append(
                        {
                            "path": p,
                            "err": "original_bytes",
                            "sha": sha256(base),
                            "size": len(base),
                        }
                    )
                    continue
                effective_bytes = base
                if arm == "refined" and p == "crates/nav/src/collision.rs":
                    overlays.append(p)
                    effective_bytes = git_out(ROOT, "cat-file", "blob", OVERLAY_BLOB)
                ee = eff[p]
                if sha256(effective_bytes) != ee["sha256"] or len(effective_bytes) != ee["size"]:
                    entry_failures.append(
                        {
                            "path": p,
                            "err": "effective_manifest",
                            "sha": sha256(effective_bytes),
                            "size": len(effective_bytes),
                        }
                    )
                    continue
                archived = payload.get(f"{arm}/{p}")
                want = effective_bytes + SUFFIXES.get(p, b"")
                if archived is None:
                    entry_failures.append({"path": p, "err": "not_retained_in_payload"})
                elif archived != want:
                    entry_failures.append(
                        {
                            "path": p,
                            "err": "archive_payload_mismatch",
                            "arch_sha": sha256(archived),
                            "want_sha": sha256(want),
                            "arch_len": len(archived),
                            "want_len": len(want),
                        }
                    )
            except Exception as exc:  # noqa: BLE001 — record first error per entry
                entry_failures.append(
                    {"path": p, "err": "exception", "msg": f"{type(exc).__name__}: {exc}"}
                )

        expected_overlays = 1 if arm == "refined" else 0
        arm_checks.append(
            {
                "name": "overlay_count",
                "ok": len(overlays) == expected_overlays,
                "overlays": overlays,
            }
        )
        arm_checks.append(
            {
                "name": "every_entry_reconstructed",
                "ok": not entry_failures,
                "failures": entry_failures[:20],
                "failure_count": len(entry_failures),
                "entry_count": len(orig),
            }
        )

        # Host span extract (adapter-only, separate from production effective sources)
        host = payload[f"{arm}/original-host-lib.rs"]
        host_git = git_out(ROOT, "show", f"{rev}:crates/host-play/src/lib.rs")
        arm_checks.append(
            {"name": "host_lib_vs_git", "ok": host == host_git, "bytes": len(host)}
        )
        spans = json.loads(payload[f"{arm}/host-spans.json"])
        arm_checks.append(
            {
                "name": "host_spans_source_sha",
                "ok": sha256(host) == spans["source_sha256"],
            }
        )
        pieces = []
        span_bad = []
        for s in spans["spans"]:
            b = host[s["start_byte"] : s["end_byte"]]
            if sha256(b) != s["sha256"]:
                span_bad.append(s)
            pieces.append(b)
        rebuilt = b"\n\n".join(pieces) + b"\n"
        arm_checks.append({"name": "host_spans_pieces", "ok": not span_bad})
        arm_checks.append(
            {
                "name": "host_probe_from_spans",
                "ok": payload[f"{arm}/crates/nav/src/host-probe.rs"] == rebuilt,
            }
        )

        source_report[arm] = {
            "entry_count": len(orig),
            "checks": arm_checks,
            "all_ok": all(c["ok"] for c in arm_checks),
        }
        for c in arm_checks:
            check(checks, f"{arm}_{c['name']}", c["ok"], {k: v for k, v in c.items() if k not in ("name", "ok")})

    check(
        checks,
        "source_counts_209_210",
        source_report["dense"]["entry_count"] == 209
        and source_report["refined"]["entry_count"] == 210,
        {a: source_report[a]["entry_count"] for a in source_report},
    )

    report = {
        "scope": "supplement checks 1+4 generated archive full member hash + git source reconstruction",
        "archive": str(ARC),
        "outer_sha256": outer,
        "member_count": len(seen),
        "uncompressed_bytes": uncomp,
        "payload_retained_under_16mib": len(payload),
        "source_report": source_report,
        "checks": checks,
        "all_ok": all(c["ok"] for c in checks),
        "elapsed_s": time.time() - t0,
        "progress": progress,
    }
    (OUT / "check1-generated-members.json").write_text(json.dumps({
        k: report[k] for k in (
            "scope", "archive", "outer_sha256", "member_count",
            "uncompressed_bytes", "payload_retained_under_16mib",
            "checks", "all_ok", "elapsed_s",
        )
    }, indent=2) + "\n")
    (OUT / "check4-source-reconstruction.json").write_text(
        json.dumps({
            "scope": report["scope"],
            "source_report": source_report,
            "checks": [c for c in checks if c["name"].startswith(("dense_", "refined_", "source_"))],
            "all_ok": all(
                c["ok"]
                for c in checks
                if c["name"].startswith(("dense_", "refined_", "source_"))
            ),
            "elapsed_s": report["elapsed_s"],
        }, indent=2)
        + "\n"
    )
    (OUT / "check1-and-4-full.json").write_text(json.dumps(report, indent=2) + "\n")
    print("ALL_OK" if report["all_ok"] else "HAS_FAILURES")
    for c in checks:
        if not c["ok"]:
            print("FAIL_DETAIL", json.dumps(c)[:500])


if __name__ == "__main__":
    main()
