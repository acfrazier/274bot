"""Focused-plus-background wrapper for the single frozen tile probe cell."""
import os
import runpy

os.environ["RENDER_OWNER_CENSUS_MODE"] = "focused-plus-background"
runpy.run_path(
    os.path.join(os.path.dirname(__file__), "run-panel-tile-probe-focused-one.py"),
    run_name="__main__",
)
