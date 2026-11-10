use super::super::execute::state_eq;
use super::*;

impl Exec<'_, '_> {
    pub(super) fn npc(
        &mut self,
        st: &mut State,
        name: &'static str,
        slot: usize,
    ) -> ExecResult<Step> {
        match name {
            // NpcOps.ts:57-68; World.ts:getNextNid; EntityList.ts:21-33.
            // A full live list throws before the active slot or population changes.
            "npc_add" => {
                let args = pop_array::<3>(st)?;
                self.validate_coord(st, args[0])?;
                self.npc_type_proof(st, args[1])?;
                self.positive(st, args[2], true)?;
                match self.machine.world.npc_spare_capacity() {
                    Tri::No => return Err(engine(st.at, "NPC list is full").into()),
                    Tri::Yes => self.npc_added(st, slot)?,
                    Tri::Unknown => {
                        let live = self.observe(
                            st,
                            AtomKind::Live("npc_capacity"),
                            CondClass::Live,
                            (0, 1),
                            true,
                        )?;
                        let mut children = self.split(
                            st,
                            Pred::Cmp {
                                lhs: live,
                                op: CmpOp::Eq,
                                rhs: Value::Known(1),
                            },
                        )?;
                        for child in &mut children {
                            if pop(child)? == Value::Known(1) {
                                self.npc_added(child, slot)?;
                            } else {
                                child.forced_end = Some(engine(st.at, "NPC list is full"));
                            }
                        }
                        return Ok(Step::Split(children));
                    }
                }
            }
            // NpcOps.ts:351-382. Failure preserves the old slot.
            "npc_find" => {
                let args = pop_array::<4>(st)?;
                self.npc_search_args(st, &args, true)?;
                let live = self.observe(
                    st,
                    AtomKind::Live("npc_find"),
                    CondClass::Live,
                    (0, 1),
                    true,
                )?;
                let mut children = self.split(
                    st,
                    Pred::Cmp {
                        lhs: live,
                        op: CmpOp::Eq,
                        rhs: Value::Known(1),
                    },
                )?;
                for child in &mut children {
                    if child.ints.last() == Some(&Value::Known(1)) {
                        let item = self.observe(
                            child,
                            AtomKind::Live("npc_item"),
                            CondClass::Live,
                            (0, i32::MAX),
                            true,
                        )?;
                        child.active_npc[slot] = Some(item);
                    }
                }
                return Ok(Step::Split(children));
            }
            // NpcOps.ts:340-348,418-437. Resetting an iterator creates a new identity.
            "npc_findall" | "npc_findallany" | "npc_huntall" => {
                let args = pop_args(st, if name == "npc_findall" { 4 } else { 3 })?;
                self.npc_search_args(st, &args, name == "npc_findall")?;
                let serial = st
                    .npc_iter
                    .as_ref()
                    .map_or(0, |i| i.serial.saturating_add(1));
                st.npc_iter = Some(NpcIter {
                    site: st.at,
                    age: IterAge::Fresh,
                    population_epoch: st.population_epoch,
                    serial,
                });
                st.loops.clear();
            }
            "npc_findnext" => return self.npc_next(st, slot),
            "nc_name" => {
                let value = pop(st)?;
                self.npc_type_proof(st, value)?;
                self.push_text(st, Text::Unknown(st.at))?;
            }
            other => {
                if st.active_npc[slot].is_none() {
                    return Err(engine(st.at, "no active NPC").into());
                }
                match other {
                    "npc_coord" | "npc_type" | "npc_category" | "npc_getmode" => {
                        let bounds = if other == "npc_coord" || other == "npc_type" {
                            (0, i32::MAX)
                        } else {
                            (i32::MIN, i32::MAX)
                        };
                        let v =
                            self.observe(st, AtomKind::Live(other), CondClass::Live, bounds, true)?;
                        self.push(st, v)?;
                    }
                    "npc_name" => self.push_text(st, Text::Unknown(st.at))?,
                    "npc_del" => {
                        st.population_epoch = st.population_epoch.saturating_add(1);
                        self.effect(st, Effect::Npc { command: name })?;
                    }
                    "npc_say" => {
                        pop_text(st)?;
                        self.effect(st, Effect::Npc { command: name })?;
                    }
                    "npc_walk" | "npc_facesquare" => {
                        let coord = pop(st)?;
                        self.validate_coord(st, coord)?;
                        self.effect(st, Effect::Npc { command: name })?;
                    }
                    "npc_setmode" => {
                        let value = pop(st)?;
                        let mode = self.required(st, value)?;
                        if !(-1..=46).contains(&mode) {
                            return Err(engine(st.at, "invalid NPC mode").into());
                        }
                        self.effect(st, Effect::Npc { command: name })?;
                    }
                    "npc_anim" => {
                        let args = pop_array::<2>(st)?;
                        self.not_null(st, args[1])?;
                        self.effect(st, Effect::Npc { command: name })?;
                    }
                    "spotanim_npc" => {
                        let args = pop_array::<3>(st)?;
                        self.not_null(st, args[0])?;
                        self.not_null(st, args[1])?;
                        self.not_null(st, args[2])?;
                        self.effect(st, Effect::Npc { command: name })?;
                    }
                    _ => return Err(unsupported(st.at, "unsupported NPC operation").into()),
                }
            }
        }
        Ok(Step::Next)
    }
    pub(super) fn npc_type_proof(&self, st: &State, value: Value) -> Result<(), End> {
        if let Some(n) = known(st, &self.arena, value) {
            if n >= 0 {
                return Ok(());
            }
            return Err(engine(st.at, "invalid NPC type"));
        }
        if self
            .arena
            .atom_of(value)
            .is_some_and(|a| matches!(&a.kind,AtomKind::Live(n) if *n=="npc_type"))
        {
            Ok(())
        } else {
            Err(unsupported(st.at, "NPC type validation needs a proof"))
        }
    }
    fn npc_search_args(&self, st: &State, args: &[Value], typed: bool) -> Result<(), End> {
        self.validate_coord(st, args[0])?;
        if typed {
            self.npc_type_proof(st, args[1])?;
        }
        let n = args.len();
        self.not_null(st, args[n - 2])?;
        let vis = self.required(st, args[n - 1])?;
        if !(0..3).contains(&vis) {
            return Err(engine(st.at, "invalid NPC hunt visibility"));
        }
        Ok(())
    }
    fn npc_added(&mut self, st: &mut State, slot: usize) -> ExecResult<()> {
        let item = self.observe(
            st,
            AtomKind::Live("npc_item"),
            CondClass::Live,
            (0, i32::MAX),
            true,
        )?;
        st.active_npc[slot] = Some(item);
        st.population_epoch = st.population_epoch.saturating_add(1);
        self.effect(st, Effect::Npc { command: "npc_add" })
    }
    fn npc_next(&mut self, st: &mut State, slot: usize) -> ExecResult<Step> {
        // NpcOps.ts:445-456; ScriptIterators.ts:328-362. The no-item arm never clears the slot.
        let Some(iter) = &st.npc_iter else {
            self.push(st, Value::Known(0))?;
            return Ok(Step::Next);
        };
        let (age, serial, population) = (iter.age, iter.serial, iter.population_epoch);
        if age == IterAge::MaybeStale {
            let suspended = self.observe(
                st,
                AtomKind::Live("npc_iterator_maybe_suspended"),
                CondClass::Live,
                (0, 1),
                true,
            )?;
            let children = self.split(
                st,
                Pred::Cmp {
                    lhs: suspended,
                    op: CmpOp::Eq,
                    rhs: Value::Known(1),
                },
            )?;
            let mut outcomes = Vec::new();
            for mut child in children {
                let stale = pop(&mut child)? == Value::Known(1);
                if let Some(iter) = &mut child.npc_iter {
                    iter.age = if stale {
                        IterAge::Stale
                    } else {
                        IterAge::Fresh
                    };
                }
                match self.npc_next(&mut child, slot)? {
                    Step::Split(mut branches) => {
                        self.reserve(&mut outcomes, branches.len(), st.at)?;
                        outcomes.append(&mut branches);
                    }
                    Step::End(end) => {
                        child.forced_end = Some(end);
                        self.reserve(&mut outcomes, 1, st.at)?;
                        outcomes.push(child);
                    }
                    Step::Next | Step::Returned => {
                        self.reserve(&mut outcomes, 1, st.at)?;
                        outcomes.push(child);
                    }
                }
            }
            return Ok(Step::Split(outcomes));
        }
        if age != IterAge::Fresh {
            let live = self.observe(
                st,
                AtomKind::Live("stale_npc_iterator"),
                CondClass::Live,
                (0, 1),
                true,
            )?;
            let mut children = self.split(
                st,
                Pred::Cmp {
                    lhs: live,
                    op: CmpOp::Eq,
                    rhs: Value::Known(1),
                },
            )?;
            for child in &mut children {
                let has_item = pop(child)? == Value::Known(1);
                if has_item {
                    child.forced_end = Some(engine(st.at, "old NPC iterator"));
                } else {
                    self.push(child, Value::Known(0))?;
                }
            }
            return Ok(Step::Split(children));
        }
        let bound = if population == st.population_epoch {
            self.npc_pass_bound(st)?
        } else {
            None
        };
        if let Some(pass) = bound {
            if let Some(index) = st
                .loops
                .iter()
                .position(|v| v.site == st.at && v.serial == serial)
            {
                let visit = &st.loops[index];
                if visit.slot == slot && state_eq(st, &visit.baseline, self.view, Some(slot)) {
                    // The >=1-item exit retains the last active NPC. It is not replaced by the
                    // initial empty exit, which may still have no active NPC at all (A2).
                    st.ops.max = visit.cost.max;
                    self.push(st, Value::Known(0))?;
                    self.audit.loop_summaries += 1;
                    st.transfers.loop_summaries += 1;
                    return Ok(Step::Next);
                }
                let baseline = self.copy_without_loops(st)?;
                *st.loops[index].baseline = baseline;
            } else {
                let baseline = self.copy_without_loops(st)?;
                let max = st
                    .ops
                    .max
                    .saturating_add(self.machine.profile.npc_capacity.saturating_mul(pass));
                self.charge(std::mem::size_of::<State>(), st.at)?;
                self.reserve(&mut st.loops, 1, st.at)?;
                st.loops.push(LoopVisit {
                    site: st.at,
                    serial,
                    baseline: Box::new(baseline),
                    cost: OpCount {
                        min: st.ops.min,
                        max,
                    },
                    slot,
                });
                // This structural upper bound is installed BEFORE either exit can run. It
                // includes every closed pass, even before dynamic equality proves a summary.
                st.ops.max = max;
                self.audit.loop_allowance_before_exit += 1;
                st.transfers.loop_allowance_before_exit += 1;
            }
        }
        let live = self.observe(
            st,
            AtomKind::Live("npc_iterator_item"),
            CondClass::Live,
            (0, 1),
            true,
        )?;
        let mut children = self.split(
            st,
            Pred::Cmp {
                lhs: live,
                op: CmpOp::Eq,
                rhs: Value::Known(1),
            },
        )?;
        for child in &mut children {
            if child.ints.last() == Some(&Value::Known(1)) {
                let item = self.observe(
                    child,
                    AtomKind::Live("npc_item"),
                    CondClass::Live,
                    (0, i32::MAX),
                    true,
                )?;
                child.active_npc[slot] = Some(item);
            }
        }
        Ok(Step::Split(children))
    }
    fn npc_pass_bound(&mut self, st: &State) -> ExecResult<Option<u32>> {
        let Some(script) = self.machine.program.script(st.at.script) else {
            return Ok(None);
        };
        #[derive(Clone, Copy)]
        struct Scan {
            pc: usize,
            next: usize,
            max: Option<u32>,
        }
        let count = script.code.len();
        self.charge(
            count * (std::mem::size_of::<Option<Option<u32>>>() + std::mem::size_of::<Scan>() + 1),
            st.at,
        )?;
        let mut visiting = vec![false; count];
        let mut memo: Vec<Option<Option<u32>>> = vec![None; count];
        let mut stack = Vec::with_capacity(count);
        if st.at.pc as usize + 1 >= count {
            return Ok(None);
        }
        stack.push(Scan {
            pc: st.at.pc as usize + 1,
            next: 0,
            max: None,
        });
        // An explicit DFS stack keeps even adversarial scripts off the native call stack.
        while let Some(scan) = stack.last().copied() {
            let Some(ins) = script.code.get(scan.pc) else {
                return Ok(None);
            };
            let Some(name) = self.machine.program.opcode(ins.opcode) else {
                return Ok(None);
            };
            visiting[scan.pc] = true;
            let offset = ins.int().unwrap_or(0);
            let target = |off: i32| usize::try_from(scan.pc as i64 + 1 + i64::from(off)).ok();
            let child = match name {
                "RETURN" => None,
                "BRANCH" => {
                    if scan.next == 0 {
                        target(offset)
                    } else {
                        None
                    }
                }
                n if n.starts_with("BRANCH_") => match scan.next {
                    0 => Some(scan.pc + 1),
                    1 => target(offset),
                    _ => None,
                },
                "SWITCH" => {
                    if scan.next == 0 {
                        Some(scan.pc + 1)
                    } else {
                        script
                            .switches
                            .get(offset as usize)
                            .and_then(|table| table.values().nth(scan.next - 1))
                            .and_then(|off| target(*off))
                    }
                }
                _ => {
                    if scan.next == 0 {
                        Some(scan.pc + 1)
                    } else {
                        None
                    }
                }
            };
            if let Some(child) = child {
                if let Some(last) = stack.last_mut() {
                    last.next += 1;
                }
                let value = if child == st.at.pc as usize {
                    Some(Some(1))
                } else if child >= count {
                    Some(None)
                } else if visiting[child] {
                    return Ok(None);
                } else {
                    memo[child]
                };
                if let Some(value) = value {
                    if let (Some(cost), Some(last)) = (value, stack.last_mut()) {
                        last.max = Some(last.max.map_or(cost, |old| old.max(cost)));
                    }
                } else {
                    stack.push(Scan {
                        pc: child,
                        next: 0,
                        max: None,
                    });
                }
                continue;
            }
            let command = crate::commands::lookup(name).map(|row| row.name);
            if scan.max.is_some() && matches!(command, Some("npc_add" | "npc_del")) {
                // The capacity proof needs a stable population. Refuse this
                // precondition at the step, before an unbounded live branch tree.
                return Err(unsupported(st.at, "NPC loop can change its population").into());
            }
            if scan.max.is_some()
                && (matches!(
                    name,
                    "GOSUB" | "GOSUB_WITH_PARAMS" | "JUMP" | "JUMP_WITH_PARAMS"
                ) || matches!(
                    command,
                    Some(
                        "npc_findall"
                            | "npc_findallany"
                            | "npc_huntall"
                            | "p_delay"
                            | "p_arrivedelay"
                            | "npc_findnext"
                    )
                ))
            {
                return Ok(None);
            }
            let value = scan.max.map(|cost| cost.saturating_add(1));
            memo[scan.pc] = Some(value);
            visiting[scan.pc] = false;
            stack.pop();
            if let Some(parent) = stack.last_mut() {
                if let Some(cost) = value {
                    parent.max = Some(parent.max.map_or(cost, |old| old.max(cost)));
                }
            } else {
                return Ok(value);
            }
        }
        Ok(None)
    }
}
