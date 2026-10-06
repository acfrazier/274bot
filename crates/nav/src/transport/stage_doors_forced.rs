//! Deterministic, requirement-gated loc motion. The statement parser and gate
//! interpreter are shared with stage doors; no loc-name geometry table exists.
use super::super::rs2_syntax::ArithmeticOp;
use super::*;
use api::query::loc_approach::LocApproach;

pub(super) const ENGINE_FORCED_PROCS: &str = include_str!("engine_forced_procs.rs2");
const SKIP_FORCED_CONFLICT: &str =
    "forced-move loc handler or configuration is ambiguous or unparsed";
const SKIP_FORCED_UNPROVEN: &str =
    "forced-move loc has no deterministic observable requirement-gated trajectory";
const SKIP_FORCED_UNOBSERVABLE: &str = "forced-move trajectory reads an unobservable varp gate";

#[derive(Default)]
struct Facts {
    locs: HashMap<i32, Option<LocFacts>>,
    objects: HashMap<(i32, String), Option<Val>>,
}

#[derive(Default, PartialEq, Eq)]
struct LocFacts {
    category: Option<String>,
    params: HashMap<String, Option<Val>>,
}

fn scalar(text: &str, src: &Sources) -> Option<Val> {
    let text = text.trim();
    if let Ok(number) = text.parse() {
        return Some(Val::Int(number));
    }
    if let Some(constant) = text.strip_prefix('^') {
        return src.constants.get(constant).copied().map(Val::Int);
    }
    if let Some((level, x, z)) = coord_literal(text) {
        return Some(Val::Coord(WorldTile { level, x, z }));
    }
    if !text.is_empty() && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Some(Val::Name(text.to_owned()));
    }
    None
}

fn insert_fact<K: Eq + std::hash::Hash, V: Eq>(
    map: &mut HashMap<K, Option<V>>,
    key: K,
    value: Option<V>,
) {
    match map.entry(key) {
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(value);
        }
        std::collections::hash_map::Entry::Occupied(mut entry) => {
            if entry.get() != &value {
                entry.insert(None);
            }
        }
    }
}

impl Facts {
    fn read(root: &Path, src: &Sources) -> Self {
        let mut facts = Self::default();
        visit_configs(&root.join("scripts"), "loc", &mut |text| {
            let mut row: Option<(i32, LocFacts)> = None;
            let mut flush = |row: &mut Option<(i32, LocFacts)>| {
                if let Some((id, value)) = row.take() {
                    insert_fact(&mut facts.locs, id, Some(value));
                }
            };
            for raw in text.lines() {
                let line = raw.split("//").next().unwrap_or("").trim();
                if let Some(name) = config_header(line) {
                    flush(&mut row);
                    row = src.ids.get(name).map(|&id| (id, LocFacts::default()));
                } else if let Some((_, row)) = &mut row {
                    if let Some(value) = line.strip_prefix("category=") {
                        let value = value.trim();
                        if row.category.as_deref().is_some_and(|old| old != value) {
                            // An unusable category cannot accidentally match a fallback handler.
                            row.category = Some(String::new());
                        } else {
                            row.category = Some(value.to_owned());
                        }
                    } else if let Some((key, value)) =
                        line.strip_prefix("param=").and_then(|p| p.split_once(','))
                    {
                        insert_fact(&mut row.params, key.trim().to_owned(), scalar(value, src));
                    }
                }
            }
            flush(&mut row);
        });
        visit_configs(&root.join("scripts"), "obj", &mut |text| {
            let mut id = None;
            for raw in text.lines() {
                let line = raw.split("//").next().unwrap_or("").trim();
                if let Some(name) = config_header(line) {
                    id = src.objs.get(name).copied();
                } else if let (Some(id), Some((key, value))) = (
                    id,
                    line.strip_prefix("param=").and_then(|p| p.split_once(',')),
                ) {
                    insert_fact(
                        &mut facts.objects,
                        (id, key.trim().to_owned()),
                        scalar(value, src),
                    );
                }
            }
        });
        facts
    }
}

#[derive(Clone, Copy)]
struct Tool {
    obj: i32,
    skill: i32,
    level: i32,
    worn: bool,
}

