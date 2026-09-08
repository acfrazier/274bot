"""Paired controller wrapper for focused renderer plus 1 fps background."""
import os
import pathlib
import runpy

os.environ["TILE_BOXED_MODE"] = "focused-plus-background"
shared = pathlib.Path(__file__).with_name("run-tile-boxed-focused-one.py")
runpy.run_path(str(shared), run_name="__main__")
