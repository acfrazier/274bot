#!/usr/bin/env python3
"""Generate the local game data and navigation payloads used by product builds."""

import argparse
import os
import subprocess
from pathlib import Path

REVISIONS = ("274", "289")
NAV_NAMES = (
    "274bot.navpack",
    "274bot.navflags",
    "274bot.navreach",
    "274bot.navcanlight",
    "274bot.navpois",
)


def nav_pack_binary(target_dir, *, windows=None):
    if windows is None:
        windows = os.name == "nt"
    return Path(target_dir) / "release" / ("nav-pack.exe" if windows else "nav-pack")


def run(command, *, cwd, env):
    subprocess.run(command, cwd=cwd, env=env, check=True)


def build_local_data(repo_root, environment, *, runner=run, windows=None):
    if environment.get("BOT_NAV_BUILD") == "skip":
        return
    root = Path(repo_root).resolve()
    target_dir = Path(environment.get("CARGO_TARGET_DIR") or root / "target")
    if not target_dir.is_absolute():
        target_dir = root / target_dir
    game_data_dir = root / "crates/api/data/game-data"
    nav_root = root / "target/local-data/nav"
    projection = root / "target/local-data/prereq-crossings-289.json"
    bun = environment.get("BUN_EXECUTABLE", "bun")

    runner(
        [bun, "tools/game-data/generate.ts", "--stage", "1"],
        cwd=root,
        env=environment,
    )
    runner(
        ["cargo", "build", "--locked", "--release", "-p", "nav-bake"],
        cwd=root,
        env=environment,
    )

    nav_pack = nav_pack_binary(target_dir, windows=windows)
    payloads = {}
    for revision in REVISIONS:
        engine = Path(environment[f"GAME_DATA_{revision}_ENGINE"])
        content = Path(environment[f"GAME_DATA_{revision}_CONTENT"])
        output_dir = nav_root / revision
        output_dir.mkdir(parents=True, exist_ok=True)
        pack = output_dir / "274bot.navpack"
        runner(
            [
                str(nav_pack),
                "--revision",
                revision,
                "--engine",
                str(engine),
                "--content",
                str(content),
                "--cache",
                str(engine / "data/pack/client"),
                "--game-data-dir",
                str(game_data_dir),
                "--out",
                str(pack),
                "--flags-out",
                str(output_dir / "274bot.navflags"),
            ],
            cwd=root,
            env=environment,
        )
        payloads[revision] = pack
        for name in NAV_NAMES:
            path = output_dir / name
            if not path.is_file():
                raise FileNotFoundError(f"nav bake omitted required payload: {path}")

    projection.parent.mkdir(parents=True, exist_ok=True)
    runner(
        [
            str(nav_pack),
            "--export-crossings",
            str(payloads["289"]),
            "--out",
            str(projection),
        ],
        cwd=root,
        env=environment,
    )
    stage_two_environment = environment.copy()
    stage_two_environment["GAME_DATA_289_PREREQ_CROSSINGS"] = str(projection)
    runner(
        [bun, "tools/game-data/generate.ts", "--stage", "2"],
        cwd=root,
        env=stage_two_environment,
    )


def main(argv=None):
    argparse.ArgumentParser(description=__doc__).parse_args(argv)
    repo_root = Path(__file__).resolve().parents[2]
    build_local_data(repo_root, os.environ.copy())


if __name__ == "__main__":
    main()
