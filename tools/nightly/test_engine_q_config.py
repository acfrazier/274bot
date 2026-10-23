#!/usr/bin/env python3
"""Run the Engine Q install layout through its real WorldConfig loader."""

import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
ENGINE_SOURCE = Path(
    os.environ.get(
        "NIGHTLY_ENGINE_Q_SOURCE",
        Path.home() / "experiments/lostcity-289-q/engine",
    )
).expanduser()


class EngineQConfigTests(unittest.TestCase):
    def test_q_config_loads_from_engine_data_config_world_json(self):
        node = shutil.which("node")
        if node is None:
            self.skipTest("Node.js is not installed")
        if not ENGINE_SOURCE.is_dir():
            self.skipTest(f"Engine Q source is not available at {ENGINE_SOURCE}")

        with tempfile.TemporaryDirectory() as temporary:
            engine_root = Path(temporary) / "engine"
            config_path = engine_root / "data" / "config" / "world.json"
            config_path.parent.mkdir(parents=True)
            shutil.copyfile(HERE / "engine-q" / "world.json", config_path)
            (engine_root / "world.json").write_text(
                '{"web":{"port":1080},"node":{"port":44594}}\n', encoding="utf-8"
            )

            result = subprocess.run(
                [
                    node,
                    "--import",
                    "tsx",
                    str(HERE / "engine-q" / "check-world-config.mjs"),
                    str(engine_root),
                    str(ENGINE_SOURCE),
                ],
                cwd=ENGINE_SOURCE,
                capture_output=True,
                text=True,
                timeout=30,
            )

        self.assertEqual(result.returncode, 0, msg=f"{result.stdout}{result.stderr}")
        observed = json.loads(result.stdout)
        self.assertEqual(Path(observed["configPath"]), Path(os.path.realpath(config_path)))
        self.assertEqual(observed["webPort"], 1180)
        self.assertEqual(observed["managementPort"], 9988)
        self.assertEqual(observed["gamePort"], 44694)
        self.assertEqual(observed["maxConnected"], 1)
        self.assertEqual(observed["dbBackend"], "sqlite")
        self.assertEqual(observed["dbName"], "lostcity-289-q")


if __name__ == "__main__":
    unittest.main()
