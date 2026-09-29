use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use api::snapshot::{GameSnapshot, WorldTile};
use nav::router::{
    find_first_with_avoid, find_first_with_fallback_avoid, find_missing_item_reqs_with_avoid,
    AvoidRect, FallbackRoute, FindOptions, MissingReq, Route,
};
use nav::traveller::Traveller;
use nav::world::NavWorld;
use nav::WorldState;

use super::{
    debug_enabled, fetch_stand, fetch_tile, fetchable_facts, route_inspect, route_or_bank_fetch,
    PendingBankFetch, RouteOutcome, StandFetch,
};
#[derive(Default)]
pub(crate) struct RouteCompletion {
    #[cfg(test)]
    sender: Option<std::sync::mpsc::Sender<()>>,
}

impl RouteCompletion {
    #[cfg(test)]
    fn channel() -> (Self, std::sync::mpsc::Receiver<()>) {
        let (sender, receiver) = std::sync::mpsc::channel();
        (
            Self {
                sender: Some(sender),
            },
            receiver,
        )
    }

    fn signal(self) {
        #[cfg(test)]
        if let Some(sender) = self.sender {
            let _ = sender.send(());
        }
    }
}

/// Host-published walk outcome copied onto the isolate snapshot.
#[derive(Clone, Copy, Default)]
pub(crate) struct PostedWalkOutcome {
    pub(crate) seq: u64,
    pub(crate) generation: u64,
    pub(crate) request_id: u64,
    pub(crate) failed: bool,
    pub(crate) x: i32,
    pub(crate) z: i32,
    pub(crate) level: i32,
    pub(crate) radius: i32,
    pub(crate) allow_teleports: bool,
    /// The settled route end is frozen `'blocked'`.
    pub(crate) blocked: bool,
}

/// One navigator-named gate short of a failed walk: the `MissingReq::Carry`
/// row [`find_missing_item_reqs`] reported for that `NoPath`. It carries only
/// what the diagnosis itself named — the display name is resolved at pack time
/// from the host obj table, and a missing one never drops the row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct MissingCarry {
    pub(crate) id: i32,
    pub(crate) count: i32,
}

/// Per-uid nav state: the whole-world traveller plus the route it is
/// following. `ctx.walk` stores the route (found off-pump over the shared
/// [`NavWorld`]); the slot pump polls [`Traveller::follow`] with a clone of
/// it one step per player-info tick. `route` being set is the "armed"
/// gate the walk hook and the busy flag read. A pending BankBudget
/// session freezes follow until its steps finish.
#[derive(Default)]
pub(crate) struct NavBot {
    pub(crate) route_generation: u64,
    pub(crate) map_route_generation: u64,
    pub(crate) route_worker: Option<Arc<()>>,
    pub(crate) pending_route: Option<ScriptRouteRequest>,
    /// Native ownership survives worker handoff, but not action revocation.
    pub(crate) native_walk: Option<script::native::HostAuthority>,
    pub(crate) native_receipt_seq: u64,
    pub(crate) native_walk_failure: Option<(u64, script::native::WalkEnd)>,
    /// A native walk's terminal that no published walk outcome carries: a
    /// host refusal, or a Pause / displacement / abort that ended its follow.
    /// Delivered once to its still-live owner by the next observation.
    pub(crate) native_end: Option<(script::native::HostAuthority, script::native::WalkEnd)>,
    /// Evidence retained with the published route, not a newer pending retarget.
    pub(crate) route_quest_evidence: Option<nav::quest_gates::QuestEvidence>,
    /// Dest, radius, allow_teleports, allow_wilderness, allow_bank_fetch.
    pub(crate) requested_route: Option<(WorldTile, i32, bool, bool, bool)>,
    /// The avoid rectangles `requested_route` was queued with, so a
    /// reconnect carries the same walk ([`hold_script_nav`]).
    pub(crate) requested_avoid: Vec<AvoidRect>,
    pub(crate) traveller: Traveller,
    pub(crate) route: Option<Route>,
    /// The walk request id `publish_route` installed `route` for. A retarget
    /// moves `walk_request_id` / `requested_route` to the new walk while the
    /// old route is still followed; only a route end whose owner is the
    /// armed walk may settle that walk's wait.
    pub(crate) route_request_id: u64,
    pub(crate) bank_fetch: Option<PendingBankFetch>,
    /// Pumps spent on the current non-Walk BankBudget step. Reset when
    /// the front step pops or the session clears.
    pub(crate) bank_fetch_attempts: u32,
    /// Snapshot identity at the last send of the current step, so a
    /// still-in-flight Deposit/Withdraw/Wear/Close/Open is not re-sent.
    pub(crate) bank_fetch_flight: Option<BankFetchFlight>,
    /// The clue duel partner this slot's accept gate validated on the offer
    /// screen: its confirm accepts only that partner, in this session.
    pub(crate) duel_offer_partner: Option<String>,
    /// Isolate-allocated walk request id for the armed / in-flight find.
    /// Distinct from `route_generation`, which remains the worker / retained-route token.
    pub(crate) walk_request_id: u64,
    /// Unpublished current-wait refusal id, distinct from `walk_request_id`.
    /// Older armed-route NoPath / mid-follow terminals must not overwrite this
    /// published outcome until a snapshot copies it.
    pub(crate) walk_live_refusal_id: u64,
    /// Last packed-walk `allow_teleports` opt-in (`Traversal.teleportsEnabled`).
    pub(crate) allow_teleports: bool,
    /// Bounded published walk outcome. Seq `0` means never published.
    pub(crate) walk_outcome_seq: u64,
    pub(crate) walk_outcome_generation: u64,
    pub(crate) walk_outcome_request_id: u64,
    pub(crate) walk_outcome_failed: bool,
    pub(crate) walk_outcome_x: i32,
    pub(crate) walk_outcome_z: i32,
    pub(crate) walk_outcome_level: i32,
    pub(crate) walk_outcome_radius: i32,
    pub(crate) walk_outcome_allow_teleports: bool,
    /// The published route end is frozen `'blocked'`
    /// ([`nav::traveller::HopFailure::EndBlocked`]).
    pub(crate) walk_outcome_blocked: bool,
    /// The navigator-named gate shorts of that same published outcome: set
    /// only where a `NoPath` was diagnosed, and cleared wherever the outcome
    /// is, so a list can never outlive the failure it belongs to.
    pub(crate) walk_missing_carry: Vec<MissingCarry>,
    pub(crate) inspect: route_inspect::InspectNav,
    pub(crate) bank_pick: super::bank::BankPickState,
    /// The game requests this slot's script sent, as dispatched here. Host
    /// data the catalog hunt watch reads; not an isolate wire.
    pub(crate) acts: crate::catalog_core::ScriptActLedger,
    /// The script walk a reconnect or an operator Pause interrupted, re-sent
    /// on the first dispatch after the relog or Resume ([`hold_script_nav`],
    /// [`take_carried_walk`]).
    pub(crate) carried_walk: Option<CarriedWalk>,
}

