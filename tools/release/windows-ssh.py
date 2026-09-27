#!/usr/bin/env python3
"""Run PowerShell and copy files through a pinned OpenSSH connection.

All host, identity, and known-hosts values are parameters or environment
variables. Strict host-key checking and batch mode are always enabled.
"""

import argparse
import base64
import os
import subprocess
from pathlib import Path


class TransportError(RuntimeError):
    pass


def ssh_options(identity=None, known_hosts=None, connect_timeout=15):
    options = [
        "-o", "BatchMode=yes",
        "-o", f"ConnectTimeout={connect_timeout}",
        "-o", "StrictHostKeyChecking=yes",
        "-o", "IdentitiesOnly=yes",
    ]
    if known_hosts:
        options += ["-o", f"UserKnownHostsFile={known_hosts}"]
    if identity:
        options += ["-i", str(identity)]
    return options


def encoded_powershell(script):
    guarded = (
        "$ErrorActionPreference='Stop';"
        "$ProgressPreference='SilentlyContinue';"
        + script
        + "; if ($LASTEXITCODE -ne $null -and $LASTEXITCODE -ne 0) "
          "{ exit $LASTEXITCODE }; exit 0"
    )
    return base64.b64encode(guarded.encode("utf-16-le")).decode("ascii")


def execute(command, timeout):
    result = subprocess.run(
        [str(value) for value in command],
        capture_output=True,
        text=True,
        timeout=timeout,
        check=False,
    )
    if result.returncode:
        raise TransportError((result.stdout + result.stderr).strip())
    return result.stdout


def run_powershell(host, script, *, identity=None, known_hosts=None,
                   connect_timeout=15, command_timeout=None):
    command = [
        "ssh",
        *ssh_options(identity, known_hosts, connect_timeout),
        host,
        "powershell",
        "-NoProfile",
        "-NonInteractive",
        "-EncodedCommand",
        encoded_powershell(script),
    ]
    return execute(command, command_timeout)


def copy_to(host, source, destination, *, identity=None, known_hosts=None,
            connect_timeout=15, command_timeout=None):
    source = Path(source)
    if not source.is_file():
        raise TransportError(f"local source is missing: {source}")
    command = [
        "scp",
        *ssh_options(identity, known_hosts, connect_timeout),
        source,
        f"{host}:{destination}",
    ]
    return execute(command, command_timeout)


def copy_from(host, source, destination, *, identity=None, known_hosts=None,
              connect_timeout=15, command_timeout=None):
    destination = Path(destination)
    destination.parent.mkdir(parents=True, exist_ok=True)
    command = [
        "scp",
        *ssh_options(identity, known_hosts, connect_timeout),
        f"{host}:{source}",
        destination,
    ]
    return execute(command, command_timeout)


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--host", default=os.environ.get("RELEASE_WINDOWS_HOST"),
                        help="user@host (or RELEASE_WINDOWS_HOST)")
    result.add_argument("--identity", type=Path,
                        default=os.environ.get("RELEASE_WINDOWS_IDENTITY"))
    result.add_argument("--known-hosts", type=Path,
                        default=os.environ.get("RELEASE_WINDOWS_KNOWN_HOSTS"))
    result.add_argument("--connect-timeout", type=int, default=15)
    commands = result.add_subparsers(dest="command", required=True)
    remote = commands.add_parser("run", help="run a PowerShell script")
    remote.add_argument("script")
    remote.add_argument("--timeout", type=int)
    put = commands.add_parser("put", help="copy one local file to Windows")
    put.add_argument("source")
    put.add_argument("destination")
    put.add_argument("--timeout", type=int)
    get = commands.add_parser("get", help="copy one Windows file locally")
    get.add_argument("source")
    get.add_argument("destination")
    get.add_argument("--timeout", type=int)
    return result


def main(argv=None):
    arguments = parser().parse_args(argv)
    if not arguments.host:
        parser().error("--host or RELEASE_WINDOWS_HOST is required")
    keywords = {
        "identity": arguments.identity,
        "known_hosts": arguments.known_hosts,
        "connect_timeout": arguments.connect_timeout,
        "command_timeout": arguments.timeout,
    }
    try:
        if arguments.command == "run":
            output = run_powershell(arguments.host, arguments.script, **keywords)
        elif arguments.command == "put":
            output = copy_to(arguments.host, arguments.source, arguments.destination, **keywords)
        else:
            output = copy_from(arguments.host, arguments.source, arguments.destination, **keywords)
    except (OSError, subprocess.SubprocessError, TransportError) as error:
        parser().error(str(error))
    if output:
        print(output, end="" if output.endswith("\n") else "\n")


if __name__ == "__main__":
    main()
