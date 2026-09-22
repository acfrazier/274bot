//! Isolate-owned clue-session machine skeleton: `api.clue.begin` / `next`.
//!
//! One token per isolate over the landed held-step identify. The JS wrapper
//! owns the page reads — the posted `snapshot.inv` `(id, count)` page, the
//! posted `hold || ours` signal, the wrapper's `lifecycleGeneration` — and
//! hands them in; this machine owns the token, the generation captured at
//! begin, the frozen clock, the identify call, the callback kinds and the
//! idle end state.
//!
//! `api::clue_logic::identify_step` over the current posted page and the
//! selected `trails()` family is the only membership decision, in its own
//! order: `missing-selected-data`, then `family-unavailable:trails`, then
//! `none-held`, then the landed row. A family absence is not `none-held`. A
//! `none-held` begin is refused with no live token — the caller begins again
//! after the pickup — and a live session that loses its held membership
//! errors `none-held` and aborts.
//!
//! This is the search slice and nothing else: no dig, talk, guardian, puzzle,
//! deposit, retry or return-grind. A held row that is a selected search
//! membership — a selected `trail_loc=^true` **and** a decodable selected
//! `trail_coord` on the same row — walks to its decoded tile and then
//! dispatches the Search/Open picker over the posted loc page; both verbs are
//! enqueued by the wrapper as `InteractReq::Walk` / `InteractReq::Loc`. The
//! picker is the frozen one, minus its `walkLeg`: nearest then action rank,
//! always at the row's own posted tile and id. Every other held step, the
//! packed 3554 `access: "constrained"` clue, the desc-only key-gated riddles
//! and the coord-only map rows included, is identified and then idled: no
//! action and no walk. Yield keeps the token live, so it is not trail
//! completion, and this machine never returns `status: "done"`, never
//! restores gear, and never emits the exact `'clue solved'` string.
//!
//! One token per isolate. A second begin, reset and stop abort the live token
//! and emit no verb for it. Pause and hold freeze this machine's own clock,
//! so a frozen call emits no callback, no walk and no loc, and does not
//! advance the session. `on_snapshot` is fan-out only: begin and next read
//! the pages the wrapper hands in at call time — the parked page, this call's
//! `here` tile and its posted loc page — so nothing is cached here and there
//! is no world copy.

use crate::isolate_fb::SnapshotReader;
use crate::task_clock::InstantTaskClock;
use api::clue_logic::identify_step;
use api::game_data::{SelectedGameData, TrailMembershipRow};
use serde_json::{json, Value};
use std::cell::RefCell;

/// No selected pin. The same public token the landed V8 held-step wrapper
/// publishes: the machine refuses with it rather than calling a page empty.
const MISSING_SELECTED_DATA: &str = "missing-selected-data";

/// The token is not this machine's live one.
const STALE: &str = "stale";

/// The live session was aborted out from under the token by the generation
/// bump (reset / stop). The token is dead and nothing is emitted for it.
const ABORTED: &str = "aborted";

thread_local! {
    static RUNTIME: RefCell<ClueRuntime> = const { RefCell::new(ClueRuntime::new()) };
}

/// The live session's phase. `Idle` is the only phase without a token, so a
/// refused begin leaves it in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// No token.
    Idle,
    /// A held step was identified and the fresh `enabled` read is unanswered.
    Gate,
    /// Enabled: the progress status line is not posted yet.
    Reporting,
    /// Progress posted: the same step stays held. A search row walks then
    /// dispatches the picker from here; every other row idles with no action
    /// and no walk, and no second callback is emitted for this step.
    Steady,
}

struct ClueRuntime {
    clock: InstantTaskClock,
    token: u64,
    phase: Phase,
    /// The wrapper's `lifecycleGeneration` captured at begin. Nothing else
    /// about the live session is captured: `enabled`, the page and the
    /// `hold || ours` signal are re-read at call time.
    generation: u64,
    /// The membership row this phase belongs to: a different held row
    /// re-arms the gate, so the enabled read is never reused across steps.
    step_id: i32,
}

impl ClueRuntime {
    const fn new() -> Self {
        Self {
            clock: InstantTaskClock::new(),
            token: 0,
            phase: Phase::Idle,
            generation: 0,
            step_id: 0,
        }
    }

    /// Abort keeps the pause/hold freeze (`InstantTaskClock` contract: abort
    /// clears the deadline only) and emits nothing.
    fn abort(&mut self) {
        self.token = self.token.wrapping_add(1);
        self.phase = Phase::Idle;
        self.generation = 0;
        self.step_id = 0;
        self.clock.deadline = None;
    }

    fn aborted(&mut self, reason: &str) -> Value {
        self.abort();
        json!({ "kind": "aborted", "token": self.token, "reason": reason })
    }

    fn emit(&self, kind: &str) -> Value {
        json!({ "kind": kind, "token": self.token })
    }

    fn begin(&mut self, selected: Option<&SelectedGameData>, input: &Value) -> Value {
        // A second begin aborts the live token first and emits nothing for
        // it: begin has no continue kind, and a refused begin is still a
        // refusal, not a resumed session.
        self.abort();
        let row = match identify(selected, input) {
            Ok(row) => row,
            Err(reason) => return self.aborted(reason),
        };
        self.token = self.token.wrapping_add(1);
        self.phase = Phase::Gate;
        // Strictly captured: `next` requires the same generation to be
        // posted again, so a wrapper that stops sending it fails closed.
        self.generation = input.get("generation").and_then(Value::as_u64).unwrap_or(0);
        self.step_id = row.id;
        json!({ "kind": "token", "token": self.token })
    }

