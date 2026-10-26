//! Conservative, pure route estimates. This module neither admits a walk nor
//! observes a live slot; callers retain the immutable route used by the plan.
//!
//! Routes, zones and wilderness reach this module only as `nav_planner::contract`
//! views (T5.1, I-nav-closure): the host manufactures them at the owner
//! boundary, and every retained zone identity is a pack-bound key, so a plan
//! judged against another pack's table is refused, never misindexed. A
//! zone's risk facts come from its facade kind view (`ZoneKindView`) and the
//! NPC-keyed [`RiskTables`]; no kind-row order or pack index reaches here.
mod facts;
mod geometry;
mod input;
mod replay;
use api::condition::PoisonState;
use api::WorldTile;
pub(crate) use facts::poison_unknown_v_rows;
pub use facts::*;
/// Return the process-local build count for risk-table test assertions.
#[cfg(feature = "test-hooks")]
pub fn risk_table_build_count_for_test() -> usize {
    facts::build_count_for_test()
}
pub use geometry::{build_plan, RoutePath};
pub use input::*;
use nav_planner::contract::{BoundZoneKey, ContractError, PackId, ZoneKey, ZoneTableView};
pub use replay::{
    admission_passes, admission_passes_cooperative, assess, assess_cooperative, candidate_cost,
    eat_line, estimated_floor, replay, Estimate, Grants, ReplayResult,
};
use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

pub(super) async fn checkpoint<F, U>(checkpoint: &mut F)
where
    F: FnMut() -> U,
    U: Future<Output = ()>,
{
    checkpoint().await;
}

/// Drive a risk kernel whose checkpoint is immediately ready. The async and
/// synchronous APIs therefore execute the same kernel without a runtime,
/// allocation, or unsafe pin projection.
pub(super) fn drive_synchronously<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("synchronous risk checkpoint suspended"),
    }
}

const MAX_PLAN_ROWS: usize = u16::MAX as usize;
const MAX_DISPLAY_ROWS: usize = 8;
const MAX_SUPPLY_ROWS: usize = 6;
const MAX_REASON_BYTES: usize = 1536;

/// Conservative owned-heap bounds for one assessment. `peak_bytes` includes
/// all simultaneous scratch and retained allocations; `retained_bytes` is
/// the largest returned `Arc<RouteAssessment>` and its separately backed
/// collections/reason. Sorting is in-place, and exact-capacity vectors make
/// their boxed conversions allocation-free. The retained input route and
/// pinned future layout are accounted for by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RiskHeapBounds {
    pub peak_bytes: usize,
    pub retained_bytes: usize,
}

/// Worst-case assessment heap under the existing u16 plan-count limits.
/// `None` means the checked upper-bound arithmetic cannot be represented.
pub fn assessment_heap_bounds() -> Option<RiskHeapBounds> {
    let plan_rows = MAX_PLAN_ROWS.checked_mul(std::mem::size_of::<ZoneInterval>())?;
    let crossing_rows = MAX_PLAN_ROWS.checked_mul(std::mem::size_of::<CrossingGeom>())?;
    let plan = plan_rows.checked_add(crossing_rows)?;
    let display = MAX_DISPLAY_ROWS.checked_mul(std::mem::size_of::<Crossing>())?;
    let supplies = MAX_SUPPLY_ROWS.checked_mul(std::mem::size_of::<SupplyNeed>())?;
    let arc_header = 2usize.checked_mul(std::mem::size_of::<usize>())?;
    let reason = arc_header.checked_add(MAX_REASON_BYTES)?;
    let allocation_alignment =
        std::mem::align_of::<RouteAssessment>().max(std::mem::align_of::<usize>());
    let alignment_slack = allocation_alignment.checked_sub(1)?.checked_mul(2)?;
    let assessment = arc_header
        .checked_add(std::mem::size_of::<RouteAssessment>())?
        .checked_add(alignment_slack)?;
    let retained_bytes = plan
        .checked_add(display)?
        .checked_add(supplies)?
        .checked_add(reason)?
        .checked_add(assessment)?;
    Some(RiskHeapBounds {
        peak_bytes: retained_bytes,
        retained_bytes,
    })
}

