#!/usr/bin/env python3
"""Adjudicate two T1.4 release soak receipts without inventing missing evidence."""
from __future__ import annotations

import argparse
from bisect import bisect_right
import hashlib
import json
import re
from collections import Counter
from pathlib import Path
import ntpath

import t14_windows_thread_evidence


class EvidenceError(ValueError):
    pass


def document(path: Path) -> dict:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise EvidenceError(f"expected JSON object: {path}")
    return value


def iter_rows(path: Path):
    seen = False
    with path.open(encoding="utf-8") as stream:
        for number, line in enumerate(stream, 1):
            value = json.loads(line)
            if not isinstance(value, dict):
                raise EvidenceError(f"non-object JSONL row {path}:{number}")
            seen = True
            yield value
    if not seen:
        raise EvidenceError(f"empty receipt: {path}")


def rows(path: Path) -> list[dict]:
    return list(iter_rows(path))


def demand(condition: bool, errors: list[str], message: str) -> None:
    if not condition:
        errors.append(message)

def arm_evidence(arm: str, operation, *args):
    try:
        return operation(*args)
    except (OSError, ValueError, KeyError, TypeError, IndexError) as error:
        raise EvidenceError(f"{arm}: missing or invalid evidence: {error}") from error


def check_nav_identity(receipt: dict, errors: list[str]) -> None:
    source_pack, selected_pack, source_flags, selected_flags = (
        receipt[key] for key in ("source_pack_sha256", "selected_pack_sha256",
                                 "source_flags_sha256", "selected_flags_sha256"))
    demand(source_pack == selected_pack and source_flags == selected_flags,
           errors, "public-build nav bytes differ from fixed selected source")


def rss_slope(samples: list[dict], cutoff_ns: int) -> tuple[float, float]:
    points = [(float(row["sample_monotonic_ns"]) / 1_000_000_000, float(row["rss_bytes"]))
              for row in samples if row["sample_monotonic_ns"] >= cutoff_ns]
    if len(points) < 2:
        raise EvidenceError("fewer than two post-warmup RSS samples")
    xs, ys = zip(*points)
    mean_x, mean_y = sum(xs) / len(xs), sum(ys) / len(ys)
    denominator = sum((x - mean_x) ** 2 for x in xs)
    if denominator == 0:
        raise EvidenceError("post-warmup RSS sample times are identical")
    slope_per_hour = 3600 * sum((x - mean_x) * (y - mean_y) for x, y in points) / denominator
    return slope_per_hour, max(xs) - min(xs)


def rss_arm_summary(summary: dict, samples: list[dict]) -> tuple[float, float, int]:
    cutoff = summary["run"]["warmup_end_monotonic_ns"]
    if not isinstance(cutoff, int):
        raise EvidenceError("missing monotonic warmup end for RSS slope fit")
    slope, window = rss_slope(samples, cutoff)
    return slope, window, samples[-1]["rss_bytes"]


def rss_thresholds(config: dict, baseline_final: int,
                   baseline_slope: float) -> tuple[float, float]:
    limits = config["limits"]
    ceiling = (baseline_final + 20 * limits["rss_final_baseline_extra_bytes_per_connection"]
               + baseline_final * limits["rss_final_baseline_extra_percent"] / 100)
    return ceiling, baseline_slope + limits["rss_slope_baseline_slack_bytes_per_hour"]


def check_samples(folder: Path, summary: dict, config: dict, errors: list[str]) -> list[dict]:
    samples = rows(folder / "process-samples.jsonl")
    last = -1.0
    interval = int(config["run"]["sample_seconds"])
    for row in samples:
        elapsed = float(row["elapsed_seconds"])
        demand(elapsed > last, errors, f"{folder.name}: nonmonotone /proc sample time")
        if last >= 0:
            demand(elapsed - last <= interval + 2, errors,
                   f"{folder.name}: /proc sampling gap > {interval + 2}s")
        last = elapsed
        threads = row.get("threads")
        demand(isinstance(threads, list) and len(threads) == row.get("task_count"),
               errors, f"{folder.name}: incomplete per-task census at {elapsed}s")
        demand(isinstance(row.get("rss_bytes"), int) and row["rss_bytes"] > 0,
               errors, f"{folder.name}: invalid RSS at {elapsed}s")
    if summary.get("result") == "workload-window-complete-awaiting-external-evaluation":
        phase_zero = summary["run"]["phase_zero_monotonic_ns"]
        last_ns = samples[-1]["sample_monotonic_ns"]
        demand(last_ns - phase_zero >=
               (int(config["run"]["duration_seconds"]) - interval - 2) * 1_000_000_000,
               errors, f"{folder.name}: /proc sampled less than the two-hour script-Start clock")
    return samples


# Linux comm truncates each name at 15 bytes. Product singleton ownership is
# source-backed; unknown names are never silently charged to L_T.
F_NAMES = {
    name[:15] for name in (
        "net-reactor", "dns-resolver", "js-deadline-supervisor",
        "work-admission", "profile-writer", "local-state-writer",
        "script-bridge", "V8 WorkerThread", "js-validation-coordinator",
        "session-log", "resource-meter", "net-tls-handshake",
        "net-https-handshake",
    )
}
TERMS = ("N", "J", "V", "F", "H", "A", "B", "G", "E", "E_v", "L_T", "R", "O")
REQUEST_PREFIXES = ("nav-find-", "nav-inspect-", "bank-pick-", "card-prepare-")


def windows_os_module(module: object) -> bool:
    if not isinstance(module, str):
        return False
    path = ntpath.normcase(module)
    return path.startswith("c:\\windows\\system32\\") and path.endswith(".dll")


