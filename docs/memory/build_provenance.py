"""Verify an explicit frozen build and its runtime fixtures before launch.

Manifest claims are recorded as build evidence, never current-checkout evidence.
This module does not establish workload, overhead or performance acceptance.
"""
import hashlib
import json
import os
import pathlib
import re
import subprocess


DIRECT_FEATURES = 'memory-profile-no-alloc,memory-owner-capture'
DIRECT_ARCHIVE_SHA256 = '2c36d254c37c5ed56a55f78728357296d62be602f8653ed9ec041cd37d099d5d'
DIRECT_SOURCE_MANIFEST_SHA256 = 'ca69e70352c5f6f8d1c9db026959af6ba8fd64886be533a0095c3ca94bb5732e'
DIRECT_ORIGINAL_HOST = 'c0709aba2f8b45e42193225cf8f4e7325b5ca9bf'
DIRECT_ORIGINAL_CLIENT = '3456edc8dabf7b25ada78110ffa56327af9f67a4'
DIRECT_REVIEWED_HOST = '3cdc3e4fbeb960fe4c270eb994ce22746787eb2d'
DIRECT_REVIEWED_CLIENT = '5c73a4a27f3d72834c2c2a071668eb197eb39fd9'
DIRECT_DIAGNOSTIC_HOST = 'dcdbeebf36665a1d07156402a5f64c823769e00d'
DIRECT_DIAGNOSTIC_CLIENT = 'b74dfb3c998b10371b263189774b055ecf880380'
DIRECT_HOST_SOURCE_DIGEST = 'b70608d10f203657cbe731c6fd5402d09c96c1053466ce6540a7ab0dacc62564'
DIRECT_CLIENT_SOURCE_DIGEST = '169ba594a834fb5ea81f392ae77909251cc46889efa2324525fff71794a7ea30'
DIRECT_BINARY_SHA256 = '392dbecc7a86f2fcd7e3f6b515aedceb3f3de3b1d4e140b95463bf20438c0c95'
DIRECT_BINARY_BYTES = 95595424
DIRECT_SOURCE_MEMBERS = 1124


def file_sha256(path):
    digest = hashlib.sha256()
    with pathlib.Path(path).open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(chunk)
    return digest.hexdigest()


def source_digest(directory):
    """Digest tracked/unignored Cargo sources using the launcher algorithm."""
    root = pathlib.Path(directory)
    files = subprocess.check_output(
        ['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z'],
        cwd=root,
    ).split(b'\0')
    digest = hashlib.sha256()
    for name in sorted(set(files)):
        if not name:
            continue
        relative = pathlib.Path(os.fsdecode(name))
        if not (relative.parts[0] == 'crates' or relative.name in ('Cargo.toml', 'Cargo.lock')):
            continue
        path = root / relative
        if path.is_file():
            digest.update(name + b'\0' + path.read_bytes() + b'\0')
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


