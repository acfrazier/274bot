//! The Luau API: one static table and one thin function for each member
//! (ADR 0023 items 3, 19 and 20; `LUAU-DESIGN-1114/design-r3.md` 7.1, 7.4).
//!
//! A binding converts its arguments, makes at most one call on its `host`,
//! and converts the result. It holds no logic of its own. A read binding
//! reads the lent frame and makes no call. `tests/thinness.rs` parses this
//! directory and keeps it so.

pub(crate) mod convert;
mod host;

pub(crate) use host::{Frame, Host};

use convert::{Args, Out, Refusal, Reply, WalkShape};
use mlua::{Lua, MultiValue};
use script::bank::request::BankSeatOperation;

/// The member classes of `design-r3.md` 7.1 item 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Reads the lent frame. No call on the host.
    Read,
    /// One host call that returns at once.
    Request,
    /// One host call that starts a wait. The prelude yields after it.
    Await,
    /// Writes run output. It takes every argument.
    Output,
}

/// How the registration function reaches a binding.
#[derive(Clone, Copy)]
pub(crate) enum Call {
    Frame(fn(&Lua, &Frame<'_>, &Args) -> Reply),
    Host(fn(&Lua, Host<'_>, &Args) -> Reply),
    Output(fn(&Lua, Host<'_>, &MultiValue) -> Reply),
}

/// One row of [`BINDINGS`].
pub(crate) struct Binding {
    /// The path under `api`, with `.` between a group and a member.
    pub(crate) path: &'static str,
    pub(crate) kind: Kind,
    /// The largest argument count. An `Output` row takes any count.
    pub(crate) arity: usize,
    pub(crate) call: Call,
    /// The Luau type of the member.
    pub(crate) signature: &'static str,
    pub(crate) doc: &'static str,
    /// The JS API v2 member with the same meaning, as `Interface.member` of
    /// `crates/script-js/host-js/index.d.ts` (`NativeOp:op` for a request
    /// op). `Snapshot` is the posted snapshot that both surfaces read.
    pub(crate) js: Option<&'static str>,
}

/// One public projection of a row, for the census and the type generator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApiMember {
    pub path: &'static str,
    pub kind: Kind,
    pub signature: &'static str,
    pub doc: &'static str,
    pub js: Option<&'static str>,
}

/// Every member that a VM exposes under `api`, in table order.
pub fn api_members() -> impl Iterator<Item = ApiMember> {
    BINDINGS.iter().map(|row| ApiMember {
        path: row.path,
        kind: row.kind,
        signature: row.signature,
        doc: row.doc,
        js: row.js,
    })
}

pub(crate) static BINDINGS: &[Binding] = &[
    Binding {
        path: "log",
        kind: Kind::Output,
        arity: usize::MAX,
        call: Call::Output(log),
        signature: "(...any) -> ()",
        doc: "Write one log line: the values joined by tabs.",
        js: Some("NativeApi.log"),
    },
    Binding {
        path: "stop",
        kind: Kind::Request,
        arity: 1,
        call: Call::Host(stop),
        signature: "(reason: string?) -> never",
        doc: "Stop the script. The entry ends at the call; onStop runs once.",
        js: Some("NativeApi.stop"),
    },
    Binding {
        path: "wait",
        kind: Kind::Await,
        arity: 0,
        call: Call::Host(wait),
        signature: "() -> ()",
        doc: "Suspend tick until the next unheld game tick.",
        js: None,
    },
    Binding {
        path: "settings.str",
        kind: Kind::Request,
        arity: 2,
        call: Call::Host(settings_str),
        signature: "(key: string, fallback: string?) -> string",
        doc: "The text setting `key`, or `fallback` (\"\" when absent).",
        js: Some("NativeSettings.str"),
    },
    Binding {
        path: "settings.num",
        kind: Kind::Request,
        arity: 2,
        call: Call::Host(settings_num),
        signature: "(key: string, fallback: number?) -> number",
        doc: "The number setting `key`, or `fallback` (0 when absent).",
        js: Some("NativeSettings.num"),
    },
    Binding {
        path: "settings.bool",
        kind: Kind::Request,
        arity: 2,
        call: Call::Host(settings_bool),
        signature: "(key: string, fallback: boolean?) -> boolean",
        doc: "The boolean setting `key`, or `fallback` (false when absent).",
        js: Some("NativeSettings.bool"),
    },
    Binding {
        path: "status.tick",
        kind: Kind::Read,
        arity: 0,
        call: Call::Frame(status_tick),
        signature: "() -> number",
        doc: "The game tick of the lent input.",
        js: Some("NativeApi.tick"),
    },
    Binding {
        path: "status.inGame",
        kind: Kind::Read,
        arity: 0,
        call: Call::Frame(status_in_game),
        signature: "() -> boolean",
        doc: "Whether the client is in game.",
        js: Some("NativeSnapshot.ingame"),
    },
    Binding {
        path: "status.tile",
        kind: Kind::Read,
        arity: 0,
        call: Call::Frame(status_tile),
        signature: "() -> (number?, number?, number?)",
        doc: "The player's x, z and level, or nil when the host has no tile.",
        js: Some("NativeSnapshot.here"),
    },
    Binding {
        path: "status.runEnergy",
        kind: Kind::Read,
        arity: 0,
        call: Call::Frame(status_run_energy),
        signature: "() -> number",
        doc: "The run energy.",
        js: Some("Snapshot.run_energy"),
    },
    Binding {
        path: "status.runEnabled",
        kind: Kind::Read,
        arity: 0,
        call: Call::Frame(status_run_enabled),
        signature: "() -> boolean",
        doc: "Whether run is on.",
        js: Some("Snapshot.run_enabled"),
    },
    Binding {
        path: "status.inCombat",
        kind: Kind::Read,
        arity: 0,
        call: Call::Frame(status_in_combat),
        signature: "() -> boolean",
        doc: "Whether the player is in combat.",
        js: Some("Snapshot.in_combat"),
    },
    Binding {
        path: "status.animating",
        kind: Kind::Read,
        arity: 0,
        call: Call::Frame(status_animating),
        signature: "() -> boolean",
        doc: "Whether the player animates.",
        js: Some("Snapshot.animating"),
    },
    Binding {
        path: "status.name",
        kind: Kind::Read,
        arity: 0,
        call: Call::Frame(status_name),
        signature: "() -> string?",
        doc: "The player's name, or nil.",
        js: Some("Snapshot.my_name"),
    },
    Binding {
        path: "walk.to",
        kind: Kind::Await,
        arity: 4,
        call: Call::Host(walk_to),
        signature: "(x: number, z: number, level: number, options: WalkOptions?) -> WalkResult",
        doc: "Walk to the exact tile and wait for the walk outcome.",
        js: Some("NativeOp:walk"),
    },
    Binding {
        path: "walk.near",
        kind: Kind::Await,
        arity: 5,
        call: Call::Host(walk_near),
        signature:
            "(x: number, z: number, level: number, radius: number, options: WalkOptions?) -> WalkResult",
        doc: "Walk to within `radius` tiles and wait for the walk outcome.",
        js: Some("NativeOp:walk-near"),
    },
    Binding {
        path: "walk.nearestBank",
        kind: Kind::Request,
        arity: 0,
        call: Call::Host(walk_nearest_bank),
        signature: "() -> ()",
        doc: "Queue a walk to the nearest packed bank stand.",
        js: Some("NativeOp:walk-nearest-bank"),
    },
    Binding {
        path: "bank.openNearest",
        kind: Kind::Await,
        arity: 0,
        call: Call::Host(bank_open_nearest),
        signature: "() -> BankResult",
        doc: "Open the nearest world bank; return its typed seat status and value.",
        js: None,
    },
    Binding {
        path: "bank.depositAll",
        kind: Kind::Await,
        arity: 0,
        call: Call::Host(bank_deposit_all),
        signature: "() -> BankResult",
        doc: "Deposit all bankable backpack items; return the typed seat status and value.",
        js: None,
    },
    Binding {
        path: "bank.close",
        kind: Kind::Await,
        arity: 0,
        call: Call::Host(bank_close),
        signature: "() -> BankResult",
        doc: "Close the current bank; return the typed seat status and value.",
        js: None,
    },
    Binding {
        path: "bank.openBooth",
        kind: Kind::Request,
        arity: 5,
        call: Call::Host(bank_open_booth),
        signature: "(x: number, z: number, level: number, id: number, options: OpenBoothOptions?) -> ()",
        doc: "Queue opening the exact bank booth loc at `(x, z, level)` with `id`.",
        js: Some("NativeOp:open-booth"),
    },
    Binding {
        path: "bank.openStand",
        kind: Kind::Request,
        arity: 5,
        call: Call::Host(bank_open_stand),
        signature: "(x: number, z: number, level: number, kind: string, options: OpenStandOptions?) -> ()",
        doc: "Queue opening a packed bank stand (`booth` or `npc`) at `(x, z, level)`.",
        js: Some("NativeOp:open-stand"),
    },
    Binding {
        path: "bank.closeNow",
        kind: Kind::Request,
        arity: 0,
        call: Call::Host(bank_close_now),
        signature: "() -> ()",
        doc: "Queue closing the open bank interface immediately.",
        js: Some("NativeOp:close"),
    },
    Binding {
        path: "bank.setNoteMode",
        kind: Kind::Request,
        arity: 1,
        call: Call::Host(bank_set_note_mode),
        signature: "(on: boolean) -> ()",
        doc: "Queue toggling bank withdraw-as-note mode.",
        js: Some("NativeOp:set-note-mode"),
    },
    Binding {
        path: "bank.deposit",
        kind: Kind::Request,
        arity: 1,
        call: Call::Host(bank_deposit),
        signature: "(name: string) -> ()",
        doc: "Queue depositing all copies of the named item into the open bank.",
        js: Some("NativeOp:deposit"),
    },
    Binding {
        path: "bank.withdraw",
        kind: Kind::Request,
        arity: 2,
        call: Call::Host(bank_withdraw),
        signature: "(name: string, action: string) -> ()",
        doc: "Queue withdrawing the named item from the open bank with `action`.",
        js: Some("NativeOp:withdraw"),
    },
    Binding {
        path: "bank.withdrawX",
        kind: Kind::Request,
        arity: 1,
        call: Call::Host(bank_withdraw_x),
        signature: "(request: WithdrawXRequest) -> ()",
        doc: "Queue a host-owned counted bank withdrawal for the current bank generation.",
        js: Some("NativeOp:withdraw-x"),
    },
    Binding {
        path: "bank.generation",
        kind: Kind::Read,
        arity: 0,
        call: Call::Frame(bank_generation),
        signature: "() -> number",
        doc: "The current bank session generation from the lent snapshot.",
        js: Some("NativeSnapshot.bank_generation"),
    },
    Binding {
        path: "dialog.continue",
        kind: Kind::Request,
        arity: 0,
        call: Call::Host(dialog_continue),
        signature: "() -> ()",
        doc: "Queue the current dialog's Continue option.",
        js: None,
    },
    Binding {
        path: "dialog.answer",
        kind: Kind::Request,
        arity: 1,
        call: Call::Host(dialog_answer),
        signature: "(option: number) -> ()",
        doc: "Queue a 1-based dialog option.",
        js: None,
    },
    Binding {
        path: "equipment.remove",
        kind: Kind::Request,
        arity: 1,
        call: Call::Host(equipment_remove),
        signature: "(name: string) -> ()",
        doc: "Queue removing the named worn item.",
        js: None,
    },
    Binding {
        path: "equipment.wear",
        kind: Kind::Request,
        arity: 1,
        call: Call::Host(equipment_wear),
        signature: "(name: string) -> ()",
        doc: "Queue wearing the named carried item.",
        js: None,
    },
    Binding {
        path: "interface.button",
        kind: Kind::Request,
        arity: 1,
        call: Call::Host(interface_button),
        signature: "(component_id: number) -> ()",
        doc: "Queue clicking the identified interface component.",
        js: None,
    },
    Binding {
        path: "inventory.option",
        kind: Kind::Request,
        arity: 2,
        call: Call::Host(inventory_option),
        signature: "(name: string, option: number) -> ()",
        doc: "Queue the named carried item's 1-based option.",
        js: None,
    },
    Binding {
        path: "inventory.useOn",
        kind: Kind::Request,
        arity: 2,
        call: Call::Host(inventory_use_on),
        signature: "(source: string, target: string) -> ()",
        doc: "Queue using one carried item on another by name.",
        js: None,
    },
    Binding {
        path: "loc.interact",
        kind: Kind::Request,
        arity: 2,
        call: Call::Host(loc_interact),
        signature: "(tile: Tile, action: string) -> ()",
        doc: "Queue the loc option at the exact tile, without an id fallback.",
        js: None,
    },

    Binding {
        path: "interact.npc",
        kind: Kind::Request,
        arity: 3,
        call: Call::Host(interact_npc),
        signature: "(name: string, action: string, index: number?) -> ()",
        doc: "Queue one NPC action by name, optionally at an exact index.",
        js: None,
    },
    Binding {
        path: "interact.loc",
        kind: Kind::Request,
        arity: 5,
        call: Call::Host(interact_loc),
        signature: "(x: number, z: number, level: number, action: string, id: number) -> ()",
        doc: "Queue one loc action at the exact tile and loc id.",
        js: None,
    },
    Binding {
        path: "interact.obj",
        kind: Kind::Request,
        arity: 5,
        call: Call::Host(interact_obj),
        signature: "(x: number, z: number, level: number, action: string, id: number) -> ()",
        doc: "Queue one ground-item action at the exact tile and item id.",
        js: None,
    },
    Binding {
        path: "interact.player",
        kind: Kind::Request,
        arity: 2,
        call: Call::Host(interact_player),
        signature: "(name: string, action: string) -> ()",
        doc: "Queue one player action by name.",
        js: None,
    },
    Binding {
        path: "interact.held",
        kind: Kind::Request,
        arity: 4,
        call: Call::Host(interact_held),
        signature:
            "(name: string, action: string, slot: number, target_item_id: number) -> ()",
        doc: "Queue one held-item action by exact item slot and target id.",
        js: None,
    },
    Binding {
        path: "interact.heldByName",
        kind: Kind::Request,
        arity: 2,
        call: Call::Host(interact_held_by_name),
        signature: "(name: string, action: string) -> ()",
        doc: "Queue one held-item action by item name and action label.",
        js: Some("NativeOp:held"),
    },
    Binding {
        path: "route.inspect",
        kind: Kind::Request,
        arity: 3,
        call: Call::Host(route_inspect),
        signature: "(from: Tile, to: Tile, options: InspectRouteOptions?) -> ()",
        doc: "Queue one route-inspection query between `from` and `to`.",
        js: Some("NativeOp:inspect-route"),
    },
    Binding {
        path: "route.result",
        kind: Kind::Read,
        arity: 0,
        call: Call::Frame(route_result),
        signature: "() -> RouteInspectResult?",
        doc: "Read the latest route-inspection result from the lent snapshot, or nil.",
        js: Some("NativeSnapshot.route_inspect_seq"),
    },
    Binding {
        path: "puzzle.move",
        kind: Kind::Request,
        arity: 4,
        call: Call::Host(puzzle_move),
        signature: "(id: number, slot: number, component: number, generation: number) -> ()",
        doc: "Queue sliding one piece on the open puzzle board under `generation`.",
        js: Some("NativeOp:puzzle-move"),
    },
    Binding {
        path: "puzzle.board",
        kind: Kind::Read,
        arity: 0,
        call: Call::Frame(puzzle_board),
        signature: "() -> PuzzleBoard?",
        doc: "Read the open puzzle board from the lent snapshot, or nil.",
        js: Some("NativeSnapshot.puzzle_board"),
    },
    Binding {
        path: "combat.fight",
        kind: Kind::Await,
        arity: 1,
        call: Call::Host(combat_fight),
        signature: "(request: CombatRequest) -> CombatOutcome",
        doc: "Run one native combat fight and return its typed outcome.",
        js: Some("NativeApi.combat.fight"),
    },
    Binding {
        path: "gather.run",
        kind: Kind::Await,
        arity: 1,
        call: Call::Host(gather_run),
        signature: "(settings: GatherSettings?) -> GatherOutcome",
        doc: "Run one native Gatherer session and return its typed outcome.",
        js: Some("NativeApi.gather.run"),
    },
    Binding {
        path: "questStatus",
        kind: Kind::Read,
        arity: 1,
        call: Call::Frame(quest_status),
        signature: "(name: string) -> QuestStatusResult",
        doc: "Read one quest-tab row status from the lent snapshot.",
        js: Some("NativeApi.questStatus"),
    },
    Binding {
        path: "questPaths",
        kind: Kind::Read,
        arity: 0,
        call: Call::Frame(quest_paths),
        signature: "() -> { QuestPathRow }",
        doc: "Read the index of released quest Paths.",
        js: Some("NativeApi.questPaths"),
    },
    Binding {
        path: "questProgress",
        kind: Kind::Await,
        arity: 1,
        call: Call::Host(quest_progress),
        signature: "(quest: string) -> QuestProgressOutcome",
        doc: "Run one owned quest progress read and return its typed progress page.",
        js: Some("NativeApi.questProgress"),
    },
];

fn log(_lua: &Lua, host: Host<'_>, args: &MultiValue) -> Reply {
    Ok(Out::from(host.log(convert::log_line(args)?)?))
}

fn stop(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    Ok(Out::from(host.stop(convert::stop_reason(args.first())?)?))
}

fn wait(_lua: &Lua, host: Host<'_>, _args: &Args) -> Reply {
    Ok(Out::from(host.wait_next_tick()?))
}

fn settings_str(lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    let fallback = convert::text_fallback(args.second(), "fallback")?;
    convert::setting_text(lua, host.setting(&convert::key(args.first())?), fallback)
}

fn settings_num(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    let fallback = convert::number_fallback(args.second(), "fallback")?;
    Ok(convert::setting_number(
        host.setting(&convert::key(args.first())?),
        fallback,
    ))
}

fn settings_bool(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    let fallback = convert::boolean_fallback(args.second(), "fallback")?;
    Ok(convert::setting_boolean(
        host.setting(&convert::key(args.first())?),
        fallback,
    ))
}

fn status_tick(_lua: &Lua, frame: &Frame<'_>, _args: &Args) -> Reply {
    Ok(convert::unsigned_out(frame.input.tick))
}

fn status_in_game(_lua: &Lua, frame: &Frame<'_>, _args: &Args) -> Reply {
    Ok(convert::boolean_out(frame.input.ingame))
}

fn status_tile(_lua: &Lua, frame: &Frame<'_>, _args: &Args) -> Reply {
    Ok(convert::tile_out(frame.input.here))
}

fn status_run_energy(_lua: &Lua, frame: &Frame<'_>, _args: &Args) -> Reply {
    Ok(convert::integer_out(frame.input.run_energy))
}

fn status_run_enabled(_lua: &Lua, frame: &Frame<'_>, _args: &Args) -> Reply {
    Ok(convert::boolean_out(frame.input.run_enabled))
}

fn status_in_combat(_lua: &Lua, frame: &Frame<'_>, _args: &Args) -> Reply {
    Ok(convert::boolean_out(frame.input.in_combat))
}

fn status_animating(_lua: &Lua, frame: &Frame<'_>, _args: &Args) -> Reply {
    Ok(convert::boolean_out(frame.input.animating))
}

fn status_name(lua: &Lua, frame: &Frame<'_>, _args: &Args) -> Reply {
    convert::text_out(lua, frame.input.my_name)
}

fn walk_to(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    host.walk(convert::walk_request(args, WalkShape::Exact)?)
        .map(Out::from)
}

fn walk_near(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    host.walk(convert::walk_request(args, WalkShape::Near)?)
        .map(Out::from)
}

fn walk_nearest_bank(_lua: &Lua, host: Host<'_>, _args: &Args) -> Reply {
    host.interact(convert::walk_nearest_bank_request())
        .map(Out::from)
}

fn bank_open_nearest(_lua: &Lua, host: Host<'_>, _args: &Args) -> Reply {
    Ok(Out::from(host.bank(convert::bank_request(
        BankSeatOperation::OpenNearestWorld,
    ))?))
}

fn bank_deposit_all(_lua: &Lua, host: Host<'_>, _args: &Args) -> Reply {
    Ok(Out::from(host.bank(convert::bank_request(
        BankSeatOperation::DepositAll,
    ))?))
}

fn bank_close(_lua: &Lua, host: Host<'_>, _args: &Args) -> Reply {
    host.bank(convert::bank_request(BankSeatOperation::Close))
        .map(Out::from)
}

fn bank_open_booth(lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    host.interact(convert::open_booth_request(lua, args)?)
        .map(Out::from)
}

fn bank_open_stand(lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    host.interact(convert::open_stand_request(lua, args)?)
        .map(Out::from)
}

fn bank_close_now(_lua: &Lua, host: Host<'_>, _args: &Args) -> Reply {
    Ok(Out::from(host.interact(convert::close_request())?))
}

fn bank_set_note_mode(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    host.interact(convert::set_note_mode_request(args)?)
        .map(Out::from)
}

fn bank_deposit(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    Ok(Out::from(host.interact(convert::deposit_request(args)?)?))
}

fn bank_withdraw(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    Ok(Out::from(host.interact(convert::withdraw_request(args)?)?))
}

fn bank_withdraw_x(lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    host.interact(convert::withdraw_x_request(lua, args)?)
        .map(Out::from)
}

fn bank_generation(_lua: &Lua, frame: &Frame<'_>, _args: &Args) -> Reply {
    Ok(convert::unsigned_out(frame.input.bank_generation))
}

fn interact_npc(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    Ok(Out::from(host.interact(convert::npc_request(args)?)?))
}

fn interact_loc(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    Ok(Out::from(host.interact(convert::loc_request(args)?)?))
}

fn interact_obj(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    Ok(Out::from(host.interact(convert::obj_request(args)?)?))
}

fn interact_player(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    Ok(Out::from(host.interact(convert::player_request(args)?)?))
}

fn interact_held(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    Ok(Out::from(host.interact(convert::held_request(args)?)?))
}

fn interact_held_by_name(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    host.interact(convert::held_by_name_request(args)?)
        .map(Out::from)
}

fn inventory_use_on(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    let frame = host.frame.ok_or(Refusal::NoInput)?;
    let request = convert::use_on_request(args, frame.input, host.game_data)?;
    Ok(Out::from(host.interact(request)?))
}

fn inventory_option(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    let frame = host.frame.ok_or(Refusal::NoInput)?;
    let request = convert::option_request(args, frame.input, host.game_data)?;
    Ok(Out::from(host.interact(request)?))
}

fn equipment_wear(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    let frame = host.frame.ok_or(Refusal::NoInput)?;
    let request = convert::wear_request(args, frame.input, host.game_data)?;
    Ok(Out::from(host.interact(request)?))
}

fn equipment_remove(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    let frame = host.frame.ok_or(Refusal::NoInput)?;
    let request = convert::remove_request(args, frame.input, host.game_data)?;
    Ok(Out::from(host.interact(request)?))
}

fn loc_interact(lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    host.interact(convert::loc_interact_request(lua, args)?)
        .map(Out::from)
}

fn dialog_continue(_lua: &Lua, host: Host<'_>, _args: &Args) -> Reply {
    host.interact(convert::continue_dialog_request())
        .map(Out::from)
}

fn dialog_answer(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    host.interact(convert::dialog_answer_request(args)?)
        .map(Out::from)
}

fn interface_button(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    host.interact(convert::interface_button_request(args)?)
        .map(Out::from)
}

fn route_inspect(lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    host.interact(convert::inspect_route_request(lua, args)?)
        .map(Out::from)
}

fn route_result(lua: &Lua, frame: &Frame<'_>, _args: &Args) -> Reply {
    convert::route_result_out(lua, frame.native.route_inspect.latest)
}

fn puzzle_move(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    host.interact(convert::puzzle_move_request(args)?)
        .map(Out::from)
}

fn puzzle_board(lua: &Lua, frame: &Frame<'_>, _args: &Args) -> Reply {
    convert::puzzle_board_out(lua, frame.native.puzzle_board)
}

fn combat_fight(lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    Ok(Out::from(host.combat(convert::combat_request(lua, args)?)?))
}

fn gather_run(lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    host.gather(convert::gather_settings(lua, args)?)
        .map(Out::from)
}

fn quest_status(lua: &Lua, frame: &Frame<'_>, args: &Args) -> Reply {
    convert::quest_status_out(
        lua,
        args.first(),
        frame.native.quest_statuses,
        frame.input.tick,
    )
}

fn quest_paths(lua: &Lua, _frame: &Frame<'_>, _args: &Args) -> Reply {
    convert::quest_paths_out(lua, script::quester::card::released_paths())
}

fn quest_progress(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    Ok(Out::from(host.quest_progress(convert::quest_name(args)?)?))
}
