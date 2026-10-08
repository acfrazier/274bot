use super::*;
use crate::combat::{
    AbortReason, Allowances, CombatEnd, CombatReport, CombatRequest, Fallback, IntruderPolicy,
    Pick, PrayerMode, PrepReadiness, Style, Tactic, Target, Unprotected,
};
use crate::native::ActionError;
use api::WorldTile;
use std::sync::Arc;

/// The selected key-keeper step an identified membership row owns: the
/// `talk_key.keys` row whose own id is the row's, or `None` when the row is not
/// a key-hunt membership.
///
/// A sibling of `talk_step` and never a fold into it: the two families publish
/// different ids, so a talk step is never hunted and a keeper is never
/// Talked-to. The held clue stays the riddle the landed identify returned — the
/// key is not membership — and the row's own `key_id`, `keeper` and `spawn` are
/// what the arm reads.
pub(super) fn key_step(selected: Option<&SelectedGameData>, id: i32) -> Option<&TalkKeyKeyRow> {
    selected?.talk_key()?.keys.iter().find(|key| key.id == id)
}

/// The packed npc type one key-keeper row names: that keeper's own id and the
/// display name the posted npc page carries beside it, and nothing else.
///
/// Only a `type` matcher is one npc. A `category` or a bare `name` keeper
/// matches many and carries no packed id, so those rows have no type here and
/// stay idle rather than hunting the nearest anything. The matcher's own script
/// alias is never part of this identity — the posted page carries no alias at
/// all — and it is never substituted for the display name the verb carries.
pub(super) fn keeper_type(keeper: &TalkKeyKeeper) -> Option<(i32, &str)> {
    let id = keeper.id?;
    let name = keeper.name.as_deref().filter(|name| !name.is_empty())?;
    (keeper.kind == KEEPER_TYPE).then_some((id, name))
}

/// Verb `encounter`: guardian (wizard) or keeper (key NPC).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Encounter {
    Guardian,
    Keeper,
}

/// One combat hand-off. The only reader of the verb's fields, and the only
/// place the guardian/keeper radii live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Delegation {
    pub id: u32,
    pub npc_type: i32,
    stand: Tile,
    pub encounter: Encounter,
}

impl Delegation {
    pub(crate) fn parse(verb: &Value) -> Option<Self> {
        if verb.get("kind").and_then(Value::as_str) != Some("combat") {
            return None;
        }
        let id = verb
            .get("id")
            .and_then(Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())?;
        if id == 0 {
            return None;
        }
        let npc_type = posted_i32(verb, "npc_type")?;
        let stand = verb.get("stand")?;
        let stand = Tile {
            x: posted_i32(stand, "x")?,
            z: posted_i32(stand, "z")?,
            level: posted_i32(stand, "level")?,
        };
        let encounter = match verb.get("encounter").and_then(Value::as_str)? {
            "guardian" => Encounter::Guardian,
            "keeper" => Encounter::Keeper,
            _ => return None,
        };
        Some(Self {
            id,
            npc_type,
            stand,
            encounter,
        })
    }

