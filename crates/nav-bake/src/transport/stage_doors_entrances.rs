//! Loc entrances whose content performs a deterministic sequence of door
//! crossings and teleports. Every live-NPC lookup is universally quantified;
//! a possible refusal or different landing rejects that state alternative.
use super::*;

// Bound the full item, NPC, quest-completion, and quest-stage product.
const MAX_ALTERNATIVES: usize = 65_536;
const MAX_READ_BLOCKS: usize = 64;

#[derive(Clone)]
struct StagePartition {
    varp: String,
    values: api::selected::InclusiveRange,
}

struct StageDimension {
    varp: String,
    ranges: Vec<api::selected::InclusiveRange>,
}

fn stage_scalar(expr: &Expr, src: &Sources) -> Option<i32> {
    match expr {
        Expr::Num(value) => Some(*value),
        Expr::Word(name) => name
            .strip_prefix('^')
            .and_then(|name| src.constants.get(name))
            .copied(),
        _ => None,
    }
}

// Exact stage partitions are sound only when every read has a bounded
// authoritative domain and every quest use is a static comparison.
fn stage_dimensions(reads: &Reads, src: &Sources) -> Result<Option<Vec<StageDimension>>, ()> {
    if reads.quests.is_empty() || !reads.stage_partitionable {
        return Ok(None);
    }
    let mut varps: Vec<_> = reads.quests.iter().collect();
    varps.sort_unstable();
    let mut dimensions = Vec::with_capacity(varps.len());
    for varp in varps {
        let Some(varp_name) = varp.strip_prefix('%') else {
            return Ok(None);
        };
        let Some((quest, complete)) = src.quest_complete.get(varp_name) else {
            return Ok(None);
        };
        if src.quest_signals.get(varp_name) != Some(quest) {
            return Ok(None);
        }
        let Some(facts) = src.quest_stage.get(varp_name).copied() else {
            return Ok(None);
        };
        let Some((min, max)) = authoritative_stage_domain(facts, *complete) else {
            return Ok(None);
        };
        let mut boundaries = vec![min];
        let thresholds = reads
            .stage_thresholds
            .get(varp)
            .into_iter()
            .flat_map(|values| values.iter().copied())
            .chain([*complete]);
        for threshold in thresholds {
            for boundary in [Some(threshold), threshold.checked_add(1)] {
                if let Some(boundary) = boundary.filter(|value| *value > min && *value <= max) {
                    boundaries.push(boundary);
                }
            }
        }
        boundaries.sort_unstable();
        boundaries.dedup();
        let mut ranges = Vec::with_capacity(boundaries.len());
        for (index, &range_min) in boundaries.iter().enumerate() {
            let range_max = boundaries.get(index + 1).map_or(max, |next| next - 1);
            ranges.push(api::selected::InclusiveRange {
                min: Some(range_min),
                max: Some(range_max),
            });
        }
        if ranges.len() > MAX_ALTERNATIVES {
            return Err(());
        }
        dimensions.push(StageDimension {
            varp: varp.clone(),
            ranges,
        });
    }
    Ok(Some(dimensions))
}

fn stage_variants(dimensions: &[StageDimension]) -> Vec<Vec<StagePartition>> {
    let mut variants = vec![Vec::new()];
    for dimension in dimensions {
        let mut next = Vec::with_capacity(variants.len() * dimension.ranges.len());
        for variant in &variants {
            for &values in &dimension.ranges {
                let mut next_variant = variant.clone();
                next_variant.push(StagePartition {
                    varp: dimension.varp.clone(),
                    values,
                });
                next.push(next_variant);
            }
        }
        variants = next;
    }
    variants
}

fn stage_partition<'a>(ranges: &'a [StagePartition], varp: &str) -> Option<&'a StagePartition> {
    ranges.iter().find(|range| range.varp == varp)
}

fn quest_gate(
    quest: &api::selected::FactKey,
    signal: &str,
    values: api::selected::InclusiveRange,
    complete: i32,
    domain_max: i32,
    colour_authoritative: bool,
) -> api::selected::QuestGate {
    if colour_authoritative
        && domain_max == complete
        && values.min == Some(complete)
        && values.max == Some(complete)
    {
        api::selected::QuestGate::Complete(quest.clone())
    } else {
        api::selected::QuestGate::Window(api::selected::StageWindow {
            quest: quest.clone(),
            signal: api::selected::FactKey::new(signal),
            values,
        })
    }
}

#[derive(Clone, Copy)]
pub(super) struct Inputs<'a> {
    facts: &'a Facts,
    positions: &'a HashMap<i32, Vec<Placement>>,
    absent: &'a [i32],
    npc_queries: &'a [Expr],
    npc_mask: usize,
    scenery_queries: &'a [Expr],
    scenery_mask: usize,
    complete: bool,
    stage_ranges: Option<&'a [StagePartition]>,
    above_queries: &'a [String],
    above_mask: usize,
}

#[derive(Default)]
struct Reads {
    items: Vec<i32>,
    npc: Vec<Expr>,
    scenery: Vec<Expr>,
    quests: HashSet<String>,
    stage_thresholds: HashMap<String, HashSet<i32>>,
    stage_partitionable: bool,
    seen: HashSet<String>,
    valid: bool,
}

impl Reads {
    fn expression(&mut self, expr: &Expr, src: &Sources) {
        match expr {
            Expr::Word(name) if name.starts_with('%') => {
                if src.quest_complete.contains_key(&name[1..])
                    || src.quest_signals.contains_key(&name[1..])
                {
                    self.quests.insert(name.clone());
                    self.stage_partitionable = false;
                }
            }
            Expr::Word(name) if name.starts_with('~') => self.procedure(&name[1..], src),
            Expr::Call(name, args) => {
                if name == "inv_total" {
                    if let [Expr::Word(inv), Expr::Word(obj)] = args.as_slice() {
                        if matches!(inv.as_str(), "inv" | "worn") {
                            if let Some(&id) = src.objs.get(obj) {
                                self.items.push(id);
                            }
                        }
                    }
                }
                if name == "npc_find" && !self.npc.contains(expr) {
                    self.npc.push(expr.clone());
                }
                if name == "loc_find" && !self.scenery.contains(expr) {
                    self.scenery.push(expr.clone());
                }
                if let Some(name) = name.strip_prefix('~') {
                    self.procedure(name, src);
                }
                for arg in args {
                    self.expression(arg, src);
                }
            }
            Expr::Cmp(_, a, b) => {
                let stage_varp = match a.as_ref() {
                    Expr::Word(varp)
                        if varp.starts_with('%') && src.quest_complete.contains_key(&varp[1..]) =>
                    {
                        Some(varp.clone())
                    }
                    _ => None,
                };
                if let Some(varp) = stage_varp {
                    self.quests.insert(varp.clone());
                    if let Some(threshold) = stage_scalar(b, src) {
                        let overflow = {
                            let thresholds = self.stage_thresholds.entry(varp).or_default();
                            if thresholds.contains(&threshold) {
                                false
                            } else if thresholds.len() >= MAX_ALTERNATIVES / 2 {
                                true
                            } else {
                                thresholds.insert(threshold);
                                false
                            }
                        };
                        self.valid &= !overflow;
                    } else {
                        self.stage_partitionable = false;
                    }
                    self.expression(b, src);
                } else {
                    self.expression(a, src);
                    self.expression(b, src);
                }
            }
            Expr::And(a, b) | Expr::Or(a, b) | Expr::Arithmetic(_, a, b) => {
                self.expression(a, src);
                self.expression(b, src);
            }
            _ => {}
        }
    }

