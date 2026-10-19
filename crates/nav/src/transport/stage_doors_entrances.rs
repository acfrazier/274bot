//! Loc entrances whose content performs a deterministic sequence of door
//! crossings and teleports. Every live-NPC lookup is universally quantified;
//! a possible refusal or different landing rejects that state alternative.
use super::*;

const MAX_ALTERNATIVES: usize = 16_384;
const MAX_READ_BLOCKS: usize = 64;

#[derive(Clone, Copy)]
pub(super) struct Inputs<'a> {
    facts: &'a Facts,
    positions: &'a HashMap<i32, Vec<Placement>>,
    absent: &'a [i32],
    npc_queries: &'a [Expr],
    npc_mask: usize,
    complete: bool,
}

#[derive(Default)]
struct Reads {
    items: Vec<i32>,
    npc: Vec<Expr>,
    quests: HashSet<String>,
    seen: HashSet<String>,
    valid: bool,
}

impl Reads {
    fn expression(&mut self, expr: &Expr, src: &Sources) {
        match expr {
            Expr::Word(name) if name.starts_with('%') => {
                if src.quest_complete.contains_key(&name[1..]) { self.quests.insert(name.clone()); }
            }
            Expr::Word(name) if name.starts_with('~') => self.procedure(&name[1..], src),
            Expr::Call(name, args) => {
                if name == "inv_total" {
                    if let [Expr::Word(inv), Expr::Word(obj)] = args.as_slice() {
                        if matches!(inv.as_str(), "inv" | "worn") {
                            if let Some(&id) = src.objs.get(obj) { self.items.push(id); }
                        }
                    }
                }
                if name == "npc_find" && !self.npc.contains(expr) { self.npc.push(expr.clone()); }
                if let Some(name) = name.strip_prefix('~') { self.procedure(name, src); }
                for arg in args { self.expression(arg, src); }
            }
            Expr::Cmp(_, a, b) | Expr::And(a, b) | Expr::Or(a, b) | Expr::Arithmetic(_, a, b) => {
                self.expression(a, src); self.expression(b, src);
            }
            _ => {}
        }
    }

    fn procedure(&mut self, name: &str, src: &Sources) {
        if !self.seen.insert(format!("~{name}")) { return; }
        if self.seen.len() > MAX_READ_BLOCKS { self.valid = false; return; }
        if let Some(block) = proc_block(src, name) { self.body(&block.body, src); }
    }

