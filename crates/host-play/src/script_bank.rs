use api::interact::Driver;
use api::snapshot::{GameSnapshot, WorldTile};
use nav::world::NavWorld;
use api::named_banks::{BankPreferences, NamedBankFacts};
use nav::router::{find_many_with_avoid_bounded, FindOptions, BANK_TARGET_BUDGET};
use nav::WorldState;
use script::isolate_fb::BankSelectionInput;
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

/// Separate completion identity from movement. Reset invalidates a running
/// capture but retains its worker, so replacements cannot grow worker count.
#[derive(Default)]
pub(crate) struct BankPickState {
    generation: u64,
    last_id: u64,
    worker: Option<Arc<()>>,
    pending: Option<BankPickRequest>,
    current: Option<(Instant, BankSelectionInput)>,
    pub(crate) posted: BankSelectionInput,
    facts: Option<Arc<NamedBankFacts>>,
}

impl BankPickState {
    pub(crate) fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.current = None;
        self.pending = None;
        self.last_id = 0;
        self.posted = BankSelectionInput::default();
    }

    pub(crate) fn poll(&mut self, now: Instant) -> BankSelectionInput {
        if self.current.is_some_and(|(deadline, _)| now >= deadline) {
            self.posted = self.current.take().unwrap().1;
            self.pending = None;
            self.generation = self.generation.wrapping_add(1);
        }
        self.posted
    }

    fn complete(&mut self, request: &BankPickRequest, index: Option<usize>, kind: PickKind, now: Instant) {
        self.poll(now);
        if self.generation != request.generation || self.last_id != request.request_id || self.current.is_none() {
            return;
        }
        self.posted = request.result(index, kind);
        self.current = None;
    }
}

struct BankPickRequest {
    generation: u64,
    request_id: u64,
    world: Arc<NavWorld>,
    facts: Arc<NamedBankFacts>,
    from: WorldTile,
    opts: FindOptions,
    state: WorldState,
    order: Vec<usize>,
    #[cfg(test)]
    test_gate: Option<Arc<tests::Gate>>,
}

impl BankPickRequest {
    fn result(&self, index: Option<usize>, kind: PickKind) -> BankSelectionInput {
        BankSelectionInput {
            generation: self.generation, request_id: self.request_id,
            bank_index: index.map_or(-1, |i| i as i32), kind: kind as u8,
        }
    }

