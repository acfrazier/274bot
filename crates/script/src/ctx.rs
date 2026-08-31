//! The compiled `Script` trait and its per-tick context. The driver is the
//! same send-side `api::interact::Driver` the kernel uses; scripts never
//! touch `client` or the world.

use api::interact::Driver;

/// Walk opt-ins a script may pass to [`ScriptCtx::walk_with`]. Both
/// default off, mirroring `nav::router::FindOptions` — the `script` crate
/// deliberately takes no `nav` dependency, so the host converts between
/// the two at the hook boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FindOptions {
    pub allow_teleports: bool,
    pub allow_wilderness: bool,
}

/// House compiled script. `tick` must return; no delayUntil.
pub trait Script: Send {
    fn name(&self) -> &str;
    fn tick(&mut self, ctx: &mut ScriptCtx<'_>);
    /// Teardown hook, run once by the slot on `stop`, before the instance
    /// is dropped.
    fn on_stop(&mut self) {}
}

/// What one observed game-tick gives a script: the send-side driver, the
/// tick number from the pump's PLAYER_INFO edge, the local player's tile,
/// the walk hooks, and the thin inventory view. `here`, `walk`,
/// `walk_with`, `inv` and `obj_names` are filled by the host's observe;
/// the walk hooks are `None` until the slot wires a traveller and
/// `inv`/`obj_names` are `None` until a body decodes an inventory.
pub struct ScriptCtx<'a> {
    pub driver: &'a mut dyn Driver,
    pub tick: u64,
    /// The local player's absolute world tile `(x, z, level)` on this
    /// observed tick, when the body has decoded one.
    pub here: Option<(i32, i32, i32)>,
    /// Queue one walk toward an absolute world tile `(x, z, level)` through
    /// the slot's traveller with default options (no teleports, no
    /// wilderness). Returns true iff the walk was queued: the
    /// route is found and armed off-pump on a short-lived worker, so
    /// "true" does not mean a path exists yet. False when no player tile
    /// is known, the host grid is missing, or the uid already has a route
    /// queued.
    pub walk: Option<&'a mut dyn FnMut(i32, i32, i32) -> bool>,
    /// Queue one walk with explicit [`FindOptions`] (teleports/wilderness
    /// opt-in). Same contract and return value as `walk`; the host shares
    /// one arm between the two hooks and converts the options.
    pub walk_with: Option<&'a mut dyn FnMut(i32, i32, i32, FindOptions) -> bool>,
    /// The observed inventory `(obj_id, count)` slots, when the body has
    /// decoded one. `None` until an inventory lands (see `has_item`).
    pub inv: Option<&'a [(i32, i32)]>,
    /// The shared obj-id → name table (one per `Play`), resolved by
    /// [`ScriptCtx::has_item`].
    pub obj_names: Option<&'a api::obj_names::ObjNames>,
}

impl ScriptCtx<'_> {
    /// Whether the observed inventory holds an object whose resolved name
    /// equals `name` (case-insensitive). `false` when the body has not
    /// decoded an inventory (`inv` or `obj_names` is `None`) — never a
    /// panic on an unwired observe.
    pub fn has_item(&self, name: &str) -> bool {
        let (Some(inv), Some(names)) = (self.inv, self.obj_names) else {
            return false;
        };
        inv.iter().any(|(id, _count)| {
            names
                .name(*id)
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_item_resolves_inventory_by_name() {
        let mut objs = vec![client::config::ObjType::default(); 3];
        objs[1].id = 1;
        objs[1].name = "Bones".into();
        let names = api::obj_names::ObjNames::from_objs(&objs);
        let inv: Vec<(i32, i32)> = vec![(1, 3)];
        let mut d = test_support::NullDriver::default();
        let ctx = ScriptCtx {
            driver: &mut d,
            tick: 0,
            here: None,
            walk: None,
            walk_with: None,
            inv: Some(&inv),
            obj_names: Some(&names),
        };
        assert!(ctx.has_item("Bones"));
        assert!(ctx.has_item("bones")); // case-insensitive
        assert!(!ctx.has_item("Vial"));
    }
}
