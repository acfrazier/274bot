use super::execute::{engine, known, pop, pop_text, unsupported, Exec, ExecResult};
use super::*;
use crate::world::{Tri, WorldError};

impl Exec<'_, '_> {
    pub(super) fn var(&mut self, st: &mut State, name: &str, operand: i32) -> ExecResult<()> {
        // CoreOps.ts:25-106,288-296. No secondary player store is modelled.
        if operand & 0x1_0000 != 0 {
            return Err(unsupported(st.at, "secondary var opcode").into());
        }
        let id = operand as u16;
        match name {
            "PUSH_VARP" => {
                let value = self.var_read(st, VarpId(id))?;
                match value {
                    Stored::Int(v) => self.push(st, v)?,
                    Stored::Str(t) => self.push_text(st, t)?,
                }
            }
            "POP_VARP" => {
                let varp = self
                    .machine
                    .program
                    .packs
                    .varps
                    .get(&VarpId(id))
                    .ok_or_else(|| engine(st.at, "invalid varp"))?;
                if varp.protect && !st.protected {
                    return Err(engine(st.at, "protected varp write without access").into());
                }
                let value = if varp.string {
                    Stored::Str(pop_text(st)?)
                } else {
                    Stored::Int(pop(st)?)
                };
                let saved = self.clone_stored(&value, st.at)?;
                self.store(st, VarpId(id), saved)?;
                self.effect(
                    st,
                    Effect::VarWrite {
                        varp: VarpId(id),
                        bits: None,
                        value,
                    },
                )?;
            }
            "PUSH_VARBIT" => {
                let bit = self
                    .machine
                    .program
                    .packs
                    .varbits
                    .get(&id)
                    .ok_or_else(|| engine(st.at, "invalid varbit"))?;
                let (base, lo, hi) = (bit.base, bit.lo, bit.hi);
                let Stored::Int(value) = self.var_read(st, base)? else {
                    return Err(engine(st.at, "varbit has string backing varp").into());
                };
                let value = self.bits(st, value, lo, hi)?;
                self.push(st, value)?;
            }
            "POP_VARBIT" => {
                let bit = self
                    .machine
                    .program
                    .packs
                    .varbits
                    .get(&id)
                    .ok_or_else(|| engine(st.at, "invalid varbit"))?;
                let (base, lo, hi) = (bit.base, bit.lo, bit.hi);
                let varp = self
                    .machine
                    .program
                    .packs
                    .varps
                    .get(&base)
                    .ok_or_else(|| engine(st.at, "varbit backing varp missing"))?;
                if varp.protect && !st.protected {
                    return Err(engine(st.at, "protected varbit write without access").into());
                }
                let mut value = pop(st)?;
                let mask = domains::mask(lo, hi);
                if let Some(n) = known(st, &self.arena, value) {
                    value = Value::Known(if n < 0 || n as u32 > mask { 0 } else { n });
                }
                let Stored::Int(old) = self.var_read(st, base)? else {
                    return Err(engine(st.at, "varbit has string backing varp").into());
                };
                let next = self.with_bits(st, old, lo, hi, value)?;
                let stored = Stored::Int(next);
                self.store(st, base, stored.clone())?;
                self.effect(
                    st,
                    Effect::VarWrite {
                        varp: base,
                        bits: Some((lo, hi)),
                        value: stored,
                    },
                )?;
            }
            "PUSH_VARN" | "PUSH_VARS" => {
                if name == "PUSH_VARN" && st.active_npc[0].is_none() {
                    return Err(engine(st.at, "no active NPC for varn").into());
                }
                let table = if name == "PUSH_VARN" {
                    &self.machine.program.packs.varns
                } else {
                    &self.machine.program.packs.varss
                };
                let string = table
                    .get(&id)
                    .copied()
                    .ok_or_else(|| engine(st.at, "invalid live variable"))?;
                if string {
                    self.push_text(st, Text::Unknown(st.at))?;
                } else {
                    let v = self.observe(
                        st,
                        AtomKind::Live(if name == "PUSH_VARN" { "varn" } else { "vars" }),
                        CondClass::Live,
                        (i32::MIN, i32::MAX),
                        true,
                    )?;
                    self.push(st, v)?;
                }
            }
            _ => return Err(unsupported(st.at, "unsupported variable opcode").into()),
        }
        Ok(())
    }
    fn store(&mut self, st: &mut State, id: VarpId, value: Stored) -> ExecResult<()> {
        if let Some((_, v)) = st.vars.iter_mut().find(|(key, _)| *key == id) {
            *v = value;
        } else {
            self.reserve(&mut st.vars, 1, st.at)?;
            st.vars.push((id, value));
        }
        Ok(())
    }
    fn clone_stored(&mut self, value: &Stored, at: Site) -> Result<Stored, EntryFailure> {
        Ok(match value {
            Stored::Int(v) => Stored::Int(*v),
            Stored::Str(t) => Stored::Str(self.clone_text(t, at)?),
        })
    }
    pub(super) fn var_read(&mut self, st: &mut State, id: VarpId) -> ExecResult<Stored> {
        if let Some((_, value)) = st.vars.iter().find(|(key, _)| *key == id) {
            return Ok(self.clone_stored(value, st.at)?);
        }
        let config = self
            .machine
            .program
            .packs
            .varps
            .get(&id)
            .ok_or_else(|| engine(st.at, "invalid varp"))?;
        let value = if config.string {
            Stored::Str(Text::Unknown(st.at))
        } else {
            let class = if st.epoch > 0 && !config.protect {
                CondClass::Live
            } else if config.transmit {
                CondClass::Requirement
            } else {
                CondClass::Hidden
            };
            Stored::Int(self.observe(st, AtomKind::Varp(id), class, (i32::MIN, i32::MAX), false)?)
        };
        let saved = self.clone_stored(&value, st.at)?;
        self.store(st, id, saved)?;
        Ok(value)
    }
    pub(super) fn bits(&mut self, st: &State, value: Value, lo: u8, hi: u8) -> ExecResult<Value> {
        if lo > hi || hi > 31 {
            return Err(engine(st.at, "invalid bit range").into());
        }
        if let Some(v) = known(st, &self.arena, value) {
            return Ok(Value::Known(
                ((v >> lo) as u32 & domains::mask(lo, hi)) as i32,
            ));
        }
        let Value::Sym(of) = value else {
            return Ok(Value::Opaque(st.at));
        };
        if let Some(Expr::WithBits {
            of: old,
            lo: l,
            hi: h,
            to,
        }) = self.arena.expr(of)
        {
            if lo == *l && hi == *h {
                return Ok(*to);
            }
            if hi < *l || lo > *h {
                let old = *old;
                return self.bits(st, Value::Sym(old), lo, hi);
            }
            return Ok(Value::Opaque(st.at));
        }
        Ok(self.expr(Expr::Bits { of, lo, hi }, st.at)?)
    }
    pub(super) fn with_bits(
        &mut self,
        st: &State,
        old: Value,
        lo: u8,
        hi: u8,
        to: Value,
    ) -> ExecResult<Value> {
        if lo > hi || hi > 31 {
            return Err(engine(st.at, "invalid bit range").into());
        }
        let mask = domains::mask(lo, hi);
        let (min, max) = domains::bounds(&self.arena, &st.domains, to);
        if min < 0 || max as u32 > mask {
            return Ok(Value::Opaque(st.at));
        }
        if let (Some(old), Some(to)) = (known(st, &self.arena, old), known(st, &self.arena, to)) {
            return Ok(Value::Known(
                ((old as u32 & !(mask << lo)) | ((to as u32 & mask) << lo)) as i32,
            ));
        }
        let Value::Sym(of) = old else {
            return Ok(Value::Opaque(st.at));
        };
        Ok(self.expr(Expr::WithBits { of, lo, hi, to }, st.at)?)
    }
    pub(super) fn suspend(&mut self, st: &mut State, ticks: u32, possible: bool) -> ExecResult<()> {
        // PlayerOps.ts:359-380; Player.ts:2173-2182,2212-2221;
        // ScriptIterators.ts:331-343,377-382. Both delay rows use this transfer.
        if !st.protected {
            return Err(unsupported(st.at, "suspension without protected player access").into());
        }
        st.ticks.min = st
            .ticks
            .min
            .saturating_add(if possible { 0 } else { ticks });
        st.ticks.max = st.ticks.max.saturating_add(ticks);
        st.epoch = st.epoch.saturating_add(1);
        st.suspended = if possible && st.suspended != Suspension::Yes {
            Suspension::Maybe
        } else {
            Suspension::Yes
        };
        st.vars.retain(|(id, _)| {
            self.machine
                .program
                .packs
                .varps
                .get(id)
                .is_some_and(|v| v.protect)
        });
        if let Some(iter) = &mut st.loc_iter {
            iter.age = age(iter.age, possible);
        }
        if let Some(iter) = &mut st.npc_iter {
            iter.age = age(iter.age, possible);
        }
        self.overlay(st, |world, w, budget| {
            world.age_bounded(
                w,
                Ticks {
                    min: if possible { 0 } else { ticks },
                    max: ticks,
                },
                budget,
            )
        })?;
        self.audit.suspensions += 1;
        st.transfers.suspensions += 1;
        self.effect(
            st,
            Effect::Delay {
                min: if possible { 0 } else { ticks },
                max: ticks,
            },
        )?;
        Ok(())
    }
    pub(super) fn close(&mut self, st: &mut State, clear_weak: bool) -> ExecResult<()> {
        // Player.ts:760-813. Never execute cancelled script tails or ignore a callback.
        if clear_weak {
            self.effect(st, Effect::WeakQueueCleared)?;
        }
        for interface in [st.modal.main, st.modal.chat, st.modal.side]
            .into_iter()
            .flatten()
        {
            let trigger = self
                .machine
                .program
                .trigger("IF_CLOSE")
                .ok_or_else(|| unsupported(st.at, "IF_CLOSE trigger number is unavailable"))?;
            if self
                .machine
                .program
                .lookup(trigger, Some(interface), None)
                .is_some()
            {
                return Err(
                    unsupported(st.at, "close callback is outside the command inventory").into(),
                );
            }
        }
        if st.modal != Modal::default() {
            st.modal = Modal::default();
            self.effect(
                st,
                Effect::Modal {
                    state: st.modal.clone(),
                },
            )?;
        }
        Ok(())
    }
    pub(super) fn finish(&mut self, st: &mut State) -> ExecResult<()> {
        // Player.ts:2222-2229. An end close never clears the weak queue.
        if st.modal.main.is_none() {
            match st.suspended {
                Suspension::Yes => {
                    self.close(st, false)?;
                    self.audit.automatic_closes += 1;
                    st.transfers.automatic_closes += 1;
                }
                Suspension::Maybe if st.modal.chat.is_some() || st.modal.side.is_some() => {
                    return Err(
                        unsupported(st.at, "possible suspension ended with a modal open").into(),
                    )
                }
                _ => {}
            }
        }
        Ok(())
    }
    pub(super) fn validate_coord(&self, st: &State, value: Value) -> Result<(), End> {
        if let Some(n) = known(st, &self.arena, value) {
            return if n >= 0 {
                Ok(())
            } else {
                Err(engine(st.at, "invalid coordinate"))
            };
        }
        let valid = match value {
            Value::Sym(id) => match self.arena.expr(id) {
                Some(Expr::CoordPlus { of, .. }) => {
                    self.validate_coord(st, Value::Sym(*of)).is_ok()
                }
                Some(Expr::Atom(atom)) => self.arena.atom(*atom).is_some_and(|a| match &a.kind {
                    AtomKind::Stand => true,
                    AtomKind::Live(n) => *n == "npc_coord",
                    // ScriptVarType.ts:4,12: string parameters have no integer local slot.
                    AtomKind::Arg(n) => self
                        .machine
                        .program
                        .script(a.site.script)
                        .and_then(|s| s.params.iter().filter(|ty| **ty != b's').nth(*n as usize))
                        .is_some_and(|ty| *ty == b'c'),
                    _ => false,
                }),
                _ => false,
            },
            Value::Opaque(site) => self.arena.near.contains(&site),
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err(unsupported(st.at, "coordinate validation needs a proof"))
        }
    }
    pub(super) fn move_to(
        &mut self,
        st: &mut State,
        value: Value,
        kind: MoveKind,
    ) -> ExecResult<super::execute::Step> {
        self.validate_coord(st, value)?;
        let pos = if let Some(n) = known(st, &self.arena, value) {
            PlayerPos::Tile(Coord(n))
        } else {
            match value {
                Value::Sym(id) => match self.arena.expr(id) {
                    Some(Expr::Atom(atom))
                        if self
                            .arena
                            .atom(*atom)
                            .is_some_and(|a| a.kind == AtomKind::Stand) =>
                    {
                        PlayerPos::Rel {
                            dx: 0,
                            dlevel: 0,
                            dz: 0,
                        }
                    }
                    Some(Expr::CoordPlus { of, dx, dlevel, dz })
                        if self
                            .arena
                            .atom_of(Value::Sym(*of))
                            .is_some_and(|a| a.kind == AtomKind::Stand) =>
                    {
                        PlayerPos::Rel {
                            dx: *dx,
                            dlevel: *dlevel,
                            dz: *dz,
                        }
                    }
                    _ => PlayerPos::Unknown(value),
                },
                Value::Opaque(site) => PlayerPos::Near(site),
                _ => return Err(unsupported(st.at, "unknown move destination").into()),
            }
        };
        if let PlayerPos::Tile(to) = pos {
            let allocation = self.machine.world.zone_alloc(to);
            if allocation.on_members == Tri::Unknown {
                return Err(unsupported(st.at, "destination zone allocation is unknown").into());
            }
            if allocation.on_members == Tri::No {
                self.no_move(st, to)?;
                return Ok(super::execute::Step::Next);
            }
            let stand_members = if let PlayerPos::Tile(stand) = st.player {
                self.machine.world.zone_alloc(stand).on_free == Tri::No
            } else {
                false
            };
            if allocation.on_free != Tri::Yes && !stand_members {
                let members = self.atom(
                    AtomKind::Members,
                    0,
                    0,
                    Site::default(),
                    CondClass::Requirement,
                    (0, 1),
                )?;
                let mut children = self.split(
                    st,
                    Pred::Cmp {
                        lhs: members,
                        op: CmpOp::Eq,
                        rhs: Value::Known(1),
                    },
                )?;
                for child in &mut children {
                    if pop(child)? == Value::Known(1) {
                        self.record_move(child, pos, kind)?;
                    } else if allocation.on_free == Tri::No {
                        self.no_move(child, to)?;
                    } else {
                        child.forced_end =
                            Some(unsupported(st.at, "free-world zone allocation is unknown"));
                    }
                }
                return Ok(super::execute::Step::Split(children));
            }
        }
        self.record_move(st, pos, kind)?;
        Ok(super::execute::Step::Next)
    }
    fn no_move(&mut self, st: &mut State, to: Coord) -> ExecResult<()> {
        self.effect(st, Effect::NoMove { to })?;
        self.charge("Invalid teleport!".len(), st.at)?;
        self.effect(
            st,
            Effect::Message {
                text: Text::Lit("Invalid teleport!".into()),
            },
        )
    }
    fn record_move(&mut self, st: &mut State, to: PlayerPos, kind: MoveKind) -> ExecResult<()> {
        st.player = to;
        self.effect(st, Effect::Move { kind, to })
    }
}
fn age(old: IterAge, possible: bool) -> IterAge {
    if !possible || old == IterAge::Stale {
        IterAge::Stale
    } else {
        IterAge::MaybeStale
    }
}
pub(super) fn world_end(site: Site, error: WorldError) -> End {
    let reason = match error {
        WorldError::InvalidCoordinate { .. } => "invalid location coordinate",
        WorldError::InvalidLocType { .. } => "invalid location type",
        WorldError::InvalidLocShape { .. } => "invalid location shape",
        WorldError::InvalidLocAngle { .. } => "invalid location angle",
        WorldError::InvalidDuration { .. } => "invalid location duration",
        WorldError::InvalidTickRange { .. } => "invalid elapsed tick range",
        WorldError::InvalidZoneKey { .. } => "invalid world zone key",
        WorldError::InvalidWorldPlacement { .. } => "invalid world placement",
        WorldError::DuplicatePlacedId { .. } => "duplicate world placement",
        WorldError::InvalidWorldZoneList { .. } => "invalid world zone list",
        WorldError::UnknownLifeState { .. } => "unknown location life state",
        WorldError::UnknownZoneState { .. } => "unknown location zone state",
        WorldError::MissingPlacedObject { .. } => "missing placed location",
        WorldError::MissingCreatedObject { .. } => "missing created location",
        WorldError::ObjectNotInZone { .. } => "location is not in its zone",
        WorldError::CreatedIdExhausted => "created location ids exhausted",
        WorldError::ListStampExhausted { .. } => "location list stamp exhausted",
        WorldError::ByteLimit { .. } => "entry byte limit",
    };
    if error.is_engine_fault() {
        engine(site, reason)
    } else {
        unsupported(site, reason)
    }
}
