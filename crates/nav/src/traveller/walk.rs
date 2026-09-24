use super::*;

/// Distinct game ticks a sent walk may sit without tile progress, with
/// no map flag and no movement, before one same-aim recovery reissue.
/// Counts at the latest observed position (partial-hop progress still
/// recovers once the player stops). Covers the short engine freeze
/// window (Bind ~3 ticks); matches the canonical WalkExecutor stallTicks
/// default of 5. Not a second hop budget.
pub(super) const WALK_STALL_RECOVER_IDLE_TICKS: u32 = 5;

pub(super) fn report_walk(
    options: &mut TravelOptions<'_>,
    snapshot: &GameSnapshot,
    at: WorldTile,
    aim: WorldTile,
    result: &SendResult<'_>,
) {
    if crate::debug_enabled() {
        let refusal = match result {
            SendResult::Sent { .. } => None,
            SendResult::Refused { reason, .. } => Some(*reason),
        };
        eprintln!(
            "[nav-walk-send] tick={} at={at:?} aim={aim:?} sent={} refusal={refusal:?} actor={:?} map_flag={:?}",
            snapshot.tick(), refusal.is_none(),
            snapshot.local_player().map(|p| (p.player.actor.moving, p.player.actor.running, p.player.actor.in_combat, &p.player.actor.target)),
            snapshot.map_flag()
        );
    }
    if let Some(cb) = options.on_event.as_mut() {
        let refusal = match result {
            SendResult::Sent { .. } => None,
            SendResult::Refused { reason, .. } => Some(*reason),
        };
        cb(TravelEvent::WalkAttempt {
            tick: snapshot.tick(),
            at,
            aim,
            refusal,
        });
    }
}

/// The next walk-hop target: the leg's last tile when it is within ~20
/// tiles, otherwise a tile ~15 steps ahead of the player (or the last
/// uncleared tile when the leg is shorter), so each hop re-routes a fresh
/// short path and never aims back toward the leg start.
pub(super) fn pick_aim(tiles: &[WorldTile], here: WorldTile, cursor: usize) -> (WorldTile, usize) {
    let last = *tiles.last().expect("walk legs are non-empty");
    if cheb(here, last) <= 20 {
        (last, tiles.len() - 1)
    } else {
        let base = tiles
            .iter()
            .position(|t| *t == here)
            .filter(|&i| i >= cursor)
            .unwrap_or(cursor);
        let idx = (base + 15).min(tiles.len() - 1).max(base + 1);
        (tiles[idx], idx)
    }
}

/// [`pick_aim`] then walk the index back until the tile is inside the
/// loaded scene, so click-ahead cannot `OffScene` at the 104-edge.
pub(super) fn pick_aim_in_scene(
    tiles: &[WorldTile],
    here: WorldTile,
    cursor: usize,
    scene: &api::snapshot::SceneView,
) -> (WorldTile, usize) {
    let (aim, mut idx) = pick_aim(tiles, here, cursor);
    if tile_in_scene(scene, aim) {
        return (aim, idx);
    }
    let here_i = tiles
        .iter()
        .position(|t| *t == here)
        .filter(|&i| i >= cursor)
        .unwrap_or(cursor);
    while idx > here_i && !tile_in_scene(scene, tiles[idx]) {
        idx -= 1;
    }
    (tiles[idx], idx)
}

