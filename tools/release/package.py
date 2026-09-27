#!/usr/bin/env python3
"""Stage release binaries, navigation and WalkTo map terrain; optionally
Developer-ID sign macOS.

Build first with cargo build --locked --release and default features. Every
platform ships panel-play and tui-play. The input must be the Cargo profile
directory containing matching nav/<revision> outputs. Revision 289 packages
also ship WalkTo map terrain: the staged tui-play bakes it from the pinned
client cache the nav bundle was built from (--map-cache and --map-unpack; by
default the nav build's BOT_NAV_ENGINE_DIR / ENGINE_DIR and
BOT_NAV_SNAPSHOT_ROOT resolution), and the bake must carry the nav bundle's
decoded content identity. --check stages into a temporary directory, verifies
every staged file, prints a summary and removes it. This tool never publishes a
release or uploads to Apple's notary service.
"""
import argparse
import hashlib
import json
import os
import plistlib
import re
import shutil
import subprocess
import tempfile
from pathlib import Path

SHIPPED_IMAGES_NAME = '274bot.mapimages.json'


def run(*args):
    return subprocess.check_output(args, text=True).strip()


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def nav_build_inputs(revision):
    """The client cache the nav build reads, resolved as crates/host-play/build.rs does."""
    engine = os.environ.get('BOT_NAV_ENGINE_DIR') or os.environ.get('ENGINE_DIR')
    if engine:
        engine = Path(engine)
    elif revision == '289':
        engine = Path.home() / 'experiments/lostcity-289/engine'
    else:
        engine = Path.home() / 'experiments/Server/engine'
    snapshots = os.environ.get('BOT_NAV_SNAPSHOT_ROOT')
    snapshots = Path(snapshots) if snapshots else \
        Path.home() / '.274bot' / ('unpack-289' if revision == '289' else 'unpack')
    return engine / 'data/pack/client', snapshots


def ship_map_terrain(p, tool, output, revision, cache, unpack, nav_manifest):
    """Bake the shipped WalkTo map terrain with the staged tui-play and verify it."""
    for label, path in (('--map-cache', cache), ('--map-unpack', unpack)):
        if not path.is_dir():
            p.error(f'{label} {path} is not a directory (the client cache the nav bundle '
                    'was built from)')
    baked = subprocess.run([str(tool), '--map-bundle', str(output), '--revision', revision,
                            '--cache', str(cache), '--unpack', str(unpack)],
                           check=True, stdout=subprocess.PIPE, text=True)
    shipped = output / 'map' / revision
    description = json.loads((shipped / SHIPPED_IMAGES_NAME).read_text())
    if json.loads(baked.stdout.strip().splitlines()[-1]) != description:
        p.error('tui-play --map-bundle printed another description than it wrote')
    content_id = json.loads(nav_manifest.read_text()).get('content_id')
    if description['identity']['content'] != content_id:
        p.error('shipped map terrain was baked from another client cache than the nav bundle '
                f'(content {description["identity"]["content"]}, nav {content_id})')
    verify_map_terrain(p, output / 'map', revision, description)
    return description


def verify_map_terrain(p, map_dir, revision, description):
    """Every shipped file against its receipt, and nothing else under map/."""
    key = description['key']
    images = map_dir / revision / 'images' / key
    data = (images / 'manifest.json').read_bytes()
    if (len(data) != description['manifest']['bytes']
            or hashlib.sha256(data).hexdigest() != description['manifest']['sha256']):
        p.error('shipped map manifest does not match its receipt')
    manifest = json.loads(data)
    if manifest['identity'] != description['identity'] or manifest['key'] != key:
        p.error('shipped map manifest belongs to another image identity')
    expected = {f'{revision}/{SHIPPED_IMAGES_NAME}', f'{revision}/images/{key}/manifest.json'}
    total = 0
    for tile in manifest['tiles']:
        k = tile['key']
        relative = f"terrain/p{k['plane']}/l{k['lod']}/{k['x']}_{k['z']}.png"
        body = (images / relative).read_bytes()
        if (len(body) != tile['payload']['bytes']
                or hashlib.sha256(body).hexdigest() != tile['payload']['sha256']):
            p.error('shipped map tile does not match its receipt: ' + relative)
        expected.add(f'{revision}/images/{key}/{relative}')
        total += len(body)
    if len(manifest['tiles']) != description['tiles'] or total != description['tile_bytes']:
        p.error('shipped map terrain does not match its description totals')
    actual = {f.relative_to(map_dir).as_posix() for f in map_dir.rglob('*') if f.is_file()}
    if actual != expected:
        p.error('unexpected shipped map files: ' + ', '.join(sorted(actual ^ expected)[:5]))


