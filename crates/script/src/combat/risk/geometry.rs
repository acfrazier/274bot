use super::{CrossingGeom, RiskInput, RiskTables, RoutePlan, UnknownWhy, ZoneInterval};
use api::WorldTile;
use nav::router::{Leg, Route};
use nav::transport::{TransportKind, WildernessRules};
use nav::zones::{ZoneClass, ZoneTable, NO_SHAPE};

#[derive(Debug, Clone, Copy)]
pub struct RoutePoint {
    pub tile: WorldTile,
    pub leg: u8,
    pub tick: i32,
    pub duration: i32,
    /// Server input delay at the end of this point's duration.
    pub hold_ticks: i32,
}
impl RoutePoint {
    pub fn input_held(self, tick: i32) -> bool {
        let end = self.tick + self.duration;
        self.hold_ticks > 0 && (end - self.hold_ticks..end).contains(&tick)
    }
}
/// Borrowed indexing over the found route: no copied tile sequence or timeline.
#[derive(Clone, Copy)]
pub struct RoutePath<'a> {
    pub route: &'a Route,
    len: usize,
    ticks: i32,
}
impl<'a> RoutePath<'a> {
    pub fn new(route: &'a Route) -> Result<Self, UnknownWhy> {
        if route.legs.len() > 256 {
            return Err(UnknownWhy::Overflow);
        }
        let mut len = 0usize;
        let mut ticks = 0i32;
        for (leg, row) in route.legs.iter().enumerate() {
            let (n, t) = match row {
                Leg::Walk { tiles } => (tiles.len(), walk_ticks(tiles.len())?),
                Leg::Transport { edge } => {
                    if edge.ticks < 0 {
                        return Err(UnknownWhy::Overflow);
                    }
                    (usize::from(!attached(route, leg)), edge.ticks)
                }
            };
            len = len.checked_add(n).ok_or(UnknownWhy::Overflow)?;
            ticks = ticks.checked_add(t).ok_or(UnknownWhy::Overflow)?;
        }
        if len > usize::from(u16::MAX) + 1 {
            return Err(UnknownWhy::Overflow);
        }
        Ok(Self { route, len, ticks })
    }
    pub fn len(self) -> usize {
        self.len
    }
    pub fn is_empty(self) -> bool {
        self.len == 0
    }
    pub fn ticks(self) -> i32 {
        self.ticks
    }
    pub fn point(self, index: u16) -> Option<RoutePoint> {
        let mut offset = 0usize;
        let mut tick = 0i32;
        for (leg, row) in self.route.legs.iter().enumerate() {
            match row {
                Leg::Walk { tiles } => {
                    let local = usize::from(index).checked_sub(offset)?;
                    if local < tiles.len() {
                        let start = walk_ticks(local).ok()?;
                        let walk_duration = walk_ticks(local + 1).ok()? - start;
                        let mut hold_ticks = 0;
                        if local + 1 == tiles.len() {
                            if let Some(Leg::Transport { edge }) = self.route.legs.get(leg + 1) {
                                if edge.takeoff.unwrap_or(edge.at) == tiles[local] {
                                    hold_ticks = edge.ticks;
                                }
                            }
                        }
                        return Some(RoutePoint {
                            tile: tiles[local],
                            leg: leg as u8,
                            tick: tick + start,
                            duration: walk_duration + hold_ticks,
                            hold_ticks,
                        });
                    }
                    offset += tiles.len();
                    tick += walk_ticks(tiles.len()).ok()?;
                }
                Leg::Transport { edge } => {
                    if !attached(self.route, leg) {
                        if offset == usize::from(index) {
                            return Some(RoutePoint {
                                tile: edge.takeoff.unwrap_or(edge.at),
                                leg: leg as u8,
                                tick,
                                duration: edge.ticks,
                                hold_ticks: edge.ticks,
                            });
                        }
                        offset += 1;
                    }
                    tick += edge.ticks;
                }
            }
        }
        None
    }
    /// On-foot route distance plus transport time (not STOP_FACTOR).
    pub fn distance(self, from: u16, to: u16) -> Result<i32, UnknownWhy> {
        let (lo, hi) = (from.min(to), from.max(to));
        let mut cost = i32::from(hi - lo);
        let mut offset = 0usize;
        for (leg, row) in self.route.legs.iter().enumerate() {
            match row {
                Leg::Walk { tiles } => offset += tiles.len(),
                Leg::Transport { edge } => {
                    let takeoff = if attached(self.route, leg) {
                        offset - 1
                    } else {
                        offset
                    };
                    if takeoff >= usize::from(lo) && takeoff < usize::from(hi) {
                        cost = cost.checked_add(edge.ticks).ok_or(UnknownWhy::Overflow)?;
                    }
                    if !attached(self.route, leg) {
                        offset += 1;
                    }
                }
            }
        }
        Ok(cost)
    }
    fn forward_on_foot(self, from: u16, to: u16) -> bool {
        let mut offset = 0usize;
        for (leg, row) in self.route.legs.iter().enumerate() {
            match row {
                Leg::Walk { tiles } => offset += tiles.len(),
                Leg::Transport { edge } => {
                    let takeoff = if attached(self.route, leg) {
                        offset - 1
                    } else {
                        offset
                    };
                    if edge.kind == TransportKind::Teleport
                        && (usize::from(from)..usize::from(to)).contains(&takeoff)
                    {
                        return false;
                    }
                    if !attached(self.route, leg) {
                        offset += 1;
                    }
                }
            }
        }
        true
    }
    /// Ceil STOP_FACTOR locally over the exposed walk tiles, with every
    /// transport hold charged on its inclusive takeoff index.
    pub fn exposure_ticks(self, from: u16, to: u16) -> Result<i32, UnknownWhy> {
        if from > to || usize::from(to) >= self.len {
            return Err(UnknownWhy::Overflow);
        }
        let mut offset = 0usize;
        let mut walks = 0usize;
        let mut holds = 0i32;
        for (leg, row) in self.route.legs.iter().enumerate() {
            match row {
                Leg::Walk { tiles } => {
                    let end = offset + tiles.len();
                    let lo = offset.max(usize::from(from));
                    let hi = end.min(usize::from(to) + 1);
                    walks += hi.saturating_sub(lo);
                    offset = end;
                }
                Leg::Transport { edge } => {
                    let takeoff = if attached(self.route, leg) {
                        offset - 1
                    } else {
                        offset
                    };
                    if (usize::from(from)..=usize::from(to)).contains(&takeoff) {
                        holds = holds.checked_add(edge.ticks).ok_or(UnknownWhy::Overflow)?;
                    }
                    if !attached(self.route, leg) {
                        offset += 1;
                    }
                }
            }
        }
        walk_ticks(walks)?
            .checked_add(holds)
            .ok_or(UnknownWhy::Overflow)
    }
    /// Geometry indices remain physical route indices. Projectile flight is
    /// an explicit temporal tail, never a rounded or invented route tile.
    pub fn interval_end_tick(self, row: &ZoneInterval) -> Result<i32, UnknownWhy> {
        let start = self.point(row.a).ok_or(UnknownWhy::Overflow)?.tick;
        let ticks = self.exposure_ticks(row.a, row.b)?;
        start
            .checked_add(ticks)
            .and_then(|end| end.checked_add(if row.ap() { super::consts::PROJ_LAG } else { 0 }))
            .ok_or(UnknownWhy::Overflow)
    }
    pub fn end_tick(self, index: u16) -> Result<i32, UnknownWhy> {
        let p = self.point(index).ok_or(UnknownWhy::Overflow)?;
        p.tick.checked_add(p.duration).ok_or(UnknownWhy::Overflow)
    }
}
fn attached(route: &Route, leg: usize) -> bool {
    let Some(Leg::Transport { edge }) = route.legs.get(leg) else {
        return false;
    };
    leg.checked_sub(1).and_then(|previous| route.legs.get(previous)).is_some_and(|row| {
        matches!(row, Leg::Walk { tiles } if tiles.last() == Some(&edge.takeoff.unwrap_or(edge.at)))
    })
}
fn walk_ticks(tiles: usize) -> Result<i32, UnknownWhy> {
    let tiles = i32::try_from(tiles).map_err(|_| UnknownWhy::Overflow)?;
    tiles
        .checked_mul(3)
        .and_then(|n| n.checked_add(1))
        .map(|n| n / 2)
        .ok_or(UnknownWhy::Overflow)
}