    fn body(&mut self, body: &[Stmt], src: &Sources) {
        for stmt in body {
            match stmt {
                Stmt::If(arms, other) => {
                    for (test, body) in arms { self.expression(test, src); self.body(body, src); }
                    if let Some(body) = other { self.body(body, src); }
                }
                Stmt::Block(body) => self.body(body, src),
                Stmt::While(test, body) => { self.expression(test, src); self.body(body, src); }
                Stmt::Switch(test, arms) => {
                    self.expression(test, src);
                    for (keys, body) in arms { for key in keys { self.expression(key, src); } self.body(body, src); }
                }
                Stmt::Def(_, Some(value)) | Stmt::Assign(_, value) | Stmt::ReturnValue(value) => self.expression(value, src),
                Stmt::Jump(name, args) => {
                    for arg in args { self.expression(arg, src); }
                    if self.seen.insert(format!("@{name}")) {
                        if self.seen.len() > MAX_READ_BLOCKS { self.valid = false; return; }
                        if let Lookup::Found(block) = Sources::one(&src.labels, name) { self.body(&block.body, src); }
                    }
                }
                Stmt::Call(name, args) => {
                    if let Some(name) = name.strip_prefix('~') { self.procedure(name, src); }
                    for arg in args { self.expression(arg, src); }
                    if name == "settimer" {
                        if let Some(Expr::Word(timer)) = args.first() {
                            if self.seen.insert(format!("timer:{timer}")) {
                                if let Lookup::Found(block) = Sources::one(&src.timers, timer) { self.body(&block.body, src); }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// A progress-only write below completion preserves every supported quest
/// predicate. Other stage predicates are deliberately not interpreted here.
pub(super) fn preserves_completion(stmt: &Stmt, src: &Sources) -> bool {
    let Stmt::If(arms, None) = stmt else { return false; };
    let [(Expr::Cmp(CmpOp::Eq, test, before), body)] = arms.as_slice() else { return false; };
    let Expr::Word(varp) = test.as_ref() else { return false; };
    let Some((_, complete)) = varp.strip_prefix('%').and_then(|name| src.quest_complete.get(name)) else { return false; };
    let scalar = |expr: &Expr| match expr {
        Expr::Num(value) => Some(*value),
        Expr::Word(name) => name.strip_prefix('^').and_then(|name| src.constants.get(name)).copied(),
        _ => None,
    };
    if !scalar(before).is_some_and(|before| before < *complete) { return false; }
    !body.is_empty() && body.iter().all(|stmt| matches!(stmt, Stmt::Assign(names, value)
        if names.as_slice() == [varp.clone()] && scalar(value).is_some_and(|after| after < *complete)))
}

impl Motion<'_> {
    pub(super) fn entrance_value(&mut self, expr: &Expr, env: &Env) -> Option<Option<Val>> {
        let input = self.entrance?;
        let (name, args) = match expr {
            Expr::Word(name) if name.starts_with('~') => (name.as_str(), &[][..]),
            Expr::Call(name, args) => (name.as_str(), args.as_slice()),
            _ => return None,
        };
        match name {
            "inv_getobj" => Some((|| {
                let [Expr::Word(inv), slot] = args else { return None; };
                if inv != "worn" { return None; }
                let Val::Int(slot) = self.value(slot, env)? else { return None; };
                Some(Val::WornSlot(slot))
            })()),
            "oc_category" => Some((|| {
                let [obj] = args else { return None; };
                let Val::WornSlot(slot) = self.value(obj, env)? else { return None; };
                Some(Val::WornCategory(slot))
            })()),
            "npc_find" => Some(input.npc_queries.iter().position(|query| query == expr)
                .map(|index| Val::Bool(input.npc_mask & (1usize << index) != 0))),
            "loc_find" => Some((|| {
                let [coord, Expr::Word(name)] = args else { return None; };
                let Val::Coord(coord) = self.value(coord, env)? else { return None; };
                let id = *self.base.src.ids.get(name)?;
                let mut matches = input.positions.get(&id).into_iter().flatten()
                    .filter(|placement| placement.x == coord.x && placement.z == coord.z && placement.level == coord.level);
                let Some(placement) = matches.next() else { return Some(Val::Bool(false)); };
                if matches.next().is_some() { return None; }
                self.base.at = coord;
                self.base.angle = placement.angle;
                self.loc_id = id;
                self.shape = placement.shape;
                self.loc_modified = false;
                self.facts = input.facts.locs.get(&id)?.as_ref()?;
                Some(Val::Bool(true))
            })()),
            name if name.starts_with('~') && !matches!(name, "~check_axis" | "~check_axis_locactive") => {
                let Some(block) = proc_block(self.base.src, &name[1..]) else { return Some(None); };
                if !self.base.src.proc_non_moving(&name[1..]) { return Some(None); }
                let saved = self.returned.take();
                let end = self.invoke(block, args, env);
                let value = self.returned.take();
                self.returned = saved;
                Some((end != End::Refused).then_some(value).flatten())
            }
            _ => None,
        }
    }

    pub(super) fn entrance_compare(&mut self, op: CmpOp, lhs: &Expr, rhs: &Expr, env: &Env) -> Option<Cond> {
        let input = self.entrance?;
        if let Expr::Word(varp) = lhs {
            if varp.starts_with('%') {
                let Some((quest, complete)) = self.base.src.quest_complete.get(&varp[1..]) else { return Some(Cond::unknown()); };
                let Some(Val::Int(value)) = self.value(rhs, env) else { return Some(Cond::unknown()); };
                if value != *complete || !matches!(op, CmpOp::Eq | CmpOp::Ne) { return Some(Cond::unknown()); }
                return Some(Cond {
                    value: Some(input.complete == (op == CmpOp::Eq)),
                    needs: if input.complete { Needs {
                        quests: vec![api::selected::QuestGate::Complete(quest.clone())],
                        quest_family: self.base.src.quest_family,
                        ..Needs::default()
                    } } else { Needs::default() },
                });
            }
        }
        if let Expr::Call(name, args) = lhs {
            if name == "inv_total" {
                let [Expr::Word(inv), Expr::Word(obj)] = args.as_slice() else { return Some(Cond::unknown()); };
                let Some(&id) = self.base.src.objs.get(obj) else { return Some(Cond::unknown()); };
                if input.absent.contains(&id) {
                    let Some(Val::Int(value)) = self.value(rhs, env) else { return Some(Cond::unknown()); };
                    let value = match op { CmpOp::Eq => value == 0, CmpOp::Ne => value != 0, CmpOp::Lt => 0 < value,
                        CmpOp::Le => 0 <= value, CmpOp::Gt => 0 > value, CmpOp::Ge => 0 >= value };
                    let mut needs = Needs::default();
                    match inv.as_str() { "inv" => needs.forbidden_held.push(id), "worn" => needs.forbidden_worn.push(id), _ => return Some(Cond::unknown()) }
                    return Some(Cond { value: Some(value), needs });
                }
                return None;
            }
        }
        let Some(left) = self.value(lhs, env) else { return None; };
        let Some(right) = self.value(rhs, env) else { return None; };
        match (left, right) {
            (Val::WornCategory(slot), Val::Name(category)) if matches!(op, CmpOp::Eq | CmpOp::Ne) => {
                let mut forbidden: Vec<_> = self.base.src.worn_slots.iter().filter_map(|(&id, &worn_slot)| {
                    if worn_slot != slot { return None; }
                    let excluded = match input.facts.categories.get(&id) {
                        Some(Some(actual)) => (actual == &category) == (op == CmpOp::Eq),
                        Some(None) => true,
                        None => op == CmpOp::Ne,
                    };
                    excluded.then_some(id)
                }).collect();
                forbidden.sort_unstable();
                Some(Cond { value: Some(false), needs: Needs { forbidden_worn: forbidden, ..Needs::default() } })
            }
            (Val::WornSlot(_), Val::Name(name)) if name == "null" => Some(Cond::unknown()),
            _ => None,
        }
    }

    pub(super) fn entrance_call(&mut self, name: &str, args: &[Expr], env: &Env) -> Option<End> {
        let proc = name.strip_prefix('~');
        if self.timer.is_some() && (matches!(name, "p_teleport" | "p_telejump" | "p_exactmove")
            || proc.is_some_and(|name| OPEN_PROCS.iter().any(|spec| spec.name == name))) {
            return Some(End::Refused);
        }
        if let Some(proc) = proc.filter(|name| OPEN_PROCS.iter().any(|spec| spec.name == *name)) {
            self.base.stand = Some(self.here);
            let Flow::Crossed(Arrival::Door(_), _) = self.base.open(proc, args, env) else { return Some(End::Refused); };
            let Some(dir) = door_dir(self.base.angle) else { return Some(End::Refused); };
            let (dx, dz) = dir_delta(dir);
            let to = if self.here == self.base.at { WorldTile { x: self.base.at.x + dx, z: self.base.at.z + dz, level: self.base.at.level } } else { self.base.at };
            return Some(if self.relocate(to).is_some() { self.moved = true; End::Continue } else { End::Refused });
        }
        if name == "settimer" {
            let [Expr::Word(timer), delay] = args else { return Some(End::Refused); };
            let valid_delay = match delay {
                Expr::Num(delay) => *delay > 0,
                Expr::Call(name, args) if name == "~random_range" => matches!(args.as_slice(), [Expr::Num(min), Expr::Num(max)] if *min > 0 && min <= max),
                _ => false,
            };
            if !valid_delay { return Some(End::Refused); }
            if let Some(active) = self.timer { return Some(if active == timer { End::Continue } else { End::Refused }); }
            let Some((timer, _)) = self.base.src.timers.get_key_value(timer) else { return Some(End::Refused); };
            let Lookup::Found(block) = Sources::one(&self.base.src.timers, timer) else { return Some(End::Refused); };
            self.timer = Some(timer);
            let end = self.run(&block.body, &mut Env::new());
            self.timer = None;
            return Some(if end == End::Refused { end } else { End::Continue });
        }
        None
    }
}

struct Trajectory { to: WorldTile, ticks: i32, needs: Needs }

fn normalize(needs: &mut Needs) -> bool {
    needs.items = max_per_key(&needs.items);
    needs.worn = max_per_key(&needs.worn);
    for ids in [&mut needs.forbidden_held, &mut needs.forbidden_worn] { ids.sort_unstable(); ids.dedup(); }
    !needs.items.iter().any(|(id, count)| *count > 0 && needs.forbidden_held.contains(id))
        && !needs.worn.iter().any(|(id, count)| *count > 0 && needs.forbidden_worn.contains(id))
}

fn weaker_than(weaker: &Needs, stronger: &Needs) -> bool {
    let positive = |a: &[(i32, i32)], b: &[(i32, i32)]| a.iter().all(|(id, count)| b.iter().any(|(other, n)| id == other && n >= count));
    positive(&weaker.items, &stronger.items) && positive(&weaker.worn, &stronger.worn)
        && positive(&weaker.skills, &stronger.skills) && positive(&weaker.varps, &stronger.varps)
        && weaker.forbidden_held.iter().all(|id| stronger.forbidden_held.contains(id))
        && weaker.forbidden_worn.iter().all(|id| stronger.forbidden_worn.contains(id))
        && (!weaker.members || stronger.members)
}

#[expect(clippy::too_many_arguments, reason = "all inputs are immutable content and one placement")]
fn trajectory<'a>(src: &'a Sources, facts: &'a Facts, positions: &'a HashMap<i32, Vec<Placement>>,
    proofs: &'a HashSet<String>, supported: &'a HashSet<String>, selectors: &'a HashMap<String, Vec<Tool>>,
    handler: &Block, id: i32, placement: &Placement, stand: WorldTile, absent: &'a [i32], npc: &'a [Expr], complete: bool,
) -> Option<Trajectory> {
    let at = WorldTile { x: placement.x, z: placement.z, level: placement.level };
    let mut result: Option<Trajectory> = None;
    for npc_mask in 0..(1usize << npc.len()) {
        let mut motion = Motion {
            base: Eval { src, supported, at, stand: Some(stand), angle: placement.angle, needs: Needs::default(), relative_landing_safe: false, jumps: 0 },
            facts: facts.locs.get(&id)?.as_ref()?, proofs, selectors, tool: None, here: stand,
            loc_id: id, loc_modified: false, shape: placement.shape, moved: false, ticks: 1, gate_read: false,
            entrance: Some(Inputs { facts, positions, absent, npc_queries: npc, npc_mask, complete }), returned: None, timer: None,
        };
        if motion.run(&handler.body, &mut Env::new()) == End::Refused || !motion.moved || motion.here == stand { return None; }
        if let Some(result) = &mut result {
            if result.to != motion.here { return None; }
            result.ticks = result.ticks.max(motion.ticks);
            result.needs.merge(motion.base.needs);
        } else { result = Some(Trajectory { to: motion.here, ticks: motion.ticks, needs: motion.base.needs }); }
    }
    let mut result = result?;
    normalize(&mut result.needs).then_some(result)
}

#[expect(clippy::too_many_arguments, reason = "shares the immutable transport bake inputs")]
pub(super) fn edges(src: &Sources, positions: &HashMap<i32, Vec<Placement>>, defs: &LocDefs,
    graph: &mut TransportGraph, collision: &WorldCollision, skipped: &mut HashMap<&'static str, usize>,
    observable: &ObservableGates, audit: &mut VarpGateAudit, facts: &Facts, proofs: &HashSet<String>,
    supported: &HashSet<String>, selectors: &HashMap<String, Vec<Tool>>,
) {
    let covered: HashSet<_> = graph.edges.iter().filter(|edge| matches!(edge.kind, TransportKind::Door | TransportKind::Ladder | TransportKind::Stairs | TransportKind::AgilityShortcut))
        .map(|edge| (edge.loc_id, edge.option)).collect();
    let mut locs: Vec<_> = positions.keys().copied().collect(); locs.sort_unstable();
    for id in locs {
        let Some(Some(fact)) = facts.locs.get(&id) else { continue; };
        let Some(def) = defs.loc(id) else { continue; };
        for option in 1..=5 {
            if covered.contains(&(id, option)) { continue; }
            let own: Vec<_> = src.ids.iter().filter(|(_, value)| **value == id)
                .map(|(name, _)| Sources::one(&src.oplocs[(option - 1) as usize], name))
                .filter(|value| !matches!(value, Lookup::Missing)).collect();
            let handler = match own.as_slice() {
                [Lookup::Found(block)] => *block,
                [] => match fact.category.as_ref().map(|name| Sources::one(&src.oplocs[(option - 1) as usize], &format!("_{name}"))) {
                    Some(Lookup::Found(block)) => block, _ => continue,
                },
                _ => continue,
            };
            let calls = reachable_calls(handler, src);
            if !calls.iter().any(|call| {
                matches!(call.as_str(), "p_teleport" | "p_telejump")
                    || call.strip_prefix('~')
                        .is_some_and(|name| OPEN_PROCS.iter().any(|spec| spec.name == name))
            }) { continue; }
            let mut reads = Reads { valid: true, ..Reads::default() }; reads.body(&handler.body, src);
            reads.items.sort_unstable(); reads.items.dedup();
            let Some(bits) = reads.items.len().checked_add(reads.npc.len()) else { continue; };
            if !reads.valid || bits >= usize::BITS as usize || (1usize << bits) > MAX_ALTERNATIVES {
                bump(skipped, "loc entrance exceeds bounded content alternatives", 1); continue;
            }
            for placement in &positions[&id] {
                let at = WorldTile { x: placement.x, z: placement.z, level: placement.level };
                let mut stands = Vec::new();
                if placement.shape == 0 {
                    let Some(dir) = door_dir(placement.angle) else { continue; }; let (dx, dz) = dir_delta(dir);
                    stands.extend([at, WorldTile { x: at.x + dx, z: at.z + dz, level: at.level }]);
                } else if matches!(placement.shape, 10 | 11 | 22) {
                    let Some(approach) = LocApproach::from_loc_def(def.width, def.length, placement.angle, def.force_approach) else { continue; };
                    for x in at.x - 1..=at.x + i32::from(approach.width) { for z in at.z - 1..=at.z + i32::from(approach.length) {
                        let stand = WorldTile { x, z, level: at.level };
                        if approach.can_operate(at, stand, collision.walkable_word(x, z, stand.level) as i32) { stands.push(stand); }
                    } }
                }
                for stand in stands.into_iter().filter(|stand| collision.standable(*stand)) {
                    for mask in 0..(1usize << reads.items.len()) {
                        let absent: Vec<_> = reads.items.iter().enumerate().filter_map(|(index, &id)| (mask & (1usize << index) != 0).then_some(id)).collect();
                        let complete = trajectory(src, facts, positions, proofs, supported, selectors, handler, id, placement, stand, &absent, &reads.npc, true);
                        let incomplete = (reads.quests.len() <= 1).then(|| trajectory(src, facts, positions, proofs, supported, selectors, handler, id, placement, stand, &absent, &reads.npc, false)).flatten();
                        let incomplete = incomplete.filter(|candidate| complete.as_ref().is_some_and(|other| other.to == candidate.to && weaker_than(&other.needs, &candidate.needs)));
                        for result in [complete, incomplete].into_iter().flatten() {
                            if !collision.standable(result.to) { continue; }
                            let mut edge = gated_edge(TransportKind::Ladder, id, at, result.to, result.needs);
                            edge.takeoff = Some(stand); edge.option = option; edge.ticks = result.ticks;
                            if observable.admit_edge(graph, edge, audit) && src.has_member_reads(handler) { audit.record_member_path(graph.edges.last().expect("admitted edge")); }
                        }
                    }
                }
            }
        }
    }
}