def windows_runtime_provenance(folder: Path, sample: dict, task: dict,
                               cache: dict[tuple[str, int, int], dict]) -> bool:
    """Accept only a matched, OS-only bounded snapshot for this live identity."""
    evidence = task.get("os_runtime_provenance")
    pid = sample.get("process_id")
    tid = task.get("tid")
    created = task.get("thread_creation_filetime")
    if (not isinstance(evidence, dict)
            or evidence.get("schema") != "t14-windows-thread-minidump-stack-projection-v2"
            or not all(isinstance(value, int) and value > 0 for value in (pid, tid, created))
            or evidence.get("sampled_pid") != pid
            or evidence.get("tid") != tid
            or evidence.get("thread_creation_filetime") != created
            or not isinstance(evidence.get("capture_sample_monotonic_ns"), int)
            or not 0 < evidence["capture_sample_monotonic_ns"] <= sample["sample_monotonic_ns"]):
        return False
    dump_path = evidence.get("dump_path")
    digest = evidence.get("dump_sha256")
    if (not isinstance(dump_path, str)
            or not re.fullmatch(r"snapshot-[0-9]{4}\.dmp", ntpath.basename(dump_path))
            or not isinstance(digest, str)
            or not re.fullmatch(r"[0-9a-f]{64}", digest)):
        return False
    name = ntpath.basename(dump_path)
    key = (name, pid, tid)
    try:
        if key not in cache:
            cache[key] = t14_windows_thread_evidence.analyze(
                folder / "thread-provenance" / name, {tid: ""}, pid)
        projection = cache[key]
        if projection["dump_sha256"] != digest or projection["sampled_pid"] != pid:
            return False
        row = next((row for row in projection["threads"] if row["tid"] == tid), None)
        if row is None or row["description_first_sample"] != "":
            return False
        ip = row["ip_module"]
        modules = row["candidate_return_address_module_counts"]
        if (not isinstance(ip, dict) or not windows_os_module(ip.get("module"))
                or row["stack_window_bytes"] < 256
                or not isinstance(modules, dict) or not modules
                or not all(isinstance(count, int) and count > 0 and windows_os_module(module)
                           for module, count in modules.items())):
            return False
        return (evidence.get("ip_module") == ip
                and evidence.get("stack_window_bytes") == row["stack_window_bytes"]
                and evidence.get("candidate_return_address_module_counts") == modules)
    except (OSError, ValueError, KeyError, TypeError):
        return False


def check_census(folder: Path, samples: list[dict], summary: dict, config: dict,
                 errors: list[str]) -> dict:
    accounts = [slot["account"] for slot in summary["slots"]]
    allowed_slots = {account[:15] for account in accounts}
    phase_zero = summary["run"]["phase_zero_monotonic_ns"]
    observed: list[dict] = []
    unknown: set[tuple[int, str, int]] = set()
    provenance_cache: dict[tuple[str, int, int], dict] = {}
    for sample in samples:
        counts = Counter({term: 0 for term in TERMS})
        fixed_seen: set[str] = set()
        tasks = sample["threads"]
        live_slots = allowed_slots & {task["comm"] for task in tasks}
        for task in tasks:
            comm = task["comm"]
            tid = task["tid"]
            windows = sample.get("sampler_platform") == "windows"
            if windows:
                raw = task.get("windows_thread_description")
                valid_identity = (isinstance(sample.get("process_id"), int)
                                  and task.get("windows_process_id") == sample["process_id"]
                                  and isinstance(task.get("thread_creation_filetime"), int)
                                  and task["thread_creation_filetime"] > 0
                                  and isinstance(raw, str)
                                  and (comm == "t14_soak_live-" and tid == sample["pid"]
                                       and task.get("windows_process_main_thread") is True
                                       or comm == raw[:15]
                                       and task.get("windows_process_main_thread") is False))
                demand(valid_identity, errors,
                       f"{folder.name}: Windows raw thread identity mismatch tid={tid}")
            if comm in allowed_slots:
                term = "N"
            elif comm.startswith("t14_soak_live-") and tid == sample["pid"]:
                term = "F"  # OS process main, separate from libtest's test thread
                fixed_seen.add("main")
            elif comm in F_NAMES:
                term = "F"
                demand(comm not in fixed_seen, errors,
                       f"{folder.name}: duplicate process singleton {comm}")
                fixed_seen.add(comm)
            elif comm == "t14_local_wss_s":
                term = "L_T"  # libtest's one named harness test thread
            elif comm == "ondemand":
                term = "H"
            elif comm == "js-isolate":
                term = "J"
            elif comm == "script-blk":
                term = "B"
            elif comm == "panel-gpu-init":
                term = "G"
            elif comm == "evidence-writer":
                term = "E_v"
            elif any(comm == f"{prefix}{account}"[:15]
                     for prefix in REQUEST_PREFIXES for account in accounts):
                term = "R"
                demand(any(comm == f"{prefix}{slot}"[:15]
                           for prefix in REQUEST_PREFIXES
                           for slot in live_slots), errors,
                       f"{folder.name}: request thread {comm} has no live slot owner")
            elif (sample.get("sampler_platform") == "windows"
                  and comm == "" and task.get("windows_thread_description") == ""
                  and task.get("windows_process_main_thread") is False
                  and windows_runtime_provenance(folder, sample, task, provenance_cache)):
                term = "O"
            else:
                term = None
                identity = (tid, comm, task.get("thread_creation_filetime", -1))
                if identity not in unknown:
                    unknown.add(identity)
                    errors.append(f"{folder.name}: unclassified task tid={tid} comm={comm!r}")
            if term is not None:
                counts[term] += 1
        elapsed = sample["elapsed_seconds"]
        demand(counts["F"] <= config["limits"]["fixed_threads"]
               and len(fixed_seen) == counts["F"], errors,
               f"{folder.name}: named singleton bound/identity failed at {elapsed}s")
        demand(counts["R"] <= config["limits"]["max_request_threads"], errors,
               f"{folder.name}: request-thread R cap exceeded at {elapsed}s")
        for term, cap in (("J", counts["N"]), ("V", 1), ("H", 1), ("A", 4),
                          ("B", 1), ("G", 1), ("E", 1), ("E_v", 1)):
            demand(counts[term] <= cap, errors,
                   f"{folder.name}: {term} exceeded its census cap at {elapsed}s")
        demand(counts["F"] >= 1 and counts["L_T"] == 1, errors,
               f"{folder.name}: missing process or libtest main at {elapsed}s")
        if phase_zero is not None and sample["sample_monotonic_ns"] >= phase_zero:
            demand(counts["N"] == config["run"]["slots"], errors,
                   f"{folder.name}: not all named slot workers lived after first 101 at {elapsed}s")
        counted = sum(counts.values())
        demand(counted == sample["task_count"], errors,
               f"{folder.name}: T != N+J+V+F+H+A+B+G+E+E_v+L_T+R+O at {elapsed}s")
        observed.append({"sample_monotonic_ns": sample["sample_monotonic_ns"],
                         "elapsed_seconds": elapsed, "T": sample["task_count"],
                         "terms": {term: counts[term] for term in TERMS}})
    return {"per_sample": observed,
            "term_maxima": {term: max(row["terms"][term] for row in observed) for term in TERMS},
            "unclassified_tasks": [{"tid": tid, "comm": comm, "thread_creation_filetime": created}
                                   for tid, comm, created in sorted(unknown)]}


