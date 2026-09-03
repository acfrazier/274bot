//! Kernel interaction: menu presses, walks, login/close/answer sends. The
//! kernel talks to the send-side through [`Driver`] so tests can record
//! calls; the real driver maps to `Client::doAction`/`tryMove`/`out` and
//! never injects a raw opcode that skips ISAAC. A `true` return means the
//! driver accepted the send, not that the server applied it.

use client::client::{Client, MiniMenuAction};
use client::io::ClientProt;

use crate::prot::{Out, Send};
use crate::snapshot::{
    GameSnapshot, GroundItemView, ItemActionFamily, ItemContainer, ItemView, LocView, NpcView,
    PlayerView, ReadContext, SceneView, ToggleControlsView, WidgetView, WorldTile,
};

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
    let out = driver.out();
    out.p1_enc(ClientProt::CLIENT_CHEAT.id);
    out.p1((cmd.len() + 1) as i32);
    out.pjstr(cmd);
    true
}

/// Tutorial-skip hop used by rs2b0t `mainlandAccount`: tele off the island
/// then `setvar tutorial 1000`. Call after `ingame && scene_state == 2`.
/// Does **not** relog — side icons stay tutorial-locked until a clean
/// IF_BUTTON logout + login (scenario `StepKind::Relog`).
pub fn mainland_hop<D: Driver + ?Sized>(driver: &mut D) {
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

// ---------------------------------------------------------------------------
// Wire types. `Interactions` resolves a `WireCommand` from
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
    NoChoice,
    Unreachable,
    InvalidTab,
    AlreadyIngame,
    DriverRejected,
}

/// The outcome of a wire interaction: the command was accepted and sent at
/// `tick`, or refused at `tick` with a [`SendReason`].
#[derive(Debug, Clone)]
pub enum SendResult<'a> {
    Sent { tick: u64, command: WireCommand<'a> },
    Refused { tick: u64, reason: SendReason },
}

// ---------------------------------------------------------------------------
// Interactions orchestration. Above `Driver`: every
// method reads the snapshot, runs the m8aq preconditions (attached/
// ingame/scene/count-dialog), re-checks the target identity, resolves a
// label or operation to a `MiniMenuAction` opcode, and dispatches the
// `WireCommand` through the existing `Driver` calls. The opcode tables
// mirror the m8aq `LiveInteractionDriver` (the client's `MiniMenuAction`
// values); `ActionResolution`/`TargetIdentity` are the `operation_of`/
// `offers_operation`/`still_present` free fns.
// ---------------------------------------------------------------------------

/// The scene-ready state (`scene_state == 2`), the m8aq `SCENE_READY`.
pub const SCENE_READY: i32 = 2;

/// The BUTTON_TARGET button type: the "use widget on target" press arm.
pub const BUTTON_TARGET: i32 = 2;

/// The answer-count ceiling (`i32::MAX`), the m8aq `MAX_COUNT`.
pub const MAX_COUNT: i32 = i32::MAX;

/// How many menu ops a target resolves (the m8aq `MAX_OPERATIONS`).
pub const MAX_OPERATIONS: i32 = 5;

/// The per-kind operation opcode tables (the m8aq `LiveInteractionDriver`
/// `NPC`/`PLAYER`/`LOC`/`OBJ`/`HELD`/`COMPONENT` arrays).
const NPC_OPS: [i32; 5] = [
    MiniMenuAction::OP_NPC1,
    MiniMenuAction::OP_NPC2,
    MiniMenuAction::OP_NPC3,
    MiniMenuAction::OP_NPC4,
    MiniMenuAction::OP_NPC5,
];
const PLAYER_OPS: [i32; 5] = [
    MiniMenuAction::OP_PLAYER1,
    MiniMenuAction::OP_PLAYER2,
    MiniMenuAction::OP_PLAYER3,
    MiniMenuAction::OP_PLAYER4,
    MiniMenuAction::OP_PLAYER5,
];
const LOC_OPS: [i32; 5] = [
    MiniMenuAction::OP_LOC1,
    MiniMenuAction::OP_LOC2,
    MiniMenuAction::OP_LOC3,
    MiniMenuAction::OP_LOC4,
    MiniMenuAction::OP_LOC5,
];
const OBJ_OPS: [i32; 5] = [
    MiniMenuAction::OP_OBJ1,
    MiniMenuAction::OP_OBJ2,
    MiniMenuAction::OP_OBJ3,
    MiniMenuAction::OP_OBJ4,
    MiniMenuAction::OP_OBJ5,
];
const HELD_OPS: [i32; 5] = [
    MiniMenuAction::OP_HELD1,
    MiniMenuAction::OP_HELD2,
    MiniMenuAction::OP_HELD3,
    MiniMenuAction::OP_HELD4,
    MiniMenuAction::OP_HELD5,
];
const COMPONENT_OPS: [i32; 5] = [
    MiniMenuAction::INV_BUTTON1,
    MiniMenuAction::INV_BUTTON2,
    MiniMenuAction::INV_BUTTON3,
    MiniMenuAction::INV_BUTTON4,
    MiniMenuAction::INV_BUTTON5,
];

