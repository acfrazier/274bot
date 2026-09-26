#!/usr/bin/env python3
"""Stage release binaries and navigation; optionally Developer-ID sign macOS.

Build first with cargo build --locked --release and default features. The input
must be the Cargo profile directory containing matching nav/<revision> outputs.
This tool never publishes a release or uploads to Apple's notary service.
"""
import argparse
import hashlib
import json
import plistlib
import re
import shutil
import subprocess
from pathlib import Path


def run(*args):
    return subprocess.check_output(args, text=True).strip()


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--platform', choices=['macos', 'windows', 'linux'], required=True)
    p.add_argument('--input', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--build-receipt', type=Path, required=True,
                   help='JSON with host_commit, client_commit, target, rustc, features, built_at')
    p.add_argument('--revision', choices=['274', '289'], default='289')
    p.add_argument('--app-profile', choices=['public-289', 'local-289', 'local-274'])
    p.add_argument('--sign-identity', help='Developer ID Application identity SHA-1')
    a = p.parse_args()
    root = Path(__file__).resolve().parents[2]
    metadata = json.loads(run('cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1',
                              '--manifest-path', str(root / 'Cargo.toml')))
    version = next(x['version'] for x in metadata['packages'] if x['name'] == 'panel')
    # Public name (e.g. "alpha 3") has one source: the panel's build line.
    build_info = (root / 'crates/panel/src/build_info.rs').read_text()
    release = re.search(r'pub const RELEASE: &str = "([^"]+)";', build_info).group(1).title()
    receipt = json.loads(a.build_receipt.read_text())
    for key in ('host_commit', 'client_commit', 'target', 'rustc', 'features', 'built_at', 'files'):
        if key not in receipt:
            p.error('missing build receipt field: ' + key)
    if receipt['features'] != 'default':
        p.error('release packages require default features')
    if a.platform == 'macos' and not a.app_profile:
        p.error('--app-profile is required for a macOS bundle')
    if a.app_profile and not a.app_profile.endswith(a.revision):
        p.error('app profile must match the bundled navigation revision')
    if a.sign_identity and a.platform != 'macos':
        p.error('--sign-identity is only supported on macOS')
    if a.sign_identity:
        identities = run('security', 'find-identity', '-v', '-p', 'codesigning')
        matches = [line for line in identities.splitlines()
                   if a.sign_identity in line and 'Developer ID Application:' in line]
        if not matches:
            p.error('a valid Developer ID Application identity is required')
    # Refuse to overwrite an earlier package or mix old and new artifacts.
    if a.output.exists():
        p.error('output already exists')
    suffix = '.exe' if a.platform == 'windows' else ''
    names = ['tui-play'] if a.platform == 'linux' else ['panel-play', 'tui-play']
    nav = a.input / 'nav' / a.revision
    nav_names = ['274bot.navpack', '274bot.navflags', '274bot.navreach',
                 '274bot.navcanlight', '274bot.navpois', '274bot.navpack.json']
    for path in [a.input / (n + suffix) for n in names] + [nav / n for n in nav_names]:
        if not path.is_file():
            p.error('missing required artifact: ' + str(path))
    for path in [a.input / (n + suffix) for n in names] + [nav / n for n in nav_names]:
        relative = str(path.relative_to(a.input)).replace('\\', '/')
        if receipt['files'].get(relative) != digest(path):
            p.error('artifact does not match build receipt: ' + relative)
    a.output.mkdir(parents=True)
    for name in names:
        shutil.copy2(a.input / (name + suffix), a.output / (name + suffix))
    nav_out = a.output / 'nav' / a.revision
    nav_out.mkdir(parents=True)
    for name in nav_names:
        shutil.copy2(nav / name, nav_out / name)
    for name in ('LICENSE', 'NOTICE.md', 'FIRST-START.md'):
        shutil.copy2(root / name, a.output / name)
    signed = []
    if a.platform == 'macos':
        app = a.output / '274bot.app'
        macos = app / 'Contents/MacOS'
        resources = app / 'Contents/Resources'
        macos.mkdir(parents=True)
        resources.mkdir()
        shutil.copy2(a.input / 'panel-play', macos / 'panel-play')
        shutil.copytree(a.output / 'nav', resources / 'nav')
        info = {
            'CFBundleName': '274bot', 'CFBundleDisplayName': '274bot ' + release,
            'CFBundleIdentifier': 'com.acfrazier.274bot',
            'CFBundleExecutable': 'panel-play', 'CFBundlePackageType': 'APPL',
            'CFBundleShortVersionString': version, 'CFBundleVersion': version,
            'NSHighResolutionCapable': True,
            'LSEnvironment': {'BOT_SERVER_PROFILE': a.app_profile},
        }
        with (app / 'Contents/Info.plist').open('wb') as f:
            plistlib.dump(info, f)
        if a.sign_identity:
            entitlements = root / 'tools/release/macos-entitlements.plist'
            for target in [a.output / n for n in names] + [app]:
                subprocess.run(['codesign', '--force', '--options', 'runtime', '--timestamp',
                                '--entitlements', str(entitlements), '--sign', a.sign_identity,
                                str(target)], check=True)
                subprocess.run(['codesign', '--verify', '--strict', '--verbose=2', str(target)], check=True)
                signed.append(str(target.relative_to(a.output)))
    receipt.update(version=version, release=release, platform=a.platform,
                   revision=int(a.revision), app_profile=a.app_profile,
                   signed=signed, notarized=False,
                   status='signed-staging' if signed else 'unsigned-staging')
    receipt['files'] = {str(f.relative_to(a.output)): {'sha256': digest(f), 'bytes': f.stat().st_size}
                        for f in sorted(a.output.rglob('*')) if f.is_file()}
    (a.output / 'release-manifest.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(a.output)


if __name__ == '__main__':
    main()
