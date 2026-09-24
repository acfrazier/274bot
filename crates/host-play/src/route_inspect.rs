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
pub(super) struct InspectHop {
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
pub(super) struct InspectTerminal {
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
pub(super) struct InspectCapture {
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
mod tests {
    use super::*;
    use api::snapshot::WorldTile;
    use client::config::{Cache, LocType, NpcType, ObjType};
    use nav::collision::{pack_walk, WorldCollision};
    use nav::pack::{BankAccess, BankStand};
    use nav::transport::{TransportEdge, TransportGraph};
    use nav::world::NavWorld;
    use std::collections::HashMap;
    use std::time::Duration;

    fn tile(x: i32, z: i32, level: i32) -> WorldTile {
        WorldTile { x, z, level }
    }

    fn open_world(width: usize, height: usize) -> WorldCollision {
        let flags = vec![0u32; width * height * 4];
        let (walk, blocked) = pack_walk(&flags);
        WorldCollision {
            origin: tile(0, 0, 0),
            width,
            height,
            walk,
            blocked,
            flags: None,
        }
    }

    fn edge(
        kind: TransportKind,
        at: WorldTile,
        to: WorldTile,
        loc_id: i32,
        item_req: Vec<(i32, i32)>,
    ) -> TransportEdge {
        TransportEdge {
            kind,
            at,
            to,
            loc_id,
            option: 1,
            ticks: 2,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req,
            quest_req: vec![],
            varp_req: vec![],
            worn_req: vec![],
            members_req: false,
        }
    }

    fn world_with(graph: TransportGraph, banks: Vec<BankStand>) -> Arc<NavWorld> {
        Arc::new(NavWorld::from_parts(open_world(8, 8), graph, banks))
    }

    fn named_cache() -> Arc<Cache> {
        let mut cache = Cache::default();
        while cache.npcs.len() <= 381 {
            cache.npcs.push(NpcType::default());
        }
        cache.npcs[170] = NpcType {
            id: 170,
            name: "Captain Klemfoodle".into(),
            op: vec![Some("Talk-to".into())],
            ..Default::default()
        };
        cache.npcs[378] = NpcType {
            id: 378,
            name: "Seaman Thresnor".into(),
            op: vec![Some("Pay-fare".into())],
            ..Default::default()
        };
        cache.npcs[381] = NpcType {
            id: 381,
            name: "Captain Barnaby".into(),
            op: vec![Some("Pay-fare".into())],
            ..Default::default()
        };
        while cache.locs.len() <= 2082 {
            cache.locs.push(LocType::default());
        }
        cache.locs[2082] = LocType {
            id: 2082,
            name: "Gangplank".into(),
            op: vec![Some("Cross".into())],
            ..Default::default()
        };
        while cache.objs.len() <= 1712 {
            cache.objs.push(ObjType::default());
        }
        cache.objs[1712] = ObjType {
            id: 1712,
            name: "Glory".into(),
            op: [None, None, None, Some("Rub".into()), None],
            ..Default::default()
        };
        Arc::new(cache)
    }

    fn names(cache: &Cache) -> Arc<ObjNames> {
        Arc::new(ObjNames::from_objs(&cache.objs))
    }

    fn req(from: WorldTile, to: WorldTile, request_id: u64) -> InspectRequest {
        InspectRequest {
            from,
            to,
            allow_teleports: false,
            allow_wilderness: true,
            allow_bank_fetch: false,
            avoid: Vec::new(),
            request_id,
            invalid_args: false,
        }
    }

    fn wait_latest(navs: &Arc<Mutex<HashMap<String, NavBot>>>, id: u64) -> InspectTerminal {
        let start = std::time::Instant::now();
        loop {
            {
                let all = navs.lock().unwrap();
                if let Some(bot) = all.get("p") {
                    if let Some(term) = bot.inspect.latest.as_ref() {
                        if term.request_id == id {
                            return term.clone();
                        }
                    }
                    if let Some(term) = bot.inspect.prev.as_ref() {
                        if term.request_id == id {
                            return term.clone();
                        }
                    }
                    if let Some(term) = bot.inspect.held.as_ref() {
                        if term.request_id == id {
                            return term.clone();
                        }
                    }
                }
            }
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "inspect {id} did not publish"
            );
            thread::yield_now();
        }
    }

    fn wait_idle(navs: &Arc<Mutex<HashMap<String, NavBot>>>) {
        let start = std::time::Instant::now();
        loop {
            {
                let all = navs.lock().unwrap();
                if let Some(bot) = all.get("p") {
                    if !bot.inspect.executing
                        && bot.inspect.pending.is_none()
                        && bot.inspect.worker.is_none()
                    {
                        return;
                    }
                }
            }
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "inspect worker did not idle"
            );
            thread::yield_now();
        }
    }

    fn fill_three_id0(navs: &Arc<Mutex<HashMap<String, NavBot>>>, world: &Arc<NavWorld>) {
        for _ in 0..3 {
            queue_inspect(
                navs,
                "p",
                &Some(Arc::clone(world)),
                Some(WorldState::empty()),
                vec![],
                None,
                None,
                req(tile(0, 0, 0), tile(2, 2, 1), 0),
            );
            wait_idle(navs);
        }
        let all = navs.lock().unwrap();
        let bot = all.get("p").unwrap();
        assert_eq!(bot.inspect.unobserved_count(), 3);
        assert!(!bot.inspect.can_admit(false));
    }

    #[test]
    fn path_attribute_module_compiles() {
        let _ = std::any::type_name::<InspectNav>();
    }

