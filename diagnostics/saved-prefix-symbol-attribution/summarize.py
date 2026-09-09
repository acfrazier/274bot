"""Summarize complete audited chains and frozen source identities, no live data."""
import hashlib
import json
import subprocess
from pathlib import Path

OUT = Path(__file__).resolve().parent
ROOT = OUT.parents[1]
mapping = json.loads((OUT/'mapping.json').read_text())
audit = json.loads((OUT/'audit.json').read_text())
pop = json.loads((OUT/'populations.json').read_text())


def chain(t):
    result = []
    while t:
        r = mapping['traces'][str(t)]
        ip = r['ip_id']
        result.append(dict(ip=ip, frames=mapping['ips'].get(str(ip), {}).get('frames', [])))
        t = r['parent_trace_id']
    return result


selected = {}
for cutoff in ('peak', 'last_mark', 'eof'):
    selected[cutoff] = []
    for g in audit['cutoffs'][cutoff]['groups'][:8]:
        rs = [r for r in pop[cutoff] if r['symbol_group'] == g['symbol']]
        # Output every member trace ID, and representative largest chains.
        entry = dict(symbol=g['symbol'], bytes=g['bytes'], count=g['count'], original_family_ids=sorted({r['family_id'] for r in rs}), trace_ids=g['traces'], largest=[dict(**r, chain=chain(r['trace_id'])) for r in rs[:3]])
        selected[cutoff].append(entry)
        if cutoff != 'eof':
            print(cutoff, g['symbol'], g['bytes'], 'families', len(entry['original_family_ids']), 'traces', len(rs))
            for r in entry['largest']:
                frames = [f['symbol'] or '<unresolved>' for c in r['chain'] for f in (c['frames'] or [dict(symbol='')])]
                print(r['trace_id'], r['bytes'], r['count'], ' <- '.join(frames[:7]))
print('mapping', {k:audit[k] for k in ('symbol_count','ip_count','trace_count','frames_with_source','still_mangled_symbols')})
for i in audit['still_mangled_symbols']:
    print('unchanged', next(r for r in mapping['symbols'] if r['id']==i))
(OUT/'leading-chains.json').write_text(json.dumps(selected, indent=2)+'\n')
host = 'c0709aba2f8b45e42193225cf8f4e7325b5ca9bf'
client = '3456edc8dabf7b25ada78110ffa56327af9f67a4'
files = [('.', host, p) for p in ['crates/nav/src/world.rs', 'crates/nav/src/pack.rs', 'crates/nav/src/collision.rs', 'crates/host-play/src/lib.rs', 'crates/api/src/snapshot.rs']]
files += [('vendor/fr-client-rust', client, p) for p in ['crates/client/src/config/if_type.rs', 'crates/client/src/sound/jagfx.rs', 'crates/client/src/dash3d/model.rs', 'crates/client/src/dash3d/anim_frame.rs', 'crates/client/src/client/client.rs']]
identities=[]
for repo, rev, p in files:
    data = subprocess.check_output(['git', '-C', str(ROOT/repo), 'show', f'{rev}:{p}'])
    current = (ROOT/repo/p).read_bytes()
    identities.append(dict(repo=repo, commit=rev, path=p, frozen_sha256=hashlib.sha256(data).hexdigest(), current_sha256=hashlib.sha256(current).hexdigest(), frozen_equals_current=data==current))
(OUT/'source-identities.json').write_text(json.dumps(identities, indent=2)+'\n')
print('frozen/current comparison', [(r['path'],r['frozen_equals_current']) for r in identities])
