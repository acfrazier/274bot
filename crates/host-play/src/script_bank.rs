use api::interact::Driver;
use api::named_banks::{BankPreferences, NamedBankFacts};
use api::quest_progress::EvidenceStamp;
use api::snapshot::{GameSnapshot, WorldTile};
use nav::bank_fetch::SAME_BANK;
use nav::router::{
    find_many_with_avoid_bounded, find_with, FindOptions, Route, BANK_TARGET_BUDGET,
};
use nav::world::NavWorld;
use nav::WorldState;
use script::isolate_fb::BankSelectionInput;
use script::native::HostAuthority;
use script::native_bank::{
    AccessKind as NativeAccessKind, BankPickReceipt as NativeBankPickReceipt,
    BankPickRequest as NativeBankPickRequest, BankStandAccess, PickKind as NativePickKind,
    SelectedBank,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const BANK_SELECTION_WINDOW: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickKind {
    NearShortcut = 1,
    Reachable = 2,
    AirFallback = 3,
    NoCandidate = 4,
}
struct NativeBankPickInput {
    request: NativeBankPickRequest,
    authority: HostAuthority,
    evidence: EvidenceStamp,
}

/// Selection completion is independent of movement. Both select-only and
/// pick-and-walk share one worker and one latest pending replacement.
#[derive(Default)]
pub(crate) struct BankPickState {
    generation: u64,
    last_id: u64,
    worker: Option<Arc<()>>,
    pending: Option<BankPickJob>,
    current: Option<ActivePick>,
    pub(crate) posted: BankSelectionInput,
    native_receipt: Option<(HostAuthority, NativeBankPickReceipt)>,
    #[cfg(test)]
    facts: Option<Arc<NamedBankFacts>>,
}

impl BankPickState {
    /// A `walk-nearest-bank` pick for the route of `route_generation` is
    /// still being resolved (its walk not armed yet).
    pub(crate) fn walking(&self, route_generation: u64) -> bool {
        self.current
            .as_ref()
            .is_some_and(|active| active.job.request.walk_generation == Some(route_generation))
    }

    pub(crate) fn take_native_receipt(&mut self) -> Option<(HostAuthority, NativeBankPickReceipt)> {
        self.native_receipt.take()
    }

    fn complete_native(&mut self, request: &BankPickRequest, index: Option<usize>, kind: PickKind) {
        let Some(native) = &request.native else {
            return;
        };
        if !native.authority.live() {
            return;
        }
        self.native_receipt = Some((
            native.authority.clone(),
            NativeBankPickReceipt {
                request_id: native.authority.request_id().get(),
                evidence: native.evidence,
                selected: request.selected(index, kind),
            },
        ));
    }
}

struct ActivePick {
    deadline: Option<Instant>,
    job: BankPickJob,
}

#[derive(Clone)]
struct BankPickJob {
    generation: u64,
    request: Arc<BankPickRequest>,
    route_only: bool,
}

impl BankPickState {
    pub(crate) fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.current = None;
        self.pending = None;
        self.last_id = 0;
        self.posted = BankSelectionInput::default();
        self.native_receipt = None;
    }

    pub(crate) fn poll(&mut self, now: Instant) -> BankSelectionInput {
        if self
            .current
            .as_ref()
            .is_some_and(|active| active.deadline.is_some_and(|deadline| now >= deadline))
        {
            let active = self.current.take().unwrap();
            self.generation = self.generation.wrapping_add(1);
            self.pending = None;
            if active.job.request.walk_generation.is_some() {
                // Discard the late ranking result. The same worker may make
                // one ordinary attempt to the captured air fallback afterward.
                let job = BankPickJob {
                    generation: self.generation,
                    route_only: true,
                    ..active.job
                };
                self.pending = Some(job.clone());
                self.current = Some(ActivePick {
                    deadline: None,
                    job,
                });
            } else if active.job.request.native.is_some() {
                self.complete_native(
                    &active.job.request,
                    active.job.request.first(),
                    PickKind::AirFallback,
                );
            } else {
                self.posted = active
                    .job
                    .result(active.job.request.first(), PickKind::AirFallback);
            }
        }
        self.posted
    }

    fn complete(&mut self, job: &BankPickJob, result: &PickResult, now: Instant) -> bool {
        self.poll(now);
        if self.generation != job.generation || self.current.is_none() {
            return false;
        }
        if job.request.walk_generation.is_none() {
            if job.request.native.is_some() {
                self.complete_native(&job.request, result.index, result.kind);
            } else {
                self.posted = job.result(result.index, result.kind);
            }
        }
        true
    }
}

