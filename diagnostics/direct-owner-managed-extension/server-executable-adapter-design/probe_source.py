#!/usr/bin/env python3
"""Source-only probe of server executable identity paths. No native/SSH/sudo."""
from __future__ import annotations

import hashlib
import json
import pathlib
import re
import tarfile

ROOT = pathlib.Path(__file__).resolve().parents[3]
OUT = pathlib.Path(__file__).resolve().parent
CONTROLLER = ROOT / "docs/memory/run_managed_cell.py"
SAMPLER = ROOT / "docs/memory/server_resources.py"
DIAGNOSTIC = ROOT / "docs/memory/run_diagnostic.py"
PLAN = ROOT / "docs/memory/direct-per-bot-owner-capture-plan.md"
QUESTION = ROOT / "docs/memory/direct-owner-server-executable-access-question.md"
FAILURE = (
    ROOT
    / "diagnostics/direct-owner-managed-extension/server-executable-access"
    / "actual-controller-failure.json"
)
TAR = (
    ROOT
    / "diagnostics/direct-owner-native-preparation/root-runtime-preparation-01"
    / "frozen-direct-owner-source.tar.gz"
)
EXPECTED_TAR = "2c36d254c37c5ed56a55f78728357296d62be602f8653ed9ec041cd37d099d5d"
FROZEN_CONTROLLER = "direct-owner-source/docs/memory/run_managed_cell.py"
NEEDLES = (
    "linux_executable_basename",
    "os.readlink",
    "/proc/{pid}/exe",
    "executable_basename",
    "server_executable_basename",
    "probe_direct_server",
    "sudo",
    "readlink",
    "ExecStart",
    "AmbientCapabilities",
)


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: pathlib.Path) -> str:
    return sha256_bytes(path.read_bytes())


def hits(text: str, needle: str) -> list[dict]:
    found = []
    for i, line in enumerate(text.splitlines(), 1):
        if needle in line:
            found.append({"line": i, "text": line.rstrip()[:240]})
    return found


def function_span(text: str, name: str) -> dict:
    lines = text.splitlines()
    start = None
    for i, line in enumerate(lines):
        if line.startswith("def " + name + "("):
            start = i
            break
    if start is None:
        raise SystemExit("missing function " + name)
    end = len(lines)
    for j in range(start + 1, len(lines)):
        if lines[j].startswith("def ") or lines[j].startswith("class "):
            end = j
            break
    body = "\n".join(lines[start:end]).rstrip() + "\n"
    return {
        "name": name,
        "start_line": start + 1,
        "end_line": end,
        "sha256": sha256_bytes(body.encode()),
        "text": body,
    }


ABS_PATH = re.compile(r"^(/[^/\x00\n]+)+$")
MAX_STDOUT = 4096


def designed_validate_pid(pid: object) -> int:
    if type(pid) is not int or isinstance(pid, bool) or pid <= 0:
        raise ValueError("pid")
    return pid


def designed_argv(pid: object) -> list[str]:
    pid = designed_validate_pid(pid)
    return [
        "/usr/bin/sudo",
        "-n",
        "--",
        "/usr/bin/readlink",
        "-n",
        "/proc/" + str(pid) + "/exe",
    ]


def designed_parse(raw: bytes) -> str:
    if not isinstance(raw, (bytes, bytearray)) or len(raw) == 0 or len(raw) > MAX_STDOUT:
        raise ValueError("size")
    if b"\x00" in raw or b"\n" in raw or b"\r" in raw or b" " in raw or b"\t" in raw:
        raise ValueError("control")
    try:
        text = bytes(raw).decode("utf-8")
    except UnicodeDecodeError as exc:
        raise ValueError("utf8") from exc
    if text.endswith(" (deleted)"):
        raise ValueError("deleted")
    if ABS_PATH.fullmatch(text) is None:
        raise ValueError("path")
    parts = text.split("/")
    if parts[0] != "" or any(part in ("", ".", "..") for part in parts[1:]):
        raise ValueError("components")
    name = pathlib.PurePosixPath(text).name
    if not name or name in (".", "..") or "/" in name or "\\" in name:
        raise ValueError("basename")
    return name


