//! Kernel interaction: menu presses, walks, login/close/answer sends. The
//! kernel talks to the send-side through [`Driver`] so tests can record
//! calls; the real driver maps to `Client::doAction`/`tryMove`/`out` and
//! never injects a raw opcode that skips ISAAC. A `true` return means the
//! driver accepted the send, not that the server applied it.

use client::client::{Client, MiniMenuAction};

use crate::prot::{Out, Send};

/// The run-on orb; `set_run(true)` presses it through `doAction` IF_BUTTON.
/// 274 draws it on controls overlay 147 (`controls:com_5`).
pub const RUN_ORB_IFACE: i32 = 153;
/// The run-off orb; `set_run(false)` presses it.
pub const RUN_ORB_OFF: i32 = 152;

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
    /// The route origin tile (local player).
    fn local_route(&self) -> Option<(i32, i32)>;
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

/// Walk to a tile (the `tryMove` path, plain ground walk), routing from
/// the local player.
pub fn walk<D: Driver + ?Sized>(driver: &mut D, x: i32, z: i32) -> bool {
    let Some((px, pz)) = driver.local_route() else {
        return false;
    };
    driver.try_move(px, pz, x, z, false, 0, 0, 0, 0, 0, 0)
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

/// Queue a login through the driver's handshake.
pub fn login<D: Driver + ?Sized>(
    driver: &mut D,
    username: &str,
    password: &str,
    reconnect: bool,
) -> bool {
    driver.login(username, password, reconnect)
}