#[derive(Clone)]
struct NativePickContext {
    authority: HostAuthority,
    evidence: EvidenceStamp,
}

struct BankPickRequest {
    request_id: u64,
    world: Arc<NavWorld>,
    facts: Arc<NamedBankFacts>,
    from: WorldTile,
    opts: FindOptions,
    state: WorldState,
    order: Vec<usize>,
    walk_generation: Option<u64>,
    native: Option<NativePickContext>,
    #[cfg(test)]
    test_gate: Option<Arc<tests::Gate>>,
}

struct PickResult {
    index: Option<usize>,
    kind: PickKind,
    route: Option<Route>,
}

impl BankPickJob {
    fn result(&self, index: Option<usize>, kind: PickKind) -> BankSelectionInput {
        BankSelectionInput {
            generation: self.generation,
            request_id: self.request.request_id,
            bank_index: index.map_or(-1, |i| i as i32),
            kind: kind as u8,
        }
    }
}

impl BankPickRequest {
    fn first(&self) -> Option<usize> {
        self.order.first().copied()
    }

    fn route_key(&self, index: usize) -> (WorldTile, i32, bool, bool, bool) {
        (
            self.facts.banks()[index].tile,
            0,
            self.opts.allow_teleports,
            self.opts.allow_wilderness,
            self.opts.allow_bank_fetch,
        )
    }

    fn route_target(&self, index: usize) -> WorldTile {
        let bank = self.facts.banks()[index];
        if self.native.is_some() {
            native_bank_access(&self.world, bank, self.from).map_or(bank.tile, |(tile, _)| tile)
        } else {
            bank.tile
        }
    }

    fn selected(&self, index: Option<usize>, kind: PickKind) -> SelectedBank {
        let Some(index) = index.filter(|&index| index < self.facts.banks().len()) else {
            return native_no_candidate();
        };
        let Ok(bank_index) = u16::try_from(index) else {
            return native_no_candidate();
        };
        if bank_index == u16::MAX {
            return native_no_candidate();
        }
        let bank = self.facts.banks()[index];
        let (access_tile, access) = match native_bank_access(&self.world, bank, self.from) {
            Some((tile, access)) => (tile, Some(Arc::new(access))),
            None => (bank.tile, None),
        };
        SelectedBank {
            bank_index,
            access_tile,
            kind: match kind {
                PickKind::NearShortcut => NativePickKind::NearShortcut,
                PickKind::Reachable => NativePickKind::Reachable,
                PickKind::AirFallback => NativePickKind::AirFallback,
                PickKind::NoCandidate => NativePickKind::NoCandidate,
            },
            access,
        }
    }
    fn fallback(&self) -> PickResult {
        let index = self.first();
        let mut route = None;
        if self.walk_generation.is_some() {
            if let Some(index) = index {
                #[cfg(test)]
                if let Some(gate) = &self.test_gate {
                    gate.single_route();
                }
                route = find_with(
                    &self.world.collision,
                    &self.world.graph,
                    self.from,
                    self.facts.banks()[index].tile,
                    self.opts,
                    &self.state,
                )
                .ok();
            }
        }
        PickResult {
            index,
            kind: PickKind::AirFallback,
            route,
        }
    }

    fn calculate(&self, route_only: bool) -> PickResult {
        if route_only {
            return self.fallback();
        }
        let routed: Vec<_> = self
            .order
            .iter()
            .copied()
            .filter(|&i| self.facts.banks()[i].routable)
            .collect();
        if routed.is_empty() {
            return self.fallback();
        }
        let targets: Vec<_> = routed
            .iter()
            .map(|&index| self.route_target(index))
            .collect();
        #[cfg(test)]
        if let Some(gate) = &self.test_gate {
            gate.bank_search();
        }
        let routes = find_many_with_avoid_bounded(
            &self.world.collision,
            &self.world.graph,
            self.from,
            &targets,
            self.opts,
            &self.state,
            &[],
            BANK_TARGET_BUDGET,
        );
        let mut best = None;
        let mut cost = f64::INFINITY;
        for (i, result) in routes.results().iter().enumerate() {
            if let Ok(result) = result {
                if result.ticks < cost {
                    cost = result.ticks;
                    best = Some(i);
                }
            }
        }
        if let Some(target) = best {
            let route = self.walk_generation.map(|_| {
                routes
                    .route(target)
                    .expect("a settled target has a reconstructible route")
            });
            return PickResult {
                index: Some(routed[target]),
                kind: PickKind::Reachable,
                route,
            };
        }
        // Do not retain the shared scratch while running the fallback search.
        drop(routes);
        self.fallback()
    }
}

