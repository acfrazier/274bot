//! Inspect-specific off-pump job. One running calculation plus one latest
//! pending per profile. Distinct from walk: reset leaves `inspect.worker`,
//! Pause does not abort inspect, and results never arm Traveller.

use super::NavBot;
use api::obj_names::ObjNames;
use api::snapshot::WorldTile;
use client::config::Cache;
use nav::bank_fetch::{plan_bank_fetch, BankStep};
use nav::router::{
    find_missing_item_reqs_with_avoid, find_with_avoid, AvoidRect, FindOptions, Leg, Route,
    RouteError,
};
use nav::transport::TransportKind;
use nav::world::NavWorld;
use nav::world_state::WorldState;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

const MAX_AVOID: usize = 16;

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
}

#[derive(Default)]
pub(super) struct InspectNav {
    pub generation: u64,
    pub worker: Option<Arc<()>>,
    pub pending: Option<InspectCapture>,
    pub running_id: u64,
    pub pending_id: u64,
    pub accepted_id: u64,
    pub replaced_id: u64,
    pub replaced_prev_id: u64,
    pub latest: Option<InspectTerminal>,
    pub prev: Option<InspectTerminal>,
    pub held: Option<InspectTerminal>,
    pub ring_posted: bool,
    pub observed_seq: u64,
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
}

impl InspectNav {
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

    fn publish(&mut self, mut term: InspectTerminal) {
        if term.seq == 0 {
            term.seq = self.next_seq();
        }
        if self.latest.is_none() {
            self.latest = Some(term);
            self.ring_posted = false;
            return;
        }
        if self.prev.is_none() {
            self.prev = self.latest.take();
            self.latest = Some(term);
            self.ring_posted = false;
            return;
        }
        if !self.can_wrap() {
            if self.held.is_none() {
                self.held = Some(term);
            }
            return;
        }
        self.prev = self.latest.take();
        self.latest = Some(term);
        self.ring_posted = false;
    }

    fn can_wrap(&self) -> bool {
        let Some(prev) = self.prev.as_ref() else {
            return true;
        };
        self.ring_posted || self.observed_seq >= prev.seq
    }

    pub(super) fn mark_posted(&mut self) {
        self.ring_posted = true;
        if let Some(held) = self.held.take() {
            self.publish(held);
        }
    }

    pub(super) fn apply_ack(&mut self, seq: u64) {
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
        self.ring_posted = true;
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
    pub inspect_ack_seq: u64,
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
    {
        let mut all = navs.lock().unwrap();
        let bot = all.entry(name.to_string()).or_default();
        bot.inspect.apply_ack(req.inspect_ack_seq);
    }
    if req.invalid_args || validate_request(req.from, req.to, &req.avoid).is_err() {
        refuse(navs, name, req.request_id, "invalid-args");
        return;
    }
    let Some(world) = world.clone() else {
        refuse(navs, name, req.request_id, "missing-graph");
        return;
    };
    let token = {
        let mut all = navs.lock().unwrap();
        let bot = all.entry(name.to_string()).or_default();
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
        };
        bot.inspect.accepted_id = req.request_id;
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
                return;
            };
            bot.inspect.pending_id = 0;
            bot.inspect.running_id = request.request_id;
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
        let identity_ok = request.request_id != 0
            && request.generation == bot.inspect.generation
            && bot
                .inspect
                .live_world
                .as_ref()
                .is_some_and(|ptr| Arc::ptr_eq(&request.world, ptr))
            && request.request_id == bot.inspect.running_id;
        if identity_ok {
            bot.inspect.publish(terminal);
        } else if bot.inspect.running_id == request.request_id && bot.inspect.pending.is_none() {
            bot.inspect.publish(InspectTerminal::refusal(
                request.request_id,
                request.generation,
                "stale",
            ));
        }
        if bot.inspect.pending.is_none() {
            bot.inspect.worker = None;
            bot.inspect.running_id = 0;
            return;
        }
    }
}

fn refuse(navs: &Arc<Mutex<HashMap<String, NavBot>>>, name: &str, request_id: u64, reason: &str) {
    let mut all = navs.lock().unwrap();
    let bot = all.entry(name.to_string()).or_default();
    if request_id == 0 {
        return;
    }
    bot.inspect.accepted_id = request_id;
    bot.inspect
        .publish(InspectTerminal::refusal(request_id, bot.inspect.generation, reason));
}

pub(super) fn reset_inspect(bot: &mut NavBot) {
    bot.inspect.generation = bot.inspect.generation.wrapping_add(1);
    bot.inspect.pending = None;
    bot.inspect.pending_id = 0;
    bot.inspect.running_id = 0;
    bot.inspect.clear_published();
}

