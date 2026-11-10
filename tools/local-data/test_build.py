import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("build.py")
spec = importlib.util.spec_from_file_location("local_data_build", SCRIPT)
build = importlib.util.module_from_spec(spec)
spec.loader.exec_module(build)


class BuildWrapperTests(unittest.TestCase):
    def test_local_bootstrap_runs_real_stages_and_crossings_in_order(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            engine_274 = root / "engine-274"
            engine_289 = root / "engine-289"
            content_274 = root / "content-274"
            content_289 = root / "content-289"
            environment = {
                "CARGO_TARGET_DIR": str(root / "cargo-target"),
                "GAME_DATA_274_ENGINE": str(engine_274),
                "GAME_DATA_274_CONTENT": str(content_274),
                "GAME_DATA_289_ENGINE": str(engine_289),
                "GAME_DATA_289_CONTENT": str(content_289),
                "RS2B0T": str(root / "rs2b0t"),
            }
            commands = []

            def fake_run(command, *, cwd, env):
                commands.append((command, Path(cwd), dict(env)))
                if "--export-crossings" in command:
                    output = Path(command[command.index("--out") + 1])
                    output.parent.mkdir(parents=True, exist_ok=True)
                    output.write_text(json.dumps({
                        "schema": 3,
                        "crossings": [],
                        "identical_edge_groups": [],
                    }))
                elif str(command[0]).endswith("nav-pack"):
                    output = Path(command[command.index("--out") + 1])
                    output.parent.mkdir(parents=True, exist_ok=True)
                    for name in build.NAV_NAMES:
                        (output.parent / name).write_bytes(name.encode())

            build.build_local_data(root, environment, runner=fake_run, windows=False)

            self.assertEqual(len(commands), 6)
            self.assertEqual(commands[0][0], [
                "bun", "tools/game-data/generate.ts", "--stage", "1",
            ])
            self.assertEqual(commands[1][0], [
                "cargo", "build", "--locked", "--release", "-p", "nav-bake",
            ])
            self.assertNotIn("--workspace", commands[1][0])
            self.assertIn("--revision", commands[2][0])
            self.assertEqual(commands[2][0][commands[2][0].index("--revision") + 1], "274")
            self.assertEqual(commands[3][0][commands[3][0].index("--revision") + 1], "289")
            self.assertEqual(
                commands[2][0][commands[2][0].index("--cache") + 1],
                str(engine_274 / "data/pack/client"),
            )
            self.assertEqual(
                commands[2][0][commands[2][0].index("--engine") + 1],
                str(engine_274),
            )
            self.assertEqual(
                commands[3][0][commands[3][0].index("--engine") + 1],
                str(engine_289),
            )
            export = commands[4][0]
            self.assertEqual(export[export.index("--export-crossings") + 1], str(
                root / "target/local-data/nav/289/274bot.navpack"
            ))
            self.assertEqual(commands[5][0], [
                "bun", "tools/game-data/generate.ts", "--stage", "2",
            ])
            self.assertEqual(
                commands[5][2]["GAME_DATA_289_PREREQ_CROSSINGS"],
                str(root / "target/local-data/prereq-crossings-289.json"),
            )
            self.assertNotIn("GAME_DATA_289_PREREQ_CROSSINGS", environment)
            for revision in build.REVISIONS:
                for name in build.NAV_NAMES:
                    self.assertTrue((root / "target/local-data/nav" / revision / name).is_file())

    def test_bootstrap_uses_resolved_bun_without_expanding_cargo_path(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            environment = {
                "BUN_EXECUTABLE": str(root / "tools with spaces/bun"),
                "PATH": "/restricted/cargo-and-system",
            }
            for revision in build.REVISIONS:
                environment[f"GAME_DATA_{revision}_ENGINE"] = str(root / revision)
                environment[f"GAME_DATA_{revision}_CONTENT"] = str(root / revision)
                output = root / "target/local-data/nav" / revision
                output.mkdir(parents=True)
                for name in build.NAV_NAMES:
                    (output / name).write_bytes(b"fixture")
            commands = []
            build.build_local_data(
                root,
                environment,
                runner=lambda command, **kwargs: commands.append((command, kwargs["env"])),
            )
            for index in (0, 5):
                self.assertEqual(commands[index][0][0], environment["BUN_EXECUTABLE"])
            self.assertEqual(commands[1][0][0], "cargo")
            for _, command_environment in commands:
                self.assertEqual(command_environment["PATH"], "/restricted/cargo-and-system")

    def test_skip_does_not_run_or_stage_local_data(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            commands = []

            build.build_local_data(
                root,
                {"BOT_NAV_BUILD": "skip"},
                runner=lambda command, **kwargs: commands.append(command),
                windows=False,
            )

            self.assertEqual(commands, [])
            self.assertFalse((root / "target/local-data").exists())

    def test_nav_pack_binary_uses_native_windows_extension(self):
        self.assertEqual(
            build.nav_pack_binary(Path("target"), windows=True),
            Path("target/release/nav-pack.exe"),
        )
        self.assertEqual(
            build.nav_pack_binary(Path("target"), windows=False),
            Path("target/release/nav-pack"),
        )


if __name__ == "__main__":
    unittest.main()