/// The use-item-on target opcode per kind (the m8aq `USE_ON`).
fn use_on_opcode(target: &OpTarget<'_>) -> i32 {
    match target {
        OpTarget::Npc(_) => MiniMenuAction::USEHELD_ONNPC,
        OpTarget::Player(_) => MiniMenuAction::USEHELD_ONPLAYER,
        OpTarget::Loc(_) => MiniMenuAction::USEHELD_ONLOC,
        OpTarget::GroundItem(_) => MiniMenuAction::USEHELD_ONOBJ,
        OpTarget::Item(_) => MiniMenuAction::USEHELD_ONHELD,
    }
}

/// The use-widget aim opcode per kind (the m8aq `AIM_AT`).
fn aim_at_opcode(target: &OpTarget<'_>) -> i32 {
    match target {
        OpTarget::Npc(_) => MiniMenuAction::TGT_NPC,
        OpTarget::Player(_) => MiniMenuAction::TGT_PLAYER,
        OpTarget::Loc(_) => MiniMenuAction::TGT_LOC,
        OpTarget::GroundItem(_) => MiniMenuAction::TGT_OBJ,
        OpTarget::Item(_) => MiniMenuAction::TGT_HELD,
    }
}

/// The target-mask bit per kind (the m8aq `AIM_BIT`).
fn aim_bit(target: &OpTarget<'_>) -> i32 {
    match target {
        OpTarget::GroundItem(_) => 0x1,
        OpTarget::Npc(_) => 0x2,
        OpTarget::Loc(_) => 0x4,
        OpTarget::Player(_) => 0x8,
        OpTarget::Item(_) => 0x10,
    }
}

/// The button-type → opcode table (the m8aq `BUTTON`).
fn button_opcode(button_type: i32) -> Option<i32> {
    match button_type {
        1 => Some(MiniMenuAction::IF_BUTTON),
        3 => Some(MiniMenuAction::CLOSE_BUTTON),
        4 => Some(MiniMenuAction::TOGGLE_BUTTON),
        5 => Some(MiniMenuAction::SELECT_BUTTON),
        6 => Some(MiniMenuAction::PAUSE_BUTTON),
        _ => None,
    }
}

/// Whether a menu label is a usable action: non-empty after trimming and
/// not the "hidden" slot (the m8aq `usable`).
fn usable(action: Option<&str>) -> bool {
    match action {
        None => false,
        Some(a) => {
            let trimmed = a.trim();
            !trimmed.is_empty() && trimmed.to_lowercase() != "hidden"
        }
    }
}

/// Case- and whitespace-insensitive label equality (the m8aq `matches`
/// string arm; the `Regex` arm is deferred).
fn matches(action: &str, wanted: &str) -> bool {
    action.trim().to_lowercase() == wanted.trim().to_lowercase()
}

/// Resolve `action` to the 1-based operation slot whose menu label
/// matches, scanning in menu order up to [`MAX_OPERATIONS`].
pub fn operation_of(target: &OpTarget<'_>, action: &str) -> Option<i32> {
    let actions = target.actions();
    let limit = actions.len().min(MAX_OPERATIONS as usize);
    for index in 0..limit {
        let candidate = actions.get(index).and_then(|a| a.as_deref());
        if usable(candidate) && matches(candidate.unwrap_or_default(), action) {
            return Some(index as i32 + 1);
        }
    }
    None
}

/// Whether `operation` addresses a usable menu slot of the target.
pub fn offers_operation(target: &OpTarget<'_>, operation: i32) -> bool {
    if !(1..=MAX_OPERATIONS).contains(&operation) {
        return false;
    }
    usable(
        target
            .actions()
            .get((operation - 1) as usize)
            .and_then(|a| a.as_deref()),
    )
}

impl<'a> OpTarget<'a> {
    /// The target's menu labels in menu order (the m8aq `actions`).
    fn actions(&self) -> &[Option<String>] {
        match self {
            OpTarget::Npc(npc) => &npc.actions,
            OpTarget::Player(player) => &player.actor.actions,
            OpTarget::Loc(loc) => &loc.actions,
            OpTarget::GroundItem(ground) => &ground.actions,
            OpTarget::Item(item) => &item.actions,
        }
    }
}

/// A name identity key: trimmed lowercase, `None` stays `None`.
fn canonical_name(name: Option<&str>) -> Option<String> {
    name.map(|n| n.trim().to_lowercase())
}

