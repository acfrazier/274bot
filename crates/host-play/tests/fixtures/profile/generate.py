"""Synthetic shared-profile fixture; no downloaded game/cache content."""
import bz2
import hashlib
import json
import struct
from pathlib import Path

ROOT = Path(__file__).parent

def jag(entries):
    table = struct.pack('>H', len(entries))
    bodies = b''
    for name, body in entries.items():
        key = 0
        for c in name.upper():
            key = (key * 61 + ord(c) - 32) & 0xffffffff
        table += struct.pack('>I', key) + len(body).to_bytes(3, 'big') * 2
        bodies += body
    raw = table + bodies
    packed = bz2.compress(raw, compresslevel=1)[4:]
    assert len(raw) != len(packed)
    return len(raw).to_bytes(3, 'big') + len(packed).to_bytes(3, 'big') + packed

entries = {}
for kind in ['obj', 'npc', 'loc']:
    record = b'\x02Fixture ' + kind.encode() + b'\x0a\x00'
    entries[kind + '.dat'] = b'\x00\x01' + record
    entries[kind + '.idx'] = b'\x00\x01' + len(record).to_bytes(2, 'big')
config = jag(entries)
# One 1x1 layer, no script predicates/scripts/children, scroll height 1.
interface_data = bytes.fromhex('0001 0000 00 00 0000 0001 0001 00 00 00 00 0001 00 0000')
interface = jag({'data': interface_data})
versionlist = jag({name + suffix: b'' for name in ['model', 'anim', 'midi', 'map'] for suffix in ['_version', '_crc']})
empty = jag({})
for name in ['title', 'config', 'interface', 'media', 'versionlist', 'textures', 'wordenc', 'sounds']:
    data = {'config': config, 'interface': interface, 'versionlist': versionlist}.get(name, empty)
    (ROOT/name).write_bytes(data)
for rev in [274, 289]:
    manifest = {'revision': rev, 'archives': {name: hashlib.sha256((ROOT/name).read_bytes()).hexdigest() for name in sorted(['title','config','interface','media','versionlist','textures','wordenc','sounds'])}}
    (ROOT/f'manifest-{rev}.json').write_text(json.dumps(manifest, indent=2) + '\n')
