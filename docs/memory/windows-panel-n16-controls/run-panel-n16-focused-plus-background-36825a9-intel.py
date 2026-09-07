"""Second, sequential N16 controller: focused renderer plus configured 1 fps background.
The shared controller still writes explicit mode/count/adapter provenance and all
managed ownership receipts. This wrapper is only run after focused-one teardown.
"""
import os
import pathlib
import runpy

os.environ["N16_DIAGNOSTIC_LABEL"] = "n16-focused-plus-background-console-intel"
os.environ["N16_DIAGNOSTIC_MODE"] = "focused-plus-background"
os.environ["N16_DIAGNOSTIC_ADAPTER"] = "Intel(R) Graphics"
shared = pathlib.Path(__file__).with_name("run-panel-n16-focused-one-36825a9-intel.py")
runpy.run_path(str(shared), run_name="__main__")
