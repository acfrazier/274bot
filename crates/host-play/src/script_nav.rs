use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use api::snapshot::WorldTile;
use nav::router::{find_missing_item_reqs, FindOptions, MissingReq, Route};
use nav::traveller::Traveller;
use nav::world::NavWorld;
use nav::WorldState;

use super::{debug_enabled, route_inspect, route_or_bank_fetch, PendingBankFetch, RouteOutcome};

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
    /// Dest, radius, allow_teleports, allow_wilderness, allow_bank_fetch.
    pub(crate) requested_route: Option<(WorldTile, i32, bool, bool, bool)>,
    pub(crate) traveller: Traveller,
    pub(crate) route: Option<Route>,
    /// The walk request id `publish_route` installed `route` for. A retarget
    /// moves `walk_request_id` / `requested_route` to the new walk while the
    /// old route is still followed; only a route end whose owner is the
    /// armed walk may settle that walk's wait.
    pub(crate) route_request_id: u64,
    pub(crate) bank_fetch: Option<PendingBankFetch>,
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
    /// The navigator-named gate shorts of that same published outcome: set
    /// only where a `NoPath` was diagnosed, and cleared wherever the outcome
    /// is, so a list can never outlive the failure it belongs to.
    pub(crate) walk_missing_carry: Vec<MissingCarry>,
    pub(crate) inspect: route_inspect::InspectNav,
    pub(crate) bank_pick: super::bank::BankPickState,
    /// The game requests this slot's script sent, as dispatched here. Host
    /// data the catalog hunt watch reads; not an isolate wire.
    pub(crate) acts: crate::catalog_core::ScriptActLedger,
}

/// The shared script walk arm: both `ctx.walk` (default options) and
/// `ctx.walk_with` (explicit options) route through
/// [`ScriptWalkArm::route`]. Each observe clones the arm once per hook
/// (all fields are `Clone`), so the two `&mut` hooks never share a
/// mutable borrow.
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
    if debug_enabled() {
        eprintln!("[nav-walk-arm {name}] {}", build());
    }
}

fn walk_arm_worker_slot() -> String {
    thread::current()
        .name()
        .and_then(|n| n.strip_prefix("nav-find-").map(str::to_string))
        .unwrap_or_else(|| "?".to_string())
}

pub(super) fn log_walk_arm_bot(build: impl FnOnce() -> String) {
    if debug_enabled() {
        eprintln!("[nav-walk-arm] {}", build());
    }
}