/// The snapshot facts a BankBudget send latched, so the next pump does
/// not re-send until the bank or pack actually changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct BankFetchFlight {
    pub(crate) bank_gen: u64,
    pub(crate) bank_com: i32,
    pub(crate) inv: Vec<(i32, i32)>,
    pub(crate) worn: Vec<i32>,
}

/// A script walk held across a reconnect or an operator Pause.
pub(crate) struct CarriedWalk {
    /// The script run it belongs to (`SlotScript::runtime_generation`): a
    /// Stop, Start or watchdog restart since the reconnect drops it.
    runtime_generation: u64,
    request: script::shim::InteractReq,
}

/// The shared arm for queued script walks and host bank-stand walks.
/// Routing stays off-pump; the slot pump owns follow and dispatch.
#[derive(Clone)]
pub(crate) struct ScriptWalkArm {
    pub(crate) here: Option<(i32, i32, i32)>,
    pub(crate) world: Option<Arc<NavWorld>>,
    pub(crate) navs: Arc<Mutex<HashMap<String, NavBot>>>,
    pub(crate) name: String,
    /// The slot's gating facts at arm time (from its live snapshot);
    /// `None` when no player is decoded — the worker then routes with
    /// the fail-closed empty [`WorldState`].
    pub(crate) state: Option<WorldState>,
    /// Open bank rows (obj id, count) at arm time — empty when closed.
    pub(crate) bank: Vec<(i32, i32)>,
}

fn walk_arm_outcome_tag(outcome: &RouteOutcome) -> &'static str {
    match outcome {
        RouteOutcome::Routed(_) => "Routed",
        RouteOutcome::BankSession { .. } => "BankSession",
        RouteOutcome::NoPath => "NoPath",
    }
}

fn log_walk_arm(name: &str, build: impl FnOnce() -> String) {
    api::host_log!(
        api::hostlog::Category::NavTrace,
        api::hostlog::Level::Debug,
        slot = name,
        "walk-arm {}",
        build()
    );
}

fn walk_arm_worker_slot() -> String {
    thread::current()
        .name()
        .and_then(|n| n.strip_prefix("nav-find-").map(str::to_string))
        .unwrap_or_else(|| "?".to_string())
}

pub(super) fn log_walk_arm_bot(build: impl FnOnce() -> String) {
    api::host_log!(
        api::hostlog::Category::NavTrace,
        api::hostlog::Level::Debug,
        "walk-arm {}",
        build()
    );
}

impl ScriptWalkArm {
    pub(crate) fn queue_native_route(
        mut self,
        snapshot: &GameSnapshot,
        request: script::native::WalkRequest,
        authority: script::native::HostAuthority,
    ) -> bool {
        if !authority.live() {
            return false;
        }
        if request.required_after.run != authority.run() {
            self.refuse_native(authority);
            return false;
        }
        if let (Some(provider), Some(family)) = (
            request.evidence,
            self.world
                .as_ref()
                .and_then(|world| world.graph.quest_family),
        ) {
            self.state
                .get_or_insert_with(Default::default)
                .quest_evidence = Some(nav::quest_gates::QuestEvidence::new(
                provider,
                family,
                request.required_after,
            ));
        }
        self.queue_route_impl(
            request.target.x,
            request.target.z,
            request.target.level,
            FindOptions {
                allow_teleports: request.options.allow_teleports,
                allow_wilderness: request.options.allow_wilderness,
                allow_bank_fetch: request.options.allow_bank_fetch,
                ..FindOptions::default()
            },
            i32::from(request.radius),
            true,
            authority.request_id().get(),
            Some(snapshot),
            RouteCompletion::default(),
            Vec::new(),
            Some(authority),
        )
    }

    /// Explicit WalkNear, including radius 0: an armed or in-flight route is
    /// replaced through the existing generation / pending-route coalescing path.
    /// A latched bank-fetch session still refuses.
    #[cfg(test)]
    pub(crate) fn route_with_radius(
        &self,
        x: i32,
        z: i32,
        level: i32,
        opts: FindOptions,
        radius: i32,
    ) -> bool {
        self.queue_route(x, z, level, opts, radius, true, 0)
    }
    pub(crate) fn publish_refusal(
        &self,
        to: WorldTile,
        radius: i32,
        allow_teleports: bool,
        request_id: u64,
    ) {
        let mut navs = self.navs.lock().unwrap();
        let bot = navs.entry(self.name.clone()).or_default();
        // Do not bump route_generation or clear a retained route / bank-fetch.
        bot.note_failure(
            bot.route_generation,
            request_id,
            to,
            radius,
            allow_teleports,
        );
    }

    /// A native walk the host will not arm gets its typed `Refused` terminal.
    /// It never publishes through the Load walk outcome: that refusal guard
    /// is released only by an isolate snapshot a compiled slot never posts.
    fn refuse_native(&self, authority: script::native::HostAuthority) {
        let mut navs = self.navs.lock().unwrap();
        navs.entry(self.name.clone())
            .or_default()
            .refuse_native(authority);
    }