impl FollowRun {
    /// One walk-hop step: send an armed (unsent) hop, or settle the sent
    /// one against the `arrived` arm and the hop budget. A still-watching
    /// settle ends the call; a matched mid-leg hop click-aheads the next
    /// walk on this same poll so the player does not stop on the hop tile.
    pub(super) fn poll_walk<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        options: &mut TravelOptions,
    ) -> Poll {
        let mut hop = self.walk.take().expect("walk hop present");
        let here = here(snapshot);
        if !hop.sent {
            return self.send_walk_hop(d, snapshot, options, hop, here);
        }
        // Last tile of the walk is the dest: exact stand. Mid-hops
        // click-ahead at Chebyshev 2 (at least `close_enough`) so the
        // next walk goes out while the player is still moving.
        const CLICK_AHEAD: i32 = 2;
        let last = hop.aim_index + 1 >= hop.tiles().len();
        let radius = if last {
            0
        } else {
            self.close_enough.max(CLICK_AHEAD)
        };
        let arms = [("arrived", arrived(hop.aim, radius))];
        if crate::debug_enabled() {
            eprintln!(
                "[nav-walk] tick={} here={here:?} aim={:?} radius={radius} ticks_waited={} sent_tile={:?} actor={:?} map_flag={:?}",
                snapshot.tick(), hop.aim, hop.ticks_waited, hop.sent_tile,
                snapshot.local_player().map(|p| (p.player.actor.moving, p.player.actor.running, p.player.actor.in_combat, &p.player.actor.target)),
                snapshot.map_flag()
            );
        }
        let mut settle = Settle::new(
            SettleOptions {
                arms: &arms,
                // The run enforces the per-hop budget; the settle only
                // reports a disconnect (an arm read alone never lapses).
                budget_ticks: u32::MAX,
                budget_ms: None,
            },
            ReadContext::new(snapshot),
        );
        match settle.poll(ReadContext::new(snapshot)) {
            Some(Outcome::Matched { .. }) => {
                // The hop completed: the leg is done exactly when the aim
                // was its last tile (the hop's arrival already proved the
                // player is within `close_enough` of the aim).
                if hop.aim_index + 1 >= hop.tiles().len() {
                    fire_leg(options, &hop.leg(), LegPhase::Done);
                    self.leg_index += 1;
                    return Poll::LegDone;
                }
                // Mid-leg click-ahead: send the next hop on this poll.
                hop.cursor = hop.aim_index + 1;
                hop.sent = false;
                self.send_walk_hop(d, snapshot, options, hop, here)
            }
            // A disconnect ends the watch; the hop was effectively dropped.
            Some(Outcome::Expired { .. }) => {
                fire_leg(options, &hop.leg(), LegPhase::Failed);
                Poll::Terminal(TravelOutcome::Stalled {
                    at: here,
                    aiming: hop.aim,
                    why: HopFailure::Dropped,
                    tries: hop.tries.max(1),
                })
            }
            // `poll` never produces `Refused` (only `Interactions` does);
            // keep watching defensively.
            Some(Outcome::Refused { .. }) => {
                self.walk = Some(hop);
                Poll::Watching
            }
            None => {
                hop.ticks_waited += 1;
                if hop.ticks_waited > self.budget {
                    let why = if hop.sent_tile == Some(here) {
                        HopFailure::Dropped
                    } else {
                        HopFailure::Expired
                    };
                    fire_leg(options, &hop.leg(), LegPhase::Failed);
                    Poll::Terminal(TravelOutcome::Stalled {
                        at: here,
                        aiming: hop.aim,
                        why,
                        tries: hop.tries.max(1),
                    })
                } else if self.should_recover_cancelled_walk(&hop, snapshot, here) {
                    self.recover_cancelled_walk(d, snapshot, options, hop, here)
                } else {
                    self.note_walk_stall_idle(&mut hop, snapshot, here);
                    self.walk = Some(hop);
                    Poll::Watching
                }
            }
        }
    }

    /// True when a sent walk has been cancelled long enough to warrant
    /// one same-aim recovery: no map flag, not moving, recovery not yet
    /// spent, and enough *distinct* game ticks with no tile progress at
    /// the latest observed position (duplicate snapshot polls do not
    /// count). Progress away from the original `sent_tile` still qualifies
    /// once the player stops; `sent_tile` remains only for Dropped/Expired.
    pub(super) fn should_recover_cancelled_walk(
        &self,
        hop: &WalkHop,
        snapshot: &GameSnapshot,
        here: WorldTile,
    ) -> bool {
        if hop.stall_recovered || hop.sent_tile.is_none() {
            return false;
        }
        if snapshot.map_flag().is_some() {
            return false;
        }
        if snapshot
            .local_player()
            .is_some_and(|p| p.player.actor.moving)
        {
            return false;
        }
        let idle = if hop.stall_idle_at != Some(here) {
            // First observation at this tile after movement (or start).
            1
        } else {
            match hop.stall_idle_last_tick {
                Some(t) if t == snapshot.tick() => hop.stall_idle_ticks,
                _ => hop.stall_idle_ticks.saturating_add(1),
            }
        };
        idle >= WALK_STALL_RECOVER_IDLE_TICKS
    }

    /// Advance or clear the cancelled-walk idle counter. Only distinct
    /// snapshot ticks count; active map flag, actor movement, or an
    /// actual tile change resets the window so a live hop is never
    /// spuriously reissued. Idle is tracked at the latest observed tile
    /// so partial-hop progress can still recover once movement stops.
    pub(super) fn note_walk_stall_idle(&self, hop: &mut WalkHop, snapshot: &GameSnapshot, here: WorldTile) {
        let active = snapshot.map_flag().is_some()
            || snapshot
                .local_player()
                .is_some_and(|p| p.player.actor.moving);
        if hop.sent_tile.is_none() || active || hop.stall_recovered {
            hop.stall_idle_ticks = 0;
            hop.stall_idle_last_tick = None;
            hop.stall_idle_at = None;
            return;
        }
        if hop.stall_idle_at != Some(here) {
            hop.stall_idle_at = Some(here);
            hop.stall_idle_ticks = 0;
            hop.stall_idle_last_tick = None;
        }
        let tick = snapshot.tick();
        if hop.stall_idle_last_tick == Some(tick) {
            return;
        }
        hop.stall_idle_last_tick = Some(tick);
        hop.stall_idle_ticks = hop.stall_idle_ticks.saturating_add(1);
    }

    /// One same-aim walk reissue after a cancelled hop. Does not reset
    /// `ticks_waited` (hop budget stays finite), does not consume another
    /// `max_hops` slot, and does not re-pick aim. At most one recovery per
    /// stalled hop; further cancellation exhausts the original bound.
    pub(super) fn recover_cancelled_walk<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        options: &mut TravelOptions<'_>,
        mut hop: WalkHop,
        here: WorldTile,
    ) -> Poll {
        let aim = hop.aim;
        let mut ix = Interactions::new(snapshot, d);
        let result = ix.walk(aim);
        report_walk(options, snapshot, here, aim, &result);
        match result {
            SendResult::Sent { .. } => {
                hop.sent = true;
                hop.sent_tick = snapshot.tick();
                hop.sent_tile = Some(here);
                hop.tries = hop.tries.max(1) + 1;
                hop.stall_recovered = true;
                hop.stall_idle_ticks = 0;
                hop.stall_idle_last_tick = None;
                hop.stall_idle_at = None;
                // Keep ticks_waited: recovery is not a fresh hop budget.
                self.walk = Some(hop);
                Poll::Watching
            }
            SendResult::Refused {
                reason:
                    SendReason::OffScene | SendReason::Unreachable | SendReason::SceneUnavailable,
                ..
            } => {
                // Spend the recovery slot so we do not re-send every poll
                // while the scene is unavailable; the original tick budget
                // still bounds the hop.
                hop.stall_recovered = true;
                hop.stall_idle_ticks = 0;
                hop.stall_idle_last_tick = None;
                hop.stall_idle_at = None;
                self.walk = Some(hop);
                Poll::Watching
            }
            SendResult::Refused { reason, .. } => {
                fire_leg(options, &hop.leg(), LegPhase::Failed);
                Poll::Terminal(TravelOutcome::Refused { at: here, reason })
            }
        }
    }

    pub(super) fn send_walk_hop<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        options: &mut TravelOptions,
        mut hop: WalkHop,
        here: WorldTile,
    ) -> Poll {
        // A preceding transport may land directly on this walk's final
        // tile (cellar shifts can land one tile off their packed `to`).
        // Complete only on full endpoint equality, including level: an
        // intermediate/clipped self-aim still has route left to follow.
        if hop.tiles().last().copied() == Some(here) {
            fire_leg(options, &hop.leg(), LegPhase::Done);
            self.leg_index += 1;
            return Poll::LegDone;
        }
        if self.hops >= self.max_hops {
            fire_leg(options, &hop.leg(), LegPhase::Failed);
            return Poll::Terminal(TravelOutcome::GaveUp {
                at: here,
                hops: self.hops,
            });
        }
        let tiles = hop.tiles();
        let here_i = tiles
            .iter()
            .position(|t| *t == here)
            .filter(|&i| i >= hop.cursor)
            .unwrap_or(hop.cursor);
        let (mut aim, mut idx) = pick_aim_in_scene(tiles, here, hop.cursor, snapshot.scene());
        loop {
            if aim == here {
                self.walk = Some(hop);
                return Poll::Watching;
            }
            let mut ix = Interactions::new(snapshot, d);
            let result = ix.walk(aim);
            report_walk(options, snapshot, here, aim, &result);
            match result {
                SendResult::Sent { .. } => {
                    self.hops += 1;
                    hop.aim = aim;
                    hop.aim_index = idx;
                    hop.sent = true;
                    hop.sent_tick = snapshot.tick();
                    hop.ticks_waited = 0;
                    hop.sent_tile = Some(here);
                    hop.stall_idle_ticks = 0;
                    hop.stall_idle_last_tick = None;
                    hop.stall_idle_at = None;
                    hop.stall_recovered = false;
                    self.walk = Some(hop);
                    return Poll::Watching;
                }
                SendResult::Refused {
                    reason: SendReason::OffScene | SendReason::Unreachable,
                    ..
                } if idx > here_i + 1 => {
                    // Far hop left the scene or the local route died:
                    // walk the index back (the v1 traveller retried the
                    // tile after `here`). Click-ahead can outrun rebuild.
                    idx -= 1;
                    aim = tiles[idx];
                }
                SendResult::Refused {
                    reason:
                        SendReason::OffScene | SendReason::Unreachable | SendReason::SceneUnavailable,
                    ..
                } => {
                    // Rebuild (`scene_state != 2`) refuses the walk before
                    // a packet goes out — wait, do not fail the follow.
                    self.walk = Some(hop);
                    return Poll::Watching;
                }
                SendResult::Refused { reason, .. } => {
                    fire_leg(options, &hop.leg(), LegPhase::Failed);
                    return Poll::Terminal(TravelOutcome::Refused { at: here, reason });
                }
            }
        }
    }
}