/// Worst-case total heap while admission retains an assessment and its
/// temporary projected replay plan and crossing-obligation bitset. Projection
/// uses exact capacities and cannot allocate a second store while shrinking.
pub fn admission_peak_heap_bytes() -> Option<usize> {
    let projected_rows = MAX_PLAN_ROWS.checked_mul(std::mem::size_of::<ZoneInterval>())?;
    let projected_crossings = MAX_PLAN_ROWS.checked_mul(std::mem::size_of::<CrossingGeom>())?;
    let projected_plan = projected_rows.checked_add(projected_crossings)?;
    let mask_words = MAX_PLAN_ROWS.div_ceil(64);
    let mask = mask_words.checked_mul(std::mem::size_of::<u64>())?;
    assessment_heap_bounds()?
        .peak_bytes
        .checked_add(projected_plan)?
        .checked_add(mask)
}

/// A facade refusal: the route, plan, key or exemption belongs to another
/// nav pack, or names a row this pack does not have.
fn nav_identity(_: ContractError) -> UnknownWhy {
    UnknownWhy::NavIdentity
}

/// Evidence-calibrated tuning, not a survival proof (design §2).
pub mod consts {
    pub const STOP_NUM: i32 = 3;
    pub const STOP_DEN: i32 = 2;
    pub const LAG: i32 = 2;
    /// Shared ranged flight is ceil((32 + 5 * distance) / 30).
    /// Tuning/inference: 289 has no pure-ranged rows. Future rows whose derived
    /// flight exceeds this cap are explicitly unsupported, never understated.
    pub const PROJ_LAG: i32 = 3;
    pub const PACE_AHEAD: u16 = 2;
    pub const TOPUP_WINDOW: i32 = 30;
    pub const POISON_PERIOD: i32 = 30;
}