def check_scene_samples(folder: Path, events: list[dict], accounts: list[str],
                        first_scene: dict[str, int | None], sample_seconds: int,
                        errors: list[str]) -> None:
    # The client enters scene_state 1 while rebuilding (snapshot.rs:979-993).
    # Play slot frame callbacks record scene2 transitions (t14_soak_live.rs:
    # 1824-1853, 2615-2630). Observed Engine A loads returned 17-45 ms
    # after the loading sample. The 1 s return limit is a policy bound.
    # Allow the next Play sample one interval plus 2 s of jitter.
    recovery_ns = (sample_seconds + 2) * 1_000_000_000
    return_ns = 1_000_000_000
    epochs: dict[str, int] = {}
    loading_since: dict[str, int] = {}
    scene_returns: dict[str, list[int]] = {account: [] for account in accounts}
    for event in events:
        if event.get("event") == "scene2" and event.get("account") in scene_returns:
            scene_returns[event["account"]].append(event["scene2_monotonic_ns"])
    for times in scene_returns.values():
        times.sort()
    for account in accounts:
        if first_scene[account] is None:
            errors.append(f"{folder.name}: {account} never entered scene2")
    last_sample_ns: int | None = None
    for event in (row for row in events if row.get("event") == "sample"):
        at = event["sample_monotonic_ns"]
        if last_sample_ns is not None:
            demand(0 < at - last_sample_ns <= recovery_ns, errors,
                   f"{folder.name}: Play sample gap exceeds {sample_seconds + 2}s at {event['elapsed_seconds']}s")
        last_sample_ns = at
        snapshots = event.get("slots")
        demand(isinstance(snapshots, list) and len(snapshots) == len(accounts), errors,
               f"{folder.name}: incomplete ten-second slot snapshot")
        if not isinstance(snapshots, list):
            continue
        seen: set[str] = set()
        for slot in snapshots:
            account = slot.get("account")
            if account not in first_scene or account in seen:
                errors.append(f"{folder.name}: unknown or duplicate account in slot snapshot")
                continue
            seen.add(account)
            at = event["sample_monotonic_ns"]
            if first_scene[account] is None or at < first_scene[account]:
                continue
            epoch = slot.get("stream_epoch")
            if not isinstance(epoch, int) or epoch <= 0:
                errors.append(f"{folder.name}: {account} lacks a live stream epoch at {event['elapsed_seconds']}s")
                continue
            if epochs.setdefault(account, epoch) != epoch:
                errors.append(f"{folder.name}: {account} changed stream epoch at {event['elapsed_seconds']}s")
            if slot.get("connected") is not True:
                errors.append(f"{folder.name}: {account} disconnected at {event['elapsed_seconds']}s")
            if slot.get("ingame") is True and slot.get("scene_state") == 2:
                since = loading_since.pop(account, None)
                if since is not None:
                    demand(at - since <= recovery_ns, errors,
                           f"{folder.name}: {account} did not return to scene2 within {sample_seconds + 2}s")
            elif slot.get("ingame") is False and slot.get("scene_state") == 1:
                since = loading_since.setdefault(account, at)
                times = scene_returns[account]
                next_return = bisect_right(times, at)
                demand(next_return < len(times) and 0 <= times[next_return] - at <= return_ns,
                       errors, f"{folder.name}: {account} map load has no scene2 return within 1s")
                demand(at - since <= recovery_ns, errors,
                       f"{folder.name}: {account} stayed in scene1 beyond {sample_seconds + 2}s")
            else:
                errors.append(f"{folder.name}: {account} left scene2 outside map loading at {event['elapsed_seconds']}s")
        demand(seen == set(accounts), errors,
               f"{folder.name}: slot snapshot lacks an account at {event['elapsed_seconds']}s")
    for account in loading_since:
        errors.append(f"{folder.name}: {account} never returned to scene2")