    fn next(&mut self, selected: Option<&SelectedGameData>, input: &Value) -> Value {
        let Some(token) = input.get("token").and_then(Value::as_u64) else {
            return self.aborted(STALE);
        };
        if token != self.token || self.phase == Phase::Idle {
            return self.aborted(STALE);
        }
        if input.get("generation").and_then(Value::as_u64) != Some(self.generation) {
            // Reset, stop and a second begin abort silently; the first thing
            // the dead token hears about it is this error, never a kind.
            return self.aborted(ABORTED);
        }
        if self.clock.frozen() {
            // Frozen: no callback, no verb and no burn. `resume` is not
            // consumed, so the gate is still unanswered after the thaw.
            return self.emit("wait");
        }
        if input.get("hold").and_then(Value::as_bool).unwrap_or(false) {
            // The posted `hold || ours` cooperative interrupt. The token
            // lives and the step is not trail completion.
            return self.emit("yield");
        }
        let row = match identify(selected, input) {
            Ok(row) => row,
            Err(reason) => return self.aborted(reason),
        };
        if self.step_id != row.id {
            // A different step is held: the previous progress and its
            // `enabled` answer belong to the old row, so this call re-asks
            // and a `resume` it carried is not the new step's answer.
            self.step_id = row.id;
            self.phase = Phase::Gate;
            return self.emit("callback.enabled");
        }
        match self.phase {
            Phase::Idle => self.aborted(STALE),
            Phase::Gate => match input.get("resume").and_then(Value::as_bool) {
                // The callback return. False means do not execute: the
                // session idles with its token live, and the following gate
                // re-reads instead of replaying this answer.
                Some(false) => self.emit("wait"),
                Some(true) => {
                    self.phase = Phase::Reporting;
                    json!({
                        "kind": "callback.log",
                        "token": self.token,
                        "message": progress(row),
                    })
                }
                None => self.emit("callback.enabled"),
            },
            Phase::Reporting => {
                self.phase = Phase::Steady;
                json!({
                    "kind": "callback.setStatus",
                    "token": self.token,
                    "message": status(row),
                })
            }
            Phase::Steady => self.search(row, input),
        }
    }

    /// `Steady` on an identified row: a search membership walks to its
    /// decoded tile and then dispatches the picker from this call's pages;
    /// every other row idles exactly as before.
    ///
    /// The two pages are the wrapper's call-time marshalling of
    /// `host().snapshot` — `here` and the posted loc page — and never a
    /// cached world copy. Emitting `walk` and `loc` repeats while the row
    /// stays held, because arrival is `here` and a stale loc id is the
    /// host's own refuse: the session waits the tick out instead of
    /// abandoning, and the pick is re-read next call.
    fn search(&self, row: &TrailMembershipRow, input: &Value) -> Value {
        let Some(tile) = search_tile(row) else {
            // Not a search membership: the desc-only key-gated riddles, the
            // coord-only map rows and the constrained 3554 clue, among
            // everything else, are identified and then idle.
            return self.emit("wait");
        };
        let Some(here) = input.get("here").and_then(posted_tile) else {
            // No posted tile: there is no arrival claim to make and no walk
            // to measure, so this tick waits rather than walking blind.
            return self.emit("wait");
        };
        if here.level != tile.level || chebyshev(here, tile) > i64::from(ARRIVE_RADIUS) {
            return json!({
                "kind": "walk",
                "token": self.token,
                "x": tile.x,
                "z": tile.z,
                "level": tile.level,
            });
        }
        match pick_loc(input, tile) {
            Some(pick) => json!({
                "kind": "loc",
                "token": self.token,
                "x": pick.tile.x,
                "z": pick.tile.z,
                "level": pick.tile.level,
                "action": pick.action,
                // The posted scene id, always present: the host matches that
                // identity and refuses a stale one rather than taking a
                // co-located other row.
                "id": pick.id,
            }),
            // Arrived with nothing to search this tick — a scene that is not
            // loaded, an empty page, or a loc id the host refused. Wait, keep
            // the token, and re-pick next call; never abandon and never a
            // token-killing `no-searchable-loc`.
            None => self.emit("wait"),
        }
    }
}

/// The landed identify over the wrapper's page and the selected family. The
/// family token and `none-held` are the helper's own; only the missing pin is
/// named here.
fn identify<'a>(
    selected: Option<&'a SelectedGameData>,
    input: &Value,
) -> Result<&'a TrailMembershipRow, &'static str> {
    let Some(data) = selected else {
        return Err(MISSING_SELECTED_DATA);
    };
    let held = posted_page(input);
    identify_step(&held, data.trails())
}

/// The posted `(id, count)` page in posted order, as the wrapper marshals it.
/// A missing, non-array, or malformed entry is skipped the same way a row
/// that is not an `i32` pair cannot be held — never a second page and never a
/// snapshot error. Only a positive count holds, and only a membership row
/// wins; that stays in the landed helper.
fn posted_page(input: &Value) -> Vec<(i32, i32)> {
    let Some(rows) = input.get("held").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut held = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(pair) = row.as_array() else {
            continue;
        };
        let (Some(id), Some(count)) = (pair.first().and_then(i32_of), pair.get(1).and_then(i32_of))
        else {
            continue;
        };
        held.push((id, count));
    }
    held
}

/// The wrapper writes page numbers as JSON integers.
fn i32_of(value: &Value) -> Option<i32> {
    i32::try_from(value.as_i64()?).ok()
}

/// One packed-coord square: `SQUARE` tiles per map square on both axes.
/// Same packed contract as the landed `nav::canlight::unpack_packed_coord`;
/// the script crate does not take a `nav` runtime dependency, so the
/// arithmetic lives here.
const SQUARE: i32 = 64;

/// Planes `0..=3`.
const LEVELS: i32 = 4;

