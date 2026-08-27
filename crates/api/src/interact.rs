//! Kernel interaction: menu presses, walks, login/close/answer sends. The
//! kernel talks to the send-side through [`Driver`] so tests can record
//! calls; the real driver maps to `Client::doAction`/`tryMove`/`out` and
//! never injects a raw opcode that skips ISAAC. A `true` return means the
//! driver accepted the send, not that the server applied it.

use client::client::{Client, MiniMenuAction};
use client::io::ClientProt;

use crate::prot::{Out, Send};
use crate::snapshot::{GroundItemView, ItemView, LocView, NpcView, PlayerView, WorldTile};

/// The run-on orb; `set_run(true)` presses it through `doAction` IF_BUTTON.
/// 274 draws it on controls overlay 147 (`controls:com_5`).
pub const RUN_ORB_IFACE: i32 = 153;
/// The run-off orb; `set_run(false)` presses it.
pub const RUN_ORB_OFF: i32 = 152;
/// Lumbridge courtyard hop (`tele` arg). Same as rs2b0t `mainlandAccount`.
pub const OFF_ISLAND_TELE: &str = "0,50,50,20,20";

/// The send-side driver the kernel writes through. `Client` implements it
/// over `doAction`/`tryMove`/`out`; tests use a recording stub.
pub trait Driver {
    /// Write a menu option at `slot` (the `doAction` path).
    fn set_menu(&mut self, slot: i32, action: i32, a: i32, b: i32, c: i32);
    /// Dispatch the menu option at `slot`. Returns true iff the driver
    /// accepted the action.
    fn do_action(&mut self, slot: i32) -> bool;
    /// Queue a walk (the `tryMove` path). Returns true iff a route exists.
    #[allow(clippy::too_many_arguments)] // mirrors the client tryMove signature
    fn try_move(
        &mut self,
        src_x: i32,
        src_z: i32,
        dx: i32,
        dz: i32,
        try_nearest: bool,
        loc_width: i32,
        loc_length: i32,
        loc_angle: i32,
        loc_shape: i32,
        forceapproach: i32,
        r#type: i32,
    ) -> bool;
    /// The route origin tile (local player), in the client's build-area
    /// (scene-relative) space, as `route_x[0]`/`route_z[0]`.
    fn local_route(&self) -> Option<(i32, i32)>;
    /// The scene origin (`map_build_base_x`, `map_build_base_z`): absolute
    /// world tiles are `base + scene` coords, so walk targets and loc
    /// tiles from the nav grid (absolute) translate by subtracting this.
    /// Test drivers return `(0, 0)` so absolute == scene.
    fn build_base(&self) -> (i32, i32);
    /// Packed loc typecode at a scene tile (`wall.typecode` / scenery),
    /// the value `interact_with_loc` matches via `type_code2`. Test
    /// drivers return `None` so [`op_loc`] falls back to the loc id.
    fn loc_typecode(&self, scene_x: i32, scene_z: i32) -> Option<i32>;
    /// The outbound packet sink (ISAAC-encrypted writes only).
    fn out(&mut self) -> &mut dyn Out;
    /// Queue a login handshake. Returns true iff the driver accepted it.
    fn login(&mut self, username: &str, password: &str, reconnect: bool) -> bool;
}

impl Driver for Client {
    fn set_menu(&mut self, slot: i32, action: i32, a: i32, b: i32, c: i32) {
        self.menu_action[slot as usize] = action;
        self.menu_param_a[slot as usize] = a;
        self.menu_param_b[slot as usize] = b;
        self.menu_param_c[slot as usize] = c;
    }

    fn do_action(&mut self, slot: i32) -> bool {
        self.doAction(slot);
        true
    }

    fn try_move(
        &mut self,
        src_x: i32,
        src_z: i32,
        dx: i32,
        dz: i32,
        try_nearest: bool,
        loc_width: i32,
        loc_length: i32,
        loc_angle: i32,
        loc_shape: i32,
        forceapproach: i32,
        r#type: i32,
    ) -> bool {
        self.tryMove(
            src_x,
            src_z,
            dx,
            dz,
            try_nearest,
            loc_width,
            loc_length,
            loc_angle,
            loc_shape,
            forceapproach,
            r#type,
        )
    }

    fn local_route(&self) -> Option<(i32, i32)> {
        self.local_player
            .as_ref()
            .map(|p| (p.route_x[0], p.route_z[0]))
    }

    fn build_base(&self) -> (i32, i32) {
        (self.map_build_base_x, self.map_build_base_z)
    }

