"""A bank document alone must never satisfy the shipped-writer gate."""

import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import upgrade_test as upgrade


class WriterEvidence(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.state = upgrade.fresh_home(Path(tmp.name))
        (self.state / 'vault-289').write_bytes(b'274VAULT')
        self.save('panel-ui.json', {'session_log_file': True, 'nav': {'pause_script_on_manual_walk_abort': False}})
        self.save('loadouts.json', [{'name': 'upgrade', 'carry': [{'item': 'Lobster', 'qty': 7}]}])
        self.save('script-settings.json', {'file:UpgradeProbe': {'mode': 'shared'}})
        self.save('js-scripts.json', [{'name': 'UpgradeProbe', 'path': str(upgrade.PROBE)}])
        self.save('servers.json', {'schema_version': 1, 'servers': [{'name': 'local-289'}]})
        self.hint = {'profile': 'local-289', 'account': 'alice', 'revision': 289,
                     'content': 'example', 'schema_version': 1, 'observed_at_unix': 100, 'rows': [[995, 1]]}
        self.path = 'bank-hints/local-289/alice.json'
        self.save(self.path, self.hint)
        self.runtime = 'script-runtime-settings/local-289/alice/289-example.json'
        self.save(self.runtime, dict(self.hint, rows=[
            {'namespace': 'UpgradeProbe', 'key': 'sentinel', 'raw': upgrade.RAW}]))
        (self.state / 'logs').mkdir()
        self.log = self.state / 'logs/session-writer.log'
        self.opened = 'bank memory observed rows=1 size=1 generation=1\n'
        self.closed = 'bank hints: saved rows=1 why=close version=1 path=' + self.path + '\n'
        self.log.write_text(self.opened + self.closed)

    def save(self, name, data):
        path = self.state / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(data))

    def check(self):
        upgrade.check_writer(self.state, 'local-289', 'alice', 100)

    def test_complete_evidence(self):
        self.check()

    def test_incomplete_or_unrelated_bank_receipts(self):
        for log in ('', self.opened, self.closed, self.opened + self.closed.replace('alice', 'bob'),
                    self.opened + self.closed.replace('why=close', 'why=slot exit')):
            with self.subTest(log=log):
                self.log.write_text(log)
                with self.assertRaisesRegex(upgrade.Failure, 'bank open or durable close'):
                    self.check()

    def test_stale_empty_or_wrong_scope(self):
        for key, value in [('observed_at_unix', 99), ('rows', []), ('revision', 274), ('content', ''), ('account', 'bob')]:
            with self.subTest(key=key):
                self.save(self.path, dict(self.hint, **{key: value}))
                with self.assertRaisesRegex(upgrade.Failure, 'scoped bank observation'):
                    self.check()

    def test_missing_shared_settings_are_not_covered_by_native_cards(self):
        self.save('script-settings.json', {})
        with self.assertRaisesRegex(upgrade.Failure, 'shared card override'):
            self.check()

    def test_existing_home_is_never_reused(self):
        with self.assertRaises(FileExistsError):
            upgrade.fresh_home(self.state.parent.parent)

    def test_runtime_requires_saved_raw_value_and_account(self):
        for key, value in [('rows', []), ('account', 'bob')]:
            doc = json.loads((self.state/self.runtime).read_text())
            self.save(self.runtime, dict(doc, **{key: value}))
            with self.assertRaisesRegex(upgrade.Failure, 'scoped runtime setting'):
                self.check()
            self.save(self.runtime, doc)

    def test_snapshot_copies_optional_stores_but_never_follows_links(self):
        with tempfile.TemporaryDirectory() as destination:
            (self.state/'rs2b0t-import').write_text('deferred')
            target = Path(destination)
            upgrade.copy_stores(self.state, target)
            self.assertEqual((target/'rs2b0t-import').read_text(), 'deferred')
            self.assertEqual((target/self.runtime).read_bytes(), (self.state/self.runtime).read_bytes())
            self.assertFalse((target/'logs').exists())
            (self.state/'panel-ui.json').unlink()
            (self.state/'panel-ui.json').symlink_to(self.log)
            with self.assertRaisesRegex(upgrade.Failure, 'link'):
                upgrade.copy_stores(self.state, target)


class HistoricalExit(unittest.TestCase):
    def test_only_product_refusal_returns_negative_control_status(self):
        refusal = 'a format from before 0.2.8'
        accepted = 'no profile named absent in the vault:'
        for code, out, status in [(1, refusal, 1), (0, refusal, 2), (1, 'other error', 2),
                                  (1, accepted, 0), (1, accepted.replace('absent', 'bob'), 2)]:
            with self.subTest(code=code, out=out), tempfile.TemporaryDirectory() as tmp:
                args = ['upgrade_test', 'control', '--binary', 'bin', '--snapshot', 'snapshot',
                        '--pass-file', 'pass', '--crc-file', 'crc', '--account', 'alice',
                        '--cache', 'cache', '--work', str(Path(tmp)/'work')]
                with patch.object(sys, 'argv', args), patch.object(upgrade, 'control',
                        side_effect=lambda *_: upgrade.startup_result(code, out, 'absent', 'snapshot')):
                    self.assertEqual(upgrade.main(), status)
