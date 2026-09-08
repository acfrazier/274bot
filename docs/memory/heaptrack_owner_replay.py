#!/usr/bin/env python3
"""Strict offline Heaptrack 1.5/format-3 recorded-prefix replay. Stdlib only.

The public runner is heaptrack_owner_runner.py. Importing this module has no
process side effects. It never invokes Heaptrack or opens a network connection.
"""
import re
import hashlib
import os
import stat
import struct
import json
import shutil
from typing import Any
from array import array
from collections import Counter
from itertools import chain

U64 = (1 << 64) - 1
I64 = (1 << 63) - 1
MIB = 1 << 20
PULSE = lambda: None  # owned runner installs its phase/resource guard


class Invalid(Exception):
    """Fail closed; callers must discard all ranking output."""


def require(condition, reason):
    if not condition:
        raise Invalid(reason)


def number(token):
    require(bool(re.fullmatch(rb'[0-9a-f]{1,16}', token)), 'invalid uint64 hex')
    return int(token, 16)


def checked(value):
    require(0 <= value <= I64, 'signed aggregate overflow/underflow')
    return value


def sized(payload):
    length, sep, rest = payload.partition(b' ')
    n = number(length)
    require(bool(sep) and len(rest) >= n, 'truncated sized string')
    text, tail = rest[:n], rest[n:]
    require(b'\x00' not in text, 'NUL string unsupported by canonical printer')
    try:
        text.decode('utf-8')
    except UnicodeDecodeError as exc:
        raise Invalid('invalid UTF-8 string') from exc
    return text, tail


def parse_record(line, raw=False, line_limit=MIB) -> tuple[str, Any]:
    require(len(line) <= line_limit, 'line cap')
    require(line.endswith(b'\n'), 'truncated record (missing newline)')
    line = line[:-1]
    if not line or line.startswith(b'#'):
        return '#', None
    require(len(line) == 1 or line[1:2] == b' ', 'record separator')
    op, payload = chr(line[0]), line[2:]
    if op == 'A':
        raise Invalid('attach unsupported')
    if op in ('s', 'x', 'm'):
        require((raw and op in ('x', 'm')) or (not raw and op == 's'), 'wrong stream opcode')
        text, tail = sized(payload)
        if op == 'm' and text != b'-':
            require(tail.startswith(b' '), 'missing module ranges')
            fields = [number(x) for x in tail[1:].split(b' ')]
            require(len(fields) >= 3 and len(fields) % 2 == 1, 'module range arity')
            for addr, size in zip(fields[1::2], fields[2::2]):
                require(fields[0] + addr + size <= U64, 'module address overflow')
            return op, (text, fields)
        require(not tail, 'extra string fields')
        return op, text
    if op in ('X', 'S'):
        require(len(line) >= 2 and b'\x00' not in payload, 'metadata shape')
        return op, payload  # X is hashed, never persisted as a command line.
    arities: dict[str, tuple[int, ...]] = {'v': (2,), 'I': (2,), 'R': (1,), 'c': (1,), 't': (2,),
               '+': (3,) if raw else (1,), '-': (1,)}
    if not raw:
        arities['a'] = (2,)
        arities['i'] = ()
    require(op in arities, 'unknown opcode')
    fields = [number(x) for x in payload.split(b' ')]
    if op == 'i':
        require(len(fields) in (2, 3) or (len(fields) >= 5 and (len(fields)-2) % 3 == 0), 'IP arity')
    else:
        require(len(fields) in arities[op], 'numeric arity')
    return op, fields


LIMITS = dict(table_bytes=256*MIB, traces=2_000_000, ips=500_000,
              descriptors=1_000_000, pointers=2_000_000, strings=200_000,
              string_bytes=64*MIB, line=MIB, depth=512, raw=3*1024*MIB,
              interpreted=1024*MIB, peak=64*MIB, output=32*MIB,
              scratch=64*MIB, cpu=180, wall=300, rss=512*MIB, address=768*MIB,
              admission_memory=768*MIB, admission_disk=1024*MIB,
              free_disk=512*MIB)