/// A validated engine coordinate, preserving all supported world tiles in 4 B.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackedTile(u32);
impl PackedTile {
    pub fn new(tile: WorldTile) -> Result<Self, UnknownWhy> {
        if !(0..=16383).contains(&tile.x)
            || !(0..=16383).contains(&tile.z)
            || !(0..=3).contains(&tile.level)
        {
            return Err(UnknownWhy::Overflow);
        }
        Ok(Self(
            tile.x as u32 | ((tile.z as u32) << 14) | ((tile.level as u32) << 28),
        ))
    }
    pub fn tile(self) -> WorldTile {
        WorldTile {
            x: (self.0 & 0x3fff) as i32,
            z: ((self.0 >> 14) & 0x3fff) as i32,
            level: ((self.0 >> 28) & 3) as i32,
        }
    }
}
/// Complete geometry for one engaged zone. The bound zone key supplies the
/// canonical group key and exact stationary shape through the pack's table
/// view; indices address the retained immutable route. NPC ids outside i16
/// are explicitly Unknown(Overflow).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneInterval {
    pub spawn: PackedTile,
    ident: i16,
    /// The engaged zone row, bound to the pack the plan was built on.
    pub zone: BoundZoneKey,
    /// Why the row's kind is unsupported for the assessed world, if it is.
    unknown: Option<UnknownKind>,
    pub a: u16,
    pub b: u16,
    pub e: u16,
    pub f: u16,
    pub r: u8,
    pub reach: u8,
    pub max_hit: u8,
    pub rate: u8,
    pub style: Style,
    flags: u8,
}
impl ZoneInterval {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        zone: BoundZoneKey,
        ident: i32,
        spawn: WorldTile,
        a: u16,
        b: u16,
        e: u16,
        f: u16,
        r: u8,
        reach: u8,
        max_hit: u8,
        rate: u8,
        style: Style,
        unknown: Option<UnknownKind>,
        ap: bool,
        hazard: bool,
    ) -> Result<Self, UnknownWhy> {
        if ident < -1 || a > b || e > a || f < b {
            return Err(UnknownWhy::Overflow);
        }
        Ok(Self {
            spawn: PackedTile::new(spawn)?,
            ident: i16::try_from(ident).map_err(|_| UnknownWhy::Overflow)?,
            zone,
            unknown,
            a,
            b,
            e,
            f,
            r,
            reach,
            max_hit,
            rate,
            style,
            flags: (u8::from(ap) << 1) | (u8::from(hazard) << 2),
        })
    }
    pub fn ident(self) -> i32 {
        i32::from(self.ident)
    }
    /// The canonical opt-out identity of the engaged row: its group key when
    /// the pack groups it, else the zone key. Refused against another pack.
    pub fn key(self, zones: ZoneTableView<'_>) -> Result<ZoneKey, UnknownWhy> {
        let view = zones.zone(self.zone).map_err(nav_identity)?;
        Ok(view.group().unwrap_or(self.zone).key())
    }
    pub fn unknown(self) -> bool {
        self.unknown.is_some()
    }
    /// The unsupported class behind [`Self::unknown`].
    pub fn unknown_kind(self) -> Option<UnknownKind> {
        self.unknown
    }
    pub fn ap(self) -> bool {
        self.flags & 2 != 0
    }
    pub fn hazard(self) -> bool {
        self.flags & 4 != 0
    }
    /// The origin-to-first-exit interval is exposure, not entry authority.
    pub fn escaping(self) -> bool {
        self.a == 0 && self.e == 0
    }
    /// S2 always sums. S3 may set this only after its coverage/census gate.
    pub fn single_safe(self) -> bool {
        self.flags & 8 != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrossingGeom {
    pub first: u16,
    pub last: u16,
    pub env_first: u16,
    pub env_last: u16,
    pub intervals: (u16, u16),
    pub retreat: u16,
    pub forward: u16,
    pub leg: u8,
}
impl CrossingGeom {
    pub const NONE: u16 = u16::MAX;
    pub fn has_way_out(self) -> bool {
        self.retreat != Self::NONE || self.forward != Self::NONE
    }
}
#[derive(Debug, Default)]
pub struct RoutePlan {
    /// False only when geometry could not be assessed. An empty complete plan
    /// proves there is no entering crossing, even when live inputs are unknown.
    pub complete: bool,
    /// The pack whose table built this plan; `None` on an incomplete plan.
    /// Admission refuses a plan against another pack's table.
    pub pack: Option<PackId>,
    pub intervals: Box<[ZoneInterval]>,
    pub crossings: Box<[CrossingGeom]>,
    // Walk-leg starts are derived from the retained route, not separately allocated.
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crossing {
    pub key: ZoneKey,
    pub first: u16,
    pub last: u16,
    pub ticks: u16,
    pub worst: u16,
    pub volley: u8,
    pub max_hit: u8,
    pub rate: u8,
    pub style: Style,
    pub floor_deep: u8,
    pub bites: u8,
    pub single: bool,
    pub protect_credited: bool,
    pub unknown: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoNeed {
    Protect { style: Style, level: u8 },
    PrayerPoints { needed: u16 },
    Antifire,
    Armour,
    Antipoison,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupplyNeed {
    Food {
        heal_at_least: u8,
        heal_at_most: u8,
        count: u8,
        item: Option<i32>,
    },
    Info(InfoNeed),
}
#[derive(Debug)]
pub struct RouteAssessment {
    pub verdict: Verdict,
    pub plan: RoutePlan,
    pub crossings: Box<[Crossing]>,
    pub more: u8,
    pub supplies: Box<[SupplyNeed]>,
    pub hp_after: u8,
    pub volley: u8,
    pub input: RiskInput,
    pub generation: u64,
    pub reason: Arc<str>,
}

/// S2a has no scene-coverage certificate: the identity-uncertain live/estimated
/// union is always summed, even with a built or count-limited nearby scene.
pub fn attacker_count(live: u16, estimated: u16) -> Result<u16, UnknownWhy> {
    live.checked_add(estimated).ok_or(UnknownWhy::Overflow)
}

#[cfg(test)]
mod poison_tests;
#[cfg(test)]
mod tests;