def parser_cases() -> list[dict]:
    cases = [
        ("ok_usr", b"/usr/local/bin/game-server", True, "game-server"),
        ("ok_nested", b"/opt/acme/bin/server", True, "server"),
        ("empty", b"", False, None),
        ("newline", b"/usr/bin/game\n", False, None),
        ("relative", b"usr/bin/game", False, None),
        ("dotdot", b"/usr/../bin/game", False, None),
        ("double_slash", b"/usr//bin/game", False, None),
        ("nul", b"/usr/bin/game\x00", False, None),
        ("deleted", b"/usr/bin/game (deleted)", False, None),
        ("comm_like", b"game-server", False, None),
        ("execstart_like", b"/usr/bin/game --config /etc/x", False, None),
        ("too_long", b"/" + b"a" * 4096, False, None),
        ("root_only", b"/", False, None),
        ("space_name", b"/usr/bin/game server", False, None),
    ]
    results = []
    for name, raw, ok, expected in cases:
        try:
            got = designed_parse(raw)
            results.append(
                {
                    "name": name,
                    "accepted": True,
                    "value": got,
                    "expect_ok": ok,
                    "pass": ok and got == expected,
                }
            )
        except ValueError as exc:
            results.append(
                {
                    "name": name,
                    "accepted": False,
                    "error": str(exc),
                    "expect_ok": ok,
                    "pass": (not ok),
                }
            )
    return results


def argv_cases() -> list[dict]:
    cases = [
        ("pid_726", 726, True, "/proc/726/exe"),
        ("pid_1", 1, True, "/proc/1/exe"),
        ("zero", 0, False, None),
        ("negative", -1, False, None),
        ("bool", True, False, None),
        ("str", "726", False, None),
        ("float", 726.0, False, None),
    ]
    results = []
    for name, pid, ok, tail in cases:
        try:
            argv = designed_argv(pid)
            results.append(
                {
                    "name": name,
                    "accepted": True,
                    "argv": argv,
                    "expect_ok": ok,
                    "pass": (
                        ok
                        and argv[:5]
                        == ["/usr/bin/sudo", "-n", "--", "/usr/bin/readlink", "-n"]
                        and argv[5] == tail
                        and "shell" not in argv
                        and "-S" not in argv
                        and "-E" not in argv
                    ),
                }
            )
        except (TypeError, ValueError):
            results.append({"name": name, "accepted": False, "expect_ok": ok, "pass": (not ok)})
    return results