def check_workload(folder: Path, summary: dict, config: dict, smoke: bool,
                   errors: list[str]) -> dict:
    count = int(config["run"]["slots"])
    slots = summary.get("slots")
    if not isinstance(slots, list) or len(slots) != count:
        raise EvidenceError(f"{folder.name}: expected {count} per-account Play summaries")
    accounts = [slot.get("account") for slot in slots]
    demand(len(set(accounts)) == count, errors, f"{folder.name}: duplicated account")
    demand(len({slot.get("site_id") for slot in slots}) >= 2, errors,
           f"{folder.name}: bank workload lacks two named content sites")
    events = rows(folder / "events.jsonl")
    kinds = Counter(event.get("event") for event in events)
    demand(kinds["unexpected-exit"] == 0 and kinds["failure"] == 0,
           errors, f"{folder.name}: unexpected exit or task failure recorded")
    demand(kinds["scene2"] >= count and kinds["script-started"] >= count,
           errors, f"{folder.name}: some account never reached scene2 and started Gatherer")
    demand(kinds["sample"] > 0, errors, f"{folder.name}: missing ten-second Play snapshots")
    demand(kinds["phase-zero"] == 1, errors, f"{folder.name}: missing/duplicate script-Start clock")
    phase_zero = summary.get("run", {}).get("phase_zero_monotonic_ns")
    demand(summary.get("run", {}).get("duration_anchor") ==
           "first-script-start-after-all-slot-setup", errors,
           f"{folder.name}: run duration did not start at the cohort's first script Start")
    starts = [event for event in events if event.get("event") == "script-started"]
    demand(len(starts) == count, errors, f"{folder.name}: duplicate/missing native script Starts")
    if starts and isinstance(phase_zero, int):
        first = min(starts, key=lambda event: event["script_start_monotonic_ns"])
        demand(first["script_start_monotonic_ns"] == phase_zero
               and summary.get("run", {}).get("phase_zero", {}).get("account") == first["account"],
               errors, f"{folder.name}: proxy phase zero differs from first successful script Start")
    for account in accounts:
        requested = [event for event in events if event.get("event") == "setup-relog-requested"
                     and event.get("account") == account]
        exited = [event for event in events if event.get("event") == "setup-relog-exit"
                  and event.get("account") == account]
        returned = [event for event in events if event.get("event") == "setup-relog-returned"
                    and event.get("account") == account]
        ready = [event for event in events if event.get("event") == "script-start-ready"
                 and event.get("account") == account]
        demand(len(requested) == len(exited) == len(returned) == len(ready) == 1,
               errors, f"{folder.name}: {account} lacks exactly one justified setup Relog")
        if requested and exited and returned and ready and isinstance(phase_zero, int):
            times = [event["event_monotonic_ns"] for event in
                     (requested[0], exited[0], returned[0], ready[0])]
            demand(all(isinstance(at, int) for at in times)
                   and times == sorted(times) and times[-1] < phase_zero
                   and requested[0].get("logout_packet_output_bytes", 0) > 0,
                   errors, f"{folder.name}: {account} setup Relog did not precede the fault window")
    demand(summary.get("limits", {}).get("unexpected_exits") == 0, errors,
           f"{folder.name}: summary reports unexpected exits")
    demand(summary.get("result") == ("smoke-complete-nonqualifying" if smoke
                                     else "workload-window-complete-awaiting-external-evaluation"), errors,
           f"{folder.name}: Play workload did not complete")
    by_account = {account: slot for account, slot in zip(accounts, slots, strict=True)}
    bank_events = Counter(event.get("account") for event in events if event.get("event") == "bank-cycle")
    for account, slot in by_account.items():
        demand(slot.get("scene2_unix_ms") is not None and slot.get("script_running_observed") is True,
               errors, f"{folder.name}: {account} lacks scene2 or native Gatherer Running proof")
        demand(slot.get("connected_at_finish") is True and slot.get("failure") is None,
               errors, f"{folder.name}: {account} exited or failed")
        if not smoke:
            demand(slot.get("warmup_qualified") is True and slot.get("bank_cycles", 0) >= 1
                   and bank_events[account] >= 1 and slot.get("walk_tile_changes", 0) > 0,
                   errors, f"{folder.name}: {account} lacks a real moved-and-deposited bank cycle")
            status = slot.get("last_task_status") or {}
            demand(status.get("deposited", 0) > 0
                   and str(status.get("bank", "")).startswith(slot.get("expected_bank_prefix", "\0")),
                   errors, f"{folder.name}: {account} bank product/status proof absent")
    first_scene = {account: min((event["scene2_monotonic_ns"] for event in events
                                 if event.get("event") == "scene2" and event.get("account") == account),
                                default=None) for account in accounts}
    check_scene_samples(folder, events, accounts, first_scene,
                        int(config["run"]["sample_seconds"]), errors)
    return {"accounts": count, "sites": sorted({slot["site_id"] for slot in slots}),
            "bank_events": sum(bank_events.values()), "play_samples": kinds["sample"]}


def check_opcode_projection(folder: Path, connection: dict, errors: list[str]) -> None:
    for direction, data in connection["proxy_b_websocket"].items():
        original = data["opcodes_read"]
        forwarded = data["opcodes_written"]
        added = data.get("fault_fragment_frames", 0) - data.get("fragmented_messages", 0)
        pings = data.get("injected_ping_frames", 0)
        consumed_pongs = data.get("injected_pong_frames_consumed", 0)
        if direction == "client_to_engine":
            added = 0
            pings = 0
            demand(consumed_pongs <= original.get("10", 0), errors,
                   f"{folder.name}: proxy consumed more Pongs than received")
        else:
            demand(consumed_pongs == 0, errors,
                   f"{folder.name}: server-to-client path consumed a Pong")
        demand(added >= 0, errors, f"{folder.name}: negative fragment-continuation count")
        for opcode in set(original) | set(forwarded) | {"0", "2", "9", "10"}:
            expected = original.get(opcode, 0)
            if opcode == "0":
                expected += added
            elif opcode == "9":
                expected += pings
            elif opcode == "10":
                expected -= consumed_pongs
            demand(forwarded.get(opcode, 0) == expected, errors,
                   f"{folder.name}: unexpected WS opcode {opcode} in {direction}")
        demand(data["frames_written"] == data["frames_read"] + added + pings - consumed_pongs,
               errors, f"{folder.name}: WS frame count diverged outside approved fragmentation/Pong consumption")


def accepted_tls_close_error(failure: dict, stage: dict,
                             deficits: tuple[int, int, int, int], before_phase_zero: bool) -> bool:
    # A two-byte deficit in proxy-B-to-client plaintext identifies the failed
    # secure.send of the terminal Close, not a read or TLS shutdown.
    return (isinstance(failure, dict) and failure.get("stage") == "tls_terminator"
            and stage.get("tls_terminator") == "failed" and stage.get("proxy_a") == "closed"
            and before_phase_zero and deficits == (0, 0, 0, 2)
            and isinstance(failure.get("error"), str)
            and re.fullmatch(
                r"SSLEOFError: EOF occurred in violation of protocol(?: \(_ssl\.c:\d+\))?",
                failure["error"],
            ) is not None)


def baseline_teardown_fault(failure: dict, stage: dict) -> bool:
    """Recognize witnessed relay teardown classes, never arbitrary faults."""
    if not isinstance(failure, dict) or len(failure) != 2:
        return False
    origin, error = failure.get("stage"), failure.get("error")
    if not isinstance(error, str):
        return False
    if origin == "proxy_a" and stage.get("proxy_a") == "failed":
        return error in (
            "BrokenPipeError: [Errno 32] Broken pipe",
            "ConnectionResetError: [Errno 54] Connection reset by peer",
            "ConnectionResetError: [Errno 104] Connection reset by peer",
        )
    if origin == "tls_terminator" and stage.get("tls_terminator") == "failed":
        return (error == "BrokenPipeError: [Errno 32] Broken pipe"
                or re.fullmatch(
                    r"SSLEOFError: EOF occurred in violation of protocol(?: \(_ssl\.c:\d+\))?",
                    error) is not None)
    return False


