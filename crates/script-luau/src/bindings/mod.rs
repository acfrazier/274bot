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

use convert::{Args, Out, Reply, WalkShape};
use mlua::{Lua, MultiValue};

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
    Ok(Out::from(
        host.walk(convert::walk_request(args, WalkShape::Exact)?)?,
    ))
}

fn walk_near(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    Ok(Out::from(
        host.walk(convert::walk_request(args, WalkShape::Near)?)?,
    ))
}