fn in_envelope(
    tile: WorldTile,
    index: u16,
    zones: &ZoneTable,
    risks: &RiskTables,
    input: &RiskInput,
) -> Result<bool, UnknownWhy> {
    let zone = &zones.zones()[usize::from(index)];
    let kind = &zones.kinds()[usize::from(zone.kind)];
    let risk = risks.kind(zone.kind).ok_or(UnknownWhy::MissingFacts)?;
    Ok(
        if zone.shape != NO_SHAPE
            || kind.npc_id < 0
            || risk.unknown_for(input.map_members).is_some()
        {
            zones.at(tile).any(|candidate| candidate == index)
        } else {
            tile.level == i32::from(zone.level)
                && (i64::from(tile.x) - i64::from(zone.spawn_x)).abs() <= i64::from(risk.r)
                && (i64::from(tile.z) - i64::from(zone.spawn_z)).abs() <= i64::from(risk.r)
        },
    )
}

fn interval(
    path: RoutePath<'_>,
    zones: &ZoneTable,
    risks: &RiskTables,
    input: &RiskInput,
    index: u16,
    a: u16,
    start: u16,
) -> Result<ZoneInterval, UnknownWhy> {
    let zone = &zones.zones()[usize::from(index)];
    let kind = &zones.kinds()[usize::from(zone.kind)];
    let risk = risks.kind(zone.kind).ok_or(UnknownWhy::MissingFacts)?;
    let mut envelope_first = None;
    let mut envelope_last = None;
    let mut last_after = None;
    for i in usize::from(start)..path.len() {
        let i = i as u16;
        let tile = path.point(i).ok_or(UnknownWhy::Overflow)?.tile;
        if in_envelope(tile, index, zones, risks, input)? {
            envelope_first.get_or_insert(i);
            envelope_last = Some(i);
            if i >= a {
                last_after = Some(i);
            }
        } else if a == 0 && envelope_first.is_some() {
            // Bound the origin's escape at its first exit. An entering
            // interval keeps the existing conservative exposure envelope.
            break;
        }
    }
    let e = envelope_first.unwrap_or(a);
    let f = envelope_last.unwrap_or(a);
    let b = last_after.unwrap_or(a);
    ZoneInterval::new(
        index,
        kind.npc_id,
        WorldTile {
            x: zone.spawn_x,
            z: zone.spawn_z,
            level: i32::from(zone.level),
        },
        a,
        b,
        e,
        f,
        risk.r,
        risk.reach,
        risk.max_hit,
        risk.rate,
        risk.style,
        risk.unknown_for(input.map_members).is_some(),
        risk.ap(),
        kind.npc_id < 0,
    )
}