def check_transport_teardown(folder: Path, arm: str, event: dict,
                             close_frames: dict[str, list[dict]],
                             phase_zero_ns: int | None, errors: list[str],
                             observations: list[dict] | None = None) -> None:
    identifier = event["connection_id"]
    stage = event["stage_status"]
    handshake = event["websocket_handshake"]
    status = handshake["upgrade_status"]
    a = event["proxy_a_ciphertext"]
    tls = event["tls_terminator"]
    def deficit(counters: dict) -> int:
        return counters.get("bytes_read", 0) - counters.get("bytes_written", 0)

    a_in = deficit(a["client_to_terminator_ciphertext"])
    a_out = deficit(a["terminator_to_client_ciphertext"])
    tls_in = deficit(tls["client_to_proxy_b_plaintext"])
    tls_out = deficit(tls["proxy_b_to_client_plaintext"])
    failures = event.get("failures", [])
    prefix = f"{folder.name}: connection {identifier}"
    demand(stage.get("proxy_b") == "closed" and tls["handshake"].get("completed") == 1,
           errors, f"{prefix}: application relay or TLS handshake did not complete")
    if status == 200:
        # A complete HTTPS response is not a failed transfer merely because
        # the client closed its read side before the TLS pump's final recv.
        ws = event["proxy_b_websocket"]
        out = ws["engine_to_client"]
        fixture = handshake.get("local_login_key_fixture")
        if fixture is not None:
            body_complete = out.get("wire_bytes_written") == fixture.get("body_bytes")
        else:
            body_complete = (out.get("proxy_b_http_fallback_bytes_read", 0)
                             == out.get("proxy_b_http_fallback_bytes_written", 0))
        demand(body_complete and out.get("http_header_bytes_written") == handshake["response_bytes"],
               errors, f"{prefix}: HTTPS response body/header did not finish at proxy B")
        demand((a_in, a_out, tls_in, tls_out) == (0, 0, 0, 0), errors,
               f"{prefix}: HTTPS transfer has unmatched ciphertext or plaintext bytes")
        demand(failures in ([], [{"stage": "tls_terminator",
                                  "error": "BrokenPipeError: [Errno 32] Broken pipe"}]),
               errors, f"{prefix}: HTTPS teardown error is not the witnessed terminal TLS write")
    elif status == 101:
        ws = event["proxy_b_websocket"]
        completed_close = all(
            len(close_frames.get(direction, [])) == 1
            and close_frames[direction][0].get("status_code") is None
            and close_frames[direction][0].get("reason") == ""
            and ws[direction]["opcodes_read"].get("8") == 1
            and ws[direction]["opcodes_written"].get("8") == 1
            for direction in ("client_to_engine", "engine_to_client")
        )
        demand(completed_close, errors, f"{prefix}: no complete zero-payload bidirectional WS Close")
        demand(event["binary_payload_hashes_match_at_proxy_b"].get("client_to_engine") is True
               and event["binary_payload_hashes_match_at_proxy_b"].get("engine_to_client") is True,
               errors, f"{prefix}: game binary payload hash diverged before Close")
        # The old teardown allowance includes small proxy-A and TLS deficits.
        # The new baseline observation rule below requires zero deficits.
        demand(a_in == tls_in == 0 and 0 <= a_out <= 24 and 0 <= tls_out <= 2,
               errors, f"{prefix}: ciphertext/plaintext deficit extends beyond terminal WS Close")
        # setup Relog precedes the first-script-Start fault window
        # (check_workload); the one-byte fault check below excludes earlier
        # connections. A reset is harmless only with complete Close, matching
        # binary hashes and no lost transport bytes.
        before_phase_zero = (isinstance(phase_zero_ns, int)
                             and isinstance(event.get("started_monotonic_ns"), int)
                             and isinstance(event.get("closed_monotonic_ns"), int)
                             and event["started_monotonic_ns"] < event["closed_monotonic_ns"] < phase_zero_ns)
        setup_reset = (before_phase_zero and completed_close
                       and stage.get("proxy_a") == "failed"
                       and stage.get("tls_terminator") == "closed"
                       and (a_in, a_out, tls_in, tls_out) == (0, 0, 0, 0)
                       and event["binary_payload_hashes_match_at_proxy_b"].get("client_to_engine") is True
                       and event["binary_payload_hashes_match_at_proxy_b"].get("engine_to_client") is True)
        matching_hashes = (
            event["binary_payload_hashes_match_at_proxy_b"].get("client_to_engine") is True
            and event["binary_payload_hashes_match_at_proxy_b"].get("engine_to_client") is True)
        # Both Close frames passed proxy B. The new rule records a baseline
        # teardown fault only when every transport deficit is zero.
        baseline_end = (arm == "baseline" and stage.get("proxy_b") == "closed"
                        and completed_close and matching_hashes
                        and tls["handshake"].get("completed") == 1
                        and (a_in, a_out, tls_in, tls_out) == (0, 0, 0, 0))
        def accepted_prior_rule(failure: dict) -> bool:
            return (
                accepted_tls_close_error(
                    failure, stage, (a_in, a_out, tls_in, tls_out), before_phase_zero)
                or failure == {"stage": "proxy_a",
                               "error": "BrokenPipeError: [Errno 32] Broken pipe"}
                or (setup_reset and failure == {
                    "stage": "proxy_a",
                    "error": "ConnectionResetError: [Errno 104] Connection reset by peer",
                })
            )

        if arm == "baseline" and observations is not None:
            for failure in failures:
                observations.append({
                    "connection_id": identifier, "failure": failure,
                    "non_gating": (baseline_end and baseline_teardown_fault(failure, stage)
                                   and not accepted_prior_rule(failure)),
                    "byte_deficits": [a_in, a_out, tls_in, tls_out],
                })
        # Both transports support TLS 1.2. This soak exercises the candidate
        # on TLS 1.3; the native-tls baseline may negotiate either version.
        version = tls["handshake"].get("tls_version")
        version_ok = version == "TLSv1.3" if arm == "candidate" else version in ("TLSv1.2", "TLSv1.3")
        demand(version_ok and all(
            accepted_prior_rule(failure)
            or (baseline_end and baseline_teardown_fault(failure, stage))
            for failure in failures
        ), errors, f"{prefix}: unexpected TLS/segmentation error")
        if not failures:
            demand(a_out == tls_out == 0, errors,
                   f"{prefix}: unreported final WS Close transport deficit")
    else:
        demand(not failures and (a_in, a_out, tls_in, tls_out) == (0, 0, 0, 0), errors,
               f"{prefix}: incomplete non-200/non-101 HTTP transfer")


