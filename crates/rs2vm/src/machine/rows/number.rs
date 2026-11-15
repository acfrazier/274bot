use super::*;
use crate::commands::{js_arith, ArithOp};

impl Exec<'_, '_> {
    pub(super) fn number(&mut self, st: &mut State, name: &str) -> ExecResult<Step> {
        match name {
            // NumberOps.ts:8-30,69-72 and ScriptState.ts:304-306: f64 then ToInt32.
            "add" | "sub" | "multiply" | "divide" | "modulo" => {
                let args = pop_array::<2>(st)?;
                let a = known(st, &self.arena, args[0]);
                let b = known(st, &self.arena, args[1]);
                let value = if let (Some(a), Some(b)) = (a, b) {
                    Value::Known(js_arith(
                        match name {
                            "add" => ArithOp::Add,
                            "sub" => ArithOp::Sub,
                            "multiply" => ArithOp::Multiply,
                            "divide" => ArithOp::Divide,
                            _ => ArithOp::Modulo,
                        },
                        a,
                        b,
                    ))
                } else {
                    let term = match (name, args[0], args[1], a, b) {
                        ("add", Value::Sym(of), _, _, Some(c)) => Some((of, c)),
                        ("add", _, Value::Sym(of), Some(c), _) => Some((of, c)),
                        ("sub", Value::Sym(of), _, _, Some(c)) => Some((of, c.wrapping_neg())),
                        _ => None,
                    };
                    if let Some((of, c)) = term {
                        if c == 0 {
                            Value::Sym(of)
                        } else {
                            self.expr(Expr::AddConst { of, c }, st.at)?
                        }
                    } else {
                        Value::Opaque(st.at)
                    }
                };
                self.push(st, value)?;
            }
            // NumberOps.ts:59-67,143-163. ECMAScript masks a single shift count to five bits.
            "testbit" | "clearbit" => {
                let args = pop_array::<2>(st)?;
                let bit = self.required(st, args[1])? as u32 & 31;
                let value = if name == "testbit" {
                    self.bits(st, args[0], bit as u8, bit as u8)?
                } else {
                    self.with_bits(st, args[0], bit as u8, bit as u8, Value::Known(0))?
                };
                self.push(st, value)?;
            }
            "getbit_range" | "clearbit_range" | "setbit_range_toint" => {
                let args = pop_args(st, if name == "setbit_range_toint" { 4 } else { 3 })?;
                let n = args.len();
                let lo = self.required(st, args[n - 2])?;
                let hi = self.required(st, args[n - 1])?;
                if !(0..32).contains(&lo) || hi < lo || hi > 31 {
                    return Err(unsupported(st.at, "unmodelled bit range").into());
                }
                let value = if name == "getbit_range" {
                    self.bits(st, args[0], lo as u8, hi as u8)?
                } else if name == "clearbit_range" {
                    self.with_bits(st, args[0], lo as u8, hi as u8, Value::Known(0))?
                } else {
                    let mask = domains::mask(lo as u8, hi as u8);
                    let to = match known(st, &self.arena, args[1]) {
                        Some(v) if v > (mask as i32) => Value::Known(mask as i32),
                        _ => args[1],
                    };
                    if let (Some(old), Some(to)) =
                        (known(st, &self.arena, args[0]), known(st, &self.arena, to))
                    {
                        Value::Known(((old as u32 & !(mask << lo)) | ((to as u32) << lo)) as i32)
                    } else {
                        self.with_bits(st, args[0], lo as u8, hi as u8, to)?
                    }
                };
                self.push(st, value)?;
            }
            // ServerOps.ts:103-108. CoordGrid.packCoord masks each component separately.
            "movecoord" => {
                let args = pop_array::<4>(st)?;
                self.validate_coord(st, args[0])?;
                let dx = self.required(st, args[1])?;
                let dlevel = self.required(st, args[2])?;
                let dz = self.required(st, args[3])?;
                let value = if let Some(c) = known(st, &self.arena, args[0]) {
                    Value::Known(Coord(c).offset(dx, dlevel, dz).0)
                } else if let Value::Sym(of) = args[0] {
                    let expr = match self.arena.expr(of) {
                        Some(Expr::CoordPlus {
                            of,
                            dx: x,
                            dlevel: y,
                            dz: z,
                        }) => Expr::CoordPlus {
                            of: *of,
                            dx: x.wrapping_add(dx),
                            dlevel: y.wrapping_add(dlevel),
                            dz: z.wrapping_add(dz),
                        },
                        _ => Expr::CoordPlus { of, dx, dlevel, dz },
                    };
                    self.expr(expr, st.at)?
                } else {
                    Value::Opaque(st.at)
                };
                self.push(st, value)?;
            }
            // ServerOps.ts:114-124. Components retain the same underlying coordinate atom.
            "coordx" | "coordz" => {
                let value = pop(st)?;
                self.validate_coord(st, value)?;
                let axis = if name == "coordx" { Axis::X } else { Axis::Z };
                let value = if let Some(c) = known(st, &self.arena, value) {
                    Value::Known(if axis == Axis::X {
                        Coord(c).x()
                    } else {
                        Coord(c).z()
                    })
                } else if let Value::Sym(of) = value {
                    self.expr(Expr::CoordPart { of, axis }, st.at)?
                } else {
                    Value::Opaque(st.at)
                };
                self.push(st, value)?;
            }
            // ServerOps.ts:94-101; CoordGrid.ts:75-79.
            "distance" => {
                let args = pop_array::<2>(st)?;
                self.validate_coord(st, args[0])?;
                self.validate_coord(st, args[1])?;
                let value = if let (Some(a), Some(b)) = (
                    known(st, &self.arena, args[0]),
                    known(st, &self.arena, args[1]),
                ) {
                    Value::Known(
                        (Coord(a).x() - Coord(b).x())
                            .abs()
                            .max((Coord(a).z() - Coord(b).z()).abs()),
                    )
                } else {
                    Value::Opaque(st.at)
                };
                self.push(st, value)?;
            }
            // ServerOps.ts:276-335: random candidates are not a proved navigation destination.
            "map_findsquare" => {
                let args = pop_array::<4>(st)?;
                self.validate_coord(st, args[0])?;
                self.positive(st, args[1], false)?;
                self.positive(st, args[2], false)?;
                let kind = self.required(st, args[3])?;
                if !(0..3).contains(&kind) {
                    return Err(engine(st.at, "invalid find-square type").into());
                }
                if !self.arena.near.contains(&st.at) {
                    if let Some((additional, bytes)) =
                        super::super::execute::growth(&self.arena.near, 1)
                    {
                        self.charge(bytes, st.at)?;
                        self.arena.near.reserve_exact(additional);
                    }
                    self.arena.near.push(st.at);
                }
                self.push(st, Value::Opaque(st.at))?;
            }
            _ => return Err(unsupported(st.at, "unsupported number row").into()),
        }
        Ok(Step::Next)
    }
}
