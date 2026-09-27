use super::*;
/// Frozen `config.ts` @ rs2b0t `beecd912` Ardougne SE bank / Barnaby pier / field.
pub const BRIMHAVEN_INSPECT_BANK: (i32, i32, i32) = (2655, 3283, 0);
pub const BRIMHAVEN_INSPECT_PIER: (i32, i32, i32) = (2683, 3272, 0);
pub const BRIMHAVEN_INSPECT_FIELD: (i32, i32, i32) = (2698, 3206, 0);
pub const BRIMHAVEN_INSPECT_BANK_RADIUS: i32 = 6;
pub const BRIMHAVEN_INSPECT_PIER_RADIUS: i32 = 8;
pub const BRIMHAVEN_INSPECT_FOOD_WITHDRAW: i32 = 20;
pub const BRIMHAVEN_INSPECT_BOAT_FARE_ROUNDTRIP: i32 = 60;

fn chebyshev(a: (i32, i32, i32), b: (i32, i32, i32)) -> i32 {
    if a.2 != b.2 {
        i32::MAX
    } else {
        (a.0 - b.0).abs().max((a.1 - b.1).abs())
    }
}

fn hop_loc_has(hops: &[RouteInspectHopFact], needle: &str) -> bool {
    hops.iter()
        .any(|hop| hop.loc_name.to_ascii_lowercase().contains(needle))
}

fn hop_loc_wrong_boat(hops: &[RouteInspectHopFact]) -> bool {
    hops.iter().any(|hop| {
        let name = hop.loc_name.to_ascii_lowercase();
        name.contains("thresnor") || name.contains("musa") || name.contains("port sarim")
    })
}

/// Authoritative inspect freshness for Core (not a new product policy).
///
/// `InspectNav.generation` starts at 0 and is the live generation even when
/// no terminal is published. `reset_inspect` (session nav reset, not catalog
/// Start) does `wrapping_add(1)` and `clear_published`; a later publish uses
/// `next_seq` from an empty latest (seq 1) stamped with the new generation.
/// `Observation` defaults (`generation`/`seq`/`live_generation` = 0,
/// `has_terminal` = false) are missing-projection placeholders, not "live
/// generation is 0". The narrow inspect projection therefore copies
/// `live_generation` even without a terminal so Start baseline can store the
/// real ring. Generation 0 is a legitimate first-session value.
///
/// Current publish: `has_terminal` and `terminal.generation == live_generation`.
/// Same live generation as the Start baseline: `seq` must advance past that
/// baseline seq. After `reset_inspect` the live generation changes; the new
/// ring's seq 1 is fresh even if the previous ring ended at seq 8. Old
/// generation / stale seq / unpublished seq 0 are rejected. v1 also requires
/// a registered `request_id != 0` at the cycle; v2 token uses `!= 0` then a
/// later distinct `request_id == 0` snapshot.
fn fresh_barnaby_inspect(now: &Observation, prior_live_generation: u64, prior_seq: u64) -> bool {
    if !now.route_inspect_has_terminal
        || now.route_inspect_generation != now.route_inspect_live_generation
        || !now.route_inspect_ok
        || !hop_loc_has(&now.route_inspect_hops, "barnaby")
        || hop_loc_wrong_boat(&now.route_inspect_hops)
    {
        return false;
    }
    if now.route_inspect_live_generation == prior_live_generation {
        now.route_inspect_seq > prior_seq
    } else {
        now.route_inspect_seq > 0
    }
}

pub fn brimhaven_moss_inspect_v1_baseline_ready(baseline: &Observation) -> bool {
    near(
        baseline.tile,
        BRIMHAVEN_INSPECT_BANK,
        BRIMHAVEN_INSPECT_BANK_RADIUS,
    ) && empty_pack(baseline)
        && baseline.item_id(LOBSTER_ID) == 0
        && baseline.item_id(COINS_ID) == 0
        && baseline.level("agility") >= 30
        && baseline.ingame
        && baseline.scene_state == 2
}

