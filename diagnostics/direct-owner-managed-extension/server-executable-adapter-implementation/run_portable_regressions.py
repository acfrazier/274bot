#!/usr/bin/env python3
"""Run affected portable tests except the independently reproduced HEAD failure."""
import pathlib
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from docs.memory import test_current_tui_calibration
from docs.memory import test_run_managed_cell

EXCLUDED = {
    "docs.memory.test_run_managed_cell.ManagedCellTests."
    "test_default_collector_stops_after_pad_before_generated_stop_and_c"
}


def flatten(suite):
    for test in suite:
        if isinstance(test, unittest.TestSuite):
            yield from flatten(test)
        else:
            yield test


loader = unittest.defaultTestLoader
loaded = unittest.TestSuite([
    loader.loadTestsFromModule(test_current_tui_calibration),
    loader.loadTestsFromModule(test_run_managed_cell),
])
selected = unittest.TestSuite(test for test in flatten(loaded) if test.id() not in EXCLUDED)
result = unittest.TextTestRunner(verbosity=1).run(selected)
raise SystemExit(0 if result.wasSuccessful() else 1)