def check_proxy(folder: Path, arm: str, full: bool, errors: list[str],
                observations: list[dict] | None = None) -> dict[int, dict]:
    fixture_config = document(folder / "effective-config.json")["proxy"]
    modulus = fixture_config["local_login_modulus"]
    source_sha256 = fixture_config["local_login_key_source_sha256"]
    fixture_body = f'const loginRsaN = "{modulus}";\n'.encode("ascii")
    fixture_sha256 = hashlib.sha256(fixture_body).hexdigest()
    fixture_count = 0
    phase_zeros: list[dict] = []
    stopped = 0
    listener_failures = 0
    close_frames: dict[str, dict[str, list[dict]]] = {}
    bad_messages = 0
    closed = []
    for event in iter_rows(folder / "proxy" / "connections.jsonl"):
        kind = event.get("type")
        if kind == "fault_phase_zero":
            phase_zeros.append(event)
        elif kind == "proxy_stopped":
            stopped += 1
        elif kind == "listener_failure":
            listener_failures += 1
        elif kind == "websocket_binary_message" and event.get("message_hashes_match") is not True:
            bad_messages += 1
        elif kind == "websocket_close_frame":
            close_frames.setdefault(event["connection_id"], {}).setdefault(
                event["direction"], []
            ).append(event)
        elif kind == "connection_close":
            closed.append(event)
    demand(len(phase_zeros) == 1, errors,
           f"{folder.name}: no unique first-script-Start phase-zero proof")
    if len(phase_zeros) == 1:
        marker = document(folder / "proxy" / "soak-start.json")
        start = phase_zeros[0]
        demand(start.get("trigger") == "first_script_start_after_setup"
               and start.get("phase_zero_monotonic_ns") == marker.get("monotonic_ns")
               and start.get("account") == marker.get("account")
               and start.get("seed") == marker.get("seed"), errors,
               f"{folder.name}: proxy marker does not match the Play script-Start phase")
    demand(stopped == 1 and listener_failures == 0, errors,
           f"{folder.name}: proxy failed or did not terminate cleanly")
    demand(bad_messages == 0, errors,
           f"{folder.name}: {bad_messages} proxy B binary message divergences")
    ports: dict[int, dict] = {}
    for event in closed:
        fixture = event.get("websocket_handshake", {}).get("local_login_key_fixture")
        if fixture is not None:
            fixture_count += 1
            demand(event["websocket_handshake"].get("upgrade_status") == 200
                   and fixture.get("rsa_source_sha256") == source_sha256
                   and fixture.get("served_body_sha256") == fixture_sha256
                   and fixture.get("body_bytes") == len(fixture_body),
                   errors, f"{folder.name}: local Engine RSA fixture provenance differs")
        check_transport_teardown(
            folder, arm, event, close_frames.get(event["connection_id"], {}),
            phase_zeros[0]["phase_zero_monotonic_ns"] if len(phase_zeros) == 1 else None,
            errors, observations)
        if event.get("websocket_handshake", {}).get("upgrade_status") != 101:
            continue  # HTTPS asset transfer retains raw bytes; not a WSS pair.
        port = event.get("client_peer_port")
        if not isinstance(port, int) or port in ports:
            errors.append(f"{folder.name}: missing/colliding physical client peer port {port}")
            continue
        ports[port] = event
        check_opcode_projection(folder, event, errors)
        for direction, matched in event.get("binary_payload_hashes_match_at_proxy_b", {}).items():
            demand(matched is True, errors,
                   f"{folder.name}: proxy B {direction} physical stream SHA mismatch")
        for direction, counters in event.get("proxy_a_ciphertext", {}).items():
            if len(phase_zeros) == 1 and event.get("started_monotonic_ns", 0) >= phase_zeros[0]["phase_zero_monotonic_ns"]:
                demand(counters.get("one_byte_writes", 0) >= min(4096, counters.get("bytes_written", 0)),
                       errors, f"{folder.name}: first 4096 faulted ciphertext bytes not sent singly ({direction})")
    demand(bool(ports), errors, f"{folder.name}: no game WebSocket connection completed")
    demand(fixture_count > 0, errors, f"{folder.name}: no Engine A local RSA fixture served")
    if full:
        for direction in ("client_to_terminator_ciphertext", "terminator_to_client_ciphertext"):
            for phase in range(4):
                demand(sum(row.get("latency_phase_chunk_counts", {}).get(direction, {}).get(str(phase), 0)
                           for row in ports.values()) > 0, errors,
                       f"{folder.name}: {direction} lacks fault phase {phase + 1}")
    if full:
        demand(sum(row["proxy_b_websocket"]["engine_to_client"].get("fragmented_messages", 0)
                   for row in ports.values()) > 0, errors,
               f"{folder.name}: no server binary frames fragmented")
        demand(sum(row["proxy_b_websocket"]["engine_to_client"].get("injected_ping_frames", 0)
                   for row in ports.values()) > 0, errors,
               f"{folder.name}: no interleaved WebSocket Ping")
        demand(sum(row["proxy_a_ciphertext"]["client_to_terminator_ciphertext"].get("stalls", 0)
                   for row in ports.values()) > 0, errors,
               f"{folder.name}: no phase-four TCP stall")
    return ports