fn native_no_candidate() -> SelectedBank {
    SelectedBank {
        bank_index: u16::MAX,
        access_tile: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        kind: NativePickKind::NoCandidate,
        access: None,
    }
}

fn bank_air_distance(from: WorldTile, to: WorldTile) -> i64 {
    let dx = i64::from(from.x) - i64::from(to.x);
    let dz = i64::from(from.z) - i64::from(to.z);
    dx * dx + dz * dz
}

fn near_bank(from: WorldTile, bank: WorldTile) -> bool {
    from.level == bank.level && from.x.abs_diff(bank.x).max(from.z.abs_diff(bank.z)) <= 4
}

fn native_bank_access(
    world: &NavWorld,
    bank: api::named_banks::NamedBank,
    from: WorldTile,
) -> Option<(WorldTile, BankStandAccess)> {
    if let Some(definition) = bank.definition {
        if let Some(operation) = definition.open_first.or(definition.object) {
            return Some((
                bank.tile,
                BankStandAccess {
                    bank,
                    stand_tile: definition.tile,
                    kind: NativeAccessKind::Booth,
                    // Object operations are resolved against the live loc row
                    // by BankMachine immediately before dispatch.
                    stand_op: 0,
                    name: Some(Arc::from(operation.name)),
                    choose: definition.choose.map(Arc::from),
                },
            ));
        }
    }
    let definition_npc = bank.definition.and_then(|definition| definition.npc);
    let stand = world
        .banks()
        .iter()
        .filter(|stand| {
            stand.tile.level == bank.tile.level
                && (stand.tile.x - bank.tile.x)
                    .abs()
                    .max((stand.tile.z - bank.tile.z).abs())
                    <= SAME_BANK
        })
        .min_by_key(|stand| {
            let kind_order = match (&stand.access, definition_npc) {
                (nav::pack::BankAccess::Npc { name, .. }, Some(wanted))
                    if name.eq_ignore_ascii_case(wanted.name) =>
                {
                    0
                }
                (nav::pack::BankAccess::Booth { .. }, Some(_)) => 1,
                (nav::pack::BankAccess::Npc { .. }, Some(_)) => 2,
                (nav::pack::BankAccess::Booth { .. }, None) => 0,
                (nav::pack::BankAccess::Npc { .. }, None) => 1,
            };
            (
                kind_order,
                bank_air_distance(bank.tile, stand.tile),
                stand.tile.x,
                stand.tile.z,
            )
        })?;
    let access_tile =
        nav::bank_fetch::bank_access_tiles(&world.collision, stand).min_by_key(|tile| {
            (
                i32::from(tile.level != from.level),
                bank_air_distance(from, *tile),
                tile.x,
                tile.z,
            )
        })?;
    let (kind, stand_op, name, choose) = match &stand.access {
        nav::pack::BankAccess::Booth { op } => (
            NativeAccessKind::Booth,
            *op,
            Some(Arc::from(stand.name.as_str())),
            None,
        ),
        nav::pack::BankAccess::Npc { name, op, choose } => (
            NativeAccessKind::Teller,
            *op,
            Some(Arc::from(name.as_str())),
            choose.as_deref().map(Arc::from).or_else(|| {
                bank.definition
                    .and_then(|definition| definition.choose.map(Arc::from))
            }),
        ),
    };
    Some((
        access_tile,
        BankStandAccess {
            bank,
            stand_tile: stand.tile,
            kind,
            stand_op,
            name,
            choose,
        },
    ))
}

