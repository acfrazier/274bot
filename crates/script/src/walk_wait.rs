//! Isolate-owned matching for a single native walk wait.
//!
//! The host publishes a bounded walk outcome on the FlatBuffer snapshot.
//! This module decides whether that outcome, or actual arrival, settles the
//! wait. JavaScript only marshals arguments and awaits the callback.
//!
//! Correlation is the isolate-allocated request id (`Wait.token`), carried on
//! the existing FlatBuffer walk request and echoed in the host outcome. Tokens
//! are process-wide so a Stop/Start or watchdog restart cannot reuse token 1
//! against an uncleared host outcome or a late old worker. `route_generation`
//! stays the host worker / retained-route token and is not used to match waits.
//!
//! A late outcome from an earlier same-target request cannot settle a new
//! wait: request ids differ, and a leftover outcome observed before `begin`
//! cannot match even if ids collide after rollover. Host coalescing keeps the
//! in-flight id and fail-closes a distinct nonzero wait. Request id `0`
//! (old buffers / ctx.walk) never settles a wait.
//!
//! Mid-follow Stall / Refused / Blocked / GaveUp publish the armed request
//! id as failed. The end of the armed walk's route publishes it as not
//! failed: frozen `WalkExecutor` returns true at the path terminal even when
//! `isArrived` is false there (`WalkExecutor.ts:316-325`, `'closest'`).
//! Arrival is [`api::query::is_arrived`], the frozen `isArrived` over the
//! last posted reach view — the rule the host follow ends on too. Genuinely
//! pending follow (`None`) keeps the caller timeout.

use crate::isolate_fb::SnapshotReader;
use crate::observed::{self, Scene};
use api::snapshot::WorldTile;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

/// JS `Number` cannot uniquely represent integers above this.
const JS_MAX_SAFE: u64 = (1 << 53) - 1;
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

pub(crate) fn alloc_token(avoid: u64) -> u64 {
    loop {
        let cur = NEXT_TOKEN.load(Ordering::Relaxed);
        let token = match cur {
            0 => 1,
            n if n > JS_MAX_SAFE => 1,
            n => n,
        };
        let next = if token >= JS_MAX_SAFE { 1 } else { token + 1 };
        if NEXT_TOKEN
            .compare_exchange_weak(cur, next, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            continue;
        }
        if token != avoid {
            return token;
        }
    }
}

