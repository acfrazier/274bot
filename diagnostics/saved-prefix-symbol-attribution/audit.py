"""Audit ONLY exported saved-prefix tables; no trace replay or owner classifier.
Run from checkout: python3 diagnostics/saved-prefix-symbol-attribution/audit.py
Outputs are confined to this script's directory. Requires frozen Apple llvm-cxxfilt.
"""
import csv
import hashlib
import json
import re
import subprocess
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

OUT = Path(__file__).resolve().parent
ROOT = OUT.parents[1]
DATA = ROOT / 'diagnostics/root-native-production-a55972a/production-replay-1/result'
PROBE = ROOT / 'diagnostics/root-native-symbols-a55972a'
UNKNOWN = '<no-recognized-application-frame>'
APP = re.compile(r'^(?:nav|client|api|host_play|script|tui)::')


def useful(symbol):
    # The leading self type must itself be in the allowlist. Never search
    # inside allocator/dependency type arguments, references, tuples or slices.
    return bool(APP.match(symbol) or (symbol.startswith('<') and APP.match(symbol[1:])))


def select(symbols):
    return next((s for s in symbols if useful(s)), UNKNOWN)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def rows(name):
    p = DATA / name
    assert p.stat().st_size < 2_000_000, name
    with p.open(newline='') as f:
        return list(csv.DictReader(f, delimiter='\t', quoting=csv.QUOTE_NONE))


def save(name, obj):
    if name in ('mapping.json', 'populations.json'):
        # Keep each table record on one readable line, rather than expanding
        # repeated frame/coverage keys into tens of thousands of lines.
        sections = []
        for key, value in sorted(obj.items()):
            if isinstance(value, list):
                rendered = '[\n' + ',\n'.join('    '+json.dumps(r, sort_keys=True) for r in value) + '\n  ]'
            elif key in ('ips', 'traces'):
                rendered = '{\n' + ',\n'.join('    '+json.dumps(str(k))+': '+json.dumps(v, sort_keys=True) for k,v in sorted(value.items())) + '\n  }'
            else:
                rendered = json.dumps(value, sort_keys=True)
            sections.append('  '+json.dumps(key)+': '+rendered)
        text = '{\n' + ',\n'.join(sections) + '\n}\n'
    else:
        text = json.dumps(obj, indent=2, sort_keys=True) + '\n'
    assert len(text.encode()) < 8_000_000, name
    (OUT / name).write_text(text)


