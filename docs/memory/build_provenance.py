"""Verify an explicit frozen build and its runtime fixtures before launch.

Manifest claims are recorded as build evidence, never current-checkout evidence.
This module does not establish workload, overhead or performance acceptance.
"""
import hashlib
import json
import pathlib
import re


def file_sha256(path):
    digest = hashlib.sha256()
    with pathlib.Path(path).open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(chunk)
    return digest.hexdigest()


def _digest(value, label, commit=False):
    pattern = r'(?:[0-9a-f]{40}|[0-9a-f]{64})' if commit else r'[0-9a-f]{64}'
    if not isinstance(value, str) or re.fullmatch(pattern, value) is None:
        raise ValueError(f'invalid {label}')
    return value


def _object(value, label):
    if not isinstance(value, dict) or not value:
        raise ValueError(f'missing {label}')
    return value


def verify_build(manifest_path, role, frontend, binary, nav_pack, nav_flags, catalog_path):
    """Return verified bindings; missing or inconsistent evidence raises.

    Runtime fixture paths must equal the explicit paths in the manifest. Symlinks
    to the same canonical path are allowed; same-hash copies are not substituted.
    Returned file bindings can be rechecked at completion with recheck_files.
    """
    if role not in ('reference', 'candidate') or frontend not in ('tui', 'panel'):
        raise ValueError('invalid build role or frontend')
    manifest_path = pathlib.Path(manifest_path).resolve(strict=True)
    manifest_bytes = manifest_path.read_bytes()
    manifest = _object(json.loads(manifest_bytes), 'manifest')
    side_name = 'control' if role == 'reference' else 'candidate'
    side = _object(manifest.get(side_name), 'manifest side')
    pre = _digest(side.get('sources_sha256_pre'), 'source pre digest')
    post = _digest(side.get('sources_sha256_post'), 'source post digest')
    if pre != post or side.get('sources_stable_across_build') is not True:
        raise ValueError('source evidence is not stable across build')
    commit = _digest(side.get('commit', side.get('commit_at_freeze_snap')), 'build commit', True)
    if not isinstance(side.get('branch'), str) or not side['branch']:
        raise ValueError('missing build branch')
    if type(side.get('build_exit')) is not int or side['build_exit'] != 0:
        raise ValueError('build did not complete successfully')
    client = _object(side.get('client'), 'client')
    client_commit = _digest(client.get('commit'), 'client commit', True)
    client_sources = _digest(client.get('sources_sha256'), 'client sources')
    features = _object(manifest.get('features'), 'features')
    requested = features.get('requested')
    if not isinstance(requested, str) or not requested:
        raise ValueError('missing requested build features')
    if features.get('locked') is not True or type(features.get('allocation_counting')) is not bool:
        raise ValueError('invalid feature or allocator evidence')
    allocator = features.get('allocator')
    if not isinstance(allocator, str) or not allocator:
        raise ValueError('missing allocator evidence')
    key = f'{side_name}_{frontend}_play'
    entry = _object(_object(manifest.get('binaries'), 'binaries').get(key), key)
    nav = _object(manifest.get('nav'), 'nav fixtures')
    catalog = _object(manifest.get('catalog'), 'catalog')
    bindings = {'manifest': {'path': str(manifest_path), 'sha256': hashlib.sha256(manifest_bytes).hexdigest()}}

    def bind(label, recorded_path, recorded_sha, actual_path=None):
        if not isinstance(recorded_path, str) or not recorded_path:
            raise ValueError(f'missing {label} path')
        recorded = pathlib.Path(recorded_path).resolve(strict=True)
        actual = pathlib.Path(actual_path).resolve(strict=True) if actual_path is not None else recorded
        if actual != recorded:
            raise ValueError(f'{label} path differs from manifest')
        expected = _digest(recorded_sha, f'{label} digest')
        if file_sha256(actual) != expected:
            raise ValueError(f'{label} hash differs from manifest')
        bindings[label] = {'path': str(actual), 'sha256': expected}

    bind('binary', entry.get('path'), entry.get('sha256'), binary)
    bind('nav_pack', nav.get('nav_pack'), nav.get('nav_pack_sha256'), nav_pack)
    bind('nav_flags', nav.get('nav_flags'), nav.get('nav_flags_sha256'), nav_flags)
    bind('catalog', catalog.get('js_scripts_json'), catalog.get('js_scripts_json_sha256'), catalog_path)
    recheck_files(bindings)
    return {
        'status': 'verified', 'role': role, 'manifest_binary_key': key,
        'manifest_path': str(manifest_path), 'manifest_sha256': bindings['manifest']['sha256'],
        'build_commit': commit, 'build_branch': side['branch'],
        'host_sources_sha256': pre, 'client_commit': client_commit,
        'client_sources_sha256': client_sources,
        'feature_flags': {k: features[k] for k in ('requested', 'locked', 'allocation_counting')},
        'allocator_provenance': allocator, 'allocation_counting': features['allocation_counting'],
        'files': bindings, 'performance_acceptance': False,
    }


def recheck_files(bindings):
    for label, entry in bindings.items():
        if file_sha256(entry['path']) != entry['sha256']:
            raise ValueError(f'{label} changed during evidence collection')
