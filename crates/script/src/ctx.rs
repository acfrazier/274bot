//! The compiled `Script` trait and its per-tick context. The driver is the
//! same send-side `api::interact::Driver` the kernel uses; scripts never
//! touch `client` or the world.

use api::interact::Driver;
pub use api::random::{DetectedRandom, RandomClaim};

/// Walk opt-ins a script may pass to [`ScriptCtx::walk_with`]. All default
/// off, mirroring `nav::router::FindOptions` — the `script` crate
/// deliberately takes no `nav` dependency, so the host converts between
/// the two at the hook boundary. `allow_bank_fetch` latches a host
/// BankBudget session when true; JS `Banking.walk` still uses defaults
/// (flag off).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FindOptions {
    pub allow_teleports: bool,
    pub allow_wilderness: bool,
    pub allow_bank_fetch: bool,
}

/// House compiled script. `tick` must return; no delayUntil.
pub trait Script: Send {
    fn name(&self) -> &str;
    fn tick(&mut self, ctx: &mut ScriptCtx<'_>);
    /// Teardown hook, run once by the slot on `stop`, before the instance
    /// is dropped.
    fn on_stop(&mut self) {}
    /// Random-event knock: whether this script claims a detected random
    /// event for itself. Called at most once per rising edge of a detected
    /// event, only while the slot is Running. Default `Host` — the host
    /// guardian talks it through and holds the slot. `Handle` means ticks
    /// and follow keep running and the host does not act.
    fn on_random(&mut self, _ev: &DetectedRandom) -> RandomClaim {
        RandomClaim::Host
    }
}

/// What one observed game-tick gives a script: the send-side driver, the
/// tick number from the pump's PLAYER_INFO edge, the local player's tile,
/// the walk hooks, and the thin inventory view. `here`, `walk`,
/// `walk_with`, `inv`, `snapshot` and `obj_names` are filled by the host's
/// observe; the walk hooks are `None` until the slot wires a traveller and
/// `inv`/`obj_names` are `None` until a body decodes an inventory.
pub struct ScriptCtx<'a> {
    pub driver: &'a mut dyn Driver,
    pub tick: u64,
    /// The local player's absolute world tile `(x, z, level)` on this
    /// observed tick, when the body has decoded one.
    pub here: Option<(i32, i32, i32)>,
    /// Queue one walk toward an absolute world tile `(x, z, level)` through
    /// the slot's traveller with default options (no teleports, no
    /// wilderness, no bank fetch). Returns true iff the walk was queued:
    /// the route is found and armed off-pump on a short-lived worker, so
    /// "true" does not mean a path exists yet. False when no player tile
    /// is known, the host grid is missing, or the uid already has a route
    /// queued.
    pub walk: Option<&'a mut dyn FnMut(i32, i32, i32) -> bool>,
    /// Queue one walk with explicit [`FindOptions`] (teleports/wilderness
    /// opt-in; `allow_bank_fetch` latches a host BankBudget session when
    /// true). Same contract
    /// and return value as `walk`; the host shares one arm between the two
    /// hooks and converts the options.
    pub walk_with: Option<&'a mut dyn FnMut(i32, i32, i32, FindOptions) -> bool>,
    /// The observed inventory `(obj_id, count)` slots, when the body has
    /// decoded one. `None` until an inventory lands (see `has_item`).
    pub inv: Option<&'a [(i32, i32)]>,
    /// The observed tick's [`api::snapshot::GameSnapshot`], filled by the
    /// host's observe; `None` until one is built. Every getter reads it
    /// fail-closed: a missing snapshot (or a missing row) yields `None`,
    /// `false`, or an empty list — never a fake value.
    pub snapshot: Option<&'a api::snapshot::GameSnapshot>,
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

    /// The transmitted value of varp `id`; `None` when the observe
    /// snapshot is absent or the varp was never transmitted (the snapshot
    /// lists only definitions the server sent — an absent id fails
    /// closed, never a fake 0).
    pub fn varp(&self, id: i32) -> Option<i32> {
        let snap = self.snapshot?;
        snap.varps().iter().find(|v| v.index == id).map(|v| v.value)
    }

