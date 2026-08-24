//! The compiled `Script` trait and its per-tick context. The driver is the
//! same send-side `api::interact::Driver` the kernel uses; scripts never
//! touch `client` or the world.

use api::interact::Driver;

/// House compiled script. `tick` must return; no delayUntil.
pub trait Script: Send {
    fn name(&self) -> &str;
    fn tick(&mut self, ctx: &mut ScriptCtx<'_>);
    /// Teardown hook, run once by the slot on `stop`, before the instance
    /// is dropped.
    fn on_stop(&mut self) {}
}

/// What one observed game-tick gives a script: the send-side driver and
/// the tick number from the pump's PLAYER_INFO edge.
pub struct ScriptCtx<'a> {
    pub driver: &'a mut dyn Driver,
    pub tick: u64,
}

/// Accept-everything driver used by in-crate unit tests. Integration
/// tests (`tests/slot.rs`) copy the `Recorder` stub from `crates/api`
/// instead, so they exercise the same trait the kernel sees.
#[cfg(test)]
pub mod test_support {
    use api::interact::Driver;
    use api::prot::Out;

    #[derive(Default)]
    pub struct OutSink;

    impl Out for OutSink {
        fn p1_enc(&mut self, _opcode: i32) {}
        fn p1(&mut self, _value: i32) {}
        fn p2(&mut self, _value: i32) {}
        fn p4(&mut self, _value: i32) {}
        fn pjstr(&mut self, _s: &str) {}
    }

    #[derive(Default)]
    pub struct NullDriver {
        pub out: OutSink,
    }

    impl Driver for NullDriver {
        fn set_menu(&mut self, _slot: i32, _action: i32, _a: i32, _b: i32, _c: i32) {}
        fn do_action(&mut self, _slot: i32) -> bool {
            true
        }
        fn try_move(
            &mut self,
            _src_x: i32,
            _src_z: i32,
            _dx: i32,
            _dz: i32,
            _try_nearest: bool,
            _loc_width: i32,
            _loc_length: i32,
            _loc_angle: i32,
            _loc_shape: i32,
            _forceapproach: i32,
            _type: i32,
        ) -> bool {
            true
        }
        fn local_route(&self) -> Option<(i32, i32)> {
            None
        }
        fn build_base(&self) -> (i32, i32) {
            (0, 0)
        }
        fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
            None
        }
        fn out(&mut self) -> &mut dyn Out {
            &mut self.out
        }
        fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
            true
        }
    }
}