    #[test]
    fn glider_and_boats_use_npc_table() {
        let cache = named_cache();
        let names = names(&cache);
        let glider = project_hop(
            &edge(
                TransportKind::Glider,
                tile(1, 1, 0),
                tile(2, 2, 0),
                170,
                vec![],
            ),
            Some(cache.as_ref()),
            Some(names.as_ref()),
        );
        assert_eq!(glider.kind, "glider");
        assert_eq!(glider.loc_name, "Captain Klemfoodle");
        let barnaby = project_hop(
            &edge(
                TransportKind::Boat,
                tile(1, 1, 0),
                tile(2, 2, 0),
                381,
                vec![],
            ),
            Some(cache.as_ref()),
            Some(names.as_ref()),
        );
        assert_eq!(barnaby.loc_name, "Captain Barnaby");
        let thresnor = project_hop(
            &edge(
                TransportKind::Boat,
                tile(1, 1, 0),
                tile(2, 2, 0),
                378,
                vec![],
            ),
            Some(cache.as_ref()),
            Some(names.as_ref()),
        );
        assert_eq!(thresnor.loc_name, "Seaman Thresnor");
        let plank = project_hop(
            &edge(
                TransportKind::Ladder,
                tile(1, 1, 0),
                tile(2, 2, 0),
                2082,
                vec![],
            ),
            Some(cache.as_ref()),
            Some(names.as_ref()),
        );
        assert_eq!(plank.kind, "ladder");
        assert_eq!(plank.loc_name, "Gangplank");
        let jewellery = project_hop(
            &TransportEdge {
                option: 4,
                loc_id: 1712,
                kind: TransportKind::Teleport,
                ..edge(
                    TransportKind::Teleport,
                    tile(0, 0, 0),
                    tile(1, 1, 0),
                    1712,
                    vec![],
                )
            },
            Some(cache.as_ref()),
            Some(names.as_ref()),
        );
        assert_eq!(jewellery.loc_name, "Glory");
        assert_eq!(jewellery.action, "Rub");
        let spell = project_hop(
            &edge(
                TransportKind::Teleport,
                tile(0, 0, 0),
                tile(1, 1, 0),
                0,
                vec![],
            ),
            Some(cache.as_ref()),
            Some(names.as_ref()),
        );
        assert_eq!(spell.loc_name, "");
        let unknown = project_hop(
            &edge(TransportKind::Boat, tile(0, 0, 0), tile(1, 1, 0), 9, vec![]),
            Some(cache.as_ref()),
            Some(names.as_ref()),
        );
        assert_eq!(unknown.loc_name, "");
    }

    #[test]
    fn bank_false_item_gate_is_nopath() {
        let mut graph = TransportGraph::default();
        let at = tile(0, 0, 0);
        let to = tile(4, 4, 1);
        graph.at.entry(at).or_default().push(0);
        graph
            .edges
            .push(edge(TransportKind::Boat, at, to, 381, vec![(995, 30)]));
        let world = world_with(graph, vec![]);
        let capture = InspectCapture {
            request_id: 1,
            generation: 0,
            world,
            state: WorldState::empty(),
            bank: vec![],
            from: at,
            to,
            opts: FindOptions {
                allow_wilderness: true,
                ..FindOptions::default()
            },
            avoid: vec![],
            cache: None,
            obj_names: None,
            test_barrier: None,
            search_budget_strict: None,
            search_budget_stand: None,
            search_budget_post: None,
        };
        let term = calculate(&capture);
        assert!(!term.ok);
        assert_eq!(term.reason, "NoPath");
        assert!(!term.bank_planned);
    }

    #[test]
    fn pre_state_stand_proof_rejects_post_only_stand() {
        let mut graph = TransportGraph::default();
        let from = tile(0, 0, 0);
        let dest = tile(0, 4, 1);
        let stand = tile(4, 4, 1);
        graph.at.entry(from).or_default().push(0);
        graph
            .edges
            .push(edge(TransportKind::Boat, from, dest, 381, vec![(995, 10)]));
        graph.at.entry(from).or_default().push(1);
        graph.edges.push(edge(
            TransportKind::Door,
            from,
            stand,
            1530,
            vec![(995, 10)],
        ));
        let world = world_with(
            graph,
            vec![BankStand {
                name: "Bank booth".into(),
                tile: stand,
                access: BankAccess::Booth { op: 2 },
            }],
        );
        let capture = InspectCapture {
            request_id: 2,
            generation: 0,
            world,
            state: WorldState::empty(),
            bank: vec![(995, 10)],
            from,
            to: dest,
            opts: FindOptions {
                allow_wilderness: true,
                allow_bank_fetch: true,
                ..FindOptions::default()
            },
            avoid: vec![],
            cache: None,
            obj_names: None,
            test_barrier: None,
            search_budget_strict: None,
            search_budget_stand: None,
            search_budget_post: None,
        };
        let term = calculate(&capture);
        assert!(!term.ok, "PRE stand search must fail without the coins");
        assert!(!term.bank_planned);
        assert_eq!(term.reason, "NoPath");
    }

    #[test]
    fn ok_hops_are_post_from_to_not_bank_steps() {
        let mut graph = TransportGraph::default();
        let from = tile(0, 0, 0);
        let dest = tile(4, 0, 1);
        graph.at.entry(from).or_default().push(0);
        graph
            .edges
            .push(edge(TransportKind::Boat, from, dest, 381, vec![(995, 10)]));
        let world = world_with(
            graph,
            vec![BankStand {
                name: "Bank booth".into(),
                tile: tile(1, 1, 0),
                access: BankAccess::Booth { op: 2 },
            }],
        );
        let capture = InspectCapture {
            request_id: 3,
            generation: 0,
            world,
            state: WorldState::empty(),
            bank: vec![(995, 10)],
            from,
            to: dest,
            opts: FindOptions {
                allow_wilderness: true,
                allow_bank_fetch: true,
                ..FindOptions::default()
            },
            avoid: vec![],
            cache: Some(named_cache()),
            obj_names: None,
            test_barrier: None,
            search_budget_strict: None,
            search_budget_stand: None,
            search_budget_post: None,
        };
        let term = calculate(&capture);
        assert!(
            term.ok,
            "open walk to stand plus post boat should plan, got {}",
            term.reason
        );
        assert!(term.bank_planned);
        assert!(term.hops.iter().all(|h| h.kind != "bank"));
        assert_eq!(term.hops[0].kind, "boat");
        assert!(term.ticks > 0.0);
    }

    #[test]
    fn delayed_calculate_barrier_is_nonblocking() {
        clear_barrier();
        let barrier = InspectBarrier::new();
        install_barrier(Arc::clone(&barrier));
        let world = world_with(TransportGraph::default(), vec![]);
        let navs = Arc::new(Mutex::new(HashMap::new()));
        queue_inspect(
            &navs,
            "p",
            &Some(world),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(2, 2, 1), 11),
        );
        barrier.wait_entered();
        {
            let all = navs.lock().unwrap();
            let bot = all.get("p").unwrap();
            assert!(bot.inspect.worker.is_some());
            assert_eq!(bot.inspect.running_id, 11);
        }
        barrier.release();
        let term = wait_latest(&navs, 11);
        assert!(!term.ok);
        clear_barrier();
    }