    fn refuse(
        &self,
        to: WorldTile,
        radius: i32,
        allow_teleports: bool,
        request_id: u64,
        native: Option<&script::native::HostAuthority>,
    ) {
        match native {
            Some(authority) => self.refuse_native(authority.clone()),
            None => self.publish_refusal(to, radius, allow_teleports, request_id),
        }
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)] // route queue packs dest/options/request id fields
    pub(crate) fn queue_route(
        &self,
        x: i32,
        z: i32,
        level: i32,
        opts: FindOptions,
        radius: i32,
        retarget: bool,
        request_id: u64,
    ) -> bool {
        self.queue_route_impl(
            x,
            z,
            level,
            opts,
            radius,
            retarget,
            request_id,
            None,
            RouteCompletion::default(),
            Vec::new(),
            None,
        )
    }

    /// Queue one walk toward `(x, z, level)` with `opts`, routing off-pump
    /// on a short-lived worker (`find_with` over the shared [`NavWorld`]).
    /// When `allow_bank_fetch` is on and the strict find fails only on
    /// missing item/worn reqs, latches a [`PendingBankFetch`] session and
    /// the post-session route. Refuses synchronously only when there is
    /// no player tile, no nav world, or a route/session already queued;
    /// the worker stores the outcome on the uid's nav bot. Returns whether
    /// the worker was spawned — not whether a path exists.
    #[allow(clippy::too_many_arguments)] // plus the borrowed arm-time scene
    pub(crate) fn queue_route_in_snapshot(
        &self,
        snapshot: &GameSnapshot,
        x: i32,
        z: i32,
        level: i32,
        opts: FindOptions,
        radius: i32,
        retarget: bool,
        request_id: u64,
    ) -> bool {
        self.queue_route_impl(
            x,
            z,
            level,
            opts,
            radius,
            retarget,
            request_id,
            Some(snapshot),
            RouteCompletion::default(),
            Vec::new(),
            None,
        )
    }

    /// Queue a walk that keeps out of `avoid` (frozen `WalkOptions.avoidZones`).
    #[allow(clippy::too_many_arguments)] // route queue plus the walk's avoid rects
    pub(crate) fn queue_route_avoiding(
        &self,
        x: i32,
        z: i32,
        level: i32,
        opts: FindOptions,
        request_id: u64,
        avoid: Vec<AvoidRect>,
    ) -> bool {
        self.queue_route_impl(
            x,
            z,
            level,
            opts,
            0,
            false,
            request_id,
            None,
            RouteCompletion::default(),
            avoid,
            None,
        )
    }

    /// [`Self::queue_route_in_snapshot`] for a walk that keeps out of `avoid`.
    #[allow(clippy::too_many_arguments)] // plus the borrowed arm-time scene and avoid rects
    pub(crate) fn queue_route_in_snapshot_avoiding(
        &self,
        snapshot: &GameSnapshot,
        x: i32,
        z: i32,
        level: i32,
        opts: FindOptions,
        radius: i32,
        request_id: u64,
        avoid: Vec<AvoidRect>,
    ) -> bool {
        self.queue_route_impl(
            x,
            z,
            level,
            opts,
            radius,
            true,
            request_id,
            Some(snapshot),
            RouteCompletion::default(),
            avoid,
            None,
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)] // deterministic worker seam for route tests
    pub(crate) fn queue_route_in_snapshot_synced(
        &self,
        snapshot: &GameSnapshot,
        x: i32,
        z: i32,
        level: i32,
        opts: FindOptions,
        radius: i32,
        retarget: bool,
        request_id: u64,
    ) -> Option<std::sync::mpsc::Receiver<()>> {
        let (completion, receiver) = RouteCompletion::channel();
        self.queue_route_impl(
            x,
            z,
            level,
            opts,
            radius,
            retarget,
            request_id,
            Some(snapshot),
            completion,
            Vec::new(),
            None,
        )
        .then_some(receiver)
    }

    #[allow(clippy::too_many_arguments)] // admission mirrors the wire request identity
    fn gate_route(
        &self,
        bot: &mut NavBot,
        to: WorldTile,
        radius: i32,
        key: (WorldTile, i32, bool, bool, bool),
        retarget: bool,
        request_id: u64,
        native: Option<&script::native::HostAuthority>,
    ) -> Option<bool> {
        if native.is_some() && bot.native_walk.as_ref().is_some_and(|owner| !owner.live()) {
            // The owner cancelled or replaced its previous walk: that follow
            // (and any bank session it latched) is abandoned, not in flight.
            super::script_walk::abort_walk_on_bot(bot);
        }
        let refuse = |bot: &mut NavBot| match native {
            Some(owner) => bot.refuse_native(owner.clone()),
            None => bot.note_failure(bot.route_generation, request_id, to, radius, key.2),
        };
        if bot.bank_fetch.is_some()
            || (!retarget && (bot.route.is_some() || bot.route_worker.is_some()))
        {
            log_walk_arm(&self.name, || {
                format!(
                    "queue_route refused-in-flight dest={to:?} r={radius} request_id={request_id} \
                     bank_fetch={} retarget={retarget} route={} worker={}",
                    bot.bank_fetch.is_some(),
                    bot.route.is_some(),
                    bot.route_worker.is_some()
                )
            });
            refuse(bot);
            return Some(false);
        }
        if bot.requested_route == Some(key)
            && (bot.route_worker.is_some() || bot.route.is_some() || bot.pending_route.is_some())
        {
            // Same-id retransmission and legacy request_id 0 keep the
            // in-flight find. A later distinct nonzero wait is refused
            // without restarting search or reassigning the armed id.
            if request_id != 0 && request_id != bot.walk_request_id {
                log_walk_arm(&self.name, || {
                    format!(
                        "queue_route coalesced-refused-distinct-id dest={to:?} r={radius} \
                         request_id={request_id} armed_id={}",
                        bot.walk_request_id
                    )
                });
                refuse(bot);
                return Some(false);
            }
            log_walk_arm(&self.name, || {
                format!(
                    "queue_route coalesced dest={to:?} r={radius} request_id={request_id} \
                     generation={}",
                    bot.route_generation
                )
            });
            return Some(true);
        }
        None
    }

    #[allow(clippy::too_many_arguments)] // queue state plus borrowed arm-time scene
    fn queue_route_impl(
        &self,
        x: i32,
        z: i32,
        level: i32,
        mut opts: FindOptions,
        radius: i32,
        retarget: bool,
        request_id: u64,
        snapshot: Option<&GameSnapshot>,
        completion: RouteCompletion,
        avoid: Vec<AvoidRect>,
        authority: Option<script::native::HostAuthority>,
    ) -> bool {
        if authority.as_ref().is_some_and(|owner| !owner.live()) {
            return false;
        }
        let to = WorldTile { x, z, level };
        let Some((hx, hz, hl)) = self.here else {
            log_walk_arm(&self.name, || {
                format!(
                    "queue_route refused-no-here dest={to:?} r={radius} request_id={request_id}"
                )
            });
            self.refuse(
                to,
                radius,
                opts.allow_teleports,
                request_id,
                authority.as_ref(),
            );
            return false;
        };
        let Some(world) = self.world.as_ref() else {
            log_walk_arm(&self.name, || {
                format!(
                    "queue_route refused-no-world dest={to:?} r={radius} request_id={request_id}"
                )
            });
            self.refuse(
                to,
                radius,
                opts.allow_teleports,
                request_id,
                authority.as_ref(),
            );
            return false;
        };
        let from = WorldTile {
            x: hx,
            z: hz,
            level: hl,
        };
        let key = (
            to,
            radius,
            opts.allow_teleports,
            opts.allow_wilderness,
            opts.allow_bank_fetch,
        );
        {
            let mut navs = self.navs.lock().unwrap();
            let bot = navs.entry(self.name.clone()).or_default();
            if let Some(result) = self.gate_route(
                bot,
                to,
                radius,
                key,
                retarget,
                request_id,
                authority.as_ref(),
            ) {
                return result;
            }
        }
        // Resolve live-scene geometry only after this request passes the
        // refusal/coalescing gates, but outside the process-wide nav mutex.
        // The second gate below closes the race with another arming thread.
        let live_candidates = if radius > 0 {
            snapshot.and_then(|snapshot| solid_target_approach_tiles(snapshot, to))
        } else {
            None
        };
        let token = {
            let mut navs = self.navs.lock().unwrap();
            let bot = navs.entry(self.name.clone()).or_default();
            if let Some(result) = self.gate_route(
                bot,
                to,
                radius,
                key,
                retarget,
                request_id,
                authority.as_ref(),
            ) {
                return result;
            }
            if let Some(ess) = bot.traveller.essence() {
                opts.essence = Some(ess);
            }
            bot.route_generation = bot.route_generation.wrapping_add(1);
            bot.walk_request_id = request_id;
            // A host or legacy walk displacing a live native walk ends it
            // visibly; the same owner's replaced walk is already revoked.
            bot.end_native_walk(script::native::WalkEnd::Cancelled);
            bot.native_walk = authority;
            bot.native_walk_failure = None;
            bot.requested_route = Some(key);
            bot.requested_avoid.clone_from(&avoid);
            bot.pending_route = Some(ScriptRouteRequest {
                generation: bot.route_generation,
                request_id,
                world: Arc::clone(world),
                from,
                to,
                radius,
                opts,
                state: self.state.clone(),
                bank: self.bank.clone(),
                live_candidates,
                avoid,
                completion,
            });
            if bot.route_worker.is_some() {
                log_walk_arm(&self.name, || {
                    format!(
                        "queue_route handed-pending dest={to:?} r={radius} request_id={request_id} \
                         generation={} from={from:?}",
                        bot.route_generation
                    )
                });
                return true;
            }
            let token = Arc::new(());
            bot.route_worker = Some(Arc::clone(&token));
            token
        };
        let navs = Arc::clone(&self.navs);
        let name = self.name.clone();
        let worker_token = Arc::clone(&token);
        log_walk_arm(&self.name, || {
            format!(
                "queue_route spawned dest={to:?} r={radius} request_id={request_id} from={from:?} \
                 allow_teleports={} allow_wilderness={} allow_bank_fetch={}",
                opts.allow_teleports, opts.allow_wilderness, opts.allow_bank_fetch
            )
        });
        let spawned = thread::Builder::new()
            .name(format!("nav-find-{name}"))
            .spawn(move || loop {
                let (request, authority) = {
                    let mut all = navs.lock().unwrap();
                    let Some(bot) = all.get_mut(&name) else {
                        log_walk_arm(&name, || "worker exit bot-gone".to_string());
                        return;
                    };
                    if !bot
                        .route_worker
                        .as_ref()
                        .is_some_and(|t| Arc::ptr_eq(t, &worker_token))
                    {
                        log_walk_arm(&name, || {
                            "worker discard stale-token before dequeue".to_string()
                        });
                        return;
                    }
                    let Some(request) = bot.pending_route.take() else {
                        bot.route_worker = None;
                        log_walk_arm(&name, || "worker exit no-pending-route".to_string());
                        return;
                    };
                    (request, bot.native_walk.clone())
                };
                if authority.as_ref().is_some_and(|owner| !owner.live()) {
                    request.completion.signal();
                    continue;
                }
                let debug = debug_enabled();
                if debug {
                    log_walk_arm(&name, || {
                        format!(
                            "worker calculate begin generation={} request_id={} from={:?} dest={:?} r={}",
                            request.generation,
                            request.request_id,
                            request.from,
                            request.to,
                            request.radius
                        )
                    });
                }
                let started = debug.then(Instant::now);
                let outcome = request.calculate();
                if debug {
                    let elapsed_ms = started.unwrap().elapsed().as_millis();
                    log_walk_arm(&name, || {
                        format!(
                            "worker calculate end generation={} request_id={} elapsed_ms={elapsed_ms} \
                             outcome={}",
                            request.generation,
                            request.request_id,
                            walk_arm_outcome_tag(&outcome)
                        )
                    });
                }
                let native_failure = authority
                    .as_ref()
                    .and_then(|_| request.native_failure(&outcome));
                let mut all = navs.lock().unwrap();
                let Some(bot) = all.get_mut(&name) else {
                    log_walk_arm(&name, || "worker exit bot-gone after calculate".to_string());
                    return;
                };
                if !bot
                    .route_worker
                    .as_ref()
                    .is_some_and(|t| Arc::ptr_eq(t, &worker_token))
                {
                    log_walk_arm(&name, || {
                        format!(
                            "worker discard stale-token after calculate generation={} \
                             live_generation={}",
                            request.generation, bot.route_generation
                        )
                    });
                    return;
                }
                if authority.as_ref().is_some_and(|owner| !owner.live()) {
                    request.completion.signal();
                    continue;
                }
                let missing = request.missing_carry(&outcome);
                // Only a route this publish installs carries its evidence; a
                // newer request's NoPath leaves the followed route's own.
                if bot.route_generation == request.generation
                    && bot.walk_request_id == request.request_id
                    && !matches!(outcome, RouteOutcome::NoPath)
                {
                    bot.route_quest_evidence = request
                        .state
                        .as_ref()
                        .and_then(|state| state.quest_evidence.clone());
                }
                bot.publish_route(
                    request.generation,
                    request.request_id,
                    request.opts.allow_teleports,
                    outcome,
                );
                // The diagnosis lands under the same lock as the outcome it
                // belongs to, so the packer can never read a list beside
                // another failure. A discarded publish (stale generation)
                // leaves the list it did not name cleared.
                bot.note_missing_carry(request.generation, request.request_id, missing);
                if bot.walk_outcome_generation == request.generation
                    && bot.walk_outcome_request_id == request.request_id
                {
                    bot.native_walk_failure =
                        native_failure.map(|end| (request.request_id, end));
                }
                request.completion.signal();
            })
            .is_ok();
        if !spawned {
            log_walk_arm(&self.name, || {
                format!("queue_route spawn-failed dest={to:?} r={radius} request_id={request_id}")
            });
            if let Some(bot) = self.navs.lock().unwrap().get_mut(&self.name) {
                if bot
                    .route_worker
                    .as_ref()
                    .is_some_and(|t| Arc::ptr_eq(t, &token))
                {
                    if let Some((to, radius, allow_teleports, ..)) = bot.requested_route {
                        bot.note_failure(
                            bot.route_generation,
                            bot.walk_request_id,
                            to,
                            radius,
                            allow_teleports,
                        );
                    }
                    bot.route_worker = None;
                    bot.pending_route = None;
                    bot.requested_route = None;
                }
            }
        }
        spawned
    }
}