    pub(crate) fn request(&self) -> CombatRequest {
        let (engage_radius, lost_radius) = match self.encounter {
            Encounter::Guardian => (GUARDIAN_RADIUS as u8, GUARDIAN_RADIUS as u8),
            Encounter::Keeper => (ARRIVE_RADIUS as u8, GUARDIAN_RADIUS as u8),
        };
        CombatRequest {
            target: Target::Npc {
                types: Arc::from([self.npc_type]),
                pick: Pick::Nearest,
                not_targeting_others: true,
            },
            tactic: Tactic::Open,
            style: Style::Melee,
            melee_mode: None,
            ranged_style: Default::default(),
            kit: None,
            spells: None,
            fallback_spells: false,
            stand: Some(WorldTile {
                x: self.stand.x,
                z: self.stand.z,
                level: self.stand.level,
            }),
            search_bounds: None,
            engage_radius,
            lost_radius,
            budget_ticks: 500,
            allow: Allowances::default(),
            fallback: Fallback::Abort,
            intruder: IntruderPolicy::default(),
            retaliate: true,
            prayer_mode: PrayerMode::Hold,
            until_ticks: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum OutcomeEnd {
    Killed = 0,
    TargetGone = 1,
    NoTarget = 2,
    Died = 3,
    Budget = 4,
    Aborted = 5,
    Cancelled = 6,
    UserInput = 7,
    Failed = 8,
}

const REASON_NONE: u8 = 0;
const REASON_NO_FOOD: u8 = 1;
const REASON_DRAGONFIRE: u8 = 2;
const REASON_NO_AMMO: u8 = 3;
const REASON_NO_RUNES: u8 = 4;
const REASON_UNATTACKABLE: u8 = 5;
const REASON_PREP_FAILED: u8 = 6;
const REASON_UNRESPONSIVE: u8 = 7;
const REASON_RETREATED: u8 = 8;
const REASON_RETREAT_FAILED: u8 = 9;
const REASON_SAFESPOT_BROKEN: u8 = 10;
const REASON_PREP_COMBAT_ROOT_MISSING: u8 = 11;
const REASON_PREP_COMBAT_ROOT_WRONG: u8 = 12;
const REASON_PREP_STYLE_ECHO_MISSING: u8 = 13;

/// Compact combat report for the `combat` page field. Built by the driver
/// from `Poll::Ready`; the machine only reads the JSON.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Outcome {
    id: u32,
    engaged_type: i32,
    ticks: u16,
    end: OutcomeEnd,
    reason: u8,
}

const _: () = assert!(std::mem::size_of::<Outcome>() <= 24);

impl Outcome {
    pub(crate) fn from_poll(id: u32, result: Result<CombatReport, ActionError>) -> Self {
        match result {
            Ok(report) => Self::from_report(id, report),
            Err(ActionError::Cancelled | ActionError::Stale) => Self::cancelled(id),
            Err(ActionError::UserInput) => Self::user_input(id),
            Err(_) => Self::failed(id),
        }
    }

    pub(crate) fn from_report(id: u32, report: CombatReport) -> Self {
        let (end, reason) = match report.end {
            CombatEnd::Killed => (OutcomeEnd::Killed, REASON_NONE),
            CombatEnd::TargetGone => (OutcomeEnd::TargetGone, REASON_NONE),
            CombatEnd::NoTarget => (OutcomeEnd::NoTarget, REASON_NONE),
            CombatEnd::Died => (OutcomeEnd::Died, REASON_NONE),
            CombatEnd::Budget => (OutcomeEnd::Budget, REASON_NONE),
            CombatEnd::Aborted(reason) => (OutcomeEnd::Aborted, abort_reason(reason)),
        };
        Self {
            id,
            engaged_type: report.engaged_npc_type,
            ticks: report.ticks,
            end,
            reason,
        }
    }

    pub(crate) fn cancelled(id: u32) -> Self {
        Self {
            id,
            engaged_type: -1,
            ticks: 0,
            end: OutcomeEnd::Cancelled,
            reason: REASON_NONE,
        }
    }

    pub(crate) fn user_input(id: u32) -> Self {
        Self {
            id,
            engaged_type: -1,
            ticks: 0,
            end: OutcomeEnd::UserInput,
            reason: REASON_NONE,
        }
    }

    pub(crate) fn failed(id: u32) -> Self {
        Self {
            id,
            engaged_type: -1,
            ticks: 0,
            end: OutcomeEnd::Failed,
            reason: REASON_NONE,
        }
    }

    pub(crate) fn page(self) -> Value {
        let mut page = json!({
            "id": self.id,
            "end": self.end_str(),
            "engaged_type": self.engaged_type,
            "ticks": self.ticks,
        });
        if let Some(reason) = self.reason_str() {
            page["reason"] = json!(reason);
        }
        page
    }

    fn end_str(self) -> &'static str {
        match self.end {
            OutcomeEnd::Killed => "killed",
            OutcomeEnd::TargetGone => "target_gone",
            OutcomeEnd::NoTarget => "no_target",
            OutcomeEnd::Died => "died",
            OutcomeEnd::Budget => "budget",
            OutcomeEnd::Aborted => "aborted",
            OutcomeEnd::Cancelled => "cancelled",
            OutcomeEnd::UserInput => "user_input",
            OutcomeEnd::Failed => "failed",
        }
    }

    fn reason_str(self) -> Option<&'static str> {
        Some(match self.reason {
            REASON_NO_FOOD => "no_food",
            REASON_DRAGONFIRE => "dragonfire",
            REASON_NO_AMMO => "no_ammo",
            REASON_NO_RUNES => "no_runes",
            REASON_UNATTACKABLE => "unattackable",
            REASON_PREP_FAILED => "prep_failed",
            REASON_PREP_COMBAT_ROOT_MISSING => "prep-readiness:combat-root-missing",
            REASON_PREP_COMBAT_ROOT_WRONG => "prep-readiness:combat-root-wrong",
            REASON_PREP_STYLE_ECHO_MISSING => "prep-readiness:style-echo-missing",
            REASON_UNRESPONSIVE => "unresponsive",
            REASON_RETREATED => "retreated",
            REASON_RETREAT_FAILED => "retreat_failed",
            REASON_SAFESPOT_BROKEN => "safespot_broken",
            _ => return None,
        })
    }
}

