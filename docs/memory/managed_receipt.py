"""Persist an immutable completion receipt for a managed diagnostic cell.

The caller records launch/finish events as they occur. This writer verifies
their artifact bindings; it does not create launch history after the fact or
grant performance acceptance. Incomplete/failed cells remain in the receipt.
"""
import datetime
import json
import math
import pathlib
import re

from build_provenance import file_sha256
import cache_provenance as cp


RAW_FILES = ('metadata.json', 'samples.jsonl', 'samples.qualification.jsonl')
DIRECT_ADMISSION_BINDINGS = {
    'release_contract', 'receipt_conflict', 'receipt_account',
    'receipt_population', 'receipt_cache', 'receipt_server_health',
}

def utc_now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat().replace('+00:00', 'Z')


def write_new_json(path, value):
    with pathlib.Path(path).open('x', encoding='utf-8') as output:
        json.dump(value, output, indent=2, allow_nan=False)
        output.write('\n')


def _load(path):
    value = json.loads(pathlib.Path(path).read_text())
    if not isinstance(value, dict):
        raise ValueError(f'expected object: {pathlib.Path(path).name}')
    return value


def _timestamp(value):
    if not isinstance(value, str) or not value.endswith('Z'):
        raise ValueError('expected explicit UTC timestamp')
    return datetime.datetime.fromisoformat(value[:-1] + '+00:00').timestamp()


def _validated_bindings(value):
    if not isinstance(value, dict) or set(value) != DIRECT_ADMISSION_BINDINGS:
        raise ValueError('exact direct admission bindings are required')
    normalized = {}
    for label, entry in value.items():
        if (not isinstance(label, str) or not label
                or not isinstance(entry, dict) or set(entry) != {'path', 'sha256'}):
            raise ValueError('invalid direct admission binding')
        if not isinstance(entry['path'], str) or not entry['path']:
            raise ValueError('invalid direct admission binding: ' + label)
        path = pathlib.Path(entry['path']).resolve(strict=True)
        digest = entry['sha256']
        if (not isinstance(digest, str)
                or re.fullmatch(r'[0-9a-f]{64}', digest) is None
                or file_sha256(path) != digest):
            raise ValueError('invalid direct admission binding: ' + label)
        normalized[label] = {'path': str(path), 'sha256': digest}
    return normalized


def create_launch(path, *, cell_id, index, kind, effective_cli, binary,
                  manifest_path, server_identity_path, host_conditions_path,
                  sampler_config, cache_provenance_path=None, capture_mode=None,
                  direct_admission_bindings=None):
    """Call immediately before starting the launcher; never overwrites a cell."""
    if not isinstance(cell_id, str) or not cell_id or type(index) is not int or index < 1:
        raise ValueError('invalid cell id/index')
    if kind not in ('matched', 'overhead', 'diagnostic'):
        raise ValueError('invalid cell kind')
    if not isinstance(effective_cli, list) or not effective_cli or not all(isinstance(v, str) for v in effective_cli):
        raise ValueError('invalid launcher argv')
    if not isinstance(sampler_config, dict) or not sampler_config:
        raise ValueError('sampler configuration is required')
    # The actual reader validates command semantics and independent qualification.
    # Here preserve exactly the command/config recorded before launch.
    value = {'schema': 1, 'id': cell_id, 'index': index, 'kind': kind,
             'effective_cli': effective_cli, 'binary': str(pathlib.Path(binary).resolve(strict=True)),
             'sampler': sampler_config, 'performance_acceptance': False}
    if capture_mode is not None:
        if capture_mode != 'direct-owner-v1':
            raise ValueError('invalid capture mode')
        value['capture_mode'] = capture_mode
        value['direct_admission_bindings'] = _validated_bindings(
            direct_admission_bindings
        )
    elif direct_admission_bindings is not None:
        raise ValueError('direct admission bindings require direct owner mode')
    value['binary_sha256'] = file_sha256(value['binary'])
    if cache_provenance_path is not None:
        cache_path = pathlib.Path(cache_provenance_path).resolve(strict=True)
        cache_sha = file_sha256(cache_path)
        cache_value = cp.verify_snapshot(_load(cache_path))
        if file_sha256(cache_path) != cache_sha:
            raise ValueError('cache snapshot changed during launch verification')
        value['cache_provenance_path'] = str(cache_path)
        value['cache_provenance_sha256'] = cache_sha
        value['cache_content_identity_sha256'] = cache_value['content_identity_sha256']
        value['cache_verified_before_launch_utc'] = utc_now()
    for label, source in (('manifest', manifest_path), ('server_identity', server_identity_path),
                          ('host_conditions', host_conditions_path)):
        source = pathlib.Path(source).resolve(strict=True)
        _load(source)
        value[label + '_path'] = str(source)
        value[label + '_sha256'] = file_sha256(source)
    value['started_utc'] = utc_now()
    write_new_json(path, value)
    return value