const MAX_LIVE_CANDIDATES: usize = 4;

/// Fixed-capacity arm-time goals. The destination tile has at most four
/// orthogonal arrival stands, so the worker receives only inline coordinates.
#[derive(Clone, Copy)]
pub(crate) struct LiveCandidates {
    tiles: [WorldTile; MAX_LIVE_CANDIDATES],
    len: usize,
}

impl LiveCandidates {
    fn empty(fill: WorldTile) -> Self {
        Self {
            tiles: [fill; MAX_LIVE_CANDIDATES],
            len: 0,
        }
    }

    fn push_unique(&mut self, tile: WorldTile) {
        if self.as_slice().contains(&tile) {
            return;
        }
        debug_assert!(self.len < MAX_LIVE_CANDIDATES);
        self.tiles[self.len] = tile;
        self.len += 1;
    }

    fn as_slice(&self) -> &[WorldTile] {
        &self.tiles[..self.len]
    }
}

/// For an in-scene solid destination, capture the target tile's own legal
/// cardinal sides. A modeled footprint loc additionally requires an operable
/// side, so a 2x2 loc cannot send a radius-1 walk to its far perimeter.
///
/// This deliberately does not filter by the current-scene flood: a shut door,
/// gate, scene edge, or transport can separate `from` from a legal stand while
/// the baked graph can still route there. `None` keeps the sequential
/// off-scene and standable-target policy; an empty or unroutable `Some`
/// falls back to the same radius goal set in
/// [`ScriptRouteRequest::calculate_solid`].
fn solid_target_approach_tiles(snapshot: &GameSnapshot, to: WorldTile) -> Option<LiveCandidates> {
    let scene = snapshot.scene();
    let query = api::query::SceneQuery::new(scene, None);
    if !query.contains(to) || query.walkable(to) {
        return None;
    }

    let mut cardinal = LiveCandidates::empty(to);
    for tile in query.arrival_stands(to) {
        cardinal.push_unique(tile);
    }

    let mut modeled = false;
    let mut operable_cardinal = LiveCandidates::empty(to);
    for loc in snapshot.locs().iter().filter(|loc| loc.tile == to) {
        let Some(operable) = query.operable_tiles(loc) else {
            continue;
        };
        modeled = true;
        for &tile in cardinal.as_slice() {
            if operable.contains(&tile) {
                operable_cardinal.push_unique(tile);
            }
        }
    }
    Some(if modeled { operable_cardinal } else { cardinal })
}