/// Recognise a whole content tool selector, not its name: skill/slot locals,
/// a sequence of level AND (worn OR carried) returns, then return(null).
/// A selected object's identity remains opaque during motion evaluation:
/// owning a lower-priority tool does not prove which tool the script chooses.
fn tool_selector(block: &Block, src: &Sources, facts: &Facts) -> Option<Vec<Tool>> {
    let [Stmt::Def(level_var, Some(Expr::Call(stat, skill))), Stmt::Def(worn_var, Some(Expr::Call(getobj, slot))), rest @ ..] =
        block.body.as_slice()
    else {
        return None;
    };
    if !block.params.is_empty() || stat != "stat" || getobj != "inv_getobj" {
        return None;
    }
    let skill = stat_id(skill)?;
    let [Expr::Word(inv), Expr::Word(slot)] = slot.as_slice() else {
        return None;
    };
    if inv != "worn" {
        return None;
    }
    let slot = *src.constants.get(slot.strip_prefix('^')?)?;
    let (last, branches) = rest.split_last()?;
    if !matches!(last, Stmt::ReturnValue(Expr::Word(name)) if name == "null") || branches.is_empty()
    {
        return None;
    }
    let mut tools = Vec::new();
    let mut seen = HashSet::new();
    for stmt in branches {
        let Stmt::If(arms, None) = stmt else {
            return None;
        };
        let [(Expr::And(level_test, possession), body)] = arms.as_slice() else {
            return None;
        };
        let Expr::Cmp(CmpOp::Ge, lhs, rhs) = level_test.as_ref() else {
            return None;
        };
        if !matches!(lhs.as_ref(), Expr::Word(name) if name == level_var) {
            return None;
        }
        let Expr::Call(param, args) = rhs.as_ref() else {
            return None;
        };
        let [Expr::Word(obj), Expr::Word(level_param)] = args.as_slice() else {
            return None;
        };
        if param != "oc_param" {
            return None;
        }
        let Expr::Or(worn, carried) = possession.as_ref() else {
            return None;
        };
        if !matches!(worn.as_ref(), Expr::Cmp(CmpOp::Eq, a, b)
            if matches!(a.as_ref(), Expr::Word(name) if name == worn_var)
            && matches!(b.as_ref(), Expr::Word(name) if name == obj))
        {
            return None;
        }
        if !matches!(carried.as_ref(), Expr::Cmp(CmpOp::Gt, a, b)
            if matches!(a.as_ref(), Expr::Call(name, args) if name == "inv_total"
                && matches!(args.as_slice(), [Expr::Word(inv), Expr::Word(name)] if inv == "inv" && name == obj))
            && matches!(b.as_ref(), Expr::Num(0)))
        {
            return None;
        }
        if !matches!(body.as_slice(), [Stmt::ReturnValue(Expr::Word(name))] if name == obj) {
            return None;
        }
        let id = *src.objs.get(obj)?;
        if !seen.insert(id) {
            return None;
        }
        let Some(Some(Val::Int(level))) = facts.objects.get(&(id, level_param.clone())) else {
            continue;
        };
        if !(0..=99).contains(level) {
            continue;
        }
        tools.push(Tool {
            obj: id,
            skill,
            level: *level,
            worn: false,
        });
        if src.worn_slots.get(&id) == Some(&slot) {
            tools.push(Tool {
                obj: id,
                skill,
                level: *level,
                worn: true,
            });
        }
    }
    (!tools.is_empty()).then_some(tools)
}

fn movement_proofs(src: &Sources) -> HashSet<String> {
    let pinned: HashMap<_, _> = header_blocks(ENGINE_FORCED_PROCS)
        .into_iter()
        .map(|(_, name, params, body)| (name, proc_identity(&params, &body)))
        .collect();
    let matches = |name: &str| {
        matches!(src.procs.get(name).map(Vec::as_slice), Some([proc])
        if pinned.get(name) == Some(&proc.identity))
    };
    let mut proofs = HashSet::new();
    if matches("forcemove") && matches("agility_walk") {
        proofs.insert("forcemove".to_owned());
    }
    if matches("agility_force_move") && matches("agility_walk") {
        proofs.insert("agility_force_move".to_owned());
    }
    if matches("agility_exactmove") {
        proofs.insert("agility_exactmove".to_owned());
    }
    proofs
}

fn proc_block<'a>(src: &'a Sources, name: &str) -> Option<&'a Block> {
    match src.procs.get(name)?.as_slice() {
        [proc] => proc.block.as_ref(),
        _ => None,
    }
}