/// The frozen picker's `ARRIVE_RADIUS`: the decoded tile itself, and one step
/// off it on either axis.
const ARRIVE_RADIUS: i32 = 1;

/// A world tile: the same three fields the posted `here` object and every
/// posted `SceneEntity` row carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Tile {
    x: i32,
    z: i32,
    level: i32,
}

/// Chebyshev distance, widened: `max(|dx|, |dz|)` over the whole `i32` range
/// so a decoded tile and a posted tile can never overflow the subtraction.
/// Callers compare levels first, because a level mismatch is not a distance.
fn chebyshev(a: Tile, b: Tile) -> i64 {
    let dx = (i64::from(a.x) - i64::from(b.x)).abs();
    let dz = (i64::from(a.z) - i64::from(b.z)).abs();
    dx.max(dz)
}

/// A selected `trail_coord` token → the tile it packs.
///
/// The landed `nav::canlight::unpack_packed_coord` contract, copied: five
/// `_`-separated integers `level_mapX_mapZ_localX_localZ`, level in `0..=3`,
/// both locals in `0..64`, and `x = mapX * 64 + localX` (`z` likewise). A
/// missing or extra part, a non-integer, an out-of-range level or local, and
/// a map that overflows the packed widening are all **not** a tile: the row
/// idles rather than walking to an invented coordinate.
fn decode_trail_coord(token: &str) -> Option<Tile> {
    let mut parts = token.split('_');
    let mut next = || parts.next()?.parse::<i32>().ok();
    let level = next()?;
    let map_x = next()?;
    let map_z = next()?;
    let local_x = next()?;
    let local_z = next()?;
    if parts.next().is_some() {
        return None;
    }
    if !(0..LEVELS).contains(&level)
        || !(0..SQUARE).contains(&local_x)
        || !(0..SQUARE).contains(&local_z)
    {
        return None;
    }
    Some(Tile {
        x: map_x.checked_mul(SQUARE)?.checked_add(local_x)?,
        z: map_z.checked_mul(SQUARE)?.checked_add(local_z)?,
        level,
    })
}

/// The identified row's selected search membership: a selected
/// `trail_loc=^true` param **and** a decodable selected `trail_coord` on the
/// same row, in file order for the coord.
///
/// The pin is the membership; the coord alone is not. A coord-only row — the
/// easy maps, the frozen `keyFrom` riddles that carry only `trail_desc`, and
/// the bounded packed 3554 clue — is not a search step and idles. A pin with
/// no coord, or an off-contract one, is the same idle: nothing is invented.
fn search_tile(row: &TrailMembershipRow) -> Option<Tile> {
    let mut located = false;
    let mut coord = None;
    for param in &row.params {
        if param.key == "trail_loc" && param.value == "^true" {
            located = true;
        } else if param.key == "trail_coord" && coord.is_none() {
            coord = Some(param.value.as_str());
        }
    }
    if !located {
        return None;
    }
    decode_trail_coord(coord?)
}

/// A posted `{ x, z, level }` value — the wrapper's `here` tile or one posted
/// loc row. A missing, null, or malformed object is not a tile.
fn posted_tile(value: &Value) -> Option<Tile> {
    Some(Tile {
        x: i32_of(value.get("x")?)?,
        z: i32_of(value.get("z")?)?,
        level: i32_of(value.get("level")?)?,
    })
}

/// The picker's chosen row: the posted tile and id the verb is dispatched at,
/// and the canonical action.
struct Pick {
    tile: Tile,
    id: i32,
    action: &'static str,
}

/// The frozen `pickSearchLoc` rank: `Search` before `Open`, matched
/// case-insensitively on the posted action strings and emitted canonically.
const SEARCH_OPS: [&str; 2] = ["Search", "Open"];

/// Whether a posted loc row lists `wanted`, ignoring ASCII case. A missing or
/// non-array `actions` matches nothing.
fn loc_action(row: &Value, wanted: &str) -> bool {
    let Some(actions) = row.get("actions").and_then(Value::as_array) else {
        return false;
    };
    actions.iter().any(|action| {
        action
            .as_str()
            .is_some_and(|text| text.eq_ignore_ascii_case(wanted))
    })
}

/// The frozen picker over this call's posted loc page: posted rows on the
/// decoded tile's level, within `ARRIVE_RADIUS` Chebyshev **of the decoded
/// tile** (not of `here`), whose actions carry `Search` then `Open`.
/// Nearest wins, then the action rank, then posted order — the scan only
/// replaces the best on a strict improvement. The verb keeps the row's own
/// tile and posted id, so the host matches the type identity it was posted
/// with; a row that is not the marshalled `{ id, x, z, level, actions }`
/// shape is skipped rather than guessed at.
fn pick_loc(input: &Value, tile: Tile) -> Option<Pick> {
    let rows = input.get("locs")?.as_array()?;
    let mut best: Option<(Pick, i64, usize)> = None;
    for row in rows {
        let (Some(id), Some(row_tile)) = (row.get("id").and_then(i32_of), posted_tile(row)) else {
            continue;
        };
        if row_tile.level != tile.level {
            continue;
        }
        let distance = chebyshev(row_tile, tile);
        if distance > i64::from(ARRIVE_RADIUS) {
            continue;
        }
        let Some(rank) = SEARCH_OPS.iter().position(|op| loc_action(row, op)) else {
            continue;
        };
        let closer = match &best {
            None => true,
            Some((_, best_distance, best_rank)) => {
                distance < *best_distance || (distance == *best_distance && rank < *best_rank)
            }
        };
        if closer {
            best = Some((
                Pick {
                    tile: row_tile,
                    id,
                    action: SEARCH_OPS[rank],
                },
                distance,
                rank,
            ));
        }
    }
    best.map(|(pick, _, _)| pick)
}