    #[test]
    fn reset_keeps_inspect_worker() {
        clear_barrier();
        let barrier = InspectBarrier::new();
        install_barrier(Arc::clone(&barrier));
        let world = world_with(TransportGraph::default(), vec![]);
        let navs = Arc::new(Mutex::new(HashMap::new()));
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(2, 2, 0), 21),
        );
        barrier.wait_entered();
        {
            let mut all = navs.lock().unwrap();
            reset_inspect(all.get_mut("p").unwrap());
            let bot = all.get("p").unwrap();
            assert!(bot.inspect.worker.is_some(), "reset must leave the worker");
            assert!(bot.inspect.latest.is_none());
        }
        queue_inspect(
            &navs,
            "p",
            &Some(world),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(3, 3, 0), 22),
        );
        {
            let all = navs.lock().unwrap();
            let bot = all.get("p").unwrap();
            assert!(bot.inspect.worker.is_some());
            assert_eq!(bot.inspect.pending_id, 22);
        }
        barrier.release();
        let _ = wait_latest(&navs, 22);
        clear_barrier();
    }

    #[test]
    fn spawn_failed_publishes_and_frees_token() {
        force_next_spawn_fail();
        let world = world_with(TransportGraph::default(), vec![]);
        let navs = Arc::new(Mutex::new(HashMap::new()));
        queue_inspect(
            &navs,
            "p",
            &Some(world),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(1, 1, 0), 31),
        );
        let all = navs.lock().unwrap();
        let bot = all.get("p").unwrap();
        assert!(bot.inspect.worker.is_none());
        assert_eq!(bot.inspect.latest.as_ref().unwrap().reason, "spawn-failed");
        assert_eq!(bot.inspect.latest.as_ref().unwrap().request_id, 31);
    }

    #[test]
    fn pending_replace_publishes_stale_for_b() {
        clear_barrier();
        let barrier = InspectBarrier::new();
        install_barrier(Arc::clone(&barrier));
        let world = world_with(TransportGraph::default(), vec![]);
        let navs = Arc::new(Mutex::new(HashMap::new()));
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(1, 1, 0), 41),
        );
        barrier.wait_entered();
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(2, 2, 0), 42),
        );
        queue_inspect(
            &navs,
            "p",
            &Some(world),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(3, 3, 0), 43),
        );
        {
            let all = navs.lock().unwrap();
            let bot = all.get("p").unwrap();
            assert_eq!(bot.inspect.running_id, 41);
            assert_eq!(bot.inspect.pending_id, 43);
            assert_eq!(bot.inspect.replaced_id, 42);
            assert_eq!(bot.inspect.latest.as_ref().unwrap().request_id, 42);
            assert_eq!(bot.inspect.latest.as_ref().unwrap().reason, "stale");
        }
        barrier.release();
        clear_barrier();
    }

    #[test]
    fn ring_holds_unacked_prev_until_apply_ack() {
        let mut nav = InspectNav::default();
        nav.publish(InspectTerminal::refusal(1, 0, "NoPath"));
        nav.publish(InspectTerminal::refusal(2, 0, "NoPath"));
        nav.publish(InspectTerminal::refusal(3, 0, "NoPath"));
        assert_eq!(nav.latest.as_ref().unwrap().request_id, 2);
        assert_eq!(nav.prev.as_ref().unwrap().request_id, 1);
        assert_eq!(nav.held.as_ref().unwrap().request_id, 3);
        nav.apply_ack(nav.prev.as_ref().unwrap().seq, 0);
        assert_eq!(nav.latest.as_ref().unwrap().request_id, 3);
        assert_eq!(nav.prev.as_ref().unwrap().request_id, 2);
        assert!(nav.held.is_none());
    }

    #[test]
    fn invalid_args_and_missing_graph() {
        let navs = Arc::new(Mutex::new(HashMap::new()));
        let mut bad = req(tile(0, 0, 9), tile(1, 1, 0), 51);
        bad.invalid_args = true;
        queue_inspect(&navs, "p", &None, None, vec![], None, None, bad);
        assert_eq!(
            navs.lock()
                .unwrap()
                .get("p")
                .unwrap()
                .inspect
                .latest
                .as_ref()
                .unwrap()
                .reason,
            "invalid-args"
        );
        queue_inspect(
            &navs,
            "p",
            &None,
            None,
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(1, 1, 0), 52),
        );
        assert_eq!(
            navs.lock()
                .unwrap()
                .get("p")
                .unwrap()
                .inspect
                .latest
                .as_ref()
                .unwrap()
                .reason,
            "missing-graph"
        );
    }

    #[test]
    fn abort_script_walk_leaves_inspect_worker() {
        clear_barrier();
        let barrier = InspectBarrier::new();
        install_barrier(Arc::clone(&barrier));
        let world = world_with(TransportGraph::default(), vec![]);
        let navs = Arc::new(Mutex::new(HashMap::new()));
        queue_inspect(
            &navs,
            "p",
            &Some(world),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(2, 2, 0), 61),
        );
        barrier.wait_entered();
        crate::abort_script_walk(&navs, "p");
        {
            let all = navs.lock().unwrap();
            let bot = all.get("p").unwrap();
            assert!(bot.inspect.worker.is_some());
            assert_eq!(bot.inspect.running_id, 61);
            assert!(bot.route_worker.is_none());
        }
        barrier.release();
        let term = wait_latest(&navs, 61);
        assert_eq!(term.request_id, 61);
        clear_barrier();
    }

    fn capture(
        world: Arc<NavWorld>,
        from: WorldTile,
        to: WorldTile,
        opts: FindOptions,
        request_id: u64,
        bank: Vec<(i32, i32)>,
        strict: Option<usize>,
        stand: Option<usize>,
        post: Option<usize>,
    ) -> InspectCapture {
        InspectCapture {
            request_id,
            generation: 0,
            world,
            state: WorldState::empty(),
            bank,
            from,
            to,
            opts,
            avoid: vec![],
            cache: None,
            obj_names: None,
            test_barrier: None,
            search_budget_strict: strict,
            search_budget_stand: stand,
            search_budget_post: post,
        }
    }

    #[test]
    fn budget_exhausted_distinct_from_nopath_strict_stand_post() {
        let from = tile(0, 0, 0);
        let far = tile(9, 9, 0);
        let open = Arc::new(NavWorld::from_parts(
            open_world(10, 10),
            TransportGraph::default(),
            vec![],
        ));
        let exhausted = calculate(&capture(
            Arc::clone(&open),
            from,
            far,
            FindOptions {
                allow_wilderness: true,
                ..FindOptions::default()
            },
            71,
            vec![],
            Some(8),
            None,
            None,
        ));
        assert!(!exhausted.ok);
        assert_eq!(exhausted.reason, "BudgetExhausted");
        assert!(!exhausted.bank_planned);

        let blocked = calculate(&capture(
            Arc::clone(&open),
            from,
            far,
            FindOptions {
                allow_wilderness: true,
                ..FindOptions::default()
            },
            72,
            vec![],
            None,
            None,
            None,
        ));
        assert!(
            blocked.ok,
            "unbounded open walk must path, got {}",
            blocked.reason
        );

        let bank_opts = FindOptions {
            allow_wilderness: true,
            allow_bank_fetch: true,
            ..FindOptions::default()
        };
        let mut stand_graph = TransportGraph::default();
        let stand_far = tile(9, 9, 0);
        let stand_dest = tile(2, 2, 1);
        stand_graph.at.entry(from).or_default().push(0);
        stand_graph.edges.push(edge(
            TransportKind::Boat,
            from,
            stand_dest,
            381,
            vec![(995, 10)],
        ));
        let stand_world = Arc::new(NavWorld::from_parts(
            open_world(10, 10),
            stand_graph,
            vec![BankStand {
                name: "Bank booth".into(),
                tile: stand_far,
                access: BankAccess::Booth { op: 2 },
            }],
        ));
        let stand_ex = calculate(&capture(
            stand_world,
            from,
            stand_dest,
            bank_opts,
            73,
            vec![(995, 10)],
            None,
            Some(8),
            None,
        ));
        assert_eq!(stand_ex.reason, "BudgetExhausted");
        assert!(!stand_ex.bank_planned);

        let mut post_graph = TransportGraph::default();
        let post_dest = tile(9, 9, 1);
        let stand_near = tile(1, 1, 0);
        post_graph.at.entry(from).or_default().push(0);
        post_graph.edges.push(edge(
            TransportKind::Boat,
            from,
            tile(0, 1, 1),
            381,
            vec![(995, 10)],
        ));
        let post_world = Arc::new(NavWorld::from_parts(
            open_world(10, 10),
            post_graph,
            vec![BankStand {
                name: "Bank booth".into(),
                tile: stand_near,
                access: BankAccess::Booth { op: 2 },
            }],
        ));
        let stand_ok_post_ex = calculate(&capture(
            post_world,
            from,
            post_dest,
            bank_opts,
            74,
            vec![(995, 10)],
            None,
            None,
            Some(8),
        ));
        assert_eq!(stand_ok_post_ex.reason, "BudgetExhausted");
        assert!(stand_ok_post_ex.bank_planned);
    }

    #[test]
    fn admission_refuses_fourth_when_ring_and_held_full() {
        let world = world_with(TransportGraph::default(), vec![]);
        let navs = Arc::new(Mutex::new(HashMap::new()));
        for id in 1..=3 {
            queue_inspect(
                &navs,
                "p",
                &Some(Arc::clone(&world)),
                Some(WorldState::empty()),
                vec![],
                None,
                None,
                req(tile(0, 0, 0), tile(1, 1, 0), id),
            );
            let _ = wait_latest(&navs, id);
        }
        {
            let all = navs.lock().unwrap();
            let bot = all.get("p").unwrap();
            assert_eq!(bot.inspect.latest.as_ref().unwrap().request_id, 2);
            assert_eq!(bot.inspect.prev.as_ref().unwrap().request_id, 1);
            assert_eq!(bot.inspect.held.as_ref().unwrap().request_id, 3);
            assert_eq!(bot.inspect.accepted_id, 3);
        }
        queue_inspect(
            &navs,
            "p",
            &Some(world),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(2, 2, 0), 4),
        );
        {
            let all = navs.lock().unwrap();
            let bot = all.get("p").unwrap();
            assert_eq!(bot.inspect.accepted_id, 3, "fourth must not be accepted");
            assert_eq!(bot.inspect.held.as_ref().unwrap().request_id, 3);
            assert_eq!(bot.inspect.latest.as_ref().unwrap().request_id, 2);
            assert_eq!(bot.inspect.refused, [4, 0, 0]);
        }
    }

    #[test]
    fn apply_ack_rejects_future_and_wrong_generation() {
        let mut nav = InspectNav::default();
        nav.publish(InspectTerminal::refusal(1, 0, "NoPath"));
        nav.publish(InspectTerminal::refusal(2, 0, "NoPath"));
        nav.publish(InspectTerminal::refusal(3, 0, "NoPath"));
        let prev_seq = nav.prev.as_ref().unwrap().seq;
        nav.apply_ack(99, 0);
        assert_eq!(nav.held.as_ref().unwrap().request_id, 3);
        nav.apply_ack(prev_seq, 7);
        assert_eq!(nav.held.as_ref().unwrap().request_id, 3);
        nav.apply_ack(prev_seq, 0);
        assert!(nav.held.is_none());
        assert_eq!(nav.latest.as_ref().unwrap().request_id, 3);
    }

    #[test]
    fn request_id_zero_publishes_preview() {
        let world = world_with(TransportGraph::default(), vec![]);
        let navs = Arc::new(Mutex::new(HashMap::new()));
        queue_inspect(
            &navs,
            "p",
            &Some(world),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(2, 2, 1), 0),
        );
        let term = wait_latest(&navs, 0);
        assert_eq!(term.request_id, 0);
        assert!(!term.ok);
        assert_eq!(term.reason, "NoPath");
        assert!(term.seq != 0);
    }

    #[test]
    fn can_admit_counts_only_unobserved_after_full_ack() {
        let mut nav = InspectNav::default();
        nav.publish(InspectTerminal::refusal(1, 0, "NoPath"));
        nav.publish(InspectTerminal::refusal(2, 0, "NoPath"));
        nav.publish(InspectTerminal::refusal(3, 0, "NoPath"));
        assert!(!nav.can_admit(false));
        let prev = nav.prev.as_ref().unwrap().seq;
        nav.apply_ack(prev, 0);
        let latest = nav.latest.as_ref().unwrap().seq;
        nav.apply_ack(latest, 0);
        assert_eq!(nav.unobserved_count(), 0);
        assert!(nav.can_admit(false));
        nav.executing = true;
        assert!(
            nav.can_admit(false),
            "acked ring + running still admits pending"
        );
        nav.executing = true;
        assert!(nav.can_admit(true));
    }

    #[test]
    fn admission_refuse_does_not_accept_when_unobserved_full() {
        let world = world_with(TransportGraph::default(), vec![]);
        let navs = Arc::new(Mutex::new(HashMap::new()));
        for id in 1..=3 {
            queue_inspect(
                &navs,
                "p",
                &Some(Arc::clone(&world)),
                Some(WorldState::empty()),
                vec![],
                None,
                None,
                req(tile(0, 0, 0), tile(1, 1, 0), id),
            );
            let _ = wait_latest(&navs, id);
        }
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(2, 2, 0), 4),
        );
        {
            let all = navs.lock().unwrap();
            let bot = all.get("p").unwrap();
            assert_eq!(bot.inspect.accepted_id, 3);
            assert!(bot.inspect.pending.is_none());
            assert_eq!(bot.inspect.held.as_ref().unwrap().request_id, 3);
            assert_eq!(bot.inspect.refused, [4, 0, 0]);
            assert_eq!(bot.inspect.latest.as_ref().unwrap().request_id, 2);
        }
        {
            let mut all = navs.lock().unwrap();
            let bot = all.get_mut("p").unwrap();
            let latest = bot.inspect.latest.as_ref().unwrap().seq;
            bot.inspect.apply_ack(latest, bot.inspect.generation);
        }
        queue_inspect(
            &navs,
            "p",
            &Some(world),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(3, 3, 0), 5),
        );
        let term = wait_latest(&navs, 5);
        assert_eq!(term.request_id, 5);
    }

    #[test]
    fn snapshot_only_overload_does_not_mailbox_or_clobber_latest() {
        let world = world_with(TransportGraph::default(), vec![]);
        let navs = Arc::new(Mutex::new(HashMap::new()));
        fill_three_id0(&navs, &world);
        let before = {
            let all = navs.lock().unwrap();
            let bot = all.get("p").unwrap();
            (
                bot.inspect.latest.as_ref().map(|t| t.seq),
                bot.inspect.held.as_ref().map(|t| t.seq),
                bot.inspect.refused,
            )
        };
        queue_inspect(
            &navs,
            "p",
            &Some(world),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(2, 2, 1), 0),
        );
        {
            let all = navs.lock().unwrap();
            let bot = all.get("p").unwrap();
            assert_eq!(bot.inspect.latest.as_ref().map(|t| t.seq), before.0);
            assert_eq!(bot.inspect.held.as_ref().map(|t| t.seq), before.1);
            assert_eq!(bot.inspect.refused, [0, 0, 0]);
            assert_eq!(bot.inspect.accepted_id, 0);
            assert!(bot.inspect.pending.is_none());
        }
    }

    #[test]
    fn duplicate_running_id_does_not_double_reserve() {
        clear_barrier();
        let barrier = InspectBarrier::new();
        install_barrier(Arc::clone(&barrier));
        let world = world_with(TransportGraph::default(), vec![]);
        let navs = Arc::new(Mutex::new(HashMap::new()));
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(1, 1, 0), 81),
        );
        barrier.wait_entered();
        queue_inspect(
            &navs,
            "p",
            &Some(world),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(2, 2, 0), 81),
        );
        {
            let all = navs.lock().unwrap();
            let bot = all.get("p").unwrap();
            assert_eq!(bot.inspect.running_id, 81);
            assert!(bot.inspect.pending.is_none());
            assert_eq!(bot.inspect.pending_id, 0);
        }
        barrier.release();
        let _ = wait_latest(&navs, 81);
        clear_barrier();
    }

    #[test]
    fn reset_rejects_old_generation_ack() {
        let mut nav = InspectNav::default();
        nav.publish(InspectTerminal::refusal(1, 0, "NoPath"));
        nav.publish(InspectTerminal::refusal(2, 0, "NoPath"));
        nav.publish(InspectTerminal::refusal(3, 0, "NoPath"));
        let old_seq = nav.prev.as_ref().unwrap().seq;
        let old_gen = nav.generation;
        let mut bot = NavBot {
            inspect: nav,
            ..Default::default()
        };
        reset_inspect(&mut bot);
        bot.inspect.publish(InspectTerminal::refusal(
            11,
            bot.inspect.generation,
            "NoPath",
        ));
        bot.inspect.publish(InspectTerminal::refusal(
            12,
            bot.inspect.generation,
            "NoPath",
        ));
        bot.inspect.publish(InspectTerminal::refusal(
            13,
            bot.inspect.generation,
            "NoPath",
        ));
        assert_eq!(bot.inspect.held.as_ref().unwrap().request_id, 13);
        bot.inspect.apply_ack(old_seq, old_gen);
        assert_eq!(
            bot.inspect.held.as_ref().unwrap().request_id,
            13,
            "old-session ACK cannot free the new generation"
        );
        let prev = bot.inspect.prev.as_ref().unwrap().seq;
        bot.inspect.apply_ack(prev, bot.inspect.generation);
        assert!(bot.inspect.held.is_none());
        assert_eq!(bot.inspect.latest.as_ref().unwrap().request_id, 13);
    }

    #[test]
    #[should_panic(expected = "inspect publish overflow")]
    fn publish_overflow_panics_when_held_full() {
        let mut nav = InspectNav::default();
        nav.publish(InspectTerminal::refusal(1, 0, "NoPath"));
        nav.publish(InspectTerminal::refusal(2, 0, "NoPath"));
        nav.publish(InspectTerminal::refusal(3, 0, "NoPath"));
        assert!(!nav.can_admit(false));
        nav.publish(InspectTerminal::refusal(4, 0, "NoPath"));
    }

    fn encode_inspect_bytes(nav: &InspectNav, tick: u64, hold: bool) -> Vec<u8> {
        use script::isolate_fb::{
            encode_snapshot_with_native, NativeFactsInput, ReachViewInput, RouteInspectFactsInput,
            SnapshotInput,
        };
        let latest_hops = nav
            .latest
            .as_ref()
            .map(|t| hop_inputs(&t.hops))
            .unwrap_or_default();
        let prev_hops = nav
            .prev
            .as_ref()
            .map(|t| hop_inputs(&t.hops))
            .unwrap_or_default();
        let latest_in = nav
            .latest
            .as_ref()
            .map(|t| terminal_to_input(t, &latest_hops))
            .unwrap_or_default();
        let prev_in = nav
            .prev
            .as_ref()
            .map(|t| terminal_to_input(t, &prev_hops))
            .unwrap_or_default();
        let posted = nav.posted();
        encode_snapshot_with_native(
            &SnapshotInput {
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
                hold,
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
            },
            NativeFactsInput {
                route_inspect: RouteInspectFactsInput {
                    latest: latest_in,
                    prev: prev_in,
                    running_id: posted.running_id,
                    pending_id: posted.pending_id,
                    accepted_id: posted.accepted_id,
                    replaced_id: posted.replaced_id,
                    replaced_prev_id: posted.replaced_prev_id,
                    refused_id: posted.refused_id,
                    refused_id_2: posted.refused_id_2,
                    refused_id_3: posted.refused_id_3,
                    unobserved: posted.unobserved,
                },
                ..Default::default()
            },
        )
    }

    fn inspect_begin(iso: &script::LoadIsolate) -> u64 {
        inspect_begin_at(iso, tile(0, 0, 0), tile(1, 1, 0))
    }

    fn inspect_begin_at(iso: &script::LoadIsolate, from: WorldTile, to: WorldTile) -> u64 {
        iso.probe(&format!(
            "globalThis.rustyscript.functions.__rs2b0t_inspect({{op:'begin',from:{{x:{},z:{},level:{}}},to:{{x:{},z:{},level:{}}},allow_wilderness:true}})",
            from.x, from.z, from.level, to.x, to.z, to.level
        ))
        .unwrap()
        .as_u64()
        .expect("token")
    }

    fn inspect_settled(iso: &script::LoadIsolate, token: u64) -> bool {
        iso.probe(&format!(
            "globalThis.rustyscript.functions.__rs2b0t_inspect({{op:'settled',token:{token}}})"
        ))
        .unwrap()
        .as_bool()
        .unwrap_or(false)
    }

    fn inspect_value(iso: &script::LoadIsolate, token: u64) -> serde_json::Value {
        iso.probe(&format!(
            "globalThis.rustyscript.functions.__rs2b0t_inspect({{op:'value',token:{token}}})"
        ))
        .unwrap()
    }

    fn drain_inspect_acks(iso: &script::LoadIsolate) -> Vec<(u64, u64)> {
        iso.drain_interacts()
            .into_iter()
            .filter_map(|req| match req {
                script::shim::InteractReq::InspectAck { seq, generation } => {
                    Some((seq, generation))
                }
                _ => None,
            })
            .collect()
    }

    fn isolate_begin_and_authorize(
        iso: &script::LoadIsolate,
        from: WorldTile,
        to: WorldTile,
        tick: u64,
    ) -> Option<u64> {
        let token = inspect_begin_at(iso, from, to);
        if inspect_settled(iso, token) {
            return None;
        }
        iso.probe(&format!(
            "globalThis.__rs_api.request({{op:'inspect-route',from:{{x:{},z:{},level:{}}},to:{{x:{},z:{},level:{}}},allow_wilderness:true,request_id:{token}}})",
            from.x, from.z, from.level, to.x, to.z, to.level
        ))
        .expect("inspect-route request");
        iso.on_game_tick(tick);
        let _ = iso.probe("true").expect("tick flush");
        let forwarded = iso.drain_interacts().into_iter().any(|req| {
            matches!(
                req,
                script::shim::InteractReq::InspectRoute { request_id, .. } if request_id == token
            )
        });
        forwarded.then_some(token)
    }

    fn inspect_force_timeout(iso: &script::LoadIsolate, token: u64) {
        iso.probe(&format!(
            "globalThis.rustyscript.functions.__rs2b0t_inspect({{op:'value',token:{token},timed_out:true}})"
        ))
        .unwrap();
    }

    fn apply_isolate_acks(navs: &Arc<Mutex<HashMap<String, NavBot>>>, iso: &script::LoadIsolate) {
        let acks = drain_inspect_acks(iso);
        let mut all = navs.lock().unwrap();
        let bot = all.get_mut("p").unwrap();
        for (seq, generation) in acks {
            bot.inspect.apply_ack(seq, generation);
        }
    }

    fn post_and_apply(
        iso: &script::LoadIsolate,
        navs: &Arc<Mutex<HashMap<String, NavBot>>>,
        tick: u64,
    ) {
        iso.post_snapshot(encode_inspect_bytes(
            &navs.lock().unwrap().get("p").unwrap().inspect,
            tick,
            false,
        ));
        let _ = iso.probe("true").unwrap();
        apply_isolate_acks(navs, iso);
    }

    #[test]
    fn integrated_host_publish_bytes_isolate_ack_without_new_request() {
        clear_barrier();
        let src = r#"
export const apiVersion = 2;
export function tick() {}
"#;
        let iso =
            script::LoadIsolate::spawn(src.into(), script::LoadShape::NativeTick, vec![]).unwrap();
        let world = world_with(TransportGraph::default(), vec![]);
        let navs = Arc::new(Mutex::new(HashMap::new()));

        let a = inspect_begin_at(&iso, tile(0, 0, 0), tile(1, 1, 0));
        let b = inspect_begin_at(&iso, tile(0, 0, 0), tile(2, 2, 0));
        let barrier = InspectBarrier::new();
        install_barrier(Arc::clone(&barrier));
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(1, 1, 0), a),
        );
        barrier.wait_entered();
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(2, 2, 0), b),
        );
        let c = inspect_begin_at(&iso, tile(0, 0, 0), tile(3, 3, 0));
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(3, 3, 0), c),
        );
        {
            let all = navs.lock().unwrap();
            let bot = all.get("p").unwrap();
            assert_eq!(bot.inspect.running_id, a);
            assert_eq!(bot.inspect.pending_id, c);
            assert_eq!(bot.inspect.replaced_id, b);
            assert_eq!(bot.inspect.latest.as_ref().unwrap().request_id, b);
            assert_eq!(bot.inspect.latest.as_ref().unwrap().reason, "stale");
            assert!(bot.inspect.worker.is_some());
        }
        iso.post_snapshot(encode_inspect_bytes(
            &navs.lock().unwrap().get("p").unwrap().inspect,
            1,
            false,
        ));
        let _ = iso.probe("true").unwrap();
        assert!(inspect_settled(&iso, b));
        assert_eq!(inspect_value(&iso, b)["reason"], "stale");
        assert!(
            !inspect_settled(&iso, a),
            "A must not stale before own drain"
        );
        assert!(!inspect_settled(&iso, c));
        let acks = drain_inspect_acks(&iso);
        assert!(
            !acks.is_empty(),
            "ACK must flush without a new inspect request"
        );
        {
            let mut all = navs.lock().unwrap();
            let bot = all.get_mut("p").unwrap();
            for (seq, generation) in acks {
                bot.inspect.apply_ack(seq, generation);
            }
        }
        barrier.release();
        let _ = wait_latest(&navs, a);
        let _ = wait_latest(&navs, c);
        iso.post_snapshot(encode_inspect_bytes(
            &navs.lock().unwrap().get("p").unwrap().inspect,
            2,
            false,
        ));
        let _ = iso.probe("true").unwrap();
        assert!(inspect_settled(&iso, a));
        assert!(inspect_settled(&iso, c));
        assert_eq!(inspect_value(&iso, a)["request_id"], a);
        assert_eq!(inspect_value(&iso, c)["request_id"], c);
        apply_isolate_acks(&navs, &iso);
        assert!(navs
            .lock()
            .unwrap()
            .get("p")
            .unwrap()
            .inspect
            .worker
            .is_none());

        clear_barrier();
        let sustain = InspectBarrier::new();
        install_barrier(Arc::clone(&sustain));
        let d = inspect_begin_at(&iso, tile(0, 0, 0), tile(4, 4, 0));
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(4, 4, 0), d),
        );
        sustain.wait_entered();
        let e = inspect_begin_at(&iso, tile(0, 0, 0), tile(5, 5, 0));
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(5, 5, 0), e),
        );
        {
            let all = navs.lock().unwrap();
            let bot = all.get("p").unwrap();
            assert_eq!(bot.inspect.running_id, d);
            assert_eq!(bot.inspect.pending_id, e);
            assert!(
                bot.inspect.can_admit(false) || bot.inspect.pending.is_some(),
                "fully acked ring must still admit running+pending"
            );
        }
        sustain.release();
        let _ = wait_latest(&navs, d);
        let _ = wait_latest(&navs, e);
        post_and_apply(&iso, &navs, 3);
        assert!(inspect_settled(&iso, d));
        assert!(inspect_settled(&iso, e));

        clear_barrier();
        let f = inspect_begin_at(&iso, tile(0, 0, 0), tile(1, 2, 0));
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(1, 2, 0), f),
        );
        let _ = wait_latest(&navs, f);
        let g = inspect_begin_at(&iso, tile(0, 0, 0), tile(1, 3, 0));
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(1, 3, 0), g),
        );
        let _ = wait_latest(&navs, g);
        let h = inspect_begin_at(&iso, tile(0, 0, 0), tile(1, 4, 0));
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(1, 4, 0), h),
        );
        let _ = wait_latest(&navs, h);
        {
            let all = navs.lock().unwrap();
            let bot = all.get("p").unwrap();
            assert_eq!(bot.inspect.held.as_ref().unwrap().request_id, h);
        }
        iso.post_snapshot(encode_inspect_bytes(
            &navs.lock().unwrap().get("p").unwrap().inspect,
            4,
            false,
        ));
        let _ = iso.probe("true").unwrap();
        assert!(inspect_settled(&iso, f));
        assert!(inspect_settled(&iso, g));
        assert!(!inspect_settled(&iso, h), "held is not posted until ack");
        let flush_acks = drain_inspect_acks(&iso);
        assert!(!flush_acks.is_empty());
        {
            let mut all = navs.lock().unwrap();
            let bot = all.get_mut("p").unwrap();
            for (seq, generation) in flush_acks {
                bot.inspect.apply_ack(seq, generation);
            }
            assert!(bot.inspect.held.is_none());
            assert_eq!(bot.inspect.latest.as_ref().unwrap().request_id, h);
        }
        iso.post_snapshot(encode_inspect_bytes(
            &navs.lock().unwrap().get("p").unwrap().inspect,
            5,
            false,
        ));
        let _ = iso.probe("true").unwrap();
        assert!(inspect_settled(&iso, h));
        apply_isolate_acks(&navs, &iso);

        let held_before_id0 = navs
            .lock()
            .unwrap()
            .get("p")
            .unwrap()
            .inspect
            .latest
            .as_ref()
            .map(|t| t.request_id);
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(6, 6, 0), 0),
        );
        let zero = wait_latest(&navs, 0);
        assert_eq!(zero.request_id, 0);
        assert_ne!(zero.seq, 0);
        assert!(!inspect_settled(&iso, 0));
        post_and_apply(&iso, &navs, 6);
        let _ = held_before_id0;

        let before_invented = navs.lock().unwrap().get("p").unwrap().inspect.accepted_id;
        iso.probe(
            "globalThis.__rs_api.request({op:'inspect-route',from:{x:0,z:0,level:0},to:{x:1,z:1,level:0},request_id:99})",
        )
        .unwrap();
        iso.on_game_tick(7);
        let leaked = iso.drain_interacts().into_iter().any(|req| {
            matches!(
                req,
                script::shim::InteractReq::InspectRoute { request_id: 99, .. }
            )
        });
        assert!(!leaked, "invented token must not reach the host queue");
        assert!(inspect_settled(&iso, 99));
        assert_eq!(
            navs.lock().unwrap().get("p").unwrap().inspect.accepted_id,
            before_invented
        );

        let hold_tok = inspect_begin(&iso);
        iso.post_snapshot(encode_inspect_bytes(
            &navs.lock().unwrap().get("p").unwrap().inspect,
            8,
            true,
        ));
        let _ = iso.probe("true").unwrap();
        assert!(!inspect_settled(&iso, hold_tok));

        clear_barrier();
        let reset_barrier = InspectBarrier::new();
        install_barrier(Arc::clone(&reset_barrier));
        let live = inspect_begin_at(&iso, tile(0, 0, 0), tile(7, 7, 0));
        queue_inspect(
            &navs,
            "p",
            &Some(world),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(7, 7, 0), live),
        );
        reset_barrier.wait_entered();
        let stale_ack = {
            let all = navs.lock().unwrap();
            let bot = all.get("p").unwrap();
            (
                bot.inspect.latest.as_ref().map(|t| t.seq).unwrap_or(1),
                bot.inspect.generation,
            )
        };
        {
            let mut all = navs.lock().unwrap();
            reset_inspect(all.get_mut("p").unwrap());
            let bot = all.get_mut("p").unwrap();
            assert!(bot.inspect.worker.is_some());
            assert_eq!(
                Arc::strong_count(bot.inspect.worker.as_ref().unwrap()),
                2,
                "live worker token stays after reset"
            );
            bot.inspect.apply_ack(stale_ack.0, stale_ack.1);
            assert_eq!(bot.inspect.observed_seq, 0);
            assert!(bot.inspect.latest.is_none());
        }
        reset_barrier.release();
        clear_barrier();
        iso.join();
    }

    fn spawn_native_iso() -> script::LoadIsolate {
        script::LoadIsolate::spawn(
            r#"
export const apiVersion = 2;
export function tick() {}
"#
            .into(),
            script::LoadShape::NativeTick,
            vec![],
        )
        .unwrap()
    }

    #[test]
    fn integrated_id0_full_then_authorized_registered_refuses_not_timeout() {
        let iso = spawn_native_iso();
        let world = world_with(TransportGraph::default(), vec![]);
        let navs = Arc::new(Mutex::new(HashMap::new()));
        fill_three_id0(&navs, &world);
        let ring_before = {
            let all = navs.lock().unwrap();
            let bot = all.get("p").unwrap();
            (
                bot.inspect.latest.as_ref().map(|t| t.seq),
                bot.inspect.prev.as_ref().map(|t| t.seq),
                bot.inspect.held.as_ref().map(|t| t.seq),
                bot.inspect.accepted_id,
            )
        };
        let dest = tile(2, 3, 1);
        let token = isolate_begin_and_authorize(&iso, tile(0, 0, 0), dest, 1)
            .expect("authorize must succeed when isolate has not seen host fullness");
        queue_inspect(
            &navs,
            "p",
            &Some(world),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), dest, token),
        );
        {
            let all = navs.lock().unwrap();
            let bot = all.get("p").unwrap();
            assert_eq!(bot.inspect.latest.as_ref().map(|t| t.seq), ring_before.0);
            assert_eq!(bot.inspect.prev.as_ref().map(|t| t.seq), ring_before.1);
            assert_eq!(bot.inspect.held.as_ref().map(|t| t.seq), ring_before.2);
            assert_eq!(bot.inspect.accepted_id, ring_before.3);
            assert!(bot.inspect.refused.contains(&token));
            assert!(bot.inspect.pending.is_none());
        }
        iso.post_snapshot(encode_inspect_bytes(
            &navs.lock().unwrap().get("p").unwrap().inspect,
            2,
            false,
        ));
        let _ = iso.probe("true").unwrap();
        assert!(inspect_settled(&iso, token));
        assert_eq!(inspect_value(&iso, token)["reason"], "stale");
        assert_ne!(inspect_value(&iso, token)["reason"], "waiter-timeout");
        iso.join();
    }

    #[test]
    fn integrated_timeout_orphans_then_authorized_registered_refuses_not_timeout() {
        let iso = spawn_native_iso();
        let world = world_with(TransportGraph::default(), vec![]);
        let navs = Arc::new(Mutex::new(HashMap::new()));
        let mut tokens = Vec::new();
        for (i, dest) in [tile(2, 2, 1), tile(2, 3, 1), tile(2, 4, 1)]
            .into_iter()
            .enumerate()
        {
            let token = isolate_begin_and_authorize(&iso, tile(0, 0, 0), dest, 10 + i as u64)
                .expect("registered begin while isolate unsettled < 3");
            queue_inspect(
                &navs,
                "p",
                &Some(Arc::clone(&world)),
                Some(WorldState::empty()),
                vec![],
                None,
                None,
                req(tile(0, 0, 0), dest, token),
            );
            wait_idle(&navs);
            tokens.push(token);
        }
        {
            let all = navs.lock().unwrap();
            let bot = all.get("p").unwrap();
            assert_eq!(bot.inspect.unobserved_count(), 3);
            assert!(!bot.inspect.can_admit(false));
        }
        for token in &tokens {
            inspect_force_timeout(&iso, *token);
            assert_eq!(inspect_value(&iso, *token)["reason"], "waiter-timeout");
        }
        let dest = tile(2, 5, 1);
        let next = isolate_begin_and_authorize(&iso, tile(0, 0, 0), dest, 20)
            .expect("new begin after timeout; isolate has not observed host fullness");
        queue_inspect(
            &navs,
            "p",
            &Some(world),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), dest, next),
        );
        {
            let all = navs.lock().unwrap();
            let bot = all.get("p").unwrap();
            assert!(bot.inspect.refused.contains(&next));
            assert_eq!(bot.inspect.unobserved_count(), 3);
            assert_ne!(bot.inspect.accepted_id, next);
            assert_eq!(bot.inspect.held.as_ref().unwrap().request_id, tokens[2]);
        }
        iso.post_snapshot(encode_inspect_bytes(
            &navs.lock().unwrap().get("p").unwrap().inspect,
            21,
            false,
        ));
        let _ = iso.probe("true").unwrap();
        assert!(inspect_settled(&iso, next));
        assert_eq!(inspect_value(&iso, next)["reason"], "stale");
        assert_ne!(inspect_value(&iso, next)["reason"], "waiter-timeout");
        iso.join();
    }

    #[test]
    fn integrated_mixed_id0_registered_ack_progress_does_not_starve() {
        let iso = spawn_native_iso();
        let world = world_with(TransportGraph::default(), vec![]);
        let navs = Arc::new(Mutex::new(HashMap::new()));
        for i in 0..6u64 {
            queue_inspect(
                &navs,
                "p",
                &Some(Arc::clone(&world)),
                Some(WorldState::empty()),
                vec![],
                None,
                None,
                req(tile(0, 0, 0), tile(2, 2, 1), 0),
            );
            wait_idle(&navs);
            assert_eq!(
                navs.lock()
                    .unwrap()
                    .get("p")
                    .unwrap()
                    .inspect
                    .latest
                    .as_ref()
                    .map(|t| t.request_id),
                Some(0),
                "id0 stays enabled under mixed traffic"
            );
            let dest = tile(2, 6 + i as i32, 1);
            let token = isolate_begin_and_authorize(&iso, tile(0, 0, 0), dest, 30 + i)
                .expect("ACK progress must keep registered begin authorizable");
            queue_inspect(
                &navs,
                "p",
                &Some(Arc::clone(&world)),
                Some(WorldState::empty()),
                vec![],
                None,
                None,
                req(tile(0, 0, 0), dest, token),
            );
            wait_idle(&navs);
            post_and_apply(&iso, &navs, 40 + i);
            assert!(inspect_settled(&iso, token));
            let value = inspect_value(&iso, token);
            assert_ne!(value["reason"], "waiter-timeout");
            assert_eq!(value["request_id"], token);
        }
        fill_three_id0(&navs, &world);
        post_and_apply(&iso, &navs, 49);
        post_and_apply(&iso, &navs, 50);
        let dest = tile(2, 20, 1);
        let after = isolate_begin_and_authorize(&iso, tile(0, 0, 0), dest, 51)
            .expect("ACK after id0 fill must admit registered traffic");
        queue_inspect(
            &navs,
            "p",
            &Some(world),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), dest, after),
        );
        wait_idle(&navs);
        post_and_apply(&iso, &navs, 52);
        assert!(inspect_settled(&iso, after));
        assert_ne!(inspect_value(&iso, after)["reason"], "waiter-timeout");
        assert_eq!(inspect_value(&iso, after)["request_id"], after);
        iso.join();
    }
}