/// Whether two contained items are the same slot (id + slot + component).
fn same_item_slot(a: &ItemView, b: &ItemView) -> bool {
    a.def.id == b.def.id && a.slot == b.slot && a.component_id == b.component_id
}

/// The container slice an item target's identity is re-checked against.
/// The widget container flattens every widget's item slots (the m8aq
/// `containerOf`; the slice variant avoids allocating).
fn container_items(container: ItemContainer, snapshot: &GameSnapshot) -> &[ItemView] {
    match container {
        ItemContainer::Inventory => snapshot.inventory(),
        ItemContainer::Equipment => snapshot.equipment(),
        ItemContainer::Bank => snapshot.bank(),
        ItemContainer::BankSide => snapshot.bank_side(),
        ItemContainer::TradeMyOffer => &snapshot.trade().my_offer,
        ItemContainer::TradeTheirOffer => &snapshot.trade().their_offer,
        ItemContainer::TradeSidePack => &snapshot.trade().side_pack,
        ItemContainer::ShopStock => &snapshot.shop().stock,
        ItemContainer::Widget => &[],
    }
}

/// Re-check the target's identity against the snapshot per kind (the
/// m8aq `TargetIdentity.stillPresent`): npc slot + type id, remote
/// player slot + canonical name (never the self slot), loc
/// typecode/layer/tile, ground-item id/tile, item id/slot/component.
pub fn still_present(target: &OpTarget<'_>, snapshot: &GameSnapshot) -> bool {
    match target {
        OpTarget::Npc(npc) => snapshot
            .npcs()
            .iter()
            .any(|v| v.index == npc.index && v.r#type == npc.r#type),
        OpTarget::Player(player) => {
            if player.index as i32 == snapshot.self_slot() {
                return false;
            }
            snapshot.players().iter().any(|v| {
                v.index == player.index
                    && canonical_name(v.actor.name.as_deref())
                        == canonical_name(player.actor.name.as_deref())
            })
        }
        OpTarget::Loc(loc) => snapshot.locs().iter().any(|v| {
            v.typecode == loc.typecode
                && v.layer == loc.layer
                && v.tile.x == loc.tile.x
                && v.tile.z == loc.tile.z
                && v.tile.level == loc.tile.level
        }),
        OpTarget::GroundItem(ground) => snapshot.ground_items().iter().any(|v| {
            v.def.id == ground.def.id
                && v.tile.x == ground.tile.x
                && v.tile.z == ground.tile.z
                && v.tile.level == ground.tile.level
        }),
        OpTarget::Item(item) => match item.container {
            ItemContainer::Widget => snapshot
                .widgets()
                .iter()
                .flat_map(|w| w.items.iter())
                .any(|v| same_item_slot(v, item)),
            container => container_items(container, snapshot)
                .iter()
                .any(|v| same_item_slot(v, item)),
        },
    }
}

/// Whether `tile` falls outside the built scene (the m8aq
/// `outsideScene`).
fn outside_scene(scene: &SceneView, tile: WorldTile) -> bool {
    let lx = tile.x - scene.base_x;
    let lz = tile.z - scene.base_z;
    lx < 0 || lz < 0 || lx >= scene.width || lz >= scene.height
}

/// Whether the widget's layer is an open modal root or an available side
/// tab's interface (the m8aq `componentVisible`).
fn component_visible(widget: &WidgetView, snapshot: &GameSnapshot) -> bool {
    let modals = snapshot.modals();
    if widget.layer_id == modals.main
        || widget.layer_id == modals.side
        || widget.layer_id == modals.chat
        || widget.layer_id == modals.tutorial
    {
        return true;
    }
    snapshot
        .side_tabs()
        .iter()
        .any(|tab| tab.available && tab.root_component_id == widget.layer_id)
}

/// How an `Interactions::interact` action is expressed. The `Regex`
/// variant is deferred; `Label` and `Operation` cover the m8aq string/
/// number arms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionSpec {
    Label(String),
    Operation(i32),
}

/// The orchestration layer above [`Driver`]: each method reads the
/// snapshot, runs the preconditions, re-checks the target identity and
/// dispatches the resolved [`WireCommand`] through the `Driver` calls.
/// Holds the snapshot + driver borrows, so callers pass views from the
/// same snapshot the `Interactions` was built with.
pub struct Interactions<'a> {
    snapshot: &'a GameSnapshot,
    driver: &'a mut dyn Driver,
}

impl<'a> Interactions<'a> {
    pub fn new(snapshot: &'a GameSnapshot, driver: &'a mut dyn Driver) -> Self {
        Interactions { snapshot, driver }
    }

