"""Project AMD64 minidump thread and full captured-stack module evidence.

The stack words are address candidates, not unwound frames. The evaluator
accepts only a complete, OS-only projection and records this limit.
"""

import bisect
import hashlib
import struct
from pathlib import Path


def analyze(dump_path: Path, descriptions: dict[int, str], process_id: int) -> dict:
    data = dump_path.read_bytes()

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
            continue
        if kind in streams or rva + size > len(data):
            raise ValueError("Duplicate or out-of-bounds minidump stream")
        streams[kind] = (rva, size)
    required_streams = (3, 4, 7, 15, 17)
    if any(kind not in streams for kind in required_streams):
        raise ValueError("Thread, module, system, misc info, or thread info stream absent")
    arch = unpack("<H", streams[7][0])[0]
    if arch != 9:
        raise ValueError(f"Unsupported minidump architecture: {arch}")

    misc_rva, misc_size = streams[15]
    if misc_size < 12:
        raise ValueError("Invalid MINIDUMP_MISC_INFO extent")
    misc_info_size, misc_flags = unpack("<II", misc_rva)
    if misc_info_size < 12 or misc_info_size > misc_size or not misc_flags & 1:
        raise ValueError("MINIDUMP_MISC_INFO does not contain a valid process ID")
    sampled_pid = unpack("<I", misc_rva + 8)[0]
    if sampled_pid != process_id:
        raise ValueError(
            f"minidump process ID {sampled_pid} does not match sampled process {process_id}"
        )

    module_rva, module_size = streams[4]
    module_count = unpack("<I", module_rva)[0]
    if module_count > 10000 or 4 + module_count * 108 > module_size:
        raise ValueError("Invalid module list extent")
    modules = []
    for i in range(module_count):
        base, size, _checksum, _timestamp, name_rva = unpack(
            "<QIIII", module_rva + 4 + i * 108
        )
        name_size = unpack("<I", name_rva)[0]
        if name_size > 32768 or name_rva + 4 + name_size > len(data):
            raise ValueError("Invalid module name")
        name = data[name_rva + 4 : name_rva + 4 + name_size].decode("utf-16-le")
        modules.append((base, base + size, name))
    modules.sort()
    starts = [module[0] for module in modules]

    def identify(value: int):
        i = bisect.bisect_right(starts, value) - 1
        if i < 0 or value >= modules[i][1]:
            return None
        return {"module": modules[i][2], "offset": f"0x{value - modules[i][0]:X}"}

    if not descriptions:
        raise ValueError("missing sampled thread identities")

    info_rva, info_size = streams[17]
    if info_size < 12:
        raise ValueError("Invalid MINIDUMP_THREAD_INFO_LIST extent")
    header_size, entry_size, info_count = unpack("<III", info_rva)
    if (header_size < 12 or entry_size < 64 or info_count > 10000
            or header_size + entry_size * info_count > info_size):
        raise ValueError("Invalid MINIDUMP_THREAD_INFO_LIST extent")
    thread_info = {}
    duplicate_info = set()
    for i in range(info_count):
        offset = info_rva + header_size + entry_size * i
        tid = unpack("<I", offset)[0]
        if tid in thread_info or tid in duplicate_info:
            thread_info.pop(tid, None)
            duplicate_info.add(tid)
            continue
        creation_time = unpack("<Q", offset + 16)[0]
        start_address = unpack("<Q", offset + 48)[0]
        thread_info[tid] = (creation_time, start_address)

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
        try:
            if tid in duplicate_info:
                raise ValueError(f"duplicate thread-info entry for TID {tid}")
            info = thread_info.get(tid)
            if info is None:
                raise ValueError(f"missing thread-info entry for TID {tid}")

            stack_start, stack_size, stack_rva = unpack("<QII", offset + 24)
            context_size, context_rva = unpack("<II", offset + 40)
            if context_size < 0x100 or context_rva + context_size > len(data):
                raise ValueError(f"Missing AMD64 thread context for TID {tid}")
            rsp = unpack("<Q", context_rva + 0x98)[0]
            rip = unpack("<Q", context_rva + 0xF8)[0]
            if stack_rva + stack_size > len(data):
                raise ValueError(f"Missing thread stack for TID {tid}")
            stack_end = stack_start + stack_size
            if rsp < stack_start or rsp >= stack_end:
                raise ValueError(f"stack pointer outside captured stack for TID {tid}")
            stack_offset = rsp - stack_start
            stack_window_bytes = stack_size - stack_offset
            if stack_window_bytes < 256:
                raise ValueError(f"short captured stack for TID {tid}")

            candidate_addresses = []
            module_counts = {}
            first_word = ((rsp + 7) & ~7) - stack_start
            for relative in range(first_word, stack_size - 7, 8):
                value = unpack("<Q", stack_rva + relative)[0]
                mapped = identify(value)
                if mapped is None:
                    continue
                module = mapped["module"]
                module_counts[module] = module_counts.get(module, 0) + 1
                if len(candidate_addresses) < 32:
                    candidate_addresses.append({
                        "stack_offset": f"0x{relative:X}",
                        "address": f"0x{value:X}",
                        **mapped,
                    })

            creation_time, start_address = info
            mapped_start = identify(start_address)
            row = {
                "tid": tid,
                "description_first_sample": descriptions.get(tid),
                "thread_creation_filetime": creation_time,
                "start_module": mapped_start["module"] if mapped_start else None,
                "ip": f"0x{rip:X}",
                "ip_module": identify(rip),
                "sp": f"0x{rsp:X}",
                "stack_start": f"0x{stack_start:X}",
                "stack_bytes": stack_size,
                "stack_window_bytes": stack_window_bytes,
                "candidate_return_addresses_first_32_hits": candidate_addresses,
                "candidate_return_address_module_counts": {
                    module: module_counts[module] for module in sorted(module_counts)
                },
            }
        except (ValueError, struct.error) as error:
            row = {"tid": tid, "error": str(error)}
        rows.append(row)

    for tid in sorted(set(descriptions) - seen):
        rows.append({"tid": tid, "error": f"thread missing from minidump: TID {tid}"})

    return {
        "schema": "t14-windows-thread-minidump-stack-projection-v2",
        "dump_path": str(dump_path),
        "dump_sha256": hashlib.sha256(data).hexdigest(),
        "minidump_architecture": "AMD64",
        "dump_bytes": len(data),
        "module_count": module_count,
        "thread_count": thread_count,
        "sampled_pid": sampled_pid,
        "threads": rows,
        "limits": (
            "Stack-word module hits are candidates, not unwound call frames; "
            "the projection covers only the captured stack from RSP and cannot "
            "observe future thread-pool callbacks."
        ),
    }