pub(super) fn calculate(capture: &InspectCapture) -> InspectTerminal {
    let pre = &capture.state;
    let avoid = capture.avoid.as_slice();
    let strict = find_with_avoid(
        &capture.world.collision,
        &capture.world.graph,
        capture.from,
        capture.to,
        capture.opts,
        pre,
        avoid,
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
    let Some(missing) = find_missing_item_reqs_with_avoid(
        &capture.world.collision,
        &capture.world.graph,
        capture.from,
        capture.to,
        capture.opts,
        pre,
        avoid,
    ) else {
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
        match find_with_avoid(
            &capture.world.collision,
            &capture.world.graph,
            capture.from,
            stand,
            FindOptions {
                allow_bank_fetch: false,
                ..capture.opts
            },
            pre,
            avoid,
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
    let post = find_with_avoid(
        &capture.world.collision,
        &capture.world.graph,
        capture.from,
        capture.to,
        FindOptions {
            allow_bank_fetch: false,
            ..capture.opts
        },
        &plan.state,
        avoid,
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
                npc.map(|n| n.name.clone()).filter(|n| !n.is_empty()).unwrap_or_default(),
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
                loc.map(|l| l.name.clone()).filter(|n| !n.is_empty()).unwrap_or_default(),
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
            op: [
                None,
                None,
                None,
                Some("Rub".into()),
                None,
            ],
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
            inspect_ack_seq: 0,
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
                }
            }
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "inspect {id} did not publish"
            );
            thread::yield_now();
        }
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
            &edge(TransportKind::Glider, tile(1, 1, 0), tile(2, 2, 0), 170, vec![]),
            Some(cache.as_ref()),
            Some(names.as_ref()),
        );
        assert_eq!(glider.kind, "glider");
        assert_eq!(glider.loc_name, "Captain Klemfoodle");
        let barnaby = project_hop(
            &edge(TransportKind::Boat, tile(1, 1, 0), tile(2, 2, 0), 381, vec![]),
            Some(cache.as_ref()),
            Some(names.as_ref()),
        );
        assert_eq!(barnaby.loc_name, "Captain Barnaby");
        let thresnor = project_hop(
            &edge(TransportKind::Boat, tile(1, 1, 0), tile(2, 2, 0), 378, vec![]),
            Some(cache.as_ref()),
            Some(names.as_ref()),
        );
        assert_eq!(thresnor.loc_name, "Seaman Thresnor");
        let plank = project_hop(
            &edge(TransportKind::Ladder, tile(1, 1, 0), tile(2, 2, 0), 2082, vec![]),
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
                ..edge(TransportKind::Teleport, tile(0, 0, 0), tile(1, 1, 0), 1712, vec![])
            },
            Some(cache.as_ref()),
            Some(names.as_ref()),
        );
        assert_eq!(jewellery.loc_name, "Glory");
        assert_eq!(jewellery.action, "Rub");
        let spell = project_hop(
            &edge(TransportKind::Teleport, tile(0, 0, 0), tile(1, 1, 0), 0, vec![]),
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
        graph.edges.push(edge(
            TransportKind::Boat,
            at,
            to,
            381,
            vec![(995, 30)],
        ));
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
        graph.edges.push(edge(
            TransportKind::Boat,
            from,
            dest,
            381,
            vec![(995, 10)],
        ));
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
        graph.edges.push(edge(
            TransportKind::Boat,
            from,
            dest,
            381,
            vec![(995, 10)],
        ));
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
        };
        let term = calculate(&capture);
        assert!(term.ok, "open walk to stand plus post boat should plan, got {}", term.reason);
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
    fn ring_holds_unacked_prev_instead_of_wrap() {
        let mut nav = InspectNav::default();
        nav.publish(InspectTerminal::refusal(1, 0, "NoPath"));
        nav.publish(InspectTerminal::refusal(2, 0, "NoPath"));
        nav.publish(InspectTerminal::refusal(3, 0, "NoPath"));
        assert_eq!(nav.latest.as_ref().unwrap().request_id, 2);
        assert_eq!(nav.prev.as_ref().unwrap().request_id, 1);
        assert_eq!(nav.held.as_ref().unwrap().request_id, 3);
        nav.mark_posted();
        assert_eq!(nav.latest.as_ref().unwrap().request_id, 3);
        assert_eq!(nav.prev.as_ref().unwrap().request_id, 2);
    }

    #[test]
    fn invalid_args_and_missing_graph() {
        let navs = Arc::new(Mutex::new(HashMap::new()));
        let mut bad = req(tile(0, 0, 9), tile(1, 1, 0), 51);
        bad.invalid_args = true;
        queue_inspect(&navs, "p", &None, None, vec![], None, None, bad);
        assert_eq!(
            navs.lock().unwrap().get("p").unwrap().inspect.latest.as_ref().unwrap().reason,
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
            navs.lock().unwrap().get("p").unwrap().inspect.latest.as_ref().unwrap().reason,
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
}