thread_local! {
    static SLOT: RefCell<WalkSlot> = const { RefCell::new(WalkSlot::new()) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WalkKey {
    tile: WorldTile,
    radius: i32,
    allow_teleports: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HostOutcome {
    seq: u64,
    generation: u64,
    request_id: u64,
    failed: bool,
    blocked: bool,
    key: WalkKey,
}

impl HostOutcome {
    const fn empty() -> Self {
        Self {
            seq: 0,
            generation: 0,
            request_id: 0,
            failed: false,
            blocked: false,
            key: WalkKey {
                tile: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                radius: 0,
                allow_teleports: false,
            },
        }
    }

    /// The host's last published outcome, or the empty one before any.
    fn posted(scene: &Scene) -> Self {
        scene
            .latest()
            .walk_outcome()
            .map_or(Self::empty(), |o| Self {
                seq: o.seq,
                generation: o.generation,
                request_id: o.request_id,
                failed: o.failed,
                blocked: o.blocked,
                key: WalkKey {
                    tile: WorldTile {
                        x: o.tile.x,
                        z: o.tile.z,
                        level: o.tile.level,
                    },
                    radius: o.radius,
                    allow_teleports: o.allow_teleports,
                },
            })
    }
}

struct Wait {
    token: u64,
    key: WalkKey,
    settled: Option<bool>,
    seq_at_begin: u64,
    /// The host's terminal outcome for this request: `Some(false)` once a
    /// failure matched (sticky), `Some(true)` for a route end.
    matched: Option<bool>,
    /// The matched route end is frozen `'blocked'` (`Traversal.ts:171–174`).
    blocked: bool,
}

/// The one live wait. The player tile and the host outcome are read from
/// the isolate scene; only the wait itself is this module's state.
struct WalkSlot {
    wait: Option<Wait>,
}

impl WalkSlot {
    const fn new() -> Self {
        Self { wait: None }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    /// A post that carried an outcome for the live request is latched now,
    /// so a later outcome cannot hide it. A failure is never overwritten.
    fn observe_outcome(&mut self, outcome: HostOutcome) {
        if let Some(wait) = self.wait.as_mut() {
            if Self::outcome_matches(outcome, wait) {
                wait.matched = Some(wait.matched.unwrap_or(true) && !outcome.failed);
                wait.blocked = wait.matched == Some(true) && outcome.blocked;
            }
        }
    }

    fn begin(&mut self, key: WalkKey, outcome: HostOutcome) -> u64 {
        let token = alloc_token(outcome.request_id);
        self.wait = Some(Wait {
            token,
            key,
            settled: None,
            seq_at_begin: outcome.seq,
            matched: None,
            blocked: false,
        });
        token
    }

    /// Frozen `isArrived` over the isolate's cached reach view (the same
    /// view the V8 reach helpers read; no copy, no flood).
    fn arrived(here: WorldTile, key: WalkKey) -> bool {
        crate::load::reach_query::with_view(|view| {
            api::query::is_arrived(here, key.tile, key.radius, || view)
        })
    }

    fn outcome_matches(outcome: HostOutcome, wait: &Wait) -> bool {
        outcome.request_id != 0
            && outcome.request_id == wait.token
            && outcome.key == wait.key
            && outcome.seq != 0
            && outcome.seq != wait.seq_at_begin
    }

    fn poll(&mut self, token: u64, here: Option<WorldTile>) -> bool {
        let Some(wait) = self.wait.as_mut() else {
            return false;
        };
        if wait.token != token {
            return false;
        }
        if wait.settled.is_some() {
            return true;
        }
        if here.is_some_and(|here| Self::arrived(here, wait.key)) {
            wait.settled = Some(true);
            return true;
        }
        // A delta snapshot may omit the walk-outcome family after the
        // host published. Re-read the merged scene each poll so a
        // matching note_failure still settles if on_snapshot missed
        // the one post that carried the field.
        if wait.matched.is_none() {
            let outcome = observed::with(HostOutcome::posted);
            if Self::outcome_matches(outcome, wait) {
                wait.matched = Some(!outcome.failed);
                wait.blocked = !outcome.failed && outcome.blocked;
            }
        }
        if let Some(value) = wait.matched {
            wait.settled = Some(value);
            return true;
        }
        false
    }

    fn value(&self, token: u64) -> bool {
        self.wait
            .as_ref()
            .is_some_and(|wait| wait.token == token && wait.settled == Some(true))
    }

    /// The wait settled on a frozen `'blocked'` route end, not on arrival.
    fn blocked(&self, token: u64) -> bool {
        self.wait
            .as_ref()
            .is_some_and(|wait| wait.token == token && wait.settled == Some(true) && wait.blocked)
    }
}

/// After the isolate applied `snap` to the scene.
pub(crate) fn on_snapshot(snap: &SnapshotReader<'_>) {
    if !snap.has_walk_outcome_seq() {
        return;
    }
    let outcome = observed::with(HostOutcome::posted);
    SLOT.with(|slot| slot.borrow_mut().observe_outcome(outcome));
}

pub(crate) fn on_reset() {
    SLOT.with(|slot| slot.borrow_mut().reset());
}

pub(crate) fn on_pause() {}

pub(crate) fn on_resume() {}

pub(crate) fn on_hold(_held: bool) {}

/// Whether `token` still owns the isolate's single native-walk wait.
///
/// This is an internal ownership probe, not a shim operation: a timed-out
/// resilient walk must not send a tokenless scene click after another walk
/// has replaced its wait.
pub(crate) fn owns(token: u64) -> bool {
    SLOT.with(|slot| {
        slot.borrow()
            .wait
            .as_ref()
            .is_some_and(|wait| wait.token == token)
    })
}

pub(crate) fn dispatch(input: &Value) -> Value {
    let op = input.get("op").and_then(Value::as_str).unwrap_or("");
    SLOT.with(|slot| {
        let mut slot = slot.borrow_mut();
        match op {
            "begin" => {
                let key = WalkKey {
                    tile: WorldTile {
                        x: json_i32(input.get("x")),
                        z: json_i32(input.get("z")),
                        level: json_i32(input.get("level")),
                    },
                    radius: json_i32(input.get("radius")),
                    allow_teleports: input
                        .get("allow_teleports")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                };
                let token = slot.begin(key, observed::with(HostOutcome::posted));
                json!(token)
            }
            "settled" => {
                json!(slot.poll(
                    json_u64(input.get("token")),
                    crate::load::reach_query::posted_here()
                ))
            }
            "value" => json!(slot.value(json_u64(input.get("token")))),
            "blocked" => json!(slot.blocked(json_u64(input.get("token")))),
            _ => Value::Null,
        }
    })
}

fn json_i32(value: Option<&Value>) -> i32 {
    value
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
        .unwrap_or(0)
}

fn json_u64(value: Option<&Value>) -> u64 {
    value
        .and_then(Value::as_u64)
        .or_else(|| {
            value
                .and_then(Value::as_i64)
                .and_then(|n| u64::try_from(n).ok())
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isolate_fb::{
        encode_snapshot, encode_snapshot_with_native, NativeFactsInput, ReachViewInput,
        SnapshotInput, TileInput,
    };

    /// `ResetSession` as the isolate runs it: the scene and the wait.
    fn on_reset() {
        crate::observed::on_reset();
        crate::load::reach_query::on_reset();
        super::on_reset();
    }

    /// One decoded post, applied the way the isolate applies it.
    fn post(snap: &SnapshotReader<'_>) {
        crate::observed::apply(snap);
        crate::load::reach_query::apply(snap);
        on_snapshot(snap);
    }

    fn empty_input(tick: u64) -> SnapshotInput<'static> {
        SnapshotInput {
            tick,
            here: None,
            ingame: true,
            inv: &[],
            inv_size: 28,
            stats: &[],
            booths: &[],
            nearest_booth: None,
            banks: &[],
            bank: &[],
            bank_side: &[],
            bank_open: false,
            bank_loaded: false,
            bank_generation: 0,
            count_dialog_open: false,
            withdraw_x_result_seq: 0,
            withdraw_x_result: false,
            withdraw_load_result_seq: 0,
            withdraw_load_result: false,
            bank_op_result_seq: 0,
            bank_op_result: false,
            hold: false,
            ours: false,
            npcs: &[],
            locs: &[],
            players: &[],
            ground: &[],
            equipment: &[],
            chat_open: false,
            chat_continue: false,
            chat_text: None,
            chat_options: &[],
            side_tab: -1,
            varps: &[],
            combat_styles: &[],
            run_energy: 0,
            run_enabled: false,
            retaliate_enabled: false,
            my_name: None,
            in_combat: false,
            animating: false,
            main_modal_id: -1,
            chat_modal_id: -1,
            make_products: &[],
            side_tab_ifaces: &[],
            spell_buttons: &[],
            chat_lines: &[],
            bank_note_on: -1,
            bank_note_off: -1,
            scene_state: 0,
            weight: 0,
            combat_level: 0,
            camera_yaw: 0,
            camera_pitch: 0,
            teleports_enabled: false,
            self_slot: 0,
            trade_offer_open: false,
            trade_confirm_open: false,
            trade_partner: None,
            trade_mine: &[],
            trade_theirs: &[],
            trade_side: &[],
            trade_accept_id: -1,
            trade_decline_id: -1,
            shop_open: false,
            shop_stock: &[],
            reach: ReachViewInput::UNAVAILABLE,
            attacked_by_player: false,
            self_target_kind: 0,
            self_target_index: -1,
            widgets: &[],
        }
    }

    fn observe(input: SnapshotInput<'_>, native: NativeFactsInput<'_>) {
        let bytes = encode_snapshot_with_native(&input, native);
        let snap = SnapshotReader::from_bytes(&bytes).expect("snapshot");
        post(&snap);
    }

    fn begin(x: i32, z: i32, level: i32, radius: i32, allow_teleports: bool) -> u64 {
        dispatch(&json!({
            "op": "begin",
            "x": x,
            "z": z,
            "level": level,
            "radius": radius,
            "allow_teleports": allow_teleports,
        }))
        .as_u64()
        .expect("token")
    }

    fn settled(token: u64) -> bool {
        dispatch(&json!({ "op": "settled", "token": token }))
            .as_bool()
            .unwrap_or(false)
    }

    fn value(token: u64) -> bool {
        dispatch(&json!({ "op": "value", "token": token }))
            .as_bool()
            .unwrap_or(false)
    }

    #[allow(clippy::too_many_arguments)] // walk-fail fixture packs outcome fields
    fn fail_native(
        seq: u64,
        generation: u64,
        request_id: u64,
        x: i32,
        z: i32,
        level: i32,
        radius: i32,
        allow_teleports: bool,
    ) -> NativeFactsInput<'static> {
        NativeFactsInput {
            walk_outcome_seq: seq,
            walk_outcome_generation: generation,
            walk_outcome_request_id: request_id,
            walk_outcome_failed: true,
            walk_outcome_x: x,
            walk_outcome_z: z,
            walk_outcome_level: level,
            walk_outcome_radius: radius,
            walk_outcome_allow_teleports: allow_teleports,
            ..Default::default()
        }
    }

    #[test]
    fn queued_nopath_settles_false_for_matching_request() {
        on_reset();
        let token = begin(2820, 3556, 0, 1, false);
        assert!(!settled(token));
        let mut input = empty_input(2);
        input.here = Some(TileInput {
            x: 2823,
            z: 3555,
            level: 0,
        });
        observe(input, fail_native(1, 1, token, 2820, 3556, 0, 1, false));
        assert!(settled(token));
        assert!(!value(token));
    }

    #[test]
    fn nopath_merged_in_scene_without_outcome_family_still_settles() {
        on_reset();
        let token = begin(2998, 3916, 0, 1, false);
        assert!(!settled(token));
        crate::observed::post(2, |p| {
            p.walk_outcome(crate::observed::WalkOutcome {
                seq: 1,
                generation: 1,
                request_id: token,
                failed: true,
                tile: crate::observed::Tile {
                    x: 2998,
                    z: 3916,
                    level: 0,
                },
                radius: 1,
                allow_teleports: false,
                blocked: false,
            });
        });
        assert!(
            settled(token),
            "note_failure must settle even when the next snapshot omits walk_outcome_seq"
        );
        assert!(!value(token));
    }

    #[test]
    fn arrival_still_wins_after_a_matching_failure_was_observed() {
        on_reset();
        let token = begin(2820, 3556, 0, 1, false);
        let mut failed = empty_input(2);
        failed.here = Some(TileInput {
            x: 2823,
            z: 3555,
            level: 0,
        });
        observe(failed, fail_native(1, 1, token, 2820, 3556, 0, 1, false));

        let mut arrived = empty_input(3);
        arrived.here = Some(TileInput {
            x: 2821,
            z: 3556,
            level: 0,
        });
        observe(arrived, NativeFactsInput::default());

        assert!(settled(token));
        assert!(
            value(token),
            "arrival keeps precedence at the eligible poll"
        );
    }

    #[test]
    fn arrival_within_radius_and_level_settles_true() {
        on_reset();
        let token = begin(2820, 3556, 0, 1, false);
        let mut input = empty_input(2);
        input.here = Some(TileInput {
            x: 2821,
            z: 3556,
            level: 0,
        });
        observe(input, NativeFactsInput::default());
        assert!(settled(token));
        assert!(value(token));
    }

    #[test]
    fn pending_without_outcome_or_arrival_does_not_settle() {
        on_reset();
        let token = begin(2820, 3556, 0, 1, false);
        let mut input = empty_input(2);
        input.here = Some(TileInput {
            x: 2823,
            z: 3555,
            level: 0,
        });
        observe(input, NativeFactsInput::default());
        assert!(!settled(token));
        assert!(!value(token));
    }

    #[test]
    fn other_request_nopath_cannot_settle() {
        on_reset();
        let token = begin(2820, 3556, 0, 1, false);
        let mut input = empty_input(2);
        input.here = Some(TileInput {
            x: 2823,
            z: 3555,
            level: 0,
        });
        observe(input, fail_native(1, 1, 1, 3200, 3200, 0, 1, false));
        assert!(!settled(token));
    }

    #[test]
    fn already_observed_seq_cannot_settle_a_later_wait() {
        on_reset();
        let mut input = empty_input(1);
        input.here = Some(TileInput {
            x: 2823,
            z: 3555,
            level: 0,
        });
        observe(input, fail_native(4, 4, 4, 2820, 3556, 0, 1, false));
        let token = begin(2820, 3556, 0, 1, false);
        assert!(
            !settled(token),
            "stale published fail must not settle a new wait"
        );
    }

    #[test]
    fn superseded_token_cannot_receive_the_later_result() {
        on_reset();
        let first = begin(2820, 3556, 0, 1, false);
        let second = begin(3200, 3200, 0, 0, false);
        let mut input = empty_input(2);
        input.here = Some(TileInput {
            x: 2823,
            z: 3555,
            level: 0,
        });
        observe(input, fail_native(1, 1, first, 2820, 3556, 0, 1, false));
        assert!(!settled(first));
        assert!(!settled(second));
        assert!(!value(first));
        assert!(!value(second));
    }

    #[test]
    fn reset_invalidates_the_current_wait() {
        on_reset();
        let token = begin(2820, 3556, 0, 1, false);
        on_reset();
        let mut input = empty_input(2);
        input.here = Some(TileInput {
            x: 2821,
            z: 3556,
            level: 0,
        });
        observe(input, fail_native(1, 1, 1, 2820, 3556, 0, 1, false));
        assert!(!settled(token));
        assert!(!value(token));
    }

    #[test]
    fn same_target_wrong_request_id_cannot_settle() {
        on_reset();
        let token = begin(2820, 3556, 0, 1, false);
        let mut later = empty_input(2);
        later.here = Some(TileInput {
            x: 2823,
            z: 3555,
            level: 0,
        });
        observe(later, fail_native(2, 5, 5, 2820, 3556, 0, 1, false));
        assert!(
            !settled(token),
            "late same-target NoPath for a different request id must not settle"
        );
        let mut newer = empty_input(3);
        newer.here = Some(TileInput {
            x: 2823,
            z: 3555,
            level: 0,
        });
        observe(newer, fail_native(3, 6, token, 2820, 3556, 0, 1, false));
        assert!(settled(token));
        assert!(!value(token));
    }

    #[test]
    fn request_id_zero_after_empty_begin_does_not_settle() {
        on_reset();
        let token = begin(2820, 3556, 0, 1, false);
        let mut input = empty_input(2);
        input.here = Some(TileInput {
            x: 2823,
            z: 3555,
            level: 0,
        });
        observe(input, fail_native(1, 0, 0, 2820, 3556, 0, 1, false));
        assert!(
            !settled(token),
            "request id 0 is not a correlated isolate wait"
        );
    }

    #[test]
    fn omitted_walk_outcome_fields_do_not_fail_a_wait() {
        on_reset();
        let token = begin(2820, 3556, 0, 1, false);
        let mut input = empty_input(2);
        input.here = Some(TileInput {
            x: 2823,
            z: 3555,
            level: 0,
        });
        let bytes = encode_snapshot(&input);
        let snap = SnapshotReader::from_bytes(&bytes).expect("snapshot");
        post(&snap);
        assert!(!settled(token));
    }

    #[test]
    fn leftover_outcome_observed_before_begin_does_not_settle() {
        on_reset();
        let mut input = empty_input(1);
        input.here = Some(TileInput {
            x: 2823,
            z: 3555,
            level: 0,
        });
        observe(input, fail_native(1, 1, 1, 2820, 3556, 0, 1, false));
        let token = begin(2820, 3556, 0, 1, false);
        assert!(
            !settled(token),
            "host leftover observed before begin must not settle a new wait"
        );
    }

    #[test]
    fn reset_then_new_wait_does_not_consume_late_old_request_id() {
        on_reset();
        let first = begin(2820, 3556, 0, 1, false);
        let mut prior = empty_input(1);
        prior.here = Some(TileInput {
            x: 2823,
            z: 3555,
            level: 0,
        });
        observe(prior, fail_native(1, 1, first, 2820, 3556, 0, 1, false));
        on_reset();
        let second = begin(2820, 3556, 0, 1, false);
        assert_ne!(
            second, first,
            "isolate restart must not reuse the previous wait token"
        );
        let mut late = empty_input(2);
        late.here = Some(TileInput {
            x: 2823,
            z: 3555,
            level: 0,
        });
        observe(late, fail_native(2, 1, first, 2820, 3556, 0, 1, false));
        assert!(
            !settled(second),
            "late old-worker request id must not settle a post-reset wait"
        );
        let mut matched = empty_input(3);
        matched.here = Some(TileInput {
            x: 2823,
            z: 3555,
            level: 0,
        });
        observe(matched, fail_native(3, 2, second, 2820, 3556, 0, 1, false));
        assert!(settled(second));
        assert!(!value(second));
    }

    #[test]
    fn new_begin_invalidates_an_unpolled_matching_failure() {
        on_reset();
        let first = begin(2820, 3556, 0, 1, false);
        let mut failed = empty_input(1);
        failed.here = Some(TileInput {
            x: 2823,
            z: 3555,
            level: 0,
        });
        observe(failed, fail_native(1, 1, first, 2820, 3556, 0, 1, false));

        let second = begin(2820, 3556, 0, 1, false);
        assert_ne!(second, first);
        assert!(
            !settled(second),
            "a new wait must not inherit the superseded wait's matched failure"
        );
    }

    #[test]
    fn wrong_level_is_not_arrival() {
        on_reset();
        let token = begin(2820, 3556, 0, 8, false);
        let mut input = empty_input(2);
        input.here = Some(TileInput {
            x: 2820,
            z: 3556,
            level: 1,
        });
        observe(input, NativeFactsInput::default());
        assert!(!settled(token));
    }

    /// The host's packed reach for `here` on a 20x20 scene at (2810,3546)
    /// split by a closed wall between world rows z=3555 and z=3556.
    fn walled_view(here: WorldTile) -> api::query::ReachQueryView {
        use client::dash3d::CollisionFlag;
        let mut scene = api::snapshot::SceneView {
            available: true,
            base_x: 2810,
            base_z: 3546,
            level: 0,
            width: 20,
            height: 20,
            collision_flags: vec![0; 400],
        };
        for lx in 0..20 {
            scene.collision_flags[lx * 20 + 9] |= CollisionFlag::W_N;
            scene.collision_flags[lx * 20 + 10] |= CollisionFlag::W_S;
        }
        let flood = api::query::SceneQuery::new(&scene, Some(here)).flood_reach();
        api::query::pack_reach_query(&scene, flood.as_ref())
    }

    fn observe_at(tick: u64, here: WorldTile, view: &api::query::ReachQueryView) {
        observe_at_with(tick, here, view, NativeFactsInput::default());
    }

    fn observe_at_with(
        tick: u64,
        here: WorldTile,
        view: &api::query::ReachQueryView,
        native: NativeFactsInput<'_>,
    ) {
        let mut input = empty_input(tick);
        input.here = Some(TileInput {
            x: here.x,
            z: here.z,
            level: here.level,
        });
        input.reach = ReachViewInput {
            available: view.available,
            base_x: view.base_x,
            base_z: view.base_z,
            level: view.level,
            width: view.width,
            height: view.height,
            walkable: &view.walkable,
            reachable: &view.reachable,
            reachable_adj: &view.reachable_adj,
            exact_rank: &view.exact_rank,
            adjacent_rank: &view.adjacent_rank,
            step: &view.step,
            canlight: &view.canlight,
            stamp: tick,
        };
        observe(input, native);
    }

    #[test]
    fn radius_through_a_closed_wall_does_not_settle_until_the_far_side() {
        on_reset();
        let token = begin(2820, 3557, 0, 2, false);
        let near = WorldTile {
            x: 2820,
            z: 3555,
            level: 0,
        };
        observe_at(2, near, &walled_view(near));
        assert!(
            !settled(token),
            "Chebyshev 2 through the wall is not arrival (frozen isArrived)"
        );

        let far = WorldTile {
            x: 2821,
            z: 3556,
            level: 0,
        };
        observe_at(3, far, &walled_view(far));
        assert!(settled(token), "reachable in-radius tile settles");
        assert!(value(token));
    }

    #[test]
    fn a_flood_from_another_tile_cannot_settle_a_reach_probe() {
        on_reset();
        let token = begin(2820, 3557, 0, 2, false);
        let far = WorldTile {
            x: 2821,
            z: 3556,
            level: 0,
        };
        // Flooded from the dest's own side of the wall: this flood does
        // reach the dest, so only the origin guard keeps it from answering
        // for `far`.
        let stale = walled_view(WorldTile {
            x: 2822,
            z: 3557,
            level: 0,
        });
        observe_at(2, far, &stale);
        assert!(
            !settled(token),
            "reach probes run from here; a flood from elsewhere answers nothing"
        );
    }

    /// Open 64x64 scene at (2790,3530): no walls, so reach is pure BFS rank.
    fn open_view(here: WorldTile) -> api::query::ReachQueryView {
        let scene = api::snapshot::SceneView {
            available: true,
            base_x: 2790,
            base_z: 3530,
            level: 0,
            width: 64,
            height: 64,
            collision_flags: vec![0; 64 * 64],
        };
        let flood = api::query::SceneQuery::new(&scene, Some(here)).flood_reach();
        api::query::pack_reach_query(&scene, flood.as_ref())
    }

    fn route_end_native(seq: u64, request_id: u64, radius: i32) -> NativeFactsInput<'static> {
        NativeFactsInput {
            walk_outcome_seq: seq,
            walk_outcome_generation: 1,
            walk_outcome_request_id: request_id,
            walk_outcome_failed: false,
            walk_outcome_x: 2820,
            walk_outcome_z: 3557,
            walk_outcome_level: 0,
            walk_outcome_radius: radius,
            walk_outcome_allow_teleports: false,
            ..Default::default()
        }
    }

    /// AR-1 / frozen `'closest'`: an r=12 walk in open terrain ends on its
    /// approach tile, 12 tiles out, where every tile has BFS rank >= 529 >
    /// 512, so `isArrived` is false. The host's route-end outcome settles it.
    #[test]
    fn route_end_settles_true_where_is_arrived_is_false() {
        on_reset();
        let token = begin(2820, 3557, 0, 12, false);
        let approach = WorldTile {
            x: 2808,
            z: 3557,
            level: 0,
        };
        let view = open_view(approach);
        observe_at(2, approach, &view);
        assert!(
            !settled(token),
            "ring 12 in open terrain is past the 512 reach budget"
        );
        observe_at_with(3, approach, &view, route_end_native(1, token, 12));
        assert!(settled(token), "the route end settles the wait");
        assert!(value(token), "frozen 'closest' returns true");
    }

    #[test]
    fn route_end_cannot_hide_a_failure_or_settle_another_request() {
        on_reset();
        let token = begin(2820, 3557, 0, 12, false);
        let approach = WorldTile {
            x: 2808,
            z: 3557,
            level: 0,
        };
        let view = open_view(approach);
        observe_at_with(2, approach, &view, route_end_native(1, token + 1, 12));
        assert!(!settled(token), "another request's route end");
        observe_at_with(3, approach, &view, route_end_native(2, token, 11));
        assert!(!settled(token), "same id, another radius");

        let mut failed = route_end_native(3, token, 12);
        failed.walk_outcome_failed = true;
        observe_at_with(4, approach, &view, failed);
        observe_at_with(5, approach, &view, route_end_native(4, token, 12));
        assert!(settled(token));
        assert!(!value(token), "a matched failure stays failed");
    }
}