fn abort_reason(reason: AbortReason) -> u8 {
    match reason {
        AbortReason::Unprotected(Unprotected::NoFood) => REASON_NO_FOOD,
        AbortReason::Unprotected(Unprotected::Dragonfire) => REASON_DRAGONFIRE,
        AbortReason::Unprotected(Unprotected::NoAmmo) => REASON_NO_AMMO,
        AbortReason::Unprotected(Unprotected::NoRunes) => REASON_NO_RUNES,
        AbortReason::Unattackable => REASON_UNATTACKABLE,
        AbortReason::PrepFailed(_) => REASON_PREP_FAILED,
        AbortReason::PrepReadiness(PrepReadiness::CombatRootMissing) => {
            REASON_PREP_COMBAT_ROOT_MISSING
        }
        AbortReason::PrepReadiness(PrepReadiness::CombatRootWrong) => REASON_PREP_COMBAT_ROOT_WRONG,
        AbortReason::PrepReadiness(PrepReadiness::StyleEchoMissing) => {
            REASON_PREP_STYLE_ECHO_MISSING
        }
        AbortReason::Unresponsive => REASON_UNRESPONSIVE,
        AbortReason::Retreated => REASON_RETREATED,
        AbortReason::RetreatFailed => REASON_RETREAT_FAILED,
        AbortReason::SafespotBroken => REASON_SAFESPOT_BROKEN,
    }
}

fn combat_report(input: &Value, id: u32) -> Option<&Value> {
    let report = input.get("combat")?;
    let got = report.get("id").and_then(Value::as_u64)?;
    (got == u64::from(id)).then_some(report)
}

fn combat_end(report: &Value) -> &str {
    report.get("end").and_then(Value::as_str).unwrap_or("")
}

fn aborted_combat(report: &Value) -> String {
    let reason = report
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("failed");
    format!("combat-{reason}")
}

impl ClueRuntime {
    /// One helper, read by both delegating arms before anything moves.
    fn driver_gate(&mut self, input: &Value) -> Option<Value> {
        if input.get("combat_driver").and_then(Value::as_bool) == Some(true) {
            None
        } else {
            Some(self.abandoned())
        }
    }

    fn alloc_delegation(&mut self) -> u32 {
        self.next_delegation = self.next_delegation.saturating_add(1);
        if self.next_delegation == 0 {
            self.next_delegation = 1;
        }
        self.next_delegation
    }

    fn guardian_id(
        &mut self,
        row: &TrailMembershipRow,
        selected: Option<&SelectedGameData>,
    ) -> Option<i32> {
        let alias = row
            .params
            .iter()
            .find(|param| param.key == "trail_guardian")
            .map(|param| param.value.as_str())?;
        let slot = match alias {
            "trail_hard" => &mut self.hard_guardian,
            "trail_hard2" => &mut self.hard2_guardian,
            _ => return None,
        };
        if *slot == 0 {
            *slot = selected
                .and_then(|data| data.npc_names())
                .and_then(|facts| facts.rows.iter().find(|npc| npc.config == alias))
                .map(|npc| npc.id)
                .unwrap_or(-1);
        }
        (*slot > 0).then_some(*slot)
    }

    fn combat_verb(&self, id: u32, npc_type: i32, stand: Tile, encounter: Encounter) -> Value {
        json!({
            "kind": "combat",
            "token": self.token,
            "id": id,
            "npc_type": npc_type,
            "stand": { "x": stand.x, "z": stand.z, "level": stand.level },
            "encounter": match encounter {
                Encounter::Guardian => "guardian",
                Encounter::Keeper => "keeper",
            },
        })
    }