    fn procedure(&mut self, name: &str, src: &Sources) {
        if !self.seen.insert(format!("~{name}")) {
            return;
        }
        if self.seen.len() > MAX_READ_BLOCKS {
            self.valid = false;
            return;
        }
        if let Some(block) = proc_block(src, name) {
            self.body(&block.body, src);
        }
    }

    fn body(&mut self, body: &[Stmt], src: &Sources) {
        for stmt in body {
            match stmt {
                Stmt::If(arms, other) => {
                    for (test, body) in arms {
                        self.expression(test, src);
                        self.body(body, src);
                    }
                    if let Some(body) = other {
                        self.body(body, src);
                    }
                }
                Stmt::Block(body) => self.body(body, src),
                Stmt::While(test, body) => {
                    self.expression(test, src);
                    self.body(body, src);
                }
                Stmt::Switch(test, arms) => {
                    self.expression(test, src);
                    for (keys, body) in arms {
                        for key in keys {
                            self.expression(key, src);
                        }
                        self.body(body, src);
                    }
                }
                Stmt::Def(_, Some(value)) | Stmt::Assign(_, value) | Stmt::ReturnValue(value) => {
                    self.expression(value, src)
                }
                Stmt::Jump(name, args) => {
                    for arg in args {
                        self.expression(arg, src);
                    }
                    if self.seen.insert(format!("@{name}")) {
                        if self.seen.len() > MAX_READ_BLOCKS {
                            self.valid = false;
                            return;
                        }
                        if let Lookup::Found(block) = Sources::one(&src.labels, name) {
                            self.body(&block.body, src);
                        }
                    }
                }
                Stmt::Call(name, args) => {
                    if let Some(name) = name.strip_prefix('~') {
                        self.procedure(name, src);
                    }
                    for arg in args {
                        self.expression(arg, src);
                    }
                    if name == "settimer" {
                        if let Some(Expr::Word(timer)) = args.first() {
                            if self.seen.insert(format!("timer:{timer}")) {
                                if let Lookup::Found(block) = Sources::one(&src.timers, timer) {
                                    self.body(&block.body, src);
                                }
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
    let Stmt::If(arms, None) = stmt else {
        return false;
    };
    let [(Expr::Cmp(CmpOp::Eq, test, before), body)] = arms.as_slice() else {
        return false;
    };
    let Expr::Word(varp) = test.as_ref() else {
        return false;
    };
    let Some((_, complete)) = varp
        .strip_prefix('%')
        .and_then(|name| src.quest_complete.get(name))
    else {
        return false;
    };
    let scalar = |expr: &Expr| match expr {
        Expr::Num(value) => Some(*value),
        Expr::Word(name) => name
            .strip_prefix('^')
            .and_then(|name| src.constants.get(name))
            .copied(),
        _ => None,
    };
    if !scalar(before).is_some_and(|before| before < *complete) {
        return false;
    }
    !body.is_empty() && body.iter().all(|stmt| matches!(stmt, Stmt::Assign(names, value)
        if names.as_slice() == [varp.clone()] && scalar(value).is_some_and(|after| after < *complete)))
}
/// Prove the complete bounded scenery update used after scripted gate motion.
/// This accepts only the two observed zone-search origins and the full
/// loc-change/loc-add loop; it does not certify arbitrary loc lookups or loops.
pub(super) fn scenery_tail(stmts: &[Stmt], src: &Sources) -> bool {
    if !matches!(stmts.len(), 3 | 4) {
        return false;
    }
    let [Stmt::Call(search, origin), Stmt::While(test, loop_body), ..] = stmts else {
        return false;
    };
    if search != "loc_findallzone"
        || !(matches!(origin.as_slice(), [Expr::Word(coord)] if coord == "coord")
            || matches!(
                origin.as_slice(),
                [Expr::Call(name, args)]
                    if name == "movecoord"
                        && matches!(args.as_slice(), [Expr::Word(coord), Expr::Num(1), Expr::Num(0), Expr::Num(0)] if coord == "coord")
            ))
        || !matches!(test, Expr::Cmp(CmpOp::Eq, lhs, rhs)
            if matches!(lhs.as_ref(), Expr::Word(name) if name == "loc_findnext")
                && matches!(rhs.as_ref(), Expr::Word(value) if value == "true"))
    {
        return false;
    }
    let [Stmt::If(arms, None)] = loop_body.as_slice() else {
        return false;
    };
    let [(category_test, category_body)] = arms.as_slice() else {
        return false;
    };
    if !matches!(category_test, Expr::Cmp(CmpOp::Eq, lhs, rhs)
        if matches!(lhs.as_ref(), Expr::Word(name) if name == "loc_category")
            && matches!(rhs.as_ref(), Expr::Word(name) if !name.is_empty()))
    {
        return false;
    }
    let [Stmt::Def(central, Some(loc_coord)), Stmt::Def(angle, Some(loc_angle)), Stmt::Def(kind, Some(Expr::Word(loc_type))), Stmt::Call(change, change_args), Stmt::If(type_arms, None)] =
        category_body.as_slice()
    else {
        return false;
    };
    if central != "$central_coord"
        || !scenery_query(loc_coord, "loc_coord")
        || angle != "$orig_angle"
        || !scenery_query(loc_angle, "loc_angle")
        || kind != "$type"
        || loc_type != "loc_type"
        || change != "loc_change"
        || !matches!(
            change_args.as_slice(),
            [Expr::Word(name), Expr::Num(3)] if name == "inviswall" && src.ids.contains_key(name)
        )
    {
        return false;
    }
    let [(type_test, additions)] = type_arms.as_slice() else {
        return false;
    };
    if !matches!(type_test, Expr::Cmp(CmpOp::Eq, lhs, rhs)
        if matches!(lhs.as_ref(), Expr::Word(name) if name == "$type")
            && matches!(rhs.as_ref(), Expr::Word(name) if src.ids.contains_key(name)))
    {
        return false;
    }
    let [Stmt::Call(left_add, left_args), Stmt::Call(right_add, right_args)] = additions.as_slice()
    else {
        return false;
    };
    if left_add != "loc_add"
        || right_add != "loc_add"
        || !scenery_loc_add(left_args, "$central_coord", -1, src)
        || !scenery_loc_add(right_args, "$central_coord", -2, src)
    {
        return false;
    }
    let finalizers = &stmts[2..];
    match finalizers {
        [Stmt::Call(delay, args)] => delay == "p_delay" && args.as_slice() == [Expr::Num(0)],
        [Stmt::Call(sound, sound_args), Stmt::Call(delay, delay_args)] => {
            sound == "sound_synth"
                && matches!(
                    sound_args.as_slice(),
                    [Expr::Word(_), Expr::Num(1), Expr::Num(0)]
                )
                && delay == "p_delay"
                && delay_args.as_slice() == [Expr::Num(0)]
        }
        _ => false,
    }
}

fn scenery_query(expr: &Expr, query: &str) -> bool {
    matches!(expr, Expr::Word(name) if name == query)
        || matches!(expr, Expr::Call(name, args) if name == query && args.is_empty())
}

fn contains_scenery_tail(body: &[Stmt], src: &Sources) -> bool {
    (0..body.len()).any(|start| scenery_tail(&body[start..], src))
}

fn contains_teleport(body: &[Stmt]) -> bool {
    let mut calls = Vec::new();
    body.iter().for_each(|stmt| stmt.calls(&mut calls));
    calls.iter().any(|call| call == "p_teleport")
}

fn has_paired_teleport(stmt: &Stmt) -> bool {
    let Stmt::If(arms, Some(otherwise)) = stmt else {
        return false;
    };
    let [(_, entering)] = arms.as_slice() else {
        return false;
    };
    contains_teleport(entering) && contains_teleport(otherwise)
}

fn calls_scenery_tail(body: &[Stmt], src: &Sources) -> bool {
    let mut calls = Vec::new();
    body.iter().for_each(|stmt| stmt.calls(&mut calls));
    calls
        .iter()
        .filter_map(|call| call.strip_prefix('@'))
        .any(|name| {
            if let Lookup::Found(block) = Sources::one(&src.labels, name) {
                contains_scenery_tail(&block.body, src)
            } else {
                false
            }
        })
}

fn has_paired_gate_tail(block: &Block, src: &Sources) -> bool {
    block.body.iter().enumerate().any(|(index, stmt)| {
        if !has_paired_teleport(stmt) {
            return false;
        }
        let suffix = &block.body[index + 1..];
        contains_scenery_tail(suffix, src) || calls_scenery_tail(suffix, src)
    })
}

// Stage windows require the paired teleport and scenery-tail structure
// evidenced by the shipyard and Varrock gate handlers.
fn has_certified_paired_gate(handler: &Block, src: &Sources) -> bool {
    let calls = reachable_calls(handler, src);
    let mut blocks = vec![handler];
    for call in &calls {
        let Some(name) = call.strip_prefix('@') else {
            continue;
        };
        if let Lookup::Found(block) = Sources::one(&src.labels, name) {
            blocks.push(block);
        }
    }
    blocks.iter().any(|block| has_paired_gate_tail(block, src))
}

fn scenery_loc_add(args: &[Expr], origin: &str, dx: i32, src: &Sources) -> bool {
    matches!(
        args,
        [Expr::Call(name, coord_args), Expr::Word(loc), Expr::Num(3), Expr::Word(shape), Expr::Num(3)]
            if name == "movecoord"
                && matches!(coord_args.as_slice(), [Expr::Word(base), Expr::Num(offset), Expr::Num(0), Expr::Num(0)] if base == origin && *offset == dx)
                && src.ids.contains_key(loc)
                && shape == "loc_shape"
    )
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
            // PlayerOps.P_FINDUID keeps protected access to the current
            // active player for its own uid (PlayerOps.ts:82-85).
            "p_finduid" => {
                Some(matches!(args, [Expr::Word(uid)] if uid == "uid").then_some(Val::Bool(true)))
            }
            "inv_getobj" => Some((|| {
                let [Expr::Word(inv), slot] = args else {
                    return None;
                };
                if inv != "worn" {
                    return None;
                }
                let Val::Int(slot) = self.value(slot, env)? else {
                    return None;
                };
                Some(Val::WornSlot(slot))
            })()),
            "oc_category" => Some((|| {
                let [obj] = args else {
                    return None;
                };
                let Val::WornSlot(slot) = self.value(obj, env)? else {
                    return None;
                };
                Some(Val::WornCategory(slot))
            })()),
            "npc_find" => Some(
                input
                    .npc_queries
                    .iter()
                    .position(|query| query == expr)
                    .map(|index| Val::Bool(input.npc_mask & (1usize << index) != 0)),
            ),
            "loc_find" => Some((|| {
                if self.obstacle == Some(obstacles::Class::Wall) {
                    let index = input
                        .scenery_queries
                        .iter()
                        .position(|query| query == expr)?;
                    if input.scenery_mask & (1usize << index) == 0 {
                        return Some(Val::Bool(false));
                    }
                }
                let [coord, Expr::Word(name)] = args else {
                    return None;
                };
                let Val::Coord(coord) = self.value(coord, env)? else {
                    return None;
                };
                let id = *self.base.src.ids.get(name)?;
                let mut matches =
                    input
                        .positions
                        .get(&id)
                        .into_iter()
                        .flatten()
                        .filter(|placement| {
                            placement.x == coord.x
                                && placement.z == coord.z
                                && placement.level == coord.level
                        });
                let Some(placement) = matches.next() else {
                    return Some(Val::Bool(false));
                };
                if matches.next().is_some() {
                    return None;
                }
                self.base.at = coord;
                self.base.angle = placement.angle;
                self.loc_id = id;
                self.shape = placement.shape;
                self.loc_modified = false;
                self.facts = input.facts.locs.get(&id)?.as_ref()?;
                Some(Val::Bool(true))
            })()),
            name if name.starts_with('~')
                && !matches!(name, "~check_axis" | "~check_axis_locactive") =>
            {
                let Some(block) = proc_block(self.base.src, &name[1..]) else {
                    return Some(None);
                };
                if !self.base.src.proc_non_moving(&name[1..]) {
                    return Some(None);
                }
                let saved = self.returned.take();
                let value_query = std::mem::replace(&mut self.value_query, true);
                let end = self.invoke(block, args, env);
                self.value_query = value_query;
                let value = self.returned.take();
                self.returned = saved;
                Some((end != End::Refused).then_some(value).flatten())
            }
            _ => None,
        }
    }

    pub(super) fn entrance_compare(
        &mut self,
        op: CmpOp,
        lhs: &Expr,
        rhs: &Expr,
        env: &Env,
    ) -> Option<Cond> {
        let input = self.entrance?;
        if let Expr::Word(varp) = lhs {
            if let Some(quest_varp) = varp.strip_prefix('%') {
                let Some((quest, complete)) = self.base.src.quest_complete.get(quest_varp) else {
                    return Some(Cond::unknown());
                };
                if self.base.src.quest_signals.get(quest_varp) != Some(quest) {
                    return Some(Cond::unknown());
                }
                let Some(Val::Int(threshold)) = self.value(rhs, env) else {
                    return Some(Cond::unknown());
                };
                let quest_stage = self.base.src.quest_stage.get(quest_varp).copied();
                let colour_authoritative =
                    quest_stage.is_some_and(|facts| facts.colour_authoritative);
                let stage_domain =
                    quest_stage.and_then(|facts| authoritative_stage_domain(facts, *complete));
                let selected_stage = input
                    .stage_ranges
                    .and_then(|ranges| stage_partition(ranges, varp));
                let above = input
                    .above_queries
                    .iter()
                    .position(|name| name == varp)
                    .is_some_and(|index| input.above_mask & (1usize << index) != 0);
                let (min, max) = if let Some(stage) = selected_stage {
                    let (Some(min), Some(max)) = (stage.values.min, stage.values.max) else {
                        return Some(Cond::unknown());
                    };
                    (min, max)
                } else if let Some((domain_min, domain_max)) = stage_domain {
                    if !input.complete {
                        (domain_min, complete.checked_sub(1)?)
                    } else if above {
                        (complete.checked_add(1)?, domain_max)
                    } else {
                        (*complete, *complete)
                    }
                } else if !input.complete {
                    (i32::MIN, complete.checked_sub(1)?)
                } else if above {
                    (complete.checked_add(1)?, i32::MAX)
                } else {
                    (*complete, *complete)
                };
                if min > max {
                    return Some(Cond::unknown());
                }
                let value = comparison_value(op, min, max, threshold);
                let needs = if let Some((_, domain_max)) = stage_domain {
                    let values = if let Some(stage) = selected_stage {
                        stage.values
                    } else {
                        let Some(branch_value) = value else {
                            return Some(Cond::unknown());
                        };
                        let Some((domain_min, domain_max)) = stage_domain else {
                            unreachable!("matched authoritative stage domain");
                        };
                        let Some(values) =
                            comparison_window(op, branch_value, threshold, domain_min, domain_max)
                        else {
                            return Some(Cond::unknown());
                        };
                        values
                    };
                    Needs {
                        quests: vec![quest_gate(
                            quest,
                            quest_varp,
                            values,
                            *complete,
                            domain_max,
                            colour_authoritative,
                        )],
                        ..Needs::default()
                    }
                } else {
                    let Some(branch_value) = value else {
                        return Some(Cond::unknown());
                    };
                    let Some(values) =
                        comparison_window(op, branch_value, threshold, i32::MIN, i32::MAX)
                            .or_else(|| comparison_window(op, branch_value, threshold, min, max))
                    else {
                        return Some(Cond::unknown());
                    };
                    // Content can change a progress varp after publishing a
                    // green row. Keep its actual comparison, never a colour
                    // completion substitute, when no stage domain is proved.
                    let window = api::selected::StageWindow {
                        quest: quest.clone(),
                        signal: api::selected::FactKey::new(quest_varp),
                        values,
                    };
                    Needs {
                        quests: vec![api::selected::QuestGate::Window(window)],
                        ..Needs::default()
                    }
                };
                return Some(Cond { value, needs });
            }
        }
        if let Expr::Call(name, args) = lhs {
            if name == "inv_total" {
                let [Expr::Word(inv), Expr::Word(obj)] = args.as_slice() else {
                    return Some(Cond::unknown());
                };
                let Some(&id) = self.base.src.objs.get(obj) else {
                    return Some(Cond::unknown());
                };
                if input.absent.contains(&id) {
                    let Some(Val::Int(value)) = self.value(rhs, env) else {
                        return Some(Cond::unknown());
                    };
                    let value = match op {
                        CmpOp::Eq => value == 0,
                        CmpOp::Ne => value != 0,
                        CmpOp::Lt => 0 < value,
                        CmpOp::Le => 0 <= value,
                        CmpOp::Gt => 0 > value,
                        CmpOp::Ge => 0 >= value,
                    };
                    let mut needs = Needs::default();
                    match inv.as_str() {
                        "inv" => needs.forbidden_held.push(id),
                        "worn" => needs.forbidden_worn.push(id),
                        _ => return Some(Cond::unknown()),
                    }
                    return Some(Cond {
                        value: Some(value),
                        needs,
                    });
                }
                return None;
            }
        }
        let left = self.value(lhs, env)?;
        let right = self.value(rhs, env)?;
        match (left, right) {
            (Val::WornCategory(slot), Val::Name(category))
                if matches!(op, CmpOp::Eq | CmpOp::Ne) =>
            {
                let mut forbidden: Vec<_> = self
                    .base
                    .src
                    .worn_slots
                    .iter()
                    .filter_map(|(&id, &worn_slot)| {
                        if worn_slot != slot {
                            return None;
                        }
                        let excluded = match input.facts.categories.get(&id) {
                            Some(Some(actual)) => (actual == &category) == (op == CmpOp::Eq),
                            Some(None) => true,
                            None => op == CmpOp::Ne,
                        };
                        excluded.then_some(id)
                    })
                    .collect();
                forbidden.sort_unstable();
                Some(Cond {
                    value: Some(false),
                    needs: Needs {
                        forbidden_worn: forbidden,
                        ..Needs::default()
                    },
                })
            }
            (Val::WornSlot(_), Val::Name(name)) if name == "null" => Some(Cond::unknown()),
            _ => None,
        }
    }

    pub(super) fn entrance_call(&mut self, name: &str, args: &[Expr], env: &Env) -> Option<End> {
        if name == "npc_facesquare" {
            // NpcOps.ts:103-106 changes only the selected NPC's facing tile.
            return Some(
                if matches!(args, [coord] if matches!(self.value(coord, env), Some(Val::Coord(_))))
                {
                    End::Continue
                } else {
                    End::Refused
                },
            );
        }
        let proc = name.strip_prefix('~');
        if self.timer.is_some()
            && (matches!(
                name,
                "p_teleport"
                    | "p_telejump"
                    | "p_exactmove"
                    | "~forcemove"
                    | "~agility_exactmove"
                    | "~agility_force_move"
            ) || proc.is_some_and(|name| OPEN_PROCS.iter().any(|spec| spec.name == name)))
        {
            return Some(End::Refused);
        }
        if let Some(proc) = proc.filter(|name| OPEN_PROCS.iter().any(|spec| spec.name == *name)) {
            self.base.stand = Some(self.here);
            let Flow::Crossed(Arrival::Door(_), _) = self.base.open(proc, args, env) else {
                return Some(End::Refused);
            };
            let Some(dir) = door_dir(self.base.angle) else {
                return Some(End::Refused);
            };
            let (dx, dz) = dir_delta(dir);
            let to = if self.here == self.base.at {
                WorldTile {
                    x: self.base.at.x + dx,
                    z: self.base.at.z + dz,
                    level: self.base.at.level,
                }
            } else {
                self.base.at
            };
            return Some(if self.relocate(to).is_some() {
                self.moved = true;
                End::Continue
            } else {
                End::Refused
            });
        }
        if name == "settimer" {
            let [Expr::Word(timer), delay] = args else {
                return Some(End::Refused);
            };
            let valid_delay = match delay {
                Expr::Num(delay) => *delay > 0,
                Expr::Call(name, args) if name == "~random_range" => {
                    matches!(args.as_slice(), [Expr::Num(min), Expr::Num(max)] if *min > 0 && min <= max)
                }
                _ => false,
            };
            if !valid_delay {
                return Some(End::Refused);
            }
            if let Some(active) = self.timer {
                return Some(if active == timer {
                    End::Continue
                } else {
                    End::Refused
                });
            }
            let Some((timer, _)) = self.base.src.timers.get_key_value(timer) else {
                return Some(End::Refused);
            };
            let Lookup::Found(block) = Sources::one(&self.base.src.timers, timer) else {
                return Some(End::Refused);
            };
            self.timer = Some(timer);
            let end = self.run(&block.body, &mut Env::new());
            self.timer = None;
            return Some(if end == End::Refused {
                end
            } else {
                End::Continue
            });
        }
        None
    }
}

struct Trajectory {
    to: WorldTile,
    ticks: i32,
    needs: Needs,
}

fn normalize(needs: &mut Needs) -> bool {
    needs.items = max_per_key(&needs.items);
    needs.worn = max_per_key(&needs.worn);
    for ids in [&mut needs.forbidden_held, &mut needs.forbidden_worn] {
        ids.sort_unstable();
        ids.dedup();
    }
    !needs
        .items
        .iter()
        .any(|(id, count)| *count > 0 && needs.forbidden_held.contains(id))
        && !needs
            .worn
            .iter()
            .any(|(id, count)| *count > 0 && needs.forbidden_worn.contains(id))
}

fn weaker_than(weaker: &Needs, stronger: &Needs) -> bool {
    let positive = |a: &[(i32, i32)], b: &[(i32, i32)]| {
        a.iter()
            .all(|(id, count)| b.iter().any(|(other, n)| id == other && n >= count))
    };
    positive(&weaker.items, &stronger.items)
        && positive(&weaker.worn, &stronger.worn)
        && positive(&weaker.skills, &stronger.skills)
        && positive(&weaker.varps, &stronger.varps)
        && weaker
            .forbidden_held
            .iter()
            .all(|id| stronger.forbidden_held.contains(id))
        && weaker
            .forbidden_worn
            .iter()
            .all(|id| stronger.forbidden_worn.contains(id))
        && (!weaker.members || stronger.members)
}

#[derive(Clone, Copy)]
struct TrajectoryInputs<'a> {
    src: &'a Sources,
    facts: &'a Facts,
    loc_facts: &'a LocFacts,
    positions: &'a HashMap<i32, Vec<Placement>>,
    proofs: &'a HashSet<String>,
    supported: &'a HashSet<String>,
    selectors: &'a HashMap<String, Vec<Tool>>,
    handler: &'a Block,
    id: i32,
    placement: &'a Placement,
    stand: WorldTile,
    absent: &'a [i32],
    npc: &'a [Expr],
    scenery: &'a [Expr],
    obstacle: Option<obstacles::Class>,
    above: &'a [String],
    complete: bool,
    stage_ranges: Option<&'a [StagePartition]>,
    above_mask: usize,
}

fn trajectory(input: TrajectoryInputs<'_>) -> Option<Trajectory> {
    let TrajectoryInputs {
        src,
        facts,
        loc_facts,
        positions,
        proofs,
        supported,
        selectors,
        handler,
        id,
        placement,
        stand,
        absent,
        npc,
        above,
        scenery,
        obstacle,
        complete,
        stage_ranges: selected_stages,
        above_mask,
    } = input;
    let at = WorldTile {
        x: placement.x,
        z: placement.z,
        level: placement.level,
    };
    let mut result: Option<Trajectory> = None;
    let above = if complete { above } else { &[] };
    // NPC and wall-scenery presence are universally quantified within one
    // typed progress alternative. Disjoint progress windows stay separate.
    for presence_mask in 0..(1usize << (npc.len() + scenery.len())) {
        let npc_mask = presence_mask & ((1usize << npc.len()) - 1);
        let scenery_mask = presence_mask >> npc.len();
        let mut motion = Motion {
            base: Eval {
                src,
                supported,
                at,
                stand: Some(stand),
                angle: placement.angle,
                needs: Needs::default(),
                relative_landing_safe: false,
                jumps: 0,
            },
            facts: loc_facts,
            proofs,
            selectors,
            tool: None,
            here: stand,
            loc_id: id,
            loc_modified: false,
            shape: placement.shape,
            moved: false,
            ticks: 1,
            gate_read: false,
            entry_modal_closed: true,
            entrance: Some(Inputs {
                facts,
                positions,
                absent,
                npc_queries: npc,
                npc_mask,
                complete,
                stage_ranges: selected_stages,
                scenery_queries: scenery,
                scenery_mask,
                above_queries: above,
                above_mask,
            }),
            returned: None,
            value_query: false,
            timer: None,
            obstacle,
        };
        if motion.run(&handler.body, &mut Env::new()) == End::Refused
            || !motion.moved
            || motion.here == stand
        {
            return None;
        }
        if let Some(result) = &mut result {
            if result.to != motion.here {
                return None;
            }
            result.ticks = result.ticks.max(motion.ticks);
            result.needs.merge(motion.base.needs);
        } else {
            result = Some(Trajectory {
                to: motion.here,
                ticks: motion.ticks,
                needs: motion.base.needs,
            });
        }
    }
    let mut result = result?;
    normalize(&mut result.needs).then_some(result)
}

pub(super) fn edges(input: EntranceInputs<'_>) {
    let EntranceInputs {
        args:
            EntranceArgs {
                src,
                positions,
                defs,
                graph,
                collision,
                skipped,
                observable,
                audit,
            },
        facts,
        proofs,
        supported,
        selectors,
    } = input;
    let first_new_edge = graph.edges.len();
    let covered: HashSet<_> = graph
        .edges
        .iter()
        .filter(|edge| {
            matches!(
                edge.kind,
                TransportKind::Door
                    | TransportKind::Ladder
                    | TransportKind::Stairs
                    | TransportKind::AgilityShortcut
            )
        })
        .map(|edge| (edge.loc_id, edge.option))
        .collect();
    let mut locs: Vec<_> = positions.keys().copied().collect();
    locs.sort_unstable();
    let empty_loc_facts = LocFacts::default();
    for id in locs {
        let fact = match facts.locs.get(&id) {
            Some(Some(fact)) => fact,
            Some(None) => continue,
            None => &empty_loc_facts,
        };
        let Some(def) = defs.loc(id) else {
            continue;
        };
        for option in 1..=5 {
            let own: Vec<_> = src
                .ids
                .iter()
                .filter(|(_, value)| **value == id)
                .map(|(name, _)| Sources::one(&src.oplocs[(option - 1) as usize], name))
                .filter(|value| !matches!(value, Lookup::Missing))
                .collect();
            let handler = match own.as_slice() {
                [Lookup::Found(block)] => *block,
                [] => match fact.category.as_ref().map(|name| {
                    Sources::one(&src.oplocs[(option - 1) as usize], &format!("_{name}"))
                }) {
                    Some(Lookup::Found(block)) => block,
                    _ => continue,
                },
                _ => continue,
            };
            let obstacle = obstacles::classify(handler, src);
            if covered.contains(&(id, option)) && obstacle != Some(obstacles::Class::Ladder) {
                continue;
            }
            let calls = reachable_calls(handler, src);
            if !calls.iter().any(|call| {
                matches!(
                    call.as_str(),
                    "p_teleport"
                        | "p_telejump"
                        | "~forcemove"
                        | "~agility_exactmove"
                        | "~agility_force_move"
                        | "p_exactmove"
                ) || call
                    .strip_prefix('~')
                    .is_some_and(|name| OPEN_PROCS.iter().any(|spec| spec.name == name))
            }) {
                continue;
            }
            let mut reads = Reads {
                stage_partitionable: true,
                valid: true,
                ..Reads::default()
            };
            reads.body(&handler.body, src);
            if obstacle != Some(obstacles::Class::Wall) {
                reads.scenery.clear();
            }
            reads.items.sort_unstable();
            reads.items.dedup();
            let paired_gate = has_certified_paired_gate(handler, src);
            let mut above: Vec<_> = reads
                .quests
                .iter()
                .filter_map(|name| {
                    let varp = &name[1..];
                    if src.quest_exact.contains(varp) {
                        return None;
                    }
                    let (_, complete) = src.quest_complete.get(varp)?;
                    (*complete < i32::MAX).then(|| name.clone())
                })
                .collect();
            above.sort_unstable();
            if !reads.valid {
                bump(
                    skipped,
                    "loc entrance exceeds bounded content alternatives",
                    1,
                );
                continue;
            }
            // Otherwise retain separately gated completion/above-stage alternatives.
            let stage_dimensions = match stage_dimensions(&reads, src) {
                Ok(dimensions) => dimensions,
                Err(()) => {
                    bump(
                        skipped,
                        "loc entrance exceeds bounded content alternatives",
                        1,
                    );
                    continue;
                }
            };
            let stage_count = stage_dimensions.as_ref().map_or(Some(1), |dimensions| {
                dimensions.iter().try_fold(1usize, |count, dimension| {
                    count.checked_mul(dimension.ranges.len())
                })
            });
            let Some(stage_count) = stage_count else {
                bump(
                    skipped,
                    "loc entrance exceeds bounded content alternatives",
                    1,
                );
                continue;
            };
            let Some(mask_bits) = reads
                .items
                .len()
                .checked_add(reads.npc.len())
                .and_then(|bits| bits.checked_add(reads.scenery.len()))
                .and_then(|bits| {
                    bits.checked_add(if stage_dimensions.is_some() {
                        0
                    } else {
                        above.len()
                    })
                })
            else {
                bump(
                    skipped,
                    "loc entrance exceeds bounded content alternatives",
                    1,
                );
                continue;
            };
            if mask_bits >= usize::BITS as usize
                || (1usize << mask_bits)
                    .checked_mul(stage_count)
                    .is_none_or(|count| count > MAX_ALTERNATIVES)
            {
                bump(
                    skipped,
                    "loc entrance exceeds bounded content alternatives",
                    1,
                );
                continue;
            }
            let stage_variants = stage_dimensions.as_deref().map(stage_variants);
            let member_path = src.has_member_reads(handler);
            let kind = if defs
                .op_at(id, option)
                .is_some_and(|operation| operation.eq_ignore_ascii_case("Pass-through"))
            {
                TransportKind::Door
            } else {
                TransportKind::Ladder
            };
            for placement in &positions[&id] {
                let at = WorldTile {
                    x: placement.x,
                    z: placement.z,
                    level: placement.level,
                };
                let mut stands = Vec::new();
                if placement.shape == 0 {
                    let Some(dir) = door_dir(placement.angle) else {
                        continue;
                    };
                    let (dx, dz) = dir_delta(dir);
                    stands.extend([
                        at,
                        WorldTile {
                            x: at.x + dx,
                            z: at.z + dz,
                            level: at.level,
                        },
                    ]);
                } else if placement.shape == 4 && obstacle == Some(obstacles::Class::Window) {
                    // Wall-decoration reach always permits the loc's own tile.
                    stands.push(at);
                } else if matches!(placement.shape, 10 | 11 | 22) {
                    let Some(approach) = LocApproach::from_loc_def(
                        def.width,
                        def.length,
                        placement.angle,
                        def.force_approach,
                    ) else {
                        continue;
                    };
                    for x in at.x - 1..=at.x + i32::from(approach.width) {
                        for z in at.z - 1..=at.z + i32::from(approach.length) {
                            let stand = WorldTile {
                                x,
                                z,
                                level: at.level,
                            };
                            if approach.can_operate(
                                at,
                                stand,
                                collision.walkable_word(x, z, stand.level) as i32,
                            ) {
                                stands.push(stand);
                            }
                        }
                    }
                }
                for stand in stands
                    .into_iter()
                    .filter(|stand| collision.standable(*stand))
                {
                    for mask in 0..(1usize << reads.items.len()) {
                        let absent: Vec<_> = reads
                            .items
                            .iter()
                            .enumerate()
                            .filter_map(|(index, &id)| {
                                (mask & (1usize << index) != 0).then_some(id)
                            })
                            .collect();
                        let trajectory_inputs = TrajectoryInputs {
                            src,
                            facts,
                            loc_facts: fact,
                            positions,
                            proofs,
                            supported,
                            selectors,
                            handler,
                            id,
                            placement,
                            stand,
                            absent: &absent,
                            npc: &reads.npc,
                            above: &above,
                            scenery: &reads.scenery,
                            obstacle,
                            complete: true,
                            stage_ranges: None,
                            above_mask: 0,
                        };
                        {
                            let mut admit = |result: Trajectory| {
                                if obstacle.is_none()
                                    && !paired_gate
                                    && result.needs.quests.is_empty()
                                    && result.needs.forbidden_held.is_empty()
                                    && result.needs.forbidden_worn.is_empty()
                                    && result.needs.items.is_empty()
                                    && result.needs.worn.is_empty()
                                {
                                    return;
                                }
                                if !collision.standable(result.to) {
                                    return;
                                }
                                if covered.contains(&(id, option))
                                    && graph.edges[..first_new_edge].iter().any(|edge| {
                                        edge.loc_id == id
                                            && edge.option == option
                                            && edge.to == result.to
                                    })
                                {
                                    return;
                                }
                                let mut edge = gated_edge(
                                    obstacle.map_or(kind, obstacles::Class::kind),
                                    id,
                                    at,
                                    result.to,
                                    result.needs,
                                );
                                edge.takeoff = Some(stand);
                                edge.option = option;
                                edge.ticks = result.ticks;
                                if observable.admit_edge(graph, edge, audit) && member_path {
                                    audit.record_member_path(
                                        graph.edges.last().expect("admitted edge"),
                                    );
                                }
                            };
                            if let Some(variants) = &stage_variants {
                                for stage_ranges in variants {
                                    if let Some(result) = trajectory(TrajectoryInputs {
                                        stage_ranges: Some(stage_ranges),
                                        ..trajectory_inputs
                                    }) {
                                        admit(result);
                                    }
                                }
                            } else {
                                for above_mask in 0..(1usize << above.len()) {
                                    let complete = trajectory(TrajectoryInputs {
                                        above_mask,
                                        ..trajectory_inputs
                                    });
                                    let incomplete = (above_mask == 0 && reads.quests.len() == 1)
                                        .then(|| {
                                            trajectory(TrajectoryInputs {
                                                complete: false,
                                                ..trajectory_inputs
                                            })
                                        })
                                        .flatten()
                                        .filter(|candidate| {
                                            complete.as_ref().is_some_and(|other| {
                                                other.to == candidate.to
                                                    && weaker_than(&other.needs, &candidate.needs)
                                            })
                                        });
                                    for result in [complete, incomplete].into_iter().flatten() {
                                        admit(result);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // Irrelevant item assignments can prove the same trajectory. Compact
    // only this producer's suffix without another allocation or old edits.
    graph.edges[first_new_edge..].sort_by(edge_order);
    let mut write = first_new_edge;
    for read in first_new_edge..graph.edges.len() {
        if write > first_new_edge && edge_order(&graph.edges[write - 1], &graph.edges[read]).is_eq()
        {
            continue;
        }
        if write != read {
            graph.edges.swap(write, read);
        }
        write += 1;
    }
    graph.edges.truncate(write);
}

#[test]
#[ignore = "requires NAV_ENTRANCE_CONTENT pointing to the pinned real content"]
fn real_camp_gate_read_set_stays_within_the_entrance_bound() {
    let root = std::path::PathBuf::from(
        std::env::var_os("NAV_ENTRANCE_CONTENT").expect("real content root"),
    );
    let selected =
        api::game_data::for_revision(client::io::ClientRevision::R289).expect("selected revision");
    let facts = selected.quest_identity().expect("selected quest facts");
    let quests = QuestSource { facts };
    let src = Sources::read(&root, &loc_ids_by_name(&root), Some(&quests));
    let name = src
        .ids
        .iter()
        .find_map(|(name, &id)| (id == 2673).then_some(name))
        .expect("camp gate loc");
    let Lookup::Found(handler) = Sources::one(&src.oplocs[0], name) else {
        panic!("camp gate handler");
    };
    let mut reads = Reads {
        valid: true,
        ..Reads::default()
    };
    reads.body(&handler.body, &src);
    reads.items.sort_unstable();
    reads.items.dedup();
    let bits = reads.items.len() + reads.npc.len() + reads.quests.len();
    assert!(
        reads.valid && bits < usize::BITS as usize && (1usize << bits) <= MAX_ALTERNATIVES,
        "camp gate reads {} blocks, {} items, {} NPC queries, {} quests",
        reads.seen.len(),
        reads.items.len(),
        reads.npc.len(),
        reads.quests.len()
    );
}

#[cfg(test)]
mod stage_window_tests {
    use super::*;

    fn range(min: i32, max: i32) -> api::selected::InclusiveRange {
        api::selected::InclusiveRange {
            min: Some(min),
            max: Some(max),
        }
    }

    #[test]
    fn stage_comparisons_keep_exact_inclusive_truth_intervals() {
        assert_eq!(
            comparison_window(CmpOp::Eq, true, 7, 0, 20),
            Some(range(7, 7))
        );
        assert_eq!(
            comparison_window(CmpOp::Eq, false, 0, 0, 20),
            Some(range(1, 20))
        );
        assert_eq!(comparison_window(CmpOp::Eq, false, 7, 0, 20), None);
        assert_eq!(
            comparison_window(CmpOp::Ne, true, 0, 0, 20),
            Some(range(1, 20))
        );
        assert_eq!(
            comparison_window(CmpOp::Ne, false, 7, 0, 20),
            Some(range(7, 7))
        );
        assert_eq!(comparison_window(CmpOp::Ne, true, 7, 0, 20), None);
        assert_eq!(
            comparison_window(CmpOp::Lt, true, 10, 0, 20),
            Some(range(0, 9))
        );
        assert_eq!(
            comparison_window(CmpOp::Lt, false, 10, 0, 20),
            Some(range(10, 20))
        );
        assert_eq!(
            comparison_window(CmpOp::Le, true, 10, 0, 20),
            Some(range(0, 10))
        );
        assert_eq!(
            comparison_window(CmpOp::Le, false, 10, 0, 20),
            Some(range(11, 20))
        );
        assert_eq!(
            comparison_window(CmpOp::Gt, true, 10, 0, 20),
            Some(range(11, 20))
        );
        assert_eq!(
            comparison_window(CmpOp::Gt, false, 10, 0, 20),
            Some(range(0, 10))
        );
        assert_eq!(
            comparison_window(CmpOp::Ge, true, 10, 0, 20),
            Some(range(10, 20))
        );
        assert_eq!(
            comparison_window(CmpOp::Ge, false, 10, 0, 20),
            Some(range(0, 9))
        );
    }

    #[test]
    fn content_domains_do_not_depend_on_journal_authority() {
        let facts = QuestStageFacts {
            colour_authoritative: true,
            stage_domain: Some(range(0, 20)),
            transmission: api::game_data::Transmission::NotTransmitted,
        };
        assert_eq!(authoritative_stage_domain(facts, 15), Some((0, 20)));
        assert_eq!(
            authoritative_stage_domain(
                QuestStageFacts {
                    transmission: api::game_data::Transmission::PostedByContent,
                    ..facts
                },
                15,
            ),
            None
        );
        assert_eq!(
            authoritative_stage_domain(
                QuestStageFacts {
                    colour_authoritative: false,
                    ..facts
                },
                15,
            ),
            Some((0, 20))
        );
        assert_eq!(
            authoritative_stage_domain(
                QuestStageFacts {
                    stage_domain: Some(range(-1, 20)),
                    ..facts
                },
                15,
            ),
            None
        );
        assert_eq!(
            authoritative_stage_domain(
                QuestStageFacts {
                    stage_domain: Some(range(0, 14)),
                    ..facts
                },
                15,
            ),
            None
        );
    }

    /// A controlled no-domain comparison, not a change to selected Hero's
    /// 0..15 domain or its exact15 completion Window.
    fn completed_progress_comparison(colour_authoritative: bool) -> Cond {
        let src = Sources {
            oplocs: std::array::from_fn(|_| HashMap::new()),
            oplocu: HashMap::new(),

            quest_complete: HashMap::from([(
                "heroquest".to_owned(),
                (api::selected::FactKey::new("hero"), 15),
            )]),
            quest_stage: HashMap::from([(
                "heroquest".to_owned(),
                QuestStageFacts {
                    colour_authoritative,
                    stage_domain: None,
                    transmission: api::game_data::Transmission::NotTransmitted,
                },
            )]),
            quest_signals: HashMap::from([(
                "heroquest".to_owned(),
                api::selected::FactKey::new("hero"),
            )]),
            quest_exact: HashSet::new(),
            labels: HashMap::new(),
            timers: HashMap::new(),
            procs: HashMap::new(),
            non_moving: std::cell::RefCell::new(HashMap::new()),
            constants: HashMap::from([("hero_complete".to_owned(), 15)]),
            varps: HashMap::from([("heroquest".to_owned(), 188)]),
            objs: HashMap::new(),
            ids: HashMap::new(),
            worn_slots: HashMap::new(),
        };
        let supported = HashSet::new();
        let loc_facts = LocFacts::default();
        let facts = Facts::default();
        let positions = HashMap::new();
        let absent: [i32; 0] = [];
        let npc_queries: [Expr; 0] = [];
        let scenery_queries: [Expr; 0] = [];
        let above_queries: [String; 0] = [];
        let proofs = HashSet::new();
        let selectors = HashMap::new();
        let at = WorldTile {
            x: 0,
            z: 0,
            level: 0,
        };
        let mut motion = Motion {
            base: Eval {
                src: &src,
                supported: &supported,
                at,
                stand: Some(at),
                angle: 0,
                needs: Needs::default(),
                relative_landing_safe: false,
                jumps: 0,
            },
            facts: &loc_facts,
            proofs: &proofs,
            selectors: &selectors,
            tool: None,
            here: at,
            loc_id: 900,
            loc_modified: false,
            shape: 0,
            moved: false,
            ticks: 1,
            gate_read: false,
            entry_modal_closed: true,
            entrance: Some(Inputs {
                facts: &facts,
                positions: &positions,
                absent: &absent,
                npc_queries: &npc_queries,
                npc_mask: 0,
                scenery_queries: &scenery_queries,
                scenery_mask: 0,
                complete: true,
                stage_ranges: None,
                above_queries: &above_queries,
                above_mask: 0,
            }),
            returned: None,
            value_query: false,
            timer: None,
            obstacle: None,
        };
        let lhs = Expr::Word("%heroquest".to_owned());
        let rhs = Expr::Word("^hero_complete".to_owned());
        motion
            .entrance_compare(CmpOp::Ge, &lhs, &rhs, &Env::new())
            .expect("quest comparison")
    }

    #[test]
    fn no_domain_comparison_preserves_its_raw_window_without_colour_substitution() {
        for colour_authoritative in [false, true] {
            let condition = completed_progress_comparison(colour_authoritative);
            assert_eq!(condition.value, Some(true));
            let [api::selected::QuestGate::Window(window)] = condition.needs.quests.as_slice()
            else {
                panic!(
                    "expected the raw progress window: {:?}",
                    condition.needs.quests
                );
            };
            assert_eq!(window.quest.0.as_ref(), "hero");
            assert_eq!(window.signal.0.as_ref(), "heroquest");
            assert_eq!(window.values.min, Some(15));
            assert_eq!(window.values.max, Some(i32::MAX));
        }
    }
}
