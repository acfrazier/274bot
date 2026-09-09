import hashlib,json,os,pathlib,platform,subprocess,sys,tarfile,time
root=pathlib.Path(__file__).resolve().parent
assert platform.system()=='Windows'
m=json.loads((root/'source-manifest.json').read_text());assert hashlib.sha256((root/'source.tar.gz').read_bytes()).hexdigest()==m['archive_sha256']
src=root/'source';src.mkdir(exist_ok=False)
with tarfile.open(root/'source.tar.gz') as t:
 ms=t.getmembers();assert len(ms)==len(m['files']) and {x.name for x in ms}=={x['path'] for x in m['files']}
 for x in ms:
  assert x.isfile() and not pathlib.PurePosixPath(x.name).is_absolute() and '..' not in pathlib.PurePosixPath(x.name).parts
  f=src/x.name;f.parent.mkdir(parents=True,exist_ok=True);f.write_bytes(t.extractfile(x).read())
def verify():
 for e in m['files']:
  b=(src/e['path']).read_bytes();assert len(b)==e['bytes'] and hashlib.sha256(b).hexdigest()==e['sha256']
verify()
tests=['test_direct_owner_spec_is_exact_opt_in_and_default_is_unchanged','test_owner_environment_is_scrubbed_and_only_direct_mode_reemits','test_direct_owner_rejects_heaptrack_and_non_n1','test_diagnostic_argv_is_exact_resource_only_contract','test_n1_cli_and_environment_agree_in_generated_contract','test_n1_spec_receipt_matches_cli_and_environment','test_default_n16_remains_the_cli_and_environment_contract','test_wall_budget_is_unchanged_without_capture_and_split_with_capture','test_invalid_n_fails_before_launch','test_invalid_programmatic_n_fails_closed_before_launch','test_missing_programmatic_n_keeps_default','test_clean_environment_forces_all_hot_flags_off','test_feature_contract_rejects_counting_or_snapshot_dedup']
module='test_current_tui_calibration'
# Discover the owning class by each existing method name; execute every selected method exactly once.
code="import importlib,unittest; [importlib.import_module(n) for n in ['run_current_tui_calibration','run_managed_cell','run_diagnostic','build_provenance','managed_receipt']]; m=importlib.import_module('"+module+"'); names="+repr(tests)+"; suite=unittest.TestSuite(); classes=[c for c in vars(m).values() if isinstance(c,type) and issubclass(c,unittest.TestCase)]; [suite.addTest(next(c for c in classes if hasattr(c,n))(n)) for n in names]; r=unittest.TextTestRunner(verbosity=2).run(suite); raise SystemExit(not r.wasSuccessful())"
cmd=[sys.executable,'-B','-c',code];log=root/'windows-contract.log';start=time.monotonic()
with log.open('xb') as f:r=subprocess.run(cmd,cwd=src/'docs/memory',stdout=f,stderr=subprocess.STDOUT,timeout=60,env=dict(os.environ,PYTHONDONTWRITEBYTECODE='1'))
verify();b=log.read_bytes();result={'scope':'native Windows imports and 13 existing pure argv/env/default contract tests only; no UI/live/process lifecycle qualification','commit':m['commit'],'platform':platform.platform(),'python':sys.version,'tests_selected':tests,'exit_code':r.returncode,'elapsed_s':time.monotonic()-start,'source_verified_before_and_after':True,'log':log.name,'bytes':len(b),'sha256':hashlib.sha256(b).hexdigest()};(root/'windows-result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