    fn loc_typecode(&self, scene_x: i32, scene_z: i32) -> Option<i32> {
        if !(0..104).contains(&scene_x) || !(0..104).contains(&scene_z) {
            return None;
        }
        let level = self.minusedlevel;
        if let Some(w) = self.world.get_wall(level, scene_x, scene_z) {
            return Some(w.typecode);
        }
        if let Some(d) = self.world.get_decor(level, scene_x, scene_z) {
            return Some(d.typecode);
        }
        if let Some(s) = self.world.get_scene(level, scene_x, scene_z) {
            return Some(s.typecode);
        }
        let gd = self.world.gd_type(level, scene_x, scene_z);
        (gd != 0).then_some(gd)
    }

    fn out(&mut self) -> &mut dyn Out {
        &mut self.out
    }

    fn login(&mut self, username: &str, password: &str, reconnect: bool) -> bool {
        Client::login(self, username, password, reconnect).is_ok()
    }
}

/// Dispatch the already-prepared menu option at `slot`.
pub fn interact<D: Driver + ?Sized>(driver: &mut D, slot: i32) -> bool {
    driver.do_action(slot)
}

/// Press an interface button (`IF_BUTTON` on `iface_id`) via the doAction
/// path, so client-code vetoes (logout, social) still apply.
pub fn press<D: Driver + ?Sized>(driver: &mut D, iface_id: i32) -> bool {
    driver.set_menu(0, MiniMenuAction::IF_BUTTON, 0, 0, iface_id);
    driver.do_action(0)
}

/// Set run on (iface 153) or off (iface 152) via the `doAction` IF_BUTTON
/// path. Run state is server-echoed; the caller decides from snapshot
/// state whether to send at all.
pub fn set_run<D: Driver + ?Sized>(driver: &mut D, on: bool) -> bool {
    press(driver, if on { RUN_ORB_IFACE } else { RUN_ORB_OFF })
}

/// Walk to an absolute world tile (the `tryMove` path, plain ground walk),
/// routing from the local player. The client routes in build-area (scene)
/// coordinates — the route head already is, and the absolute target is
/// translated through [`Driver::build_base`] before `try_move`.
pub fn walk<D: Driver + ?Sized>(driver: &mut D, x: i32, z: i32) -> bool {
    let Some((px, pz)) = driver.local_route() else {
        return false;
    };
    let (bx, bz) = driver.build_base();
    driver.try_move(px, pz, x - bx, z - bz, false, 0, 0, 0, 0, 0, 0)
}

/// Interact with a loc via OP_LOC1 through the `doAction` path. The client
/// dispatches `interact_with_loc(b, c, a, OPLOC1)` and looks up `a` with
/// `world.type_code2` (exact typecode match), so the menu carries
/// `a=typecode` (loc id in bits 14..28), `b=x`, `c=z` in scene coordinates.
/// The absolute loc tile is translated through [`Driver::build_base`].
/// When the driver has no typecode at that tile, `a` falls back to `loc_id`
/// (stub drivers).
pub fn op_loc<D: Driver + ?Sized>(driver: &mut D, x: i32, z: i32, loc_id: i32) -> bool {
    let (bx, bz) = driver.build_base();
    let sx = x - bx;
    let sz = z - bz;
    let a = driver.loc_typecode(sx, sz).unwrap_or(loc_id);
    driver.set_menu(0, MiniMenuAction::OP_LOC1, a, sx, sz);
    driver.do_action(0)
}

/// Close the open modal (`CLOSE_MODAL`).
pub fn close_modal<D: Driver + ?Sized>(driver: &mut D) -> bool {
    Send::close_modal().write(driver.out());
    true
}

/// Answer a count dialog with `amount` (`RESUME_P_COUNTDIALOG`).
pub fn answer_count<D: Driver + ?Sized>(driver: &mut D, amount: i32) -> bool {
    Send::count_dialog(amount).write(driver.out());
    true
}

/// Queue a `CLIENT_CHEAT` (`::` command) through the ISAAC sink.
/// `cmd` is the cheat without the `::` prefix (Java `chatInput.substring(2)`).
pub fn cheat<D: Driver + ?Sized>(driver: &mut D, cmd: &str) -> bool {
    let out = driver.out();
    out.p1_enc(ClientProt::CLIENT_CHEAT.id);
    out.p1((cmd.len() + 1) as i32);
    out.pjstr(cmd);
    true
}

/// Tutorial-skip hop used by rs2b0t `mainlandAccount`: tele off the island
/// then `setvar tutorial 1000`. Call after `ingame && scene_state == 2`.
/// Does **not** relog (side icons stay tutorial-locked until campaign 2).
pub fn mainland_hop<D: Driver + ?Sized>(driver: &mut D) {
    let tele = format!("tele {OFF_ISLAND_TELE}");
    cheat(driver, &tele);
    cheat(driver, "setvar tutorial 1000");
}