def verify_staged(p, output, manifest):
    """Re-hash every staged file against release-manifest.json."""
    listed = set(manifest['files']) | {'release-manifest.json'}
    actual = {f.relative_to(output).as_posix() for f in output.rglob('*') if f.is_file()}
    if actual != listed:
        p.error('staged files differ from the release manifest: '
                + ', '.join(sorted(actual ^ listed)[:5]))
    for name, row in manifest['files'].items():
        path = output / name
        if path.stat().st_size != row['bytes'] or digest(path) != row['sha256']:
            p.error('staged file does not match the release manifest: ' + name)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--platform', choices=['macos', 'windows', 'linux'], required=True)
    p.add_argument('--input', type=Path, required=True)
    p.add_argument('--output', type=Path)
    p.add_argument('--build-receipt', type=Path, required=True,
                   help='JSON with host_commit, client_commit, target, rustc, features, built_at')
    p.add_argument('--revision', choices=['274', '289'], default='289')
    p.add_argument('--app-profile', choices=['public-289', 'local-289', 'local-274'])
    p.add_argument('--sign-identity', help='Developer ID Application identity SHA-1')
    p.add_argument('--map-cache', type=Path,
                   help='jag directory of the client cache the nav bundle was built from '
                        '(default: the nav build\'s engine data/pack/client)')
    p.add_argument('--map-unpack', type=Path,
                   help='snapshot root holding that cache\'s decoded snapshot '
                        '(default: the nav build\'s BOT_NAV_SNAPSHOT_ROOT)')
    p.add_argument('--check', action='store_true',
                   help='stage into a temporary directory, verify, print a summary and remove it')
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
    if a.check and (a.output or a.sign_identity):
        p.error('--check stages into a temporary directory; drop --output and --sign-identity')
    if not a.check and not a.output:
        p.error('--output is required')
    if a.sign_identity:
        identities = run('security', 'find-identity', '-v', '-p', 'codesigning')
        matches = [line for line in identities.splitlines()
                   if a.sign_identity in line and 'Developer ID Application:' in line]
        if not matches:
            p.error('a valid Developer ID Application identity is required')
    map_cache, map_unpack = nav_build_inputs(a.revision)
    map_cache = a.map_cache or map_cache
    map_unpack = a.map_unpack or map_unpack
    scratch = Path(tempfile.mkdtemp(prefix='274bot-package-check-')) if a.check else None
    output = scratch / 'package' if a.check else a.output
    # Refuse to overwrite an earlier package or mix old and new artifacts.
    if output.exists():
        p.error('output already exists')
    suffix = '.exe' if a.platform == 'windows' else ''
    # Every platform ships the panel beside the TUI. The panel's fonts are
    # compiled in; it reads only the adjacent nav/ and map/ resources.
    names = ['panel-play', 'tui-play']
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
    try:
        output.mkdir(parents=True)
        for name in names:
            shutil.copy2(a.input / (name + suffix), output / (name + suffix))
        nav_out = output / 'nav' / a.revision
        nav_out.mkdir(parents=True)
        for name in nav_names:
            shutil.copy2(nav / name, nav_out / name)
        # The operator's 289 decision ships WalkTo terrain; a 274 package has none.
        map_images = None
        if a.revision == '289':
            map_images = ship_map_terrain(p, output / ('tui-play' + suffix), output, a.revision,
                                          map_cache, map_unpack, nav_out / '274bot.navpack.json')
        for name in ('LICENSE', 'NOTICE.md', 'FIRST-START.md'):
            shutil.copy2(root / name, output / name)
        signed = []
        if a.platform == 'macos':
            app = output / '274bot.app'
            macos = app / 'Contents/MacOS'
            resources = app / 'Contents/Resources'
            macos.mkdir(parents=True)
            resources.mkdir()
            shutil.copy2(a.input / 'panel-play', macos / 'panel-play')
            shutil.copytree(output / 'nav', resources / 'nav')
            if map_images:
                shutil.copytree(output / 'map', resources / 'map')
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
                for target in [output / n for n in names] + [app]:
                    subprocess.run(['codesign', '--force', '--options', 'runtime', '--timestamp',
                                    '--entitlements', str(entitlements), '--sign', a.sign_identity,
                                    str(target)], check=True)
                    subprocess.run(['codesign', '--verify', '--strict', '--verbose=2', str(target)],
                                   check=True)
                    signed.append(str(target.relative_to(output)))
        receipt.update(version=version, release=release, platform=a.platform,
                       revision=int(a.revision), app_profile=a.app_profile,
                       signed=signed, notarized=False,
                       status='signed-staging' if signed else 'unsigned-staging')
        if map_images:
            receipt['map_images'] = map_images
        receipt['files'] = {f.relative_to(output).as_posix(): {'sha256': digest(f),
                                                              'bytes': f.stat().st_size}
                            for f in sorted(output.rglob('*')) if f.is_file()}
        (output / 'release-manifest.json').write_text(json.dumps(receipt, indent=2) + '\n')
        if a.check:
            verify_staged(p, output, receipt)
            print(json.dumps({
                'check': 'ok', 'platform': a.platform, 'revision': int(a.revision),
                'files': len(receipt['files']),
                'bytes': sum(row['bytes'] for row in receipt['files'].values()),
                'map_images': map_images,
            }, indent=2))
        else:
            print(output)
    finally:
        if scratch:
            shutil.rmtree(scratch)


if __name__ == '__main__':
    main()