    /// The effective level of stat `id`: id 16 is the run energy value,
    /// every other id reads the stats table's effective level. `None`
    /// when the observe snapshot is absent or the id is outside the
    /// table (fail-closed).
    pub fn stat_level(&self, id: i32) -> Option<i32> {
        let snap = self.snapshot?;
        match id {
            16 => Some(snap.runenergy()),
            id => snap
                .stats()
                .iter()
                .find(|s| s.index == id)
                .map(|s| s.effective),
        }
    }

    /// The quest journal's colour for the quest named `name`
    /// (case-insensitive); `None` when the observe snapshot is absent or
    /// no journal row matches. The colour is the client-stored value
    /// (`0xF800` green = done, `0xF80000` red = not started, yellow =
    /// started).
    pub fn quest_status(&self, name: &str) -> Option<i32> {
        let snap = self.snapshot?;
        snap.quest_statuses()
            .iter()
            .find(|q| q.name.eq_ignore_ascii_case(name))
            .map(|q| q.colour)
    }

    /// Whether a chat modal is open (the chat modal root is present).
    /// `false` when the observe snapshot is absent.
    pub fn chat_open(&self) -> bool {
        self.snapshot.is_some_and(|s| s.modals().chat != -1)
    }

    /// The chat modal's BUTTON_OK choices, empty when the observe
    /// snapshot is absent or the modal has no choice buttons.
    pub fn chat_options(&self) -> &[api::snapshot::ChatOptionView] {
        match self.snapshot {
            Some(snap) => snap.chat_options(),
            None => &[],
        }
    }

    /// Whether a bank interface is open: the open main modal's withdraw
    /// component is present. `false` when the observe snapshot is absent.
    pub fn bank_open(&self) -> bool {
        self.snapshot.is_some_and(|s| s.bank_component_id() != -1)
    }

    /// Whether the bank's item list has actually been decoded: the open
    /// bank's withdraw component is present and shows at least one item
    /// row. The component appears a beat before the server fills the
    /// list, so an empty list is *not* proof of an empty bank — it means
    /// the list has not loaded yet (fail-closed).
    pub fn bank_loaded(&self) -> bool {
        self.snapshot
            .is_some_and(|s| s.bank_component_id() != -1 && !s.bank().is_empty())
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
    use api::snapshot::GameSnapshot;
    use client::client::{Client, ClientConfig};
    use client::config::if_type::{ButtonType, ComponentType, IfType, IfTypeMut};
    use client::config::{Cache, VarpType};
    use client::io::ServerProt;
    use std::sync::Arc;

    fn cfg() -> ClientConfig {
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: false,
        }
    }

