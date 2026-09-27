use super::*;
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
    /// Protocol revision captured by the bound session. Legacy recorders and
    /// stubs keep the existing revision-274 behavior by default.
    fn revision(&self) -> ClientRevision {
        ClientRevision::R274
    }
    /// Target captured by a bound session. Legacy recorders retain the existing
    /// process default; real bound clients never re-read it for cheat policy.
    fn session_target(&self) -> client::BotTarget {
        client::bot_target()
    }
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
    /// Dismiss the local amount prompt after a successful count submission,
    /// matching the client's keyboard path. Recorders may have no local UI.
    fn count_dialog_submitted(&mut self) {}
    /// Queue a login handshake. Returns true iff the driver accepted it.
    fn login(&mut self, username: &str, password: &str, reconnect: bool) -> bool;
    /// Switch the active side tab locally, the client's
    /// `handle_tab_clicks` behavior (flip `active_icon` + redraw flags).
    /// Returns false when `tab` is not a bound side icon. Defaults to
    /// false for stubs that do not model the side icons.
    fn click_side_tab(&mut self, _tab: i32) -> bool {
        false
    }
    /// Host-side orbit yaw write (`Client::orbit_camera_yaw`); no opcode.
    /// Default no-op for recording stubs.
    fn set_orbit_camera_yaw(&mut self, _yaw: i32) -> bool {
        false
    }
    /// Local amount prompt (`Client.dialog_input_open`). Stubs have none.
    fn count_dialog_open(&self) -> bool {
        false
    }
    /// Social add/del/PM prompt, which `handle_chat_input` consumes before
    /// the amount dialog. Stubs have none.
    fn social_prompt_open(&self) -> bool {
        false
    }
    /// Whether `GameShell::apply_key` can enqueue without wrapping over an
    /// unread slot of the 128-entry ring. Stubs have no ring.
    fn can_enqueue_key(&self) -> bool {
        true
    }
    /// Existing GameShell key down/up. Default no-op for recorders.
    fn apply_key(&mut self, _down: bool, _java_code: i32, _ch: i32) {}
    /// Existing `Client::handle_chat_input` poll of the GameShell ring.
    fn handle_chat_input(&mut self) {}
    /// Unread GameShell ring entries. Stubs have none.
    fn unread_keys_pending(&self) -> bool {
        false
    }
    /// Existing single-character chat-branch body, without a frame poll.
    fn consume_chat_key(&mut self, _key: i32) {}
    /// Zero native GameShell idle after a successfully accepted input-class
    /// send. Recorders default to no-op; the real client driver writes
    /// `shell.idle_cycles = 0` the same way mouse/key entrypoints do.
    fn note_input_activity(&mut self) {}
}

impl Driver for Client {
    fn revision(&self) -> ClientRevision {
        Client::revision(self)
    }

    fn session_target(&self) -> client::BotTarget {
        Client::session_target(self)
    }
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

    fn count_dialog_submitted(&mut self) {
        self.dialog_input_open = false;
        self.redraw_chat = true;
    }

    fn login(&mut self, username: &str, password: &str, reconnect: bool) -> bool {
        Client::login(self, username, password, reconnect).is_ok()
    }

    fn click_side_tab(&mut self, tab: i32) -> bool {
        // `iconLoop`/`handle_tab_clicks` (client-ts 2787): a tab is only
        // clickable when it has a bound interface; the click selects it
        // and redraws the side panel and icon strips.
        let Some(&bound) = self.side_icon.get(tab as usize) else {
            return false;
        };
        if bound == -1 {
            return false;
        }
        self.active_icon = tab;
        self.redraw_side = true;
        self.redraw_icons = true;
        true
    }

    fn set_orbit_camera_yaw(&mut self, yaw: i32) -> bool {
        self.orbit_camera_yaw = yaw;
        self.orbit_camera_yaw_velocity = 0;
        true
    }

    fn count_dialog_open(&self) -> bool {
        self.dialog_input_open
    }

    fn social_prompt_open(&self) -> bool {
        self.social_input_open
    }

    fn can_enqueue_key(&self) -> bool {
        let next = (self.shell.key_queue_write + 1) & 0x7f;
        next != self.shell.key_queue_read
    }

    fn apply_key(&mut self, down: bool, java_code: i32, ch: i32) {
        self.shell.apply_key(down, java_code, ch);
    }

    fn handle_chat_input(&mut self) {
        Client::handle_chat_input(self);
    }

    fn unread_keys_pending(&self) -> bool {
        self.shell.key_queue_write != self.shell.key_queue_read
    }

    fn consume_chat_key(&mut self, key: i32) {
        Client::consume_chat_key(self, key);
    }

    fn note_input_activity(&mut self) {
        self.shell.idle_cycles = 0;
    }
}

fn mark_input_activity<D: Driver + ?Sized>(driver: &mut D, accepted: bool) -> bool {
    if accepted {
        driver.note_input_activity();
    }
    accepted
}