def verify_direct_owner_build(manifest_path, role, frontend, binary, nav_pack, nav_flags,
                              catalog_path):
    """Wrap ordinary verification with the exact direct-owner build lineage."""
    result = verify_build(
        manifest_path, role, frontend, binary, nav_pack, nav_flags, catalog_path
    )
    if role != 'candidate' or frontend != 'tui':
        raise ValueError('direct owner build requires candidate TUI')
    manifest_path = pathlib.Path(manifest_path).resolve(strict=True)
    manifest = _object(json.loads(manifest_path.read_bytes()), 'manifest')
    candidate = _object(manifest.get('candidate'), 'candidate')
    client = _object(candidate.get('client'), 'candidate client')
    features = _object(manifest.get('features'), 'features')
    if (
        candidate.get('commit') != DIRECT_DIAGNOSTIC_HOST
        or candidate.get('branch') != 'codex/direct-owner-host-archive'
        or candidate.get('sources_sha256_pre') != DIRECT_HOST_SOURCE_DIGEST
        or candidate.get('sources_sha256_post') != DIRECT_HOST_SOURCE_DIGEST
        or candidate.get('sources_stable_across_build') is not True
        or type(candidate.get('build_exit')) is not int
        or candidate.get('build_exit') != 0
        or client.get('commit') != DIRECT_DIAGNOSTIC_CLIENT
        or client.get('sources_sha256') != DIRECT_CLIENT_SOURCE_DIGEST
    ):
        raise ValueError('direct owner diagnostic source/build identity mismatch')
    if (
        features.get('requested') != DIRECT_FEATURES
        or features.get('locked') is not True
        or features.get('allocation_counting') is not False
        or features.get('allocator') != 'std::alloc::System'
        or features.get('snapshot_dedup') is not False
    ):
        raise ValueError('direct owner feature contract mismatch')
    if file_sha256(binary) != DIRECT_BINARY_SHA256:
        raise ValueError('direct owner runtime binary is not the fresh bound build')

    lineage = _object(candidate.get('source_lineage'), 'candidate.source_lineage')
    exact = {
        'source_member_count': DIRECT_SOURCE_MEMBERS,
        'original_host_commit': DIRECT_ORIGINAL_HOST,
        'original_client_commit': DIRECT_ORIGINAL_CLIENT,
        'reviewed_host_commit': DIRECT_REVIEWED_HOST,
        'reviewed_client_commit': DIRECT_REVIEWED_CLIENT,
        'diagnostic_host_commit': DIRECT_DIAGNOSTIC_HOST,
        'diagnostic_client_commit': DIRECT_DIAGNOSTIC_CLIENT,
    }
    for key, expected in exact.items():
        if lineage.get(key) != expected:
            raise ValueError(f'direct owner lineage mismatch: {key}')
    lineage_files = {}
    for label in ('archive', 'source_manifest', 'materialization', 'build_result'):
        entry = _object(lineage.get(label), f'lineage {label}')
        path_value = entry.get('path')
        if not isinstance(path_value, str) or not path_value:
            raise ValueError(f'missing lineage {label} path')
        path = pathlib.Path(path_value).resolve(strict=True)
        expected = _digest(entry.get('sha256'), f'lineage {label} digest')
        if file_sha256(path) != expected:
            raise ValueError(f'lineage {label} hash differs from manifest')
        lineage_files[label] = {'path': str(path), 'sha256': expected}
    if lineage_files['archive']['sha256'] != DIRECT_ARCHIVE_SHA256:
        raise ValueError('direct owner archive identity mismatch')
    if lineage_files['source_manifest']['sha256'] != DIRECT_SOURCE_MANIFEST_SHA256:
        raise ValueError('direct owner source manifest identity mismatch')

    materialization = _object(
        json.loads(pathlib.Path(lineage_files['materialization']['path']).read_bytes()),
        'materialization receipt',
    )
    provenance = _object(materialization.get('source_provenance'), 'source provenance')
    if (
        materialization.get('schema') != 'root-clean-owner-source-materialization-v1'
        or materialization.get('source_member_count') != DIRECT_SOURCE_MEMBERS
        or materialization.get('checkout_host_commit') != DIRECT_DIAGNOSTIC_HOST
        or materialization.get('checkout_client_commit') != DIRECT_DIAGNOSTIC_CLIENT
        or materialization.get('host_clean') is not True
        or materialization.get('client_clean') is not True
        or _object(materialization.get('archive'), 'materialization archive').get('sha256') != DIRECT_ARCHIVE_SHA256
        or materialization.get('source_manifest_sha256') != DIRECT_SOURCE_MANIFEST_SHA256
        or provenance.get('host_original') != DIRECT_ORIGINAL_HOST
        or provenance.get('client_original') != DIRECT_ORIGINAL_CLIENT
        or provenance.get('host_reviewed') != DIRECT_REVIEWED_HOST
        or provenance.get('client_reviewed') != DIRECT_REVIEWED_CLIENT
    ):
        raise ValueError('materialization receipt contradicts direct owner lineage')

    build = _object(
        json.loads(pathlib.Path(lineage_files['build_result']['path']).read_bytes()),
        'fresh build result',
    )
    binary_record = _object(build.get('binary'), 'fresh build binary')
    step = _object(build.get('step'), 'fresh build step')
    command = step.get('command')
    if (
        build.get('built') is not True
        or binary_record.get('sha256') != DIRECT_BINARY_SHA256
        or binary_record.get('bytes') != DIRECT_BINARY_BYTES
        or type(step.get('exit')) is not int
        or step.get('exit') != 0
        or not isinstance(command, list)
        or '--offline' not in command
        or '--locked' not in command
        or DIRECT_FEATURES not in command
    ):
        raise ValueError('fresh direct owner build receipt mismatch')
    for phase in ('source_before', 'source_after'):
        source = _object(build.get(phase), f'fresh build {phase}')
        digests = _object(source.get('source_digests'), f'{phase} source digests')
        if (
            source.get('host_commit') != DIRECT_DIAGNOSTIC_HOST
            or source.get('client_commit') != DIRECT_DIAGNOSTIC_CLIENT
            or source.get('host_clean') is not True
            or source.get('client_clean') is not True
            or source.get('source_member_count') != DIRECT_SOURCE_MEMBERS
            or _object(source.get('archive'), f'{phase} archive').get('sha256') != DIRECT_ARCHIVE_SHA256
            or digests.get('host') != DIRECT_HOST_SOURCE_DIGEST
            or digests.get('client') != DIRECT_CLIENT_SOURCE_DIGEST
        ):
            raise ValueError(f'fresh build {phase} source mismatch')

    for label, binding in lineage_files.items():
        result['files']['source_lineage_' + label] = binding
    result['source_lineage_files'] = {
        label: result['files']['source_lineage_' + label] for label in lineage_files
    }
    recheck_files(result['files'])
    return result


def recheck_files(bindings):
    for label, entry in bindings.items():
        if file_sha256(entry['path']) != entry['sha256']:
            raise ValueError(f'{label} changed during evidence collection')