/// Candidate destinations for an explicit radius request. Exact walks retain
/// their old routing behavior. Bound enumeration to the loaded scene size.
pub(crate) fn approach_tiles(
    world: &NavWorld,
    from: WorldTile,
    to: WorldTile,
    radius: i32,
) -> Vec<WorldTile> {
    let r = radius.clamp(0, 104);
    let mut tiles = Vec::new();
    for dx in -r..=r {
        for dz in -r..=r {
            let t = WorldTile {
                x: to.x + dx,
                z: to.z + dz,
                level: to.level,
            };
            if world.collision.standable(t) {
                tiles.push(t);
            }
        }
    }
    if world.collision.standable(to) {
        let connected = nav::router::local_step_component(&world.collision, to, r);
        tiles.retain(|tile| connected.contains(tile));
    }
    tiles.sort_by_key(|t| {
        (
            (t.x - from.x).abs().max((t.z - from.z).abs()),
            (t.x - to.x).abs().max((t.z - to.z).abs()),
            t.x,
            t.z,
        )
    });
    tiles
}

pub(crate) struct ScriptRouteRequest {
    pub(crate) generation: u64,
    pub(crate) request_id: u64,
    pub(crate) world: Arc<NavWorld>,
    pub(crate) from: WorldTile,
    pub(crate) to: WorldTile,
    pub(crate) radius: i32,
    pub(crate) opts: FindOptions,
    pub(crate) state: Option<WorldState>,
    pub(crate) bank: Vec<(i32, i32)>,
    /// Arm-time target-cardinal goals derived from the borrowed live scene.
    /// `None` keeps the sequential radius policy; with `Some`, the radius
    /// goal set is the fallback when no stand routes.
    pub(crate) live_candidates: Option<LiveCandidates>,
    /// Frozen `WalkOptions.avoidZones` rectangles: every search of this
    /// walk keeps out of them.
    pub(crate) avoid: Vec<AvoidRect>,
    pub(crate) completion: RouteCompletion,
}
impl ScriptRouteRequest {
    fn native_failure(&self, outcome: &RouteOutcome) -> Option<script::native::WalkEnd> {
        if !matches!(outcome, RouteOutcome::NoPath) {
            return None;
        }
        let empty = WorldState::empty();
        let state = self.state.as_ref().unwrap_or(&empty);
        let mut targets = if self.radius <= 0 {
            vec![self.to]
        } else {
            approach_tiles(&self.world, self.from, self.to, self.radius)
        };
        if let Some(stands) = &self.live_candidates {
            targets.extend_from_slice(stands.as_slice());
        }
        Some(
            match nav::router::find_unresolved_quest_gates(
                &self.world.collision,
                &self.world.graph,
                self.from,
                &targets,
                self.opts,
                state,
                &self.avoid,
            ) {
                Ok(Some(gates)) => script::native::WalkEnd::NeedsEvidence(gates),
                Ok(None) => script::native::WalkEnd::Failed,
                Err(_) => script::native::WalkEnd::Refused,
            },
        )
    }

    /// The navigator-named gate shorts of a failed walk: the strict find's own
    /// diagnosis, re-run with only the `item_req`/`worn_req` gates ignored.
    /// A routed outcome names none, and neither does a failure the relaxed
    /// re-run cannot diagnose (budget, off-graph, a skill/quest/varp gate) —
    /// a `NoPath` is never read into a shopping list from its dest geometry.
    /// A `worn_req` any-of alternative is not a carry and is never posted as
    /// one.
    fn missing_carry(&self, outcome: &RouteOutcome) -> Vec<MissingCarry> {
        if !matches!(outcome, RouteOutcome::NoPath) {
            return Vec::new();
        }
        let empty = WorldState::empty();
        let state = self.state.as_ref().unwrap_or(&empty);
        let Some(missing) = find_missing_item_reqs_with_avoid(
            &self.world.collision,
            &self.world.graph,
            self.from,
            self.to,
            self.opts,
            state,
            &self.avoid,
        ) else {
            return Vec::new();
        };
        missing
            .into_iter()
            .filter_map(|req| match req {
                MissingReq::Carry { id, count } => Some(MissingCarry { id, count }),
                MissingReq::WearAny { .. } => None,
            })
            .collect()
    }