    pub fn interact<'t>(&mut self, target: OpTarget<'t>, action: ActionSpec) -> SendResult<'t> {
        let snapshot = self.snapshot;
        if let Some(reason) = self.precondition(snapshot, false) {
            return refuse(snapshot, reason);
        }
        if let Some(reason) = self.check_target(&target, snapshot) {
            return refuse(snapshot, reason);
        }
        let operation = match &action {
            ActionSpec::Operation(n) => offers_operation(&target, *n).then_some(*n),
            ActionSpec::Label(label) => operation_of(&target, label),
        };
        let Some(operation) = operation else {
            return refuse(snapshot, SendReason::InvalidAction);
        };
        self.dispatch(
            WireCommand::Op { target, operation },
            snapshot.tick() as u64,
        )
    }

    /// Wear or wield an inventory item by obj id (the BankBudget
    /// session's arm): resolves the held item, dispatches its `Wear` menu
    /// op (or `Wield` for weapons), with the same preconditions as
    /// [`Interactions::interact`]. Refuses `StaleTarget` when the item is
    /// not held, `InvalidAction` when its menu has no Wear/Wield slot.
    pub fn wear(&mut self, id: i32) -> SendResult<'a> {
        let snapshot = self.snapshot;
        if let Some(reason) = self.precondition(snapshot, false) {
            return refuse(snapshot, reason);
        }
        let Some(item) = snapshot.inventory().iter().find(|it| it.def.id == id) else {
            return refuse(snapshot, SendReason::StaleTarget);
        };
        let target = OpTarget::Item(item);
        if let Some(reason) = self.check_target(&target, snapshot) {
            return refuse(snapshot, reason);
        }
        let operation = operation_of(&target, "Wear").or_else(|| operation_of(&target, "Wield"));
        let Some(operation) = operation else {
            return refuse(snapshot, SendReason::InvalidAction);
        };
        self.dispatch(
            WireCommand::Op { target, operation },
            snapshot.tick() as u64,
        )
    }

    pub fn use_item_on<'t>(&mut self, item: &'t ItemView, target: OpTarget<'t>) -> SendResult<'t> {
        let snapshot = self.snapshot;
        if let Some(reason) = self.precondition(snapshot, false) {
            return refuse(snapshot, reason);
        }
        if item.container != ItemContainer::Inventory {
            return refuse(snapshot, SendReason::UnsupportedTarget);
        }
        if !still_present(&OpTarget::Item(item), snapshot) {
            return refuse(snapshot, SendReason::StaleTarget);
        }
        if let Some(reason) = self.check_target(&target, snapshot) {
            return refuse(snapshot, reason);
        }
        self.dispatch(
            WireCommand::UseItem {
                select: item,
                target,
            },
            snapshot.tick() as u64,
        )
    }

    pub fn use_widget_on<'t>(
        &mut self,
        widget: &WidgetView,
        target: OpTarget<'t>,
    ) -> SendResult<'t> {
        let snapshot = self.snapshot;
        if let Some(reason) = self.precondition(snapshot, false) {
            return refuse(snapshot, reason);
        }
        let ctx = ReadContext::new(snapshot);
        let Some(live) = ctx.component(widget.component_id) else {
            return refuse(snapshot, SendReason::StaleTarget);
        };
        if live.button_type != BUTTON_TARGET {
            return refuse(snapshot, SendReason::InvalidAction);
        }
        if !component_visible(live, snapshot) {
            return refuse(snapshot, SendReason::ComponentNotVisible);
        }
        if (live.target_mask & aim_bit(&target)) == 0 {
            return refuse(snapshot, SendReason::TargetMaskMismatch);
        }
        if let Some(reason) = self.check_target(&target, snapshot) {
            return refuse(snapshot, reason);
        }
        self.dispatch(
            WireCommand::UseWidget {
                component_id: live.component_id,
                target,
            },
            snapshot.tick() as u64,
        )
    }

    pub fn press<'t>(&mut self, widget: &WidgetView) -> SendResult<'t> {
        let snapshot = self.snapshot;
        if let Some(reason) = self.precondition(snapshot, false) {
            return refuse(snapshot, reason);
        }
        let ctx = ReadContext::new(snapshot);
        let Some(live) = ctx.component(widget.component_id) else {
            return refuse(snapshot, SendReason::StaleTarget);
        };
        if live.button_type == 0 || live.button_type == BUTTON_TARGET {
            return refuse(snapshot, SendReason::InvalidAction);
        }
        if live.client_code > 0 {
            return refuse(snapshot, SendReason::ClientSideOnly);
        }
        if !component_visible(live, snapshot) {
            return refuse(snapshot, SendReason::ComponentNotVisible);
        }
        self.dispatch(
            WireCommand::Button {
                component_id: live.component_id,
                button_type: live.button_type,
            },
            snapshot.tick() as u64,
        )
    }

    pub fn continue_dialog<'t>(&mut self) -> SendResult<'t> {
        let snapshot = self.snapshot;
        if let Some(reason) = self.precondition(snapshot, false) {
            return refuse(snapshot, reason);
        }
        let component_id = snapshot.chat_continue_component_id();
        if component_id == -1 {
            return refuse(snapshot, SendReason::NoContinue);
        }
        self.dispatch(
            WireCommand::Continue { component_id },
            snapshot.tick() as u64,
        )
    }

    /// Open the nearest scene loc whose actions include `Use-quickly` on
    /// the player's plane. None → fail closed. Never a Banker Talk-to/Bank.
    pub fn open_nearest_booth(&mut self) -> SendResult<'a> {
        let snapshot = self.snapshot;
        if let Some(reason) = self.precondition(snapshot, false) {
            return refuse(snapshot, reason);
        }
        let Some((px, pz, level)) = snapshot.tile() else {
            return refuse(snapshot, SendReason::OffScene);
        };
        let Some(loc) = snapshot.nearest_use_quickly_booth() else {
            return refuse(snapshot, SendReason::StaleTarget);
        };
        let cheb = (loc.tile.x - px).abs().max((loc.tile.z - pz).abs());
        if cheb > 1 {
            let dest = WorldTile {
                x: loc.tile.x + (px - loc.tile.x).signum(),
                z: loc.tile.z + (pz - loc.tile.z).signum(),
                level,
            };
            return self.walk(dest);
        }
        let Some(op) = operation_of(&OpTarget::Loc(loc), "Use-quickly") else {
            return refuse(snapshot, SendReason::InvalidAction);
        };
        self.interact(OpTarget::Loc(loc), ActionSpec::Operation(op))
    }

    /// Answer the chat modal's `option`-th choice button (1-based, the
    /// `p_choiceN` option an edge's `option` names — e.g. the cart
    /// driver's "Yes please…" fare choice). Refuses with `NoChoice` when
    /// no such choice is up, `StaleTarget` when the live component is
    /// gone, and the same visibility/pressability checks as [`press`].
    pub fn answer_choice<'t>(&mut self, option: i32) -> SendResult<'t> {
        let snapshot = self.snapshot;
        if let Some(reason) = self.precondition(snapshot, false) {
            return refuse(snapshot, reason);
        }
        let Some(choice) = snapshot.chat_options().get((option - 1) as usize) else {
            return refuse(snapshot, SendReason::NoChoice);
        };
        let ctx = ReadContext::new(snapshot);
        let Some(live) = ctx.component(choice.component_id) else {
            return refuse(snapshot, SendReason::StaleTarget);
        };
        if live.button_type == 0 || live.button_type == BUTTON_TARGET {
            return refuse(snapshot, SendReason::InvalidAction);
        }
        if live.client_code > 0 {
            return refuse(snapshot, SendReason::ClientSideOnly);
        }
        if !component_visible(live, snapshot) {
            return refuse(snapshot, SendReason::ComponentNotVisible);
        }
        self.dispatch(
            WireCommand::Button {
                component_id: live.component_id,
                button_type: live.button_type,
            },
            snapshot.tick() as u64,
        )
    }

    pub fn close_modal<'t>(&mut self) -> SendResult<'t> {
        let snapshot = self.snapshot;
        if let Some(reason) = self.precondition(snapshot, false) {
            return refuse(snapshot, reason);
        }
        let modals = snapshot.modals();
        if modals.main == -1 && modals.side == -1 && modals.chat == -1 && modals.tutorial == -1 {
            return refuse(snapshot, SendReason::NoModalOpen);
        }
        self.dispatch(WireCommand::Close, snapshot.tick() as u64)
    }

    pub fn answer_count<'t>(&mut self, value: i32) -> SendResult<'t> {
        let snapshot = self.snapshot;
        if let Some(reason) = self.precondition(snapshot, true) {
            return refuse(snapshot, reason);
        }
        if !snapshot.count_dialog_open() {
            return refuse(snapshot, SendReason::NoCountDialog);
        }
        // `i32` cannot exceed `MAX_COUNT`, so the negative check is the
        // whole m8aq `value < 0 || value > MAX_COUNT` gate here.
        if value < 0 {
            return refuse(snapshot, SendReason::InvalidCount);
        }
        self.dispatch(WireCommand::Count { value }, snapshot.tick() as u64)
    }

    /// Buy `qty` of a shop stock row by name: resolve the stock item,
    /// press the matching Buy inv-button (`Buy 1`/`Buy 5`/`Buy 10`), then
    /// `answer_count` when a count dialog is already up and `qty` is not
    /// one of the fixed buy amounts.
    pub fn shop_buy(&mut self, name: &str, qty: i32) -> SendResult<'a> {
        let snapshot = self.snapshot;
        if let Some(reason) = self.precondition(snapshot, false) {
            return refuse(snapshot, reason);
        }
        if !snapshot.shop().open {
            return refuse(snapshot, SendReason::NoModalOpen);
        }
        if qty < 1 {
            return refuse(snapshot, SendReason::InvalidCount);
        }
        let wanted = name.trim().to_lowercase();
        let Some(item) = snapshot.shop().stock.iter().find(|it| {
            it.def
                .name
                .as_deref()
                .is_some_and(|n| n.trim().to_lowercase() == wanted)
        }) else {
            return refuse(snapshot, SendReason::StaleTarget);
        };
        let target = OpTarget::Item(item);
        if let Some(reason) = self.check_target(&target, snapshot) {
            return refuse(snapshot, reason);
        }
        // iop[0]=Value, iop[1]=Buy 1, iop[2]=Buy 5, iop[3]=Buy 10.
        let operation = match qty {
            1 => 2,
            5 => 3,
            10 => 4,
            _ => 2,
        };
        let result = self.dispatch(
            WireCommand::Op {
                target,
                operation,
            },
            snapshot.tick() as u64,
        );
        match result {
            SendResult::Refused { .. } => result,
            SendResult::Sent { .. }
                if !matches!(qty, 1 | 5 | 10) && snapshot.count_dialog_open() =>
            {
                self.answer_count(qty)
            }
            other => other,
        }
    }

    pub fn walk<'t>(&mut self, tile: WorldTile) -> SendResult<'t> {
        let snapshot = self.snapshot;
        if let Some(reason) = self.precondition(snapshot, false) {
            return refuse(snapshot, reason);
        }
        if tile.level != snapshot.scene().level {
            return refuse(snapshot, SendReason::LevelMismatch);
        }
        if outside_scene(snapshot.scene(), tile) {
            return refuse(snapshot, SendReason::OffScene);
        }
        self.dispatch_with(
            WireCommand::Walk { tile },
            snapshot.tick() as u64,
            SendReason::Unreachable,
        )
    }

    pub fn click_side_tab<'t>(&mut self, tab: i32) -> SendResult<'t> {
        let snapshot = self.snapshot;
        if !snapshot.attached() {
            return refuse(snapshot, SendReason::NotAttached);
        }
        if !snapshot.ingame() {
            return refuse(snapshot, SendReason::NotIngame);
        }
        let available = snapshot
            .side_tabs()
            .iter()
            .any(|t| t.index == tab && t.available);
        if !available {
            return refuse(snapshot, SendReason::InvalidTab);
        }
        self.dispatch(WireCommand::SideTab { tab }, snapshot.tick() as u64)
    }

    pub fn login<'t>(&mut self, username: &str, password: &str) -> SendResult<'t> {
        let snapshot = self.snapshot;
        if !snapshot.attached() {
            return refuse(snapshot, SendReason::NotAttached);
        }
        if snapshot.ingame() {
            return refuse(snapshot, SendReason::AlreadyIngame);
        }
        self.dispatch(
            WireCommand::Login {
                username: username.to_string(),
                password: password.to_string(),
            },
            snapshot.tick() as u64,
        )
    }

    pub fn clear_local_modal<'t>(&mut self, component_id: i32) -> SendResult<'t> {
        let snapshot = self.snapshot;
        if !snapshot.attached() {
            return refuse(snapshot, SendReason::NotAttached);
        }
        let modals = snapshot.modals();
        if modals.main != component_id
            && modals.side != component_id
            && modals.chat != component_id
            && modals.tutorial != component_id
        {
            return refuse(snapshot, SendReason::NoModalOpen);
        }
        self.dispatch(
            WireCommand::ClearLocalModal { component_id },
            snapshot.tick() as u64,
        )
    }

    pub fn set_run<'t>(&mut self, on: bool) -> SendResult<'t> {
        self.set_toggle(self.snapshot.run_controls().copied(), on)
    }

    pub fn set_retaliate<'t>(&mut self, on: bool) -> SendResult<'t> {
        self.set_toggle(self.snapshot.retaliate_controls().copied(), on)
    }

    /// Press the bank Note (on) / Item (off) toggle on the open main
    /// modal. Refuses when the bank is shut or the toggle pair is absent.
    pub fn set_note_mode<'t>(&mut self, on: bool) -> SendResult<'t> {
        self.set_toggle(self.snapshot.bank_note_controls().copied(), on)
    }

    /// The shared `set_run`/`set_retaliate` arm: precondition, the
    /// toggle pair, the on/off component, then the button press.
    fn set_toggle<'t>(&mut self, controls: Option<ToggleControlsView>, on: bool) -> SendResult<'t> {
        let snapshot = self.snapshot;
        if let Some(reason) = self.precondition(snapshot, false) {
            return refuse(snapshot, reason);
        }
        let Some(controls) = controls else {
            return refuse(snapshot, SendReason::ComponentNotVisible);
        };
        let component_id = if on {
            controls.on_component_id
        } else {
            controls.off_component_id
        };
        let ctx = ReadContext::new(snapshot);
        let Some(live) = ctx.component(component_id) else {
            return refuse(snapshot, SendReason::ComponentNotVisible);
        };
        self.dispatch(
            WireCommand::Button {
                component_id,
                button_type: live.button_type,
            },
            snapshot.tick() as u64,
        )
    }

    /// The m8aq `precondition`: attached, ingame, no open count dialog
    /// (unless `allow_count_dialog`), scene available + ready.
    fn precondition(
        &self,
        snapshot: &GameSnapshot,
        allow_count_dialog: bool,
    ) -> Option<SendReason> {
        if !snapshot.attached() {
            return Some(SendReason::NotAttached);
        }
        if !snapshot.ingame() {
            return Some(SendReason::NotIngame);
        }
        if snapshot.count_dialog_open() && !allow_count_dialog {
            return Some(SendReason::CountDialogOpen);
        }
        if !snapshot.scene().available || snapshot.scene_state() != SCENE_READY {
            return Some(SendReason::SceneUnavailable);
        }
        None
    }

    /// The m8aq `checkTarget`: unsupported item families, tile legality
    /// for tile-addressed targets, then the identity re-check.
    fn check_target(&self, target: &OpTarget<'_>, snapshot: &GameSnapshot) -> Option<SendReason> {
        if let OpTarget::Item(item) = target {
            if item.action_family == ItemActionFamily::None {
                return Some(SendReason::UnsupportedTarget);
            }
        }
        if let OpTarget::Loc(loc) = target {
            if loc.tile.level != snapshot.scene().level {
                return Some(SendReason::LevelMismatch);
            }
            if outside_scene(snapshot.scene(), loc.tile) {
                return Some(SendReason::OffScene);
            }
        }
        if let OpTarget::GroundItem(ground) = target {
            if ground.tile.level != snapshot.scene().level {
                return Some(SendReason::LevelMismatch);
            }
            if outside_scene(snapshot.scene(), ground.tile) {
                return Some(SendReason::OffScene);
            }
        }
        if !still_present(target, snapshot) {
            return Some(SendReason::StaleTarget);
        }
        None
    }

    fn dispatch<'t>(&mut self, command: WireCommand<'t>, tick: u64) -> SendResult<'t> {
        self.dispatch_with(command, tick, SendReason::DriverRejected)
    }

    fn dispatch_with<'t>(
        &mut self,
        command: WireCommand<'t>,
        tick: u64,
        rejected_reason: SendReason,
    ) -> SendResult<'t> {
        let accepted = self.send_command(&command);
        if accepted {
            SendResult::Sent { tick, command }
        } else {
            SendResult::Refused {
                tick,
                reason: rejected_reason,
            }
        }
    }

    /// Translate one `WireCommand` into `Driver` calls (the m8aq
    /// `LiveInteractionDriver.dispatch`): op/use/button arms through
    /// `set_menu`/`do_action`, walk through `try_move`, close/side-tab
    /// through the client-local `doAction`/`click_side_tab` arms, count/
    /// login through the `Send`/`Driver` primitives.
    fn send_command(&mut self, command: &WireCommand<'_>) -> bool {
        match command {
            WireCommand::Op { target, operation } => {
                let Some(opcode) = opcode_for(target, *operation) else {
                    return false;
                };
                self.fire(target, opcode)
            }
            WireCommand::UseItem { select, target } => {
                if !self.menu_action(
                    MiniMenuAction::USEHELD_START,
                    select.def.id,
                    select.slot,
                    select.component_id,
                ) {
                    return false;
                }
                if self.fire(target, use_on_opcode(target)) {
                    return true;
                }
                self.menu_action(MiniMenuAction::CANCEL, 0, 0, 0);
                false
            }
            WireCommand::UseWidget {
                component_id,
                target,
            } => {
                if !self.menu_action(MiniMenuAction::TGT_BUTTON, 0, 0, *component_id) {
                    return false;
                }
                if self.fire(target, aim_at_opcode(target)) {
                    return true;
                }
                self.menu_action(MiniMenuAction::CANCEL, 0, 0, 0);
                false
            }
            WireCommand::Button {
                component_id,
                button_type,
            } => {
                let Some(opcode) = button_opcode(*button_type) else {
                    return false;
                };
                self.menu_action(opcode, 0, 0, *component_id)
            }
            WireCommand::Continue { component_id } => {
                self.menu_action(MiniMenuAction::PAUSE_BUTTON, 0, 0, *component_id)
            }
            // The m8aq `close`/`clear-local-modal` arms dispatch
            // CLOSE_BUTTON, so `Client::close_modal()` runs — it clears
            // the client's local modal ids AND writes CLOSE_MODAL.
            WireCommand::Close | WireCommand::ClearLocalModal { .. } => {
                self.menu_action(MiniMenuAction::CLOSE_BUTTON, 0, 0, 0)
            }
            WireCommand::Count { value } => answer_count(&mut *self.driver, *value),
            WireCommand::Walk { tile } => walk(&mut *self.driver, tile.x, tile.z),
            WireCommand::SideTab { tab } => self.driver.click_side_tab(*tab),
            WireCommand::Login { username, password } => {
                login(&mut *self.driver, username, password, false)
            }
        }
    }

    /// `set_menu(0, ...)` + `do_action(0)` (the m8aq `menuAction`).
    fn menu_action(&mut self, action: i32, a: i32, b: i32, c: i32) -> bool {
        self.driver.set_menu(0, action, a, b, c);
        self.driver.do_action(0)
    }

    /// Dispatch `opcode` at a target. The menu params match the client's
    /// `doAction` arms (npc/player slot, item id/slot/component, loc/
    /// ground-item scene coords with the packed typecode or obj id),
    /// mirroring the m8aq `LiveInteractionDriver.fire`.
    fn fire(&mut self, target: &OpTarget<'_>, opcode: i32) -> bool {
        match target {
            OpTarget::Npc(npc) => self.menu_action(opcode, npc.index as i32, 0, 0),
            OpTarget::Player(player) => self.menu_action(opcode, player.index as i32, 0, 0),
            OpTarget::Item(item) => {
                self.menu_action(opcode, item.def.id, item.slot, item.component_id)
            }
            OpTarget::Loc(loc) => {
                let (lx, lz) = self.scene_coords(loc.tile);
                if !self.in_scene(loc.tile.level, lx, lz) {
                    return false;
                }
                self.menu_action(opcode, loc.typecode, lx, lz)
            }
            OpTarget::GroundItem(ground) => {
                let (lx, lz) = self.scene_coords(ground.tile);
                if !self.in_scene(ground.tile.level, lx, lz) {
                    return false;
                }
                self.menu_action(opcode, ground.def.id, lx, lz)
            }
        }
    }

    /// The absolute tile translated through the driver's build base into
    /// scene coordinates.
    fn scene_coords(&self, tile: WorldTile) -> (i32, i32) {
        let (bx, bz) = self.driver.build_base();
        (tile.x - bx, tile.z - bz)
    }

    /// The driver-side scene guard for tile-addressed dispatches (the
    /// m8aq `fire` re-check): level + bounds against the snapshot scene.
    fn in_scene(&self, level: i32, lx: i32, lz: i32) -> bool {
        let scene = self.snapshot.scene();
        if !scene.available || level != scene.level {
            return false;
        }
        lx >= 0 && lz >= 0 && lx < scene.width && lz < scene.height
    }
}

