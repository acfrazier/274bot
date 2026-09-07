"""Repeated, artifact-bound resource screen; latency and final gates stay separate.

The four resource cells are reference/candidate/candidate/reference with hot
profiles disabled. An explicit independently re-read overhead quartet records
why profiled CPU is a separate diagnostic. No overhead subtraction is applied.
"""
import pathlib

import instrumentation_overhead as io
import matched_evidence_adapter as mea


ROLES = ('reference', 'candidate', 'candidate', 'reference')


def unavailable(reason, **details):
    return dict(status='unavailable', reason=reason, final_acceptance=False,
                accepted_rss_saving=False, **details)


def _bind(path, role, manifest):
    # Sidecar paths are read from the receipt, then independently hash/path
    # checked by bind_side. Their contents are never accepted from this load.
    import json
    receipt = json.loads(pathlib.Path(path).read_text())
    if not isinstance(receipt, dict):
        raise ValueError('receipt must be a JSON object')
    return mea.bind_side(path, role=role, manifest_path=manifest,
                         server_identity_path=receipt.get('server_identity_path'),
                         host_conditions_path=receipt.get('host_conditions_path'))


def _range(values):
    return [min(values), max(values)]


def analyze(receipts, *, manifest, overhead_receipts):
    """Consume explicit chronological R/C/C/R receipts and O/I/I/O receipts."""
    try:
        return _analyze(receipts, manifest=manifest, overhead_receipts=overhead_receipts)
    except (ValueError, TypeError, KeyError, OSError, OverflowError) as error:
        return unavailable('artifact_validation_failed', detail=str(error))