/// `::tele` argument for an absolute world tile (`level,mx,mz,lx,lz`).
pub fn tele_args(level: i32, x: i32, z: i32) -> String {
    format!(
        "tele {},{},{},{},{}",
        level,
        x.div_euclid(64),
        z.div_euclid(64),
        x.rem_euclid(64),
        z.rem_euclid(64)
    )
}

/// Skip tutorial and `::tele` to an absolute tile, sent through [`cheat`];
/// the host flushes after.
pub fn seed_at<D: Driver + ?Sized>(driver: &mut D, level: i32, x: i32, z: i32) {
    cheat(driver, "setvar tutorial 1000");
    cheat(driver, &tele_args(level, x, z));
}

/// Queue a login through the driver's handshake.
pub fn login<D: Driver + ?Sized>(
    driver: &mut D,
    username: &str,
    password: &str,
    reconnect: bool,
) -> bool {
    driver.login(username, password, reconnect)
}

/// The logout button's client code (`IfType.client_code == 205`); the
/// client vetoes the press server-side, so this is the safe logout path.
pub const CC_LOGOUT: i32 = 205;

/// The slot index of the first iface whose client code is [`CC_LOGOUT`].
pub fn logout_iface_id(ifaces: &[Option<client::config::IfType>]) -> Option<i32> {
    ifaces.iter().enumerate().find_map(|(i, c)| {
        c.as_ref()
            .filter(|c| c.client_code == CC_LOGOUT)
            .map(|_| i as i32)
    })
}

/// Press the logout button (IF_BUTTON on the CC_LOGOUT iface) via the
/// doAction path. Missing iface → `false`, no panic.
pub fn logout<D: Driver + ?Sized>(
    driver: &mut D,
    ifaces: &[Option<client::config::IfType>],
) -> bool {
    let Some(id) = logout_iface_id(ifaces) else {
        return false;
    };
    press(driver, id)
}

// ---------------------------------------------------------------------------
// Wire types (api-v2). `Interactions` (task 9) resolves a `WireCommand` from
// a target/action pair, gates it, and returns a `SendResult`; these enums
// only carry the command + refusal shape — the `Driver` calls stay here.
// ---------------------------------------------------------------------------

/// Which snapshot view an interaction targets. Carries a borrow of the
/// view so identity (`index`/`tile`/`component_id`) survives until send.
#[derive(Debug, Clone)]
pub enum OpTarget<'a> {
    Npc(&'a NpcView),
    Player(&'a PlayerView),
    Loc(&'a LocView),
    GroundItem(&'a GroundItemView),
    Item(&'a ItemView),
}

/// One resolved wire interaction, ready to dispatch through [`Driver`].
/// `Walk` tiles are absolute world tiles (the `tryMove` path translates
/// through [`Driver::build_base`]); `Login` carries the full credential
/// pair so the command is self-contained.
#[derive(Debug, Clone)]
pub enum WireCommand<'a> {
    Op {
        target: OpTarget<'a>,
        operation: i32,
    },
    UseItem {
        select: &'a ItemView,
        target: OpTarget<'a>,
    },
    UseWidget {
        component_id: i32,
        target: OpTarget<'a>,
    },
    Button {
        component_id: i32,
        button_type: i32,
    },
    Continue {
        component_id: i32,
    },
    Close,
    Count {
        value: i32,
    },
    Walk {
        tile: WorldTile,
    },
    SideTab {
        tab: i32,
    },
    Login {
        username: String,
        password: String,
    },
    ClearLocalModal {
        component_id: i32,
    },
}

/// Why a wire interaction was refused before dispatch. Every variant is one
/// precondition or legality check the task-9 gate runs; the host surfaces
/// these verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendReason {
    NotAttached,
    NotIngame,
    SceneUnavailable,
    OffScene,
    LevelMismatch,
    StaleTarget,
    InvalidAction,
    UnsupportedTarget,
    ComponentNotVisible,
    ClientSideOnly,
    TargetMaskMismatch,
    CountDialogOpen,
    NoCountDialog,
    InvalidCount,
    NoModalOpen,
    NoContinue,
    Unreachable,
    InvalidTab,
    AlreadyIngame,
    DriverRejected,
}

/// The outcome of a wire interaction: the command was accepted and sent at
/// `tick`, or refused at `tick` with a [`SendReason`].
#[derive(Debug, Clone)]
pub enum SendResult<'a> {
    Sent {
        tick: u64,
        command: WireCommand<'a>,
    },
    Refused {
        tick: u64,
        reason: SendReason,
    },
}