/// Progress line for the identified step: landed alias, role and id only.
/// Never an invented coord, npc or answer, and never `'clue solved'`.
fn progress(row: &TrailMembershipRow) -> String {
    format!("clue step held: {} {} [{}]", row.role, row.alias, row.id)
}

/// Status line for the identified step. Trail completion — `done`, restore,
/// then the exact `'clue solved'` string — is a later slice, not this one.
fn status(row: &TrailMembershipRow) -> String {
    format!("clue: {}", row.alias)
}

/// The posted snapshot is not this machine's page: begin and next read the
/// page the wrapper hands in at call time, so nothing is cached here.
/// Registered so the machine's hook set matches the isolate's fan-out.
pub fn on_snapshot(_snap: &SnapshotReader<'_>) {}

pub fn on_pause() {
    RUNTIME.with(|rt| {
        let held = rt.borrow().clock.held;
        rt.borrow_mut().clock.set_freeze(true, held);
    });
}

pub fn on_resume() {
    RUNTIME.with(|rt| {
        let held = rt.borrow().clock.held;
        rt.borrow_mut().clock.set_freeze(false, held);
    });
}

pub fn on_hold(held: bool) {
    RUNTIME.with(|rt| {
        let paused = rt.borrow().clock.paused;
        rt.borrow_mut().clock.set_freeze(paused, held);
    });
}

pub fn on_reset() {
    RUNTIME.with(|rt| rt.borrow_mut().abort());
}

/// The machine's only entry point: the selected pin comes from the native
/// registration's captured `game_data`, and the payload is the wrapper's
/// marshalled call — never a host wire.
pub fn dispatch(selected: Option<&SelectedGameData>, input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => RUNTIME.with(|rt| rt.borrow_mut().begin(selected, input)),
        "next" => RUNTIME.with(|rt| rt.borrow_mut().next(selected, input)),
        _ => json!({ "kind": "notImpl", "reason": "unknown clue op" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::game_data::TrailParam;
    use client::io::ClientRevision;
    use std::sync::Arc;

    const CASKET: i32 = 3531;
    const CLUE: i32 = 3554;
    /// `trail_clue_easy_simple001`: the selected search membership,
    /// `trail_loc=^true` with `trail_coord=1_50_50_9_18`.
    const SEARCH: i32 = 2677;
    /// `trail_clue_easy_map001`: a selected `trail_coord` with no `trail_loc`.
    const MAP: i32 = 2713;
    /// `trail_clue_medium_riddle001`: a frozen `keyFrom` riddle, selected
    /// `trail_desc` only.
    const RIDDLE: i32 = 2831;

    fn selected() -> Arc<SelectedGameData> {
        api::game_data::for_revision(ClientRevision::R274).expect("selected data")
    }

    /// A held id that is not a membership row: a challenge-answer row, which
    /// stays unread.
    fn unrelated(data: &SelectedGameData) -> i32 {
        data.trails()
            .expect("trails")
            .challenge_answers
            .first()
            .expect("challenge row")
            .id
    }

    fn payload(op: &str, token: Option<u64>, held: Value, extra: Value) -> Value {
        let mut call = json!({ "op": op, "generation": 4, "held": held });
        if let Some(token) = token {
            call["token"] = json!(token);
        }
        for (key, value) in extra.as_object().expect("extra") {
            call[key] = value.clone();
        }
        call
    }

    fn begin(data: &SelectedGameData, held: Value) -> Value {
        dispatch(Some(data), &payload("begin", None, held, json!({})))
    }

    fn call(data: &SelectedGameData, token: u64, held: Value, extra: Value) -> Value {
        dispatch(Some(data), &payload("next", Some(token), held, extra))
    }

    fn token_of(step: &Value) -> u64 {
        step["token"].as_u64().expect("token")
    }

    /// The wrapper's posted `here` tile.
    fn here(x: i32, z: i32, level: i32) -> Value {
        json!({ "x": x, "z": z, "level": level })
    }

    /// One wrapper-marshalled posted loc row.
    fn loc(id: i32, x: i32, z: i32, level: i32, actions: &[&str]) -> Value {
        json!({ "id": id, "x": x, "z": z, "level": level, "actions": actions })
    }

    /// One row of the selected family.
    fn row(data: &SelectedGameData, id: i32) -> &TrailMembershipRow {
        data.trails()
            .expect("trails")
            .rows
            .iter()
            .find(|row| row.id == id)
            .unwrap_or_else(|| panic!("row {id}"))
    }

    /// Drive a held row to `Steady`: begin, the gate, the log line, then the
    /// status line. The search verbs start on the call after this one.
    fn steady(data: &SelectedGameData, id: i32) -> u64 {
        let page = json!([[id, 1]]);
        let token = token_of(&begin(data, page.clone()));
        let gate = call(data, token, page.clone(), json!({}));
        assert_eq!(gate["kind"], "callback.enabled", "{gate}");
        let logged = call(data, token, page.clone(), json!({ "resume": true }));
        assert_eq!(logged["kind"], "callback.log", "{logged}");
        let posted = call(data, token, page, json!({}));
        assert_eq!(posted["kind"], "callback.setStatus", "{posted}");
        token
    }

    #[test]
    fn a_frozen_clock_emits_wait_and_keeps_the_gate_unanswered() {
        on_reset();
        let data = selected();
        let opened = begin(&data, json!([[CASKET, 1]]));
        assert_eq!(opened["kind"], "token");
        let token = token_of(&opened);
        let gate = call(&data, token, json!([[CASKET, 1]]), json!({}));
        assert_eq!(gate["kind"], "callback.enabled", "{gate}");

        // Paused: no callback, no verb, and the answer is not consumed. The
        // freeze wins over the posted interrupt carried in the same call.
        on_pause();
        let paused = call(
            &data,
            token,
            json!([[CASKET, 1]]),
            json!({ "resume": true, "hold": true }),
        );
        assert_eq!(paused["kind"], "wait", "{paused}");
        assert!(paused.get("message").is_none(), "{paused}");
        on_resume();
        let after_pause = call(&data, token, json!([[CASKET, 1]]), json!({}));
        assert_eq!(
            after_pause["kind"], "callback.enabled",
            "a frozen call burns nothing: {after_pause}"
        );

        // A held clock is the same wait.
        on_hold(true);
        let held_clock = call(
            &data,
            token,
            json!([[CASKET, 1]]),
            json!({ "resume": true }),
        );
        assert_eq!(held_clock["kind"], "wait", "{held_clock}");
        on_hold(false);

        // The posted `hold || ours` signal without a frozen clock: yield, and
        // the token survives it.
        let yielded = call(&data, token, json!([[CASKET, 1]]), json!({ "hold": true }));
        assert_eq!(yielded["kind"], "yield", "{yielded}");
        assert_eq!(token_of(&yielded), token, "{yielded}");

        // The gate was still open, so the enabled answer lands now.
        let enabled = call(
            &data,
            token,
            json!([[CASKET, 1]]),
            json!({ "resume": true }),
        );
        assert_eq!(enabled["kind"], "callback.log", "{enabled}");
    }

    #[test]
    fn a_generation_bump_aborts_the_session_without_a_verb() {
        on_reset();
        let data = selected();
        let opened = begin(&data, json!([[CLUE, 1]]));
        let token = token_of(&opened);

        let bumped = dispatch(
            Some(&data),
            &payload(
                "next",
                Some(token),
                json!([[CLUE, 1]]),
                json!({ "generation": 5 }),
            ),
        );
        assert_eq!(bumped["kind"], "aborted", "{bumped}");
        assert_eq!(bumped["reason"], "aborted", "{bumped}");
        for kind in [
            "wait",
            "yield",
            "callback.enabled",
            "callback.log",
            "callback.setStatus",
        ] {
            assert_ne!(bumped["kind"], kind, "{bumped}");
        }
        // The token died with the bump: the same call on the old generation
        // is stale, not a resumed session.
        let after = call(&data, token, json!([[CLUE, 1]]), json!({}));
        assert_eq!(after["kind"], "aborted", "{after}");
        assert_eq!(after["reason"], "stale", "{after}");
    }

    #[test]
    fn enabled_is_re_read_at_every_gate_and_never_captured() {
        on_reset();
        let data = selected();
        // Extra begin keys are ignored, not captured: no `enabled`, `resume`,
        // hold flag or page identity is frozen into the session.
        let opened = dispatch(
            Some(&data),
            &payload(
                "begin",
                None,
                json!([[CLUE, 1]]),
                json!({ "enabled": false, "resume": false, "hold": true }),
            ),
        );
        assert_eq!(opened["kind"], "token", "{opened}");
        let token = token_of(&opened);
        assert!(
            opened.get("kind").is_some() && opened.get("reason").is_none(),
            "begin never returns a continue kind and never a reason: {opened}"
        );

        let first = call(&data, token, json!([[CLUE, 1]]), json!({}));
        assert_eq!(first["kind"], "callback.enabled", "{first}");
        // False means do not execute: an idle continue with the token live.
        let denied = call(&data, token, json!([[CLUE, 1]]), json!({ "resume": false }));
        assert_eq!(denied["kind"], "wait", "{denied}");
        assert_eq!(token_of(&denied), token, "{denied}");
        // The next tick asks again rather than replaying the captured answer.
        let again = call(&data, token, json!([[CLUE, 1]]), json!({}));
        assert_eq!(again["kind"], "callback.enabled", "{again}");
        // And a later true is honored, because nothing was captured at begin.
        let enabled = call(&data, token, json!([[CLUE, 1]]), json!({ "resume": true }));
        assert_eq!(enabled["kind"], "callback.log", "{enabled}");
        let message = enabled["message"].as_str().unwrap_or("");
        assert!(message.contains("trail_clue_hard_sextant028"), "{enabled}");
        assert!(message.contains("3554"), "{enabled}");

        // The constrained 3554 row keeps the token and then idles.
        let steady = call(&data, token, json!([[CLUE, 1]]), json!({}));
        assert_eq!(steady["kind"], "callback.setStatus", "{steady}");
        assert!(
            steady["message"]
                .as_str()
                .unwrap_or("")
                .contains("trail_clue_hard_sextant028"),
            "{steady}"
        );
        for _ in 0..2 {
            let idle = call(&data, token, json!([[CLUE, 1]]), json!({}));
            assert_eq!(idle["kind"], "wait", "{idle}");
            assert_eq!(token_of(&idle), token, "{idle}");
        }
    }

    #[test]
    fn a_new_held_step_re_arms_the_gate() {
        on_reset();
        let data = selected();
        let token = token_of(&begin(&data, json!([[CASKET, 1]])));
        assert_eq!(
            call(&data, token, json!([[CASKET, 1]]), json!({}))["kind"],
            "callback.enabled"
        );
        assert_eq!(
            call(
                &data,
                token,
                json!([[CASKET, 1]]),
                json!({ "resume": true })
            )["kind"],
            "callback.log"
        );
        // The clue replaces the casket: the enabled read is not reused.
        let re_armed = call(&data, token, json!([[CLUE, 1]]), json!({ "resume": true }));
        assert_eq!(re_armed["kind"], "callback.enabled", "{re_armed}");
        assert_eq!(token_of(&re_armed), token, "{re_armed}");
    }

    #[test]
    fn none_held_is_a_refusal_and_never_a_live_token_or_a_done() {
        on_reset();
        let data = selected();
        let challenge = unrelated(&data);
        for held in [json!([]), json!([[challenge, 1]]), json!([[CASKET, 0]])] {
            let refused = begin(&data, held.clone());
            assert_eq!(refused["kind"], "aborted", "{held} {refused}");
            assert_eq!(refused["reason"], "none-held", "{held} {refused}");
            // No live token: the refusal's own number is not a session.
            let after = call(&data, token_of(&refused), held.clone(), json!({}));
            assert_eq!(after["reason"], "stale", "{held} {after}");
        }

        // A live session that loses its held membership errors `none-held`
        // and aborts — it does not quietly become done.
        let token = token_of(&begin(&data, json!([[CASKET, 1]])));
        let lost = call(&data, token, json!([[challenge, 1]]), json!({}));
        assert_eq!(lost["kind"], "aborted", "{lost}");
        assert_eq!(lost["reason"], "none-held", "{lost}");
        let after = call(&data, token, json!([[CASKET, 1]]), json!({}));
        assert_eq!(after["reason"], "stale", "{after}");
    }

    #[test]
    fn a_missing_selected_pin_is_not_none_held() {
        on_reset();
        let refused = dispatch(
            None,
            &payload("begin", None, json!([[CASKET, 1]]), json!({})),
        );
        assert_eq!(refused["kind"], "aborted", "{refused}");
        assert_eq!(refused["reason"], "missing-selected-data", "{refused}");
        // The live token's view of a vanished pin is the same refusal.
        let data = selected();
        let token = token_of(&begin(&data, json!([[CASKET, 1]])));
        let gone = dispatch(
            None,
            &payload("next", Some(token), json!([[CASKET, 1]]), json!({})),
        );
        assert_eq!(gone["kind"], "aborted", "{gone}");
        assert_eq!(gone["reason"], "missing-selected-data", "{gone}");
    }

    #[test]
    fn the_exact_clue_solved_string_is_never_emitted() {
        on_reset();
        let data = selected();
        let token = token_of(&begin(&data, json!([[CASKET, 1]])));
        let steps = vec![
            call(&data, token, json!([[CASKET, 1]]), json!({})),
            call(
                &data,
                token,
                json!([[CASKET, 1]]),
                json!({ "resume": true }),
            ),
            call(&data, token, json!([[CASKET, 1]]), json!({})),
            call(&data, token, json!([[CASKET, 1]]), json!({})),
            call(&data, token, json!([[CASKET, 1]]), json!({ "hold": true })),
        ];
        for step in &steps {
            let text = step.to_string();
            assert!(!text.contains("clue solved"), "{step}");
            assert!(!text.contains("ownsEquipment"), "{step}");
            assert!(
                step["status"].is_null(),
                "the machine emits kinds, not the public status: {step}"
            );
            assert_ne!(step["kind"], "done", "{step}");
        }
        assert_eq!(
            steps
                .iter()
                .map(|step| step["kind"].clone())
                .collect::<Vec<_>>(),
            vec![
                json!("callback.enabled"),
                json!("callback.log"),
                json!("callback.setStatus"),
                json!("wait"),
                json!("yield"),
            ],
            "{steps:?}"
        );
    }

    #[test]
    fn the_packed_coord_contract_is_copied_and_off_contract_tokens_idle() {
        // The pinned vector: `trail_clue_easy_simple001`'s selected token.
        assert_eq!(
            decode_trail_coord("1_50_50_9_18"),
            Some(Tile {
                x: 3209,
                z: 3218,
                level: 1
            })
        );
        // The landed `nav::canlight` vector, plus both ends of each bound.
        assert_eq!(
            decode_trail_coord("0_50_53_50_24"),
            Some(Tile {
                x: 3250,
                z: 3416,
                level: 0
            })
        );
        assert_eq!(
            decode_trail_coord("3_0_0_0_0"),
            Some(Tile {
                x: 0,
                z: 0,
                level: 3
            })
        );
        assert_eq!(
            decode_trail_coord("0_0_0_63_63"),
            Some(Tile {
                x: 63,
                z: 63,
                level: 0
            })
        );
        for token in [
            "",
            "1_50_50_9",
            "1_50_50_9_18_1",
            "1_50_50_9_x",
            "one_50_50_9_18",
            "4_50_50_9_18",
            "-1_50_50_9_18",
            "1_50_50_64_18",
            "1_50_50_9_64",
            "1_50_50_-1_18",
            "1_50_50_9_-1",
            " 1_50_50_9_18",
            "1_50_50_9_18 ",
            // A map that would overflow the packed widening is not a tile.
            "2147483647_50_50_9_18",
            "1_2147483647_50_9_18",
        ] {
            assert_eq!(decode_trail_coord(token), None, "{token:?}");
        }
    }

    #[test]
    fn membership_is_the_loc_pin_plus_a_decodable_coord_on_the_same_row() {
        let data = selected();
        assert_eq!(
            search_tile(row(&data, SEARCH)),
            Some(Tile {
                x: 3209,
                z: 3218,
                level: 1
            })
        );
        // The coord-only maps, the desc-only frozen `keyFrom` riddles, the
        // constrained 3554 clue and a paramless casket are not search rows.
        for id in [MAP, RIDDLE, CLUE, CASKET] {
            assert_eq!(search_tile(row(&data, id)), None, "{id}");
        }
        // The selected pin is the membership, and every pinned row on this
        // pin carries a decodable coord.
        let facts = data.trails().expect("trails");
        let pinned: Vec<&TrailMembershipRow> = facts
            .rows
            .iter()
            .filter(|row| {
                row.params
                    .iter()
                    .any(|param| param.key == "trail_loc" && param.value == "^true")
            })
            .collect();
        assert_eq!(pinned.len(), 58, "selected trail_loc=^true rows");
        for row in &pinned {
            assert!(search_tile(row).is_some(), "{}", row.alias);
        }
        // Synthetic rows: the pin without a coord, an off-contract coord, and
        // a bare `true` that is not the `^true` pin all idle.
        let pin = || TrailParam {
            key: "trail_loc".into(),
            value: "^true".into(),
        };
        let coord = |value: &str| TrailParam {
            key: "trail_coord".into(),
            value: value.into(),
        };
        let member = |params: Vec<TrailParam>| TrailMembershipRow {
            alias: "trail_clue_test".into(),
            id: 1,
            role: "clue".into(),
            params,
            access: None,
        };
        assert_eq!(search_tile(&member(vec![pin()])), None);
        assert_eq!(search_tile(&member(vec![coord("1_50_50_9_18")])), None);
        assert_eq!(search_tile(&member(vec![pin(), coord("1_50_50_9")])), None);
        assert_eq!(
            search_tile(&member(vec![
                TrailParam {
                    key: "trail_loc".into(),
                    value: "true".into(),
                },
                coord("1_50_50_9_18"),
            ])),
            None
        );
        assert_eq!(
            search_tile(&member(vec![pin(), coord("1_50_50_9_18")])),
            Some(Tile {
                x: 3209,
                z: 3218,
                level: 1
            })
        );
    }

    #[test]
    fn a_search_row_walks_to_the_decoded_tile_and_then_picks_the_loc() {
        on_reset();
        let data = selected();
        let page = json!([[SEARCH, 1]]);
        let token = steady(&data, SEARCH);

        // Not arrived: the walk is the decoded tile, and it repeats.
        for _ in 0..2 {
            let walk = call(
                &data,
                token,
                page.clone(),
                json!({ "here": here(3200, 3218, 1) }),
            );
            assert_eq!(walk["kind"], "walk", "{walk}");
            assert_eq!(walk["x"], 3209, "{walk}");
            assert_eq!(walk["z"], 3218, "{walk}");
            assert_eq!(walk["level"], 1, "{walk}");
            assert_eq!(token_of(&walk), token, "{walk}");
        }
        // Another level is not arrival, even standing on the decoded tile,
        // and neither is two tiles away on one axis.
        for far in [here(3209, 3218, 0), here(3211, 3218, 1)] {
            let walk = call(&data, token, page.clone(), json!({ "here": far }));
            assert_eq!(walk["kind"], "walk", "{walk}");
        }

        // Arrived. The nearest row wins over the better rank: the adjacent
        // `Search` loses to the `Open` on the decoded tile.
        let nearest = call(
            &data,
            token,
            page.clone(),
            json!({
                "here": here(3209, 3218, 1),
                "locs": [
                    loc(11, 3210, 3218, 1, &["Search"]),
                    loc(12, 3209, 3218, 1, &["Open"]),
                ],
            }),
        );
        assert_eq!(nearest["kind"], "loc", "{nearest}");
        assert_eq!(nearest["action"], "Open", "{nearest}");
        assert_eq!(nearest["id"], 12, "{nearest}");
        assert_eq!(
            (
                nearest["x"].clone(),
                nearest["z"].clone(),
                nearest["level"].clone()
            ),
            (json!(3209), json!(3218), json!(1)),
            "{nearest}"
        );

        // Same distance: the rank decides, not the posted order.
        let ranked = call(
            &data,
            token,
            page.clone(),
            json!({
                "here": here(3209, 3218, 1),
                "locs": [
                    loc(13, 3209, 3218, 1, &["Open"]),
                    loc(14, 3209, 3218, 1, &["Search", "Pick"]),
                ],
            }),
        );
        assert_eq!(ranked["kind"], "loc", "{ranked}");
        assert_eq!(ranked["action"], "Search", "{ranked}");
        assert_eq!(ranked["id"], 14, "{ranked}");

        // Same distance and rank: the first posted row wins.
        let first = call(
            &data,
            token,
            page.clone(),
            json!({
                "here": here(3209, 3218, 1),
                "locs": [
                    loc(15, 3209, 3218, 1, &["Search"]),
                    loc(16, 3209, 3218, 1, &["search"]),
                ],
            }),
        );
        assert_eq!(first["id"], 15, "{first}");
        assert_eq!(first["action"], "Search", "{first}");

        // Off-contract rows are skipped: another level, distance two, no
        // searchable action, a non-array action list, and no posted id. The
        // survivor matches case-insensitively and keeps its own tile.
        let filtered = call(
            &data,
            token,
            page,
            json!({
                "here": here(3209, 3218, 1),
                "locs": [
                    loc(20, 3209, 3218, 2, &["Search"]),
                    loc(21, 3211, 3218, 1, &["Search"]),
                    loc(22, 3209, 3218, 1, &["Pick", "Use"]),
                    json!({ "id": 23, "x": 3209, "z": 3218, "level": 1, "actions": "Search" }),
                    json!({ "x": 3209, "z": 3218, "level": 1, "actions": ["Search"] }),
                    loc(25, 3210, 3217, 1, &["sEaRcH"]),
                ],
            }),
        );
        assert_eq!(filtered["kind"], "loc", "{filtered}");
        assert_eq!(filtered["action"], "Search", "{filtered}");
        assert_eq!(filtered["id"], 25, "{filtered}");
        assert_eq!(
            (
                filtered["x"].clone(),
                filtered["z"].clone(),
                filtered["level"].clone()
            ),
            (json!(3210), json!(3217), json!(1)),
            "the verb keeps the posted row's own tile: {filtered}"
        );
    }

    #[test]
    fn an_arrived_search_row_waits_without_a_loc_and_never_abandons() {
        on_reset();
        let data = selected();
        let page = json!([[SEARCH, 1]]);
        let token = steady(&data, SEARCH);
        for locs in [json!([]), json!([loc(11, 3209, 3218, 1, &["Pick"])])] {
            let idle = call(
                &data,
                token,
                page.clone(),
                json!({ "here": here(3209, 3218, 1), "locs": locs }),
            );
            assert_eq!(idle["kind"], "wait", "{idle}");
            assert_eq!(token_of(&idle), token, "{idle}");
            let text = idle.to_string();
            for forbidden in ["no-searchable-loc", "abandon", "done", "clue solved"] {
                assert!(!text.contains(forbidden), "{idle}");
            }
        }
        // No posted `here` at all is the same wait: no arrival claim and no
        // blind walk.
        let no_tile = call(&data, token, page.clone(), json!({ "locs": [] }));
        assert_eq!(no_tile["kind"], "wait", "{no_tile}");
        let malformed = call(
            &data,
            token,
            page.clone(),
            json!({ "here": json!({ "x": 1 }) }),
        );
        assert_eq!(malformed["kind"], "wait", "{malformed}");
        // And the session is still live for the loc that appears later.
        let picked = call(
            &data,
            token,
            page,
            json!({
                "here": here(3208, 3218, 1),
                "locs": [loc(42, 3209, 3218, 1, &["Search"])],
            }),
        );
        assert_eq!(picked["kind"], "loc", "{picked}");
        assert_eq!(picked["id"], 42, "{picked}");
    }

    #[test]
    fn freeze_and_yield_beat_the_search_walk() {
        on_reset();
        let data = selected();
        let page = json!([[SEARCH, 1]]);
        let token = steady(&data, SEARCH);
        let scene = json!({
            "here": here(3100, 3300, 1),
            "locs": [loc(11, 3209, 3218, 1, &["Search"])],
        });
        // Frozen: wait, and neither verb rides along with it.
        on_pause();
        let paused = call(&data, token, page.clone(), scene.clone());
        assert_eq!(paused["kind"], "wait", "{paused}");
        on_resume();
        on_hold(true);
        let held_clock = call(&data, token, page.clone(), scene.clone());
        assert_eq!(held_clock["kind"], "wait", "{held_clock}");
        on_hold(false);
        // The posted `hold || ours` interrupt, unfrozen: yield, still no verb.
        let yielded = call(
            &data,
            token,
            page.clone(),
            json!({
                "here": here(3100, 3300, 1),
                "locs": [loc(11, 3209, 3218, 1, &["Search"])],
                "hold": true,
            }),
        );
        assert_eq!(yielded["kind"], "yield", "{yielded}");
        for step in [&paused, &held_clock, &yielded] {
            assert!(step.get("x").is_none(), "{step}");
            assert!(step.get("action").is_none(), "{step}");
            assert!(step.get("id").is_none(), "{step}");
        }
        // The walk is still there once the interrupt clears.
        let walking = call(&data, token, page, scene);
        assert_eq!(walking["kind"], "walk", "{walking}");
    }

    #[test]
    fn non_search_rows_stay_idle_over_a_fully_posted_scene() {
        on_reset();
        let data = selected();
        let scene = json!({
            "here": here(3209, 3218, 1),
            "locs": [loc(11, 3209, 3218, 1, &["Search"])],
        });
        for id in [CLUE, MAP, RIDDLE, CASKET] {
            let page = json!([[id, 1]]);
            let token = steady(&data, id);
            for _ in 0..2 {
                let idle = call(&data, token, page.clone(), scene.clone());
                assert_eq!(idle["kind"], "wait", "{id} {idle}");
                assert_eq!(token_of(&idle), token, "{id} {idle}");
                assert!(idle.get("x").is_none(), "{id} {idle}");
                assert!(idle.get("action").is_none(), "{id} {idle}");
            }
        }
    }

    #[test]
    fn the_search_row_emits_only_walk_and_loc_and_never_a_completion() {
        on_reset();
        let data = selected();
        let page = json!([[SEARCH, 1]]);
        let token = steady(&data, SEARCH);
        let steps = vec![
            call(
                &data,
                token,
                page.clone(),
                json!({ "here": here(3200, 3218, 1) }),
            ),
            call(
                &data,
                token,
                page.clone(),
                json!({
                    "here": here(3209, 3218, 1),
                    "locs": [loc(11, 3209, 3218, 1, &["Search"])],
                }),
            ),
            call(
                &data,
                token,
                page.clone(),
                json!({ "here": here(3209, 3218, 1) }),
            ),
            call(&data, token, page, json!({ "hold": true })),
        ];
        for step in &steps {
            let text = step.to_string();
            for forbidden in ["clue solved", "abandon", "supplies-needed", "dead", "done"] {
                assert!(!text.contains(forbidden), "{step}");
            }
            assert!(
                step["status"].is_null(),
                "the machine emits kinds, not the public status: {step}"
            );
            assert!(
                matches!(
                    step["kind"].as_str().unwrap_or(""),
                    "walk" | "loc" | "wait" | "yield"
                ),
                "the search row emits walk, loc, wait or yield only: {step}"
            );
        }
        assert_eq!(
            steps
                .iter()
                .map(|step| step["kind"].clone())
                .collect::<Vec<_>>(),
            vec![json!("walk"), json!("loc"), json!("wait"), json!("yield"),],
            "{steps:?}"
        );
    }

    #[test]
    fn an_unknown_op_is_not_impl() {
        on_reset();
        let data = selected();
        let step = dispatch(Some(&data), &json!({ "op": "solve" }));
        assert_eq!(step["kind"], "notImpl", "{step}");
    }
}