    /// A solid in-scene target: its arrival stands, then the radius goal set
    /// frozen PathFinder falls back to, in order: a strict route to a stand,
    /// a BankBudget session to a stand, a strict route to a radius tile, a
    /// session to a radius tile. The strict search for the stands also
    /// answers for the tiles as a search over them alone would, where that
    /// needs no more settles ([`find_first_with_fallback`]: the tiles keep
    /// their own proof and budget), and so does the search under what a
    /// session can fetch; a search over the tiles alone runs only when its
    /// stand search left them undecided. Of each such pair at most one stops
    /// at the unproven budget, so the arm spends at most two budget-limited
    /// searches, one strict and one fetchable, plus one fetchable search for
    /// each planned session whose post-state re-find refuses its goal (the
    /// trip deposits a carried obj the route still needs).
    fn calculate_solid(
        &self,
        stands: &[WorldTile],
        tiles: &[WorldTile],
        state: &WorldState,
    ) -> RouteOutcome {
        let debug = debug_enabled();
        let slot = walk_arm_worker_slot();
        let started = debug.then(Instant::now);
        let search = find_first_with_fallback_avoid(
            &self.world.collision,
            &self.world.graph,
            self.from,
            stands,
            tiles,
            self.opts,
            state,
            &self.avoid,
        );
        if debug {
            let elapsed_ms = started.unwrap().elapsed().as_millis();
            let scratch = search.scratch_capacities();
            log_walk_arm(&slot, || {
                format!(
                    "approach solid dest={:?} r={} stands={} tiles={} settled={} \
                     scratch={}/{}/{}/{} reverse={}/{} proof={:?} elapsed_ms={elapsed_ms} \
                     stand={} tile={:?}",
                    self.to,
                    self.radius,
                    stands.len(),
                    tiles.len(),
                    search.settled(),
                    scratch.distances,
                    scratch.predecessors,
                    scratch.settled,
                    scratch.heap,
                    scratch.reverse,
                    scratch.reverse_queue,
                    search.proof(),
                    search.route().is_ok(),
                    search.fallback().map(|tile| match tile {
                        FallbackRoute::Routed(route) => Ok(route.dest),
                        other => Err(other.clone()),
                    })
                )
            });
        }
        let strict_tile = match search.into_routes() {
            (Ok(route), _) => return RouteOutcome::Routed(route),
            (Err(_), tile) => tile,
        };

        let fetchable = fetchable_facts(&self.world, self.opts, state, &self.bank);
        let mut fetch_tiles = None;
        if let Some(fetchable) = &fetchable {
            // A strict route to a tile beats a session to one, so a routed
            // strict tile leaves the stands alone to search.
            let tiles = match strict_tile {
                Some(FallbackRoute::Routed(_)) => &[][..],
                _ => tiles,
            };
            match fetch_stand(
                &self.world,
                self.from,
                stands,
                tiles,
                self.opts,
                state,
                fetchable,
                &self.bank,
                &self.avoid,
            ) {
                StandFetch::Outcome(outcome) => return outcome,
                StandFetch::Tiles(known) => fetch_tiles = known,
            }
        }

        let strict_tile = match strict_tile {
            Some(FallbackRoute::Routed(route)) => Some(route),
            Some(FallbackRoute::Undecided) => find_first_with_avoid(
                &self.world.collision,
                &self.world.graph,
                self.from,
                tiles,
                self.opts,
                state,
                &self.avoid,
            )
            .into_route()
            .ok(),
            Some(FallbackRoute::Failed(_)) | None => None,
        };
        if let Some(route) = strict_tile {
            return RouteOutcome::Routed(route);
        }

        let outcome = match &fetchable {
            Some(fetchable) => fetch_tile(
                &self.world,
                self.from,
                tiles,
                fetch_tiles,
                self.opts,
                state,
                fetchable,
                &self.bank,
                &self.avoid,
            ),
            None => RouteOutcome::NoPath,
        };
        if debug {
            let elapsed_ms = started.unwrap().elapsed().as_millis();
            log_walk_arm(&slot, || {
                format!(
                    "approach solid end elapsed_ms={elapsed_ms} fetchable={} outcome={}",
                    fetchable.is_some(),
                    walk_arm_outcome_tag(&outcome)
                )
            });
        }
        outcome
    }

    fn calculate_in_order(
        &self,
        candidates: &[WorldTile],
        state: &WorldState,
        label: &str,
    ) -> RouteOutcome {
        let debug = debug_enabled();
        let slot = walk_arm_worker_slot();
        for (idx, &target) in candidates.iter().enumerate() {
            if debug {
                log_walk_arm(&slot, || {
                    format!(
                        "{label} begin idx={idx} target={target:?} generation={} request_id={}",
                        self.generation, self.request_id
                    )
                });
            }
            let started = debug.then(Instant::now);
            let outcome = route_or_bank_fetch(
                &self.world,
                self.from,
                target,
                self.opts,
                state,
                &self.bank,
                &self.avoid,
            );
            if debug {
                let elapsed_ms = started.unwrap().elapsed().as_millis();
                log_walk_arm(&slot, || {
                    format!(
                        "{label} end idx={idx} target={target:?} elapsed_ms={elapsed_ms} outcome={}",
                        walk_arm_outcome_tag(&outcome)
                    )
                });
            }
            if !matches!(outcome, RouteOutcome::NoPath) {
                return outcome;
            }
        }
        RouteOutcome::NoPath
    }

    pub(crate) fn calculate(&self) -> RouteOutcome {
        let empty = WorldState::empty();
        let state = self.state.as_ref().unwrap_or(&empty);
        if self.radius <= 0 {
            return route_or_bank_fetch(
                &self.world,
                self.from,
                self.to,
                self.opts,
                state,
                &self.bank,
                &self.avoid,
            );
        }

        // Frozen PathFinder falls back to the radius goal set after the
        // target-cardinal goals fail. Standable and off-scene destinations
        // keep the old sequential policy over the same set.
        let generated = approach_tiles(&self.world, self.from, self.to, self.radius);
        match self.live_candidates.as_ref() {
            Some(stands) => self.calculate_solid(stands.as_slice(), &generated, state),
            None => {
                if debug_enabled() {
                    let slot = walk_arm_worker_slot();
                    log_walk_arm(&slot, || {
                        format!(
                            "approach fallback dest={:?} r={} candidates={}",
                            self.to,
                            self.radius,
                            generated.len()
                        )
                    });
                }
                self.calculate_in_order(&generated, state, "approach fallback")
            }
        }
    }
}
impl NavBot {
    fn bump_walk_outcome_seq(&mut self) {
        self.walk_outcome_seq = self.walk_outcome_seq.wrapping_add(1);
        if self.walk_outcome_seq == 0 {
            self.walk_outcome_seq = 1;
        }
    }

    /// End the native walk this bot follows: its still-live owner gets `end`
    /// on the next observation; a revoked owner is owed nothing.
    pub(crate) fn end_native_walk(&mut self, end: script::native::WalkEnd) {
        if let Some(owner) = self.native_walk.take() {
            self.native_walk_failure = None;
            if owner.live() {
                self.native_end = Some((owner, end));
            }
        }
    }

    /// A native walk the host refused to arm: nothing was routed for it.
    pub(crate) fn refuse_native(&mut self, owner: script::native::HostAuthority) {
        if owner.live() {
            self.native_end = Some((owner, script::native::WalkEnd::Refused));
        }
    }

