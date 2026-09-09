#!/usr/bin/env python3
"""Strict composite source binding for the coordinate-refined nav candidate."""
import hashlib
import json
from pathlib import Path
import stat

SCHEMA = 'nav-coordinate-source-v1'
CANDIDATE = 'coordinate-ebf0f30'
DENSE = '29b7aea779322c8611f83dc193e939ca7d756f75'
REFINED_BASE = '8385babb23fd15b876506d4a3f6154984a6b2df1'
CLIENT = '3456edc8dabf7b25ada78110ffa56327af9f67a4'
POLICY = 'complete-base-plus-exact-overlays-v1'
OVERLAY = {
    'arm': 'refined',
    'path': 'crates/nav/src/collision.rs',
    'source_commit': 'ebf0f30f0422229ea75de39db06944091d6497a5',
    'old_git_blob': '7446a044e12cddf4bd2d8ff0f7093cbaea4d7af1',
    'old_sha256': '688713da89546ab61755f6cdb95714f466ff270657b5a936e53a5d2e4702b8ef',
    'old_bytes': 65420,
    'new_git_blob': '07e0b215aa1fd21df87f308f0a889aa4ce8bb4a7',
    'new_sha256': '760ae7c88c35fac6183f2db51af1f69af8d5e3225110889bc84a5a4944720b0b',
    'new_bytes': 65821,
}
TOP_FIELDS = {'schema','candidate_id','dense_base','refined_base','client',
              'effective_tree_policy','overlays'}
OVERLAY_FIELDS = set(OVERLAY)


def digest_bytes(data):
    return hashlib.sha256(data).hexdigest()


def git_blob(data):
    return hashlib.sha1(b'blob '+str(len(data)).encode()+b'\0'+data).hexdigest()


def validate(value):
    if not isinstance(value,dict) or set(value)!=TOP_FIELDS:
        raise ValueError('source binding fields/schema mismatch')
    expected=(SCHEMA,CANDIDATE,DENSE,REFINED_BASE,CLIENT,POLICY)
    actual=tuple(value[k] for k in ('schema','candidate_id','dense_base','refined_base','client','effective_tree_policy'))
    if actual!=expected:
        raise ValueError('source binding identity mismatch')
    overlays=value['overlays']
    if not isinstance(overlays,list) or len(overlays)!=1 or not isinstance(overlays[0],dict):
        raise ValueError('exactly one collision overlay required')
    if set(overlays[0])!=OVERLAY_FIELDS or overlays[0]!=OVERLAY:
        raise ValueError('collision overlay identity mismatch')
    return value


def load(path,expected_sha256):
    path=Path(path)
    if not isinstance(expected_sha256,str) or len(expected_sha256)!=64 or any(c not in '0123456789abcdef' for c in expected_sha256):
        raise ValueError('external source-binding SHA256 required')
    st=path.lstat()
    if not stat.S_ISREG(st.st_mode) or st.st_size>16384 or path.is_symlink():
        raise ValueError('source binding must be a bounded regular file')
    data=path.read_bytes()
    if digest_bytes(data)!=expected_sha256:
        raise ValueError('source-binding SHA256 mismatch')
    return validate(json.loads(data))


def reference(path,expected_sha256,binding):
    validate(binding)
    return dict(schema=SCHEMA,candidate_id=CANDIDATE,
        filename=Path(path).name,sha256=expected_sha256)


def arms(binding=None):
    if binding is None:return [('dense',DENSE),('tiled',REFINED_BASE)]
    validate(binding)
    return [('dense',DENSE),('refined',REFINED_BASE)]


def apply(dest,entries,binding,arm,git_fetch):
    validate(binding);dest=Path(dest)
    by_path={e['path']:e for e in entries}
    if len(by_path)!=len(entries):raise ValueError('duplicate source path')
    overlay=OVERLAY if arm=='refined' else None
    if arm not in ('dense','refined'):raise ValueError('unknown bound arm')
    if overlay:
        entry=by_path.get(overlay['path'])
        if not entry or (entry.get('git_blob'),entry.get('sha256'),entry.get('size'))!=(overlay['old_git_blob'],overlay['old_sha256'],overlay['old_bytes']):
            raise ValueError('overlay base blob mismatch')
        path=dest/overlay['path']
        old=path.read_bytes()
        if (git_blob(old),digest_bytes(old),len(old))!=(overlay['old_git_blob'],overlay['old_sha256'],overlay['old_bytes']):
            raise ValueError('overlay base bytes moved')
        new=git_fetch(overlay['new_git_blob'])
        if (git_blob(new),digest_bytes(new),len(new))!=(overlay['new_git_blob'],overlay['new_sha256'],overlay['new_bytes']):
            raise ValueError('overlay Git object mismatch')
        path.write_bytes(new)
    return effective_entries(dest,entries,binding,arm)


def effective_entries(dest,entries,binding,arm):
    validate(binding);dest=Path(dest);out=[]
    for entry in entries:
        path=dest/entry['path'];data=path.read_bytes()
        overlay=OVERLAY if arm=='refined' and entry['path']==OVERLAY['path'] else None
        expected=(overlay['new_git_blob'],overlay['new_sha256'],overlay['new_bytes']) if overlay else (entry['git_blob'],entry['sha256'],entry['size'])
        if (git_blob(data),digest_bytes(data),len(data))!=expected:
            raise ValueError('effective source mismatch: '+entry['path'])
        out.append(dict(path=entry['path'],git_blob=expected[0],sha256=expected[1],size=expected[2],
                        provenance='overlay' if overlay else 'base'))
    return out


def verify_effective(dest,entries,binding,arm,allowed_additions=()):
    dest=Path(dest);result=effective_entries(dest,entries,binding,arm)
    admitted={e['path'] for e in entries}|set(allowed_additions)
    roots=('crates/nav/','crates/api/','vendor/fr-client-rust/crates/client/')
    for path in dest.rglob('*'):
        if path.is_symlink():raise ValueError('symlink in effective source')
        if path.is_file():
            rel=str(path.relative_to(dest))
            if rel.startswith(roots) and rel not in admitted:
                raise ValueError('unadmitted effective source: '+rel)
    return result
