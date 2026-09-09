"""Small selection regression fixtures, distinct from production table checks."""
import unittest
import audit


class SelectionTests(unittest.TestCase):
    def test_plain_application(self):
        for crate in ('nav', 'client', 'api', 'host_play', 'script', 'tui'):
            with self.subTest(crate=crate):
                self.assertTrue(audit.useful(crate+'::module::function'))

    def test_angle_self(self):
        self.assertTrue(audit.useful('<client::config::IfType>::unpack'))
        self.assertTrue(audit.useful('<script::load::JsCard as core::clone::Clone>::clone'))

    def test_allocator_incidental_self_is_excluded(self):
        for symbol in (
            '<alloc::vec::Vec<client::config::IfType>>::resize',
            '<alloc::raw_vec::RawVec<api::snapshot::WidgetView>>::grow_one',
            'alloc::vec::from_iter::<host_play::load_template::{closure#0}>',
            '<std::sync::Once<client::sound::Sound>>::call_once',
            '<std::thread::Thread as core::ops::FnOnce<script::Slot>>::call',
            '<[client::Thing] as alloc::slice::ToOwned>::to_owned',
            '<&client::Thing>::clone',
            '<(client::Thing, u8)>::clone',
            'external::f<nav::World>',
            '_RNvSomeUnresolvedSymbol',
            '',
        ):
            with self.subTest(symbol=symbol):
                self.assertFalse(audit.useful(symbol))

    def test_first_callee_not_last_caller(self):
        symbols = ['<alloc::vec::Vec<api::View>>::resize', 'api::snapshot::walk_widget_tree', '<host_play::Play>::new']
        self.assertEqual(audit.select(symbols), 'api::snapshot::walk_widget_tree')

    def test_unresolved_before_or_after_does_not_remove_group(self):
        self.assertEqual(audit.select(['', 'nav::pack::decode', '']), 'nav::pack::decode')

    def test_no_match_preserved(self):
        self.assertEqual(audit.select([]), audit.UNKNOWN)
        self.assertEqual(audit.select(['malloc', '_RNvUnknown']), audit.UNKNOWN)

    def test_allowlist_boundary(self):
        self.assertFalse(audit.useful('client_extra::f'))
        self.assertFalse(audit.useful('host::drain_and_rebuild_snapshot'))
        self.assertFalse(audit.useful('tui_play::main'))


if __name__ == '__main__':
    unittest.main()
