"""Bind managed cache and continuous process evidence to native qualification.

Called after the receipt/build/workload binding, not a replacement for it.
No caller-supplied performance or overhead label is promoted to evidence.
"""
import json
import os
import pathlib

import build_provenance as bp
import cache_provenance as cp
import managed_receipt as mr
import process_evidence as pe


# Actual Win32 ctypes sampler identity prefix (windows_process_sample.py).
# Used to require that dependency from producer evidence, not reader sys.platform.
_WINDOWS_CREATION_IDENTITY_PREFIX = 'windows_creation_filetime:'


def _object(value, name):
    if not isinstance(value, dict) or not value:
        raise ValueError('missing object: ' + name)
    return value


def _bound_file(record, path_key, hash_key):
    path_value = record.get(path_key)
    if not isinstance(path_value, str) or not path_value:
        raise ValueError('missing path: ' + path_key)
    path = pathlib.Path(path_value).resolve(strict=True)
    digest = bp._digest(record.get(hash_key), hash_key)
    if bp.file_sha256(path) != digest:
        raise ValueError('file hash mismatch: ' + path_key)
    return path, digest


def _same_directory_identity(left, right):
    """Compare existing directories by filesystem identity, not spelling."""
    try:
        left_path = pathlib.Path(left)
        right_path = pathlib.Path(right)
        if not left_path.is_dir() or not right_path.is_dir():
            return False
        return os.path.samefile(left_path, right_path)
    except (OSError, ValueError, TypeError):
        return False


def _roles_require_windows_process_sample(roles):
    """True when runtime role identities prove the Win32 sampler produced them.

    Historical Windows receipts carry ``windows_creation_filetime:<u64>``
    start identities. A Mac/Linux reader must still demand the bound
    ``windows_process_sample.py`` bytes rather than blessing an unbound receipt
    because the reviewing machine is not win32.
    """
    if not isinstance(roles, dict):
        return False
    for identity in roles.values():
        if not isinstance(identity, dict):
            continue
        start = identity.get('start_identity')
        if isinstance(start, str) and start.startswith(_WINDOWS_CREATION_IDENTITY_PREFIX):
            return True
    return False


def _cache(receipt, native):
    path, digest = _bound_file(receipt, 'cache_provenance_path', 'cache_provenance_sha256')
    value = cp.verify_snapshot(json.loads(path.read_text()))
    if value['content_identity_sha256'] != receipt.get('cache_content_identity_sha256'):
        raise ValueError('cache content identity differs from receipt')
    before = mr._timestamp(receipt.get('cache_verified_before_launch_utc'))
    after = mr._timestamp(receipt.get('cache_verified_after_completion_utc'))
    if not before <= mr._timestamp(receipt['started_utc']) < mr._timestamp(receipt['ended_utc']) <= after:
        raise ValueError('cache verification does not bracket managed cell')
    settings = _object(native['match_keys']['qualification_settings'], 'native settings')
    if settings.get('cache_dir_canonical_available') is not True or not _same_directory_identity(
        settings.get('cache_dir_canonical'), value['cache_dir']
    ):
        raise ValueError('native cache directory differs from fingerprint')
    if bp.file_sha256(path) != digest:
        raise ValueError('cache fingerprint changed while reading')
    return {'status':'available', 'path':str(path), 'artifact_sha256':digest,
            'match_keys':{k:value[k] for k in ('cache_dir','unpack_root','snapshot_version','content_identity_sha256')},
            'scope':value['scope'], 'network_asset_contents':value['network_asset_contents']}