def run():
    inputs = sorted(DATA.glob('*.tsv')) + [DATA / 'receipt.json'] + sorted(PROBE.glob('*.json'))
    before = {str(p.relative_to(ROOT)): sha(p) for p in inputs}
    strings = {}
    for r in rows('strings.tsv'):
        i, s = int(r['string_id']), json.loads(r['text_json'])
        assert i not in strings and len(s.encode()) == int(r['utf8_bytes'])
        strings[i] = s
    ips = {}
    for r in rows('ips.tsv'):
        i = int(r['ip_id'])
        assert i not in ips
        fields = [int(x, 16) for x in json.loads(r['fields_hex_json'])]
        assert len(fields) in (2, 3) or (len(fields) >= 5 and (len(fields)-5) % 3 == 0)
        ips[i] = fields
    traces = {}
    for r in rows('traces.tsv'):
        i = int(r['trace_id'])
        assert i not in traces
        traces[i] = (int(r['ip_id']), int(r['parent_trace_id']))
    symbol_ids = sorted({f[j] for f in ips.values() for j in range(2, len(f), 3) if f[j]})
    reference = json.loads((PROBE / 'symbols.json').read_text())
    exe = Path(reference['executable'])
    assert sha(exe) == reference['sha256']
    argv = [str(exe), '--no-strip-underscore']
    assert reference['argv'] == argv
    raw = [strings[i] for i in symbol_ids]
    assert all('\n' not in s for s in raw)
    proc = subprocess.run(argv, input='\n'.join(raw)+'\n', text=True, capture_output=True, check=True, timeout=15)
    decoded = proc.stdout.splitlines()
    assert len(decoded) == len(raw) and not proc.stderr
    symbols = dict(zip(symbol_ids, decoded))
    refs = {r['id']: r for r in reference['symbols']}
    assert set(refs) == set(symbol_ids)
    for i in symbol_ids:
        assert refs[i] == dict(id=i, raw=strings[i], demangled=symbols[i])
    frame_table = {}
    for i, fields in ips.items():
        frame_table[i] = []
        for j in range(2, len(fields), 3):
            sid = fields[j]
            fid = fields[j+1] if j+1 < len(fields) else 0
            line = fields[j+2] if j+2 < len(fields) else 0
            frame_table[i].append(dict(symbol_id=sid, symbol=symbols.get(sid, ''), file=strings[fid], line=line))
    chains = {}
    for trace in traces:
        seen, chain = set(), []
        t = trace
        while t:
            assert t in traces and t not in seen
            seen.add(t)
            ip, t = traces[t]
            assert ip == 0 or ip in ips
            chain.append(ip)
        chains[trace] = chain
    receipt = json.loads((DATA / 'receipt.json').read_text())
    for name, identity in receipt['files'].items():
        p = DATA / name
        assert p.stat().st_size == identity['bytes'] and sha(p) == identity['sha256']
    family_metadata = {r['family_id']: json.loads(r['provenance_json']) for r in rows('families.tsv')}
    assert all(r['ownership'] == 'unknown' for r in family_metadata.values())
    summaries, populations = {}, {}
    for cutoff in ('peak', 'last_mark', 'eof'):
        stack_rows = rows(cutoff+'-stacks.tsv')
        desc = rows(cutoff+'-descriptors.tsv')
        family_rows = rows(cutoff+'-families.tsv')
        by_trace = defaultdict(lambda: [0, 0])
        dids = set()
        for r in desc:
            d, t, size, count, calls, cost = (int(r[k]) for k in ('descriptor_id', 'trace_id', 'size', 'live_count', 'allocation_calls', 'requested_bytes'))
            assert d not in dids and count > 0 and calls >= count and size * count == cost
            dids.add(d)
            by_trace[t][0] += cost
            by_trace[t][1] += count
        original = {}
        groups: Any = defaultdict(lambda: dict(bytes=0, count=0, traces=[]))
        families = defaultdict(lambda: [0, 0])
        coverage = Counter()
        details = []
        for r in stack_rows:
            t, b, c = (int(r[k]) for k in ('trace_id', 'requested_bytes', 'live_count'))
            assert t not in original and by_trace[t] == [b, c]
            original[t] = [b, c]
            families[r['family_id']][0] += b
            families[r['family_id']][1] += c
            chain = chains[t]
            frames = [f for ip in chain for f in frame_table.get(ip, [])]
            name = select(f['symbol'] for f in frames)
            flags = dict(
                no_application=name == UNKNOWN,
                unresolved_any=not chain or any(not frame_table.get(ip) or any(not f['symbol'] for f in frame_table[ip]) for ip in chain),
                missing_ip=any(ip == 0 for ip in chain),
                source_absent_any=not frames or any(not f['file'] or not f['line'] for f in frames),
                still_mangled_any=any(f['symbol'].startswith(('_R', '_ZN')) for f in frames),
                incidental_application_generic=any(not useful(f['symbol']) and re.search(r'(?:nav|client|api|host_play|script|tui)::', f['symbol']) for f in frames),
            )
            for flag, present in flags.items():
                if present:
                    coverage[flag+'_bytes'] += b
                    coverage[flag+'_stacks'] += 1
            groups[name]['bytes'] += b
            groups[name]['count'] += c
            groups[name]['traces'].append(t)
            details.append(dict(trace_id=t, family_id=r['family_id'], bytes=b, count=c, symbol_group=name, flags=flags))
        assert dict(by_trace) == original
        declared = {r['family_id']: [int(r['requested_bytes']), int(r['live_count'])] for r in family_rows}
        assert len(declared) == len(family_rows) and declared == dict(families)
        assert set(declared) <= set(family_metadata)
        total = [sum(v[i] for v in original.values()) for i in (0, 1)]
        assert total == [receipt['cutoffs'][cutoff][k] for k in ('bytes', 'count')]
        assert [sum(g[k] for g in groups.values()) for k in ('bytes', 'count')] == total
        root_groups = json.loads((PROBE / (cutoff+'-symbol-groups.json')).read_text())
        root_traces = {}
        for g in root_groups:
            assert sum(int(r['requested_bytes']) for r in g['traces']) == g['requested_bytes']
            assert sum(int(r['live_count']) for r in g['traces']) == g['count']
            for r in g['traces']:
                t = int(r['trace_id'])
                assert t not in root_traces
                root_traces[t] = g['symbol']
                source_row = next(s for s in stack_rows if int(s['trace_id']) == t)
                assert all(r[k] == v for k, v in source_row.items())
                actual = [dict(ip=ip, symbol_id=f['symbol_id'], symbol=f['symbol'], file=f['file'], line=f['line']) for ip in chains[t] for f in frame_table.get(ip, []) if useful(f['symbol'])]
                assert r['application_frames'] == actual
        assert root_traces == {r['trace_id']: r['symbol_group'] for r in details}
        expected = {g['symbol']: [g['requested_bytes'], g['count']] for g in root_groups}
        assert len(expected) == len(root_groups) and expected == {k: [v['bytes'], v['count']] for k, v in groups.items()}
        root_summary = json.loads((PROBE / 'summary.json').read_text())['cutoffs'][cutoff]
        assert {g['symbol']: [g['bytes'], g['count']] for g in root_summary} == expected
        summaries[cutoff] = dict(bytes=total[0], count=total[1], descriptor_rows=len(desc), stack_rows=len(stack_rows), original_families=len(families), symbol_groups=len(groups), coverage=dict(coverage), groups=[dict(symbol=k, **v) for k, v in sorted(groups.items(), key=lambda kv: (-kv[1]['bytes'], kv[0]))])
        populations[cutoff] = details
    assert before == {str(p.relative_to(ROOT)): sha(p) for p in inputs}
    save('mapping.json', dict(demangler=dict(argv=argv, sha256=sha(exe), version=subprocess.check_output([str(exe), '--version'], text=True, timeout=15)), symbols=[dict(id=i, raw=strings[i], demangled=symbols[i]) for i in symbol_ids], ips={i: dict(fields_hex=[format(x,'x') for x in f], frames=frame_table[i]) for i,f in ips.items()}, traces={i: dict(ip_id=v[0], parent_trace_id=v[1]) for i,v in traces.items()}))
    save('populations.json', populations)
    save('audit.json', dict(status='supplemental_symbol_audit_only', acceptance=False, inputs_sha256=before, symbol_count=len(symbol_ids), still_mangled_symbols=[i for i in symbol_ids if symbols[i].startswith(('_R','_ZN'))], ip_count=len(ips), trace_count=len(traces), frames_with_source=sum(bool(f['file'] and f['line']) for fs in frame_table.values() for f in fs), root_exact_mapping_and_populations_equal=True, canonical_replayed=False, cutoffs=summaries, read_buffer_trace_3862={k: next((r for r in v if r['trace_id']==3862), None) for k,v in populations.items()}))
    for k, s in summaries.items():
        print(k, {x: v for x,v in s.items() if x != 'groups'})
        print('leading groups', [(g['symbol'], g['bytes'], g['count']) for g in s['groups'][:8]])
    print('PASS: all exported populations, root symbol strings/frames/groups, immutable input hashes')


if __name__ == '__main__':
    run()