    fn calculate(&self) -> (Option<usize>, PickKind) {
        let routed: Vec<_> = self.order.iter().copied().filter(|&i| self.facts.banks()[i].routable).collect();
        let targets: Vec<_> = routed.iter().map(|&i| self.facts.banks()[i].tile).collect();
        let routes = find_many_with_avoid_bounded(
            &self.world.collision, &self.world.graph, self.from, &targets,
            self.opts, &self.state, &[], BANK_TARGET_BUDGET,
        );
        let mut best = None;
        let mut cost = f64::INFINITY;
        for (i, result) in routes.results().iter().enumerate() {
            if let Ok(result) = result {
                if result.ticks < cost {
                    cost = result.ticks;
                    best = Some(routed[i]);
                }
            }
        }
        match best {
            Some(index) => (Some(index), PickKind::Reachable),
            None => (self.order.first().copied(), PickKind::AirFallback),
        }
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
    let now = Instant::now();
    let mut all = navs.lock().unwrap();
    let bot = all.entry(name.to_string()).or_default();
    let pick = &mut bot.bank_pick;
    if request_id == 0 || request_id <= pick.last_id {
        return;
    }
    pick.generation = pick.generation.wrapping_add(1);
    pick.last_id = request_id;
    pick.pending = None;
    pick.current = None;
    let Some(world) = world else {
        pick.posted = BankSelectionInput { request_id, generation: pick.generation, kind: PickKind::NoCandidate as u8, ..Default::default() };
        return;
    };
    let facts = Arc::clone(pick.facts.get_or_insert_with(|| world.named_bank_facts(None)));
    let state = state.unwrap_or_default();
    let mut order: Vec<_> = (0..facts.banks().len()).filter(|&i| {
        facts.banks()[i].eligible(
            |id| if id == 10 { fishing_base } else { None },
            |quest| state.quests.contains(quest),
            preferences,
        )
    }).collect();
    order.sort_by_key(|&i| bank_air_distance(from, facts.banks()[i].air_tile()));
    let request = BankPickRequest {
        generation: pick.generation, request_id, world: Arc::clone(world), facts, from,
        opts: FindOptions { allow_wilderness, essence: bot.traveller.essence(), ..Default::default() },
        state, order,
        #[cfg(test)]
        test_gate: tests::capture_gate(),
    };
    let first = request.order.first().copied();
    if first.is_none() || first.is_some_and(|i| near_bank(from, request.facts.banks()[i].tile)) {
        pick.posted = request.result(first, if first.is_some() { PickKind::NearShortcut } else { PickKind::NoCandidate });
        return;
    }
    pick.current = Some((now + BANK_SELECTION_WINDOW, request.result(first, PickKind::AirFallback)));
    pick.pending = Some(request);
    if pick.worker.is_some() {
        return;
    }
    let token = Arc::new(());
    pick.worker = Some(Arc::clone(&token));
    drop(all);
    let worker_navs = Arc::clone(navs);
    let worker_name = name.to_string();
    let spawned = std::thread::Builder::new().name(format!("bank-pick-{name}")).spawn(move || {
        loop {
            let request = {
                let mut all = worker_navs.lock().unwrap();
                let Some(bot) = all.get_mut(&worker_name) else { return };
                let pick = &mut bot.bank_pick;
                if !pick.worker.as_ref().is_some_and(|live| Arc::ptr_eq(live, &token)) { return; }
                pick.poll(Instant::now());
                match pick.pending.take() {
                    Some(request) => request,
                    None => { pick.worker = None; return; }
                }
            };
            // Every flood, and the search tree's destruction, is off the slot lock.
            #[cfg(test)]
            if let Some(gate) = &request.test_gate { gate.enter(); }
            let (index, kind) = request.calculate();
            if let Some(bot) = worker_navs.lock().unwrap().get_mut(&worker_name) {
                bot.bank_pick.complete(&request, index, kind, Instant::now());
            }
            #[cfg(test)]
            if let Some(gate) = &request.test_gate { gate.finished(); }
        }
    });
    if spawned.is_err() {
        if let Some(bot) = navs.lock().unwrap().get_mut(name) {
            let pick = &mut bot.bank_pick;
            pick.worker = None;
            pick.pending = None;
            if let Some((_, fallback)) = pick.current.take() { pick.posted = fallback; }
        }
    }
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

pub(crate) fn nearest_bank_booth(
    world: &NavWorld,
    (x, z, level): (i32, i32, i32),
) -> Option<WorldTile> {
    world
        .banks()
        .iter()
        .filter(|stand| matches!(stand.access, nav::pack::BankAccess::Booth { .. }))
        .min_by_key(|stand| {
            let distance = stand.tile.x.abs_diff(x).max(stand.tile.z.abs_diff(z));
            if stand.tile.level == level {
                u64::from(distance)
            } else {
                (u64::MAX / 2).saturating_add(u64::from(distance))
            }
        })
        .map(|stand| stand.tile)
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

pub(super) fn open_bank_at_here<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    here: Option<(i32, i32, i32)>,
    world: Option<&NavWorld>,
) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    // Prefer a packed booth stand's tile; else any Use-quickly loc.
    let target_tile = world.and_then(|w| {
        here.and_then(|(hx, hz, hl)| {
            w.banks()
                .iter()
                .min_by_key(|s| {
                    (
                        s.tile.level != hl,
                        (s.tile.x - hx).abs().max((s.tile.z - hz).abs()),
                    )
                })
                .map(|s| (s.tile.x, s.tile.z, s.tile.level))
        })
    });
    if let Some((x, z, level)) = target_tile {
        if let Some(loc) = snapshot
            .locs()
            .iter()
            .find(|l| l.tile.x == x && l.tile.z == z && l.tile.level == level)
        {
            if let Some(op) = action_slot(&loc.actions, "Use-quickly") {
                return matches!(
                    ix.interact(OpTarget::Loc(loc), ActionSpec::Operation(op)),
                    SendResult::Sent { .. }
                );
            }
        }
    }
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
