#!/usr/bin/env python3
"""Run one T1.4 local WSS cohort with a fresh HOME and a private test CA.

The contract values live only in T1.4/soak-config.json. This runner resolves
per-run paths/ports and records the effective config; it never changes the OS
trust store, the operator's HOME, the engine files, or the source config.
"""

import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import selectors
import shutil
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent.parent
SOURCE_CONFIG = ROOT / "T1.4" / "soak-config.json"


def openssl(*args: str) -> None:
    subprocess.run(["openssl", *args], check=True, capture_output=True, text=True)

def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def local_engine_public_modulus(engine_dir: Path) -> tuple[str, str]:
    source = engine_dir / "data/config/private.pem"
    digest = hashlib.sha256(source.read_bytes()).hexdigest()
    result = subprocess.run(
        ["openssl", "rsa", "-in", str(source), "-modulus", "-noout"],
        check=True, capture_output=True, text=True,
    )
    label, separator, hex_value = result.stdout.strip().partition("=")
    if label != "Modulus" or not separator or not hex_value:
        raise ValueError(f"could not extract local Engine RSA modulus from {source}")
    modulus = str(int(hex_value, 16))
    if len(modulus) < 250:
        raise ValueError(f"local Engine RSA modulus is too short: {source}")
    return modulus, digest


def make_ca(home: Path) -> tuple[Path, Path, Path]:
    pki = home / "pki"
    pki.mkdir(mode=0o700)
    ca = pki / "ca.pem"
    server = pki / "server.pem"
    key = pki / "server.key"
    ca_key = pki / "ca.key"
    request = pki / "server.csr"
    extensions = pki / "server.ext"
    extensions.write_text("subjectAltName=IP:127.0.0.1\nextendedKeyUsage=serverAuth\n", encoding="ascii")
    openssl(
        "req", "-x509", "-newkey", "rsa:2048", "-sha256", "-days", "1", "-nodes",
        "-subj", "/CN=T1.4 local test CA", "-keyout", str(ca_key), "-out", str(ca),
        "-addext", "basicConstraints=critical,CA:TRUE",
    )
    openssl(
        "req", "-newkey", "rsa:2048", "-sha256", "-nodes",
        "-subj", "/CN=localhost", "-keyout", str(key), "-out", str(request),
    )
    openssl(
        "x509", "-req", "-in", str(request), "-CA", str(ca),
        "-CAkey", str(ca_key), "-CAcreateserial", "-out", str(server),
        "-days", "1", "-sha256", "-extfile", str(extensions),
    )
    os.chmod(ca_key, 0o600)
    os.chmod(key, 0o600)
    return ca, server, key


def write_roster(home: Path, config: dict, port: int) -> Path:
    profile = config["profile"]
    roster = {
        "schema_version": 1,
        "servers": [{
            "name": profile["name"], "revision": 289, "transport": "wss",
            "worlds": [{
                "number": 1, "host": profile["host"], "port": port,
                "node_id": profile["node_id"], "asset_host": profile["asset_host"],
                "asset_port": port,
            }],
            "login_key": {"engine_dir": profile["engine_dir"]},
            "vault": "vault-t14-local-soak", "members": None,
            "allow_plaintext_offhost": False,
        }],
    }
    bot_dir = home / ".274bot"
    bot_dir.mkdir(mode=0o700)
    path = bot_dir / "servers.json"
    path.write_text(json.dumps(roster, indent=2) + "\n", encoding="utf-8")
    return path


def ready_proxy(process: subprocess.Popen, ready: Path) -> dict:
    assert process.stdout is not None
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    try:
        if not selector.select(timeout=30):
            raise RuntimeError("proxy did not publish readiness within 30 seconds")
        line = process.stdout.readline().strip()
        if "ready" not in line.lower() or not ready.is_file():
            raise RuntimeError(f"proxy failed before readiness (status {process.poll()}): {line}")
        receipt = json.loads(ready.read_text(encoding="utf-8"))
        return receipt
    finally:
        selector.close()