    /// `Steady` on an identified guarded row: the first Dig, the fight, or the
    /// post-kill redig.
    pub(super) fn guarded(
        &mut self,
        row: &TrailMembershipRow,
        tile: Tile,
        input: &Value,
        selected: Option<&SelectedGameData>,
    ) -> Value {
        if let Some(abandon) = self.driver_gate(input) {
            return abandon;
        }
        if self.guardian.is_none() {
            return self.spawn(tile, input);
        }
        if self.guardian.as_ref().is_some_and(|g| g.post_kill) {
            return self.dig(tile, input);
        }
        self.fight(row, tile, input, selected)
    }

    /// The guarded row's first Dig: the landed walk-then-Dig of the sibling
    /// unguarded arm, and the verb that spawns the wizard.
    pub(super) fn spawn(&mut self, tile: Tile, input: &Value) -> Value {
        match arrival(tile, input) {
            Arrival::Unknown => self.emit("wait"),
            Arrival::Walking => self.walk(tile, None),
            Arrival::Arrived if !spade_posted(input) => self.emit(SUPPLIES_NEEDED),
            Arrival::Arrived => {
                self.guardian = Some(Guardian {
                    delegation: None,
                    post_kill: false,
                    lost_since: None,
                });
                self.dig_verb()
            }
        }
    }

    /// Observe the spawn and hand the fight to the driver, or resume from a
    /// combat report. The machine never emits `npc` Attack or `if-button`.
    pub(super) fn fight(
        &mut self,
        row: &TrailMembershipRow,
        tile: Tile,
        input: &Value,
        selected: Option<&SelectedGameData>,
    ) -> Value {
        if let Some(id) = self.guardian.as_ref().and_then(|g| g.delegation) {
            if let Some(report) = combat_report(input, id) {
                return self.apply_guardian_report(report, tile, input, selected, row);
            }
            return self.emit("wait");
        }
        self.observe_guardian(row, tile, input, selected)
    }

    fn apply_guardian_report(
        &mut self,
        report: &Value,
        tile: Tile,
        input: &Value,
        selected: Option<&SelectedGameData>,
        row: &TrailMembershipRow,
    ) -> Value {
        match combat_end(report) {
            "killed" => {
                self.kill();
                self.dig(tile, input)
            }
            "died" => self.dead(),
            "target_gone" | "no_target" => self.ended(GUARDIAN_LOST),
            "budget" => self.aborted("combat-budget"),
            "aborted" => self.aborted(&aborted_combat(report)),
            "failed" => self.aborted("combat-failed"),
            "cancelled" | "user_input" => {
                if let Some(guardian) = self.guardian.as_mut() {
                    guardian.delegation = None;
                    if guardian.lost_since.is_none() {
                        guardian.lost_since = Some(self.clock.now());
                    }
                }
                self.observe_guardian(row, tile, input, selected)
            }
            _ => self.emit("wait"),
        }
    }

    fn observe_guardian(
        &mut self,
        row: &TrailMembershipRow,
        tile: Tile,
        input: &Value,
        selected: Option<&SelectedGameData>,
    ) -> Value {
        let Some(npc_type) = self.guardian_id(row, selected) else {
            return self.emit("wait");
        };
        let Some(page) = input.get("npcs").and_then(Value::as_array) else {
            return self.guardian_missing();
        };
        if pick_npc(
            Some(npc_type),
            guardian_names(row),
            page,
            posted_i32(input, "self_slot"),
            posted_here(input),
        )
        .is_none()
        {
            return self.guardian_missing();
        }
        let id = self.alloc_delegation();
        if let Some(guardian) = self.guardian.as_mut() {
            guardian.delegation = Some(id);
            guardian.lost_since = None;
        }
        self.combat_verb(id, npc_type, tile, Encounter::Guardian)
    }

    fn guardian_missing(&mut self) -> Value {
        let now = self.clock.now();
        let expired = self
            .guardian
            .as_ref()
            .and_then(|g| g.lost_since)
            .is_some_and(|since| {
                now.saturating_duration_since(since) >= Duration::from_millis(KILL_GRACE_MS)
            });
        if expired {
            self.ended(GUARDIAN_LOST)
        } else {
            self.emit("wait")
        }
    }