/// Dispatch the already-prepared menu option at `slot`.
pub fn interact<D: Driver + ?Sized>(driver: &mut D, slot: i32) -> bool {
    let accepted = driver.do_action(slot);
    mark_input_activity(driver, accepted)
}

/// Press an interface button (`IF_BUTTON` on `iface_id`) via the doAction
/// path, so client-code vetoes (logout, social) still apply.
pub fn press<D: Driver + ?Sized>(driver: &mut D, iface_id: i32) -> bool {
    driver.set_menu(0, MiniMenuAction::IF_BUTTON, 0, 0, iface_id);
    let accepted = driver.do_action(0);
    mark_input_activity(driver, accepted)
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
    walk_with_nearest(driver, x, z, false)
}

/// Walk toward an absolute world tile, accepting the client's nearest
/// reachable fallback when the exact tile is blocked.
pub fn walk_nearest<D: Driver + ?Sized>(driver: &mut D, x: i32, z: i32) -> bool {
    walk_with_nearest(driver, x, z, true)
}

fn walk_with_nearest<D: Driver + ?Sized>(
    driver: &mut D,
    x: i32,
    z: i32,
    try_nearest: bool,
) -> bool {
    let Some((px, pz)) = driver.local_route() else {
        return false;
    };
    let (bx, bz) = driver.build_base();
    let accepted = driver.try_move(px, pz, x - bx, z - bz, try_nearest, 0, 0, 0, 0, 0, 0);
    mark_input_activity(driver, accepted)
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
    let accepted = driver.do_action(0);
    mark_input_activity(driver, accepted)
}

/// Close the open modal (`CLOSE_MODAL`).
pub fn close_modal<D: Driver + ?Sized>(driver: &mut D) -> bool {
    let revision = driver.revision();
    let accepted = Send::close_modal().write_for_revision(revision, driver.out());
    mark_input_activity(driver, accepted)
}

/// Answer a count dialog with `amount` (`RESUME_P_COUNTDIALOG`).
pub fn answer_count<D: Driver + ?Sized>(driver: &mut D, amount: i32) -> bool {
    let revision = driver.revision();
    if !Send::count_dialog(amount).write_for_revision(revision, driver.out()) {
        return false;
    }
    driver.count_dialog_submitted();
    mark_input_activity(driver, true)
}

/// Queue a `CLIENT_CHEAT` (`::` command) through the ISAAC sink.
/// `cmd` is the cheat without the `::` prefix (Java `chatInput.substring(2)`).
/// `setstat <skill> 99` for the skills `[debugproc,maxme]` advances.
/// The debug heading and live nav kit share this list — never `~maxme`.
pub const MAXME_SETSTATS: &[&str] = &[
    "setstat attack 99",
    "setstat defence 99",
    "setstat strength 99",
    "setstat hitpoints 99",
    "setstat ranged 99",
    "setstat prayer 99",
    "setstat magic 99",
    "setstat cooking 99",
    "setstat woodcutting 99",
    "setstat fletching 99",
    "setstat fishing 99",
    "setstat firemaking 99",
    "setstat crafting 99",
    "setstat smithing 99",
    "setstat mining 99",
    "setstat herblore 99",
    "setstat agility 99",
    "setstat thieving 99",
    "setstat runecraft 99",
];

pub fn cheat<D: Driver + ?Sized>(driver: &mut D, cmd: &str) -> bool {
    if !cheat_allowed(driver.session_target()) {
        return false;
    }
    let revision = driver.revision();
    let out = driver.out();
    out.p1_enc(map_client_prot(revision, ClientProt::CLIENT_CHEAT).id);
    out.p1((cmd.len() + 1) as i32);
    out.pjstr(cmd);
    true
}

/// `CLIENT_CHEAT`, TutSkip, and mainland hop stay on the local engine.
pub fn cheat_allowed(target: client::BotTarget) -> bool {
    target == client::BotTarget::Local
}

/// Tutorial-skip hop used by rs2b0t `mainlandAccount`: tele off the island
/// then `setvar tutorial 1000`. Call after `ingame && scene_state == 2`.
/// Does **not** relog — side icons stay tutorial-locked until a clean
/// IF_BUTTON logout + login (scenario `StepKind::Relog`).
pub fn mainland_hop<D: Driver + ?Sized>(driver: &mut D) {
    if !cheat_allowed(driver.session_target()) {
        return;
    }
    let tele = format!("tele {OFF_ISLAND_TELE}");
    cheat(driver, &tele);
    cheat(driver, "setvar tutorial 1000");
}

/// Cheat body for an absolute world tile (`tele level,mx,mz,lx,lz`).
/// Not a `~` debugproc.
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

/// Skip tutorial (`setvar tutorial 1000`) and `tele` to an absolute tile,
/// sent through [`cheat`]; the host flushes after.
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
pub fn logout_iface_id(ifaces: &[Option<Box<client::config::IfType>>]) -> Option<i32> {
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
    ifaces: &[Option<Box<client::config::IfType>>],
) -> bool {
    let Some(id) = logout_iface_id(ifaces) else {
        return false;
    };
    press(driver, id)
}