def _process(receipt, native, server_identity):
    sampler = _object(receipt.get('sampler'), 'sampler')
    result = _object(receipt.get('sampler_result'), 'sampler_result')
    if type(result.get('exit_code')) is not int or result['exit_code'] != 0:
        raise ValueError('collector did not exit successfully')
    if native.get('observation_wall_span_source') != 'native_elapsed_wall_bracket':
        raise ValueError('native wall envelope required for process coverage')
    roles = _object(result.get('role_identities'), 'role identities')
    declared = _object(sampler.get('roles'), 'prelaunch roles')
    if not {'game_server','controller'} <= set(declared) or {'launcher','collector'} & set(declared):
        raise ValueError('invalid prelaunch role set')
    helper_names = {name for name in roles if name.startswith('conpty_helper_')}
    handed_off = result.get('conpty_helpers')
    if helper_names and (not isinstance(handed_off, dict) or set(handed_off) != helper_names):
        raise ValueError('ConPTY helper handoff differs from runtime roles')
    if not helper_names and handed_off not in (None, {}):
        raise ValueError('unexpected ConPTY helper handoff')
    if set(roles) != set(declared) | {'launcher','collector'} | helper_names:
        raise ValueError('runtime roles differ from declared and owned processes')
    for name,pid in declared.items():
        role = _object(roles[name], 'role ' + name)
        if type(pid) is not int or type(role.get('pid')) is not int or role['pid'] != pid:
            raise ValueError('runtime PID differs from prelaunch declaration')
    for role in ('launcher','collector'):
        _object(roles[role], role)
        pid = result.get(role + '_pid')
        if type(pid) is not int or type(roles[role].get('pid')) is not int or roles[role]['pid'] != pid:
            raise ValueError('owned child PID differs from receipt')
    for name in sorted(helper_names):
        helper = _object((handed_off or {}).get(name), name)
        if helper != roles[name]:
            raise ValueError('ConPTY helper identity differs from receipt role')
    if roles['game_server'] != {k:server_identity.get(k) for k in ('pid','start_identity')}:
        raise ValueError('sampled server identity differs from server sidecar')
    modules = _object(sampler.get('modules'), 'sampler modules')
    backend = sampler.get('process_backend')
    expected = {'process_accounting.py','server_resources.py'}
    if backend == 'libproc':
        expected.add('native_process_sample.py')
    elif backend != 'system':
        raise ValueError('unsupported sampler backend')
    # system backend on Windows uses windows_process_sample via server_resources.
    # Require that pin from producer identity evidence (not reader platform).
    if backend == 'system' and _roles_require_windows_process_sample(roles):
        expected.add('windows_process_sample.py')
    if set(modules) != expected:
        raise ValueError('incomplete sampler dependency bindings')
    bindings = {}
    for name in sorted(expected):
        module = _object(modules[name], name)
        path, digest = _bound_file(module,'path','sha256')
        bindings[name] = {'path':str(path),'sha256':digest}
    main = bindings['process_accounting.py']
    if pathlib.Path(sampler.get('module','')).resolve() != pathlib.Path(main['path']) or sampler.get('module_sha256') != main['sha256']:
        raise ValueError('actual accounting script differs from module bindings')
    output, digest = _bound_file(receipt,'sampler_output_path','sampler_output_sha256')
    if not isinstance(result.get('output'),str) or pathlib.Path(result['output']).resolve() != output:
        raise ValueError('sampler result output differs from bound output')
    evidence = pe.validate(output,digest,roles=roles,observation=native['observation_wall_span'],
                           interval_s=sampler.get('interval_s'),duration_mode=sampler.get('duration_mode'),
                           process_backend=backend)
    if evidence.get('status') != 'available':
        raise ValueError('process coverage unavailable: ' + str(evidence.get('reason')))
    bp.recheck_files(bindings)
    evidence['module_bindings'] = bindings
    return evidence


def bind(receipt, native, server_identity):
    """Return independently checked accounting, or an explicit missing proof."""
    out = {'status':'unavailable','instrumentation_overhead_measured':False,'performance_acceptance':False}
    try:
        _object(receipt, 'receipt')
        _object(native, 'native qualification')
        _object(server_identity, 'server identity')
        if native.get('status') != 'available':
            raise ValueError('native qualification unavailable')
        out['cache'] = _cache(receipt,native)
        out['process'] = _process(receipt,native,server_identity)
        out['status'] = 'available'
        out['match_keys'] = {
            'cache_settings':out['cache']['match_keys'],
            'renderer_settings':{'by_ordinal':native['match_keys']['renderer_config_by_ordinal']},
            'sampler_backend':out['process']['process_backend'],
            'sampler_modules':{k:v['sha256'] for k,v in out['process']['module_bindings'].items()},
        }
    except (ValueError,TypeError,KeyError,OSError,OverflowError) as error:
        out['reason'] = str(error)
    return out