    /// The kill was reported: drop the delegation and redig.
    pub(super) fn kill(&mut self) {
        if let Some(guardian) = self.guardian.as_mut() {
            guardian.delegation = None;
            guardian.post_kill = true;
            guardian.lost_since = None;
        }
    }

    /// `Steady` on an identified key-keeper row: walk, delegate the fight,
    /// then Take the key.
    pub(super) fn keys(
        &mut self,
        row: &TrailMembershipRow,
        input: &Value,
        selected: Option<&SelectedGameData>,
    ) -> Value {
        let Some(key) = key_step(selected, row.id) else {
            return self.emit("wait");
        };
        if holds(input, key.key_id) {
            return self.emit("wait");
        }
        let Some(spawn) = key.spawn.as_ref() else {
            return self.emit("wait");
        };
        let Some((keeper_id, keeper_name)) = keeper_type(&key.keeper) else {
            return self.emit("wait");
        };
        if let Some(abandon) = self.driver_gate(input) {
            return abandon;
        }
        let tile = Tile {
            x: spawn.x,
            z: spawn.z,
            level: spawn.plane,
        };
        if let Some(id) = self.keeper.as_ref().and_then(|keeper| keeper.delegation) {
            if let Some(report) = combat_report(input, id) {
                match combat_end(report) {
                    "killed" => {
                        if let Some(keeper) = self.keeper.as_mut() {
                            keeper.delegation = None;
                            keeper.post_kill = true;
                        }
                    }
                    "died" => return self.dead(),
                    "target_gone" | "no_target" | "cancelled" | "user_input" => {
                        if let Some(keeper) = self.keeper.as_mut() {
                            keeper.delegation = None;
                        }
                        return self.emit("wait");
                    }
                    "budget" => return self.aborted("combat-budget"),
                    "aborted" => return self.aborted(&aborted_combat(report)),
                    "failed" => return self.aborted("combat-failed"),
                    _ => return self.emit("wait"),
                }
            } else {
                return self.emit("wait");
            }
        }
        let after_kill = self.keeper.as_ref().is_some_and(|keeper| keeper.post_kill);
        match arrival(tile, input) {
            Arrival::Unknown => self.emit("wait"),
            Arrival::Walking => self.walk(tile, None),
            Arrival::Arrived if after_kill => match pick_key(input, key.key_id, tile) {
                Some(drop) => {
                    let Some(size) = posted_inv_size(input) else {
                        return self.emit("wait");
                    };
                    if occupied(input) >= i64::from(size) {
                        return self.emit("wait");
                    }
                    json!({
                        "kind": "obj",
                        "token": self.token,
                        "x": drop.tile.x,
                        "z": drop.tile.z,
                        "level": drop.tile.level,
                        "name": drop.name,
                        "action": TAKE,
                    })
                }
                None => self.emit("wait"),
            },
            Arrival::Arrived => {
                match input
                    .get("npcs")
                    .and_then(Value::as_array)
                    .and_then(|page| pick_keeper(keeper_id, keeper_name, page, tile))
                {
                    Some(_) => {
                        let id = self.alloc_delegation();
                        self.keeper = Some(Keeper {
                            delegation: Some(id),
                            post_kill: false,
                        });
                        self.combat_verb(id, keeper_id, tile, Encounter::Keeper)
                    }
                    None => self.emit("wait"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prep_readiness_reasons_remain_distinct_in_clue_reports() {
        for (readiness, expected) in [
            (
                PrepReadiness::CombatRootMissing,
                "prep-readiness:combat-root-missing",
            ),
            (
                PrepReadiness::CombatRootWrong,
                "prep-readiness:combat-root-wrong",
            ),
            (
                PrepReadiness::StyleEchoMissing,
                "prep-readiness:style-echo-missing",
            ),
        ] {
            let outcome = Outcome {
                id: 1,
                engaged_type: 81,
                ticks: 8,
                end: OutcomeEnd::Aborted,
                reason: abort_reason(AbortReason::PrepReadiness(readiness)),
            };
            assert_eq!(outcome.page()["reason"].as_str(), Some(expected));
        }
    }
}
