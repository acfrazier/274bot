"""Validate schema-2 explicit-process samples against a native wall envelope.

This is resource accounting, not an instrumentation overhead experiment.
CPU endpoints enclose the observation and retain their timing uncertainty.
"""
import datetime
import hashlib
import json
import math
import pathlib
import statistics


def number(value, name, *, positive=False):
    if type(value) not in (int, float) or not math.isfinite(value):
        raise ValueError(f'{name}: finite number required')
    if value < 0 or (positive and value == 0):
        raise ValueError(f'{name}: invalid sign')
    return float(value)


def utc(value):
    if not isinstance(value, str) or not value.endswith('Z'):
        raise ValueError('UTC Z timestamp required')
    return datetime.datetime.fromisoformat(value[:-1] + '+00:00').timestamp()


def _validate(path, digest, *, roles, observation, interval_s, duration_mode,
              process_backend, clock_tolerance_s=0.05):
    path = pathlib.Path(path)
    data = path.read_bytes()
    if not isinstance(digest, str) or hashlib.sha256(data).hexdigest() != digest:
        raise ValueError('process artifact hash mismatch')
    rows = [json.loads(line) for line in data.splitlines()]
    if len(rows) < 4 or any(not isinstance(r, dict) for r in rows):
        raise ValueError('metadata, at least two samples and summary required')
    meta, summary = rows[0], rows[-1]
    samples = rows[1:-1]
    if meta.get('type') != 'metadata' or type(meta.get('schema')) is not int or meta['schema'] != 2:
        raise ValueError('schema-2 metadata required')
    if summary.get('type') != 'summary' or any(r.get('type') != 'sample' for r in samples):
        raise ValueError('unexpected or misplaced record')
    if process_backend not in ('system', 'libproc') or meta.get('process_backend') != process_backend:
        raise ValueError('process backend mismatch')
    interval = number(interval_s, 'interval', positive=True)
    if number(meta.get('interval_s'), 'metadata interval', positive=True) != interval:
        raise ValueError('interval mismatch')
    tolerance = number(meta.get('cadence_tolerance_s'), 'cadence tolerance')
    clock_tolerance = number(clock_tolerance_s, 'clock tolerance')
    if tolerance > interval:
        raise ValueError('cadence tolerance exceeds interval')
    if duration_mode not in ('fixed', 'stop_controlled') or meta.get('duration_mode') != duration_mode:
        raise ValueError('duration mode mismatch')
    if duration_mode == 'stop_controlled':
        if meta.get('duration_s') is not None or summary.get('completion') != 'controlled_stop' or summary.get('status') != 'closed':
            raise ValueError('controlled stop incomplete')
    else:
        number(meta.get('duration_s'), 'fixed duration', positive=True)
        if summary.get('completion') != 'required_grid_complete' or summary.get('status') != 'ok' or summary.get('grid_complete') is not True:
            raise ValueError('fixed grid incomplete')
        if type(meta.get('required_grid_sample_count')) is not int or meta['required_grid_sample_count'] != len(samples):
            raise ValueError('fixed grid count mismatch')
    if summary.get('fail_reason') is not None or summary.get('role_errors') != []:
        raise ValueError('collector failure')
    for key in ('sample_count', 'ok_sample_count'):
        if type(summary.get(key)) is not int or summary[key] != len(samples):
            raise ValueError('summary sample count mismatch')
    if not isinstance(roles, dict) or not roles:
        raise ValueError('expected roles required')
    pids = []
    for name, identity in roles.items():
        if not isinstance(name, str) or not name or not isinstance(identity, dict):
            raise ValueError('invalid role')
        pid = identity.get('pid')
        if type(pid) is not int or pid <= 0 or not isinstance(identity.get('start_identity'), str) or not identity['start_identity']:
            raise ValueError('role PID and start identity required')
        pids.append(pid)
    if len(set(pids)) != len(pids) or meta.get('roles') != {k: {'pid': v['pid']} for k, v in roles.items()}:
        raise ValueError('role mapping mismatch or duplicate PID')
    left, right = (number(v, 'observation wall time') for v in observation)
    if right <= left:
        raise ValueError('empty observation')
    points = {name: [] for name in roles}
    offsets = []
    last_sched = None
    prior_end = None
    for index, row in enumerate(samples):
        if type(row.get('sample_index')) is not int or row['sample_index'] != index:
            raise ValueError('sample grid index mismatch')
        sched = number(row.get('scheduled_monotonic_s'), 'scheduled time')
        a = number(row.get('acquisition_start_monotonic_s'), 'acquisition start')
        b = number(row.get('acquisition_end_monotonic_s'), 'acquisition end')
        wa, wb = utc(row.get('acquisition_before_utc')), utc(row.get('acquisition_after_utc'))
        if b < a or wb < wa or a < sched or a - sched > tolerance + 1e-6:
            raise ValueError('invalid acquisition bracket or late sample')
        if prior_end is not None and (a <= prior_end[0] or wa < prior_end[1]):
            raise ValueError('overlapping or reset acquisition clock')
        if last_sched is not None and not math.isclose(sched-last_sched, interval, abs_tol=1e-6):
            raise ValueError('missing scheduled interval')
        if abs((wb-wa)-(b-a)) > clock_tolerance:
            raise ValueError('wall clock jump during sweep')
        offsets.extend((wa-a, wb-b))
        prior_end, last_sched = (b, wb), sched
        if not isinstance(row.get('roles'), dict) or set(row['roles']) != set(roles):
            raise ValueError('missing or extra role sample')
        for name, identity in roles.items():
            r = row['roles'][name]
            if not isinstance(r, dict):
                raise ValueError('role sample must be an object')
            if r.get('status') != 'ok' or r.get('role') != name or type(r.get('pid')) is not int or r['pid'] != identity['pid'] or r.get('start_identity') != identity['start_identity']:
                raise ValueError('failed role or changed identity')
            ra = number(r.get('acquisition_start_monotonic_s'), 'role start')
            rb = number(r.get('acquisition_end_monotonic_s'), 'role end')
            if not a <= ra <= rb <= b:
                raise ValueError('role acquisition outside sweep')
            cpu = r.get('cpu') or {}
            if not isinstance(cpu, dict):
                raise ValueError('CPU sample must be an object')
            u = number(cpu.get('cumulative_user_s'), 'user CPU')
            s = number(cpu.get('cumulative_system_s'), 'system CPU')
            rss = number(r.get('resident_bytes'), 'RSS')
            if points[name] and (u < points[name][-1]['u'] or s < points[name][-1]['s']):
                raise ValueError('CPU counter reset')
            points[name].append(dict(wa=wa, wb=wb, a=ra, b=rb, u=u, s=s, rss=rss))
    if max(offsets)-min(offsets) > clock_tolerance:
        raise ValueError('wall and monotonic clocks diverged')
    results = {}
    for name, series in points.items():
        before = [p for p in series if p['wb'] <= left]
        after = [p for p in series if p['wa'] >= right]
        inside = [p for p in series if p['wa'] >= left and p['wb'] <= right]
        if not before or not after or not inside:
            raise ValueError('process series does not enclose observation')
        first, last = before[-1], after[0]
        if left-first['wb'] > interval+tolerance+clock_tolerance or last['wa']-right > interval+tolerance+clock_tolerance:
            raise ValueError('endpoint too far from observation')
        duration_min, duration_max = last['a']-first['b'], last['b']-first['a']
        if duration_min <= 0:
            raise ValueError('CPU endpoint brackets overlap')
        cpu_s = last['u']+last['s']-first['u']-first['s']
        results[name] = {
            **roles[name], 'resident_median_bytes': statistics.median(p['rss'] for p in inside),
            'resident_sample_count': len(inside), 'sampled_resident_peak_bytes': max(p['rss'] for p in series),
            'cpu_s_enclosing_observation': cpu_s,
            'cpu_cores_interval': [cpu_s/duration_max, cpu_s/duration_min],
            'cpu_duration_interval_s': [duration_min, duration_max],
            'cpu_wall_envelope': [first['wa'], last['wb']],
            'cpu_endpoint_excess_s': [left-first['wa'], last['wb']-right],
        }
    if hashlib.sha256(path.read_bytes()).hexdigest() != digest:
        raise ValueError('process artifact changed while reading')
    return {'status': 'available', 'roles': results, 'process_backend': process_backend,
            'observation_wall_envelope': [left, right], 'continuous_coverage': True,
            'sample_count': len(samples), 'artifact_sha256': digest,
            'instrumentation_overhead_measured': False, 'performance_acceptance': False,
            'pressure': {'status': 'unassessed', 'rows': [s.get('host_pressure') for s in samples]},
            'waited_children_cpu': {'status': 'unassessed', 'rows': [s.get('waited_children_cpu') for s in samples]},
            'scope': 'explicit PIDs only; RSS samples are not continuous peak capture; CPU brackets enclose observation'}


def validate(path, digest, **kwargs):
    """Invalid, mutated or incomplete series remain unavailable."""
    try:
        return _validate(path, digest, **kwargs)
    except (ValueError, TypeError, KeyError, OSError, OverflowError) as error:
        return {'status': 'unavailable', 'reason': str(error),
                'instrumentation_overhead_measured': False, 'performance_acceptance': False}
