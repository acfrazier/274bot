#!/usr/bin/env python3
"""Run the real Grinch cells; own only the copied scratch engine process groups."""
from __future__ import annotations

import argparse
import contextlib
import difflib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request


PROTECTED_PORTS = {44594, 1080, 9898, 44694, 1180, 9988}
ANCHOR = (3185, 3440, 0)
NEAR = (3184, 3440, 0)
OUTSIDE = (3198, 3440, 0)
CASES = {
    "A": [("a-tradeable-drops", "grinch_a_tradeable_drops_live", 360),
          ("a-green-mask-selection", "grinch_a_green_mask_selection_live", 360),
          ("a-negative-candidates", "grinch_a_negative_candidates_live", 360),
          ("a-private-untradeable-drops", "grinch_a_private_untradeable_drops_live", 300)],
    "B": [("b-pumpkin-respawns", "grinch_b_pumpkin_respawns_live", 850),
          ("b-contested-pumpkin", "grinch_b_contested_pumpkin_live", 1500),
          ("b-scythe-duplicate", "grinch_b_scythe_duplicate_live", 520),
          ("b-bunny-ears-duplicate", "grinch_b_bunny_ears_duplicate_live", 520),
          ("b-yoyo-repeat", "grinch_b_yoyo_repeat_live", 500)],
    "C": [("c-deposit-28", "grinch_c_deposit_28_live", 650),
          ("c-deposit-27-preserve-item", "grinch_c_deposit_27_preserve_item_live", 650),
          ("c-inventory-blocked", "grinch_c_inventory_blocked_live", 180)],
}
OBJECTS = {
    1959: ("scripts/skill_cooking/configs/cooking_generic/cooking.obj", "pumpkin"),
    1419: ("scripts/_unpack/225/all.obj", "scythe"),
    1037: ("scripts/general_use/configs/holiday.obj", "bunnyears"),
    4079: ("scripts/holidayevents/xmas2004/configs/xmas_yoyo.obj", "xmas_yoyo"),
}


def dump(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, sort_keys=True, indent=2) + "\n", encoding="utf-8")


def digest_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(65536):
            digest.update(chunk)
    return digest.hexdigest()


def digest_inputs(root: Path, paths: list[Path]) -> str:
    digest = hashlib.sha256()
    for path in sorted(paths):
        digest.update(path.relative_to(root).as_posix().encode())
        digest.update(b"\0")
        digest.update(bytes.fromhex(digest_file(path)))
    return digest.hexdigest()


def source_identity(engine: Path) -> str:
    paths = [engine / "package.json", engine / "package-lock.json"]
    for directory in (engine / "src", engine / "tools"):
        paths.extend(path for path in directory.rglob("*") if path.is_file())
    return digest_inputs(engine, paths)


def content_identity(content: Path) -> str:
    return digest_inputs(content, [path for directory in (content / "scripts", content / "maps")
                                   for path in directory.rglob("*") if path.is_file()])


def set_respawn(text: str, alias: str) -> str:
    section = re.compile(r"(?m)^\[" + re.escape(alias) + r"\]\s*\n(?:(?!^\[)[\s\S])*")
    matches = list(section.finditer(text))
    if len(matches) != 1:
        raise ValueError(f"expected exactly one [{alias}] object definition")
    match = matches[0]
    body = match.group(0)
    if re.search(r"(?m)^respawnrate=", body):
        body = re.sub(r"(?m)^respawnrate=.*$", "respawnrate=502", body)
    else:
        body = body.rstrip("\n") + "\nrespawnrate=502\n\n"
    return text[:match.start()] + body + text[match.end():]


def add_objects(text: str, rows: list[dict]) -> str:
    marker = "==== OBJ ===="
    if text.count(marker) != 1:
        raise ValueError("JM2 fixture map must contain exactly one OBJ section")
    start = text.index(marker) + len(marker)
    end = text.find("====", start)
    if end < 0:
        end = len(text)
    additions = []
    for row in rows:
        tile = row["tile"]
        additions.append(f'{tile["plane"]} {tile["x"] % 64} {tile["z"] % 64}: '
                         f'{row["item_id"]} {row["quantity"]}')
    patched = text[:end].rstrip("\n") + "\n" + "\n".join(additions) + "\n" + text[end:]
    if patched[:start] != text[:start]:
        raise ValueError("object patch changed a non-object map section")
    return patched


