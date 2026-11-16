#!/usr/bin/env python3
"""Keep each surface's runtime out of every product build that lacks its feature.

The products are panel, tui and host-play. Each builds in four compositions:
both script surfaces (the default features), JS only, Luau only, and neither.
A composition's dependency graph (normal and build edges) must hold every crate
of the surfaces it compiles in and no crate of the others. A build without a
surface then has none of its code. The build edges count because `luau0-src`
compiles the Luau VM from source for `mlua-sys`.

Script paint needs the raster crates. The front ends get them from either
surface, through frontend-core, and host-play gets them from JS alone. Panel's
unchanged Wayland window decorations may also use tiny-skia/ab_glyph, only
through winit -> sctk-adwaita. No other workspace crate may feed that origin.
"""
from __future__ import annotations

import json
import re
import subprocess
import sys

PRODUCTS = ("panel", "tui", "host-play")
# The crates that only one surface brings in.
SURFACE_CRATES = {
    "js": ("script-js", "v8", "rustyscript", "deno_core", "deno_ast", "flatbuffers", "tokio"),
    "luau": ("script-luau", "mlua", "mlua-sys", "luau0-src"),
}
RASTER = ("tiny-skia", "ab_glyph")
WINDOW_RASTER = set(RASTER)
# Composition name -> (cargo feature arguments, the surfaces that it compiles in).
COMPOSITIONS = {
    "both": ((), frozenset({"js", "luau"})),
    "js-only": (("--no-default-features", "--features", "js"), frozenset({"js"})),
    "luau-only": (("--no-default-features", "--features", "luau"), frozenset({"luau"})),
    "neither": (("--no-default-features",), frozenset()),
}


def rules(product: str, surfaces: frozenset[str]) -> tuple[tuple[str, ...], tuple[str, ...]]:
    """Return the crates that one product must have and must not have."""
    required = tuple(name for surface in sorted(surfaces) for name in SURFACE_CRATES[surface])
    forbidden = tuple(
        name for surface in sorted(SURFACE_CRATES) if surface not in surfaces for name in SURFACE_CRATES[surface]
    )
    raster_surfaces = {"js", "luau"} if product in ("panel", "tui") else {"js"}
    if not surfaces & raster_surfaces:
        forbidden += RASTER
    return required, forbidden


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


def cargo(*args: str, absent_ok: bool = False) -> str:
    """Run cargo. With `absent_ok`, a package that the resolve lacks reads as an empty tree."""
    command = ["cargo", *args]
    print("+ " + " ".join(command), flush=True)
    process = subprocess.run(command, text=True, capture_output=True, check=False)
    if process.stdout:
        print(process.stdout, end="", flush=True)
    if process.stderr:
        print(process.stderr, end="", file=sys.stderr, flush=True)
    if process.returncode:
        if absent_ok and "did not match any packages" in process.stderr:
            return ""
        raise ValueError(f"cargo command failed with exit {process.returncode}")
    return process.stdout


def count_package(name: str, tree: str) -> int:
    """Count the lines of a cargo-tree --prefix none output that name this package."""
    return len(re.findall(r"^" + re.escape(name) + r" v", tree, flags=re.MULTILINE))


def main(compositions: list[str]) -> int:
    # These names come from Cargo, so a future workspace crate is covered too.
    metadata = json.loads(cargo("metadata", "--locked", "--no-deps", "--format-version", "1"))
    members = set(metadata["workspace_members"])
    workspace = {package["name"] for package in metadata["packages"] if package["id"] in members}
    if not set(PRODUCTS).issubset(workspace):
        raise ValueError("native product is missing from Cargo workspace metadata")
    for composition in compositions:
        features, surfaces = COMPOSITIONS[composition]
        for product in PRODUCTS:
            required, forbidden = rules(product, surfaces)
            normal = cargo("tree", "--locked", "-p", product, *features, "-e", "normal,build", "--prefix", "none", "--color", "never")
            if not normal.strip():
                raise ValueError(f"{composition} {product}: empty dependency tree")
            counts = {name: count_package(name, normal) for name in (*required, *forbidden)}
            for name in required:
                if not counts[name]:
                    raise ValueError(f"{composition} {product}: {name} is missing from the dependency tree")
            origins: dict[str, list[tuple[str, ...]]] = {}
            for dependency in forbidden:
                inverted = cargo("tree", "--locked", "-p", product, *features, "-e", "normal,build", "--prefix", "depth", "--no-dedupe", "--color", "never", "-i", dependency, absent_ok=True)
                paths = check_origin(product, dependency, inverted, workspace)
                if bool(counts[dependency]) != bool(paths):
                    raise ValueError(f"{composition} {product}: normal/inverse tree presence differs for {dependency}")
                origins[dependency] = paths
            print(json.dumps({"composition": composition, "product": product, "dependency_counts": counts, "allowed_inverse_paths": origins}), flush=True)
    return 0