def sample_cohort(command: list[str], repo: Path, env: dict, output: Path, interval: int) -> int:
    """Sample the live test process from outside it; never substitute cargo RSS."""
    pid_file = output / "pid.json"
    with (output / "cohort.log").open("w", encoding="utf-8") as log:
        cargo = subprocess.Popen(command, cwd=repo, env=env, stdout=log,
                                 stderr=subprocess.STDOUT, start_new_session=True)
        try:
            while not pid_file.exists():
                if cargo.poll() is not None:
                    raise RuntimeError(f"cohort exited {cargo.returncode} before publishing its PID")
                try:
                    cargo.wait(timeout=1)
                except subprocess.TimeoutExpired:
                    pass
            pid = json.loads(pid_file.read_text(encoding="utf-8"))["pid"]
            if not isinstance(pid, int) or pid <= 0:
                raise RuntimeError(f"invalid cohort PID receipt: {pid!r}")
            proc = Path("/proc") / str(pid)
            if not proc.is_dir():
                raise RuntimeError(f"cohort process {pid} vanished before first sample")
            first = time.monotonic()
            with (output / "process-samples.jsonl").open("x", encoding="utf-8") as samples:
                while proc.is_dir():
                    try:
                        status = (proc / "status").read_text(encoding="utf-8")
                        threads = []
                        for task in sorted((proc / "task").iterdir()):
                            try:
                                threads.append({
                                    "tid": int(task.name),
                                    "comm": (task / "comm").read_text(encoding="utf-8").strip(),
                                })
                            except FileNotFoundError:
                                # A task which completed during enumeration no
                                # longer belongs in this live-task sample.
                                continue
                    except FileNotFoundError:
                        if cargo.poll() is not None:
                            break
                        raise RuntimeError(f"cohort {pid} disappeared while cargo is still running")
                    rss = next((int(line.split()[1]) * 1024 for line in status.splitlines()
                                if line.startswith("VmRSS:")), None)
                    if rss is None:
                        raise RuntimeError(f"cohort {pid} has no VmRSS in /proc status")
                    value = {
                        "elapsed_seconds": round(time.monotonic() - first, 3),
                        "sample_monotonic_ns": time.monotonic_ns(),
                        "rss_bytes": rss,
                        "task_count": len(threads),
                        "threads": threads,
                        "pid": pid,
                    }
                    samples.write(json.dumps(value) + "\n")
                    samples.flush()
                    if cargo.poll() is not None:
                        break
                    try:
                        cargo.wait(timeout=interval)
                    except subprocess.TimeoutExpired:
                        pass
            return cargo.wait()
        finally:
            if cargo.poll() is None:
                os.killpg(cargo.pid, signal.SIGTERM)
                try:
                    cargo.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    os.killpg(cargo.pid, signal.SIGKILL)
                    cargo.wait()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=SOURCE_CONFIG)
    parser.add_argument("--output", type=Path, required=True, help="new absent per-run evidence directory")
    parser.add_argument("--seed", type=int, required=True, help="same value for dev baseline and candidate")
    run_mode = parser.add_mutually_exclusive_group()
    run_mode.add_argument("--smoke", action="store_true", help="nonqualifying two-slot infrastructure smoke")
    run_mode.add_argument("--diagnostic-seconds", type=int,
                          help="nonqualifying full-cohort window from the actual phase-zero Start")
    parser.add_argument("--engine-dir", type=Path, help="diagnostic/smoke engine tree, read only")
    parser.add_argument("--upstream-port", type=int, help="diagnostic/smoke engine HTTP/WS port")
    parser.add_argument("--name-prefix", help="private live-account namespace, 1 to 4 ASCII characters")
    parser.add_argument("--smoke-duration-seconds", type=int,
                        help="duration of a nonqualifying smoke (default: 180 seconds)")
    parser.add_argument("--repo", type=Path, default=ROOT, help="repo with the T1.4 live test installed")
    parser.add_argument("--mode", choices=("baseline", "candidate"), required=True,
                        help="baseline has no candidate-only observer feature")
    parser.add_argument("--diagnostic-no-ws-faults", action="store_true",
                        help="smoke-only isolation: retain TCP faults, suppress proxy WS fragmentation/Ping")
    args = parser.parse_args()
    repo = args.repo.resolve(strict=True)
    output = args.output.resolve()
    source = args.config.resolve(strict=True)
    if source != SOURCE_CONFIG and not args.smoke:
        raise ValueError("qualification runs must use the one committed T1.4/soak-config.json")
    config = copy.deepcopy(json.loads(source.read_text(encoding="utf-8")))
    if args.seed < 0:
        raise ValueError("seed must be nonnegative")
    if args.smoke_duration_seconds is not None and not args.smoke:
        raise ValueError("--smoke-duration-seconds requires --smoke")
    if args.smoke_duration_seconds is not None and args.smoke_duration_seconds <= 0:
        raise ValueError("--smoke-duration-seconds must be positive")
    if args.diagnostic_seconds is not None and args.diagnostic_seconds <= 0:
        raise ValueError("--diagnostic-seconds must be positive")
    diagnostic = args.diagnostic_seconds is not None
    if (args.engine_dir is not None or args.upstream_port is not None) and not (args.smoke or diagnostic):
        raise ValueError("engine overrides are diagnostic/smoke-only")
    if args.upstream_port is not None and not 1 <= args.upstream_port <= 65535:
        raise ValueError("--upstream-port must be a valid TCP port")
    if args.name_prefix is not None:
        if not args.name_prefix.isascii() or not args.name_prefix.isalnum() or not 1 <= len(args.name_prefix) <= 4:
            raise ValueError("--name-prefix must contain 1 to 4 ASCII letters or digits")
        config["run"]["name_prefix"] = args.name_prefix
    if args.engine_dir is not None:
        config["profile"]["engine_dir"] = str(args.engine_dir.resolve(strict=True))
    if args.upstream_port is not None:
        config["proxy"]["upstream_port"] = args.upstream_port
    if diagnostic:
        config["run"]["duration_seconds"] = args.diagnostic_seconds
        config["_sources"]["diagnostic_override"] = (
            f"nonqualifying full 20-account diagnostic for {args.diagnostic_seconds} phase-zero seconds; "
            "source duration, workload, fault phases and all acceptance limits remain unchanged"
        )
    if args.smoke:
        config["run"]["slots"] = 2
        config["run"]["duration_seconds"] = args.smoke_duration_seconds or 180
        # Smoke exercises the first phase only; changing the approved four
        # 1800-second fault phases would cease to be the same shared proxy.
        config["run"]["warmup_min_seconds"] = 0
        config["_sources"]["smoke_override"] = (
            f"nonqualifying two-slot infrastructure smoke for {config['run']['duration_seconds']} seconds"
        )
    elif not diagnostic:
        if config["run"]["slots"] != 20 or config["run"]["duration_seconds"] != 7200:
            raise ValueError("qualification cohort must remain 20 slots for 7200 seconds")
    if args.diagnostic_no_ws_faults and not args.smoke:
        raise ValueError("diagnostic no-WS-faults mode is smoke-only and cannot qualify")
    if args.diagnostic_no_ws_faults:
        config["fault"]["diagnostic_disable_ws_faults"] = True
        config["_sources"]["fault.diagnostic_disable_ws_faults"] = (
            "nonqualifying smoke diagnostic only; approved fault values are unchanged"
        )
    config["fault"]["seed"] = args.seed
    snapshot_root = Path(os.environ.get(
        "BOT_NAV_SNAPSHOT_ROOT", str(Path.home() / ".274bot" / "unpack-289")
    )).resolve(strict=True)
    if not snapshot_root.is_dir():
        raise ValueError(f"audited 289 navigation snapshot is not a directory: {snapshot_root}")
    source_nav_pack = Path(config["profile"]["nav_pack"]).resolve(strict=True)
    source_nav_flags = Path(config["profile"]["nav_flags"]).resolve(strict=True)
    selected_nav_pack = repo / "target/release/nav/289/274bot.navpack"
    selected_nav_flags = repo / "target/release/nav/289/274bot.navflags"
    config["profile"]["nav_pack"] = str(selected_nav_pack)
    config["profile"]["nav_flags"] = str(selected_nav_flags)
    config["_sources"]["profile.nav_pack"] = (
        "source config's original pack bytes, selected from the build's current "
        "audited public navigation row; exact source/selected SHA verified in nav-identity.json"
    )
    config["_sources"]["profile.nav_flags"] = (
        "source config's original flags bytes, selected from the build's current "
        "audited public navigation row; exact source/selected SHA verified in nav-identity.json"
    )
    output.mkdir(parents=True, mode=0o700, exist_ok=False)
    home = output / "home"
    home.mkdir(mode=0o700)
    ca, server, key = make_ca(home)
    cache_source = Path(config["profile"]["engine_dir"]) / "data" / "pack" / "client"
    if not cache_source.is_dir():
        raise FileNotFoundError(f"immutable Engine A client pack missing: {cache_source}")
    cache_copy = home / "client-cache"
    shutil.copytree(cache_source, cache_copy)
    config["profile"]["cache_dir"] = str(cache_copy)
    config["profile"]["ca_pem"] = str(ca)
    config["proxy"]["cert_pem"] = str(server)
    config["proxy"]["key_pem"] = str(key)
    world_path = Path(config["profile"]["engine_dir"]) / "data/config/world.json"
    world_data = world_path.read_bytes()
    world_node_id = json.loads(world_data)["node"]["id"]
    if type(world_node_id) is not int or not 1 <= world_node_id <= 65535:
        raise ValueError(f"Engine A node.id is invalid: {world_path}")
    config["profile"]["node_id"] = world_node_id
    config["_sources"]["profile.node_id"] = (
        f"read-only Engine A data/config/world.json node.id; SHA-256 {hashlib.sha256(world_data).hexdigest()}"
    )
    modulus, source_sha256 = local_engine_public_modulus(Path(config["profile"]["engine_dir"]))
    config["proxy"]["local_login_modulus"] = modulus
    config["proxy"]["local_login_key_source_sha256"] = source_sha256
    config["_sources"]["proxy.local_login_modulus"] = (
        "read-only Engine A data/config/private.pem public RSA modulus; "
        "local HTTPS /client/client.js fixture only, identical dev/candidate"
    )
    config["run"]["qualification"] = not (args.smoke or diagnostic)
    config["run"]["source_config"] = str(source)
    effective = output / "effective-config.json"
    proxy_output = output / "proxy"
    proxy_output.mkdir(mode=0o700)
    proxy_log = (output / "proxy.log").open("w", encoding="utf-8")
    command = [sys.executable, str(ROOT / "tools" / "t14_soak_proxy.py"),
               "--config", str(effective), "--output", str(proxy_output)]
    proxy = None
    try:
        effective.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")
        proxy = subprocess.Popen(command, cwd=ROOT, stdout=subprocess.PIPE,
                                 stderr=proxy_log, text=True, bufsize=1)
        receipt = ready_proxy(proxy, proxy_output / "ready.json")
        if receipt.get("a_host") != "127.0.0.1":
            raise RuntimeError(f"proxy readiness lacks loopback A endpoint: {receipt!r}")
        port = receipt.get("a_port")
        if not isinstance(port, int) or not (0 < port <= 65535):
            raise RuntimeError(f"proxy readiness lacks assigned A port: {receipt!r}")
        config["profile"]["port"] = port
        config["profile"]["http_port"] = port
        config["run"]["proxy_ready"] = receipt
        effective.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")
        roster = write_roster(home, config, port)
        harness = repo / "crates" / "host-play" / "tests" / "t14_soak_live.rs"
        source_bytes = harness.read_bytes()
        commit = subprocess.run(
            ["git", "-C", str(repo), "rev-parse", "HEAD"],
            check=True, text=True, capture_output=True,
        ).stdout.strip()
        (output / "run-manifest.json").write_text(json.dumps({
            "mode": "diagnostic" if diagnostic else "smoke" if args.smoke else "qualification",
            "build": args.mode, "profile": "release",
            "repo": str(repo), "config": str(effective),
            "roster": str(roster), "seed": args.seed,
            "source_config": str(source), "proxy_ready": receipt,
            "commit": commit,
            "shared_harness_sha256": hashlib.sha256(source_bytes).hexdigest(),
            "nav_build_snapshot_root": str(snapshot_root),
            "nav_build_mode": "require",
            "proxy_sha256": hashlib.sha256(
                (ROOT / "tools" / "t14_soak_proxy.py").read_bytes()
            ).hexdigest(),
        }, indent=2) + "\n", encoding="utf-8")
        env = os.environ.copy()
        # Cargo/rustup are build tools, not the game profile; preserve their
        # shared installed toolchain while isolating every client's HOME.
        env.setdefault("CARGO_HOME", str(Path.home() / ".cargo"))
        env.setdefault("RUSTUP_HOME", str(Path.home() / ".rustup"))
        env.update({
            "HOME": str(home), "LIVE": "1", "BOT_T14_SOAK_CONFIG": str(effective),
            "SSL_CERT_FILE": str(ca),
            "SSL_CERT_DIR": str(home),
            "BOT_T14_SOAK_OUTPUT": str(output), "BOT_T14_SOAK_PROXY_PORT": str(port),
            "BOT_LIVE_NAME_PREFIX": config["run"]["name_prefix"],
            "BOT_SERVER_PROFILE": config["profile"]["name"],
            "ENGINE_DIR": config["profile"]["engine_dir"],
            "NAV_PACK": config["profile"]["nav_pack"],
            "NAV_FLAGS": config["profile"]["nav_flags"],
            "BOT_NAV_BUILD": "require",
            "BOT_NAV_SNAPSHOT_ROOT": str(snapshot_root),
        })
        features = "live-harness,test-support"
        if args.mode == "candidate":
            features += ",soak-observers"
        cargo_test = ["cargo", "test", "--release", "--locked", "-p", "host-play",
                      "--features", features, "--test", "t14_soak_live"]
        # The game trusts only this run's private CA. Build tools must still
        # use the host trust store to fetch uncached release dependencies.
        build_env = env.copy()
        build_env.pop("SSL_CERT_FILE", None)
        build_env.pop("SSL_CERT_DIR", None)
        with (output / "prebuild.log").open("x", encoding="utf-8") as build_log:
            build = subprocess.run([*cargo_test, "--no-run"], cwd=repo, env=build_env,
                                   stdout=build_log, stderr=subprocess.STDOUT)
        if build.returncode != 0:
            raise RuntimeError(f"release harness prebuild exited {build.returncode}; see prebuild.log")
        test_command = [*cargo_test, "t14_local_wss_soak", "--", "--ignored",
                        "--nocapture", "--test-threads=1"]
        result = sample_cohort(test_command, repo, env, output,
                               config["run"]["sample_seconds"])
        nav_identity = {
            "source_pack": str(source_nav_pack),
            "selected_pack": str(selected_nav_pack),
            "source_pack_sha256": file_sha256(source_nav_pack),
            "selected_pack_sha256": file_sha256(selected_nav_pack),
            "source_flags": str(source_nav_flags),
            "selected_flags": str(selected_nav_flags),
            "source_flags_sha256": file_sha256(source_nav_flags),
            "selected_flags_sha256": file_sha256(selected_nav_flags),
        }
        (output / "nav-identity.json").write_text(
            json.dumps(nav_identity, indent=2) + "\n", encoding="utf-8"
        )
        if (nav_identity["source_pack_sha256"] != nav_identity["selected_pack_sha256"]
                or nav_identity["source_flags_sha256"] != nav_identity["selected_flags_sha256"]):
            raise RuntimeError("selected public-build navigation bytes differ from the fixed source pack/flags")
        (output / "cohort-exit.json").write_text(json.dumps({"code": result}) + "\n", encoding="utf-8")
        return result
    finally:
        if proxy is not None:
            proxy.send_signal(signal.SIGTERM) if proxy.poll() is None else None
            try:
                proxy.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proxy.kill()
                proxy.wait()
            if proxy.stdout is not None:
                proxy.stdout.close()
        proxy_log.close()


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"t14 soak setup failed: {error}", file=sys.stderr)
        sys.exit(2)
