from pathlib import Path
import json,hashlib,os,plistlib,sys
root=Path(__file__).resolve().parents[2];source=sys.argv[1]
assert source.replace('-','').isalnum()
build=json.loads((root/f'docs/compat/evidence/catalog-headed/binary-{source}.json').read_text());binary=Path(build['binary']);assert hashlib.sha256(binary.read_bytes()).hexdigest()==build['binary_sha256']
app=root/f'.superpowers/catalog-headed/Live Qualification {source}.app';exe=app/'Contents/MacOS/catalog_watch';exe.parent.mkdir(parents=True,exist_ok=True)
if not exe.exists():os.link(binary,exe)
assert hashlib.sha256(exe.read_bytes()).hexdigest()==build['binary_sha256']
with (app/'Contents/Info.plist').open('wb') as f:plistlib.dump(dict(CFBundleExecutable='catalog_watch',CFBundleIdentifier='dev.274bot.live-qualification-'+source,CFBundleName='Live Qualification '+source,CFBundlePackageType='APPL',CFBundleVersion='1.0',NSHighResolutionCapable=True),f)
print(exe)