pub(super) fn queue_native_bank_pick(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    world: &Option<Arc<NavWorld>>,
    state: Option<WorldState>,
    request: NativeBankPickRequest,
    authority: HostAuthority,
    evidence: EvidenceStamp,
) {
    let from = request.from;
    let preferences = request.preferences;
    let allow_wilderness = request.allow_wilderness;
    let request_id = authority.request_id().get();
    queue_bank(
        navs,
        name,
        world,
        state,
        from,
        FindOptions {
            allow_wilderness,
            ..FindOptions::default()
        },
        request_id,
        preferences,
        None,
        false,
        Some(NativeBankPickInput {
            request,
            authority,
            evidence,
        }),
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn queue_bank_pick(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    world: &Option<Arc<NavWorld>>,
    state: Option<WorldState>,
    from: WorldTile,
    allow_wilderness: bool,
    request_id: u64,
    preferences: BankPreferences,
    fishing_base: Option<i32>,
) {
    queue_bank(
        navs,
        name,
        world,
        state,
        from,
        FindOptions {
            allow_wilderness,
            ..Default::default()
        },
        request_id,
        preferences,
        fishing_base,
        false,
        None,
    );
}

/// The raw native verb keeps native option defaults. Compatibility callers
/// use select-only then their own walk, so they retain their explicit options.
pub(super) fn queue_bank_walk(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    world: &Option<Arc<NavWorld>>,
    state: Option<WorldState>,
    from: WorldTile,
    fishing_base: Option<i32>,
) -> bool {
    queue_bank(
        navs,
        name,
        world,
        state,
        from,
        FindOptions::default(),
        0,
        BankPreferences::default(),
        fishing_base,
        true,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn queue_bank(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    world: &Option<Arc<NavWorld>>,
    state: Option<WorldState>,
    from: WorldTile,
    mut opts: FindOptions,
    request_id: u64,
    preferences: BankPreferences,
    fishing_base: Option<i32>,
    walking: bool,
    native: Option<NativeBankPickInput>,
) -> bool {
    let now = Instant::now();
    let mut all = navs.lock().unwrap();
    let bot = all.entry(name.to_string()).or_default();
    if walking {
        if bot.bank_fetch.is_some() {
            return false;
        }
        if let Some(active) = &bot.bank_pick.current {
            match active.job.request.walk_generation {
                // The raw verb has no waiter to cancel. Do not orphan an
                // awaited select-only request by replacing it with this verb.
                None => return false,
                Some(generation) if generation == bot.route_generation => return true,
                Some(_) => {}
            }
        }
    } else if native.is_none() && (request_id == 0 || request_id <= bot.bank_pick.last_id) {
        return false;
    }
    let pick = &mut bot.bank_pick;
    pick.generation = pick.generation.wrapping_add(1);
    if !walking && native.is_none() {
        pick.last_id = request_id;
    }
    pick.pending = None;
    pick.current = None;
    pick.native_receipt = None;
    let facts = native
        .as_ref()
        .map(|native| Arc::clone(&native.request.facts))
        .or_else(|| {
            world
                .as_ref()
                .and_then(|world| world.named_bank_facts().cloned())
        });
    #[cfg(test)]
    let facts = if native.is_none() {
        pick.facts.clone().or(facts)
    } else {
        facts
    };
    let (Some(world), Some(facts)) = (world.as_ref(), facts) else {
        if let Some(native) = native.as_ref() {
            if native.authority.live() {
                pick.native_receipt = Some((
                    native.authority.clone(),
                    NativeBankPickReceipt {
                        request_id,
                        evidence: native.evidence,
                        selected: native_no_candidate(),
                    },
                ));
            }
        } else if !walking {
            pick.posted = BankSelectionInput {
                request_id,
                generation: pick.generation,
                kind: PickKind::NoCandidate as u8,
                ..Default::default()
            };
        }
        return false;
    };
    let state = state.unwrap_or_default();
    let from = native.as_ref().map_or(from, |native| native.request.from);
    let mut order: Vec<_> = if let Some(native) = native.as_ref() {
        native
            .request
            .eligible
            .iter()
            .map(|&index| usize::from(index))
            .filter(|&index| {
                index < facts.banks().len()
                    && native
                        .request
                        .explicit_bank
                        .is_none_or(|explicit| usize::from(explicit) == index)
            })
            .collect()
    } else {
        (0..facts.banks().len())
            .filter(|&i| {
                facts.banks()[i].eligible(
                    |id| if id == 10 { fishing_base } else { None },
                    |quest| state.quests.contains(quest),
                    preferences,
                )
            })
            .collect()
    };
    order.sort_by_key(|&i| bank_air_distance(from, facts.banks()[i].air_tile()));
    opts.essence = bot.traveller.essence();
    let first = order.first().copied();
    let walk_generation = if walking && first.is_some() {
        bot.route_generation = bot.route_generation.wrapping_add(1);
        bot.walk_request_id = 0;
        bot.pending_route = None;
        Some(bot.route_generation)
    } else {
        None
    };
    let native = native.map(|native| NativePickContext {
        authority: native.authority,
        evidence: native.evidence,
    });
    let request = Arc::new(BankPickRequest {
        request_id,
        world: Arc::clone(world),
        facts,
        from,
        opts,
        state,
        order,
        walk_generation,
        native,
        #[cfg(test)]
        test_gate: tests::capture_gate(),
    });
    let near = first.is_some_and(|index| near_bank(from, request.route_target(index)));
    let job = BankPickJob {
        generation: pick.generation,
        request,
        route_only: near,
    };
    if first.is_none() || (near && !walking) {
        if job.request.native.is_some() {
            pick.complete_native(
                &job.request,
                first,
                if near {
                    PickKind::NearShortcut
                } else {
                    PickKind::NoCandidate
                },
            );
        } else if !walking {
            pick.posted = job.result(
                first,
                if near {
                    PickKind::NearShortcut
                } else {
                    PickKind::NoCandidate
                },
            );
        }
        return first.is_some();
    }
    if walking {
        bot.requested_route = first.map(|index| job.request.route_key(index));
    }
    pick.current = Some(ActivePick {
        deadline: (!near).then_some(now + BANK_SELECTION_WINDOW),
        job: job.clone(),
    });
    pick.pending = Some(job);
    if pick.worker.is_some() {
        return true;
    }
    let token = Arc::new(());
    pick.worker = Some(Arc::clone(&token));
    drop(all);
    let worker_navs = Arc::clone(navs);
    let worker_name = name.to_string();
    let spawned = spawn_bank_worker(format!("bank-pick-{name}"), move || {
        #[cfg(test)]
        let mut last_gate: Option<Arc<tests::Gate>> = None;
        loop {
            let job = {
                let mut all = worker_navs.lock().unwrap();
                let Some(bot) = all.get_mut(&worker_name) else {
                    return;
                };
                let pick = &mut bot.bank_pick;
                if !pick
                    .worker
                    .as_ref()
                    .is_some_and(|live| Arc::ptr_eq(live, &token))
                {
                    return;
                }
                pick.poll(Instant::now());
                let job = match pick.pending.take() {
                    Some(job) => job,
                    None => {
                        pick.worker = None;
                        #[cfg(test)]
                        if let Some(gate) = &last_gate {
                            gate.idle();
                        }
                        return;
                    }
                };
                if job.request.walk_generation.is_some_and(|generation| {
                    generation != bot.route_generation
                        || bot.walk_request_id != 0
                        || bot.requested_route
                            != job
                                .request
                                .first()
                                .map(|index| job.request.route_key(index))
                }) {
                    if pick.generation == job.generation {
                        pick.current = None;
                    }
                    continue;
                }
                job
            };
            // Floods, reconstruction, fallback routing and scratch destruction
            // all happen without the slot/nav mutex.
            #[cfg(test)]
            if let Some(gate) = &job.request.test_gate {
                last_gate = Some(Arc::clone(gate));
                gate.enter();
            }
            let mut result = job.request.calculate(job.route_only);
            {
                let mut all = worker_navs.lock().unwrap();
                if let Some(bot) = all.get_mut(&worker_name) {
                    if bot.bank_pick.complete(&job, &result, Instant::now()) {
                        if let (Some(generation), Some(index), Some(first)) = (
                            job.request.walk_generation,
                            result.index,
                            job.request.first(),
                        ) {
                            if bot.route_generation == generation
                                && bot.walk_request_id == 0
                                && bot.requested_route == Some(job.request.route_key(first))
                            {
                                bot.requested_route = Some(job.request.route_key(index));
                                bot.publish_route(
                                    generation,
                                    0,
                                    job.request.opts.allow_teleports,
                                    result.route.take().map_or(
                                        super::RouteOutcome::NoPath,
                                        super::RouteOutcome::Routed,
                                    ),
                                );
                            }
                        }
                    }
                }
            }
            // An obsolete route is also dropped outside the mutex.
            drop(result);
            #[cfg(test)]
            if let Some(gate) = &job.request.test_gate {
                gate.finished();
            }
        }
    });
    if spawned.is_err() {
        if let Some(bot) = navs.lock().unwrap().get_mut(name) {
            let pick = &mut bot.bank_pick;
            pick.worker = None;
            pick.pending = None;
            if let Some(active) = pick.current.take() {
                if let Some(generation) = active.job.request.walk_generation {
                    bot.publish_route(
                        generation,
                        0,
                        active.job.request.opts.allow_teleports,
                        super::RouteOutcome::NoPath,
                    );
                } else if active.job.request.native.is_some() {
                    pick.complete_native(
                        &active.job.request,
                        active.job.request.first(),
                        PickKind::AirFallback,
                    );
                } else {
                    pick.posted = active
                        .job
                        .result(active.job.request.first(), PickKind::AirFallback);
                }
            }
        }
        return false;
    }
    true
}

fn spawn_bank_worker(
    name: String,
    work: impl FnOnce() + Send + 'static,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    #[cfg(test)]
    if tests::capture_gate()
        .is_some_and(|gate| gate.fail_spawn.load(std::sync::atomic::Ordering::Relaxed))
    {
        return Err(std::io::Error::other("injected bank worker spawn failure"));
    }
    std::thread::Builder::new().name(name).spawn(work)
}

pub(super) fn dispatch_observed_bank_op(
    driver: &mut dyn Driver,
    snapshot: &GameSnapshot,
    obj_names: Option<&api::obj_names::ObjNames>,
    inventory: Option<&[(i32, i32)]>,
    req: &script::shim::InteractReq,
) -> Option<script::slot::PendingBankOp> {
    use api::interact::{ActionSpec, Interactions, OpTarget, SendResult};
    use script::shim::InteractReq;
    use script::slot::{PendingBankOp, PendingBankOpKind};

    if snapshot.bank_component_id() < 0 || !snapshot.bank_loaded() {
        return None;
    }
    let generation = snapshot.bank_session_generation();
    let inventory_count = |id| {
        inventory.map_or(0, |items| {
            items
                .iter()
                .filter(|(item_id, _)| *item_id == id)
                .map(|(_, count)| *count)
                .sum()
        })
    };
    match req {
        InteractReq::Deposit { name } => {
            let item = snapshot.bank_side().iter().find(|item| {
                obj_names
                    .and_then(|names| names.name(item.def.id))
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(name))
            })?;
            let op = all_slot(&item.actions)?;
            let before_count = snapshot
                .bank_side()
                .iter()
                .filter(|row| row.def.id == item.def.id)
                .map(|row| row.count)
                .sum();
            matches!(
                Interactions::new(snapshot, driver)
                    .interact(OpTarget::Item(item), ActionSpec::Operation(op)),
                SendResult::Sent { .. }
            )
            .then(|| {
                PendingBankOp::new(
                    PendingBankOpKind::Deposit,
                    item.def.id,
                    before_count,
                    inventory_count(item.def.id),
                    generation,
                )
            })
        }
        InteractReq::Withdraw { name, action } => {
            let item = snapshot.bank().iter().find(|item| {
                obj_names
                    .and_then(|names| names.name(item.def.id))
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(name))
            })?;
            let op = action_slot(&item.actions, action)?;
            let kind = if open_only_withdraw_x(&item.actions, op) {
                PendingBankOpKind::WithdrawXAction
            } else {
                PendingBankOpKind::Withdraw
            };
            let before_count = snapshot
                .bank()
                .iter()
                .filter(|row| row.def.id == item.def.id)
                .map(|row| row.count)
                .sum();
            matches!(
                Interactions::new(snapshot, driver)
                    .interact(OpTarget::Item(item), ActionSpec::Operation(op)),
                SendResult::Sent { .. }
            )
            .then(|| {
                PendingBankOp::new(
                    kind,
                    item.def.id,
                    before_count,
                    inventory_count(item.def.id),
                    generation,
                )
            })
        }
        _ => None,
    }
}

/// Action-label lookup matching rs2b0t's `norm` (lowercase, whitespace and
/// `-`/`_` separators gone): `Withdraw All`, `Withdraw-All` and
/// `Withdraw  All` all resolve to the same slot.
pub(super) fn action_slot(actions: &[Option<String>], wanted: &str) -> Option<i32> {
    let wanted = norm_action(wanted);
    actions
        .iter()
        .position(|a| a.as_deref().map(norm_action).as_deref() == Some(wanted.as_str()))
        .map(|i| i as i32 + 1)
}

/// Whether the resolved bank op is the open-only `Withdraw X` label: it
/// opens the amount dialog and cannot move inventory on its own, so the
/// accepted send *is* the result (frozen `Bank.withdraw` returns
/// `Input.invButton` → `actions.menuAction`). Every other withdraw label
/// settles on the observed delta.
fn open_only_withdraw_x(actions: &[Option<String>], op: i32) -> bool {
    let index = match usize::try_from(op) {
        Ok(index) if index >= 1 => index - 1,
        _ => return false,
    };
    actions
        .get(index)
        .and_then(|label| label.as_deref())
        .is_some_and(|label| norm_action(label) == norm_action("Withdraw X"))
}

/// The bank-side op slot whose label contains "all" (Deposit All / the
/// deposit window's bulk op), 1-based.
pub(super) fn all_slot(actions: &[Option<String>]) -> Option<i32> {
    actions
        .iter()
        .position(|a| a.as_deref().is_some_and(|s| norm_action(s).contains("all")))
        .map(|i| i as i32 + 1)
}

/// Select one bounded fill operation in compatibility order: an exact
/// amount, then All only when the requested fill consumes current stock,
/// then the host-owned X continuation.
pub(crate) fn fill_withdraw_action(
    actions: &[Option<String>],
    count: i32,
    stock: i32,
) -> Option<(i32, bool)> {
    action_slot(actions, &format!("Withdraw {count}"))
        .map(|op| (op, false))
        .or_else(|| {
            (count == stock)
                .then(|| all_slot(actions).map(|op| (op, false)))
                .flatten()
        })
        .or_else(|| action_slot(actions, "Withdraw X").map(|op| (op, true)))
}

fn norm_action(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
        .collect()
}

/// Open the bank the player stands at, by the packed stands of the one
/// bank building ([`SAME_BANK`]) nearest `here`: a teller in the scene
/// first (its packed op), then that building's booths nearest `here`
/// first, so the booth the player stands at is the one used.
pub(crate) fn open_bank_at_here<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    here: Option<(i32, i32, i32)>,
    world: Option<&NavWorld>,
) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    use nav::pack::BankAccess;
    if let Some(world) = world {
        if !world.banks().is_empty() {
            let mut stands: Vec<&nav::pack::BankStand> = world.banks().iter().collect();
            if let Some((hx, hz, hl)) = here {
                stands.sort_by_key(|s| {
                    (
                        i32::from(s.tile.level != hl),
                        (s.tile.x - hx).abs().max((s.tile.z - hz).abs()),
                    )
                });
            }
            let nearest = stands[0];
            let cluster: Vec<&nav::pack::BankStand> = stands
                .iter()
                .copied()
                .filter(|s| {
                    s.tile.level == nearest.tile.level
                        && (s.tile.x - nearest.tile.x)
                            .abs()
                            .max((s.tile.z - nearest.tile.z).abs())
                            <= SAME_BANK
                })
                .collect();
            for stand in cluster
                .iter()
                .filter(|s| matches!(s.access, BankAccess::Npc { .. }))
            {
                if let BankAccess::Npc { name, op, choose } = &stand.access {
                    if open_npc_bank_at_here(
                        driver,
                        snapshot,
                        here,
                        stand.tile,
                        name,
                        *op,
                        choose.as_deref(),
                    ) {
                        return true;
                    }
                }
            }
            // `cluster` keeps the nearest-`here` order of `stands`.
            for stand in cluster
                .iter()
                .filter(|s| matches!(s.access, BankAccess::Booth { .. }))
            {
                if open_booth_stand(driver, snapshot, stand) {
                    return true;
                }
            }
            return false;
        }
    }
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    for loc in snapshot.locs() {
        if let Some(op) = action_slot(&loc.actions, "Use-quickly") {
            return matches!(
                ix.interact(OpTarget::Loc(loc), ActionSpec::Operation(op)),
                SendResult::Sent { .. }
            );
        }
    }
    false
}

