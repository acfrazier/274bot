//! Conservative, pure route estimates. This module neither admits a walk nor
//! observes a live slot; callers retain the immutable route used by the plan.
mod facts;
mod geometry;
mod input;
mod replay;
use api::WorldTile;
pub use facts::*;
pub use geometry::{build_plan, RoutePath};
pub use input::*;
use nav::zones::{ZoneKey, ZoneTable};
pub use replay::{
    admission_passes, assess, candidate_cost, eat_line, estimated_floor, replay, Estimate, Grants,
    ReplayResult,
};
use std::sync::Arc;

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

/// Complete geometry for one engaged zone. The bound zone table supplies the
/// canonical group key and exact stationary shape; indices address the retained
/// immutable route. NPC ids outside i16 are explicitly Unknown(Overflow).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneInterval {
    pub spawn: PackedTile,
    ident: i16,
    pub zone: u16,
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
        zone: u16,
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
        unknown: bool,
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
            a,
            b,
            e,
            f,
            r,
            reach,
            max_hit,
            rate,
            style,
            flags: u8::from(unknown) | (u8::from(ap) << 1) | (u8::from(hazard) << 2),
        })
    }
    pub fn ident(self) -> i32 {
        i32::from(self.ident)
    }
    pub fn key(self, zones: &ZoneTable) -> ZoneKey {
        zones.key(self.zone)
    }
    pub fn unknown(self) -> bool {
        self.flags & 1 != 0
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
