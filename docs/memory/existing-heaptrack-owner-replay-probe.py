#!/usr/bin/env python3
"""Bounded source-contract probe ONLY for the already saved 2001 Python smoke.
Not a production parser, capture tool, or lifecycle/owner classifier.
Run from the campaign checkout; stdout is the audit receipt. No file writes.
"""
import collections
import hashlib
import json
from pathlib import Path

ROOT = Path('diagnostics/owner-capture-evidence-2015')
CAP = ROOT / 'heaptrack-seam-smoke-2001/capture'
manifest = json.loads((ROOT / 'owner-capture-evidence-2015-manifest.json').read_text())
files = {}
for name in ('alloc.raw', 'alloc.interpreted', 'peak-stacks.txt',
             'peak-analysis.log', 'interpret-stderr.txt'):
    p = CAP / name
    assert p.is_file() and not p.is_symlink() and p.stat().st_size < 2_000_000
    b = p.read_bytes()
    digest = hashlib.sha256(b).hexdigest()
    entry = next(e for e in manifest['files'] if e['archive_path'] == str(p.relative_to(ROOT)))
    assert entry['sha256'] == digest and entry['bytes'] == len(b)
    assert b.endswith(b'\n')
    files[name] = {'bytes': len(b), 'sha256': digest}
raw = (CAP / 'alloc.raw').read_bytes().splitlines()
lines = (CAP / 'alloc.interpreted').read_bytes().splitlines()
assert raw[0] == lines[0] == b'v 10500 3'
# Reconstruct the interpreter's pointer-to-(size, trace) conversion independently.
pointers = {}
raw_events = []
unknown_frees = reuse = raw_temporary = 0
last_ptr = None
seen_ptrs = set()
raw_modes = collections.Counter()
for line in raw:
    mode = line[:1]
    raw_modes[mode.decode()] += 1
    if mode == b'+':
        size, trace, ptr = (int(x, 16) for x in line[2:].split())
        assert ptr not in pointers, 'duplicate live pointer: interpretation would overwrite'
        reuse += ptr in seen_ptrs
        seen_ptrs.add(ptr)
        pointers[ptr] = (size, trace)
        raw_events.append(('+', size, trace))
        last_ptr = ptr
    elif mode == b'-':
        ptr = int(line[2:], 16)
        temporary = ptr == last_ptr
        last_ptr = None
        info = pointers.pop(ptr, None)
        if info is None:
            unknown_frees += 1
        else:
            raw_temporary += temporary
            raw_events.append(('-', *info))
    else:
        assert mode in (b'v', b'x', b'X', b'I', b'm', b't', b'c', b'R'), mode

strings = [b'']
ips = [[]]
traces = [(0, 0)]
infos = []
live = []
events = []
counts = collections.Counter()
current = peak = allocations = frees = temporary = 0
last_index = 0  # Exact canonical analyzer convention, including zero-index collision.
stamp = 0
peak_line = peak_stamp = 0
peak_live = None
marks = []
max_multiplicity = 0
for number, line in enumerate(lines, 1):
    mode = line[:1].decode()
    counts[mode] += 1
    if mode == 's':
        n, text = line[2:].split(b' ', 1)
        assert len(text) == int(n, 16)
        strings.append(text)
    elif mode == 'i':
        fields = [int(x, 16) for x in line[2:].split()]
        assert len(fields) in (2, 3) or (len(fields) >= 5 and (len(fields) - 2) % 3 == 0)
        assert fields[1] < len(strings)
        for i in range(2, len(fields), 3):
            assert fields[i] < len(strings)
            if i + 1 < len(fields):
                assert fields[i + 1] < len(strings)
        ips.append(fields)
    elif mode == 't':
        ip, parent = (int(x, 16) for x in line[2:].split())
        assert 0 < ip < len(ips) and 0 <= parent < len(traces)
        traces.append((ip, parent))
    elif mode == 'a':
        size, trace = (int(x, 16) for x in line[2:].split())
        assert 0 < trace < len(traces)
        infos.append((size, trace))
        live.append(0)
    elif mode in ('+', '-'):
        index = int(line[2:], 16)
        assert 0 <= index < len(infos)
        size, trace = infos[index]
        events.append((mode, size, trace))
        if mode == '+':
            live[index] += 1
            allocations += 1
            current += size
            last_index = index
            max_multiplicity = max(max_multiplicity, live[index])
            if current > peak:
                peak = current
                peak_line, peak_stamp = number, stamp
                peak_live = live.copy()
        else:
            assert live[index] > 0
            live[index] -= 1
            frees += 1
            current -= size
            temporary += last_index == index
            last_index = 0
        assert current >= 0
    elif mode == 'c':
        new = int(line[2:], 16)
        assert new >= stamp
        stamp = new
        marks.append({'line': number, 'timestamp_ms': stamp,
                      'live_bytes': current, 'live_count': sum(live)})
    else:
        assert mode in ('v', 'X', 'I', 'R', '', '#'), mode
assert raw_events == events
assert len(pointers) == sum(live)
assert sum(size for size, _ in pointers.values()) == current
assert sum(n * info[0] for n, info in zip(live, infos)) == current
assert allocations == 6388 and sum(live) == 34 and raw_temporary == 637 and temporary == 638
assert peak == 5730024

def family_bytes(population, symbol):
    result = 0
    for n, (size, trace) in zip(population, infos):
        found = False
        while trace:
            ip, trace = traces[trace]
            found |= any(strings[ips[ip][j]] == symbol for j in range(2, len(ips[ip]), 3))
        if found:
            result += n * size
    return result

known = family_bytes(peak_live, b'PyByteArray_Resize')
assert known == 2048 * 2049 == 4196352
costs = [int(line.rsplit(b' ', 1)[1]) for line in (CAP / 'peak-stacks.txt').read_bytes().splitlines()]
assert all(x >= 0 for x in costs) and sum(costs) == peak
assert b'total memory leaked: 407.23K' in (CAP / 'peak-analysis.log').read_bytes()
assert round(current / 1000, 2) == 407.23
chosen = max((m for m in marks if m['timestamp_ms'] <= 500), key=lambda m: m['line'])
print(json.dumps({
    'scope': 'saved owned Python fixture only; not production replay',
    'files': files, 'raw_modes': raw_modes, 'interpreted_modes': counts,
    'raw_interpreted_event_sequences_equal': True,
    'unknown_raw_frees_discarded_by_interpreter': unknown_frees,
    'pointer_reuse_after_free': reuse, 'max_live_multiplicity_of_one_info_id': max_multiplicity,
    'allocations': allocations, 'frees': frees, 'eof_live_count': sum(live),
    'eof_unsuppressed_live_bytes': current, 'interpreter_temporary': raw_temporary,
    'canonical_analyzer_temporary': temporary,
    'global_peak_bytes': peak, 'global_peak_line': peak_line,
    'global_peak_previous_timestamp_ms': peak_stamp, 'known_bytearray_peak_bytes': known,
    'flamegraph_rows': len(costs), 'positive_rows': sum(x > 0 for x in costs),
    'first_timestamp': marks[0], 'last_timestamp': marks[-1],
    'timestamp_mark_at_or_before_500ms': chosen,
    'bytearray_eof_bytes': family_bytes(live, b'PyByteArray_Resize'),
    'last_record_modes': [line[:1].decode() for line in lines[-4:]],
    'checks_passed': True,
}, indent=2))