def complete(path, *, launch_path, run_dir, launcher_exit_code, sampler_result,
             runner_errors=None):
    """Persist a receipt after children finish, preserving failures and gaps.

    Hashes bind raw bytes; they do not turn malformed data into valid metrics.
    Consumers must independently qualify/analyze and validate the full chain.
    """
    if type(launcher_exit_code) is not int:
        raise ValueError('launcher exit code must be an integer')
    if runner_errors is not None and (not isinstance(runner_errors, list)
            or any(not isinstance(e, str) or not e for e in runner_errors)):
        raise ValueError('runner errors must be a list of nonempty strings')
    launch_path = pathlib.Path(launch_path).resolve(strict=True)
    launch = _load(launch_path)
    launch_sha = file_sha256(launch_path)
    receipt = dict(launch)
    receipt.update(launch_path=str(launch_path), launch_sha256=launch_sha,
                   ended_utc=utc_now(), launcher_exit_code=launcher_exit_code,
                   sampler_result=sampler_result, raw_hashes={}, binding_errors=[])
    errors = receipt['binding_errors']
    receipt['runner_errors'] = list(runner_errors or [])
    errors.extend('runner:' + error for error in receipt['runner_errors'])
    if run_dir is not None and not pathlib.Path(run_dir).is_dir():
        errors.append('frontend_run_directory_missing')
        run_dir = None
    if run_dir is None:
        receipt.update(run_dir=None, exit_code=None, status='unavailable')
        errors.append('frontend_run_directory_unavailable')
    else:
        run_dir = pathlib.Path(run_dir).resolve(strict=True)
        receipt['run_dir'] = str(run_dir)
        raw_sources = {name: run_dir / name for name in RAW_FILES}
        if launch.get('capture_mode') == 'direct-owner-v1':
            raw_sources.update({
                'samples.owners.jsonl': run_dir / 'samples.owners.jsonl',
                'frontend-handoff.json': launch_path.parent / 'frontend-handoff.json',
                'direct-owner-guard.jsonl': launch_path.parent / 'direct-owner-guard.jsonl',
                'direct-owner-guard-summary.json': launch_path.parent / 'direct-owner-guard-summary.json',
            })
        for name, source in raw_sources.items():
            if source.is_file():
                receipt['raw_hashes'][name] = file_sha256(source)
            else:
                errors.append('missing_raw_file:' + name)
        try:
            meta = _load(run_dir / 'metadata.json')
            if meta.get('tui_input_probes') is True:
                source = run_dir / 'input-probes.jsonl'
                if source.is_file():
                    receipt['raw_hashes']['input-probes.jsonl'] = file_sha256(source)
                else:
                    errors.append('missing_raw_file:input-probes.jsonl')
            if (launch.get('capture_mode') == 'direct-owner-v1'
                    and meta.get('capture_mode') != 'direct-owner-v1'):
                errors.append('metadata_capture_mode_mismatch')
            receipt['exit_code'] = meta.get('exit_code')
            if meta.get('run_dir') is None or pathlib.Path(meta['run_dir']).resolve() != run_dir:
                errors.append('metadata_run_dir_mismatch')
            if meta.get('binary') is None or pathlib.Path(meta['binary']).resolve() != pathlib.Path(launch['binary']):
                errors.append('metadata_binary_mismatch')
            if meta.get('binary_sha256') != launch['binary_sha256']:
                errors.append('metadata_binary_hash_mismatch')
            if meta.get('effective_cli') != launch['effective_cli']:
                errors.append('metadata_launcher_argv_mismatch')
            begin, end = meta.get('started_unix'), meta.get('ended_unix')
            if (type(begin) not in (int, float) or type(end) not in (int, float)
                    or not math.isfinite(begin) or not math.isfinite(end)
                    or not _timestamp(launch['started_utc']) <= begin < end <= _timestamp(receipt['ended_utc'])):
                errors.append('metadata_time_envelope_invalid')
            if type(meta.get('exit_code')) is not int or meta['exit_code'] != 0:
                errors.append('frontend_failed_or_incomplete')
        except (ValueError, OSError, TypeError, KeyError):
            receipt.setdefault('exit_code', None)
            errors.append('metadata_unreadable_or_invalid')
    if launcher_exit_code != 0:
        errors.append('launcher_failed')
    if not isinstance(sampler_result, dict) or type(sampler_result.get('exit_code')) is not int or sampler_result['exit_code'] != 0:
        errors.append('sampler_failed_or_incomplete')
    sampler_path = sampler_result.get('output') if isinstance(sampler_result, dict) else None
    if not isinstance(sampler_path, str) or not pathlib.Path(sampler_path).is_file():
        errors.append('sampler_output_missing')
    else:
        sampler_path = pathlib.Path(sampler_path).resolve(strict=True)
        receipt['sampler_output_path'] = str(sampler_path)
        receipt['sampler_output_sha256'] = file_sha256(sampler_path)
    # Recheck every external artifact against its recorded pre-launch binding.
    for label in ('binary', 'manifest', 'server_identity', 'host_conditions'):
        source = launch['binary'] if label == 'binary' else launch[label + '_path']
        try:
            if file_sha256(source) != launch[label + '_sha256']:
                errors.append(label + '_changed_since_launch')
        except OSError:
            errors.append(label + '_unreadable_at_completion')
    if file_sha256(launch_path) != launch_sha:
        errors.append('launch_changed_during_completion')
    if 'cache_provenance_path' in launch:
        try:
            cache_path = pathlib.Path(launch['cache_provenance_path'])
            if file_sha256(cache_path) != launch['cache_provenance_sha256']:
                raise ValueError('snapshot changed')
            cache_value = cp.verify_snapshot(_load(cache_path))
            if (file_sha256(cache_path) != launch['cache_provenance_sha256']
                    or cache_value['content_identity_sha256'] != launch['cache_content_identity_sha256']):
                raise ValueError('cache identity changed')
            receipt['cache_verified_after_completion_utc'] = utc_now()
        except (ValueError, OSError, TypeError, KeyError):
            errors.append('cache_provenance_changed_or_invalid')
    if launch.get('capture_mode') == 'direct-owner-v1':
        bindings = launch.get('direct_admission_bindings')
        if not isinstance(bindings, dict) or set(bindings) != DIRECT_ADMISSION_BINDINGS:
            errors.append('direct_admission_bindings_missing')
        else:
            for label, entry in bindings.items():
                try:
                    if (not isinstance(label, str) or not isinstance(entry, dict)
                            or set(entry) != {'path', 'sha256'}
                            or file_sha256(entry['path']) != entry['sha256']):
                        raise ValueError('changed')
                except (OSError, TypeError, ValueError, KeyError):
                    errors.append('direct_admission_changed:' + str(label))
    if run_dir is not None:
        for name, digest in receipt['raw_hashes'].items():
            try:
                source = (run_dir / name if name in RAW_FILES or name in
                          ('samples.owners.jsonl', 'input-probes.jsonl') else launch_path.parent / name)
                if file_sha256(source) != digest:
                    errors.append('raw_changed_during_completion:' + name)
            except OSError:
                errors.append('raw_unreadable_during_completion:' + name)
    receipt['status'] = 'completed' if not errors else 'failed_or_unavailable'
    # No assertion of qualification, overhead or final binding eligibility.
    receipt['performance_acceptance'] = False
    write_new_json(path, receipt)
    return receipt