def _analyze(receipts, *, manifest, overhead_receipts):
    if not isinstance(receipts, (list, tuple)) or len(receipts) != 4:
        return unavailable('four_ordered_resource_receipts_required')
    if not isinstance(overhead_receipts, (list, tuple)) or len(overhead_receipts) != 4:
        return unavailable('explicit_overhead_quartet_required')
    import json
    overhead_first = json.loads(pathlib.Path(overhead_receipts[0]).read_text())
    if not isinstance(overhead_first, dict):
        return unavailable('overhead_receipt_must_be_object')
    overhead = io.analyze_instrumentation_overhead(
        overhead_receipts, manifest_path=manifest,
        server_identity_path=overhead_first.get('server_identity_path'),
        host_conditions_path=overhead_first.get('host_conditions_path'))
    if overhead.get('resource_deltas_available') is not True:
        return unavailable('overhead_resource_screen_unavailable', overhead=overhead)
    sides = [_bind(path, role, manifest) for path, role in zip(receipts, ROLES)]
    for i, side in enumerate(sides):
        if (side.get('binding_ok') is not True or side.get('qualified') is not True
                or side.get('native_qualification', {}).get('status') != 'available'
                or side.get('managed_resources', {}).get('status') != 'available'):
            return unavailable('resource_side_unavailable', index=i, side_reason=side.get('reason'))
        keys = side.get('match_keys', {})
        if (keys.get('frontend') != 'tui' or keys.get('terminal') is not True
                or any(keys.get(flag) is not False for flag in io.PROFILE_GROUP_KEYS + (io.GPU_PROFILE_KEY,))):
            return unavailable('resource_cells_require_unprofiled_real_tui', index=i)
        if any(keys.get(flag) is not False for flag in ('allocation_counting', 'diagnostic_sidecar', 'stack_logging', 'failure_capture', 'nav_captures')):
            return unavailable('resource_cell_contamination_flags', index=i)
        if side.get('role') != ROLES[i]:
            return unavailable('resource_role_order_invalid', index=i)
    identities = [side['run_identity']['identity_sha256'] for side in sides]
    if len(set(identities)) != 4 or len({side['run_dir'] for side in sides}) != 4:
        return unavailable('duplicate_resource_run')
    spans = [side['observation_wall_span'] for side in sides]
    if any(spans[i][1] > spans[i + 1][0] for i in range(3)):
        return unavailable('resource_order_or_overlap_invalid')
    for indices in ((0, 3), (1, 2)):
        if sides[indices[0]]['binary_sha256'] != sides[indices[1]]['binary_sha256']:
            return unavailable('same_role_binary_changed')
    if sides[1]['binary_sha256'] != overhead['protocol']['same_binary_sha256']:
        return unavailable('overhead_candidate_binary_mismatch')
    # Exact runtime/fixture/host comparison across all four resource runs.
    for side in sides[1:]:
        if (not mea._typed_equal(side['match_keys'], sides[0]['match_keys'])
                or not mea._typed_equal(side['native_qualification']['match_keys'], sides[0]['native_qualification']['match_keys'])):
            return unavailable('resource_runtime_or_fixture_mismatch')
    # The N16 overhead experiment informs profile choice, not an extrapolated
    # overhead bound at another scale. All resource helpers are measured anew.
    overhead_side = _bind(overhead_receipts[0], 'candidate', manifest)
    for key in ('sampler_backend', 'sampler_modules', 'host_conditions', 'server_start_identity',
                'server_configuration', 'cache_settings', 'feature_flags', 'tui_input_probes'):
        if not mea._typed_equal(sides[1]['match_keys'].get(key), overhead_side.get('match_keys', {}).get(key)):
            return unavailable('overhead_environment_mismatch', field=key)
    resources = [io._extract_cell_resources(side) for side in sides]
    if any(row is None or row['continuous_coverage'] is not True for row in resources):
        return unavailable('continuous_resources_unavailable')
    role_error = io._identity_check([row['roles'] for row in resources])
    if role_error:
        return unavailable(role_error)
    hosts = [row['host'] for row in resources]
    ref_cpu = [hosts[i]['cpu_cores'] for i in (0, 3)]
    cand_cpu = [hosts[i]['cpu_cores'] for i in (1, 2)]
    cpu = io._cpu_screen(ref_cpu, cand_cpu)
    # Preserve the tested arithmetic, label its inputs for this experiment.
    cpu = {key.replace('off', 'reference').replace('on_', 'candidate_'): value for key, value in cpu.items()}
    cpu['rule'] = 'candidate max / reference min <=1.05 with both replicated spreads <=5%; crossing margin is inconclusive'
    if isinstance(cpu.get('reason'), str):
        cpu['reason'] = (cpu['reason'].replace('OFF', 'reference').replace('ON', 'candidate')
                         .replace('off_spread', 'reference_spread').replace('on_spread', 'candidate_spread'))
    ref_rss = [hosts[i]['resident_median_bytes'] for i in (0, 3)]
    cand_rss = [hosts[i]['resident_median_bytes'] for i in (1, 2)]
    minimum_reduction = min(ref_rss) - max(cand_rss)
    largest_spread = max(max(ref_rss) - min(ref_rss), max(cand_rss) - min(cand_rss))
    rss_supported = minimum_reduction > largest_spread
    return dict(
        status='screen_available', final_acceptance=False, accepted_rss_saving=False,
        candidate_resource_screen_supported=(rss_supported and cpu['status'] == 'within_5pct_empirical_screen'),
        cpu=cpu, rss=dict(reference_medians=ref_rss, candidate_medians=cand_rss,
                         reference_range=_range(ref_rss), candidate_range=_range(cand_rss),
                         minimum_observed_reduction_bytes=minimum_reduction,
                         largest_within_role_spread_bytes=largest_spread,
                         reduction_exceeds_observed_variation=rss_supported,
                         paired_candidate_minus_reference_bytes=[cand_rss[0]-ref_rss[0], cand_rss[1]-ref_rss[1]]),
        overhead_screen=overhead, resources_by_cell=resources,
        cells=[dict(role=role, receipt=str(path), run_identity=ident, observation_wall_span=span)
               for role, path, ident, span in zip(ROLES, receipts, identities, spans)],
        renderer_evidence='profile-off rows do not identify the actual backend; native draw/settings and requested TUI policy matched',
        pending=['longer_confirmation', 'latency_companion', 'actual_renderer_backend_evidence', 'absolute_targets', 'behavior_lifecycle', 'target_hardware_or_resource_limits', 'whole_branch_review'],
        note='Observed replicate ranges, not confidence intervals. No helper or profiler subtraction. This resource-only screen does not grant full pair or campaign acceptance.')