def main() -> int:
    controller = CONTROLLER.read_text()
    sampler = SAMPLER.read_text()
    diagnostic = DIAGNOSTIC.read_text()
    plan = PLAN.read_text()
    question = QUESTION.read_text()
    failure = json.loads(FAILURE.read_text())
    tar_sha = sha256_file(TAR)
    if tar_sha != EXPECTED_TAR:
        raise SystemExit("frozen tar digest mismatch")

    frozen_controller = None
    with tarfile.open(TAR, "r:gz") as archive:
        member = archive.extractfile(FROZEN_CONTROLLER)
        if member is None:
            raise SystemExit("frozen controller missing")
        frozen_bytes = member.read()
        frozen_controller = {
            "path": FROZEN_CONTROLLER,
            "sha256": sha256_bytes(frozen_bytes),
            "bytes": len(frozen_bytes),
            "has_linux_executable_basename": b"def linux_executable_basename" in frozen_bytes,
            "same_as_6f2b410_checkout": sha256_bytes(frozen_bytes) == sha256_file(CONTROLLER),
        }

    exe_fn = function_span(controller, "linux_executable_basename")
    probe_fn = function_span(controller, "probe_direct_server")
    basename_fn = function_span(controller, "_basename")
    (OUT / "excerpt-linux-executable-basename.py.txt").write_text(exe_fn["text"])
    (OUT / "excerpt-probe-direct-server.py.txt").write_text(probe_fn["text"])
    (OUT / "excerpt-basename.py.txt").write_text(basename_fn["text"])

    preflight_calls = []
    for i, line in enumerate(controller.splitlines(), 1):
        if "executable_basename(" in line or "executable_basename=" in line:
            preflight_calls.append({"line": i, "text": line.strip()[:240]})

    parse_results = parser_cases()
    argv_results = argv_cases()
    result = {
        "schema": "server-executable-adapter-source-probe-v1",
        "native_commands": False,
        "ssh": False,
        "sudo_invoked": False,
        "permission_changes": False,
        "files": {
            "run_managed_cell.py": {
                "git_blob": "2c00e549317bbf7cce2222047e38470f0dde1e69",
                "sha256": sha256_file(CONTROLLER),
                "bytes": CONTROLLER.stat().st_size,
                "matches_6f2b410_blob": True,
            },
            "server_resources.py": {
                "sha256": sha256_file(SAMPLER),
                "bytes": SAMPLER.stat().st_size,
            },
            "run_diagnostic.py": {
                "sha256": sha256_file(DIAGNOSTIC),
                "bytes": DIAGNOSTIC.stat().st_size,
            },
            "plan": {"sha256": sha256_file(PLAN)},
            "question": {"sha256": sha256_file(QUESTION)},
            "failure_json": {"sha256": sha256_file(FAILURE)},
            "frozen_tar_sha256": tar_sha,
        },
        "frozen_controller": frozen_controller,
        "functions": {
            "linux_executable_basename": {
                "start_line": exe_fn["start_line"],
                "end_line": exe_fn["end_line"],
                "sha256": exe_fn["sha256"],
            },
            "probe_direct_server": {
                "start_line": probe_fn["start_line"],
                "end_line": probe_fn["end_line"],
                "sha256": probe_fn["sha256"],
            },
            "_basename": {
                "start_line": basename_fn["start_line"],
                "end_line": basename_fn["end_line"],
                "sha256": basename_fn["sha256"],
            },
        },
        "searches": {
            "controller": {needle: hits(controller, needle) for needle in NEEDLES},
            "sampler": {
                needle: hits(sampler, needle)
                for needle in ("/proc/", "exe", "readlink", "sudo", "comm")
            },
            "diagnostic_exe": hits(diagnostic, "os.readlink"),
            "plan_permission": hits(plan, "permission"),
            "plan_restart": hits(plan, "server restart"),
        },
        "executable_basename_sites": preflight_calls,
        "failure": {
            "server_pid": failure.get("server_pid"),
            "euid": failure.get("euid"),
            "start_identity": (failure.get("sample") or {}).get("start_identity"),
            "error_type": (failure.get("error") or {}).get("type"),
            "error_message": (failure.get("error") or {}).get("message"),
            "error_cause": (failure.get("error") or {}).get("cause"),
            "unit": failure.get("unit"),
        },
        "observations_from_source": {
            "controller_uses_os_readlink_only_for_server_exe": True,
            "controller_has_no_sudo": not any(
                "sudo" in line.lower() for line in controller.splitlines()
            ),
            "sampler_reads_stat_not_exe": (
                "def _linux_sample" in sampler
                and '(proc / "stat")' in sampler
                and "exe" not in sampler
            ),
            "current_probe_is_tcp_loopback_43594": "DIRECT_SERVER_PORT = 43594" in controller
            and "socket.create_connection(('127.0.0.1', DIRECT_SERVER_PORT)" in controller,
            "preflight_calls_executable_basename_twice": sum(
                1 for row in preflight_calls if row["text"].startswith("executable_basename(")
                or "executable_basename(int(" in row["text"]
            )
            >= 2,
            "diagnostic_exe_is_own_children_not_server": "_linux_direct_children" in diagnostic,
        },
        "designed_parser_cases": parse_results,
        "designed_argv_cases": argv_results,
        "parser_all_pass": all(row["pass"] for row in parse_results),
        "argv_all_pass": all(row["pass"] for row in argv_results),
        "question_forbids_comm_execstart": "Service ExecStart" in question
        and "not substituted as live image proof" in question,
        "plan_5_3_forbids_permission_change": "No cache rebuild, account reset, server restart, installation or permission"
        in plan,
    }
    (OUT / "probe-result.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    if not result["parser_all_pass"] or not result["argv_all_pass"]:
        raise SystemExit("designed contract cases failed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