def check_candidate(folder: Path, proxy: dict[int, dict], limit: int,
                    errors: list[str]) -> dict:
    ledger = rows(folder / "candidate-five-compartment-ledger.jsonl")
    ports = set()
    sampled_ports: dict[int, set[int]] = {}
    for sample in ledger:
        port = sample.get("client_local_port")
        demand(port in proxy, errors, f"candidate ledger lacks corresponding proxy port {port}")
        ports.add(port)
        if isinstance(port, int):
            sampled_ports.setdefault(sample.get("elapsed_seconds"), set()).add(port)
        compartments = tuple(sample.get(key, {}).get("reserved_bytes") for key in
                             ("received_plaintext", "pending_tls_output", "handshake_deframer",
                              "outbound_ring", "inbound_ring"))
        demand(compartments == (16384, 131072, 65536, 5000, 32768), errors,
               f"candidate incorrect five-compartment reservation at {sample.get('elapsed_seconds')}s")
        demand(sample.get("reserved_total_bytes") == limit and sample.get("overrun") is False,
               errors, f"candidate WSS five-compartment overrun on port {port}")
        demand(sample.get("actual_whole_connection_bytes") is None, errors,
               "candidate incorrectly labels reserved compartments as measured whole connection")
        for key in ("received_plaintext", "pending_tls_output", "outbound_ring", "inbound_ring"):
            row = sample.get(key, {})
            actual = row.get("observed_bytes")
            demand(actual is None or (isinstance(actual, int) and 0 <= actual <= row["reserved_bytes"]),
                   errors, f"candidate {key} known occupancy exceeded on port {port}")
    demand(bool(ports), errors, "candidate five-compartment ledger has no WSS ports")
    warmup_end_ns = document(folder / "summary.json")["run"]["warmup_end_monotonic_ns"]
    for event in iter_rows(folder / "events.jsonl"):
        if event.get("event") != "sample":
            continue
        if warmup_end_ns is None or event["sample_monotonic_ns"] < warmup_end_ns:
            continue
        active = sum(slot.get("connected") is True for slot in event["slots"])
        demand(len(sampled_ports.get(event["elapsed_seconds"], set())) == active,
               errors, f"candidate five-compartment ledger did not cover exactly the active WSS ports at {event['elapsed_seconds']}s")
    records = rows(folder / "candidate-wss-payloads.jsonl")
    expected_prefixes: dict[tuple[int, str, str, int], str] = {}
    latest: dict[int, dict] = {}
    for record in records:
        port = record.get("client_local_port")
        if not isinstance(port, int):
            errors.append("candidate payload digest is missing its physical client port")
            continue
        if record.get("elapsed_seconds", -1) >= latest.get(port, {}).get("elapsed_seconds", -1):
            latest[port] = record
        for own, direction, flow in (("outbound", "client_to_engine", "read"),
                                     ("inbound", "engine_to_client", "written")):
            stream = record[own]
            key = (port, direction, flow, stream["bytes"])
            previous = expected_prefixes.setdefault(key, stream["sha256"])
            demand(previous == stream["sha256"], errors,
                   f"candidate nonmonotone SHA at the same byte count on port {port}")
    demand(set(latest) == set(proxy), errors,
           "candidate WSS payload ports do not biject with proxy A game connections")
    for port, digest in latest.items():
        pair = proxy.get(port)
        if not pair:
            continue
        for own, direction, flow in (("outbound", "client_to_engine", "read"),
                                     ("inbound", "engine_to_client", "written")):
            recorded = digest[own]
            actual_hash = pair["binary_payload_sha256"][direction][flow]
            actual_bytes = pair["binary_payload_bytes"][direction][flow]
            demand(recorded.get("sha256") == actual_hash and recorded.get("bytes") == actual_bytes,
                   errors, f"candidate {own} complete-binary SHA/byte mismatch on port {port}")
    seen_prefixes: set[tuple[int, str, str, int]] = set()
    empty_hash = hashlib.sha256(b"").hexdigest()
    for key, digest in expected_prefixes.items():
        if key[3] == 0:
            demand(digest == empty_hash, errors, f"candidate nonempty SHA at zero bytes on {key[0]}")
            seen_prefixes.add(key)
    for message in iter_rows(folder / "proxy" / "connections.jsonl"):
        if message.get("type") != "websocket_binary_message":
            continue
        port = message.get("client_peer_port")
        direction = message.get("direction")
        flow = "read" if direction == "client_to_engine" else "written"
        key = (port, direction, flow, message.get(f"rolling_payload_bytes_{flow}"))
        if key in expected_prefixes:
            seen_prefixes.add(key)
            demand(message.get(f"rolling_payload_sha256_{flow}") == expected_prefixes[key],
                   errors, f"candidate minute payload SHA differs at {key[3]} bytes on port {port}")
    demand(seen_prefixes == set(expected_prefixes), errors,
           f"candidate has {len(set(expected_prefixes) - seen_prefixes)} unmatched minute SHA prefixes")
    return {"ledger_samples": len(ledger), "payload_snapshots": len(records),
            "matched_minute_prefixes": len(seen_prefixes), "wss_ports": len(proxy)}


