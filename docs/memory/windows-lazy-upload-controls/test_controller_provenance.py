"""Exercise actual controller output expressions against the strict reader."""
import ast
import copy
import pathlib
import sys
import types
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import matched_evidence_adapter as reader
from test_matched_evidence_adapter import NATIVE_WINDOWS_CONDITIONS

CONTROLLER = pathlib.Path(__file__).with_name('run-panel-lazy-upload-focused-one.py')


def output_expression(name, namespace):
    tree = ast.parse(CONTROLLER.read_text())
    for node in tree.body:
        if (isinstance(node, ast.Expr) and isinstance(node.value, ast.Call)
                and isinstance(node.value.func, ast.Name) and node.value.func.id == 'dump'
                and isinstance(node.value.args[0], ast.Name)
                and node.value.args[0].id == name):
            return eval(compile(ast.Expression(node.value.args[1]), str(CONTROLLER), 'eval'), namespace)
    raise AssertionError('controller output missing: ' + name)


class ControllerProvenance(unittest.TestCase):
    def test_real_condition_expression_satisfies_reader_for_four_cells(self):
        for role in ('baseline', 'candidate'):
            for mode in ('focused-one', 'focused-plus-background'):
                with self.subTest(role=role, mode=mode):
                    preflight = copy.deepcopy(NATIVE_WINDOWS_CONDITIONS['native_preflight'])
                    value = output_expression('conditions', {
                        'platform': types.SimpleNamespace(platform=lambda: 'Windows-11'),
                        'os': types.SimpleNamespace(environ={'USERNAME': 'BotTest'}),
                        'role': role, 'mode': mode, 'cell_id': role + '-' + mode + '-fixture',
                        'preflight_record': preflight,
                    })
                    self.assertTrue(reader._native_windows_conditions_complete(value))
                    for field in ('purpose', 'terminal_transport_expected', 'panel_render_attribution'):
                        broken = copy.deepcopy(value)
                        del broken[field]
                        self.assertFalse(reader._native_windows_conditions_complete(broken), field)
                    del value['native_preflight']['dxdiag']
                    self.assertFalse(reader._native_windows_conditions_complete(value))

    def test_real_server_expression_retains_frozen_configuration(self):
        value = output_expression('server_id', {
            'pid': 6728, 'sample': {'start_identity': 'filetime:123'}, 'launch': {},
            'server': pathlib.Path('/fixture/server'), 'sha': lambda path: 'a' * 64,
        })
        config = value['configuration']
        self.assertEqual(config['bind_host'], '127.0.0.1')
        self.assertEqual(config['node_version'], '24.19.0')
        for field in ('world_json_sha256', 'maps_addition_sha256', 'wordenc_addition_sha256'):
            self.assertEqual(config[field], 'a' * 64)
        self.assertEqual(value['port_listen'], 43594)


if __name__ == '__main__':
    unittest.main()
