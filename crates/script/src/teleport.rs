//! Rust-owned spellbook teleport. Selected-cache metadata names the
//! non-target button; compact snapshot facts observe Magic XP and tile
//! change. JavaScript sends the caller name and dispatches the returned
//! if-button. A queued click is not arrival.

use crate::isolate_fb::SnapshotReader;
use api::game_data::SelectedGameData;
use serde_json::{json, Value};
use std::cell::RefCell;

pub const SETTLE_POLLS: u32 = 14;

thread_local! {
    static RUNTIME: RefCell<TeleportRuntime> = const { RefCell::new(TeleportRuntime::new()) };
    static NATIVE_OBSERVATION: RefCell<NativeObservation> = const { RefCell::new(NativeObservation::new()) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Tile {
    x: i32,
    z: i32,
    level: i32,
}

struct NativeObservation {
    ingame: bool,
    here: Option<Tile>,
    magic_xp: Option<i32>,
    magic_level: Option<i32>,
}

impl NativeObservation {
    const fn new() -> Self {
        Self {
            ingame: false,
            here: None,
            magic_xp: None,
            magic_level: None,
        }
    }

    fn update(&mut self, snap: &SnapshotReader<'_>) {
        if snap.has_ingame() {
            if !snap.ingame() {
                *self = Self::new();
                return;
            }
            self.ingame = true;
        }
        if snap.has_here() {
            self.here = snap.here().map(|tile| Tile {
                x: tile.x(),
                z: tile.z(),
                level: tile.level(),
            });
        }
        if snap.has_stats() {
            let mut xp = None;
            let mut level = None;
            for row in snap.stats() {
                if row.name().eq_ignore_ascii_case("magic") {
                    xp = Some(row.xp());
                    level = Some(row.effective());
                    break;
                }
            }
            self.magic_xp = xp;
            self.magic_level = level;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    WaitSettle,
}

struct TeleportRuntime {
    paused: bool,
    held: bool,
    token: u64,
    phase: Phase,
    polls_left: u32,
    start_here: Option<Tile>,
    start_xp: Option<i32>,
}

impl TeleportRuntime {
    const fn new() -> Self {
        Self {
            paused: false,
            held: false,
            token: 0,
            phase: Phase::Idle,
            polls_left: 0,
            start_here: None,
            start_xp: None,
        }
    }

    fn frozen(&self) -> bool {
        self.paused || self.held
    }

    fn abort_runtime(&mut self) {
        self.token = self.token.wrapping_add(1);
        self.phase = Phase::Idle;
        self.polls_left = 0;
        self.start_here = None;
        self.start_xp = None;
        self.paused = false;
        self.held = false;
    }
}

pub fn on_snapshot(snap: &SnapshotReader<'_>) {
    NATIVE_OBSERVATION.with(|obs| obs.borrow_mut().update(snap));
}

pub fn on_pause() {
    RUNTIME.with(|rt| rt.borrow_mut().paused = true);
}

pub fn on_resume() {
    RUNTIME.with(|rt| rt.borrow_mut().paused = false);
}

pub fn on_hold(held: bool) {
    RUNTIME.with(|rt| rt.borrow_mut().held = held);
}

pub fn on_reset() {
    RUNTIME.with(|rt| rt.borrow_mut().abort_runtime());
    NATIVE_OBSERVATION.with(|obs| *obs.borrow_mut() = NativeObservation::new());
}

pub fn dispatch(data: Option<&SelectedGameData>, input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => begin(
            data,
            input.get("name").and_then(Value::as_str).unwrap_or(""),
        ),
        "next" => next(input.get("token").and_then(Value::as_u64).unwrap_or(0)),
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    }
}

fn begin(data: Option<&SelectedGameData>, name: &str) -> Value {
    let Some(selected) = data else {
        return json!({ "kind": "notImpl", "reason": "missing selected teleports" });
    };
    if selected.teleports().is_empty() {
        return json!({ "kind": "notImpl", "reason": "missing selected teleports" });
    }
    let Some(spell) = selected.teleport(name) else {
        return json!({ "kind": "unknown" });
    };
    if !spell.available() {
        return json!({ "kind": "notImpl", "reason": "absent control" });
    }
    let obs = NATIVE_OBSERVATION.with(|obs| {
        let borrowed = obs.borrow();
        (borrowed.here, borrowed.magic_xp, borrowed.magic_level)
    });
    if let Some(level) = obs.2 {
        if level < spell.level {
            return json!({ "kind": "done", "result": false, "reason": "level" });
        }
    }
    RUNTIME.with(|rt| {
        let mut runtime = rt.borrow_mut();
        runtime.abort_runtime();
        runtime.phase = Phase::WaitSettle;
        runtime.polls_left = SETTLE_POLLS;
        runtime.start_here = obs.0;
        runtime.start_xp = obs.1;
        json!({
            "kind": "if-button",
            "token": runtime.token,
            "component_id": spell.component_id,
        })
    })
}

fn next(token: u64) -> Value {
    RUNTIME.with(|rt| {
        let mut runtime = rt.borrow_mut();
        if token != runtime.token || runtime.phase == Phase::Idle {
            return json!({ "kind": "aborted", "token": runtime.token });
        }
        if runtime.frozen() {
            return json!({ "kind": "wait", "token": runtime.token });
        }
        let obs = NATIVE_OBSERVATION.with(|obs| {
            let borrowed = obs.borrow();
            (borrowed.ingame, borrowed.here, borrowed.magic_xp)
        });
        if !obs.0 {
            runtime.phase = Phase::Idle;
            return json!({ "kind": "aborted", "token": runtime.token });
        }
        if arrived(runtime.start_here, runtime.start_xp, obs.1, obs.2) {
            runtime.phase = Phase::Idle;
            return json!({
                "kind": "done",
                "result": true,
                "token": runtime.token,
            });
        }
        if runtime.polls_left == 0 {
            runtime.phase = Phase::Idle;
            return json!({
                "kind": "done",
                "result": false,
                "reason": "timeout",
                "token": runtime.token,
            });
        }
        runtime.polls_left = runtime.polls_left.saturating_sub(1);
        json!({ "kind": "wait", "token": runtime.token })
    })
}

fn arrived(
    start_here: Option<Tile>,
    start_xp: Option<i32>,
    here: Option<Tile>,
    xp: Option<i32>,
) -> bool {
    let Some(before) = start_here else {
        return false;
    };
    let Some(after) = here else {
        return false;
    };
    let Some(xp_before) = start_xp else {
        return false;
    };
    let Some(xp_after) = xp else {
        return false;
    };
    after != before && xp_after > xp_before
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::io::ClientRevision;

    fn data(rev: ClientRevision) -> std::sync::Arc<SelectedGameData> {
        api::game_data::for_revision(rev).expect("selected data")
    }

    #[test]
    fn both_selected_caches_post_the_audited_teleports() {
        for rev in [ClientRevision::R274, ClientRevision::R289] {
            let data = data(rev);
            let varrock = data.teleport("Varrock").expect("Varrock");
            assert_eq!(varrock.component_id, 1164);
            assert_eq!(varrock.level, 25);
            assert_eq!(varrock.experience, 350);
            assert_eq!(varrock.x, 3213);
            assert_eq!(varrock.z, 3424);
            assert_eq!(
                varrock
                    .runes
                    .iter()
                    .map(|rune| (rune.name.as_str(), rune.count))
                    .collect::<Vec<_>>(),
                vec![("Fire rune", 1), ("Air rune", 3), ("Law rune", 1)]
            );
            let falador = data
                .teleport("Cast @gre@Falador teleport")
                .expect("Falador live label");
            assert_eq!(falador.component_id, 1170);
            assert_eq!(falador.level, 37);
            assert!(data.teleport("Wind Strike").is_none());
            assert!(data.teleport("High level alchemy").is_none());
            assert_eq!(data.teleports().len(), 7);
            assert_eq!(data.teleports()[6].name, "Trollheim");
        }
    }

    #[test]
    fn missing_controls_do_not_invent_a_button() {
        let missing = dispatch(None, &json!({ "op": "begin", "name": "Varrock" }));
        assert_eq!(missing["kind"], "notImpl");
        let unknown = dispatch(
            Some(data(ClientRevision::R274).as_ref()),
            &json!({ "op": "begin", "name": "Nowhere" }),
        );
        assert_eq!(unknown["kind"], "unknown");
    }
}
