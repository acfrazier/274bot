#!/usr/bin/env python3
"""Stage the reviewed overlays plus the deterministic transcript-test overlay.

This helper binds the immutable frozen source and existing reviewed TUI/coverage
patches, applies one integration-test-only patch, and optionally runs the exact
snapshot transcript tests or the existing coverage matrix via the reviewed
coverage helper's bounded process-group runner. It never runs live tests or a
production binary.
"""
from __future__ import annotations

import argparse
import bz2
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import sys
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
TEST_PATH = "crates/host-play/tests/snapshot_frame_equivalence.rs"
GOLDEN_PATH = "crates/host-play/tests/fixtures/snapshot_frame_transcript_golden.json"
EXPECTED = {
    "archive": "2c36d254c37c5ed56a55f78728357296d62be602f8653ed9ec041cd37d099d5d",
    "manifest": "ca69e70352c5f6f8d1c9db026959af6ba8fd64886be533a0095c3ca94bb5732e",
    "coverage_helper": "6e1aab8eae3bd217f1f2b320f8b1b45e7a8fae924563dd7c74449ed4af5a3082",
    "coverage_patch": "dcb84047157f75c7ad21868b72eb7c3434cb818e8ba3e4b732f6b40d435c1b6e",
    "tui_patch": "3bf08949fee5f44a2c6dc82525004a9cc0f47759526d43bda6f77cc9f33a51e4",
    "transcript_patch": "1e5ac15b25d113a180f8212fdd282bd63d76e79c14aa200560c2184106deb995",
    "test_before": "4fd5aa58f64c555ed99657d0b37ccf65769fd3513b294a3fccbbd90a0ccab39a",
    "test_after": "90abb21d1a3ec7f91c6bcffc59dd9312ae5d962a08c9d4d59a2f02a09ebc5d66",
    "golden": "783a7d5077829887147240a17545b69351ad2938c7304785489c899fb2dd81d4",
}
ADVERSARIAL_LOC_COUNT = 13


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_json(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def load_helper(path: Path):
    spec = importlib.util.spec_from_file_location("reviewed_coverage_overlay", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load reviewed helper: {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def verify_inputs(args: argparse.Namespace) -> dict[str, dict[str, Any]]:
    paths = {
        "archive": args.archive,
        "manifest": args.manifest,
        "coverage_helper": args.coverage_helper,
        "coverage_patch": args.coverage_patch,
        "tui_patch": args.tui_patch,
        "transcript_patch": args.transcript_patch,
    }
    rows = {}
    for name, path in paths.items():
        path = path.resolve()
        actual = sha256_file(path)
        if actual != EXPECTED[name]:
            raise RuntimeError(f"{name} identity mismatch: {actual}")
        rows[name] = {"path": str(path), "bytes": path.stat().st_size, "sha256": actual}
    return rows


def verify_transcript_contract(source: Path, manifest: dict[str, Any], helper) -> dict[str, Any]:
    test = source / TEST_PATH
    golden = source / GOLDEN_PATH
    if sha256_file(test) != EXPECTED["test_after"]:
        raise RuntimeError("corrected transcript test identity mismatch")
    if sha256_file(golden) != EXPECTED["golden"]:
        raise RuntimeError("golden transcript changed")

    text = test.read_text()
    required = [
        "const GOLDEN_FIRST_LOC_ID: usize = 4_671;",
        "Client::from_shared(cfg(), Arc::new(cache), Arc::new(Vec::new()), Vec::new())",
        "cache.locs = (0..GOLDEN_FIRST_LOC_ID)",
        "SNAPSHOT_FRAME_FIXTURE_CACHE_DIR",
        "live, golden,",
        "live, peer_tx,",
    ]
    missing = [token for token in required if token not in text]
    if missing:
        raise RuntimeError(f"transcript contract token missing: {missing}")
    forbidden = ["Client::new(cfg())", 'cache_dir: "/tmp".into()']
    present = [token for token in forbidden if token in text]
    if present:
        raise RuntimeError(f"ambient fixture token remains: {present}")

    patch_text = Path(__file__).with_name("transcript-test-only.patch").read_text()
    labels = [line for line in patch_text.splitlines() if line.startswith(("--- ", "+++ "))]
    expected_labels = [f"--- a/{TEST_PATH}", f"+++ b/{TEST_PATH}"]
    if labels != expected_labels:
        raise RuntimeError(f"patch escapes integration-test path: {labels}")
    if GOLDEN_PATH in patch_text or "Cargo.toml" in patch_text or "Cargo.lock" in patch_text:
        raise RuntimeError("transcript patch mentions golden, manifest, or lock path")

    derived = dict(helper.DERIVED_HASHES)
    derived[TEST_PATH] = EXPECTED["test_after"]
    identity = helper.verify_tree(source, manifest, derived)
    return {
        "verified": True,
        "changed_path": TEST_PATH,
        "integration_test_only": True,
        "production_or_live_binary_source_changed": False,
        "test_before_sha256": EXPECTED["test_before"],
        "test_after_sha256": EXPECTED["test_after"],
        "golden_sha256": EXPECTED["golden"],
        "full_golden_equality_retained": True,
        "peer_feature_equality_retained": True,
        "identity": identity,
    }


def jag_hash(name: str) -> int:
    value = 0
    for char in name.upper():
        value = ((value * 61) + ord(char) - 32) & 0xFFFF_FFFF
    return value


def g3(value: int) -> bytes:
    return value.to_bytes(3, "big")


def generated_config_jag(loc_count: int) -> bytes:
    files = [
        ("loc.dat", b"\0\0" + (b"\0" * loc_count)),
        ("loc.idx", loc_count.to_bytes(2, "big") + (b"\0\1" * loc_count)),
    ]
    table = bytearray(len(files).to_bytes(2, "big"))
    body = bytearray()
    for name, data in files:
        table.extend(jag_hash(name).to_bytes(4, "big"))
        table.extend(g3(len(data)))
        table.extend(g3(len(data)))
        body.extend(data)
    unpacked = bytes(table + body)
    packed = bz2.compress(unpacked, compresslevel=9)
    if not packed.startswith(b"BZh9"):
        raise RuntimeError("unexpected bzip2 framing")
    packed = packed[4:]
    return g3(len(unpacked)) + g3(len(packed)) + packed


def make_cache_fixtures(output: Path) -> dict[str, dict[str, Any]]:
    root = output / "generated-cache-fixtures"
    empty = root / "empty"
    populated = root / "prepopulated"
    empty.mkdir(parents=True)
    populated.mkdir()
    # This valid generated config JAG decodes to 13 locs. The original
    # Client::new fixture therefore appends id 13 here but id 0 in the empty
    # directory, making ambient cache dependence directly observable.
    config = populated / "config"
    config.write_bytes(generated_config_jag(ADVERSARIAL_LOC_COUNT))
    files = {
        "config": {
            "bytes": config.stat().st_size,
            "sha256": sha256_file(config),
        }
    }
    return {
        "empty": {"path": str(empty), "files": {}},
        "prepopulated": {
            "path": str(populated),
            "generated_loc_count": ADVERSARIAL_LOC_COUNT,
            "files": files,
        },
    }


def run_original_probes(helper, source, env, output, fixtures, timeout):
    test = source / TEST_PATH
    original = test.read_text()
    old = '        cache_dir: "/tmp".into(),'
    new = (
        '        cache_dir: std::env::var("SNAPSHOT_FRAME_FIXTURE_CACHE_DIR")\n'
        '            .expect("generated probe cache dir"),'
    )
    if original.count(old) != 1:
        raise RuntimeError("original probe cache-dir seam changed")
    test.write_text(original.replace(old, new))
    probe_hash = sha256_file(test)
    rows = []
    try:
        command = [
            "cargo",
            "test",
            "--offline",
            "--locked",
            "-p",
            "host-play",
            "--test",
            "snapshot_frame_equivalence",
            "transcript_matches_golden_and_peer_feature_build",
            "--features",
            "memory-profile-no-alloc",
            "--",
            "--exact",
            "--test-threads=1",
        ]
        for condition, expected_id in (("empty", 0), ("prepopulated", ADVERSARIAL_LOC_COUNT)):
            step_env = dict(env)
            step_env["SNAPSHOT_FRAME_FIXTURE_CACHE_DIR"] = fixtures[condition]["path"]
            step = helper.run_step(
                f"original-{condition}-ambient-probe",
                command,
                source,
                output,
                step_env,
                timeout,
            )
            log_text = Path(step["log"]).read_text(errors="replace")
            observed = f"loc_ids: [{expected_id}]" in log_text
            step.update(
                {
                    "cache_condition": condition,
                    "expected_failure": True,
                    "expected_loc_id": expected_id,
                    "expected_loc_id_observed": observed,
                    "probe_test_sha256": probe_hash,
                }
            )
            step["environment"]["SNAPSHOT_FRAME_FIXTURE_CACHE_DIR"] = fixtures[condition]["path"]
            rows.append(step)
            write_json(output / "original-probe-steps.json", rows)
    finally:
        test.write_text(original)
    if sha256_file(test) != EXPECTED["test_before"]:
        raise RuntimeError("original transcript source was not restored after probes")
    return rows


def focused_matrix(source: Path) -> list[tuple[str, list[str], Path, str]]:
    base = [
        "cargo",
        "test",
        "--offline",
        "--locked",
        "-p",
        "host-play",
        "--test",
        "snapshot_frame_equivalence",
        "transcript_matches_golden_and_peer_feature_build",
    ]
    exact = ["--", "--exact", "--test-threads=1"]
    rows = []
    for condition in ("empty", "prepopulated"):
        rows.extend(
            [
                (
                    f"snapshot-{condition}-feature-off",
                    [*base, "--features", "memory-profile-no-alloc", *exact],
                    source,
                    condition,
                ),
                (
                    f"snapshot-{condition}-feature-on",
                    [*base, "--features", "memory-profile-no-alloc,snapshot-dedup", *exact],
                    source,
                    condition,
                ),
            ]
        )
    return rows


def run_rows(helper, rows, env, output, fixtures, timeout, stop_on_failure):
    results = []
    for name, command, cwd, condition in rows:
        step_env = dict(env)
        step_env["SNAPSHOT_FRAME_FIXTURE_CACHE_DIR"] = fixtures[condition]["path"]
        step = helper.run_step(name, command, cwd, output, step_env, timeout)
        step["cache_condition"] = condition
        step["environment"]["SNAPSHOT_FRAME_FIXTURE_CACHE_DIR"] = fixtures[condition]["path"]
        results.append(step)
        write_json(output / "steps.json", results)
        if stop_on_failure and (
            step["exit"] != 0 or step["timed_out"] or not step["cleanup"]["group_absent"]
        ):
            break
    return results


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--archive",
        type=Path,
        default=ROOT / "artifact/frozen/frozen-direct-owner-source.tar.gz",
    )
    parser.add_argument(
        "--manifest",
        type=Path,
        default=ROOT / "artifact/frozen/frozen-source-manifest.json",
    )
    parser.add_argument(
        "--coverage-helper",
        type=Path,
        default=ROOT / "coverage-overlay/stage_coverage_overlay.py",
    )
    parser.add_argument(
        "--coverage-patch",
        type=Path,
        default=ROOT / "coverage-overlay/coverage-test-only.patch",
    )
    parser.add_argument(
        "--tui-patch",
        type=Path,
        default=ROOT / "tui-test-overlay/tui-test-only.patch",
    )
    parser.add_argument(
        "--transcript-patch",
        type=Path,
        default=Path(__file__).with_name("transcript-test-only.patch"),
    )
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--target-dir", type=Path)
    parser.add_argument("--build-target")
    parser.add_argument("--run-focused", action="store_true")
    parser.add_argument("--run-full-coverage-matrix", action="store_true")
    parser.add_argument("--timeout", type=int, default=900)
    args = parser.parse_args()

    output = args.output_dir.resolve()
    if output.exists():
        print(json.dumps({"prepared": False, "error": "output-dir must be fresh"}), file=sys.stderr)
        return 2
    output.mkdir(parents=True)
    receipt: dict[str, Any] = {
        "schema": "direct-owner-transcript-test-overlay-v1",
        "prepared": False,
        "steps": [],
        "native_linux_qualified": False,
        "live_qualified": False,
        "performance_qualified": False,
    }
    try:
        inputs = verify_inputs(args)
        helper = load_helper(args.coverage_helper.resolve())
        manifest = json.loads(args.manifest.read_text())

        base_stage = output / "reviewed-base-stage"
        prior_argv = sys.argv
        try:
            sys.argv = [
                str(args.coverage_helper),
                "--archive",
                str(args.archive),
                "--manifest",
                str(args.manifest),
                "--output-dir",
                str(base_stage),
                "--coverage-patch",
                str(args.coverage_patch),
                "--tui-patch",
                str(args.tui_patch),
                "--keep-source",
            ]
            if helper.main() != 0:
                raise RuntimeError("reviewed base overlay staging failed")
        finally:
            sys.argv = prior_argv

        source = base_stage / "derived-source"
        before = sha256_file(source / TEST_PATH)
        if before != EXPECTED["test_before"]:
            raise RuntimeError(f"transcript test before-hash mismatch: {before}")

        fixtures = make_cache_fixtures(output)
        target = (args.target_dir or (output / "target")).resolve()
        env = helper.safe_environment(target)
        env["PYTHONDONTWRITEBYTECODE"] = "1"
        if args.build_target:
            env["CARGO_BUILD_TARGET"] = args.build_target
        for key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"):
            env.pop(key, None)

        original_probes = []
        if args.run_focused:
            original_probes = run_original_probes(
                helper,
                source,
                env,
                output,
                fixtures,
                args.timeout,
            )
        helper.apply_patch(
            source,
            args.transcript_patch.resolve(),
            EXPECTED["transcript_patch"],
            "deterministic-transcript-integration-test",
        )
        contract = verify_transcript_contract(source, manifest, helper)
        locks = [helper.verify_locks(source, manifest, "after-transcript-stage")]

        steps = []
        if args.run_focused:
            steps.extend(
                run_rows(
                    helper,
                    focused_matrix(source),
                    env,
                    output,
                    fixtures,
                    args.timeout,
                    False,
                )
            )
        if args.run_full_coverage_matrix and all(
            row["exit"] == 0 and not row["timed_out"] and row["cleanup"]["group_absent"]
            for row in steps
        ):
            matrix = helper.gap_matrix(source) + helper.feature_off_matrix(source) + helper.feature_on_matrix(source)
            full_rows = []
            for name, command, cwd in matrix:
                full_rows.append((name, command, cwd, "empty"))
            start = len(steps)
            full_results = []
            for name, command, cwd, condition in full_rows:
                step_env = dict(env)
                step_env["SNAPSHOT_FRAME_FIXTURE_CACHE_DIR"] = fixtures[condition]["path"]
                if name in {"feature-off-host", "feature-on-host"}:
                    step_env.pop("BOT_CPU", None)
                    step_env["R274_TEST_FORCE_NO_GPU"] = "1"
                step = helper.run_step(name, command, cwd, output, step_env, args.timeout)
                step["cache_condition"] = condition
                step["environment"]["SNAPSHOT_FRAME_FIXTURE_CACHE_DIR"] = fixtures[condition]["path"]
                full_results.append(step)
                write_json(output / "steps.json", [*steps, *full_results])
                if step["exit"] != 0 or step["timed_out"] or not step["cleanup"]["group_absent"]:
                    break
            steps.extend(full_results)
            receipt["full_matrix_expected_steps"] = 19
            receipt["full_matrix_completed_steps"] = len(steps) - start

        locks.append(helper.verify_locks(source, manifest, "after-transcript-tests"))
        post = verify_transcript_contract(source, manifest, helper)
        all_passed = all(
            row["exit"] == 0 and not row["timed_out"] and row["cleanup"]["group_absent"]
            for row in steps
        )
        original_dependence_exposed = bool(
            args.run_focused
            and len(original_probes) == 2
            and all(
                row["exit"] == 101
                and not row["timed_out"]
                and row["cleanup"]["group_absent"]
                and row["expected_loc_id_observed"]
                for row in original_probes
            )
        )
        requested = (4 if args.run_focused else 0) + (19 if args.run_full_coverage_matrix else 0)
        all_requested_passed = (
            len(steps) == requested
            and all_passed
            and (not args.run_focused or original_dependence_exposed)
        )
        receipt.update(
            {
                "prepared": True,
                "inputs": inputs,
                "source": str(source),
                "source_member_count": 1124,
                "source_only_contract": contract,
                "post_test_contract": post,
                "locks": locks,
                "cache_fixtures": fixtures,
                "original_probe_steps": original_probes,
                "original_ambient_dependence_exposed": original_dependence_exposed,
                "steps": steps,
                "requested_step_count": requested,
                "all_requested_passed": all_requested_passed,
                "platform": {
                    "system": platform.system(),
                    "release": platform.release(),
                    "machine": platform.machine(),
                },
                "native_linux_qualified": bool(
                    platform.system() == "Linux"
                    and platform.machine() == "x86_64"
                    and args.run_focused
                    and args.run_full_coverage_matrix
                    and all_requested_passed
                ),
                "live_qualified": False,
                "performance_qualified": False,
                "limitations": [
                    "The only new source delta is an integration test under crates/host-play/tests.",
                    "The committed golden and all equality assertions remain byte-for-byte unchanged.",
                    "Generated cache fixtures contain no private or ambient cache data.",
                    "A macOS run is local generated evidence, not native Linux qualification.",
                    "No frontend, account, vault, server, live test, or production binary is run.",
                    "The separate Linux GPU texture brightness failure is outside this overlay.",
                ],
            }
        )
        write_json(output / "overlay-receipt.json", receipt)
        print(json.dumps(receipt, indent=2, sort_keys=True))
        return 0 if all_requested_passed else 1
    except (OSError, RuntimeError, KeyError, ValueError, json.JSONDecodeError) as error:
        receipt["error"] = str(error)
        write_json(output / "overlay-receipt.json", receipt)
        print(json.dumps(receipt, indent=2, sort_keys=True), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