def spawn(item: int, position: tuple[int, int, int]) -> dict:
    x, z, plane = position
    return {"item_id": item, "quantity": 1, "tile": {"x": x, "z": z, "plane": plane}}


def fixture_rows(slug: str) -> list[dict]:
    if slug == "b-pumpkin-respawns":
        return [spawn(1959, position) for position in (ANCHOR, NEAR, OUTSIDE)]
    item = {"b-contested-pumpkin": 1959, "b-scythe-duplicate": 1419,
            "b-bunny-ears-duplicate": 1037, "b-yoyo-repeat": 4079}[slug]
    return [spawn(item, ANCHOR)]


def reserve_ports() -> tuple[list[int], list[socket.socket]]:
    sockets: list[socket.socket] = []
    ports: list[int] = []
    try:
        for preferred in (44794, 1280, 10098):
            endpoint = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            sockets.append(endpoint)
            try:
                endpoint.bind(("127.0.0.1", preferred))
            except OSError:
                endpoint.bind(("127.0.0.1", 0))
            port = endpoint.getsockname()[1]
            if port in PROTECTED_PORTS or port in ports:
                raise RuntimeError("port allocator selected a protected or duplicate endpoint")
            endpoint.listen(1)
            ports.append(port)
        return ports, sockets
    except BaseException:
        for endpoint in sockets:
            endpoint.close()
        raise


def process_group_alive(pid: int) -> bool:
    try:
        os.killpg(pid, 0)
        return True
    except ProcessLookupError:
        return False


def stop_tree(process: subprocess.Popen) -> dict:
    # Match tools/nightly/run_nightly.py: only the group this runner created.
    if process_group_alive(process.pid):
        os.killpg(process.pid, signal.SIGTERM)
    try:
        process.wait(timeout=15)
    except subprocess.TimeoutExpired:
        pass
    if process_group_alive(process.pid):
        os.killpg(process.pid, signal.SIGKILL)
    process.wait(timeout=15)
    deadline = time.monotonic() + 15
    while process_group_alive(process.pid) and time.monotonic() < deadline:
        time.sleep(0.05)
    stopped = not process_group_alive(process.pid)
    if not stopped:
        raise RuntimeError(f"owned process group {process.pid} survived termination")
    return {"pid": process.pid, "process_group": process.pid,
            "return_code": process.returncode, "process_tree_stopped": stopped}


