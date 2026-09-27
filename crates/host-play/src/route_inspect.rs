//! Inspect-specific off-pump job. One running calculation plus one latest
//! pending per profile. Distinct from walk: reset leaves `inspect.worker`,
//! Pause does not abort inspect, and results never arm Traveller.

use super::NavBot;
use api::obj_names::ObjNames;
use api::snapshot::WorldTile;
use client::config::Cache;
use nav::bank_fetch::{plan_bank_fetch, BankStep};
use nav::router::{
    find_missing_item_reqs_with_avoid, find_missing_item_reqs_with_avoid_bounded, find_with_avoid,
    find_with_avoid_bounded, AvoidRect, FindOptions, Leg, Route, RouteError,
};
use nav::transport::TransportKind;
use nav::world::NavWorld;
use nav::world_state::WorldState;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

const MAX_AVOID: usize = 16;
/// Isolate unsettled-waiter cap. Host published storage plus admission
/// equal this bound: RING(2)+HELD(1)=3. Registered overflow posts a
/// refuse identity; it does not accept a fourth terminal.
const RING: usize = 2;
const HELD: usize = 1;
const CAPACITY: usize = RING + HELD;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InspectHop {
    pub kind: String,
    pub loc_id: i32,
    pub loc_name: String,
    pub action: String,
    pub option: i32,
    pub from_x: i32,
    pub from_z: i32,
    pub from_level: i32,
    pub to_x: i32,
    pub to_z: i32,
    pub to_level: i32,
    pub ticks: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InspectTerminal {
    pub seq: u64,
    pub generation: u64,
    pub request_id: u64,
    pub ok: bool,
    pub reason: String,
    pub bank_planned: bool,
    pub ticks: f64,
    pub hops: Vec<InspectHop>,
}

impl InspectTerminal {
    fn refusal(request_id: u64, generation: u64, reason: &str) -> Self {
        Self {
            seq: 0,
            generation,
            request_id,
            ok: false,
            reason: reason.into(),
            bank_planned: false,
            ticks: 0.0,
            hops: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct InspectCapture {
    pub request_id: u64,
    pub generation: u64,
    pub world: Arc<NavWorld>,
    pub state: WorldState,
    pub bank: Vec<(i32, i32)>,
    pub from: WorldTile,
    pub to: WorldTile,
    pub opts: FindOptions,
    pub avoid: Vec<AvoidRect>,
    pub cache: Option<Arc<Cache>>,
    pub obj_names: Option<Arc<ObjNames>>,
    pub test_barrier: Option<Arc<InspectBarrier>>,
    /// Injectable node budgets for I1 fixtures. `None` uses production
    /// [`find_with_avoid`] / [`find_missing_item_reqs_with_avoid`].
    pub search_budget_strict: Option<usize>,
    pub search_budget_stand: Option<usize>,
    pub search_budget_post: Option<usize>,
}

#[derive(Default)]
pub(crate) struct InspectNav {
    pub generation: u64,
    pub worker: Option<Arc<()>>,
    pub pending: Option<InspectCapture>,
    pub running_id: u64,
    pub pending_id: u64,
    pub accepted_id: u64,
    pub replaced_id: u64,
    pub replaced_prev_id: u64,
    pub refused: [u64; 3],
    pub latest: Option<InspectTerminal>,
    pub prev: Option<InspectTerminal>,
    pub held: Option<InspectTerminal>,
    pub observed_seq: u64,
    pub executing: bool,
    pub live_world: Option<Arc<NavWorld>>,
}

#[derive(Clone, Default)]
pub(crate) struct PostedInspect {
    pub(super) latest: Option<InspectTerminal>,
    pub(super) prev: Option<InspectTerminal>,
    pub(super) running_id: u64,
    pub(super) pending_id: u64,
    pub(super) accepted_id: u64,
    pub(super) replaced_id: u64,
    pub(super) replaced_prev_id: u64,
    pub(super) refused_id: u64,
    pub(super) refused_id_2: u64,
    pub(super) refused_id_3: u64,
    pub(super) unobserved: u64,
}

impl InspectNav {
    /// Read-only copy of the slot inspect generation and, when present, the
    /// already-published latest terminal. Callers invoke this only for the
    /// active inspect case. `reset_inspect` bumps generation and clears
    /// terminals; a missing terminal must not be treated as generation 0.
    pub(crate) fn published_core_facts(&self) -> crate::catalog_core::RouteInspectPublished {
        match self.latest.as_ref() {
            Some(latest) => crate::catalog_core::RouteInspectPublished {
                live_generation: self.generation,
                has_terminal: true,
                seq: latest.seq,
                generation: latest.generation,
                request_id: latest.request_id,
                ok: latest.ok,
                reason: latest.reason.clone(),
                hop_loc_names: latest.hops.iter().map(|hop| hop.loc_name.clone()).collect(),
            },
            None => crate::catalog_core::RouteInspectPublished {
                live_generation: self.generation,
                has_terminal: false,
                ..Default::default()
            },
        }
    }

    fn next_seq(&self) -> u64 {
        let cur = self.latest.as_ref().map(|t| t.seq).unwrap_or(0);
        if cur == u64::MAX {
            1
        } else {
            cur + 1
        }
    }

    fn note_replaced(&mut self, id: u64) {
        if id == 0 {
            return;
        }
        self.replaced_prev_id = self.replaced_id;
        self.replaced_id = id;
    }

    fn slot_unobserved(term: Option<&InspectTerminal>, observed_seq: u64) -> bool {
        term.is_some_and(|t| t.seq > observed_seq)
    }

    fn unobserved_count(&self) -> usize {
        usize::from(Self::slot_unobserved(
            self.latest.as_ref(),
            self.observed_seq,
        )) + usize::from(Self::slot_unobserved(self.prev.as_ref(), self.observed_seq))
            + usize::from(self.held.is_some())
    }

    /// Count only unobserved retention plus the jobs that will still
    /// produce a terminal. Observed ring history is not capacity.
    fn can_admit(&self, replacing_pending: bool) -> bool {
        let reserved = usize::from(self.executing) + usize::from(replacing_pending) + 1;
        self.unobserved_count() + reserved <= CAPACITY
    }

    fn is_duplicate(&self, request_id: u64) -> bool {
        request_id != 0
            && (self.running_id == request_id
                || self
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.request_id == request_id))
    }

    fn note_refused(&mut self, id: u64) {
        if id == 0 || self.refused.contains(&id) {
            return;
        }
        if let Some(slot) = self.refused.iter_mut().find(|slot| **slot == 0) {
            *slot = id;
            return;
        }
        self.refused[0] = self.refused[1];
        self.refused[1] = self.refused[2];
        self.refused[2] = id;
    }

    /// Registered token that cannot reserve storage: identity-only refuse.
    /// Does not overwrite unobserved ring/held terminals. Snapshot-only 0
    /// has no waiter and leaves the previous published latest in place.
    fn refuse_unreserved(&mut self, request_id: u64) {
        self.note_refused(request_id);
    }

    fn max_ackable_seq(&self) -> u64 {
        self.latest
            .as_ref()
            .map(|t| t.seq)
            .unwrap_or(0)
            .max(self.prev.as_ref().map(|t| t.seq).unwrap_or(0))
    }

    fn publish(&mut self, mut term: InspectTerminal) {
        if term.seq == 0 {
            term.seq = self.next_seq();
        }
        if self.latest.is_none() {
            self.latest = Some(term);
            return;
        }
        if self.prev.is_none() {
            self.prev = self.latest.take();
            self.latest = Some(term);
            return;
        }
        if !self.can_wrap() {
            assert!(
                self.held.is_none(),
                "inspect publish overflow: accepted terminal with wrap blocked and held occupied"
            );
            self.held = Some(term);
            return;
        }
        self.prev = self.latest.take();
        self.latest = Some(term);
    }

    fn can_wrap(&self) -> bool {
        let Some(prev) = self.prev.as_ref() else {
            return true;
        };
        self.observed_seq >= prev.seq
    }

    pub(super) fn apply_ack(&mut self, seq: u64, generation: u64) {
        if generation != self.generation || seq == 0 {
            return;
        }
        if seq > self.max_ackable_seq() {
            return;
        }
        if seq > self.observed_seq {
            self.observed_seq = seq;
        }
        if self.held.is_some() && self.can_wrap() {
            if let Some(held) = self.held.take() {
                self.publish(held);
            }
        }
    }

    pub(super) fn posted(&self) -> PostedInspect {
        PostedInspect {
            latest: self.latest.clone(),
            prev: self.prev.clone(),
            running_id: self.running_id,
            pending_id: self.pending_id,
            accepted_id: self.accepted_id,
            replaced_id: self.replaced_id,
            replaced_prev_id: self.replaced_prev_id,
            refused_id: self.refused[0],
            refused_id_2: self.refused[1],
            refused_id_3: self.refused[2],
            unobserved: self.unobserved_count() as u64,
        }
    }

    pub(super) fn clear_published(&mut self) {
        self.latest = None;
        self.prev = None;
        self.held = None;
        self.running_id = 0;
        self.pending_id = 0;
        self.accepted_id = 0;
        self.replaced_id = 0;
        self.replaced_prev_id = 0;
        self.refused = [0; 3];
        self.observed_seq = 0;
    }
}

#[derive(Clone, Debug)]
pub(super) struct InspectRequest {
    pub from: WorldTile,
    pub to: WorldTile,
    pub allow_teleports: bool,
    pub allow_wilderness: bool,
    pub allow_bank_fetch: bool,
    pub avoid: Vec<AvoidRect>,
    pub request_id: u64,
    pub invalid_args: bool,
}

pub(super) fn validate_request(
    from: WorldTile,
    to: WorldTile,
    avoid: &[AvoidRect],
) -> Result<(), &'static str> {
    if !(0..=3).contains(&from.level) || !(0..=3).contains(&to.level) {
        return Err("invalid-args");
    }
    if avoid.len() > MAX_AVOID {
        return Err("invalid-args");
    }
    for rect in avoid {
        if rect.min_x > rect.max_x || rect.min_z > rect.max_z {
            return Err("invalid-args");
        }
        if let Some(level) = rect.level {
            if !(0..=3).contains(&level) {
                return Err("invalid-args");
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)] // inspect queue packs nav/world/state/request fields
pub(super) fn queue_inspect(
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    world: &Option<Arc<NavWorld>>,
    state: Option<WorldState>,
    bank: Vec<(i32, i32)>,
    cache: Option<Arc<Cache>>,
    obj_names: Option<Arc<ObjNames>>,
    req: InspectRequest,
) {
    let Some(world) = world.clone() else {
        let mut all = navs.lock().unwrap();
        let bot = all.entry(name.to_string()).or_default();
        if bot.inspect.is_duplicate(req.request_id) {
            return;
        }
        if !bot.inspect.can_admit(bot.inspect.pending.is_some()) {
            bot.inspect.refuse_unreserved(req.request_id);
            return;
        }
        if req.invalid_args || validate_request(req.from, req.to, &req.avoid).is_err() {
            refuse_locked(bot, req.request_id, "invalid-args");
            return;
        }
        refuse_locked(bot, req.request_id, "missing-graph");
        return;
    };
    let token = {
        let mut all = navs.lock().unwrap();
        let bot = all.entry(name.to_string()).or_default();
        if bot.inspect.is_duplicate(req.request_id) {
            return;
        }
        if !bot.inspect.can_admit(bot.inspect.pending.is_some()) {
            bot.inspect.refuse_unreserved(req.request_id);
            return;
        }
        if req.invalid_args || validate_request(req.from, req.to, &req.avoid).is_err() {
            refuse_locked(bot, req.request_id, "invalid-args");
            return;
        }
        let generation = bot.inspect.generation;
        if let Some(old) = bot.inspect.pending.take() {
            bot.inspect.note_replaced(old.request_id);
            bot.inspect.publish(InspectTerminal::refusal(
                old.request_id,
                old.generation,
                "stale",
            ));
        }
        let mut opts = FindOptions {
            allow_teleports: req.allow_teleports,
            allow_wilderness: req.allow_wilderness,
            allow_bank_fetch: req.allow_bank_fetch,
            ..FindOptions::default()
        };
        if let Some(ess) = bot.traveller.essence() {
            opts.essence = Some(ess);
        }
        let capture = InspectCapture {
            request_id: req.request_id,
            generation,
            world,
            state: state.unwrap_or_else(WorldState::empty),
            bank,
            from: req.from,
            to: req.to,
            opts,
            avoid: req.avoid,
            cache,
            obj_names,
            test_barrier: take_test_barrier(),
            search_budget_strict: None,
            search_budget_stand: None,
            search_budget_post: None,
        };
        if req.request_id != 0 {
            bot.inspect.accepted_id = req.request_id;
        }
        bot.inspect.live_world = Some(Arc::clone(&capture.world));
        if bot.inspect.worker.is_some() {
            bot.inspect.pending = Some(capture);
            bot.inspect.pending_id = req.request_id;
            return;
        }
        bot.inspect.pending = Some(capture);
        bot.inspect.pending_id = req.request_id;
        if force_spawn_fail() {
            bot.inspect.pending = None;
            bot.inspect.pending_id = 0;
            bot.inspect.publish(InspectTerminal::refusal(
                req.request_id,
                generation,
                "spawn-failed",
            ));
            return;
        }
        let token = Arc::new(());
        bot.inspect.worker = Some(Arc::clone(&token));
        token
    };
    let navs = Arc::clone(navs);
    let name = name.to_string();
    let worker_token = Arc::clone(&token);
    let spawned = thread::Builder::new()
        .name(format!("nav-inspect-{name}"))
        .spawn({
            let navs = Arc::clone(&navs);
            let name = name.clone();
            let worker_token = Arc::clone(&worker_token);
            move || inspect_worker(navs, name, worker_token)
        })
        .is_ok();
    if !spawned {
        if let Some(bot) = navs.lock().unwrap().get_mut(&name) {
            if bot
                .inspect
                .worker
                .as_ref()
                .is_some_and(|t| Arc::ptr_eq(t, &token))
            {
                let generation = bot.inspect.generation;
                let id = bot
                    .inspect
                    .pending
                    .as_ref()
                    .map(|p| p.request_id)
                    .unwrap_or(0);
                bot.inspect.pending = None;
                bot.inspect.pending_id = 0;
                bot.inspect.worker = None;
                if id != 0 {
                    bot.inspect
                        .publish(InspectTerminal::refusal(id, generation, "spawn-failed"));
                }
            }
        }
    }
}

fn inspect_worker(navs: Arc<Mutex<HashMap<String, NavBot>>>, name: String, worker_token: Arc<()>) {
    loop {
        let request = {
            let mut all = navs.lock().unwrap();
            let Some(bot) = all.get_mut(&name) else {
                return;
            };
            if !bot
                .inspect
                .worker
                .as_ref()
                .is_some_and(|t| Arc::ptr_eq(t, &worker_token))
            {
                return;
            }
            let Some(request) = bot.inspect.pending.take() else {
                bot.inspect.worker = None;
                bot.inspect.running_id = 0;
                bot.inspect.pending_id = 0;
                bot.inspect.executing = false;
                return;
            };
            bot.inspect.pending_id = 0;
            bot.inspect.running_id = request.request_id;
            bot.inspect.executing = true;
            request
        };
        wait_capture_barrier(&request);
        let terminal = calculate(&request);
        let mut all = navs.lock().unwrap();
        let Some(bot) = all.get_mut(&name) else {
            return;
        };
        if !bot
            .inspect
            .worker
            .as_ref()
            .is_some_and(|t| Arc::ptr_eq(t, &worker_token))
        {
            return;
        }
        let identity_ok = request.generation == bot.inspect.generation
            && bot
                .inspect
                .live_world
                .as_ref()
                .is_some_and(|ptr| Arc::ptr_eq(&request.world, ptr))
            && request.request_id == bot.inspect.running_id;
        if identity_ok {
            bot.inspect.publish(terminal);
        } else if request.request_id != 0
            && bot.inspect.running_id == request.request_id
            && bot.inspect.pending.is_none()
        {
            bot.inspect.publish(InspectTerminal::refusal(
                request.request_id,
                request.generation,
                "stale",
            ));
        }
        if bot.inspect.pending.is_none() {
            bot.inspect.worker = None;
            bot.inspect.running_id = 0;
            bot.inspect.executing = false;
            return;
        }
    }
}

fn refuse_locked(bot: &mut NavBot, request_id: u64, reason: &str) {
    if request_id != 0 {
        bot.inspect.accepted_id = request_id;
    }
    bot.inspect.publish(InspectTerminal::refusal(
        request_id,
        bot.inspect.generation,
        reason,
    ));
}

pub(super) fn reset_inspect(bot: &mut NavBot) {
    bot.inspect.generation = bot.inspect.generation.wrapping_add(1);
    bot.inspect.pending = None;
    bot.inspect.pending_id = 0;
    bot.inspect.running_id = 0;
    bot.inspect.executing = false;
    bot.inspect.clear_published();
}

pub(super) fn calculate(capture: &InspectCapture) -> InspectTerminal {
    let pre = &capture.state;
    let avoid = capture.avoid.as_slice();
    let strict = search_path(
        capture,
        capture.from,
        capture.to,
        capture.opts,
        pre,
        avoid,
        capture.search_budget_strict,
    );
    match strict {
        Ok(route) => ok_terminal(capture, route, false),
        Err(RouteError::BudgetExhausted) => {
            InspectTerminal::refusal(capture.request_id, capture.generation, "BudgetExhausted")
        }
        Err(RouteError::NoPath) if !capture.opts.allow_bank_fetch => {
            InspectTerminal::refusal(capture.request_id, capture.generation, "NoPath")
        }
        Err(RouteError::NoPath) => calculate_bank(capture, pre, avoid),
    }
}

fn calculate_bank(
    capture: &InspectCapture,
    pre: &WorldState,
    avoid: &[AvoidRect],
) -> InspectTerminal {
    let Some(missing) = search_missing(capture, pre, avoid) else {
        return InspectTerminal::refusal(capture.request_id, capture.generation, "NoPath");
    };
    let Some(plan) = plan_bank_fetch(
        &missing,
        pre,
        &capture.bank,
        capture.world.banks(),
        capture.from,
    ) else {
        return InspectTerminal::refusal(capture.request_id, capture.generation, "NoPath");
    };
    let bank_planned = true;
    if let Some(stand) = plan.steps.iter().find_map(|step| match step {
        BankStep::Walk { x, z, level } => Some(WorldTile {
            x: *x,
            z: *z,
            level: *level,
        }),
        _ => None,
    }) {
        match search_path(
            capture,
            capture.from,
            stand,
            FindOptions {
                allow_bank_fetch: false,
                ..capture.opts
            },
            pre,
            avoid,
            capture.search_budget_stand,
        ) {
            Ok(_) => {}
            Err(RouteError::BudgetExhausted) => {
                return InspectTerminal::refusal(
                    capture.request_id,
                    capture.generation,
                    "BudgetExhausted",
                );
            }
            Err(RouteError::NoPath) => {
                return InspectTerminal {
                    bank_planned: false,
                    ..InspectTerminal::refusal(capture.request_id, capture.generation, "NoPath")
                };
            }
        }
    }
    let post = search_path(
        capture,
        capture.from,
        capture.to,
        FindOptions {
            allow_bank_fetch: false,
            ..capture.opts
        },
        &plan.state,
        avoid,
        capture.search_budget_post,
    );
    match post {
        Ok(route) => ok_terminal(capture, route, bank_planned),
        Err(RouteError::BudgetExhausted) => InspectTerminal {
            bank_planned,
            ..InspectTerminal::refusal(capture.request_id, capture.generation, "BudgetExhausted")
        },
        Err(RouteError::NoPath) => InspectTerminal {
            bank_planned,
            ..InspectTerminal::refusal(capture.request_id, capture.generation, "NoPath")
        },
    }
}

fn search_path(
    capture: &InspectCapture,
    from: WorldTile,
    to: WorldTile,
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    budget: Option<usize>,
) -> Result<Route, RouteError> {
    match budget {
        Some(n) => find_with_avoid_bounded(
            &capture.world.collision,
            &capture.world.graph,
            from,
            to,
            opts,
            state,
            avoid,
            n,
        ),
        None => find_with_avoid(
            &capture.world.collision,
            &capture.world.graph,
            from,
            to,
            opts,
            state,
            avoid,
        ),
    }
}

fn search_missing(
    capture: &InspectCapture,
    pre: &WorldState,
    avoid: &[AvoidRect],
) -> Option<Vec<nav::router::MissingReq>> {
    match capture.search_budget_strict {
        Some(n) => find_missing_item_reqs_with_avoid_bounded(
            &capture.world.collision,
            &capture.world.graph,
            capture.from,
            capture.to,
            capture.opts,
            pre,
            avoid,
            n,
        ),
        None => find_missing_item_reqs_with_avoid(
            &capture.world.collision,
            &capture.world.graph,
            capture.from,
            capture.to,
            capture.opts,
            pre,
            avoid,
        ),
    }
}

fn ok_terminal(capture: &InspectCapture, route: Route, bank_planned: bool) -> InspectTerminal {
    let hops = route
        .legs
        .iter()
        .filter_map(|leg| match leg {
            Leg::Transport { edge } => Some(project_hop(
                edge,
                capture.cache.as_deref(),
                capture.obj_names.as_deref(),
            )),
            Leg::Walk { .. } => None,
        })
        .collect();
    InspectTerminal {
        seq: 0,
        generation: capture.generation,
        request_id: capture.request_id,
        ok: true,
        reason: String::new(),
        bank_planned,
        ticks: route.ticks,
        hops,
    }
}

fn project_hop(
    edge: &nav::transport::TransportEdge,
    cache: Option<&Cache>,
    obj_names: Option<&ObjNames>,
) -> InspectHop {
    let kind = kind_name(edge.kind);
    let (loc_name, action) = match edge.kind {
        TransportKind::Boat | TransportKind::Npc | TransportKind::Glider => {
            let npc = cache.and_then(|c| c.npcs.get(edge.loc_id as usize));
            (
                npc.map(|n| n.name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_default(),
                npc.map(|n| op_at(&n.op, edge.option)).unwrap_or_default(),
            )
        }
        TransportKind::Door
        | TransportKind::Ladder
        | TransportKind::Stairs
        | TransportKind::AgilityShortcut
        | TransportKind::SpiritTree
        | TransportKind::EssenceExit => {
            let loc = cache.and_then(|c| c.locs.get(edge.loc_id as usize));
            (
                loc.map(|l| l.name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_default(),
                loc.map(|l| op_at(&l.op, edge.option)).unwrap_or_default(),
            )
        }
        TransportKind::Teleport if edge.loc_id > 0 => {
            let name = obj_names
                .and_then(|names| names.name(edge.loc_id))
                .unwrap_or("")
                .to_string();
            let action = cache
                .and_then(|c| c.objs.get(edge.loc_id as usize))
                .map(|o| op_at_obj(&o.op, edge.option))
                .unwrap_or_default();
            (name, action)
        }
        TransportKind::Teleport => (String::new(), String::new()),
    };
    InspectHop {
        kind: kind.into(),
        loc_id: edge.loc_id,
        loc_name,
        action,
        option: edge.option,
        from_x: edge.at.x,
        from_z: edge.at.z,
        from_level: edge.at.level,
        to_x: edge.to.x,
        to_z: edge.to.z,
        to_level: edge.to.level,
        ticks: edge.ticks,
    }
}

fn kind_name(kind: TransportKind) -> &'static str {
    match kind {
        TransportKind::Door => "door",
        TransportKind::Ladder => "ladder",
        TransportKind::Stairs => "stairs",
        TransportKind::Boat => "boat",
        TransportKind::Teleport => "teleport",
        TransportKind::AgilityShortcut => "agilityshortcut",
        TransportKind::Glider => "glider",
        TransportKind::SpiritTree => "spirittree",
        TransportKind::Npc => "npc",
        TransportKind::EssenceExit => "essenceexit",
    }
}

fn op_at(ops: &[Option<String>], option: i32) -> String {
    if option <= 0 {
        return String::new();
    }
    ops.get((option as usize).saturating_sub(1))
        .and_then(|op| op.clone())
        .unwrap_or_default()
}

fn op_at_obj(ops: &[Option<String>; 5], option: i32) -> String {
    op_at(ops.as_slice(), option)
}

pub(super) fn terminal_to_input<'a>(
    term: &'a InspectTerminal,
    hops: &'a [script::isolate_fb::InspectHopInput<'a>],
) -> script::isolate_fb::RouteInspectTerminalInput<'a> {
    script::isolate_fb::RouteInspectTerminalInput {
        seq: term.seq,
        generation: term.generation,
        request_id: term.request_id,
        ok: term.ok,
        reason: Some(term.reason.as_str()),
        bank_planned: term.bank_planned,
        ticks: term.ticks,
        hops,
    }
}

pub(super) fn hop_inputs(hops: &[InspectHop]) -> Vec<script::isolate_fb::InspectHopInput<'_>> {
    hops.iter()
        .map(|h| script::isolate_fb::InspectHopInput {
            kind: h.kind.as_str(),
            loc_id: h.loc_id,
            loc_name: h.loc_name.as_str(),
            action: h.action.as_str(),
            option: h.option,
            from_x: h.from_x,
            from_z: h.from_z,
            from_level: h.from_level,
            to_x: h.to_x,
            to_z: h.to_z,
            to_level: h.to_level,
            ticks: h.ticks,
        })
        .collect()
}

thread_local! {
    static FORCE_SPAWN_FAIL: AtomicBool = const { AtomicBool::new(false) };
    static TEST_BARRIER: std::cell::RefCell<Option<Arc<InspectBarrier>>> =
        const { std::cell::RefCell::new(None) };
}

fn force_spawn_fail() -> bool {
    FORCE_SPAWN_FAIL.with(|f| f.swap(false, Ordering::SeqCst))
}

fn take_test_barrier() -> Option<Arc<InspectBarrier>> {
    TEST_BARRIER.with(|b| b.borrow().clone())
}

fn wait_capture_barrier(capture: &InspectCapture) {
    if let Some(barrier) = &capture.test_barrier {
        barrier.enter();
        barrier.wait_release();
    }
}

pub(crate) struct InspectBarrier {
    entered: (Mutex<bool>, Condvar),
    release: (Mutex<bool>, Condvar),
}

impl InspectBarrier {
    #[cfg(test)]
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: (Mutex::new(false), Condvar::new()),
            release: (Mutex::new(false), Condvar::new()),
        })
    }

    fn enter(&self) {
        let (lock, cv) = &self.entered;
        let mut g = lock.lock().unwrap();
        *g = true;
        cv.notify_all();
    }

    #[cfg(test)]
    pub(crate) fn wait_entered(&self) {
        let (lock, cv) = &self.entered;
        let mut g = lock.lock().unwrap();
        while !*g {
            g = cv.wait(g).unwrap();
        }
    }

    fn wait_release(&self) {
        let (lock, cv) = &self.release;
        let mut g = lock.lock().unwrap();
        while !*g {
            g = cv.wait(g).unwrap();
        }
    }

    #[cfg(test)]
    pub(crate) fn release(&self) {
        let (lock, cv) = &self.release;
        let mut g = lock.lock().unwrap();
        *g = true;
        cv.notify_all();
    }
}

#[cfg(test)]
pub(crate) fn install_barrier(barrier: Arc<InspectBarrier>) {
    TEST_BARRIER.with(|b| *b.borrow_mut() = Some(barrier));
}

#[cfg(test)]
pub(crate) fn clear_barrier() {
    TEST_BARRIER.with(|b| *b.borrow_mut() = None);
}

#[cfg(test)]
pub(crate) fn force_next_spawn_fail() {
    FORCE_SPAWN_FAIL.with(|f| f.store(true, Ordering::SeqCst));
}

#[cfg(test)]
#[path = "route_inspect_tests.rs"]
mod tests;