/// Parsed call-closure discovery is independent of evaluation and does not
/// admit anything. Ambiguity, unknown control flow and every gate are checked
/// again on the selected path before an edge can be emitted.
pub(super) fn reachable_calls(block: &Block, src: &Sources) -> HashSet<String> {
    let mut calls = Vec::new();
    block.body.iter().for_each(|stmt| stmt.calls(&mut calls));
    let mut seen = HashSet::new();
    while let Some(call) = calls.pop() {
        if !seen.insert(call.clone()) {
            continue;
        }
        let body = if let Some(name) = call.strip_prefix('~') {
            proc_block(src, name)
        } else if let Some(name) = call.strip_prefix('@') {
            match Sources::one(&src.labels, name) {
                Lookup::Found(block) => Some(block),
                _ => None,
            }
        } else {
            None
        };
        if let Some(body) = body {
            body.body.iter().for_each(|stmt| stmt.calls(&mut calls));
        }
    }
    seen
}

#[derive(PartialEq, Eq)]
enum End {
    Continue,
    Return,
    Refused,
}

fn has_requirement(needs: &Needs) -> bool {
    needs.members
        || needs
            .skills
            .iter()
            .chain(&needs.items)
            .chain(&needs.worn)
            .chain(&needs.varps)
            .any(|(_, minimum)| *minimum > 0)
}

fn tool_needs(tool: Tool) -> Needs {
    let mut needs = Needs {
        skills: vec![(tool.skill, tool.level)],
        ..Needs::default()
    };
    if tool.worn {
        needs.worn.push((tool.obj, 1));
    } else {
        needs.items.push((tool.obj, 1));
    }
    needs
}

struct Motion<'a> {
    base: Eval<'a>,
    facts: &'a LocFacts,
    proofs: &'a HashSet<String>,
    selectors: &'a HashMap<String, Vec<Tool>>,
    tool: Option<(&'a str, Tool)>,
    here: WorldTile,
    loc_id: i32,
    loc_modified: bool,
    shape: i32,
    moved: bool,
    ticks: i32,
    gate_read: bool,
}