def evaluate(baseline: Path, candidate: Path, smoke: bool) -> dict:
    errors: list[str] = []
    base_errors: list[str] = []
    cand_errors: list[str] = []
    baseline_teardown: list[dict] = []
    lhs = arm_evidence("baseline", document, baseline / "run-manifest.json")
    rhs = arm_evidence("candidate", document, candidate / "run-manifest.json")
    for receipt, arm in ((lhs, "baseline"), (rhs, "candidate")):
        if receipt.get("build") != arm:
            raise EvidenceError(f"{arm}: run manifest build must be {arm}")
    for key in ("shared_harness_sha256", "proxy_sha256", "seed", "profile", "mode",
                "nav_build_mode", "nav_build_snapshot_root"):
        demand(lhs.get(key) == rhs.get(key), errors, f"baseline/candidate {key} differ")
    demand(lhs["mode"] == ("smoke" if smoke else "qualification"), errors,
           "incorrect smoke/qualification mode")
    demand(lhs["profile"] == "release", errors, "both builds must use release profile")
    left_nav = arm_evidence(lhs["build"], document, baseline / "nav-identity.json")
    right_nav = arm_evidence(rhs["build"], document, candidate / "nav-identity.json")
    for receipt, arm, arm_errors in ((left_nav, "baseline", base_errors),
                                    (right_nav, "candidate", cand_errors)):
        arm_evidence(arm, check_nav_identity, receipt, arm_errors)
    for key in ("source_pack_sha256", "selected_pack_sha256",
                "source_flags_sha256", "selected_flags_sha256"):
        demand(left_nav[key] == right_nav[key], errors,
               f"baseline/candidate {key} navigation provenance differs")
    left_config = arm_evidence(lhs["build"], document, baseline / "effective-config.json")
    right_config = arm_evidence(rhs["build"], document, candidate / "effective-config.json")
    for key in ("fault", "limits", "workload"):
        demand(left_config[key] == right_config[key], errors,
               f"baseline/candidate {key} contract differs")
    for key in ("slots", "duration_seconds", "warmup_min_seconds", "sample_seconds", "hash_seconds"):
        demand(left_config["run"][key] == right_config["run"][key], errors,
               f"baseline/candidate run.{key} differs")
    demand(left_config["run"]["slots"] == (2 if smoke else 20), errors,
           "incorrect cohort size")
    demand(left_config["run"]["duration_seconds"] == (180 if smoke else 7200), errors,
           "incorrect run duration")
    demand(left_config["fault"]["seed"] == lhs["seed"], errors,
           "proxy seed differs from shared run manifest")
    for folder, label, arm_errors in ((baseline, lhs["build"], base_errors),
                                      (candidate, rhs["build"], cand_errors)):
        demand(arm_evidence(label, document, folder / "cohort-exit.json").get("code") == 0,
               arm_errors, "cohort process failed")
    base_summary = arm_evidence(lhs["build"], document, baseline / "summary.json")
    cand_summary = arm_evidence(rhs["build"], document, candidate / "summary.json")
    base_work = arm_evidence(lhs["build"], check_workload,
                             baseline, base_summary, left_config, smoke, base_errors)
    cand_work = arm_evidence(rhs["build"], check_workload,
                             candidate, cand_summary, right_config, smoke, cand_errors)
    base_samples = arm_evidence(lhs["build"], check_samples,
                                baseline, base_summary, left_config, base_errors)
    cand_samples = arm_evidence(rhs["build"], check_samples,
                                candidate, cand_summary, right_config, cand_errors)
    base_census = arm_evidence(lhs["build"], check_census,
                               baseline, base_samples, base_summary, left_config, base_errors)
    cand_census = arm_evidence(rhs["build"], check_census,
                               candidate, cand_samples, cand_summary, right_config, cand_errors)
    base_proxy = arm_evidence(lhs["build"], check_proxy, baseline, "baseline", not smoke,
                              base_errors, baseline_teardown)
    cand_proxy = arm_evidence(rhs["build"], check_proxy, candidate, "candidate", not smoke, cand_errors)
    limit = arm_evidence("candidate", lambda: right_config["limits"]["connection_bytes"])
    candidate_receipts = arm_evidence(
        rhs["build"], check_candidate, candidate, cand_proxy, limit, cand_errors)
    result = {
        "classification": "infrastructure-smoke-nonqualifying" if smoke else "T1.4-local-candidate-gate",
        "baseline_commit": lhs["commit"], "candidate_commit": rhs["commit"],
        "shared_harness_sha256": lhs["shared_harness_sha256"],
        "baseline": {**base_work, "proxy_wss_connections": len(base_proxy),
                     "teardown_observations": baseline_teardown,
                     "tls_versions_observed": sorted({
                         row["tls_terminator"]["handshake"]["tls_version"]
                         for row in base_proxy.values()
                         if isinstance(row["tls_terminator"]["handshake"].get("tls_version"), str)
                     }), "census": base_census},
        "candidate": {**cand_work, **candidate_receipts, "census": cand_census},
    }
    if not smoke:
        baseline_slope, baseline_window, final_base = arm_evidence(
            "baseline", rss_arm_summary, base_summary, base_samples)
        candidate_slope, candidate_window, final_candidate = arm_evidence(
            "candidate", rss_arm_summary, cand_summary, cand_samples)
        final_ceiling, max_slope = arm_evidence(
            "candidate", rss_thresholds, right_config, final_base, baseline_slope)
        demand(candidate_slope <= max_slope, errors,
               "candidate post-warmup RSS slope exceeded baseline +1 MiB/hour")
        demand(final_candidate <= final_ceiling, errors,
               "candidate final RSS exceeded baseline + 20 reservations +5%")
        result["rss"] = {
            "baseline_slope_bytes_per_hour": baseline_slope,
            "candidate_slope_bytes_per_hour": candidate_slope,
            "baseline_slope_window_seconds": baseline_window,
            "candidate_slope_window_seconds": candidate_window,
            "baseline_final_bytes": final_base,
            "candidate_final_bytes": final_candidate,
            "candidate_final_ceiling_bytes": final_ceiling,
        }
    reasons = ([f"baseline/candidate: {reason}" for reason in errors]
               + [f"{lhs['build']}: {reason}" for reason in base_errors]
               + [f"{rhs['build']}: {reason}" for reason in cand_errors])
    result["verdict"] = "FAIL" if reasons else ("NONQUALIFYING_SMOKE_OK" if smoke else "PASS")
    result["reasons"] = reasons
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="new absent JSON summary path")
    parser.add_argument("--smoke", action="store_true")
    args = parser.parse_args()
    try:
        result = evaluate(args.baseline, args.candidate, args.smoke)
    except (OSError, ValueError, KeyError, TypeError) as error:
        message = str(error)
        if not message.startswith(("baseline: ", "candidate: ")):
            message = f"baseline/candidate: missing or invalid evidence: {message}"
        result = {"verdict": "FAIL", "reasons": [message]}
    with args.output.open("x", encoding="utf-8") as output:
        output.write(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    return 0 if result["verdict"] in ("PASS", "NONQUALIFYING_SMOKE_OK") else 1


if __name__ == "__main__":
    raise SystemExit(main())