impl ScriptWalkArm {
    /// Queue one walk toward `(x, z, level)` with `opts`, routing off-pump
    /// on a short-lived worker (`find_with` over the shared [`NavWorld`]).
    /// When `allow_bank_fetch` is on and the strict find fails only on
    /// missing item/worn reqs, latches a [`PendingBankFetch`] session and
    /// the post-session route. Refuses synchronously only when there is
    /// no player tile, no nav world, or a route/session already queued;
    /// the worker stores the outcome on the uid's nav bot. Returns whether
    /// the worker was spawned — not whether a path exists.
    pub(crate) fn route(&self, x: i32, z: i32, level: i32, opts: FindOptions) -> bool {
        self.queue_route(x, z, level, opts, 0, false, 0)
    }
    /// Explicit WalkNear, including radius 0. Unlike [`Self::route`], an
    /// armed or in-flight route is replaced through the existing generation /
    /// pending-route coalescing path. A latched bank-fetch session still refuses.
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
    fn publish_refusal(&self, to: WorldTile, radius: i32, allow_teleports: bool, request_id: u64) {
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

    #[allow(clippy::too_many_arguments)] // route queue packs dest/options/request id fields
    pub(crate) fn queue_route(
        &self,
        x: i32,
        z: i32,
        level: i32,
        mut opts: FindOptions,
        radius: i32,
        retarget: bool,
        request_id: u64,
    ) -> bool {
        let to = WorldTile { x, z, level };
        let Some((hx, hz, hl)) = self.here else {
            log_walk_arm(&self.name, || {
                format!(
                    "queue_route refused-no-here dest={to:?} r={radius} request_id={request_id}"
                )
            });
            self.publish_refusal(to, radius, opts.allow_teleports, request_id);
            return false;
        };
        let Some(world) = self.world.as_ref() else {
            log_walk_arm(&self.name, || {
                format!(
                    "queue_route refused-no-world dest={to:?} r={radius} request_id={request_id}"
                )
            });
            self.publish_refusal(to, radius, opts.allow_teleports, request_id);
            return false;
        };
        let from = WorldTile {
            x: hx,
            z: hz,
            level: hl,
        };
        let token = {
            let mut navs = self.navs.lock().unwrap();
            let bot = navs.entry(self.name.clone()).or_default();
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
                bot.note_failure(
                    bot.route_generation,
                    request_id,
                    to,
                    radius,
                    opts.allow_teleports,
                );
                return false;
            }
            let key = (
                to,
                radius,
                opts.allow_teleports,
                opts.allow_wilderness,
                opts.allow_bank_fetch,
            );
            if bot.requested_route == Some(key)
                && (bot.route_worker.is_some()
                    || bot.route.is_some()
                    || bot.pending_route.is_some())
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
                    bot.note_failure(
                        bot.route_generation,
                        request_id,
                        to,
                        radius,
                        opts.allow_teleports,
                    );
                    return false;
                }
                log_walk_arm(&self.name, || {
                    format!(
                        "queue_route coalesced dest={to:?} r={radius} request_id={request_id} \
                         generation={}",
                        bot.route_generation
                    )
                });
                return true;
            }
            if let Some(ess) = bot.traveller.essence() {
                opts.essence = Some(ess);
            }
            bot.route_generation = bot.route_generation.wrapping_add(1);
            bot.walk_request_id = request_id;
            bot.requested_route = Some(key);
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
                let request = {
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
                    request
                };
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
                let missing = request.missing_carry(&outcome);
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
}
impl ScriptRouteRequest {
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
        let Some(missing) = find_missing_item_reqs(
            &self.world.collision,
            &self.world.graph,
            self.from,
            self.to,
            self.opts,
            state,
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
            );
        }
        let candidates = approach_tiles(&self.world, self.from, self.to, self.radius);
        let debug = debug_enabled();
        let slot = walk_arm_worker_slot();
        if debug {
            log_walk_arm(&slot, || {
                format!(
                    "approach enumerate dest={:?} r={} candidates={}",
                    self.to,
                    self.radius,
                    candidates.len()
                )
            });
        }
        for (idx, target) in candidates.into_iter().enumerate() {
            if debug {
                log_walk_arm(&slot, || {
                    format!(
                        "approach begin idx={idx} target={target:?} generation={} request_id={}",
                        self.generation, self.request_id
                    )
                });
            }
            let started = debug.then(Instant::now);
            let outcome =
                route_or_bank_fetch(&self.world, self.from, target, self.opts, state, &self.bank);
            if debug {
                let elapsed_ms = started.unwrap().elapsed().as_millis();
                log_walk_arm(&slot, || {
                    format!(
                        "approach end idx={idx} target={target:?} elapsed_ms={elapsed_ms} \
                         outcome={}",
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
}
impl NavBot {
    fn bump_walk_outcome_seq(&mut self) {
        self.walk_outcome_seq = self.walk_outcome_seq.wrapping_add(1);
        if self.walk_outcome_seq == 0 {
            self.walk_outcome_seq = 1;
        }
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
    /// `WalkExecutor.ts:316-325`). The matching isolate wait settles true.
    pub(super) fn note_route_end(
        &mut self,
        generation: u64,
        request_id: u64,
        to: WorldTile,
        radius: i32,
        allow_teleports: bool,
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

pub(crate) fn reset_script_nav(navs: &Arc<Mutex<HashMap<String, NavBot>>>, name: &str) {
    if let Some(nav) = navs.lock().unwrap().get_mut(name) {
        nav.route_generation = nav.route_generation.wrapping_add(1);
        nav.route_worker = None;
        nav.pending_route = None;
        nav.requested_route = None;
        nav.traveller.clear();
        nav.route = None;
        nav.bank_fetch = None;
        nav.walk_request_id = 0;
        nav.clear_walk_outcome();
        route_inspect::reset_inspect(nav);
        nav.bank_pick.reset();
    }
}