def run_owned(command: list[str], *, cwd: Path, env: dict[str, str], log: Path,
              timeout: int) -> None:
    with log.open("wb") as output:
        process = subprocess.Popen(command, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                   stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
        receipt = log.with_suffix(".process.json")
        identity = {"command": command, "cwd": str(cwd), "pid": process.pid,
                    "process_group": process.pid}
        try:
            dump(receipt, identity | {"process_tree_stopped": False})
            code = process.wait(timeout=timeout)
        finally:
            if process_group_alive(process.pid):
                stopped = stop_tree(process)
            else:
                stopped = {"return_code": process.returncode, "process_tree_stopped": True}
            dump(receipt, identity | stopped)
        if code:
            raise RuntimeError(f"command exited {code}: {command}; see {log}")


def copy_inputs(source: Path, destination: Path, source_content: Path) -> tuple[Path, Path]:
    engine = destination / "engine"
    content = destination / "content"
    # No hardlinks or writable symlinks back into Engine A, including node_modules.
    shutil.copytree(source, engine, symlinks=False,
                    ignore=shutil.ignore_patterns(".git", ".husky", "*.log", "db.sqlite-wal", "db.sqlite-shm"))
    shutil.copytree(source_content, content, symlinks=False,
                    ignore=shutil.ignore_patterns(".git", ".husky"))
    # SQLite's online backup gives one consistent account snapshot, including WAL.
    (engine / "db.sqlite").unlink()
    with contextlib.closing(sqlite3.connect(f"file:{source / 'db.sqlite'}?mode=ro", uri=True)) as original:
        with contextlib.closing(sqlite3.connect(engine / "db.sqlite")) as copied:
            original.backup(copied)
    if not (engine / "data/players").is_dir() or not (content / "maps").is_dir():
        raise RuntimeError("copy lacks player storage or real map inputs")
    return engine, content


def patch_content(content: Path, rows: list[dict], evidence: Path) -> dict:
    changes = []
    for item in sorted({row["item_id"] for row in rows}):
        relative, alias = OBJECTS[item]
        path = content / relative
        before = path.read_text(encoding="utf-8")
        after = set_respawn(before, alias)
        path.write_text(after, encoding="utf-8")
        changes.append({"path": relative, "before_sha256": hashlib.sha256(before.encode()).hexdigest(),
                        "after_sha256": digest_file(path), "alias": alias, "respawnrate": 502})
    by_map: dict[str, list[dict]] = {}
    for row in rows:
        tile = row["tile"]
        relative = f'maps/m{tile["x"] // 64}_{tile["z"] // 64}.jm2'
        by_map.setdefault(relative, []).append(row)
    for relative, additions in by_map.items():
        path = content / relative
        before = path.read_text(encoding="utf-8")
        after = add_objects(before, additions)
        path.write_text(after, encoding="utf-8")
        changes.append({"path": relative, "before_sha256": hashlib.sha256(before.encode()).hexdigest(),
                        "after_sha256": digest_file(path), "added_obj_rows": additions})
    raw = json.dumps(changes, sort_keys=True, separators=(",", ":")).encode()
    dump(evidence / "content-patch.json", {"changes": changes, "sha256": hashlib.sha256(raw).hexdigest()})
    return {"sha256": hashlib.sha256(raw).hexdigest(), "preserved_existing_map_rows": True,
            "real_map_packer_used": False, "real_config_packer_used": False,
            "raw_respawn_rates": {str(row["item_id"]): 502 for row in rows}, "static_spawns": rows}


def wait_ready(process: subprocess.Popen, config: dict, deadline: float) -> dict:
    url = f'http://127.0.0.1:{config["web"]["managementPort"]}/setup/config'
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("scratch engine exited before its endpoint became ready")
        try:
            with urllib.request.urlopen(url, timeout=2) as response:
                observed = json.load(response)["config"]
            # The engine endpoint returns its WorldConfig, not an inferred profile.
            if (observed["node"]["id"] != config["node"]["id"]
                    or observed["node"]["port"] != config["node"]["port"]
                    or observed["web"]["port"] != config["web"]["port"]
                    or observed["web"]["managementPort"] != config["web"]["managementPort"]
                    or observed["node"]["members"] is not True):
                raise RuntimeError("scratch setup/config did not match its chosen endpoints and members mode")
            return observed
        except (urllib.error.URLError, TimeoutError, ConnectionError):
            time.sleep(0.2)
    raise TimeoutError("scratch engine readiness exceeded 120 seconds")


def require_one_passed_test(text: str) -> None:
    if not re.search(r"^test result: ok[.] 1 passed; 0 failed; 0 ignored;", text, re.MULTILINE):
        raise RuntimeError("the exact live test did not execute and pass once")


def cargo_case(args: argparse.Namespace, name: str, timeout: int, env: dict[str, str], log: Path) -> None:
    command = [str(args.isohome), "cargo", "test", "-p", "host-play", "--features",
               "test-support,live-harness", "--test", "grinch_live", name,
               "--", "--ignored", "--exact", "--nocapture", "--test-threads=1"]
    run_owned(command, cwd=args.worktree, env=env | {"GRINCH_STDERR_LOG": str(log)},
              log=log, timeout=timeout)
    require_one_passed_test(log.read_text(encoding="utf-8", errors="replace"))


def check_disk(directory: Path, phase: str) -> None:
    free = shutil.disk_usage(directory).free
    minimum = 130 * 1024 ** 3
    dump(directory / f"disk-{phase}.json", {"free_bytes": free, "minimum_bytes": minimum})
    if free < minimum:
        raise RuntimeError(f"scratch disk below Main's 130 GiB minimum before {phase}")


def instrument_cycles(engine: Path, directory: Path) -> None:
    """Observe actual scratch phases and packet sequences without changing game logic."""
    path = engine / "src/engine/World.ts"
    original = path.read_text(encoding="utf-8")
    if "GRINCH_CYCLE" in original:
        raise ValueError("scratch World.ts was already instrumented")
    fields = """    private grinchObjectTurn: number = 0;
    private readonly grinchOutputSequences = new Map<string, number>();
    private grinchCycle = {
        cycle: 0,
        object_turn: 0,
        phase_order: [] as string[],
        active_actors: [] as {
            actor: { account_id: string; player_name: string };
            active: boolean;
            tile: { x: number; z: number; plane: number };
        }[],
        player_processing_order: [] as string[]
    };
"""
    replacements = [
        ("    currentTick: number = 0; // the current tick of the game world.\n",
         "    currentTick: number = 0; // the current tick of the game world.\n" + fields),
        ("            const drift: number = Math.max(0, start - this.nextTick);\n",
         """            const drift: number = Math.max(0, start - this.nextTick);
            this.grinchCycle.cycle = this.currentTick;
            this.grinchCycle.phase_order.length = 0;
            this.grinchCycle.player_processing_order.length = 0;
"""),
        ("            this.processPlayers();\n",
         """            this.processPlayers();
            this.grinchCycle.phase_order.push("player_processing");
"""),
        ("            this.processZones();\n",
         """            this.processZones();
            this.grinchCycle.object_turn = ++this.grinchObjectTurn;
            this.grinchCycle.phase_order.push("object_lifecycle");
"""),
        ("            this.processClientsOut();\n",
         """            this.processClientsOut();
            this.grinchCycle.phase_order.push("client_output");
            this.grinchCycle.active_actors = Array.from(this.playerLoop.all(), player => ({
                actor: { account_id: player.username, player_name: player.displayName },
                active: true,
                tile: { x: player.x, z: player.z, plane: player.level }
            }));
            console.log("GRINCH_CYCLE " + JSON.stringify(this.grinchCycle));
"""),
        ("""    private processPlayers(): void {
        const start: number = Date.now();

        for (const player of this.playerLoop.all()) {
""",
         """    private processPlayers(): void {
        const start: number = Date.now();

        for (const player of this.playerLoop.all()) {
            this.grinchCycle.player_processing_order.push(player.username);
"""),
        ("                player.updatePlayers();\n",
         """                player.updatePlayers();
                const grinchPlayerInfoSequence = (this.grinchOutputSequences.get(player.username) ?? 0) + 1;
                this.grinchOutputSequences.set(player.username, grinchPlayerInfoSequence);
"""),
        ("                player.encodeOut();\n",
         """                player.encodeOut();
                console.log("GRINCH_CLIENT_OUTPUT " + JSON.stringify({
                    cycle: this.currentTick,
                    actor: { account_id: player.username, player_name: player.displayName },
                    player_info_sequence: grinchPlayerInfoSequence
                }));
"""),
    ]
    path.write_text(patched, encoding="utf-8")
    (directory / "cycle-instrumentation.diff").write_text("".join(difflib.unified_diff(
        original.splitlines(keepends=True), patched.splitlines(keepends=True),
        fromfile="source/src/engine/World.ts", tofile="scratch/src/engine/World.ts")),
        encoding="utf-8")
    dump(directory / "cycle-instrumentation.json", {
        "path": "src/engine/World.ts",
        "source_sha256": hashlib.sha256(original.encode()).hexdigest(),
        "instrumented_sha256": digest_file(path),
        "scope": "scratch_copy_log_only",
        "cycle_prefix": "GRINCH_CYCLE ",
        "client_output_prefix": "GRINCH_CLIENT_OUTPUT ",
        "player_info_sequence_origin": 1,
        "object_turn_origin": "first_completed_processZones_is_1",
        "phase_hook_count": len(replacements),
    })


def instrument_take_receipts(engine: Path, directory: Path) -> None:
    """Bind each actual server object-option result to its input-processing cycle."""
    path = engine / "src/network/game/client/handler/OpObjHandler.ts"
    original = path.read_text(encoding="utf-8")
    if "GRINCH_TAKE_RECEIVE" in original:
        raise ValueError("scratch OpObjHandler.ts was already instrumented")
    anchor = "        const { x, z, obj: objId } = message;\n"
    rejected = "            return false;\n"
    accepted = "        return true;\n"
    declaration = "export default class OpObjHandler extends ClientGameMessageHandler<OpObj> {\n"
    if (original.count(anchor) != 1 or original.count(rejected) != 4
            or original.count(accepted) != 1 or original.count(declaration) != 1):
        raise ValueError("scratch OpObjHandler.ts return anchors changed")
    receipt = """        const grinchReceipt = (accepted: boolean, reason: string, action: string | null = null): void => {
            console.log("GRINCH_TAKE_RECEIVE " + JSON.stringify({
                receipt_sequence: ++grinchTakeReceiveSequence,
                cycle: World.currentTick,
                actor: { account_id: player.username, player_name: player.displayName },
                item_id: objId,
                tile: { x, z, plane: player.level },
                option: message.op,
                action,
                accepted,
                reason
            }));
        };
"""
    patched = original.replace(declaration, "let grinchTakeReceiveSequence = 0;\n\n" + declaration, 1)
    patched = patched.replace(anchor, anchor + receipt, 1)
    patched = patched.replace(rejected,
                              '            grinchReceipt(false, "handler_rejected");\n' + rejected)
    patched = patched.replace(accepted,
                              '        grinchReceipt(true, "interaction_armed", type.op[message.op - 1] ?? null);\n' + accepted,
                              1)
    (directory / "take-instrumentation.diff").write_text("".join(difflib.unified_diff(
        original.splitlines(keepends=True), patched.splitlines(keepends=True),
        fromfile="source/src/network/game/client/handler/OpObjHandler.ts",
        tofile="scratch/src/network/game/client/handler/OpObjHandler.ts")),
        encoding="utf-8")
    path.write_text(patched, encoding="utf-8")
    dump(directory / "take-instrumentation.json", {
        "path": "src/network/game/client/handler/OpObjHandler.ts",
        "source_sha256": hashlib.sha256(original.encode()).hexdigest(),
        "instrumented_sha256": digest_file(path),
        "scope": "scratch_copy_log_only",
        "prefix": "GRINCH_TAKE_RECEIVE ",
        "receipt_sequence_origin": 1,
        "rejected_return_hook_count": 4,
        "armed_return_hook_count": 1,
    })


def command_stop_receipts(directory: Path) -> list[dict]:
    receipts = []
    for name in ("pack", "capture"):
        if not (directory / f"{name}.log").is_file():
            continue
        path = directory / f"{name}.process.json"
        stopped = path.is_file() and json.loads(path.read_text()).get("process_tree_stopped") is True
        receipts.append({"command": name, "receipt": str(path), "process_tree_stopped": stopped})
    return receipts


def run_scratch(args: argparse.Namespace, case: tuple[str, str, int], base_env: dict[str, str],
                original_identity: str, source_content: Path) -> None:
    directory = args.evidence_dir / "scratch" / case[0]
    try:
        capture_scratch(args, case, base_env, original_identity, source_content)
    finally:
        manifest = directory / "engine-evidence.json"
        authority = json.loads(manifest.read_text())["scratch"] if manifest.is_file() else {}
        engine_stopped = (authority.get("start_count") == 0
                          or authority.get("process_tree_stopped") is True
                          or (not manifest.is_file() and not (directory / "server.log").is_file()))
        commands = command_stop_receipts(directory)
        stopped = engine_stopped and all(command["process_tree_stopped"] for command in commands)
        if stopped:
            for name in ("engine", "content"):
                copied = directory / name
                if copied.is_dir():
                    shutil.rmtree(copied)
        if directory.is_dir():
            dump(directory / "copy-cleanup.json", {
                "process_tree_stopped_or_not_started": stopped,
                "engine_process_tree_stopped_or_not_started": engine_stopped,
                "owned_command_stop_receipts": commands,
                "engine_copy_removed": not (directory / "engine").exists(),
                "content_copy_removed": not (directory / "content").exists(),
            })
        if not stopped:
            raise RuntimeError("scratch process stop is unproven; retaining engine/content copies")


def capture_scratch(args: argparse.Namespace, case: tuple[str, str, int], base_env: dict[str, str],
                    original_identity: str, source_content: Path) -> None:
    slug, test, timeout = case
    directory = args.evidence_dir / "scratch" / slug
    directory.mkdir(parents=True, exist_ok=False)
    check_disk(directory, "copy")
    engine, content = copy_inputs(args.engine_dir, directory, source_content)
    # copytree dereferences .bin symlinks; launch the package's original path
    # so TSX's relative module imports still resolve inside the isolated copy.
    launcher = engine / (args.engine_dir / "node_modules/.bin/tsx").resolve().relative_to(
        args.engine_dir.resolve())
    if slug.startswith("b-"):
        patch = patch_content(content, fixture_rows(slug), directory)
    else:
        patch = None
    source_content_identity = content_identity(source_content)
    copied_content_identity = content_identity(content)
    if not slug.startswith("b-") and copied_content_identity != source_content_identity:
        raise RuntimeError("unpatched scratch content differs from source content")
    instrument_cycles(engine, directory)
    instrument_take_receipts(engine, directory)
    config_path = engine / "data/config/world.json"
    config = json.loads(config_path.read_text())
    ports, reservations = reserve_ports()
    config["node"].update(id=ports[0], port=ports[0], members=True, debugProfile=True)
    config["web"].update(port=ports[1], managementPort=ports[2])
    config["build"].update(srcDir="../content", startup=False, liveReload=False)
    config["easyStartup"] = False
    config["login"]["enabled"] = False
    config["friend"]["enabled"] = False
    config["logger"]["enabled"] = False
    dump(config_path, config)
    lifecycle = directory / "lifecycle.json"
    manifest = directory / "engine-evidence.json"
    env = base_env | {"ENGINE_DIR": str(engine), "WORLD_ENGINE_DIR": str(engine),
                      "GRINCH_ENGINE_DIR": str(engine), "GRINCH_GAME_PORT": str(ports[0]),
                      "GRINCH_HTTP_PORT": str(ports[1]), "GRINCH_MANAGEMENT_PORT": str(ports[2]),
                      "GRINCH_ENDPOINT_ID": str(ports[0]), "GRINCH_MEMBERS": "1",
                      "GRINCH_ENGINE_EVIDENCE": str(manifest),
                      "GRINCH_SERVER_LOG": str(directory / "server.log"),
                      "GRINCH_HOST_LOG": str(directory / "host.log")}
    authority = {"request_id": args.request_id + "/" + slug, "scratch_id": str(directory),
                 "source_engine_identity": original_identity,
                 "source_content_identity": source_content_identity,
                 "engine_identity": source_identity(engine),
                 "content_identity": copied_content_identity, "world_identity": digest_file(config_path),
                 "account_store_identity": digest_inputs(engine, [engine / "db.sqlite"] +
                     [path for path in (engine / "data/players").rglob("*") if path.is_file()]),
                 "ports": {name: {"host": "127.0.0.1", "port": port}
                           for name, port in zip(("game", "web", "management"), ports)},
                 "members_mode": True, "copied_engine_content_map_inputs_and_account_storage": True,
                 "respawn_phase_convention": "first_object_turn_in_take_cycle_and_output_after_lifecycle",
                 "content_patch": patch, "server_tick_ms": 600, "start_count": 0, "stop_count": 0,
                 "process_tree_stopped": False, "engine_a_touched": False, "engine_q_touched": False}
    authority["ports"].update(probed_free_before_start=True, selected_before_login=True)
    process = None
    try:
        command = [str(launcher), "tools/pack/Build.ts"]
        run_owned(command, cwd=engine, env=env, log=directory / "pack.log", timeout=900)
        if not slug.startswith("b-") and content_identity(content) != source_content_identity:
            raise RuntimeError("unpatched scratch content changed during pack")
        if patch is not None:
            patch.update(real_map_packer_used=True, real_config_packer_used=True)
        authority["content_identity"] = content_identity(content)
        for endpoint in reservations:
            endpoint.close()
        reservations.clear()
        with (directory / "server.log").open("wb") as output:
            process = subprocess.Popen([str(launcher), "src/app.ts"],
                                       cwd=engine, env=env, stdin=subprocess.DEVNULL,
                                       stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
            dump(manifest, {"scratch": authority})
            dump(lifecycle, {"pid": process.pid, "process_group": process.pid, "start_count": 1,
                             "stop_count": 0, "process_tree_stopped": False})
            observed = wait_ready(process, config, time.monotonic() + 120)
            dump(directory / "observed-config.json", observed)
            cargo_case(args, test, timeout, env, directory / "capture.log")
    finally:
        for endpoint in reservations:
            endpoint.close()
        if process is not None:
            stopped = stop_tree(process)
            authority["stop_count"] = 1
            authority["process_tree_stopped"] = stopped["process_tree_stopped"]
        dump(manifest, {"scratch": authority})


def run(args: argparse.Namespace) -> None:
    for name in ("engine_dir", "catalog_root", "nav_pack", "evidence_dir", "worktree", "isohome"):
        path = getattr(args, name)
        if not path.is_absolute():
            raise ValueError(f"--{name.replace('_', '-')} must be absolute")
    args.evidence_dir.parent.mkdir(parents=True, exist_ok=True)
    if shutil.disk_usage(args.evidence_dir.parent).free < 130 * 1024 ** 3:
        raise RuntimeError("scratch disk has less than Main's required 130 GiB free")
    args.evidence_dir.mkdir(parents=True, exist_ok=False)
    source_config = json.loads((args.engine_dir / "data/config/world.json").read_text())
    source_content = (args.engine_dir / source_config["build"]["srcDir"]).resolve()
    original_identity = source_identity(args.engine_dir)
    env = os.environ.copy() | {"LIVE": "1", "BOT_CPU": "1", "BOT_DEBUG": "1",
        "BOT_LIVE_NAME_PREFIX": "gr25", "GRINCH_ENGINE_DIR": str(args.engine_dir),
        "GRINCH_CATALOG_ROOT": str(args.catalog_root), "GRINCH_NAV_PACK": str(args.nav_pack),
        "LIVE_EVIDENCE_DIR": str(args.evidence_dir), "NAV_PACK": str(args.nav_pack),
        "WORLD_NAV_PACK": str(args.nav_pack), "NAV_ARRIVAL_PACK": str(args.nav_pack),
        "BOT_NAV_BUILD": "skip", "ENGINE_DIR": str(args.engine_dir),
        "WORLD_ENGINE_DIR": str(args.engine_dir)}
    if any(name in os.environ for name in ("QUESTER_TICK_MS", "CARGO_TARGET_DIR")):
        raise ValueError("unset QUESTER_TICK_MS and CARGO_TARGET_DIR before the Grinch runner")
    a_manifest = args.evidence_dir / "engine-a-evidence.json"
    engine_a = {"kind": "engine_a", "engine_identity": original_identity,
        "content_identity": content_identity(source_content), "server_tick_ms": 600,
        "content_unchanged": True, "controlled_only_by_orchestrator": True}
    dump(a_manifest, engine_a)
    for cell in args.cells:
        try:
            for case in CASES[cell]:
                slug, test, timeout = case
                run_scratch(args, case, env, original_identity, source_content)
        finally:
            observed_source = source_identity(args.engine_dir)
            observed_content = content_identity(source_content)
            unchanged = (observed_source == original_identity
                         and observed_content == engine_a["content_identity"])
            dump(args.evidence_dir / f"cell-{cell.lower()}-source-integrity.json", {
                "source_engine_before": original_identity, "source_engine_after": observed_source,
                "source_content_before": engine_a["content_identity"],
                "source_content_after": observed_content, "unchanged": unchanged})
            engine_a["content_unchanged"] = unchanged
            dump(a_manifest, engine_a)
            if not unchanged:
                raise RuntimeError("Engine A source or content changed during the live cells")
        cargo_case(args, f"grinch_{cell.lower()}_cell_oracle_live", 120, env,
                   args.evidence_dir / f"cell-{cell.lower()}-oracle.log")
    dump(args.evidence_dir / "runner-result.json", {"status": "PASS", "cells": args.cells,
        "engine_a_started_or_stopped": False, "scratch_process_trees_stopped": True})


def self_test(engine: Path | None = None) -> None:
    # Local input/process checks are not live evidence or an engine qualification.
    original = "[first]\nname=First\n\n[pumpkin]\nname=Pumpkin\n\n[last]\nname=Last\n"
    patched = set_respawn(original, "pumpkin")
    assert "[first]\nname=First" in patched and "[last]\nname=Last" in patched
    assert patched.count("respawnrate=502") == 1
    assert set_respawn(patched, "pumpkin") == patched
    map_input = "==== MAP ====\n0 0 0: 1\n\n==== OBJ ====\n0 1 2: 995 4\n"
    rows = fixture_rows("b-pumpkin-respawns")
    changed = add_objects(map_input, rows)
    assert changed.startswith(map_input) and len(rows) == 3
    assert "0 49 48: 1959 1" in changed and "0 62 48: 1959 1" in changed
    try:
        set_respawn(original, "missing")
    except ValueError:
        pass
    else:
        raise AssertionError("an absent object section was accepted")
    ports, reservations = reserve_ports()
    try:
        assert len(set(ports)) == 3 and not set(ports) & PROTECTED_PORTS
    finally:
        for endpoint in reservations:
            endpoint.close()
    require_one_passed_test("test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured;")
    for invalid in ("test result: ok. 0 passed; 0 failed; 0 ignored;",
                    "test result: ok. 1 passed; 0 failed; 1 ignored;",
                    "test result: FAILED. 0 passed; 1 failed; 0 ignored;"):
        try:
            require_one_passed_test(invalid)
        except RuntimeError:
            pass
        else:
            raise AssertionError("an empty, ignored or failed live-test result was accepted")
    with tempfile.TemporaryDirectory(prefix="grinch-runner-self-test-") as temporary:
        directory = Path(temporary)
        for name, source, timeout, expected_error in [
                ("success", "print('owned process success')", 5, None),
                ("failure", "raise SystemExit(7)", 5, RuntimeError),
                ("timeout", "import time; time.sleep(2)", 1, subprocess.TimeoutExpired)]:
            log = directory / f"{name}.log"
            try:
                run_owned([sys.executable, "-c", source], cwd=directory,
                          env=os.environ.copy(), log=log, timeout=timeout)
            except (RuntimeError, subprocess.TimeoutExpired) as error:
                if expected_error is None or not isinstance(error, expected_error):
                    raise
            else:
                assert expected_error is None, f"{name} did not fail as required"
            receipt = json.loads(log.with_suffix(".process.json").read_text())
            assert receipt["process_tree_stopped"] is True
            assert receipt["pid"] == receipt["process_group"]
            assert not process_group_alive(receipt["process_group"])
        # Missing command-stop receipts must retain copies even before server start.
        stop_fixture = directory / "command-stop-fixture"
        stop_fixture.mkdir()
        (stop_fixture / "pack.log").write_text("synthetic missing-receipt input\n")
        assert not all(entry["process_tree_stopped"] for entry in command_stop_receipts(stop_fixture))
        shutil.copyfile(directory / "success.process.json", stop_fixture / "pack.process.json")
        assert all(entry["process_tree_stopped"] for entry in command_stop_receipts(stop_fixture))
        (stop_fixture / "capture.log").write_text("synthetic missing-receipt input\n")
        assert not all(entry["process_tree_stopped"] for entry in command_stop_receipts(stop_fixture))
        if engine is not None:
            if not engine.is_absolute():
                raise ValueError("--engine-dir must be absolute for source compatibility checks")
            copied = directory / "engine"
            paths = ("src/engine/World.ts", "src/network/game/client/handler/OpObjHandler.ts")
            source_digests = {}
            for relative in paths:
                source = engine / relative
                source_digests[relative] = digest_file(source)
                destination = copied / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(source, destination)
            instrument_cycles(copied, directory)
            instrument_take_receipts(copied, directory)
            for relative in paths:
                assert digest_file(engine / relative) == source_digests[relative]
                destination = copied / relative
                assert digest_file(destination) != source_digests[relative]
                subprocess.run([str(engine / "node_modules/.bin/esbuild"), str(destination),
                                "--format=esm", "--log-level=error",
                                f"--outfile={directory / (destination.stem + '.mjs')}"],
                               check=True, stdin=subprocess.DEVNULL)
            for instrument in (instrument_cycles, instrument_take_receipts):
                try:
                    instrument(copied, directory)
                except ValueError:
                    pass
                else:
                    raise AssertionError("duplicate source instrumentation was accepted")
    print("PASS: Grinch runner patch, port, exact-test-cardinality and owned-process checks (non-live)")


def interrupted(signum: int, _frame: object) -> None:
    raise KeyboardInterrupt(f"received signal {signum}; stopping owned process groups")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--worktree", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--isohome", type=Path)
    parser.add_argument("--engine-dir", type=Path)
    parser.add_argument("--catalog-root", type=Path)
    parser.add_argument("--nav-pack", type=Path)
    parser.add_argument("--evidence-dir", type=Path)
    parser.add_argument("--request-id", default="GRINCH-1020")
    parser.add_argument("--cells", nargs="+", choices=tuple(CASES), default=["A", "B", "C"])
    args = parser.parse_args()
    if args.self_test:
        self_test(args.engine_dir)
        return
    for name in ("isohome", "engine_dir", "catalog_root", "nav_pack", "evidence_dir"):
        if getattr(args, name) is None:
            parser.error(f"--{name.replace('_', '-')} is required")
    signal.signal(signal.SIGTERM, interrupted)
    try:
        run(args)
    except (Exception, KeyboardInterrupt) as error:
        if args.evidence_dir.is_dir():
            dump(args.evidence_dir / "runner-result.json", {"status": "FAIL", "error": str(error)})
        print(f"FAIL: {error}", file=sys.stderr)
        raise SystemExit(1) from error


if __name__ == "__main__":
    main()