    /// The terminal a follow failure owes a native walk beyond `Failed`: a
    /// host send refusal is `Refused`, an unworkable leg is `Blocked`, and a
    /// crossing whose quest evidence was unproven at send time names the
    /// gates to acquire, or blocks honestly when evidence disproves them.
    pub(crate) fn note_native_follow_failure(&mut self, outcome: &nav::traveller::TravelOutcome) {
        use api::selected::Truth;
        let end = match outcome {
            nav::traveller::TravelOutcome::Refused { .. } => Some(script::native::WalkEnd::Refused),
            nav::traveller::TravelOutcome::Blocked { .. } => Some(script::native::WalkEnd::Blocked),
            nav::traveller::TravelOutcome::EvidenceUnproven {
                verdict: Truth::Unknown,
                unresolved,
                ..
            } if !unresolved.is_empty() => Some(script::native::WalkEnd::NeedsEvidence(
                Arc::clone(unresolved),
            )),
            nav::traveller::TravelOutcome::EvidenceUnproven { .. } => {
                Some(script::native::WalkEnd::Blocked)
            }
            _ => None,
        };
        self.native_walk_failure = end.map(|end| (self.walk_request_id, end));
    }

    /// Older armed-route results may publish only after the current wait
    /// refusal has been copied into a snapshot, or when they *are* that wait.
    pub(super) fn armed_outcome_may_publish(&self, request_id: u64) -> bool {
        self.walk_live_refusal_id == 0 || request_id == self.walk_live_refusal_id
    }

    /// A snapshot carrying outcome `posted_seq` reached the isolate. The
    /// live refusal guard is released only when that was the latest
    /// published outcome; a newer one stays guarded until it is posted.
    pub(crate) fn mark_walk_outcome_posted(&mut self, posted_seq: u64) {
        if self.walk_outcome_seq == posted_seq {
            self.walk_live_refusal_id = 0;
        }
    }

    pub(crate) fn note_failure(
        &mut self,
        generation: u64,
        request_id: u64,
        to: WorldTile,
        radius: i32,
        allow_teleports: bool,
    ) {
        // Legacy request id 0 never settles a wait. Do not replace a live
        // nonzero refusal with it before the isolate can observe the refusal.
        if request_id == 0 && self.walk_live_refusal_id != 0 {
            log_walk_arm_bot(|| {
                format!(
                    "note_failure skipped legacy-id dest={to:?} r={radius} request_id={request_id} \
                     live_refusal_id={}",
                    self.walk_live_refusal_id
                )
            });
            return;
        }
        log_walk_arm_bot(|| {
            format!(
                "note_failure generation={generation} walk_request_id={} request_id={request_id} \
                 dest={to:?} r={radius} allow_teleports={allow_teleports} seq={}",
                self.walk_request_id,
                self.walk_outcome_seq.wrapping_add(1)
            )
        });
        self.bump_walk_outcome_seq();
        self.walk_outcome_generation = generation;
        self.walk_outcome_request_id = request_id;
        self.walk_outcome_failed = true;
        self.walk_outcome_blocked = false;
        self.walk_outcome_x = to.x;
        self.walk_outcome_z = to.z;
        self.walk_outcome_level = to.level;
        self.walk_outcome_radius = radius;
        self.walk_outcome_allow_teleports = allow_teleports;
        // A failure raised here names no short: only the strict find's own
        // diagnosis attaches a shopping list, and it lands after this.
        self.walk_missing_carry.clear();
        if request_id != 0 && request_id != self.walk_request_id {
            self.walk_live_refusal_id = request_id;
        } else {
            self.walk_live_refusal_id = 0;
        }
    }

    /// The armed walk's route reached its end: publish a settled, not
    /// failed, outcome for `request_id` (frozen `'closest'`,
    /// `WalkExecutor.ts:316-325`, or `'blocked'` when the follow ended next
    /// to a refused last tile, `WalkExecutor.ts:1092-1094`). The matching
    /// isolate wait settles true.
    #[allow(clippy::too_many_arguments)] // the published outcome's fields
    pub(super) fn note_route_end(
        &mut self,
        generation: u64,
        request_id: u64,
        to: WorldTile,
        radius: i32,
        allow_teleports: bool,
        blocked: bool,
    ) {
        log_walk_arm_bot(|| {
            format!(
                "note_route_end generation={generation} request_id={request_id} dest={to:?} \
                 r={radius} seq={}",
                self.walk_outcome_seq.wrapping_add(1)
            )
        });
        self.bump_walk_outcome_seq();
        self.walk_outcome_generation = generation;
        self.walk_outcome_request_id = request_id;
        self.walk_outcome_failed = false;
        self.walk_outcome_blocked = blocked;
        self.walk_outcome_x = to.x;
        self.walk_outcome_z = to.z;
        self.walk_outcome_level = to.level;
        self.walk_outcome_radius = radius;
        self.walk_outcome_allow_teleports = allow_teleports;
        self.walk_missing_carry.clear();
        self.walk_live_refusal_id = 0;
    }

    pub(crate) fn clear_walk_outcome(&mut self) {
        self.bump_walk_outcome_seq();
        self.walk_outcome_failed = false;
        self.walk_outcome_blocked = false;
        self.walk_outcome_generation = self.route_generation;
        self.walk_outcome_request_id = 0;
        self.walk_outcome_x = 0;
        self.walk_outcome_z = 0;
        self.walk_outcome_level = 0;
        self.walk_outcome_radius = 0;
        self.walk_outcome_allow_teleports = false;
        self.walk_live_refusal_id = 0;
        // The family posts a present empty vector: a routed outcome names no
        // short, and the clear is never omitted.
        self.walk_missing_carry.clear();
    }

    /// Whether a script walk is armed, in flight or following.
    pub(crate) fn script_walk_armed(&self) -> bool {
        self.route.is_some()
            || self.bank_fetch.is_some()
            || self.route_worker.is_some()
            || self.pending_route.is_some()
            || self.requested_route.is_some()
            || self.walk_request_id != 0
    }

    /// Record the shopping list of the failure this attempt published. Only
    /// the outcome that is live when the diagnosis lands carries one: a
    /// superseded worker (stale generation) and a refusal `note_failure`
    /// declined to publish (legacy request id) both leave the list they never
    /// named cleared rather than attaching it to another failure's page.
    pub(crate) fn note_missing_carry(
        &mut self,
        generation: u64,
        request_id: u64,
        missing: Vec<MissingCarry>,
    ) {
        if self.route_generation != generation
            || !self.walk_outcome_failed
            || self.walk_outcome_generation != generation
            || self.walk_outcome_request_id != request_id
        {
            return;
        }
        self.walk_missing_carry = missing;
    }

