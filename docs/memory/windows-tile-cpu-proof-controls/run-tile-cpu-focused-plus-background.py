"""CPU fallback focused-plus-background wrapper."""
import os
import pathlib
import runpy
os.environ["TILE_CPU_MODE"] = "focused-plus-background"
runpy.run_path(str(pathlib.Path(__file__).with_name("run-tile-cpu-focused-one.py")), run_name="__main__")
