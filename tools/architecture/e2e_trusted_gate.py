"""End-to-end enforcement qualification with real nested fixture suites.

Run after the corpus, from the worktree, with the same environment:

    python3 -m unittest discover -s tools/architecture -p 'e2e_*.py' -v

Qualification runs `check.py --self-test` and the `test_*.py` corpus. That
corpus never launches a nested suite, so no qualification process needs a
recursion flag that E' could read. This module does launch the real suites,
which is why it must stay outside the `test_*.py` pattern.
"""
from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("architecture_gate_corpus", HERE / "test_trusted_gate.py")
corpus = importlib.util.module_from_spec(_spec)
sys.modules[_spec.name] = corpus
_spec.loader.exec_module(corpus)


class RealSuiteQualificationTests(corpus.GateFixture):
    real_suites = True

    def test_two_merge_flow_with_real_qualification_suites(self) -> None:
        self._flow_two_merge()

    def test_production_only_e_prime_is_refused_with_real_suites(self) -> None:
        self._poisoned_gate_e_prime("production-only-E-prime", corpus.PRODUCTION_ONLY_POISON)


if __name__ == "__main__":
    unittest.main()
