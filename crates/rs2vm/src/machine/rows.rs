use super::execute::{
    engine, known, pop, pop_args, pop_array, pop_text, unsupported, Exec, ExecResult, Step,
};
use super::transfers::world_end;
use super::*;
use crate::program::ConfigValue;
use crate::world::{LocInfo, Tri};

mod layout;
mod npc;
mod number;

impl Exec<'_, '_> {
    pub(super) fn command(
        &mut self,
        st: &mut State,
        opcode: &str,
        operand: i32,
    ) -> ExecResult<Step> {
        let Some(row) = crate::commands::lookup(opcode) else {
            return Err(
                unsupported(st.at, "opcode is outside the closed command inventory").into(),
            );
        };
        let name = row.name;
        if operand != 0
            && !matches!(
                name,
                "loc_coord" | "loc_type" | "loc_category" | "loc_findnext"
            )
        {
            return Err(unsupported(st.at, "unsupported secondary command").into());
        }
        let slot = usize::from(operand != 0);
        if matches!(
            name,
            "add"
                | "sub"
                | "multiply"
                | "divide"
                | "modulo"
                | "testbit"
                | "getbit_range"
                | "clearbit"
                | "clearbit_range"
                | "setbit_range_toint"
                | "movecoord"
                | "coordx"
                | "coordz"
                | "distance"
                | "map_findsquare"
        ) {
            return self.number(st, name);
        }
        if matches!(
            name,
            "string_length"
                | "lowercase"
                | "split_init"
                | "split_pagecount"
                | "split_linecount"
                | "split_get"
                | "split_getanim"
        ) {
            return self.layout(st, name);
        }
        if name.starts_with("npc_") || matches!(name, "spotanim_npc" | "nc_name") {
            return self.npc(st, name, slot);
        }
        match name {
            // PlayerOps.ts:440-452. The two commands have the same zone-allocation guard.
            "p_teleport" | "p_telejump" => {
                let to = pop(st)?;
                return self.move_to(
                    st,
                    to,
                    if name == "p_teleport" {
                        MoveKind::Teleport
                    } else {
                        MoveKind::Telejump
                    },
                );
            }
            // PlayerOps.ts:939-947: exactMove preserves the player's current height plane.
            "p_exactmove" => {
                let args = pop_array::<5>(st)?;
                self.validate_coord(st, args[0])?;
                self.validate_coord(st, args[1])?;
                let to = match (known(st, &self.arena, args[1]), st.player) {
                    (Some(end), PlayerPos::Tile(cur)) => {
                        Value::Known(Coord::new(Coord(end).x(), cur.level(), Coord(end).z()).0)
                    }
                    _ => {
                        return Err(unsupported(
                            st.at,
                            "exact movement needs a proved current level",
                        )
                        .into())
                    }
                };
                return self.move_to(
                    st,
                    to,
                    MoveKind::Exact {
                        start: args[0],
                        end: args[1],
                        start_cycle: args[2],
                        end_cycle: args[3],
                        direction: args[4],
                    },
                );
            }
            // PlayerOps.ts:359-380. Delay invalidation is one shared transfer.
            "p_delay" => {
                let value = pop(st)?;
                let n = self.required(st, value)?;
                if n == -1 {
                    return Err(engine(st.at, "null delay").into());
                }
                if n < 0 {
                    return Err(unsupported(st.at, "negative delay timing").into());
                }
                self.suspend(st, (n as u32).saturating_add(1), false)?;
            }
            "p_arrivedelay" => self.suspend(st, 1, true)?,
            // PlayerOps.ts:655-657,724-727; Player.ts:760-813,2222-2229.
            "if_openchat" => {
                if st.modal.main.is_some() || st.modal.side.is_some() {
                    return Err(unsupported(st.at, "chat open with another modal slot open").into());
                }
                let value = pop(st)?;
                let id = self.not_null(st, value)?;
                st.modal.chat = Some(id);
                self.effect(
                    st,
                    Effect::Modal {
                        state: st.modal.clone(),
                    },
                )?;
            }
            "if_close" => self.close(st, true)?,
            // PlayerOps.ts:425-427; Player.ts:819-825. Stop at input, not at its cancelled tail.
            "p_pausebutton" => {
                if st.modal == Modal::default() {
                    return Err(unsupported(st.at, "pause has no modal to cancel").into());
                }
                self.close(st, true)?;
                self.audit.cancellations += 1;
                st.transfers.cancellations += 1;
                return Ok(Step::End(End::AwaitingInput));
            }
            "uid" => {
                let value = self.observe(
                    st,
                    AtomKind::Uid,
                    CondClass::Opaque,
                    (i32::MIN, i32::MAX),
                    false,
                )?;
                self.push(st, value)?;
            }
            "p_finduid" => {
                if !st.protected {
                    return Err(
                        unsupported(st.at, "p_finduid without protected entry access").into(),
                    );
                }
                let uid = pop(st)?;
                let own = self.observe(
                    st,
                    AtomKind::Uid,
                    CondClass::Opaque,
                    (i32::MIN, i32::MAX),
                    false,
                )?;
                let same = uid == own
                    || matches!(
                        domains::recorded(
                            &st.conds,
                            &Pred::Cmp {
                                lhs: uid,
                                op: CmpOp::Eq,
                                rhs: own
                            }
                        ),
                        Some(true)
                    );
                if !same {
                    return Err(
                        unsupported(st.at, "p_finduid does not prove the entry player").into(),
                    );
                }
                st.protected = true;
                self.push(st, Value::Known(1))?;
            }
            // PlayerOps.ts:149-181,890-904,1102-1105. Starting never runs the target.
            "queue" | "longqueue" => {
                let args = pop_args(st, if name == "queue" { 3 } else { 4 })?;
                let id = self.required(st, args[0])?;
                if id < 0 || self.machine.program.script(ScriptId(id as u32)).is_none() {
                    return Err(engine(st.at, "queue script does not exist").into());
                }
                self.effect(
                    st,
                    Effect::Defer {
                        kind: DeferKind::Start,
                        command: name,
                    },
                )?;
            }
            "settimer" => {
                let Text::Lit(types) = pop_text(st)? else {
                    return Err(unsupported(st.at, "unknown timer argument types").into());
                };
                for ty in types.chars().rev() {
                    if ty == 's' {
                        pop_text(st)?;
                    } else {
                        for _ in 0..ty.len_utf16() {
                            pop(st)?;
                        }
                    }
                }
                let args = pop_array::<2>(st)?;
                let id = self.required(st, args[0])?;
                if id < 0 || self.machine.program.script(ScriptId(id as u32)).is_none() {
                    return Err(engine(st.at, "timer script does not exist").into());
                }
                self.effect(
                    st,
                    Effect::Defer {
                        kind: DeferKind::Start,
                        command: name,
                    },
                )?;
            }
            "cleartimer" | "clearqueue" => {
                pop(st)?;
                self.effect(
                    st,
                    Effect::Defer {
                        kind: DeferKind::Cancel,
                        command: name,
                    },
                )?;
            }
            "gettimer" => {
                let value = pop(st)?;
                let id = self.required(st, value)?;
                if id < 0 || self.machine.program.script(ScriptId(id as u32)).is_none() {
                    return Err(engine(st.at, "timer script does not exist").into());
                }
                let v = self.observe(
                    st,
                    AtomKind::Live(name),
                    CondClass::Live,
                    (i32::MIN, i32::MAX),
                    true,
                )?;
                self.push(st, v)?;
            }
            "mes" => {
                let text = pop_text(st)?;
                self.effect(st, Effect::Message { text })?;
            }
            "text_gender" => {
                pop_text(st)?;
                pop_text(st)?;
                self.push_text(st, Text::Unknown(st.at))?;
            }
            "coord" => {
                let v = self.position(st)?;
                self.push(st, v)?;
            }
            "map_members" => {
                let v =
                    self.observe(st, AtomKind::Members, CondClass::Requirement, (0, 1), false)?;
                self.push(st, v)?;
            }
            "map_clock" => {
                let v = self.observe(
                    st,
                    AtomKind::Live(name),
                    CondClass::Live,
                    (i32::MIN, i32::MAX),
                    true,
                )?;
                self.push(st, v)?;
            }
            "stat" => {
                let v = pop(st)?;
                let id = self.required(st, v)?;
                if !(0..21).contains(&id) {
                    return Err(engine(st.at, "invalid stat id").into());
                }
                let v = self.observe(
                    st,
                    AtomKind::Stat { id, base: false },
                    if st.epoch == 0 {
                        CondClass::Requirement
                    } else {
                        CondClass::Live
                    },
                    (i32::MIN, i32::MAX),
                    false,
                )?;
                self.push(st, v)?;
            }
            "inv_total" | "inv_totalcat" | "inv_getobj" => {
                let args = pop_array::<2>(st)?;
                let inv = self.not_null(st, args[0])?;
                if !self.machine.program.packs.invs.contains_key(&inv) {
                    return Err(engine(st.at, "invalid inventory").into());
                }
                let key = self.required(st, args[1])?;
                let value = if name == "inv_total" && key == -1 {
                    Value::Known(0)
                } else {
                    if key < 0 {
                        return Err(engine(st.at, "negative inventory argument").into());
                    }
                    let kind = match name {
                        "inv_total" => AtomKind::InvTotal { inv, obj: key },
                        "inv_totalcat" => AtomKind::InvCat { inv, category: key },
                        _ => AtomKind::InvSlot { inv, slot: key },
                    };
                    self.observe(
                        st,
                        kind,
                        if st.epoch == 0 {
                            CondClass::Requirement
                        } else {
                            CondClass::Live
                        },
                        if name == "inv_getobj" {
                            (i32::MIN, i32::MAX)
                        } else {
                            (0, i32::MAX)
                        },
                        false,
                    )?
                };
                self.push(st, value)?;
            }
            // InvOps.ts:129-141. Mutations must not leave pre-write inventory observations usable.
            "inv_del" => {
                if !st.protected {
                    return Err(
                        unsupported(st.at, "inventory write without protected access").into(),
                    );
                }
                let args = pop_array::<3>(st)?;
                let inv = self.not_null(st, args[0])?;
                if !self.machine.program.packs.invs.contains_key(&inv) {
                    return Err(engine(st.at, "invalid inventory").into());
                }
                self.not_null(st, args[1])?;
                self.positive(st, args[2], true)?;
                self.effect(st, Effect::Inv { args })?;
                st.epoch = st.epoch.saturating_add(1);
            }
            "obj_add" => {
                let args = pop_array::<4>(st)?;
                self.validate_coord(st, args[0])?;
                self.not_null(st, args[1])?;
                self.positive(st, args[2], true)?;
                self.positive(st, args[3], true)?;
                self.effect(st, Effect::Obj { command: name })?;
            }
            "loc_add" | "loc_change" | "loc_del" | "loc_coord" | "loc_type" | "loc_angle"
            | "loc_shape" | "loc_category" | "loc_param" | "loc_name" | "lc_param"
            | "loc_findallzone" | "loc_findnext" => return self.loc(st, name, slot),
            // EnumOps.ts:7-23. An absent string enum key pushes the integer default.
            "enum" => {
                let args = pop_array::<4>(st)?;
                let input = self.required(st, args[0])?;
                let output = self.required(st, args[1])?;
                let id = self.not_null(st, args[2])?;
                let table = self
                    .machine
                    .program
                    .packs
                    .enums
                    .get(&id)
                    .ok_or_else(|| engine(st.at, "invalid enum"))?;
                if input != i32::from(table.input) || output != i32::from(table.output) {
                    return Err(engine(st.at, "enum type mismatch").into());
                }
                if let Some(key) = known(st, &self.arena, args[3]) {
                    match table.values.get(&key) {
                        Some(ConfigValue::Text(v)) => {
                            self.charge(v.len(), st.at)?;
                            self.push_text(st, Text::Lit(v.clone()))?;
                        }
                        Some(ConfigValue::Int(v)) => self.push(st, Value::Known(*v))?,
                        None => self.push(st, Value::Known(table.int_default))?,
                    }
                } else if table
                    .values
                    .values()
                    .any(|v| matches!(v, ConfigValue::Text(_)))
                {
                    return Err(unsupported(st.at, "enum has unknown output stack").into());
                } else {
                    self.push(st, Value::Opaque(st.at))?;
                }
            }
            _ => self.cosmetic(st, name)?,
        }
        Ok(Step::Next)
    }
    pub(super) fn required(&self, st: &State, v: Value) -> Result<i32, End> {
        known(st, &self.arena, v).ok_or_else(|| unsupported(st.at, "operand is not known"))
    }
    pub(super) fn not_null(&self, st: &State, v: Value) -> Result<i32, End> {
        let n = self.required(st, v)?;
        if n == -1 {
            Err(engine(st.at, "null operand"))
        } else {
            Ok(n)
        }
    }
    pub(super) fn positive(&self, st: &State, v: Value, nonzero: bool) -> Result<i32, End> {
        let n = self.required(st, v)?;
        if n < i32::from(nonzero) {
            Err(engine(st.at, "operand is below its minimum"))
        } else {
            Ok(n)
        }
    }
    pub(super) fn position(&mut self, st: &State) -> ExecResult<Value> {
        Ok(match st.player {
            PlayerPos::Tile(c) => Value::Known(c.0),
            PlayerPos::Near(site) => Value::Opaque(site),
            PlayerPos::Unknown(v) => v,
            PlayerPos::Rel { dx, dlevel, dz } => {
                let base = self.atom(
                    AtomKind::Stand,
                    0,
                    0,
                    Site::default(),
                    CondClass::Input,
                    (i32::MIN, i32::MAX),
                )?;
                if dx == 0 && dlevel == 0 && dz == 0 {
                    base
                } else {
                    let Value::Sym(of) = base else {
                        return Err(unsupported(st.at, "stand expression missing").into());
                    };
                    self.expr(Expr::CoordPlus { of, dx, dlevel, dz }, st.at)?
                }
            }
        })
    }
    fn cosmetic(&mut self, st: &mut State, name: &'static str) -> ExecResult<()> {
        let sig = self
            .machine
            .program
            .signatures
            .get(name)
            .copied()
            .ok_or_else(|| unsupported(st.at, "missing cosmetic command signature"))?;
        let expected = match name {
            "anim" | "if_setanim" | "if_setnpchead" => (2, 0),
            "facesquare" | "if_openoverlay" => (1, 0),
            "sound_synth" | "hint_coord" => (3, 0),
            "cam_lookat" | "cam_moveto" | "spotanim_map" | "p_locmerge" => (4, 0),
            "projanim_map" => (9, 0),
            "if_settext" => (1, 1),
            "cam_reset" | "hint_stop" => (0, 0),
            _ => return Err(unsupported(st.at, "unsupported cosmetic command").into()),
        };
        if (sig.int_in, sig.string_in) != expected {
            return Err(unsupported(st.at, "cosmetic signature mismatch").into());
        }
        let args = pop_args(st, sig.int_in)?;
        for _ in 0..sig.string_in {
            pop_text(st)?;
        }
        match name {
            "facesquare" | "cam_lookat" | "cam_moveto" => self.validate_coord(st, args[0])?,
            "hint_coord" => self.validate_coord(st, args[1])?,
            "spotanim_map" => {
                self.not_null(st, args[0])?;
                self.validate_coord(st, args[1])?;
            }
            "projanim_map" => {
                self.not_null(st, args[2])?;
                self.validate_coord(st, args[0])?;
                self.validate_coord(st, args[1])?;
            }
            "if_settext" | "if_setanim" => {
                self.not_null(st, args[0])?;
            }
            "if_setnpchead" => {
                self.not_null(st, args[0])?;
                self.npc_type_proof(st, args[1])?;
            }
            "p_locmerge" => {
                self.active_loc(st, 0)?;
                self.validate_coord(st, args[2])?;
                self.validate_coord(st, args[3])?;
            }
            "sound_synth" => {
                self.not_null(st, args[0])?;
            }
            "anim" | "cam_reset" | "hint_stop" | "if_openoverlay" => {}
            _ => return Err(unsupported(st.at, "unimplemented cosmetic command").into()),
        }
        if sig.int_out != 0 || sig.string_out != 0 {
            return Err(unsupported(st.at, "cosmetic signature has outputs").into());
        }
        self.effect(st, Effect::Cosmetic { command: name })
    }
    fn active_loc(&self, st: &State, slot: usize) -> Result<LocObj, End> {
        st.active_loc[slot].ok_or_else(|| engine(st.at, "no active loc"))
    }
    fn loc(&mut self, st: &mut State, name: &str, slot: usize) -> ExecResult<Step> {
        match name {
            // LocOps.ts:18-77. Every mutation uses the same path-local ordered overlay.
            "loc_add" => {
                let args = pop_array::<5>(st)?;
                let tile = Coord(self.required(st, args[0])?);
                let ty = LocTypeId(self.not_null(st, args[1])?);
                let angle = self.required(st, args[2])?;
                let shape = self.required(st, args[3])?;
                let duration = self.required(st, args[4])?;
                if !(0..4).contains(&angle) || !(0..23).contains(&shape) {
                    return Err(engine(st.at, "invalid loc shape or angle").into());
                }
                let obj = self.overlay(st, |world, w, budget| {
                    world.loc_add_bounded(
                        w,
                        tile,
                        LocInfo {
                            ty,
                            shape: shape as u8,
                            angle: angle as u8,
                        },
                        duration,
                        budget,
                    )
                })?;
                st.active_loc[slot] = Some(obj);
                self.effect(
                    st,
                    Effect::Loc {
                        kind: LocKind::Add,
                        obj,
                        ty,
                        duration,
                    },
                )?;
            }
            "loc_change" => {
                let args = pop_array::<2>(st)?;
                let ty = LocTypeId(self.not_null(st, args[0])?);
                let duration = self.required(st, args[1])?;
                let obj = self.active_loc(st, slot)?;
                self.overlay(st, |world, w, budget| {
                    world.loc_change_bounded(w, obj, ty, duration, budget)
                })?;
                self.effect(
                    st,
                    Effect::Loc {
                        kind: LocKind::Change,
                        obj,
                        ty,
                        duration,
                    },
                )?;
            }
            "loc_del" => {
                let v = pop(st)?;
                let duration = self.positive(st, v, true)?;
                let obj = self.active_loc(st, slot)?;
                let (info, _) = st
                    .world
                    .read(self.machine.world, obj)
                    .map_err(|e| world_end(st.at, e))?;
                self.overlay(st, |world, w, budget| {
                    world.loc_del_bounded(w, obj, duration, budget)
                })?;
                self.effect(
                    st,
                    Effect::Loc {
                        kind: LocKind::Del,
                        obj,
                        ty: info.ty,
                        duration,
                    },
                )?;
            }
            "loc_findallzone" => {
                let value = pop(st)?;
                let tile = Coord(self.required(st, value)?);
                self.validate_coord(st, value)?;
                let zone = ZoneKey::of(tile);
                let items = self.overlay(st, |world, w, budget| {
                    world.zone_iter_bounded(w, zone, budget)
                })?;
                st.loc_iter = Some(LocIter {
                    zone,
                    stamp: st.world.stamp(zone),
                    items,
                    next: 0,
                    age: IterAge::Fresh,
                });
            }
            // LocOps.ts:102-112 and ScriptIterators.ts:329-343. Failed iteration leaves the slot.
            "loc_findnext" => {
                let Some(iter) = &st.loc_iter else {
                    self.push(st, Value::Known(0))?;
                    return Ok(Step::Next);
                };
                if iter.stamp != st.world.stamp(iter.zone) {
                    return Err(unsupported(st.at, "loc list changed during iteration").into());
                }
                let item = iter.items.get(iter.next).copied();
                if item.is_none() {
                    self.push(st, Value::Known(0))?;
                    return Ok(Step::Next);
                }
                if iter.age == IterAge::Stale {
                    return Err(engine(st.at, "old loc iterator").into());
                }
                if iter.age == IterAge::MaybeStale {
                    let live = self.observe(
                        st,
                        AtomKind::Live("loc_iterator_suspended"),
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
                            child.forced_end = Some(engine(st.at, "old loc iterator"));
                        } else {
                            if let Some(iter) = &mut child.loc_iter {
                                iter.age = IterAge::Fresh;
                                iter.next += 1;
                            }
                            child.active_loc[slot] = item;
                            self.push(child, Value::Known(1))?;
                        }
                    }
                    return Ok(Step::Split(children));
                }
                if let Some(iter) = &mut st.loc_iter {
                    iter.next += 1;
                }
                st.active_loc[slot] = item;
                self.push(st, Value::Known(1))?;
            }
            "lc_param" => {
                let args = pop_array::<2>(st)?;
                let ty = LocTypeId(self.not_null(st, args[0])?);
                let id = self.not_null(st, args[1])?;
                self.loc_param(st, ty, id)?;
            }
            other => {
                let obj = self.active_loc(st, slot)?;
                let (info, _) = st
                    .world
                    .read(self.machine.world, obj)
                    .map_err(|e| world_end(st.at, e))?;
                if st.epoch > 0 {
                    st.world_reads_after_suspend += 1;
                }
                match other {
                    "loc_coord" => {
                        let c = st
                            .world
                            .tile(self.machine.world, obj)
                            .map_err(|e| world_end(st.at, e))?;
                        self.push(st, Value::Known(c.0))?;
                    }
                    "loc_type" => self.push(st, Value::Known(info.ty.0))?,
                    "loc_angle" => self.push(st, Value::Known(i32::from(info.angle)))?,
                    "loc_shape" => self.push(st, Value::Known(i32::from(info.shape)))?,
                    "loc_category" => {
                        let cfg = self
                            .machine
                            .program
                            .packs
                            .locs
                            .get(&info.ty)
                            .ok_or_else(|| engine(st.at, "invalid loc type"))?;
                        self.push(st, Value::Known(cfg.category.unwrap_or(-1)))?;
                    }
                    "loc_name" => self.push_text(st, Text::Unknown(st.at))?,
                    "loc_param" => {
                        let value = pop(st)?;
                        let id = self.not_null(st, value)?;
                        self.loc_param(st, info.ty, id)?;
                    }
                    _ => return Err(unsupported(st.at, "unsupported loc operation").into()),
                }
            }
        }
        Ok(Step::Next)
    }
    fn loc_param(&mut self, st: &mut State, ty: LocTypeId, id: i32) -> ExecResult<()> {
        let param = self
            .machine
            .program
            .packs
            .params
            .get(&id)
            .ok_or_else(|| engine(st.at, "invalid param"))?;
        let loc = self
            .machine
            .program
            .packs
            .locs
            .get(&ty)
            .ok_or_else(|| engine(st.at, "invalid loc type"))?;
        if param.string {
            let source = match loc.params.get(&id) {
                Some(ConfigValue::Text(s)) => s,
                _ => &param.string_default,
            };
            self.charge(source.len(), st.at)?;
            self.push_text(st, Text::Lit(source.clone()))?;
        } else {
            let value = match loc.params.get(&id) {
                Some(ConfigValue::Int(n)) => *n,
                _ => param.int_default,
            };
            self.push(st, Value::Known(value))?;
        }
        Ok(())
    }
}