pub fn route_inspect_brimhaven_v2_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, BRIMHAVEN_INSPECT_PIER, 4) && baseline.ingame && baseline.scene_state == 2
}

/// Ordered v1 witness: restock after empty-pack Start, then a fresh accepted
/// Barnaby inspect (`request_id != 0`), then a later observation with an
/// actual tile change toward/at the pier. First `accepted_tile` is kept.
/// Seed, fallback-without-accept, same-frame pier, wrong-boat, and stale
/// generation cannot qualify.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BrimhavenMossInspectCycle {
    pub restocked: bool,
    pub accepted_seq: Option<u64>,
    pub accepted_tile: Option<(i32, i32, i32)>,
    pub walk_progress: bool,
}

impl BrimhavenMossInspectCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        if !self.restocked
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(LOBSTER_ID) >= BRIMHAVEN_INSPECT_FOOD_WITHDRAW
            && now.item_id(COINS_ID) >= BRIMHAVEN_INSPECT_BOAT_FARE_ROUNDTRIP
            && baseline.item_id(LOBSTER_ID) == 0
            && baseline.item_id(COINS_ID) == 0
        {
            self.restocked = true;
        }
        if self.restocked
            && self.accepted_seq.is_none()
            && now.route_inspect_request_id != 0
            && fresh_barnaby_inspect(
                now,
                baseline.route_inspect_live_generation,
                baseline.route_inspect_seq,
            )
        {
            self.accepted_seq = Some(now.route_inspect_seq);
            // First accepted tile only; repeated later terminals must not refresh it.
            self.accepted_tile = now.tile;
        }
        if let (Some(_), Some(from)) = (self.accepted_seq, self.accepted_tile) {
            let later_tile = now.tile.filter(|tile| *tile != from);
            if let Some(tile) = later_tile {
                self.walk_progress |= near(
                    Some(tile),
                    BRIMHAVEN_INSPECT_PIER,
                    BRIMHAVEN_INSPECT_PIER_RADIUS,
                ) || chebyshev(tile, BRIMHAVEN_INSPECT_PIER)
                    < chebyshev(from, BRIMHAVEN_INSPECT_PIER);
            }
        }
    }

    pub fn qualified(&self) -> bool {
        self.restocked && self.accepted_seq.is_some() && self.walk_progress
    }
}

/// Ordered v2 witness: consumed token result, then a later request_id 0
/// result, then ordinary arrival on a distinct bank tile.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RouteInspectBrimhavenV2Cycle {
    pub token_seq: Option<u64>,
    pub snap0_seq: Option<u64>,
    pub walked: bool,
}

impl RouteInspectBrimhavenV2Cycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        if self.token_seq.is_none()
            && now.route_inspect_request_id != 0
            && fresh_barnaby_inspect(
                now,
                baseline.route_inspect_live_generation,
                baseline.route_inspect_seq,
            )
        {
            self.token_seq = Some(now.route_inspect_seq);
        }
        if let Some(token_seq) = self.token_seq {
            if self.snap0_seq.is_none()
                && now.route_inspect_request_id == 0
                && fresh_barnaby_inspect(
                    now,
                    // id0 is a later publish on this same live ring; seq must
                    // advance past the token. v2 request_id 0 is distinct from
                    // a v1 registered identity.
                    now.route_inspect_live_generation,
                    token_seq,
                )
            {
                self.snap0_seq = Some(now.route_inspect_seq);
            }
        }
        if self.snap0_seq.is_some() {
            self.walked |= near(
                now.tile,
                BRIMHAVEN_INSPECT_BANK,
                BRIMHAVEN_INSPECT_BANK_RADIUS,
            ) && !near(now.tile, BRIMHAVEN_INSPECT_PIER, 4);
        }
    }

    pub fn qualified(&self) -> bool {
        self.token_seq.is_some() && self.snap0_seq.is_some() && self.walked
    }
}