/// Wire `Interactions` over the same snapshot + driver pair (the m8aq
/// `createInteractions`; the Settle wiring lands with the settle task).
pub fn create_interactions<'a>(
    snapshot: &'a GameSnapshot,
    driver: &'a mut dyn Driver,
) -> Interactions<'a> {
    Interactions::new(snapshot, driver)
}

/// A `Refused` result at the snapshot's tick (the m8aq `refuse`).
fn refuse<'t>(snapshot: &GameSnapshot, reason: SendReason) -> SendResult<'t> {
    SendResult::Refused {
        tick: snapshot.tick() as u64,
        reason,
    }
}

/// The per-kind operation opcode for a 1..=MAX_OPERATIONS slot (the m8aq
/// `opcodeFor`).
fn opcode_for(target: &OpTarget<'_>, operation: i32) -> Option<i32> {
    if !(1..=MAX_OPERATIONS).contains(&operation) {
        return None;
    }
    let table: &[i32; 5] = match target {
        OpTarget::Npc(_) => &NPC_OPS,
        OpTarget::Player(_) => &PLAYER_OPS,
        OpTarget::Loc(_) => &LOC_OPS,
        OpTarget::GroundItem(_) => &OBJ_OPS,
        OpTarget::Item(item) => match item.action_family {
            ItemActionFamily::Held => &HELD_OPS,
            ItemActionFamily::Component => &COMPONENT_OPS,
            ItemActionFamily::None => return None,
        },
    };
    Some(table[(operation - 1) as usize])
}
