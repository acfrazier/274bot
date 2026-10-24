use super::{
    drive_synchronously, CrossingGeom, KindRisk, RiskInput, RiskTables, RoutePlan, UnknownWhy,
    ZoneInterval,
};
use api::WorldTile;
use nav_planner::contract::{
    LegView, RouteView, TransportKind, WildernessView, ZoneClass, ZoneKey, ZoneTableView, ZoneView,
};
use std::future::Future;

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
/// The route is the owner's retained one, seen through its facade view.
#[derive(Clone, Copy)]
pub struct RoutePath<'a> {
    view: RouteView<'a>,
    len: usize,
    ticks: i32,
}
impl<'a> RoutePath<'a> {
    pub fn new(view: RouteView<'a>) -> Result<Self, UnknownWhy> {
        if view.leg_count() > 256 {
            return Err(UnknownWhy::Overflow);
        }
        let mut len = 0usize;
        let mut ticks = 0i32;
        for (leg, row) in view.legs().enumerate() {
            let (n, t) = match row {
                LegView::Walk(tiles) => (tiles.len(), walk_ticks(tiles.len())?),
                LegView::Transport(hop) => {
                    if hop.ticks() < 0 {
                        return Err(UnknownWhy::Overflow);
                    }
                    (usize::from(!attached(view, leg)), hop.ticks())
                }
            };
            len = len.checked_add(n).ok_or(UnknownWhy::Overflow)?;
            ticks = ticks.checked_add(t).ok_or(UnknownWhy::Overflow)?;
        }
        if len > usize::from(u16::MAX) + 1 {
            return Err(UnknownWhy::Overflow);
        }
        Ok(Self { view, len, ticks })
    }
    /// The route this path indexes.
    pub fn view(self) -> RouteView<'a> {
        self.view
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
        for (leg, row) in self.view.legs().enumerate() {
            match row {
                LegView::Walk(tiles) => {
                    let local = usize::from(index).checked_sub(offset)?;
                    if local < tiles.len() {
                        let start = walk_ticks(local).ok()?;
                        let walk_duration = walk_ticks(local + 1).ok()? - start;
                        let mut hold_ticks = 0;
                        if local + 1 == tiles.len() {
                            if let Ok(LegView::Transport(hop)) = self.view.leg(leg + 1) {
                                if hop.effective_takeoff() == tiles[local] {
                                    hold_ticks = hop.ticks();
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
                LegView::Transport(hop) => {
                    if !attached(self.view, leg) {
                        if offset == usize::from(index) {
                            return Some(RoutePoint {
                                tile: hop.effective_takeoff(),
                                leg: leg as u8,
                                tick,
                                duration: hop.ticks(),
                                hold_ticks: hop.ticks(),
                            });
                        }
                        offset += 1;
                    }
                    tick += hop.ticks();
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
        for (leg, row) in self.view.legs().enumerate() {
            match row {
                LegView::Walk(tiles) => offset += tiles.len(),
                LegView::Transport(hop) => {
                    let takeoff = if attached(self.view, leg) {
                        offset - 1
                    } else {
                        offset
                    };
                    if takeoff >= usize::from(lo) && takeoff < usize::from(hi) {
                        cost = cost.checked_add(hop.ticks()).ok_or(UnknownWhy::Overflow)?;
                    }
                    if !attached(self.view, leg) {
                        offset += 1;
                    }
                }
            }
        }
        Ok(cost)
    }
    fn forward_on_foot(self, from: u16, to: u16) -> bool {
        let mut offset = 0usize;
        for (leg, row) in self.view.legs().enumerate() {
            match row {
                LegView::Walk(tiles) => offset += tiles.len(),
                LegView::Transport(hop) => {
                    let takeoff = if attached(self.view, leg) {
                        offset - 1
                    } else {
                        offset
                    };
                    if hop.kind() == TransportKind::Teleport
                        && (usize::from(from)..usize::from(to)).contains(&takeoff)
                    {
                        return false;
                    }
                    if !attached(self.view, leg) {
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
        for (leg, row) in self.view.legs().enumerate() {
            match row {
                LegView::Walk(tiles) => {
                    let end = offset + tiles.len();
                    let lo = offset.max(usize::from(from));
                    let hi = end.min(usize::from(to) + 1);
                    walks += hi.saturating_sub(lo);
                    offset = end;
                }
                LegView::Transport(hop) => {
                    let takeoff = if attached(self.view, leg) {
                        offset - 1
                    } else {
                        offset
                    };
                    if (usize::from(from)..=usize::from(to)).contains(&takeoff) {
                        holds = holds.checked_add(hop.ticks()).ok_or(UnknownWhy::Overflow)?;
                    }
                    if !attached(self.view, leg) {
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
/// A transport leg is attached when the preceding walk ends on its
/// effective takeoff: the hop then charges its hold on that walk tile
/// instead of occupying a route index of its own.
fn attached(view: RouteView<'_>, leg: usize) -> bool {
    let Ok(LegView::Transport(hop)) = view.leg(leg) else {
        return false;
    };
    leg.checked_sub(1)
        .and_then(|previous| view.leg(previous).ok())
        .is_some_and(|row| {
            matches!(row, LegView::Walk(tiles) if tiles.last() == Some(&hop.effective_takeoff()))
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

/// The pack row index behind a zone view (its bound key is always a zone
/// row, never a group): a dense per-pack key for deduplicating the zones of
/// one table view, never a lookup into any other table.
pub(super) fn zone_index(zone: ZoneView<'_>) -> Result<u16, UnknownWhy> {
    match zone.key().key() {
        ZoneKey::Zone(index) => Ok(index),
        ZoneKey::Group(_) => Err(UnknownWhy::NavIdentity),
    }
}

/// The pack row index a retained interval names (ordering only; the key is
/// always a zone row).
pub(super) fn row_index(row: &ZoneInterval) -> u16 {
    match row.zone.key() {
        ZoneKey::Zone(index) | ZoneKey::Group(index) => index,
    }
}

fn in_envelope(tile: WorldTile, zone: ZoneView<'_>, risk: &KindRisk, input: &RiskInput) -> bool {
    if zone.has_shape()
        || zone.kind().npc_id().is_none_or(|id| id < 0)
        || risk.unknown_for(input.map_members).is_some()
    {
        zone.contains(tile)
    } else {
        let (spawn_x, spawn_z) = zone.spawn();
        tile.level == i32::from(zone.plane())
            && (i64::from(tile.x) - i64::from(spawn_x)).abs() <= i64::from(risk.r)
            && (i64::from(tile.z) - i64::from(spawn_z)).abs() <= i64::from(risk.r)
    }
}

async fn interval<F, U>(
    path: RoutePath<'_>,
    zone: ZoneView<'_>,
    risks: &RiskTables,
    input: &RiskInput,
    a: u16,
    start: u16,
    checkpoint: &mut F,
) -> Result<ZoneInterval, UnknownWhy>
where
    F: FnMut() -> U,
    U: Future<Output = ()>,
{
    let kind = zone.kind();
    let risk = risks.zone_kind(kind);
    let mut envelope_first = None;
    let mut envelope_last = None;
    let mut last_after = None;
    for i in usize::from(start)..path.len() {
        let i = i as u16;
        let tile = path.point(i).ok_or(UnknownWhy::Overflow)?.tile;
        let mut stop = false;
        if in_envelope(tile, zone, &risk, input) {
            envelope_first.get_or_insert(i);
            envelope_last = Some(i);
            if i >= a {
                last_after = Some(i);
            }
        } else if a == 0 && envelope_first.is_some() {
            // Bound the origin's escape at its first exit. An entering
            // interval keeps the existing conservative exposure envelope.
            stop = true;
        }
        super::checkpoint(checkpoint).await;
        if stop {
            break;
        }
    }
    let e = envelope_first.unwrap_or(a);
    let f = envelope_last.unwrap_or(a);
    let b = last_after.unwrap_or(a);
    let (spawn_x, spawn_z) = zone.spawn();
    ZoneInterval::new(
        zone.key(),
        kind.npc_id().unwrap_or(-1),
        WorldTile {
            x: spawn_x,
            z: spawn_z,
            level: i32::from(zone.plane()),
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
        risk.unknown_for(input.map_members),
        risk.ap(),
        kind.npc_id().is_none_or(|id| id < 0),
    )
}

/// Retains every interval/crossing, not just the eight display witnesses.
/// Two allocation-free passes allocate exactly-sized output slices.
pub fn build_plan(
    path: RoutePath<'_>,
    zones: ZoneTableView<'_>,
    risks: &RiskTables,
    wilderness: WildernessView<'_>,
    input: &RiskInput,
) -> Result<RoutePlan, UnknownWhy> {
    let mut checkpoint = || std::future::ready(());
    drive_synchronously(build_plan_cooperative(
        path,
        zones,
        risks,
        wilderness,
        input,
        &mut checkpoint,
    ))
}

pub(super) async fn build_plan_cooperative<F, U>(
    path: RoutePath<'_>,
    zones: ZoneTableView<'_>,
    risks: &RiskTables,
    wilderness: WildernessView<'_>,
    input: &RiskInput,
    checkpoint: &mut F,
) -> Result<RoutePlan, UnknownWhy>
where
    F: FnMut() -> U,
    U: Future<Output = ()>,
{
    if path.view().pack() != zones.pack() {
        return Err(UnknownWhy::NavIdentity);
    }
    let mut count = 0usize;
    acquired(path, zones, risks, wilderness, input, checkpoint, |_| {
        count += 1;
        Ok(())
    })
    .await?;
    if count > usize::from(u16::MAX) {
        return Err(UnknownWhy::Overflow);
    }
    let mut intervals = Vec::with_capacity(count);
    acquired(path, zones, risks, wilderness, input, checkpoint, |row| {
        intervals.push(row);
        Ok(())
    })
    .await?;
    sort_intervals(&mut intervals, checkpoint).await;
    // Count crossing runs first so conversion to Box never shrinks/reallocates.
    let crossing_count = crossing_runs(path, &intervals, checkpoint, |_| {}).await?;
    let mut crossings = Vec::with_capacity(crossing_count);
    crossing_runs(path, &intervals, checkpoint, |row| crossings.push(row)).await?;
    Ok(RoutePlan {
        complete: true,
        pack: Some(zones.pack()),
        intervals: intervals.into_boxed_slice(),
        crossings: crossings.into_boxed_slice(),
    })
}
/// The router's activity predicate at one tile, against the assessed combat
/// level (unknown activates every level-rule zone, as the router does).
fn active(
    zone: ZoneView<'_>,
    input: &RiskInput,
    wilderness: WildernessView<'_>,
    tile: WorldTile,
) -> bool {
    zone.class() == ZoneClass::Always
        || input
            .combat
            .is_none_or(|combat| zone.cap().is_some_and(|cap| u16::from(combat) <= cap))
        || wilderness.contains(tile)
}
/// Whether a zone is engaged at `tile`: inside its acquisition area and
/// active there.
pub(super) fn engaged_at(
    zone: ZoneView<'_>,
    input: &RiskInput,
    wilderness: WildernessView<'_>,
    tile: WorldTile,
) -> bool {
    zone.contains(tile) && active(zone, input, wilderness, tile)
}
/// An 8-KiB worker-stack bitset replaces a heap scratch allocation. We visit
/// the table's indexed bucket candidates, never every world zone per tile.
async fn acquired<F, U>(
    path: RoutePath<'_>,
    zones: ZoneTableView<'_>,
    risks: &RiskTables,
    wilderness: WildernessView<'_>,
    input: &RiskInput,
    checkpoint: &mut F,
    mut visit: impl FnMut(ZoneInterval) -> Result<(), UnknownWhy>,
) -> Result<(), UnknownWhy>
where
    F: FnMut() -> U,
    U: Future<Output = ()>,
{
    let mut seen = [0u64; 1024];
    for i in 0..path.len() {
        let i = i as u16;
        let tile = path.point(i).ok_or(UnknownWhy::Overflow)?.tile;
        for zone in zones.containing(tile) {
            let index = zone_index(zone)?;
            let word = usize::from(index) / 64;
            let bit = 1u64 << (index % 64);
            if active(zone, input, wilderness, tile) && seen[word] & bit == 0 {
                seen[word] |= bit;
                // `i` is the first acquisition index. It is 0 only when the
                // origin is engaged (inside A_z and active), the router's own
                // origin predicate. A fringe origin inside E_z but outside A_z
                // that walks into A_z enters the zone: a judged crossing.
                let mut next = i;
                let mut start = 0;
                loop {
                    let row = interval(path, zone, risks, input, next, start, checkpoint).await?;
                    visit(row)?;
                    if !row.escaping() {
                        break;
                    }
                    let mut reentry = None;
                    for candidate in usize::from(row.b) + 1..path.len() {
                        let candidate = candidate as u16;
                        let tile = path.point(candidate).ok_or(UnknownWhy::Overflow)?.tile;
                        let engaged = engaged_at(zone, input, wilderness, tile);
                        super::checkpoint(checkpoint).await;
                        if engaged {
                            reentry = Some(candidate);
                            break;
                        }
                    }
                    let Some(candidate) = reentry else { break };
                    next = candidate;
                    start = row.b.checked_add(1).ok_or(UnknownWhy::Overflow)?;
                }
            }
            super::checkpoint(checkpoint).await;
        }
        super::checkpoint(checkpoint).await;
    }
    Ok(())
}

async fn crossing_runs<F, U>(
    path: RoutePath<'_>,
    intervals: &[ZoneInterval],
    checkpoint: &mut F,
    mut emit: impl FnMut(CrossingGeom),
) -> Result<usize, UnknownWhy>
where
    F: FnMut() -> U,
    U: Future<Output = ()>,
{
    // Retain origin exposure for observation/replay, but never make leaving
    // it an admission crossing. Re-entering after its exit has a later `a`.
    // `ZoneInterval::new` enforces `e <= a`, so every `a = 0` row is escaping
    // and, sorted by `a`, escaping rows are exactly the prefix. (An engaged
    // origin outside its own envelope fails as `Unknown(Overflow)` instead.)
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
            let separate = gap >= 2 * super::consts::LAG + 3;
            super::checkpoint(checkpoint).await;
            if separate {
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
        super::checkpoint(checkpoint).await;
    }
    Ok(count)
}

async fn sort_intervals<F, U>(rows: &mut [ZoneInterval], checkpoint: &mut F)
where
    F: FnMut() -> U,
    U: Future<Output = ()>,
{
    async fn sift_down<F, U>(
        rows: &mut [ZoneInterval],
        mut root: usize,
        end: usize,
        checkpoint: &mut F,
    ) where
        F: FnMut() -> U,
        U: Future<Output = ()>,
    {
        while root < end / 2 {
            let left = root * 2 + 1;
            let mut largest = left;
            if left + 1 < end
                && (
                    rows[left + 1].a,
                    rows[left + 1].b,
                    row_index(&rows[left + 1]),
                ) > (rows[left].a, rows[left].b, row_index(&rows[left]))
            {
                largest = left + 1;
            }
            if (rows[root].a, rows[root].b, row_index(&rows[root]))
                >= (rows[largest].a, rows[largest].b, row_index(&rows[largest]))
            {
                super::checkpoint(checkpoint).await;
                break;
            }
            rows.swap(root, largest);
            root = largest;
            super::checkpoint(checkpoint).await;
        }
    }

    let len = rows.len();
    for root in (0..len / 2).rev() {
        sift_down(rows, root, len, checkpoint).await;
    }
    for end in (1..len).rev() {
        rows.swap(0, end);
        super::checkpoint(checkpoint).await;
        sift_down(rows, 0, end, checkpoint).await;
    }
}