fn open_booth_stand<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    stand: &nav::pack::BankStand,
) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    snapshot
        .locs()
        .iter()
        .find(|loc| {
            loc.tile.x == stand.tile.x
                && loc.tile.z == stand.tile.z
                && loc.tile.level == stand.tile.level
        })
        .and_then(|loc| {
            action_slot(&loc.actions, "Use-quickly").map(|slot| {
                matches!(
                    ix.interact(OpTarget::Loc(loc), ActionSpec::Operation(slot)),
                    SendResult::Sent { .. }
                )
            })
        })
        .unwrap_or(false)
}

fn open_npc_bank_at_here<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    here: Option<(i32, i32, i32)>,
    stand: WorldTile,
    name: &str,
    op: i32,
    choose: Option<&str>,
) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    if let Some(want) = choose.map(str::trim).filter(|s| !s.is_empty()) {
        let want = want.to_lowercase();
        if let Some(option) = snapshot
            .chat_options()
            .iter()
            .position(|row| !row.text.is_empty() && row.text.to_lowercase().contains(&want))
        {
            return matches!(
                ix.answer_choice(i32::try_from(option + 1).unwrap_or(i32::MAX)),
                SendResult::Sent { .. }
            );
        }
        if snapshot.chat_continue_component_id() >= 0 {
            return matches!(ix.continue_dialog(), SendResult::Sent { .. });
        }
    }
    let wanted = name.trim();
    let Some(npc) = snapshot
        .npcs()
        .iter()
        .filter(|npc| {
            npc.name
                .as_deref()
                .is_some_and(|got| got.trim().eq_ignore_ascii_case(wanted))
        })
        .filter(|npc| {
            let d_stand = (npc.tile.x - stand.x)
                .abs()
                .max((npc.tile.z - stand.z).abs())
                + i32::from(npc.tile.level != stand.level) * 10_000;
            let d_here = here
                .map(|(hx, hz, hl)| {
                    (npc.tile.x - hx).abs().max((npc.tile.z - hz).abs())
                        + i32::from(npc.tile.level != hl) * 10_000
                })
                .unwrap_or(i32::MAX);
            d_stand <= SAME_BANK || d_here <= 10
        })
        .min_by_key(|npc| {
            let d_stand = (npc.tile.x - stand.x)
                .abs()
                .max((npc.tile.z - stand.z).abs())
                + i32::from(npc.tile.level != stand.level) * 10_000;
            let d_here = here
                .map(|(hx, hz, hl)| {
                    (npc.tile.x - hx).abs().max((npc.tile.z - hz).abs())
                        + i32::from(npc.tile.level != hl) * 10_000
                })
                .unwrap_or(i32::MAX);
            (d_stand, d_here, npc.distance, npc.index)
        })
    else {
        return false;
    };
    let packed = (op >= 1).then_some(op).filter(|&slot| {
        npc.actions
            .get((slot as usize).saturating_sub(1))
            .and_then(|a| a.as_deref())
            .is_some_and(|label| !label.is_empty())
    });
    let slot = packed.or_else(|| action_slot(&npc.actions, "Bank"));
    let Some(slot) = slot else {
        return false;
    };
    matches!(
        ix.interact(OpTarget::Npc(npc), ActionSpec::Operation(slot)),
        SendResult::Sent { .. }
    )
}

pub(super) fn deposit_all_backpack<D: Driver>(driver: &mut D, snapshot: &GameSnapshot) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    let mut wrote = false;
    for item in snapshot.bank_side() {
        if let Some(op) = all_slot(&item.actions) {
            wrote |= matches!(
                ix.interact(OpTarget::Item(item), ActionSpec::Operation(op)),
                SendResult::Sent { .. }
            );
        }
    }
    wrote
}

pub(super) fn withdraw_id<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    id: i32,
    count: i32,
) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    let Some(item) = snapshot.bank().iter().find(|it| it.def.id == id) else {
        return false;
    };
    let label = match count {
        1 => "Withdraw 1",
        5 => "Withdraw 5",
        10 => "Withdraw 10",
        _ => "Withdraw All",
    };
    if let Some(op) = action_slot(&item.actions, label)
        .or_else(|| action_slot(&item.actions, "Withdraw 1"))
        .or_else(|| action_slot(&item.actions, "Withdraw All"))
    {
        return matches!(
            ix.interact(OpTarget::Item(item), ActionSpec::Operation(op)),
            SendResult::Sent { .. }
        );
    }
    false
}

#[cfg(test)]
#[path = "script_bank_tests.rs"]
mod tests;