def self_test() -> int:
    import unittest

    workspace = {"panel", "tui", "host-play", "frontend-core", "script", "script-js", "script-luau"}
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

    class CompositionRuleTests(unittest.TestCase):
        @staticmethod
        def surfaces(composition):
            return COMPOSITIONS[composition][1]

        def test_every_surface_crate_is_required_or_forbidden_in_each_cell(self):
            every = {name for names in SURFACE_CRATES.values() for name in names}
            for composition in COMPOSITIONS:
                for product in PRODUCTS:
                    required, forbidden = rules(product, self.surfaces(composition))
                    with self.subTest(composition=composition, product=product):
                        self.assertEqual(every, set(required) | (set(forbidden) - set(RASTER)))
                        self.assertFalse(set(required) & set(forbidden))

        def test_both_surfaces_forbid_nothing(self):
            for product in PRODUCTS:
                self.assertEqual(rules(product, self.surfaces("both"))[1], ())

        def test_js_only_requires_js_and_forbids_every_luau_crate(self):
            required, forbidden = rules("host-play", self.surfaces("js-only"))
            self.assertEqual(required, SURFACE_CRATES["js"])
            self.assertEqual(forbidden, SURFACE_CRATES["luau"])

        def test_luau_only_forbids_every_js_crate_and_the_host_raster_only(self):
            self.assertEqual(rules("host-play", self.surfaces("luau-only"))[1], SURFACE_CRATES["js"] + RASTER)
            for product in ("panel", "tui"):
                required, forbidden = rules(product, self.surfaces("luau-only"))
                self.assertEqual(required, SURFACE_CRATES["luau"])
                self.assertEqual(forbidden, SURFACE_CRATES["js"])

        def test_neither_forbids_both_surfaces_and_the_raster(self):
            for product in PRODUCTS:
                required, forbidden = rules(product, self.surfaces("neither"))
                self.assertEqual(required, ())
                self.assertEqual(forbidden, SURFACE_CRATES["js"] + SURFACE_CRATES["luau"] + RASTER)

        def test_a_luau_crate_is_rejected_wherever_it_is_forbidden(self):
            tree = "0mlua v0.12.2\n1script-luau v0.2.9\n2tui v0.2.9\n"
            with self.assertRaises(ValueError):
                check_origin("tui", "mlua", tree, workspace)

        def test_mlua_sys_does_not_count_as_mlua(self):
            tree = "tui v0.2.9\nmlua-sys v0.13.0\nluau0-src v0.22.0+luau740\n"
            self.assertEqual((count_package("mlua", tree), count_package("mlua-sys", tree)), (0, 1))
            self.assertEqual(count_package("luau0-src", tree), 1)

    suite = unittest.TestSuite()
    for case in (NativeOriginTests, CompositionRuleTests):
        suite.addTests(unittest.defaultTestLoader.loadTestsFromTestCase(case))
    return int(not unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful())


if __name__ == "__main__":
    try:
        arguments = sys.argv[1:]
        if arguments == ["--self-test"]:
            raise SystemExit(self_test())
        if len(arguments) == 2 and arguments[0] == "--composition" and arguments[1] in COMPOSITIONS:
            raise SystemExit(main([arguments[1]]))
        if arguments:
            raise ValueError(f"usage: native_dependencies.py [--self-test | --composition {{{'|'.join(COMPOSITIONS)}}}]")
        raise SystemExit(main(list(COMPOSITIONS)))
    except (KeyError, OSError, ValueError, json.JSONDecodeError) as error:
        print(f"native dependency check: {error}", file=sys.stderr)
        raise SystemExit(1) from error