impl Motion<'_> {
    fn run(&mut self, body: &[Stmt], env: &mut Env) -> End {
        for stmt in body {
            let end = self.statement(stmt, env);
            if end != End::Continue {
                return end;
            }
        }
        End::Continue
    }

    fn statement(&mut self, stmt: &Stmt, env: &mut Env) -> End {
        match stmt {
            Stmt::Block(body) => self.run(body, env),
            Stmt::Return(calls) if calls.is_empty() => End::Return,
            Stmt::ReturnValue(value) => {
                if self.value(value, env).is_some() {
                    End::Return
                } else {
                    End::Refused
                }
            }
            Stmt::Def(name, Some(value)) => match self.value(value, env) {
                Some(value) => {
                    env.insert(name.clone(), value);
                    End::Continue
                }
                None => End::Refused,
            },
            Stmt::Def(name, None) => {
                env.remove(name);
                End::Continue
            }
            Stmt::Assign(names, value) if matches!(names.as_slice(), [name] if name.starts_with('$')) => {
                match self.value(value, env) {
                    Some(value) => {
                        env.insert(names[0].clone(), value);
                        End::Continue
                    }
                    None => End::Refused,
                }
            }
            Stmt::If(arms, other) => {
                // Cosmetic-only branches cannot gate the trajectory.
                if arms
                    .iter()
                    .all(|(_, body)| body.iter().all(|s| self.presentation(s)))
                    && other
                        .as_deref()
                        .is_none_or(|body| body.iter().all(|s| self.presentation(s)))
                {
                    return End::Continue;
                }
                for (test, body) in arms {
                    let cond = self.condition(test, env);
                    self.gate_read |= has_requirement(&cond.needs);
                    self.base.needs.merge(cond.needs);
                    match cond.value {
                        Some(true) => return self.run(body, env),
                        Some(false) => {}
                        None => return End::Refused,
                    }
                }
                other
                    .as_deref()
                    .map_or(End::Continue, |body| self.run(body, env))
            }
            Stmt::Jump(name, args) => {
                let Lookup::Found(block) = Sources::one(&self.base.src.labels, name) else {
                    return End::Refused;
                };
                let end = self.invoke(block, args, env);
                if end == End::Refused {
                    end
                } else {
                    End::Return
                }
            }
            Stmt::Call(_, _) if self.presentation(stmt) => End::Continue,
            Stmt::Call(name, args) => self.call(name, args, env),
            _ => End::Refused,
        }
    }

    fn invoke(&mut self, block: &Block, args: &[Expr], env: &Env) -> End {
        if self.base.jumps >= MAX_JUMPS || block.params.len() != args.len() {
            return End::Refused;
        }
        self.base.jumps += 1;
        let mut frame = Env::new();
        for (name, value) in block.params.iter().zip(args) {
            let Some(value) = self.value(value, env) else {
                return End::Refused;
            };
            frame.insert(name.clone(), value);
        }
        self.run(&block.body, &mut frame)
    }

    fn price(&mut self, ticks: i32) -> Option<()> {
        if ticks < 0 {
            return None;
        }
        self.ticks = self.ticks.checked_add(ticks)?;
        Some(())
    }

    fn relocate(&mut self, to: WorldTile) -> Option<()> {
        if !in_world_box(&to) {
            return None;
        }
        self.here = to;
        Some(())
    }

    fn price_and_relocate_chebyshev(&mut self, to: WorldTile) -> Option<()> {
        if to.level != self.here.level {
            return None;
        }
        let distance =
            to.x.checked_sub(self.here.x)?
                .checked_abs()?
                .max(to.z.checked_sub(self.here.z)?.checked_abs()?);
        self.price(distance)?;
        self.relocate(to)?;
        self.moved = true;
        Some(())
    }

    fn call(&mut self, name: &str, args: &[Expr], env: &Env) -> End {
        let result = match name {
            "~forcemove" if self.proofs.contains("forcemove") => match args {
                [dest] => self.value(dest, env).and_then(|value| match value {
                    Val::Coord(to) => self.price_and_relocate_chebyshev(to),
                    _ => None,
                }),
                _ => None,
            },
            "~agility_force_move" if self.proofs.contains("agility_force_move") => match args {
                [xp, animation, dest] => {
                    if !matches!(self.value(xp, env), Some(Val::Int(n)) if n >= 0)
                        || self.value(animation, env).is_none()
                    {
                        return End::Refused;
                    }
                    self.value(dest, env).and_then(|value| match value {
                        Val::Coord(to) => self.price_and_relocate_chebyshev(to),
                        _ => None,
                    })
                }
                _ => None,
            },
            "~agility_exactmove" if self.proofs.contains("agility_exactmove") => match args {
                [animation, animation_delay, delay, start, end, cycle0, cycle1, direction, merge] =>
                {
                    if self.value(animation, env).is_none()
                        || !matches!(self.value(animation_delay, env), Some(Val::Int(n)) if n >= 0)
                    {
                        return End::Refused;
                    }
                    let values = (
                        self.value(delay, env),
                        self.value(start, env),
                        self.value(end, env),
                        self.value(cycle0, env),
                        self.value(cycle1, env),
                        self.value(direction, env),
                        self.value(merge, env),
                    );
                    match values {
                        (
                            Some(Val::Int(delay)),
                            Some(Val::Coord(start)),
                            Some(Val::Coord(end)),
                            Some(Val::Int(cycle0)),
                            Some(Val::Int(cycle1)),
                            Some(Val::Int(direction)),
                            Some(Val::Bool(_)),
                        ) if start == self.here
                            && end.level == self.here.level
                            && delay >= 0
                            && cycle0 >= 0
                            && cycle1 >= cycle0
                            && (0..=3).contains(&direction) =>
                        {
                            delay
                                .checked_add(1)
                                .and_then(|delay| self.price(delay))
                                .and_then(|()| self.relocate(end))
                                .map(|()| self.moved = true)
                        }
                        _ => None,
                    }
                }
                _ => None,
            },
            "~forcemove" | "~agility_exactmove" | "~agility_force_move" => None,
            "p_exactmove" => match args {
                [start, end, cycle0, cycle1, direction] => {
                    let values = (
                        self.value(start, env),
                        self.value(end, env),
                        self.value(cycle0, env),
                        self.value(cycle1, env),
                        self.value(direction, env),
                    );
                    match values {
                        (
                            Some(Val::Coord(start)),
                            Some(Val::Coord(end)),
                            Some(Val::Int(cycle0)),
                            Some(Val::Int(cycle1)),
                            Some(Val::Int(direction)),
                        ) if start == self.here
                            && end.level == self.here.level
                            && cycle0 >= 0
                            && cycle1 >= cycle0
                            && (0..=3).contains(&direction) =>
                        {
                            self.relocate(end).map(|()| self.moved = true)
                        }
                        _ => None,
                    }
                }
                _ => None,
            },
            "p_teleport" | "p_telejump" => match args {
                [dest] => self.value(dest, env).and_then(|value| match value {
                    Val::Coord(to) => self.relocate(to),
                    _ => None,
                }),
                _ => None,
            },
            "p_delay" => match args {
                [delay] => self.value(delay, env).and_then(|value| match value {
                    Val::Int(delay) if delay >= 0 => {
                        delay.checked_add(1).and_then(|n| self.price(n))
                    }
                    _ => None,
                }),
                _ => None,
            },
            "p_arrivedelay" if args.is_empty() => self.price(1),
            // Temporary loc replacements are part of the interaction, not an
            // open-door settle token or permission to walk through scenery.
            "loc_change" => match args {
                [loc, duration] => match (self.value(loc, env), self.value(duration, env)) {
                    (Some(Val::Name(loc)), Some(Val::Int(n)))
                        if n >= 0 && self.base.src.ids.contains_key(&loc) =>
                    {
                        self.loc_modified = true;
                        Some(())
                    }
                    _ => None,
                },
                _ => None,
            },
            _ if name.starts_with('~') => {
                let Some(block) = proc_block(self.base.src, &name[1..]) else {
                    return End::Refused;
                };
                return match self.invoke(block, args, env) {
                    End::Refused => End::Refused,
                    _ => End::Continue,
                };
            }
            _ => None,
        };
        if result.is_some() {
            End::Continue
        } else {
            End::Refused
        }
    }

    fn presentation(&self, stmt: &Stmt) -> bool {
        if let Stmt::Call(name, args) = stmt {
            if name == "stat_advance" {
                return matches!(args.as_slice(), [Expr::Word(skill), Expr::Num(xp)]
                    if STATS.contains(&skill.as_str()) && *xp >= 0);
            }
        }
        matches!(stmt, Stmt::Call(name, _) if matches!(name.as_str(),
            "mes" | "anim" | "sound_synth" | "facesquare" | "loc_anim" |
            "spotanim" | "spotanim_map" | "stat_advance" | "p_stopaction")
            && self.base.src.certified(stmt))
    }

    fn select_tool(&mut self, name: &str) -> Option<Val> {
        let (selector, tool) = self.tool?;
        if selector != name {
            return None;
        }
        self.base.needs.merge(tool_needs(tool));
        Some(Val::Tool)
    }

    fn value(&mut self, expr: &Expr, env: &Env) -> Option<Val> {
        match expr {
            Expr::Arithmetic(op, a, b) => {
                let (Val::Int(a), Val::Int(b)) = (self.value(a, env)?, self.value(b, env)?) else {
                    return None;
                };
                let value = match op {
                    ArithmeticOp::Add => a.checked_add(b),
                    ArithmeticOp::Subtract => a.checked_sub(b),
                    ArithmeticOp::Multiply => a.checked_mul(b),
                    ArithmeticOp::Divide => a.checked_div(b),
                    ArithmeticOp::Modulo => a.checked_rem(b),
                }?;
                Some(Val::Int(value))
            }
            Expr::Word(name) if name == "coord" => Some(Val::Coord(self.here)),
            Expr::Word(name) if name == "loc_shape" => Some(Val::Int(self.shape)),
            Expr::Word(name) if name == "null" => Some(Val::Name("null".to_owned())),
            Expr::Word(name) if name == "loc_type" => {
                (!self.loc_modified).then_some(Val::Loc(LocRef::Itself))
            }
            Expr::Word(name)
                if name.starts_with('~') && self.selectors.contains_key(&name[1..]) =>
            {
                self.select_tool(&name[1..])
            }
            Expr::Call(name, args)
                if args.is_empty()
                    && matches!(
                        name.as_str(),
                        "coord" | "loc_coord" | "loc_angle" | "loc_shape" | "loc_type"
                    ) =>
            {
                match name.as_str() {
                    "coord" => Some(Val::Coord(self.here)),
                    "loc_coord" => Some(Val::Coord(self.base.at)),
                    "loc_angle" => Some(Val::Int(self.base.angle)),
                    "loc_shape" => Some(Val::Int(self.shape)),
                    _ => (!self.loc_modified).then_some(Val::Loc(LocRef::Itself)),
                }
            }
            Expr::Call(name, args)
                if name.starts_with('~') && self.selectors.contains_key(&name[1..]) =>
            {
                if !args.is_empty() {
                    return None;
                }
                self.select_tool(&name[1..])
            }
            Expr::Call(name, args) if name == "calc" => {
                let [value] = args.as_slice() else {
                    return None;
                };
                self.value(value, env)
            }
            Expr::Call(name, args)
                if matches!(
                    name.as_str(),
                    "multiply" | "add" | "sub" | "divide" | "mod" | "min" | "max"
                ) =>
            {
                let [a, b] = args.as_slice() else {
                    return None;
                };
                let (Val::Int(a), Val::Int(b)) = (self.value(a, env)?, self.value(b, env)?) else {
                    return None;
                };
                let value = match name.as_str() {
                    "multiply" => a.checked_mul(b),
                    "add" => a.checked_add(b),
                    "sub" => a.checked_sub(b),
                    "divide" => a.checked_div(b),
                    "mod" => a.checked_rem(b),
                    "min" => Some(a.min(b)),
                    _ => Some(a.max(b)),
                }?;
                Some(Val::Int(value))
            }
            Expr::Call(name, args) if name == "movecoord" => {
                let [coord, dx, dy, dz] = args.as_slice() else {
                    return None;
                };
                let (Val::Coord(coord), Val::Int(dx), Val::Int(dy), Val::Int(dz)) = (
                    self.value(coord, env)?,
                    self.value(dx, env)?,
                    self.value(dy, env)?,
                    self.value(dz, env)?,
                ) else {
                    return None;
                };
                Some(Val::Coord(WorldTile {
                    x: coord.x.checked_add(dx)?,
                    z: coord.z.checked_add(dz)?,
                    level: coord.level.checked_add(dy)?,
                }))
            }
            Expr::Call(name, args) if matches!(name.as_str(), "coordx" | "coordy" | "coordz") => {
                let [coord] = args.as_slice() else {
                    return None;
                };
                let Val::Coord(coord) = self.value(coord, env)? else {
                    return None;
                };
                Some(Val::Int(match name.as_str() {
                    "coordx" => coord.x,
                    "coordy" => coord.level,
                    _ => coord.z,
                }))
            }
            Expr::Call(name, args) if name == "loc_param" && !self.loc_modified => {
                let [Expr::Word(param)] = args.as_slice() else {
                    return None;
                };
                self.facts.params.get(param)?.clone()
            }
            Expr::Call(name, args) if name == "~check_axis" => {
                if !self.base.supported.contains("check_axis") {
                    return None;
                }
                let [coord, loc, angle] = args.as_slice() else {
                    return None;
                };
                let (Val::Coord(coord), Val::Coord(loc), Val::Int(angle)) = (
                    self.value(coord, env)?,
                    self.value(loc, env)?,
                    self.value(angle, env)?,
                ) else {
                    return None;
                };
                Some(Val::Bool(check_axis(coord, loc, angle)))
            }
            Expr::Call(name, args) if name == "~check_axis_locactive" => {
                if !self.base.supported.contains("check_axis_locactive") {
                    return None;
                }
                let [coord] = args.as_slice() else {
                    return None;
                };
                let Val::Coord(coord) = self.value(coord, env)? else {
                    return None;
                };
                Some(Val::Bool(check_axis(coord, self.base.at, self.base.angle)))
            }
            _ => self.base.value(expr, env),
        }
    }

    fn condition(&mut self, expr: &Expr, env: &Env) -> Cond {
        match expr {
            Expr::Or(a, b) | Expr::And(a, b) => {
                let (a, b) = (self.condition(a, env), self.condition(b, env));
                if matches!(expr, Expr::Or(..)) {
                    match (a.value, b.value) {
                        (Some(true), Some(true)) => cheaper(a, b),
                        (Some(true), _) => a,
                        (_, Some(true)) => b,
                        (Some(false), Some(false)) => both(false, a, b),
                        _ => Cond::unknown(),
                    }
                } else {
                    match (a.value, b.value) {
                        (Some(false), Some(false)) => cheaper(a, b),
                        (Some(false), _) => a,
                        (_, Some(false)) => b,
                        (Some(true), Some(true)) => both(true, a, b),
                        _ => Cond::unknown(),
                    }
                }
            }
            Expr::Cmp(op, lhs, rhs) => {
                let Some(right) = self.value(rhs, env) else {
                    return Cond::unknown();
                };
                let rewritten = match &right {
                    Val::Int(number) => Some(Expr::Num(*number)),
                    Val::Name(name) => Some(Expr::Word(name.clone())),
                    _ => None,
                };
                let gate = matches!(lhs.as_ref(), Expr::Word(name) if name.starts_with('%') || name == "map_members")
                    || matches!(lhs.as_ref(), Expr::Call(name, _) if matches!(name.as_str(), "stat" | "inv_total" | "inv_getobj"));
                if gate {
                    return rewritten
                        .map_or_else(Cond::unknown, |rhs| self.base.compare(*op, lhs, &rhs, env));
                }
                let Some(left) = self.value(lhs, env) else {
                    return Cond::unknown();
                };
                let value = match (&left, &right) {
                    (Val::Int(a), Val::Int(b)) => match op {
                        CmpOp::Eq => a == b,
                        CmpOp::Ne => a != b,
                        CmpOp::Lt => a < b,
                        CmpOp::Le => a <= b,
                        CmpOp::Gt => a > b,
                        CmpOp::Ge => a >= b,
                    },
                    (Val::Tool, Val::Name(name)) | (Val::Name(name), Val::Tool)
                        if name == "null" =>
                    {
                        let value = match op {
                            CmpOp::Eq => false,
                            CmpOp::Ne => true,
                            _ => return Cond::unknown(),
                        };
                        let Some((_, tool)) = self.tool else {
                            return Cond::unknown();
                        };
                        return Cond {
                            value: Some(value),
                            needs: tool_needs(tool),
                        };
                    }
                    (Val::Coord(a), Val::Coord(b)) => match op {
                        CmpOp::Eq => a == b,
                        CmpOp::Ne => a != b,
                        _ => return Cond::unknown(),
                    },
                    (Val::Bool(a), Val::Bool(b)) => match op {
                        CmpOp::Eq => a == b,
                        CmpOp::Ne => a != b,
                        _ => return Cond::unknown(),
                    },
                    (Val::Loc(LocRef::Itself), Val::Name(name))
                    | (Val::Name(name), Val::Loc(LocRef::Itself)) => {
                        let Some(&id) = self.base.src.ids.get(name) else {
                            return Cond::unknown();
                        };
                        match op {
                            CmpOp::Eq => self.loc_id == id,
                            CmpOp::Ne => self.loc_id != id,
                            _ => return Cond::unknown(),
                        }
                    }
                    _ => return Cond::unknown(),
                };
                Cond::known(value)
            }
            _ => match self.value(expr, env) {
                Some(Val::Bool(value)) => Cond::known(value),
                _ => Cond::unknown(),
            },
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn forced_move_edges(
    root: &Path,
    src: &Sources,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    defs: &LocDefs,
    graph: &mut TransportGraph,
    collision: &WorldCollision,
    skipped: &mut HashMap<&'static str, usize>,
    observable: &ObservableGates,
    audit: &mut VarpGateAudit,
) {
    let facts = Facts::read(root, src);
    let proofs = movement_proofs(src);
    let supported = src.supported_procs().0;
    let selectors: HashMap<_, _> = src
        .procs
        .iter()
        .filter_map(|(name, definitions)| {
            let [definition] = definitions.as_slice() else {
                return None;
            };
            Some((
                name.clone(),
                tool_selector(definition.block.as_ref()?, src, &facts)?,
            ))
        })
        .collect();
    let mut names: HashMap<i32, Vec<&str>> = HashMap::new();
    for (name, &id) in ids {
        names.entry(id).or_default().push(name);
    }
    let mut locs: Vec<_> = positions.keys().copied().collect();
    locs.sort_unstable();
    // Existing dedicated producers retain their explicitly supported classes.
    // Their edges are not duplicated by the newly inferred class producer.
    let covered: HashSet<_> = graph
        .edges
        .iter()
        .filter(|edge| edge.kind == TransportKind::AgilityShortcut)
        .map(|edge| (edge.loc_id, edge.option))
        .collect();
    for id in locs {
        let Some(Some(loc_facts)) = facts.locs.get(&id) else {
            continue;
        };
        let Some(def) = defs.loc(id) else {
            continue;
        };
        for option in 1..=5 {
            if covered.contains(&(id, option)) {
                continue;
            }
            let handlers: Vec<_> = names
                .get(&id)
                .into_iter()
                .flatten()
                .map(|name| Sources::one(&src.oplocs[(option - 1) as usize], name))
                .filter(|handler| !matches!(handler, Lookup::Missing))
                .collect();
            let handler = match handlers.as_slice() {
                [Lookup::Found(block)] => *block,
                [] => match loc_facts.category.as_ref().map(|category| {
                    Sources::one(&src.oplocs[(option - 1) as usize], &format!("_{category}"))
                }) {
                    Some(Lookup::Found(block)) => block,
                    Some(Lookup::Unusable) => {
                        bump(skipped, SKIP_FORCED_CONFLICT, 1);
                        continue;
                    }
                    _ => continue,
                },
                _ => {
                    bump(skipped, SKIP_FORCED_CONFLICT, 1);
                    continue;
                }
            };
            let calls = reachable_calls(handler, src);
            let member_path = src.has_member_reads(handler);
            if !calls.iter().any(|call| {
                matches!(
                    call.as_str(),
                    "~forcemove" | "~agility_exactmove" | "~agility_force_move" | "p_exactmove"
                )
            }) {
                continue;
            }
            let selector_names: Vec<_> = selectors
                .keys()
                .filter(|name| calls.contains(&format!("~{name}")))
                .collect();
            if selector_names.len() > 1 {
                bump(skipped, SKIP_FORCED_UNPROVEN, 1);
                continue;
            }
            let choices: Vec<_> = match selector_names.as_slice() {
                [name] => selectors[*name]
                    .iter()
                    .map(|&tool| Some((name.as_str(), tool)))
                    .collect(),
                _ => vec![None],
            };
            let before = graph.edges.len();
            let mut proven = false;
            for placement in &positions[&id] {
                if !matches!(placement.shape, 10 | 11 | 22) {
                    continue;
                }
                let at = WorldTile {
                    x: placement.x,
                    z: placement.z,
                    level: placement.level,
                };
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
                        if !collision.standable(stand)
                            || !approach.can_operate(
                                at,
                                stand,
                                collision.walkable_word(x, z, stand.level) as i32,
                            )
                        {
                            continue;
                        }
                        for &tool in &choices {
                            let mut motion = Motion {
                                base: Eval {
                                    src,
                                    supported: &supported,
                                    at,
                                    stand: Some(stand),
                                    angle: placement.angle,
                                    needs: Needs::default(),
                                    relative_landing_safe: false,
                                    jumps: 0,
                                },
                                facts: loc_facts,
                                proofs: &proofs,
                                selectors: &selectors,
                                tool,
                                here: stand,
                                loc_id: id,
                                loc_modified: false,
                                shape: placement.shape,
                                moved: false,
                                ticks: 1,
                                gate_read: false,
                            };
                            let mut env = Env::new();
                            if motion.run(&handler.body, &mut env) == End::Refused
                                || !motion.moved
                                || motion.here == stand
                                || !collision.standable(motion.here)
                            {
                                continue;
                            }
                            let needs = motion.base.needs;
                            if !motion.gate_read || !has_requirement(&needs) {
                                continue;
                            }
                            let mut edge = gated_edge(
                                TransportKind::AgilityShortcut,
                                id,
                                at,
                                motion.here,
                                needs,
                            );
                            edge.takeoff = Some(stand);
                            edge.option = option;
                            edge.ticks = motion.ticks;
                            proven = true;
                            if observable.admit_edge(graph, edge, audit) && member_path {
                                audit
                                    .record_member_path(graph.edges.last().expect("admitted edge"));
                            }
                        }
                    }
                }
            }
            if !proven {
                bump(skipped, SKIP_FORCED_UNPROVEN, 1);
            } else if graph.edges.len() == before {
                bump(skipped, SKIP_FORCED_UNOBSERVABLE, 1);
            }
        }
    }
}