/// Retains every interval/crossing, not just the eight display witnesses.
/// Two allocation-free passes allocate exactly-sized output slices.
pub fn build_plan(
    path: RoutePath<'_>,
    zones: &ZoneTable,
    risks: &RiskTables,
    wilderness: &WildernessRules,
    input: &RiskInput,
) -> Result<RoutePlan, UnknownWhy> {
    let mut count = 0usize;
    acquired(path, zones, risks, wilderness, input, |_| {
        count += 1;
        Ok(())
    })?;
    if count > usize::from(u16::MAX) {
        return Err(UnknownWhy::Overflow);
    }
    let mut intervals = Vec::with_capacity(count);
    acquired(path, zones, risks, wilderness, input, |row| {
        intervals.push(row);
        Ok(())
    })?;
    intervals.sort_unstable_by_key(|row| (row.a, row.b, row.zone));
    // Count crossing runs first so conversion to Box never shrinks/reallocates.
    let crossing_count = crossing_runs(path, &intervals, |_| {})?;
    let mut crossings = Vec::with_capacity(crossing_count);
    crossing_runs(path, &intervals, |row| crossings.push(row))?;
    Ok(RoutePlan {
        complete: true,
        intervals: intervals.into_boxed_slice(),
        crossings: crossings.into_boxed_slice(),
    })
}
/// An 8-KiB worker-stack bitset replaces a heap scratch allocation. We visit
/// the table's indexed bucket candidates, never every world zone per tile.
fn acquired(
    path: RoutePath<'_>,
    zones: &ZoneTable,
    risks: &RiskTables,
    wilderness: &WildernessRules,
    input: &RiskInput,
    mut visit: impl FnMut(ZoneInterval) -> Result<(), UnknownWhy>,
) -> Result<(), UnknownWhy> {
    let mut seen = [0u64; 1024];
    for i in 0..path.len() {
        let i = i as u16;
        let tile = path.point(i).ok_or(UnknownWhy::Overflow)?.tile;
        for index in zones.at(tile) {
            let zone = &zones.zones()[usize::from(index)];
            let active = zone.class == ZoneClass::Always
                || input
                    .combat
                    .is_none_or(|combat| u16::from(combat) <= zone.cap)
                || wilderness.contains(tile);
            let word = usize::from(index) / 64;
            let bit = 1u64 << (index % 64);
            if active && seen[word] & bit == 0 {
                seen[word] |= bit;
                let origin = path.point(0).ok_or(UnknownWhy::Overflow)?.tile;
                let mut next = if in_envelope(origin, index, zones, risks, input)? {
                    0
                } else {
                    i
                };
                let mut start = 0;
                loop {
                    let row = interval(path, zones, risks, input, index, next, start)?;
                    visit(row)?;
                    if !row.escaping() {
                        break;
                    }
                    let mut reentry = None;
                    for candidate in usize::from(row.b) + 1..path.len() {
                        let candidate = candidate as u16;
                        let tile = path.point(candidate).ok_or(UnknownWhy::Overflow)?.tile;
                        if zones.at(tile).any(|zone| zone == index)
                            && (zone.class == ZoneClass::Always
                                || input
                                    .combat
                                    .is_none_or(|combat| u16::from(combat) <= zone.cap)
                                || wilderness.contains(tile))
                        {
                            reentry = Some(candidate);
                            break;
                        }
                    }
                    let Some(candidate) = reentry else { break };
                    next = candidate;
                    start = row.b.checked_add(1).ok_or(UnknownWhy::Overflow)?;
                }
            }
        }
    }
    Ok(())
}
fn crossing_runs(
    path: RoutePath<'_>,
    intervals: &[ZoneInterval],
    mut emit: impl FnMut(CrossingGeom),
) -> Result<usize, UnknownWhy> {
    // Retain origin exposure for observation/replay, but never make leaving
    // it an admission crossing. Re-entering after its exit has a later `a`.
    let mut start = intervals.partition_point(|row| row.escaping());
    let mut count = 0usize;
    while start < intervals.len() {
        let first = intervals[start].a;
        let mut last = intervals[start].b;
        let mut e = intervals[start].e;
        let mut f = intervals[start].f;
        let mut end = start + 1;
        let mut tail = path.interval_end_tick(&intervals[start])?;
        while end < intervals.len() {
            let next = intervals[end];
            let gap = path.point(next.a).ok_or(UnknownWhy::Overflow)?.tick - tail;
            if gap >= 2 * super::consts::LAG + 3 {
                break;
            }
            last = last.max(next.b);
            e = e.min(next.e);
            f = f.max(next.f);
            tail = tail.max(path.interval_end_tick(&next)?);
            end += 1;
        }
        let leg = path.point(first).ok_or(UnknownWhy::Overflow)?.leg;
        let retreat = e
            .checked_sub(1)
            .filter(|i| path.point(*i).is_some_and(|p| p.leg == leg))
            .unwrap_or(CrossingGeom::NONE);
        let next = usize::from(f) + 1;
        let forward = if next < path.len() && path.forward_on_foot(first, next as u16) {
            if next == usize::from(CrossingGeom::NONE) {
                return Err(UnknownWhy::Overflow);
            }
            next as u16
        } else {
            CrossingGeom::NONE
        };
        emit(CrossingGeom {
            first,
            last,
            env_first: e,
            env_last: f,
            leg,
            intervals: (start as u16, end as u16),
            retreat,
            forward,
        });
        count += 1;
        start = end;
    }
    Ok(count)
}