class Budget:
    def __init__(self, limits=None):
        self.limits = dict(LIMITS)
        for key, value in (limits or {}).items():
            require(key in LIMITS and type(value) is int and 0 < value <= LIMITS[key], 'invalid limit override')
            self.limits[key] = value
        self.used = self.high = 0

    def charge(self, n):
        self.used += n
        require(0 <= self.used <= self.limits['table_bytes'], 'table byte cap')
        self.high = max(self.high, self.used)

    def count(self, key, n):
        require(n <= self.limits[key], key + ' cap')


class Input:
    """Bounded regular-file stream; identity verified in the same pass, not a fourth pass."""
    def __init__(self, entry, cap, line_limit):
        require(set(entry) == {'path', 'bytes', 'sha256'}, 'input entry schema')
        require(type(entry['bytes']) is int and 0 < entry['bytes'] <= cap, 'input size cap')
        require(bool(re.fullmatch('[0-9a-f]{64}', entry['sha256'])), 'hash format')
        self.entry, self.line_limit = entry, line_limit
        self.digest = hashlib.sha256()
        self.lines = self.offset = 0

    def __iter__(self):
        fd = os.open(self.entry['path'], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        with os.fdopen(fd, 'rb') as stream:
            before = os.fstat(stream.fileno())
            require(stat.S_ISREG(before.st_mode) and before.st_size == self.entry['bytes'], 'input identity/size')
            while True:
                line = stream.readline(self.line_limit + 1)
                if not line:
                    break
                PULSE()
                require(len(line) <= self.line_limit, 'line cap')
                require(line.endswith(b'\n'), 'truncated record')
                self.digest.update(line)
                self.lines += 1
                start = self.offset
                self.offset += len(line)
                require(self.offset <= self.entry['bytes'], 'input grew')
                yield self.lines, start, line
            after = os.fstat(stream.fileno())
            require((before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) ==
                    (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns), 'input changed during pass')
        require(self.offset == self.entry['bytes'] and self.digest.hexdigest() == self.entry['sha256'], 'input hash mismatch')


class Tables:
    """Packed uint64 columns and one string arena; no expanded per-event stacks."""
    def __init__(self, budget):
        self.b = budget
        self.text = bytearray()
        self.strings = array('Q', [0, 0])  # offset,length pairs; ID 0 absent
        self.ips = array('Q')
        self.ip_offsets = array('Q', [0, 0])  # offset,length pairs
        self.traces = array('Q', [0, 0])
        self.desc = array('Q')
        self.live = array('Q')
        self.calls = array('Q')
        self.trace_calls = array('Q', [0])
        self.descriptor_keys = set()
        self.suppressions = []

    def add(self, key, column, values, n):
        self.b.count(key, n)
        self.b.charge(16*len(values))  # 2x packed bytes includes array growth
        column.extend(values)

    def string(self, index):
        off, length = self.strings[index*2:index*2+2]
        return bytes(self.text[off:off+length])

    def ip(self, index):
        off, length = self.ip_offsets[index*2:index*2+2]
        return self.ips[off:off+length]

    def definition(self, op, fields):
        if op == 's':
            self.b.count('string_bytes', len(self.text)+len(fields))
            self.add('strings', self.strings, [len(self.text), len(fields)], len(self.strings)//2)
            self.b.charge(2*len(fields))
            self.text.extend(fields)
        elif op == 'i':
            require(fields[1] < len(self.strings)//2, 'module reference')
            for index in range(2, len(fields), 3):
                require(fields[index] < len(self.strings)//2, 'function reference')
                if index+1 < len(fields):
                    require(fields[index+1] < len(self.strings)//2, 'file reference')
            self.add('ips', self.ip_offsets, [len(self.ips), len(fields)], len(self.ip_offsets)//2)
            self.b.charge(16*len(fields))
            self.ips.extend(fields)
        elif op == 't':
            require(fields[0] < len(self.ip_offsets)//2 and fields[1] < len(self.traces)//2, 'trace IP/parent reference')
            self.add('traces', self.traces, fields, len(self.traces)//2)
            self.b.charge(16)
            self.trace_calls.append(0)
        elif op == 'a':
            require(fields[1] < len(self.traces)//2, 'descriptor trace reference')
            key = tuple(fields)
            require(key not in self.descriptor_keys, 'duplicate size/trace descriptor')
            self.b.charge(192)
            self.descriptor_keys.add(key)
            self.add('descriptors', self.desc, fields, len(self.live)+1)
            self.b.charge(32)
            self.live.append(0)
            self.calls.append(0)
        elif op == 'S':
            self.b.charge(128+len(fields))
            self.suppressions.append(fields)


def emit_digest(digest, op, *values):
    digest.update(op.encode('ascii') + struct.pack('>'+'Q'*len(values), *values))


class Session:
    def __init__(self):
        self.seen = set()
        self.stamp = 0
        self.marks = 0
        self.metadata = hashlib.sha256()

    def accept(self, op, fields, line):
        if op == '#':
            return
        require('v' in self.seen or (line == 1 and op == 'v'), 'version must be first record')
        if op in ('v', 'X', 'I', 'x'):
            require(op not in self.seen, 'duplicate session metadata')
            self.seen.add(op)
        if op == 'v':
            require(fields == [0x10500, 3], 'unsupported version')
        elif op == 'c':
            require(fields[0] >= self.stamp, 'nonmonotonic timestamp')
            self.stamp = fields[0]
            self.marks += 1
        if op in ('X', 'I', 'R', 'S'):
            data = fields if isinstance(fields, bytes) else struct.pack('>'+'Q'*len(fields), *fields)
            self.metadata.update(op.encode()+struct.pack('>Q', len(data))+data)

    def finish(self, raw):
        require({'v', 'X', 'I'} <= self.seen and (not raw or 'x' in self.seen), 'missing direct-preload metadata')


def raw_pass(entry, budget):
    pointers = {}
    session = Session()
    events = hashlib.sha256()
    trace_hash = hashlib.sha256()
    traces = total = allocations = frees = unknown = temporary = last_ptr = high = 0
    stream = Input(entry, budget.limits['raw'], budget.limits['line'])
    for line, offset, data in stream:
        op, fields = parse_record(data, raw=True, line_limit=budget.limits['line'])
        session.accept(op, fields, line)
        if op == 't':
            require(fields[1] <= traces, 'raw parent reference')
            traces += 1
            budget.count('traces', traces)
            emit_digest(trace_hash, 't', *fields)
        elif op == '+':
            size, trace, ptr = fields
            require(ptr != 0 and ptr not in pointers, 'raw duplicate-live/null pointer')
            require(trace <= traces, 'raw trace reference')
            budget.count('pointers', len(pointers)+1)
            if len(pointers)+1 > high:
                budget.charge(384)  # retain peak map-capacity charge even after frees
            pointers[ptr] = (size, trace)
            high = max(high, len(pointers))
            total = checked(total+size)
            allocations = checked(allocations+1)
            last_ptr = ptr
            emit_digest(events, '+', size, trace)
        elif op == '-':
            ptr = fields[0]
            is_temp = ptr == last_ptr
            last_ptr = 0
            info = pointers.pop(ptr, None)
            if info is None:
                unknown = checked(unknown+1)
            else:
                total = checked(total-info[0])
                frees = checked(frees+1)
                temporary = checked(temporary+int(is_temp))
                emit_digest(events, '-', *info)
        elif op == 'c':
            emit_digest(events, 'c', fields[0])
    session.finish(True)
    require(sum(size for size, _ in pointers.values()) == total, 'raw final sum')
    result = dict(bytes=total, count=len(pointers), allocations=allocations, frees=frees,
                  unknown_frees=unknown, temporary=temporary, max_live_pointers=high,
                  events=events.hexdigest(), traces=traces, trace_digest=trace_hash.hexdigest(),
                  metadata=session.metadata.hexdigest(), lines=stream.lines)
    budget.charge(-384*high)
    return result


def interpreted_pass(entry, tables, requested_time=None, checkpoints=None) -> tuple[dict[str, Any], dict]:
    b = tables.b
    session = Session()
    events, trace_hash = hashlib.sha256(), hashlib.sha256()
    stream = Input(entry, b.limits['interpreted'], b.limits['line'])
    total = count = allocations = frees = temporary = last_index = high = 0
    peak = dict(line=0, offset=0, timestamp_ms=0, bytes=0, count=0)
    last_mark = requested = next_mark = last_event = None
    snapshots = {}
    # Pass two validates every definition against the retained compact table.
    definitions = Counter({'s': 1, 'i': 1, 't': 1})
    suppression_index = 0
    if checkpoints is not None:
        tables.live = array('Q', [0])*len(tables.live)
        tables.calls = array('Q', [0])*len(tables.calls)
        tables.trace_calls = array('Q', [0])*len(tables.trace_calls)

    def snapshot():
        rows = {}
        descriptor_rows = []
        for index, n in enumerate(tables.live):
            if not n:
                continue
            size, trace = tables.desc[index*2:index*2+2]
            value = checked(n*size)
            if trace not in rows:
                b.charge(384)
                rows[trace] = [0, 0, 0]
            row = rows[trace]
            row[0] = checked(row[0]+value)
            row[1] = checked(row[1]+n)
            row[2] = tables.trace_calls[trace]
            b.charge(384)
            descriptor_rows.append((index, trace, size, n, tables.calls[index], value))
        require(sum(r[0] for r in rows.values()) == total and sum(r[1] for r in rows.values()) == count, 'snapshot sums')
        return dict(rows=rows, descriptors=descriptor_rows, bytes=total, count=count)

    for line, offset, data in stream:
        op, fields = parse_record(data, line_limit=b.limits['line'])
        session.accept(op, fields, line)
        if op in ('s', 'i', 't', 'a', 'S'):
            if checkpoints is None:
                tables.definition(op, fields)
            else:
                index = definitions[op]
                if op == 's':
                    actual = tables.string(index)
                elif op == 'i':
                    actual = list(tables.ip(index))
                elif op == 't':
                    actual = list(tables.traces[index*2:index*2+2])
                elif op == 'a':
                    actual = list(tables.desc[index*2:index*2+2])
                else:
                    actual = tables.suppressions[suppression_index]
                    suppression_index += 1
                require(actual == fields, 'second pass definition mismatch')
            definitions[op] += 1
            if op == 't':
                addr = tables.ip(fields[0])[0] if fields[0] else 0
                emit_digest(trace_hash, 't', addr, fields[1])
        elif op in ('+', '-'):
            index = fields[0]
            require(index < definitions['a'], 'descriptor reference before use')
            size, trace = tables.desc[index*2:index*2+2]
            if op == '+':
                tables.live[index] = checked(tables.live[index]+1)
                tables.calls[index] = checked(tables.calls[index]+1)
                tables.trace_calls[trace] = checked(tables.trace_calls[trace]+1)
                allocations = checked(allocations+1)
                total, count = checked(total+size), checked(count+1)
                high = max(high, tables.live[index])
                last_index = index
                if total > peak['bytes']:
                    peak = dict(line=line, offset=offset, timestamp_ms=session.stamp, bytes=total, count=count)
            else:
                require(tables.live[index] > 0, 'descriptor underflow')
                tables.live[index] -= 1
                total, count = checked(total-size), checked(count-1)
                frees = checked(frees+1)
                temporary = checked(temporary+int(index == last_index))
                last_index = 0
            last_event = dict(line=line, offset=offset, previous_timestamp_ms=session.stamp)
            emit_digest(events, op, size, trace)
        elif op == 'c':
            last_mark = dict(line=line, offset=offset, timestamp_ms=session.stamp,
                             ordinal=session.marks, bytes=total, count=count)
            emit_digest(events, 'c', session.stamp)
            if requested_time is not None:
                if session.stamp <= requested_time:
                    requested = last_mark
                elif next_mark is None:
                    next_mark = last_mark
        if checkpoints is not None:
            names = [name for name, pos in checkpoints.items() if pos == line]
            if names:
                snap = snapshot()
                for name in names:
                    snapshots[name] = snap
    session.finish(False)
    require(last_mark is not None, 'no actual timestamp mark')
    if requested_time is not None:
        require(type(requested_time) is int and 0 <= requested_time <= last_mark['timestamp_ms'] and requested is not None, 'requested time outside actual marks')
    if checkpoints is not None:
        snapshots['eof'] = snapshot()
        if checkpoints['peak'] == 0:
            snapshots['peak'] = dict(rows={}, descriptors=[], bytes=0, count=0)
    result = dict(peak=peak, last_mark=last_mark, requested=requested, next_mark=next_mark,
                  last_event=last_event, eof=dict(line=stream.lines, offset=stream.offset, bytes=total, count=count),
                  events=events.hexdigest(), metadata=session.metadata.hexdigest(), trace_digest=trace_hash.hexdigest(),
                  allocations=allocations, frees=frees, temporary=temporary, max_multiplicity=high,
                  definitions=dict(definitions), sha256=stream.digest.hexdigest())
    return result, snapshots


NEW = {b'operator new(unsigned long)', b'operator new[](unsigned long)',
       b'operator new(unsigned int)', b'operator new[](unsigned int)'}
STOP = {b'main', b'__libc_start_main', b'__static_initialization_and_destruction_0'}


def pretty(symbol):
    result = bytearray()
    depth = 0
    for c in symbol:
        if c in (60, 62) and len(result) >= 8:
            suffix = b'operator'+bytes([c]) if result[-1] == c else b'operator'
            if result.endswith(suffix):
                result.append(c)
                continue
        if c == 60:
            depth += 1
            if depth == 1:
                result.append(c)
        elif c == 62:
            depth -= 1
        if not depth:
            result.append(c)
    return bytes(result)


def stack(tables, trace):
    frames = []
    while trace:
        require(len(frames) < tables.b.limits['depth'], 'stack depth cap')
        ip, trace = tables.traces[trace*2:trace*2+2]
        frames.append(ip)
    return frames


def canonical(tables, trace):
    if trace == 0:
        return b'??'
    # Analyzer rewrites EVERY trace node that begins with operator-new to its
    # normalized parent. Consequently skip such nodes anywhere along the chain.
    pieces = []
    rendered_bytes = 0
    for ip_id in stack(tables, trace):
        if not ip_id:
            break
        ip = tables.ip(ip_id)
        fn = tables.string(ip[2]) if len(ip) > 2 else b''
        if fn in NEW:
            continue
        text = pretty(fn) if len(ip) > 2 and ip[2] else ('0x%x' % ip[0]).encode()
        if len(ip) >= 5 and ip[3]:
            text += b' ('+tables.string(ip[3]).rsplit(b'/', 1)[-1]+b')'
        text += b';'
        for i in range(5, len(ip), 3):
            text += pretty(tables.string(ip[i]))+b' ('+tables.string(ip[i+1]).rsplit(b'/', 1)[-1]+b');'
        pieces.append(text)
        rendered_bytes += len(text)
        require(rendered_bytes <= tables.b.limits['line'], 'rendered stack cap')
        if fn in STOP:
            break
    return b''.join(reversed(pieces))


def reconcile_peak(entry, tables, snapshot):
    # Multiset keyed by full normalized printable stack; collisions add costs,
    # but original trace IDs remain separate in outputs. Never compare sums only.
    expected = Counter()
    charge = 0
    for trace, (cost, _, _) in snapshot['rows'].items():
        if cost:
            key = canonical(tables, trace)
            if key not in expected:
                n = 320+len(key)
                tables.b.charge(n)
                charge += n
            expected[key] = checked(expected[key]+cost)
    stream = Input(entry, tables.b.limits['peak'], tables.b.limits['line'])
    total = positive = rows = 0
    for _, _, line in stream:
        key, sep, token = line[:-1].rpartition(b' ')
        require(bool(sep) and bool(re.fullmatch(rb'[0-9]{1,19}', token)), 'canonical cost syntax')
        cost = checked(int(token))
        rows += 1
        total = checked(total+cost)
        if cost:
            positive += 1
            require(key in expected and expected[key] >= cost, 'canonical positive stack mismatch')
            expected[key] -= cost
    require(not any(expected.values()) and total == snapshot['bytes'], 'canonical peak mismatch')
    tables.b.charge(-charge)
    return dict(rows=rows, positive_rows=positive, bytes=total)


def analyze(entries, requested_time=None, limits=None, phase=lambda n: None) -> dict[str, Any]:
    require(requested_time is None or (type(requested_time) is int and 0 <= requested_time <= U64), 'invalid requested time')
    budget = Budget(limits)
    phase(1)
    raw = raw_pass(entries['raw'], budget)
    phase(2)
    tables = Tables(budget)
    first, _ = interpreted_pass(entries['interpreted'], tables, requested_time)
    for key in ('events', 'metadata', 'trace_digest', 'allocations', 'frees'):
        require(raw[key] == first[key], 'raw conversion mismatch: '+key)
    require(raw['traces'] == len(tables.traces)//2-1, 'raw trace count mismatch')
    require((raw['bytes'], raw['count']) == (first['eof']['bytes'], first['eof']['count']), 'raw final population mismatch')
    checkpoints = {name: first[name]['line'] for name in ('peak', 'last_mark')}
    if requested_time is not None:
        checkpoints['requested'] = first['requested']['line']
    phase(3)
    second, snapshots = interpreted_pass(entries['interpreted'], tables, requested_time, checkpoints)
    require(first == second, 'interpreted passes differ')
    comparison = reconcile_peak(entries['peak'], tables, snapshots['peak'])
    return dict(raw=raw, first=first, snapshots=snapshots, tables=tables,
                canonical_equal=True, canonical=comparison, table_high_bytes=budget.high)


HOST = 'c0709aba2f8b45e42193225cf8f4e7325b5ca9bf'
CLIENT = '3456edc8dabf7b25ada78110ffa56327af9f67a4'
# Conservative source-domain mapping from the frozen ledger. Ranges are source
# anchors, NOT a claim that every call below a matching symbol owns its objects.
# In particular broad constructors and publication paths remain mixed/unknown.
RULES = (
    ('nav::world::NavWorld::load_pack', 'crates/nav/src/world.rs', 49, 56, 'nav_world_startup', 'process_shared_or_transient', HOST),
    ('client::client::client::Client::from_shared', 'crates/client/src/client/client.rs', 300, 948, 'client_cache_interface_world', 'mixed_shared_private', CLIENT),
    ('client::core::world::World::new', 'crates/client/src/core/world.rs', 30, 130, 'client_world', 'source_per_client', CLIENT),
)
DOMAINS = (
    ('crates/api/src/snapshot.rs', 'game_snapshot_families', 'unknown', HOST, '532-687'),
    ('crates/script/src/isolate_fb.rs', 'script_fingerprint_encoded_buffers', 'unknown', HOST, '1386-1448'),
    ('crates/script/src/slot.rs', 'script_slot', 'unknown', HOST, '26-52'),
    ('crates/host-play/src/memory.rs', 'harness_infrastructure', 'process_infrastructure', HOST, '1014-1078'),
)


def classify(tables, trace, frozen):
    frames = stack(tables, trace)
    fallback = None
    unknown_symbol = not frames or any(len(tables.ip(i)) < 3 or not tables.ip(i)[2] for i in frames)
    for ip_id in frames:
        ip = tables.ip(ip_id)
        if len(ip) < 3 or not ip[2]:
            unknown_symbol = True
        for index in range(2, len(ip), 3):
            fn = tables.string(ip[index]).decode('utf-8')
            file = tables.string(ip[index+1]).decode('utf-8') if index+1 < len(ip) else ''
            line = ip[index+2] if index+2 < len(ip) else 0
            if not fn:
                unknown_symbol = True
                continue
            if fallback is None and fn.encode() not in NEW:
                fallback = (fn, file, line)
            # Only useful application Rust frames, never arbitrary 'alloc::'
            # functions that happen to mention an owner type in a template.
            useful = fn.startswith(('nav::', 'client::', 'api::', 'host_play::', 'script::', 'tui_play::'))
            if not useful:
                continue
            domain, scope, anchor = 'unresolved_source', 'unknown', None
            if frozen:
                for symbol, path, start, end, owner, boundary, commit in RULES:
                    if (fn == symbol or fn.startswith(symbol+'::h')) and (file == path or file.endswith('/'+path)) and start <= line <= end:
                        domain, scope = owner, boundary
                        anchor = dict(commit=commit, path=path, lines=f'{start}-{end}')
                        break
                if anchor is None:
                    for path, owner, boundary, commit, lines in DOMAINS:
                        if file == path or file.endswith('/'+path):
                            domain, scope = owner, boundary
                            anchor = dict(commit=commit, path=path, lines=lines, mapping='domain_only_not_exact_allocation_owner')
                            break
            return dict(symbol=fn, file=file, line=line, domain=domain, ownership=scope,
                        anchor=anchor, unknown_symbol=unknown_symbol)
    fn, file, line = fallback or ('<unresolved>', '', 0)
    return dict(symbol=fn, file=file, line=line, domain='allocator_or_native_boundary',
                ownership='unknown', anchor=None, unknown_symbol=unknown_symbol)


def json_cell(value):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':'))


class Output:
    def __init__(self, directory, limits):
        self.directory, self.limits = directory, limits
        self.total = 0
        self.files = {}

    def write(self, name, rows):
        digest = hashlib.sha256()
        size = 0
        with (self.directory/name).open('xb') as out:
            os.chmod(self.directory/name, 0o600)
            for row in rows:
                PULSE()
                data = row.encode('utf-8')
                self.total += len(data)
                size += len(data)
                require(self.total <= self.limits['output'], 'output cap')
                require(self.total <= self.limits['scratch'], 'scratch cap')
                require(shutil.disk_usage(self.directory).free >= self.limits['free_disk'], 'free disk guard')
                out.write(data)
                digest.update(data)
        self.files[name] = dict(bytes=size, sha256=digest.hexdigest())


def write_outputs(result, directory, provenance, resources=None):
    """Writes into runner-owned unpublished scratch only. Runner removes on failure."""
    tables, first = result['tables'], result['first']
    b = tables.b
    out = Output(directory, b.limits)
    traces, ips, strings = set(), set(), {0}
    families, membership = {}, {}
    frozen = provenance.get('host') == HOST and provenance.get('client') == CLIENT
    for snapshot in result['snapshots'].values():
        for trace in snapshot['rows']:
            if trace in membership:
                continue
            value = classify(tables, trace, frozen)
            symbol_unknown = value.pop('unknown_symbol')
            key = json_cell(value)
            family = hashlib.sha256(key.encode()).hexdigest()
            if family not in families:
                b.charge(768+len(key)*2)
                families[family] = value
            b.charge(192)
            membership[trace] = (family, symbol_unknown)
            node = trace
            depth = 0
            while node:
                depth += 1
                require(depth <= b.limits['depth'], 'stack depth cap')
                if node not in traces:
                    b.charge(128)
                    traces.add(node)
                ip, node = tables.traces[node*2:node*2+2]
                if ip not in ips:
                    b.charge(128)
                    ips.add(ip)
    for index in ips:
        ip = tables.ip(index)
        refs = [ip[1]] if ip else []
        for i in range(2, len(ip), 3):
            refs.append(ip[i])
            if i+1 < len(ip):
                refs.append(ip[i+1])
        for ref in refs:
            if ref not in strings:
                b.charge(128)
                strings.add(ref)
    # All text is JSON-escaped inside TSV cells, preserving every UTF-8 byte and
    # tabs/CR/backslashes. Trace parent links preserve full stacks without copies.

    def string_rows():
        yield 'string_id\tutf8_bytes\ttext_json\n'
        for index in sorted(strings):
            text = tables.string(index)
            yield f'{index}\t{len(text)}\t{json_cell(text.decode("utf-8"))}\n'
    out.write('strings.tsv', string_rows())
    out.write('ips.tsv', ('ip_id\tfields_hex_json\n' if index is None else
              f'{index}\t{json_cell([format(n, "x") for n in tables.ip(index)])}\n'
              for index in [None]+sorted(ips)))
    out.write('traces.tsv', ('trace_id\tip_id\tparent_trace_id\n' if index is None else
              f'{index}\t{tables.traces[index*2]}\t{tables.traces[index*2+1]}\n'
              for index in [None]+sorted(traces)))
    out.write('families.tsv', ('family_id\tprovenance_json\n' if index is None else
              f'{index}\t{json_cell(families[index])}\n' for index in [None]+sorted(families)))
    cutoffs = {}
    for name, snapshot in result['snapshots'].items():
        family_costs = {}
        unknown_bytes = unknown_symbol_bytes = 0
        for trace, (cost, count, calls) in snapshot['rows'].items():
            family, unknown = membership[trace]
            if family not in family_costs:
                b.charge(256)
                family_costs[family] = [0, 0]
            family_costs[family][0] = checked(family_costs[family][0]+cost)
            family_costs[family][1] = checked(family_costs[family][1]+count)
            if families[family]['ownership'] == 'unknown':
                unknown_bytes += cost
            if unknown:
                unknown_symbol_bytes += cost
        ordered = sorted(snapshot['rows'], key=lambda trace: (-snapshot['rows'][trace][0], trace))
        out.write(name+'-stacks.tsv', chain(['trace_id\tfamily_id\trequested_bytes\tlive_count\tallocation_calls_full_trace\n'],
            (f'{trace}\t{membership[trace][0]}\t'+ '\t'.join(map(str, snapshot['rows'][trace]))+'\n' for trace in ordered)))
        ordered_f = sorted(family_costs, key=lambda f: (-family_costs[f][0], f))
        out.write(name+'-families.tsv', chain(['family_id\trequested_bytes\tlive_count\n'],
            (f'{f}\t{family_costs[f][0]}\t{family_costs[f][1]}\n' for f in ordered_f)))
        out.write(name+'-descriptors.tsv', chain(['descriptor_id\ttrace_id\tsize\tlive_count\tallocation_calls\trequested_bytes\n'],
            ('\t'.join(map(str, row))+'\n' for row in sorted(snapshot['descriptors'], key=lambda r: (-r[5], r[0])))))
        require(sum(v[0] for v in family_costs.values()) == snapshot['bytes'], 'family accounting')
        b.charge(-256*len(family_costs))
        mark = first.get(name)
        cutoffs[name] = dict(kind={'peak': 'first_common_time_global_peak_validation', 'last_mark': 'last_actual_timestamp_prefix',
            'eof': 'captured_prefix_end', 'requested': 'requested_mark_prefix'}[name],
            actual_mark=mark if name in ('last_mark', 'requested') else None,
            position=first['eof'] if name == 'eof' else mark,
            next_mark=first['next_mark'] if name == 'requested' else None,
            last_observed_mark=first['last_mark'] if name == 'eof' else None,
            event_tail_time_unbounded=(name == 'eof' and first['last_event'] is not None and
                                       first['last_event']['line'] > first['last_mark']['line']),
            requested_time=provenance.get('requested_time') if name == 'requested' else None,
            bytes=snapshot['bytes'], count=snapshot['count'], unknown_ownership_bytes=unknown_bytes,
            unknown_symbol_bytes=unknown_symbol_bytes)
    receipt = dict(schema='heaptrack-owner-replay/v1', status='validated_prefix_diagnostic',
        capture_complete=False, acceptance=False, population='recorded_tracked_requested_bytes',
        provenance=provenance, raw=result['raw'], interpreted=first, cutoffs=cutoffs,
        canonical_peak=result['canonical'], canonical_full_positive_multiset_equal=True,
        leak_comparison='unsuppressed_replay_only_no_exact_canonical_leak_oracle',
        suppressions=[s.decode('utf-8') for s in tables.suppressions], suppressions_applied=False,
        tables=dict(strings=len(tables.strings)//2-1, ips=len(tables.ip_offsets)//2-1,
                    traces=len(tables.traces)//2-1, descriptors=len(tables.live), high_charged_bytes=b.high),
        resources=resources or {}, limits=b.limits, files=out.files.copy(),
        limitations=['Capture FAILED; missing observe-end/Stop/post-join is not repaired by replay.',
          'Unknown frees and unrecorded/realloc-to-zero events remain missing coverage.',
          'Hashes do not detect pre-manifest whole-record suffix loss; EOF is not shutdown.',
          'Descriptor IDs are not pointers, instances, ages, snapshots or publication epochs.',
          'Requested bytes are not RSS, allocator retained pages, V8/pool/mmap/GPU inventory.',
          'Source domains are not per-instance ownership, private N1 attribution or N16 scaling.'])
    out.write('receipt.json', [json.dumps(receipt, ensure_ascii=False, indent=2)+'\n'])
    return out.total
