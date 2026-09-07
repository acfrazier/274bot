"""Explicit-PID macOS libproc counters without a ps subprocess.

RUSAGE_INFO_V0 CPU times are Mach absolute units, converted with the host's
mach_timebase_info. Resident size is current bytes, not physical footprint or
ru_maxrss. This optional backend does not alter server_resources defaults.
"""
import ctypes
import functools
import math
import os
import sys
import time

from server_resources import SampleError


class UsageV0(ctypes.Structure):
    # SDK sys/resource.h rusage_info_v0, 16-byte UUID followed by ten u64s.
    _fields_ = [('uuid', ctypes.c_ubyte * 16)] + [(name, ctypes.c_uint64) for name in (
        'user_time', 'system_time', 'pkg_idle_wakeups', 'interrupt_wakeups',
        'pageins', 'wired_size', 'resident_size', 'physical_footprint',
        'process_start', 'process_exit')]


class Timebase(ctypes.Structure):
    _fields_ = [('numer', ctypes.c_uint32), ('denom', ctypes.c_uint32)]


@functools.lru_cache(maxsize=1)
def _api():
    if sys.platform != 'darwin':
        raise SampleError('libproc backend is supported only on macOS')
    library = ctypes.CDLL('/usr/lib/libproc.dylib', use_errno=True)
    library.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
    library.proc_pid_rusage.restype = ctypes.c_int
    system = ctypes.CDLL('/usr/lib/libSystem.B.dylib')
    system.mach_timebase_info.argtypes = [ctypes.POINTER(Timebase)]
    system.mach_timebase_info.restype = ctypes.c_int
    base = Timebase()
    if system.mach_timebase_info(ctypes.byref(base)) != 0 or not base.numer or not base.denom:
        raise SampleError('mach_timebase_info failed')
    if ctypes.sizeof(UsageV0) != 96:
        raise SampleError('unsupported rusage_info_v0 ABI layout')
    return library, int(base.numer), int(base.denom)


def decode(usage, numer, denom):
    if type(numer) is not int or type(denom) is not int or numer <= 0 or denom <= 0:
        raise SampleError('invalid Mach timebase')
    if usage.process_start == 0 or usage.process_exit != 0:
        raise SampleError('process is not live or start identity is unavailable')
    scale = numer / denom / 1_000_000_000
    return {
        'start_identity': f'darwin_proc_start_abstime:{usage.process_start}',
        'resident_bytes': int(usage.resident_size),
        'user_s': int(usage.user_time) * scale,
        'system_s': int(usage.system_time) * scale,
        'physical_footprint_bytes': int(usage.physical_footprint),
        'process_uuid': bytes(usage.uuid).hex(),
        'provenance': {
            'os': 'darwin', 'backend': 'libproc_rusage_v0',
            'rss': 'proc_pid_rusage(RUSAGE_INFO_V0).ri_resident_size; current bytes',
            'cpu': 'ri_user_time/ri_system_time Mach units * timebase.numer / timebase.denom / 1e9',
            'mach_timebase_numer': numer, 'mach_timebase_denom': denom,
            'identity': 'ri_proc_start_abstime; PID supplied separately by role mapping',
            'sampling_subprocesses': False,
            'physical_footprint': 'separate gauge, never added to or substituted for RSS',
        },
    }


def sample_process(pid, timeout=5.0):
    if type(pid) is not int or not 0 < pid <= 2_147_483_647:
        raise SampleError('PID must be a positive signed 32-bit integer')
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0:
        raise SampleError('native sample timeout must be finite and positive')
    started = time.monotonic()
    try:
        api, numer, denom = _api()
        usage = UsageV0()
        ctypes.set_errno(0)
        if api.proc_pid_rusage(pid, 0, ctypes.byref(usage)) != 0:
            code = ctypes.get_errno()
            raise SampleError(f'proc_pid_rusage failed for PID {pid}: {os.strerror(code)}')
        value = decode(usage, numer, denom)
    except (OSError, AttributeError) as error:
        raise SampleError(f'libproc unavailable: {error}') from error
    elapsed = time.monotonic() - started
    if elapsed > timeout:
        raise SampleError('native process sample exceeded acquisition budget')
    value['provenance']['native_call_duration_s'] = elapsed
    value['provenance']['timeout_semantics'] = 'post-call acquisition budget check; kernel call is not cancellable'
    return value
