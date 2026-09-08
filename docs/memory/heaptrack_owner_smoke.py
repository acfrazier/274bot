#!/usr/bin/env python3
"""Produce an owned portable replay receipt from the existing 2001 fixture only.

This does not capture, profile, run a game, retrieve inputs, or qualify Linux.
The output directory must be new. Run from the campaign checkout.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

from test_heaptrack_owner_replay import smoke_manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    args.output.mkdir(mode=0o700)
    manifest, digest = smoke_manifest(args.output)
    command = [sys.executable, '-B', str(Path(__file__).with_name('heaptrack_owner_runner.py')),
               '--manifest', str(manifest), '--manifest-sha256', digest,
               '--output', str(args.output/'replay'), '--portable-fixture']
    result = subprocess.run(command, check=False)
    print(json.dumps({'exit_code': result.returncode, 'manifest_sha256': digest,
                      'receipt': str(args.output/'replay/result/receipt.json'),
                      'runner': str(args.output/'replay/runner.json')}))
    return result.returncode


if __name__ == '__main__':
    raise SystemExit(main())
