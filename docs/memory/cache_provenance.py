"""Read-only fingerprints for the local client's cache inputs.

Run outside measurement windows. This records file evidence, not a claim that
all files were resident/read or that network-loaded assets were observed.
"""
import hashlib
import json
import pathlib

from build_provenance import file_sha256


JAGS = ('config', 'interface', 'textures', 'media', 'title', 'sounds', 'wordenc', 'versionlist')
SNAPSHOTS = ('models.bin', 'anims.bin')
STORE_FILES = ('main_file_cache.dat', *(f'main_file_cache.idx{i}' for i in range(1, 5)))


def verify_snapshot(value):
    """Recompute the complete scope; an omitted file cannot evade recheck."""
    if not isinstance(value, dict):
        raise ValueError('cache snapshot must be an object')
    actual = capture(value['cache_dir_requested'], value['unpack_root_requested'])
    if json.dumps(actual, sort_keys=True, allow_nan=False) != json.dumps(value, sort_keys=True, allow_nan=False):
        raise ValueError('cache snapshot differs from complete current fingerprint')
    return actual


def capture(cache_dir, unpack_root):
    """Capture explicit paths using the current client local-cache resolution.

    The eight jags and active version model/anim snapshots must exist. Record
    every present file store at cache_dir and its parent, including explicit
    absences, because cache_read may fall back from the former to the latter.
    Client version_hash is SHA256(versionlist)[:8 bytes].hex(), not 8 hex chars.
    """
    cache_requested = pathlib.Path(cache_dir).absolute()
    unpack_requested = pathlib.Path(unpack_root).absolute()
    cache_dir = cache_requested.resolve(strict=True)
    unpack_root = unpack_requested.resolve(strict=True)
    if not cache_dir.is_dir() or not unpack_root.is_dir():
        raise ValueError('cache and unpack paths must be directories')
    version = file_sha256(cache_dir / 'versionlist')[:16]
    snapshot_dir = unpack_root / version
    records = []

    def record(label, path, required):
        path = pathlib.Path(path)
        if not path.is_file():
            if required:
                raise ValueError(f'required cache input missing: {label}')
            records.append({'label': label, 'path': str(path.absolute()), 'present': False})
            return
        actual = path.resolve(strict=True)
        before = actual.stat()
        digest = file_sha256(actual)
        after = actual.stat()
        if (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns) != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns):
            raise ValueError(f'cache input changed while hashing: {label}')
        records.append({'label': label, 'path': str(path.absolute()), 'canonical_path': str(actual),
                        'present': True, 'size_bytes': after.st_size, 'sha256': digest})

    for name in JAGS:
        record('jag/' + name, cache_dir / name, True)
    for name in SNAPSHOTS:
        record('snapshot/' + name, snapshot_dir / name, True)
    for label, store_dir in (('direct', cache_dir), ('parent', cache_dir.parent)):
        for name in STORE_FILES:
            record('store/' + label + '/' + name, store_dir / name, False)
    if next(row['sha256'] for row in records if row['label'] == 'jag/versionlist')[:16] != version:
        raise ValueError('cache version changed while capturing')
    value = {'schema': 1, 'cache_dir': str(cache_dir), 'unpack_root': str(unpack_root),
             'cache_dir_requested': str(cache_requested), 'unpack_root_requested': str(unpack_requested),
             'snapshot_version': version, 'snapshot_dir': str(snapshot_dir), 'files': records,
             'scope': '8 jags, active version models/anims, direct and parent file-store inputs',
             'network_asset_contents': 'not captured by this file fingerprint',
             'performance_acceptance': False}
    value['content_identity_sha256'] = hashlib.sha256(json.dumps(
        [{k: row[k] for k in ('label', 'present', 'size_bytes', 'sha256') if k in row} for row in records],
        sort_keys=True, separators=(',', ':')).encode()).hexdigest()
    recheck(value)
    return value


def recheck(value):
    """Reject changed bytes, changed symlink targets and newly present inputs."""
    for key in ('cache_dir', 'unpack_root'):
        if str(pathlib.Path(value[key + '_requested']).resolve(strict=True)) != value[key]:
            raise ValueError('cache directory binding changed: ' + key)
    for row in value['files']:
        path = pathlib.Path(row['path'])
        if not row['present']:
            if path.exists():
                raise ValueError('previously absent cache input appeared: ' + row['label'])
        elif (not path.is_file() or str(path.resolve()) != row['canonical_path']
              or file_sha256(path) != row['sha256']):
            raise ValueError('cache input changed: ' + row['label'])
