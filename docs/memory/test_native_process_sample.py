import ctypes
import os
import resource
import sys
import time
import unittest
from unittest import mock

import native_process_sample as ns


class NativeSampleTests(unittest.TestCase):
    def usage(self):
        value = ns.UsageV0()
        value.process_start = 123
        value.user_time = 24_000_000
        value.system_time = 12_000_000
        value.resident_size = 8192
        value.physical_footprint = 4096
        return value

    def test_sdk_v0_layout(self):
        self.assertEqual(ctypes.sizeof(ns.UsageV0), 96)
        self.assertEqual(ns.UsageV0.user_time.offset, 16)
        self.assertEqual(ns.UsageV0.resident_size.offset, 64)
        self.assertEqual(ns.UsageV0.process_start.offset, 80)

    def test_mach_conversion_and_distinct_rss(self):
        value = ns.decode(self.usage(), 125, 3)
        self.assertAlmostEqual(value['user_s'], 1)
        self.assertAlmostEqual(value['system_s'], 0.5)
        self.assertEqual(value['resident_bytes'], 8192)
        self.assertEqual(value['physical_footprint_bytes'], 4096)
        self.assertFalse(value['provenance']['sampling_subprocesses'])

    def test_exited_and_unknown_identity_rejected(self):
        for field, val in [('process_exit', 456), ('process_start', 0)]:
            value = self.usage()
            setattr(value, field, val)
            with self.assertRaises(ns.SampleError):
                ns.decode(value, 125, 3)

    def test_bad_pid_and_budget_rejected_before_api(self):
        with mock.patch.object(ns, '_api') as api:
            for pid in (True, 0, -1, 2**31, 1.5):
                with self.assertRaises(ns.SampleError): ns.sample_process(pid)
            for budget in (None, True, 0, float('nan')):
                with self.assertRaises(ns.SampleError): ns.sample_process(1, budget)
            api.assert_not_called()

    def test_native_failure_returns_no_zero_sample(self):
        api = mock.Mock()
        api.proc_pid_rusage.return_value = -1
        with mock.patch.object(ns, '_api', return_value=(api, 125, 3)), self.assertRaises(ns.SampleError):
            ns.sample_process(123)

    @unittest.skipUnless(sys.platform == 'darwin', 'requires macOS libproc')
    def test_live_self_counters_match_getrusage_units_without_children(self):
        before_children = resource.getrusage(resource.RUSAGE_CHILDREN)
        first = ns.sample_process(os.getpid())
        until = time.monotonic() + 0.025
        while time.monotonic() < until:
            sum(range(100))
        last = ns.sample_process(os.getpid())
        usage = resource.getrusage(resource.RUSAGE_SELF)
        self.assertEqual(first['start_identity'], last['start_identity'])
        self.assertGreater(last['resident_bytes'], 0)
        self.assertGreater(last['user_s'], first['user_s'])
        self.assertAlmostEqual(last['user_s'], usage.ru_utime, delta=0.005)
        self.assertAlmostEqual(last['system_s'], usage.ru_stime, delta=0.005)
        after_children = resource.getrusage(resource.RUSAGE_CHILDREN)
        self.assertEqual(before_children.ru_utime, after_children.ru_utime)
        self.assertEqual(before_children.ru_stime, after_children.ru_stime)


if __name__ == '__main__':
    unittest.main()
