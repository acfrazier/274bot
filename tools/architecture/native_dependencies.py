#!/usr/bin/env python3
"""Keep script/runtime dependencies out of each native product's normal graph.

Panel's unchanged Wayland window decorations may use tiny-skia/ab_glyph only
through winit -> sctk-adwaita. No other workspace crate may feed that origin.
"""
from __future__ import annotations

import json
import re
import subprocess
import sys

PRODUCTS = ("panel", "tui", "host-play")
FORBIDDEN = (
    "script-js", "v8", "rustyscript", "deno_core", "deno_ast", "flatbuffers",
    "tiny-skia", "ab_glyph", "tokio",
)
WINDOW_RASTER = {"tiny-skia", "ab_glyph"}


def inverse_paths(text: str) -> list[tuple[str, ...]]:
    """Read complete cargo-tree --prefix depth --no-dedupe rootward paths."""
    paths: list[tuple[str, ...]] = []
    stack: list[str] = []
    for line in text.splitlines():
        if not line.strip():
            continue
        match = re.fullmatch(r"(\d+)([A-Za-z0-9_-]+) v\S+(?: .*)?", line)
        if match is None or "(*)" in line:
            raise ValueError(f"incomplete or unrecognized inverse tree line: {line}")
        depth, package = int(match[1]), match[2]
        if depth > len(stack):
            raise ValueError(f"inverse tree skips a parent: {line}")
        if depth < len(stack):
            paths.append(tuple(stack))
        del stack[depth:]
        stack.append(package)
    if stack:
        paths.append(tuple(stack))
    return paths


def check_origin(
    product: str, dependency: str, text: str, workspace: set[str]
) -> list[tuple[str, ...]]:
    paths = inverse_paths(text)
    if not paths:
        return []
    if product != "panel" or dependency not in WINDOW_RASTER:
        raise ValueError(f"{product}: unexpected native dependency {dependency}")
    for path in paths:
        if path[:3] != (dependency, "sctk-adwaita", "winit") or path[-1] != product:
            raise ValueError(f"{product}: disallowed {dependency} origin: {' <- '.join(path)}")
        if workspace.intersection(path[1:-1]):
            raise ValueError(f"{product}: workspace crate feeds window raster: {' <- '.join(path)}")
    return paths


def cargo(*args: str) -> str:
    command = ["cargo", *args]
    print("+ " + " ".join(command), flush=True)
    process = subprocess.run(command, text=True, capture_output=True, check=False)
    if process.stdout:
        print(process.stdout, end="", flush=True)
    if process.stderr:
        print(process.stderr, end="", file=sys.stderr, flush=True)
    if process.returncode:
        raise ValueError(f"cargo command failed with exit {process.returncode}")
    return process.stdout


def main() -> int:
    # These names come from Cargo, so a future workspace crate is covered too.
    metadata = json.loads(cargo("metadata", "--locked", "--no-deps", "--format-version", "1"))
    members = set(metadata["workspace_members"])
    workspace = {package["name"] for package in metadata["packages"] if package["id"] in members}
    if not set(PRODUCTS).issubset(workspace):
        raise ValueError("native product is missing from Cargo workspace metadata")
    for product in PRODUCTS:
        normal = cargo("tree", "--locked", "-p", product, "--no-default-features", "-e", "normal", "--prefix", "none", "--color", "never")
        if not normal.strip():
            raise ValueError(f"{product}: empty normal dependency tree")
        counts = {name: len(re.findall(r"^" + re.escape(name) + r" v", normal, flags=re.MULTILINE)) for name in FORBIDDEN}
        origins: dict[str, list[tuple[str, ...]]] = {}
        for dependency in FORBIDDEN:
            inverted = cargo("tree", "--locked", "-p", product, "--no-default-features", "-e", "normal", "--prefix", "depth", "--no-dedupe", "--color", "never", "-i", dependency)
            paths = check_origin(product, dependency, inverted, workspace)
            if bool(counts[dependency]) != bool(paths):
                raise ValueError(f"{product}: normal/inverse tree presence differs for {dependency}")
            origins[dependency] = paths
        print(json.dumps({"product": product, "normal_dependency_counts": counts, "allowed_inverse_paths": origins}), flush=True)
    return 0


def self_test() -> int:
    import unittest

    workspace = {"panel", "tui", "host-play", "frontend-core", "script", "script-js"}
    csd = "0tiny-skia v0.11.4\n1sctk-adwaita v0.10.1\n2winit v0.30.13\n3dear-imgui-winit v0.15.1\n4panel v0.2.6\n3panel v0.2.6\n"

    class NativeOriginTests(unittest.TestCase):
        def test_empty_inverse_is_absent(self):
            self.assertEqual(check_origin("panel", "v8", "", workspace), [])

        def test_both_unmodified_window_paths_are_allowed(self):
            self.assertEqual(check_origin("panel", "tiny-skia", csd, workspace), [
                ("tiny-skia", "sctk-adwaita", "winit", "dear-imgui-winit", "panel"),
                ("tiny-skia", "sctk-adwaita", "winit", "panel"),
            ])

        def test_script_origin_is_rejected_beside_allowed_paths(self):
            with self.assertRaises(ValueError):
                check_origin("panel", "tiny-skia", csd + "0tiny-skia v0.11.4\n1script v0.2.6\n2host-play v0.2.6\n3panel v0.2.6\n", workspace)

        def test_workspace_intermediate_before_window_is_rejected(self):
            with self.assertRaises(ValueError):
                check_origin("panel", "tiny-skia", "0tiny-skia v0.11.4\n1sctk-adwaita v0.10.1\n2winit v0.30.13\n3frontend-core v0.2.6\n4panel v0.2.6\n", workspace)

        def test_window_exception_is_panel_only(self):
            with self.assertRaises(ValueError):
                check_origin("tui", "tiny-skia", csd.replace("panel", "tui"), workspace)

        def test_non_raster_name_has_no_exception(self):
            with self.assertRaises(ValueError):
                check_origin("panel", "v8", csd.replace("tiny-skia", "v8"), workspace)

        def test_incomplete_tree_is_rejected(self):
            for text in ("0tiny-skia v0.11.4\n2winit v0.30.13\n", csd.replace("v0.15.1", "v0.15.1 (*)")):
                with self.subTest(text=text), self.assertRaises(ValueError):
                    check_origin("panel", "tiny-skia", text, workspace)

    suite = unittest.defaultTestLoader.loadTestsFromTestCase(NativeOriginTests)
    return int(not unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful())


if __name__ == "__main__":
    try:
        if sys.argv[1:] == ["--self-test"]:
            raise SystemExit(self_test())
        if sys.argv[1:]:
            raise ValueError("usage: native_dependencies.py [--self-test]")
        raise SystemExit(main())
    except (KeyError, OSError, ValueError, json.JSONDecodeError) as error:
        print(f"native dependency check: {error}", file=sys.stderr)
        raise SystemExit(1) from error