    /// A synthetic mainland client carrying every family the ctx getters
    /// read: a transmitted varp table (101 = 5), run energy 42 and stat 7
    /// at 40, a quest tab (a red "Cook's Assistant" row and a green
    /// "Lost City" row), a chat modal with two BUTTON_OK choices, and an
    /// open bank (a "Withdraw 1" component holding obj 5 × 2).
    fn seeded() -> Client {
        let mut c = Client::new(cfg());
        c.runenergy = 42;
        c.stat_effective_level[7] = 40;
        let cache = Cache {
            varps: (0..102).map(|_| VarpType::default()).collect(),
            ..Default::default()
        };
        c.cache = Arc::new(cache);
        c.var = vec![0; 102];
        c.var[101] = 5;
        // Quest tab (side 2): one red row, one green row.
        c.side_icon[2] = 700;
        c.set_iface(
            700,
            IfType {
                id: 700,
                children: Some(vec![701, 702]),
                ..Default::default()
            },
        );
        c.set_iface(
            701,
            IfType {
                id: 701,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            701,
            IfTypeMut {
                text: "Cook's Assistant".into(),
                colour: 0xF80000,
                ..Default::default()
            },
        );
        c.set_iface(
            702,
            IfType {
                id: 702,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            702,
            IfTypeMut {
                text: "Lost City".into(),
                colour: 0xF800,
                ..Default::default()
            },
        );
        // Chat modal 2000: two BUTTON_OK choices and a continue button.
        c.set_iface(
            2000,
            IfType {
                id: 2000,
                layer_id: 2000,
                r#type: ComponentType::TYPE_LAYER,
                children: Some(vec![2001, 2002, 2003]),
                ..Default::default()
            },
        );
        c.set_iface(
            2001,
            IfType {
                id: 2001,
                layer_id: 2000,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            2001,
            IfTypeMut {
                button_type: ButtonType::BUTTON_OK,
                text: "Yes".into(),
                ..Default::default()
            },
        );
        c.set_iface(
            2002,
            IfType {
                id: 2002,
                layer_id: 2000,
                r#type: ComponentType::TYPE_TEXT,
                button_text: "No thanks".into(),
                ..Default::default()
            },
        );
        c.set_iface_mut(
            2002,
            IfTypeMut {
                button_type: ButtonType::BUTTON_OK,
                ..Default::default()
            },
        );
        c.set_iface(
            2003,
            IfType {
                id: 2003,
                layer_id: 2000,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            2003,
            IfTypeMut {
                button_type: ButtonType::BUTTON_CONTINUE,
                ..Default::default()
            },
        );
        c.chat_modal_id = 2000;
        // Bank: the open main modal's "Withdraw 1" component holding
        // stored 6 (obj 5) × 2.
        c.main_modal_id = 600;
        c.set_iface(
            600,
            IfType {
                id: 600,
                layer_id: 600,
                r#type: ComponentType::TYPE_LAYER,
                children: Some(vec![601]),
                ..Default::default()
            },
        );
        c.set_iface(
            601,
            IfType {
                id: 601,
                layer_id: 600,
                r#type: ComponentType::TYPE_INV,
                iop: [Some("Withdraw 1".into()), None, None, None, None],
                ..Default::default()
            },
        );
        c.set_iface_mut(
            601,
            IfTypeMut {
                link_obj_type: Some(vec![6, 0]),
                link_obj_number: Some(vec![2, 0]),
                ..Default::default()
            },
        );
        for prot in [
            ServerProt::VARP_SYNC,
            ServerProt::UPDATE_RUNENERGY,
            ServerProt::UPDATE_STAT,
            ServerProt::IF_OPENMAIN,
            ServerProt::IF_OPENCHAT,
            ServerProt::UPDATE_INV_FULL,
        ] {
            c.bump_gens(prot);
        }
        c
    }

    fn snap(c: &mut Client) -> GameSnapshot {
        let mut s = GameSnapshot::new();
        s.rebuild(c);
        s
    }

    /// A ctx with every optional view unwired except `snapshot`.
    fn mk_ctx<'a>(driver: &'a mut dyn Driver, snapshot: Option<&'a GameSnapshot>) -> ScriptCtx<'a> {
        ScriptCtx {
            driver,
            tick: 0,
            here: None,
            walk: None,
            walk_with: None,
            inv: None,
            snapshot,
            obj_names: None,
        }
    }

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
            snapshot: None,
            obj_names: Some(&names),
        };
        assert!(ctx.has_item("Bones"));
        assert!(ctx.has_item("bones")); // case-insensitive
        assert!(!ctx.has_item("Vial"));
    }

    #[test]
    fn find_options_default_bank_fetch_off() {
        let o = FindOptions::default();
        assert!(!o.allow_teleports);
        assert!(!o.allow_wilderness);
        assert!(!o.allow_bank_fetch, "BankBudget defaults off");
    }

    #[test]
    fn varp_reads_transmitted_values_and_fails_closed() {
        let mut c = seeded();
        let s = snap(&mut c);
        let mut d = test_support::NullDriver::default();
        let ctx = mk_ctx(&mut d, Some(&s));
        assert_eq!(ctx.varp(101), Some(5));
        assert_eq!(
            ctx.varp(102),
            None,
            "an id beyond the transmitted table fails closed"
        );
        let mut bare_d = test_support::NullDriver::default();
        let bare = mk_ctx(&mut bare_d, None);
        assert_eq!(bare.varp(101), None, "no snapshot fails closed");
    }

    #[test]
    fn stat_level_16_is_run_energy_other_ids_read_effective() {
        let mut c = seeded();
        let s = snap(&mut c);
        let mut d = test_support::NullDriver::default();
        let ctx = mk_ctx(&mut d, Some(&s));
        assert_eq!(ctx.stat_level(16), Some(42), "id 16 reads run energy");
        assert_eq!(
            ctx.stat_level(7),
            Some(40),
            "every other id reads the stats table"
        );
        assert_eq!(ctx.stat_level(99), None, "outside the table fails closed");
        let mut bare_d = test_support::NullDriver::default();
        let bare = mk_ctx(&mut bare_d, None);
        assert_eq!(bare.stat_level(16), None, "no snapshot fails closed");
    }

    #[test]
    fn quest_status_reads_the_journal_colour() {
        let mut c = seeded();
        let s = snap(&mut c);
        let mut d = test_support::NullDriver::default();
        let ctx = mk_ctx(&mut d, Some(&s));
        assert_eq!(
            ctx.quest_status("Lost City"),
            Some(0xF800),
            "the green row is done"
        );
        assert_eq!(
            ctx.quest_status("cook's assistant"),
            Some(0xF80000),
            "case-insensitive; the red row is not done"
        );
        assert_eq!(
            ctx.quest_status("Pirate's Treasure"),
            None,
            "a quest never in the journal fails closed"
        );
        let mut bare_d = test_support::NullDriver::default();
        let bare = mk_ctx(&mut bare_d, None);
        assert_eq!(bare.quest_status("Lost City"), None);
    }

    #[test]
    fn chat_open_and_options_read_the_chat_modal() {
        let mut c = seeded();
        let s = snap(&mut c);
        let mut d = test_support::NullDriver::default();
        let ctx = mk_ctx(&mut d, Some(&s));
        assert!(ctx.chat_open());
        assert_eq!(ctx.chat_options().len(), 2);
        assert_eq!(ctx.chat_options()[0].text, "Yes");
        assert_eq!(ctx.chat_options()[1].text, "No thanks");
        // Closing the modal clears both reads.
        c.chat_modal_id = -1;
        c.bump_gens(ServerProt::IF_OPENCHAT);
        let s = snap(&mut c);
        let mut closed_d = test_support::NullDriver::default();
        let closed = mk_ctx(&mut closed_d, Some(&s));
        assert!(!closed.chat_open());
        assert!(closed.chat_options().is_empty());
        let mut bare_d = test_support::NullDriver::default();
        let bare = mk_ctx(&mut bare_d, None);
        assert!(!bare.chat_open(), "no snapshot fails closed");
        assert!(bare.chat_options().is_empty());
    }

    #[test]
    fn bank_open_and_loaded_distinguish_component_from_items() {
        let mut c = seeded();
        let s = snap(&mut c);
        let mut d = test_support::NullDriver::default();
        let ctx = mk_ctx(&mut d, Some(&s));
        assert!(ctx.bank_open());
        assert!(ctx.bank_loaded(), "the withdraw component has item rows");
        // An open bank whose list has not filled is NOT loaded: an empty
        // item list is not proof of an empty bank.
        c.set_iface_mut(
            601,
            IfTypeMut {
                link_obj_type: Some(vec![0, 0]),
                link_obj_number: Some(vec![0, 0]),
                ..Default::default()
            },
        );
        c.bump_gens(ServerProt::UPDATE_INV_FULL);
        let s = snap(&mut c);
        let mut empty_d = test_support::NullDriver::default();
        let empty = mk_ctx(&mut empty_d, Some(&s));
        assert!(empty.bank_open(), "the component is still present");
        assert!(!empty.bank_loaded(), "empty list ≠ empty bank");
        let mut bare_d = test_support::NullDriver::default();
        let bare = mk_ctx(&mut bare_d, None);
        assert!(!bare.bank_open(), "no snapshot fails closed");
        assert!(!bare.bank_loaded());
    }
}
