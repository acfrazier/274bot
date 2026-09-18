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
//! id as failed. Arrival still requires Chebyshev ≤ radius and same level.
//! Genuinely pending follow (`None`) keeps the caller timeout.

use crate::isolate_fb::SnapshotReader;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

/// JS `Number` cannot uniquely represent integers above this.
const JS_MAX_SAFE: u64 = (1 << 53) - 1;
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

fn alloc_token(avoid: u64) -> u64 {
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
struct Tile {
    x: i32,
    z: i32,
    level: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WalkKey {
    tile: Tile,
    radius: i32,
    allow_teleports: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HostOutcome {
    seq: u64,
    generation: u64,
    request_id: u64,
    failed: bool,
    key: WalkKey,
}

impl HostOutcome {
    const fn empty() -> Self {
        Self {
            seq: 0,
            generation: 0,
            request_id: 0,
            failed: false,
            key: WalkKey {
                tile: Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                radius: 0,
                allow_teleports: false,
            },
        }
    }
}

struct Wait {
    token: u64,
    key: WalkKey,
    settled: Option<bool>,
    seq_at_begin: u64,
    matched_failure: bool,
}

struct WalkSlot {
    here: Option<Tile>,
    outcome: HostOutcome,
    wait: Option<Wait>,
}

impl WalkSlot {
    const fn new() -> Self {
        Self {
            here: None,
            outcome: HostOutcome::empty(),
            wait: None,
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    fn observe(&mut self, snap: &SnapshotReader<'_>) {
        if snap.has_here() {
            self.here = snap.here().map(|tile| Tile {
                x: tile.x(),
                z: tile.z(),
                level: tile.level(),
            });
        }
        if snap.has_walk_outcome_seq() {
            let outcome = HostOutcome {
                seq: snap.walk_outcome_seq(),
                generation: snap.walk_outcome_generation(),
                request_id: snap.walk_outcome_request_id(),
                failed: snap.walk_outcome_failed(),
                key: WalkKey {
                    tile: Tile {
                        x: snap.walk_outcome_x(),
                        z: snap.walk_outcome_z(),
                        level: snap.walk_outcome_level(),
                    },
                    radius: snap.walk_outcome_radius(),
                    allow_teleports: snap.walk_outcome_allow_teleports(),
                },
            };
            self.outcome = outcome;
            if let Some(wait) = self.wait.as_mut() {
                if Self::fail_matches(outcome, wait) {
                    wait.matched_failure = true;
                }
            }
        }
    }

    fn begin(&mut self, key: WalkKey) -> u64 {
        let token = alloc_token(self.outcome.request_id);
        self.wait = Some(Wait {
            token,
            key,
            settled: None,
            seq_at_begin: self.outcome.seq,
            matched_failure: false,
        });
        token
    }

    fn arrived(here: Tile, key: WalkKey) -> bool {
        if here.level != key.tile.level {
            return false;
        }
        let dist = (here.x - key.tile.x).abs().max((here.z - key.tile.z).abs());
        dist <= key.radius
    }

    fn fail_matches(outcome: HostOutcome, wait: &Wait) -> bool {
        outcome.failed
            && outcome.request_id != 0
            && outcome.request_id == wait.token
            && outcome.key == wait.key
            && outcome.seq != 0
            && outcome.seq != wait.seq_at_begin
    }

    fn poll(&mut self, token: u64) -> bool {
        let Some(wait) = self.wait.as_mut() else {
            return false;
        };
        if wait.token != token {
            return false;
        }
        if wait.settled.is_some() {
            return true;
        }
        if self.here.is_some_and(|here| Self::arrived(here, wait.key)) {
            wait.settled = Some(true);
            return true;
        }
        if wait.matched_failure {
            wait.settled = Some(false);
            return true;
        }
        false
    }

    fn value(&self, token: u64) -> bool {
        self.wait
            .as_ref()
            .is_some_and(|wait| wait.token == token && wait.settled == Some(true))
    }
}

pub(crate) fn on_snapshot(snap: &SnapshotReader<'_>) {
    SLOT.with(|slot| slot.borrow_mut().observe(snap));
}

pub(crate) fn on_reset() {
    SLOT.with(|slot| slot.borrow_mut().reset());
}

pub(crate) fn on_pause() {}

pub(crate) fn on_resume() {}

pub(crate) fn on_hold(_held: bool) {}

pub(crate) fn dispatch(input: &Value) -> Value {
    let op = input.get("op").and_then(Value::as_str).unwrap_or("");
    SLOT.with(|slot| {
        let mut slot = slot.borrow_mut();
        match op {
            "begin" => {
                let token = slot.begin(WalkKey {
                    tile: Tile {
                        x: json_i32(input.get("x")),
                        z: json_i32(input.get("z")),
                        level: json_i32(input.get("level")),
                    },
                    radius: json_i32(input.get("radius")),
                    allow_teleports: input
                        .get("allow_teleports")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                });
                json!(token)
            }
            "settled" => json!(slot.poll(json_u64(input.get("token")))),
            "value" => json!(slot.value(json_u64(input.get("token")))),
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
            widgets: &[],
        }
    }

    fn observe(input: SnapshotInput<'_>, native: NativeFactsInput<'_>) {
        let bytes = encode_snapshot_with_native(&input, native);
        let snap = SnapshotReader::from_bytes(&bytes).expect("snapshot");
        on_snapshot(&snap);
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
        on_snapshot(&snap);
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
}
