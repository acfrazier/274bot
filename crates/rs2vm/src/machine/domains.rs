use super::{Arena, AtomId, Axis, CmpOp, Cond, CondClass, Expr, ExprId, Pred, Value};
use crate::world::{ByteBudget, WorldError};
use std::borrow::Cow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Projection {
    Whole,
    Bits(u8, u8),
    Axis(Axis),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Key(AtomId, Projection);
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Domain {
    pub lo: i32,
    pub hi: i32,
    not: Vec<i32>,
}
impl Domain {
    fn new(lo: i32, hi: i32) -> Self {
        Self {
            lo,
            hi,
            not: Vec::new(),
        }
    }
    fn contains(&self, n: i32) -> bool {
        n >= self.lo && n <= self.hi && !self.not.contains(&n)
    }
    fn normalize(&mut self) {
        while self.lo <= self.hi && self.not.contains(&self.lo) {
            if self.lo == i32::MAX {
                self.lo = 1;
                self.hi = 0;
                break;
            }
            self.lo += 1;
        }
        while self.lo <= self.hi && self.not.contains(&self.hi) {
            if self.hi == i32::MIN {
                self.lo = 1;
                self.hi = 0;
                break;
            }
            self.hi -= 1;
        }
        self.not.retain(|n| *n >= self.lo && *n <= self.hi);
        self.not.sort_unstable();
        self.not.dedup();
    }
    fn compare(&mut self, op: CmpOp, n: i32, budget: &mut ByteBudget) -> Result<bool, WorldError> {
        match op {
            CmpOp::Eq => {
                if !self.contains(n) {
                    return Ok(false);
                }
                self.lo = n;
                self.hi = n;
            }
            CmpOp::Ne => {
                if self.contains(n) {
                    budget.reserve(&mut self.not, 1)?;
                    self.not.push(n);
                }
            }
            CmpOp::Lt => {
                let Some(hi) = n.checked_sub(1) else {
                    return Ok(false);
                };
                self.hi = self.hi.min(hi);
            }
            CmpOp::Le => self.hi = self.hi.min(n),
            CmpOp::Gt => {
                let Some(lo) = n.checked_add(1) else {
                    return Ok(false);
                };
                self.lo = self.lo.max(lo);
            }
            CmpOp::Ge => self.lo = self.lo.max(n),
        }
        self.normalize();
        Ok(self.lo <= self.hi)
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Domains(Vec<(Key, Domain)>);
impl Domains {
    pub fn bytes(&self) -> usize {
        self.0.capacity() * std::mem::size_of::<(Key, Domain)>()
            + self
                .0
                .iter()
                .map(|(_, d)| d.not.capacity() * 4)
                .sum::<usize>()
    }
    pub fn clone_bytes(&self) -> usize {
        self.0.len() * std::mem::size_of::<(Key, Domain)>()
            + self.0.iter().map(|(_, d)| d.not.len() * 4).sum::<usize>()
    }
    fn initial(arena: &Arena, key: Key) -> Domain {
        match key.1 {
            Projection::Whole => arena
                .atom(key.0)
                .map_or(Domain::new(i32::MIN, i32::MAX), |a| {
                    Domain::new(a.bounds.0, a.bounds.1)
                }),
            Projection::Bits(lo, hi) => {
                if hi - lo == 31 {
                    Domain::new(i32::MIN, i32::MAX)
                } else {
                    Domain::new(0, mask(lo, hi) as i32)
                }
            }
            Projection::Axis(Axis::Level) => Domain::new(0, 3),
            Projection::Axis(_) => Domain::new(0, 16383),
        }
    }
    fn get(&self, arena: &Arena, key: Key) -> Cow<'_, Domain> {
        self.0.iter().find(|(k, _)| *k == key).map_or_else(
            || Cow::Owned(Self::initial(arena, key)),
            |(_, d)| Cow::Borrowed(d),
        )
    }
    fn owned(
        &self,
        arena: &Arena,
        key: Key,
        budget: &mut ByteBudget,
    ) -> Result<Domain, WorldError> {
        match self.get(arena, key) {
            Cow::Owned(domain) => Ok(domain),
            Cow::Borrowed(domain) => {
                budget.charge(domain.not.len() * 4)?;
                Ok(domain.clone())
            }
        }
    }
    fn set(&mut self, key: Key, domain: Domain, budget: &mut ByteBudget) -> Result<(), WorldError> {
        if let Some((_, old)) = self.0.iter_mut().find(|(k, _)| *k == key) {
            *old = domain;
        } else {
            budget.reserve(&mut self.0, 1)?;
            self.0.push((key, domain));
        }
        Ok(())
    }
}
pub(super) fn mask(lo: u8, hi: u8) -> u32 {
    if hi < lo {
        0
    } else if hi - lo == 31 {
        u32::MAX
    } else {
        (1u32 << (hi - lo + 1)) - 1
    }
}
fn atom_expr(arena: &Arena, id: ExprId) -> Option<AtomId> {
    match arena.expr(id)? {
        Expr::Atom(a) => Some(*a),
        _ => None,
    }
}
fn key(arena: &Arena, domains: &Domains, value: Value) -> Option<(Key, i32)> {
    let Value::Sym(id) = value else {
        return None;
    };
    match arena.expr(id)? {
        Expr::Atom(a) => Some((Key(*a, Projection::Whole), 0)),
        Expr::Bits { of, lo, hi } => {
            Some((Key(atom_expr(arena, *of)?, Projection::Bits(*lo, *hi)), 0))
        }
        Expr::CoordPart { of, axis } => {
            Some((Key(atom_expr(arena, *of)?, Projection::Axis(*axis)), 0))
        }
        Expr::AddConst { of, c } => {
            let (k, offset) = key(arena, domains, Value::Sym(*of))?;
            let d = domains.get(arena, k);
            let min = d.lo.checked_add(offset)?;
            let max = d.hi.checked_add(offset)?;
            min.checked_add(*c)?;
            max.checked_add(*c)?;
            Some((k, offset.checked_add(*c)?))
        }
        _ => None,
    }
}
/// Apply one branch without weakening its recorded predicate. Unsupported shapes narrow nothing.
pub(super) fn assume(arena: &Arena, domains: &mut Domains, pred: &Pred, truth: bool) -> bool {
    // Coverage queries own no entry state. Their temporary domains are bounded by the finite
    // already-recorded predicates; execution uses assume_bounded instead.
    assume_bounded(arena, domains, pred, truth, &mut ByteBudget::unlimited()).unwrap_or(true)
}
pub(super) fn assume_bounded(
    arena: &Arena,
    domains: &mut Domains,
    pred: &Pred,
    truth: bool,
    budget: &mut ByteBudget,
) -> Result<bool, WorldError> {
    match pred {
        Pred::Cmp { lhs, op, rhs } => {
            let op = if truth { *op } else { op.negate() };
            if let (Value::Known(a), Value::Known(b)) = (lhs, rhs) {
                return Ok(op.test(*a, *b));
            }
            if lhs == rhs && !matches!(lhs, Value::Opaque(_)) {
                return Ok(op.test(0, 0));
            }
            let (value, op, c) = match (lhs, rhs) {
                (_, Value::Known(c)) => (*lhs, op, *c),
                (Value::Known(c), _) => (*rhs, op.reverse(), *c),
                _ => return Ok(true),
            };
            let Some((k, offset)) = key(arena, domains, value) else {
                return Ok(true);
            };
            let Some(c) = c.checked_sub(offset) else {
                return Ok(true);
            };
            let mut d = domains.owned(arena, k, budget)?;
            if !d.compare(op, c, budget)? {
                return Ok(false);
            }
            domains.set(k, d, budget)?;
            Ok(true)
        }
        Pred::In { of, set } => {
            if let Value::Known(n) = of {
                return Ok(set.contains(n) == truth);
            }
            let Some((k, offset)) = key(arena, domains, *of) else {
                return Ok(true);
            };
            if offset != 0 {
                return Ok(true);
            }
            let mut d = domains.owned(arena, k, budget)?;
            if truth {
                let values = set.iter().copied().filter(|n| d.contains(*n));
                let (Some(lo), Some(hi)) = (values.clone().min(), values.max()) else {
                    return Ok(false);
                };
                // Keep the complete predicate, but do not expand a sparse range into a bit set.
                if i64::from(hi) - i64::from(lo) > 4096 {
                    return Ok(true);
                }
                d.lo = lo;
                d.hi = hi;
                d.not.retain(|n| *n >= lo && *n <= hi);
                let extra = (lo..=hi)
                    .filter(|n| !set.contains(n) && !d.not.contains(n))
                    .count();
                budget.reserve(&mut d.not, extra)?;
                for n in lo..=hi {
                    if !set.contains(&n) && !d.not.contains(&n) {
                        d.not.push(n);
                    }
                }
            } else {
                budget.reserve(&mut d.not, set.len())?;
                for n in set.iter().copied() {
                    if d.contains(n) {
                        d.not.push(n);
                    }
                }
            }
            d.normalize();
            if d.lo > d.hi {
                return Ok(false);
            }
            domains.set(k, d, budget)?;
            Ok(true)
        }
    }
}
pub(super) fn known(arena: &Arena, domains: &Domains, value: Value) -> Option<i32> {
    if let Value::Known(v) = value {
        return Some(v);
    }
    let (k, offset) = key(arena, domains, value)?;
    let d = domains.get(arena, k);
    (d.lo == d.hi).then(|| d.lo.wrapping_add(offset))
}
pub(super) fn bounds(arena: &Arena, domains: &Domains, value: Value) -> (i32, i32) {
    if let Some(n) = known(arena, domains, value) {
        return (n, n);
    }
    key(arena, domains, value).map_or((i32::MIN, i32::MAX), |(k, c)| {
        let d = domains.get(arena, k);
        (d.lo.wrapping_add(c), d.hi.wrapping_add(c))
    })
}
pub(super) fn class(arena: &Arena, value: Value) -> CondClass {
    match value {
        Value::Known(_) => CondClass::Requirement,
        Value::Opaque(_) => CondClass::Opaque,
        Value::Sym(id) => match arena.expr(id) {
            Some(Expr::Atom(a)) => arena.atom(*a).map_or(CondClass::Opaque, |a| a.class),
            Some(Expr::WithBits { of, to, .. }) => {
                class(arena, Value::Sym(*of)).max(class(arena, *to))
            }
            Some(
                Expr::AddConst { of, .. }
                | Expr::Bits { of, .. }
                | Expr::CoordPart { of, .. }
                | Expr::CoordPlus { of, .. },
            ) => class(arena, Value::Sym(*of)),
            None => CondClass::Opaque,
        },
    }
}
pub(super) fn pred_class(arena: &Arena, pred: &Pred) -> CondClass {
    match pred {
        Pred::Cmp { lhs, rhs, .. } => class(arena, *lhs).max(class(arena, *rhs)),
        Pred::In { of, .. } => class(arena, *of),
    }
}
fn same_relation(a: &Pred, at: bool, b: &Pred, bt: bool) -> bool {
    match (a, b) {
        (
            Pred::Cmp {
                lhs: a,
                op: ao,
                rhs: b,
            },
            Pred::Cmp {
                lhs: c,
                op: bo,
                rhs: d,
            },
        ) => {
            let ao = if at { *ao } else { ao.negate() };
            let bo = if bt { *bo } else { bo.negate() };
            (a == c && b == d && ao == bo) || (a == d && b == c && ao == bo.reverse())
        }
        _ => a == b && at == bt,
    }
}
pub(super) fn recorded(conds: &[Cond], pred: &Pred) -> Option<bool> {
    if matches!(
        pred,
        Pred::Cmp {
            lhs: Value::Opaque(_),
            ..
        } | Pred::Cmp {
            rhs: Value::Opaque(_),
            ..
        } | Pred::In {
            of: Value::Opaque(_),
            ..
        }
    ) {
        return None;
    }
    for c in conds {
        if same_relation(&c.pred, c.truth, pred, true) {
            return Some(true);
        }
        if same_relation(&c.pred, c.truth, pred, false) {
            return Some(false);
        }
    }
    None
}
pub(super) fn disjoint(arena: &Arena, a: &[Cond], b: &[Cond]) -> bool {
    let mut d = Domains::default();
    for c in a.iter().chain(b).filter(|c| c.class <= CondClass::Input) {
        if a.iter()
            .chain(b)
            .filter(|q| q.class <= CondClass::Input)
            .any(|q| same_relation(&c.pred, c.truth, &q.pred, !q.truth))
        {
            return true;
        }
        if !assume(arena, &mut d, &c.pred, c.truth) {
            return true;
        }
    }
    false
}
