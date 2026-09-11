import hashlib,json,pathlib,tarfile,io
root=pathlib.Path(__file__).resolve().parents[2];base=root/'.superpowers/platform-preparation';short='b1cff8a7';source_manifest=json.loads((root/f'docs/compat/evidence/catalog-headed/source-{short}.json').read_text());source=pathlib.Path(source_manifest['source_root']);old=base/'catalog-source-c933f37c.tar.gz';assert hashlib.sha256(old.read_bytes()).hexdigest()=='2b848e33b3e7325bfcad3f43c7693f000d2958bb4b8d7a3b55bb1c766baa81fa';dest=base/f'platform-isolated-{short}.tar.gz';assert not dest.exists();files={}
with tarfile.open(old) as previous,tarfile.open(dest,'w:gz') as out:
 m=json.load(previous.extractfile('source-manifest.json'))
 for name,h in m['files'].items():
  if not (name.startswith('inputs/') or name.startswith('nav/')):continue
  data=previous.extractfile(name).read();assert hashlib.sha256(data).hexdigest()==h,name;info=tarfile.TarInfo(name);info.size=len(data);info.mode=0o644;out.addfile(info,io.BytesIO(data));files[name]=h
 for name,h in source_manifest['files'].items():
  p=source/name;assert hashlib.sha256(p.read_bytes()).hexdigest()==h,name;out.add(p,arcname='source/'+name);files['source/'+name]=h
 overlay_name='source/crates/tui/examples/platform_proof.rs';overlay=b'fn main() -> std::process::ExitCode {\n    let _stores = script::IsolatedEnv::enter(&format!("platform-proof-{}", std::process::id()));\n    tui::bin::main()\n}\n';assert overlay_name not in files;info=tarfile.TarInfo(overlay_name);info.size=len(overlay);info.mode=0o644;out.addfile(info,io.BytesIO(overlay));files[overlay_name]=hashlib.sha256(overlay).hexdigest()
 manifest=dict(overlays={overlay_name:overlay.decode()},host_commit=source_manifest['host_commit'],client_commit=source_manifest['client_commit'],files=files,scope='Exact host/client export plus previously hash-verified immutable catalogs/nav; new empty build target required')
 data=(json.dumps(manifest,indent=2)+'\n').encode();info=tarfile.TarInfo('source-manifest.json');info.size=len(data);info.mode=0o644;out.addfile(info,io.BytesIO(data))
e=root/'docs/compat/evidence/platform-isolated';e.mkdir(exist_ok=True);receipt=dict(archive=str(dest),archive_sha256=hashlib.sha256(dest.read_bytes()).hexdigest(),archive_bytes=dest.stat().st_size,host_commit=manifest['host_commit'],client_commit=manifest['client_commit'],source_files=len(files),old_inputs_archive_sha256=hashlib.sha256(old.read_bytes()).hexdigest());(e/f'archive-{short}.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt),flush=True)