    pub(crate) fn publish_route(
        &mut self,
        generation: u64,
        request_id: u64,
        allow_teleports: bool,
        outcome: RouteOutcome,
    ) {
        // Superseded workers and aborted/restarted runs keep a newer
        // route_generation. A late NoPath for the same dest must not publish.
        if self.route_generation != generation {
            log_walk_arm_bot(|| {
                format!(
                    "publish_route discard-stale-generation published={generation} \
                     live={} request_id={request_id} outcome={}",
                    self.route_generation,
                    walk_arm_outcome_tag(&outcome)
                )
            });
            return;
        }
        let (route, pending) = match outcome {
            RouteOutcome::Routed(route) => {
                log_walk_arm_bot(|| {
                    format!(
                        "publish_route Routed request_id={request_id} walk_request_id={} \
                         route_dest={:?}",
                        self.walk_request_id, route.dest
                    )
                });
                (route, None)
            }
            RouteOutcome::BankSession { pending, route } => {
                log_walk_arm_bot(|| {
                    format!(
                        "publish_route BankSession request_id={request_id} walk_request_id={} \
                         route_dest={:?} session_steps={} session_dest={:?}",
                        self.walk_request_id,
                        route.dest,
                        pending.steps.len(),
                        pending.dest
                    )
                });
                (route, Some(pending))
            }
            RouteOutcome::NoPath => {
                log_walk_arm_bot(|| {
                    format!(
                        "publish_route NoPath request_id={request_id} walk_request_id={} \
                         may_publish={}",
                        self.walk_request_id,
                        self.armed_outcome_may_publish(request_id)
                    )
                });
                if let Some((to, radius, allow_teleports, ..)) = self.requested_route {
                    if self.armed_outcome_may_publish(request_id) {
                        self.note_failure(generation, request_id, to, radius, allow_teleports);
                    }
                }
                // The retained route belongs to the previous request. A later
                // request for this failed destination must be allowed to retry.
                self.requested_route = None;
                return;
            }
        };
        self.traveller.clear();
        self.route = Some(route);
        self.map_route_generation = crate::walk_map::next_map_route_generation();
        self.route_request_id = request_id;
        self.bank_fetch = pending;
        self.allow_teleports = allow_teleports;
    }
}

/// End the session's navigation: every route, find, bank-fetch session,
/// inspect and bank pick of the slot, a walk a reconnect was carrying, and
/// the duel offer its accept gate validated.
pub(crate) fn reset_script_nav(navs: &Arc<Mutex<HashMap<String, NavBot>>>, name: &str) {
    if let Some(nav) = navs.lock().unwrap().get_mut(name) {
        end_route_follow(nav);
        nav.clear_walk_outcome();
        route_inspect::reset_inspect(nav);
        nav.bank_pick.reset();
        nav.carried_walk = None;
        nav.duel_offer_partner = None;
    }
}

/// The walk request's avoid entries for a carried re-dispatch.
fn carried_avoid(avoid: &[AvoidRect]) -> Vec<script::shim::InspectAvoidWire> {
    avoid
        .iter()
        .map(|rect| script::shim::InspectAvoidWire::Rect {
            min_x: rect.min_x,
            max_x: rect.max_x,
            min_z: rect.min_z,
            max_z: rect.max_z,
            level: rect.level,
        })
        .collect()
}

/// A reconnect the slot relogs through with its Load script's work held
/// (`SlotScript::reconnect_session_work`). The connection's route follow
/// ends as in [`reset_script_nav`]; what the held script still waits on
/// stays: the published walk outcome, and the route-inspect and bank-pick
/// workers with their results (pure computations, valid on any session).
///
/// With `carry` (the script run's `runtime_generation`), the walk the
/// script had armed is kept and re-armed on the relogged session under its
/// own request id ([`take_carried_walk`]), so the held walk wait settles on
/// it. Frozen resumes the paused `WalkExecutor.walkTo` itself
/// (`AutoRelogin.ts:159-163`), which repaths from wherever the player
/// stands when its follow sees a deviation or a stall
/// (`WalkExecutor.ts:916-918`, `1097-1099`). A typed native walk is never
/// carried as a legacy request: its follow ends `Cancelled` to its owner,
/// which revalidates and re-walks after Resume or the relog.
pub(crate) fn hold_script_nav(
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    carry: Option<u64>,
) {
    if let Some(nav) = navs.lock().unwrap().get_mut(name) {
        // A duel's screens belong to the dropped connection.
        nav.duel_offer_partner = None;
        let armed = nav.route.is_some()
            || nav.route_worker.is_some()
            || nav.pending_route.is_some()
            || nav.bank_fetch.is_some();
        let picking = nav.bank_pick.walking(nav.route_generation);
        if let (Some(runtime_generation), true, true) = (carry, picking, !armed) {
            // A nearest-bank walk still choosing its bank: re-ask for it.
            nav.carried_walk = Some(CarriedWalk {
                runtime_generation,
                request: script::shim::InteractReq::WalkNearestBank,
            });
        } else if let (Some(runtime_generation), true, Some(requested), None) =
            (carry, armed, nav.requested_route, &nav.native_walk)
        {
            let (to, radius, allow_teleports, allow_wilderness, allow_bank_fetch) = requested;
            let request_id = nav.walk_request_id;
            let request = if radius > 0 {
                script::shim::InteractReq::WalkNear {
                    x: to.x,
                    z: to.z,
                    level: to.level,
                    radius,
                    allow_teleports,
                    allow_wilderness,
                    allow_bank_fetch,
                    request_id,
                    avoid: carried_avoid(&nav.requested_avoid),
                }
            } else {
                script::shim::InteractReq::Walk {
                    x: to.x,
                    z: to.z,
                    level: to.level,
                    allow_teleports,
                    allow_wilderness,
                    allow_bank_fetch,
                    request_id,
                    avoid: carried_avoid(&nav.requested_avoid),
                }
            };
            nav.carried_walk = Some(CarriedWalk {
                runtime_generation,
                request,
            });
        }
        end_route_follow(nav);
    }
}

/// The walk a reconnect carried for the script run `runtime_generation`,
/// once: the caller dispatches it ahead of the run's own requests.
pub(crate) fn take_carried_walk(
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    runtime_generation: u64,
) -> Option<script::shim::InteractReq> {
    let carried = navs.lock().unwrap().get_mut(name)?.carried_walk.take()?;
    (carried.runtime_generation == runtime_generation).then_some(carried.request)
}

fn end_route_follow(nav: &mut NavBot) {
    nav.route_generation = nav.route_generation.wrapping_add(1);
    nav.route_worker = None;
    nav.pending_route = None;
    nav.requested_route = None;
    nav.traveller.clear();
    nav.route = None;
    nav.bank_fetch = None;
    nav.walk_request_id = 0;
    nav.route_quest_evidence = None;
    nav.end_native_walk(script::native::WalkEnd::Cancelled);
}
