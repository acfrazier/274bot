"""Project bounded AMD64 minidump stack/module evidence for Windows census.

The stack words are address candidates, not unwound frames. The evaluator
accepts only a complete, OS-only bounded projection and records this limit.
"""

import bisect
import hashlib
import struct
from pathlib import Path


def analyze(dump_path: Path, descriptions: dict[int, str], process_id: int) -> dict:
    data = dump_path.read_bytes()
    if len(data) > 8 * 1024 * 1024:
        raise ValueError("Windows thread minidump exceeds 8 MiB")

    def unpack(fmt: str, offset: int):
        size = struct.calcsize(fmt)
        if offset < 0 or offset + size > len(data):
            raise ValueError(f"Out-of-bounds minidump read: {offset}+{size}")
        return struct.unpack_from(fmt, data, offset)

    if data[:4] != b"MDMP":
        raise ValueError("Not a MINIDUMP")
    stream_count, directory_rva = unpack("<II", 8)
    if stream_count > 1024:
        raise ValueError("Unexpected stream count")
    streams = {}
    for i in range(stream_count):
        kind, size, rva = unpack("<III", directory_rva + 12 * i)
        if (kind, size, rva) == (0, 0, 0):
            continue  # MINIDUMP may reserve zeroed directory slots.
        if kind in streams or rva + size > len(data):
            raise ValueError("Duplicate or out-of-bounds minidump stream")
        streams[kind] = (rva, size)
    if 3 not in streams or 4 not in streams or 7 not in streams:
        raise ValueError("Thread, module, or system info stream absent")
    arch = unpack("<H", streams[7][0])[0]
    if arch != 9:
        raise ValueError(f"Unsupported minidump architecture: {arch}")

    module_rva, module_size = streams[4]
    module_count = unpack("<I", module_rva)[0]
    if module_count > 10000 or 4 + module_count * 108 > module_size:
        raise ValueError("Invalid module list extent")
    modules = []
    for i in range(module_count):
        base, size, _checksum, _timestamp, name_rva = unpack("<QIIII", module_rva + 4 + i * 108)
        name_size = unpack("<I", name_rva)[0]
        if name_size > 32768 or name_rva + 4 + name_size > len(data):
            raise ValueError("Invalid module name")
        name = data[name_rva + 4 : name_rva + 4 + name_size].decode("utf-16-le")
        modules.append((base, base + size, name))
    modules.sort()
    starts = [m[0] for m in modules]

    def identify(value: int):
        i = bisect.bisect_right(starts, value) - 1
        if i < 0 or value >= modules[i][1]:
            return None
        return {"module": modules[i][2], "offset": f"0x{value - starts[i]:X}"}

    if not descriptions:
        raise ValueError("missing sampled thread identities")
    thread_rva, thread_size = streams[3]
    thread_count = unpack("<I", thread_rva)[0]
    if thread_count > 10000 or 4 + 48 * thread_count > thread_size:
        raise ValueError("Invalid thread list extent")
    rows = []
    seen = set()
    for i in range(thread_count):
        offset = thread_rva + 4 + 48 * i
        tid = unpack("<I", offset)[0]
        if tid in seen:
            raise ValueError(f"duplicate minidump thread ID {tid}")
        seen.add(tid)
        stack_start, stack_size, stack_rva = unpack("<QII", offset + 24)
        context_size, context_rva = unpack("<II", offset + 40)
        if context_size < 0x108 or context_rva + context_size > len(data):
            raise ValueError(f"Missing AMD64 thread context for TID {tid}")
        rsp, rip = unpack("<Q", context_rva + 0xA0)[0], unpack("<Q", context_rva + 0x100)[0]
        if stack_rva + stack_size > len(data):
            raise ValueError(f"Missing thread stack for TID {tid}")
        if rsp < stack_start or rsp >= stack_start + stack_size:
            raise ValueError(f"stack pointer outside captured stack for TID {tid}")
        stack_offset = rsp - stack_start
        addresses = []
        stack_limit = min(stack_size, stack_offset + 2048)
        if stack_limit - stack_offset < 256:
            raise ValueError(f"short captured stack for TID {tid}")
        for relative in range((stack_offset + 7) & ~7, stack_limit - 7, 8):
            value = unpack("<Q", stack_rva + relative)[0]
            mapped = identify(value)
            if mapped:
                addresses.append({"stack_offset": f"0x{relative:X}", "address": f"0x{value:X}", **mapped})
        row = {
            "tid": tid,
            "description_first_sample": descriptions.get(tid),
            "ip": f"0x{rip:X}",
            "ip_module": identify(rip),
            "sp": f"0x{rsp:X}",
            "stack_start": f"0x{stack_start:X}",
            "stack_bytes": stack_size,
            "stack_window_bytes": stack_limit - stack_offset,
            "candidate_return_addresses_first_2048_stack_bytes": addresses[:32],
            "candidate_return_address_module_counts": {
                module: sum(1 for address in addresses if address["module"] == module)
                for module in sorted({address["module"] for address in addresses})
            },
        }
        rows.append(row)
    return {
        "schema": "t14-windows-thread-minidump-stack-projection-v2",
        "dump_path": str(dump_path),
        "dump_sha256": hashlib.sha256(data).hexdigest(),
        "minidump_architecture": "AMD64",
        "dump_bytes": len(data),
        "module_count": module_count,
        "thread_count": thread_count,
        "sampled_pid": process_id,
        "threads": rows,
        "limits": "Stack-word module hits are candidates, not unwound call frames; the bounded projection does not prove absence of deeper product frames.",
    }


