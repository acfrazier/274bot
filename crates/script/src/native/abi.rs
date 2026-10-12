//! Current machine-hook ABI characterization, independent of the `load` feature.
//!
//! This is a path × value-use description of the native machines as they run today, not an
//! activation plan or a seat scheduler. §28 keeps the §25 axes and the V5-07 value corrections,
//! but replaces the seat transition rules. The only future-path notes below are unapproved
//! possibilities recorded for existing discrepancies; they do not select or activate a path.
//!
//! The callback paths have different Promise and throw behavior:
//!
//! - `Awaited`: `Step::Call` stores a returned Promise and polls it before the row steps again.
//!   The callback is not reinvoked while that same Promise is pending. A throw/rejection reaches
//!   the family as `Reply::Threw`; these member handlers fail the run with that same thrown value.
//! - `CallSync`: a returned Promise is an opaque empty object, whether settled or pending. The
//!   family receives a thrown callback as `Reply::Threw` and fails the affected run with it.
//! - `Ask`: `Cx::ask` never waits; a returned Promise fails the row with the current
//!   `not impl: a synchronous machine callback returned a promise` error. A thrown value fails
//!   the row with that same value.
//!
//! Current order is row-local: poll an existing pending callback, then step; synchronous asks
//! execute in place; a Hunt effect's queued notes are called once each in note order before the
//! effect is applied; the step's ops are placed at its queue position; then a `Step::Call` runs
//! and a tick-step checkpoint can settle its Promise. A kick has no checkpoint and polls on the
//! next drive. Current sources: `machine.rs:1291-1294,1311-1368,1345`; `load/machine_v8.rs:257-307`;
//! `hunt.rs:584-585,794-807`. This does not assert seat-side `last_seq`, lineage, Decide/carry, or
//! child-before-parent scheduling: those are not active in this in-isolate machine host.
//!
//! The `families` arrays below name concrete registered `Family::NAME` values that expose each
//! callback key. In families which share one hook list (notably Hunt), an entry describes the
//! real member surface and path category; it does not claim that every run of every listed family
//! calls that hook.

/// Current callback invocation path, independent of value consumption.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HookPath {
    /// `Step::Call` stores a returned Promise and polls it before the row steps again.
    Awaited,
    /// `Step::Call` is listed in `Family::SYNC_HOOKS`; a Promise becomes `{}`.
    CallSync,
    /// `Cx::ask` invokes inline; a returned Promise fails the row.
    Ask,
}

/// Whether the family uses a fulfilled callback value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HookValue {
    /// The family reads or branches on the returned value.
    Used,
    /// The family discards a fulfilled value, but still handles a throw.
    Ignored,
}

/// Observable treatment of a Promise returned from a callback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromiseBehavior {
    /// One Promise is retained and polled; a pending row does not call the hook again.
    PolledSamePromise,
    /// `Called::Pending` is represented to this synchronous family as an empty object.
    OpaqueEmptyObject,
    /// The ask ends the row with the synchronous-Promise `not impl` failure.
    NotImplementedFailure,
}

/// Observable propagation of the callback's thrown value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThrowBehavior {
    /// The `Step::Call` family receives `Reply::Threw` and fails with the original thrown value.
    FamilyFailsWithThrownValue,
    /// `Cx::ask` fails the row with the original thrown value.
    AskFailsWithThrownValue,
}

/// Current versus frozen call form; proposals do not select a future path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrozenPathStatus {
    /// The current native call form matches the frozen call form for this member.
    MatchesFrozenCallForm,
    /// An existing difference; a candidate alignment is recorded but not approved here.
    ExistingDiscrepancy {
        q5_row: u8,
        proposed_path: HookPath,
        proposal_approved: bool,
    },
}

/// One callback member and its current registered family expansion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HookCharacterization {
    families: &'static [&'static str],
    member: &'static str,
    call_site: &'static str,
    path: HookPath,
    value: HookValue,
    promise: PromiseBehavior,
    thrown: ThrowBehavior,
    frozen_path: FrozenPathStatus,
    evidence: &'static str,
}
impl HookCharacterization {
    /// Concrete current registered families exposing this callback.
    pub const fn families(&self) -> &'static [&'static str] {
        self.families
    }
    /// Callback key.
    pub const fn member(&self) -> &'static str {
        self.member
    }
    /// Separate call sites can have different axes for one family/member.
    pub const fn call_site(&self) -> &'static str {
        self.call_site
    }
    /// Invocation axis.
    pub const fn path(&self) -> HookPath {
        self.path
    }
    /// Fulfilled-value axis.
    pub const fn value(&self) -> HookValue {
        self.value
    }
    /// Current Promise coercion or waiting behavior.
    pub const fn promise(&self) -> PromiseBehavior {
        self.promise
    }
    /// Current thrown-value propagation.
    pub const fn thrown(&self) -> ThrowBehavior {
        self.thrown
    }
    /// An existing discrepancy is not a new approval.
    pub const fn frozen_path(&self) -> FrozenPathStatus {
        self.frozen_path
    }
    /// Current and frozen source locations, using §25's $S/$TS aliases.
    pub const fn evidence(&self) -> &'static str {
        self.evidence
    }
}

const HUNT_FAMILIES: &[&str] = &[
    "hunt-fight",
    "hunt-hold",
    "hunt-retreat",
    "hunt-walkspot",
    "hunt-enter",
    "hunt-leave",
    "hunt-key",
    "hunt-cell",
    "hunt-bank",
    "hunt-wait-fed",
    "hunt-teleport-out",
    "hunt-acquire",
];

const SUSTAIN_FAMILIES: &[&str] = &[
    "hunt-fight",
    "hunt-hold",
    "hunt-retreat",
    "hunt-walkspot",
    "hunt-enter",
    "hunt-leave",
    "hunt-key",
    "hunt-cell",
    "hunt-bank",
    "hunt-wait-fed",
    "hunt-teleport-out",
    "hunt-acquire",
    "walk-resilient",
    "reach-npc-dialog",
    "walk-to",
    "return-to-anchor",
    "walk-hops",
];

const SYNC_LOG_FAMILIES: &[&str] = &[
    "periodic_bank",
    "banking_open",
    "bank_access",
    "bank_npc_access",
    "dialog",
    "reach-npc-dialog",
    "reach-entity-op",
    "walk-resilient",
    "walk-opening",
    "walk-to",
    "return-to-anchor",
    "walk-hops",
];

const fn hook(
    families: &'static [&'static str],
    member: &'static str,
    call_site: &'static str,
    path: HookPath,
    value: HookValue,
    frozen_path: FrozenPathStatus,
    evidence: &'static str,
) -> HookCharacterization {
    let (promise, thrown) = match path {
        HookPath::Awaited => (
            PromiseBehavior::PolledSamePromise,
            ThrowBehavior::FamilyFailsWithThrownValue,
        ),
        HookPath::CallSync => (
            PromiseBehavior::OpaqueEmptyObject,
            ThrowBehavior::FamilyFailsWithThrownValue,
        ),
        HookPath::Ask => (
            PromiseBehavior::NotImplementedFailure,
            ThrowBehavior::AskFailsWithThrownValue,
        ),
    };
    HookCharacterization {
        families,
        member,
        call_site,
        path,
        value,
        promise,
        thrown,
        frozen_path,
        evidence,
    }
}

const NO_DISCREPANCY: FrozenPathStatus = FrozenPathStatus::MatchesFrozenCallForm;
const CLUE_PATH_NOT_APPROVED: FrozenPathStatus = FrozenPathStatus::ExistingDiscrepancy {
    q5_row: 14,
    proposed_path: HookPath::CallSync,
    proposal_approved: false,
};
const HUNT_NOTIFICATION_PATH_NOT_APPROVED: FrozenPathStatus =
    FrozenPathStatus::ExistingDiscrepancy {
        q5_row: 21,
        proposed_path: HookPath::CallSync,
        proposal_approved: false,
    };

/// Every member named in §25 R5-6.2, expanded to its concrete current family names.
pub const HOOK_CHARACTERIZATIONS: &[HookCharacterization] = &[
    // Awaited / used.
    hook(
        HUNT_FAMILIES,
        "eatOnce",
        "Hunt eat decision; truthy fulfillment sets eatOk",
        HookPath::Awaited,
        HookValue::Used,
        NO_DISCREPANCY,
        "$TS/api/combat/hunting/combat.ts:314; $S/hunt.rs:622-624",
    ),
    hook(
        HUNT_FAMILIES,
        "leave",
        "Hunt leave effect; fulfillment is compared with true",
        HookPath::Awaited,
        HookValue::Used,
        NO_DISCREPANCY,
        "$TS/api/supply.ts:542; $S/hunt.rs:699-703",
    ),
    hook(
        &["reach-entity-op"],
        "interact",
        "Reach entity operation; truthy fulfillment means dispatched",
        HookPath::Awaited,
        HookValue::Used,
        NO_DISCREPANCY,
        "$TS/api/reach/Reach.ts:224; $S/reach_entity.rs:596-601",
    ),
    // Awaited / ignored. §28.1 applies V5-07: armSpecial, sustain and afterDeposit are ignored.
    hook(
        HUNT_FAMILIES,
        "armSpecial",
        "Hunt arm-special effect",
        HookPath::Awaited,
        HookValue::Ignored,
        NO_DISCREPANCY,
        "$TS/api/combat/hunting/combat.ts:402; $S/hunt.rs:624-630",
    ),
    hook(
        SUSTAIN_FAMILIES,
        "sustain",
        "Hunt and walk sustain pump",
        HookPath::Awaited,
        HookValue::Ignored,
        NO_DISCREPANCY,
        "$TS/api/combat/hunting/combat.ts:635,675; $S/hunt.rs:627-629; $S/walk.rs:1592-1619; $S/boat_fare.rs:1441-1448; $S/anchor_return.rs:175-180; $S/reach.rs:287-293; $S/reach_entity.rs:1036-1042",
    ),
    hook(
        &["bank_nearest", "periodic_bank"],
        "afterDeposit",
        "BankNearest and PeriodicBank post-deposit continuation",
        HookPath::Awaited,
        HookValue::Ignored,
        NO_DISCREPANCY,
        "$TS/api/bank/bankRules.ts:103-105; $S/periodic_bank.rs:268-282,384-410,474-493",
    ),
    hook(
        &["death_recovery"],
        "walkBack",
        "DeathRecovery walk-back callback before validation",
        HookPath::Awaited,
        HookValue::Ignored,
        NO_DISCREPANCY,
        "$S/death_recovery.rs:200-205,244-256 (frozen result ignored at :255)",
    ),
    // Call-sync / used.
    hook(
        &["bank_deposit"],
        "match",
        "Bank deposit matcher; `{}` is truthy, so the row is deposited",
        HookPath::CallSync,
        HookValue::Used,
        NO_DISCREPANCY,
        "$TS/api/bank/bankRules.ts:103-105; $S/bank_deposit.rs:458-462",
    ),
    hook(
        &["bank_nearest", "periodic_bank"],
        "deposit",
        "Bank matcher; `{}` is truthy, so the row is deposited",
        HookPath::CallSync,
        HookValue::Used,
        NO_DISCREPANCY,
        "$TS/api/bank/bankRules.ts:103-105; $S/periodic_bank.rs:385-410,474-493",
    ),
    hook(
        &["bank_withdraw_to", "bank_close_confirm"],
        "count",
        "Withdraw count; `{}` becomes NaN when used as a number",
        HookPath::CallSync,
        HookValue::Used,
        NO_DISCREPANCY,
        "$TS/api/bank/stealRules.ts:37-43,69-75; $S/bank_withdraw.rs:190-198,353-357",
    ),
    hook(
        &["shop"],
        "pick",
        "Shop matches.find(pick); `{}` is truthy, so the first row is taken",
        HookPath::CallSync,
        HookValue::Used,
        NO_DISCREPANCY,
        "$TS/api/shop/Shop.ts:124; $S/shop.rs:217-221; crates/script/tests/shop.rs:468",
    ),
    hook(
        &["trade"],
        "pick",
        "Trade matches.find(pick); `{}` is truthy, so the first row is taken",
        HookPath::CallSync,
        HookValue::Used,
        NO_DISCREPANCY,
        "$TS/api/trade/Trade.ts:99,114; $S/trade.rs:407-411; crates/script/tests/trade.rs:777",
    ),
    hook(
        &["periodic_bank"],
        "destination",
        "PeriodicBank destination option; `{}` has no x and parses as absent",
        HookPath::CallSync,
        HookValue::Used,
        NO_DISCREPANCY,
        "$TS/api/tasks/PeriodicBank.ts:49-51; $S/periodic_bank.rs:486-493,518-524; $S/banking_open.rs:52-70",
    ),
    hook(
        &["periodic_bank"],
        "commonJunk",
        "PeriodicBank common-junk option; `{}` is truthy",
        HookPath::CallSync,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/periodic_bank.rs:486-493,525-532",
    ),
    hook(
        &["periodic_bank"],
        "returnTo",
        "PeriodicBank return tile option; `{}` has no x and parses as absent",
        HookPath::CallSync,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/periodic_bank.rs:486-493,533-537",
    ),
    hook(
        &["reach-entity-op"],
        "expect",
        "Reach entity expected-state predicate; `{}` is truthy",
        HookPath::CallSync,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/reach_entity.rs:478-483",
    ),
    hook(
        &["reach-entity-op"],
        "target",
        "Reach entity target closure; `{}` is not a tile",
        HookPath::CallSync,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/reach_entity.rs:478-483",
    ),
    hook(
        &["partner-trade"],
        "productNamesToOffer",
        "Partner trade offer-name projection reads `{}`",
        HookPath::CallSync,
        HookValue::Used,
        NO_DISCREPANCY,
        "$TS/api/trade/drivePartnerTrade.ts; $S/drive_partner_trade.rs:214-240",
    ),
    hook(
        &["partner-trade"],
        "inventoryMetric",
        "Partner trade inventory metric reads `{}`",
        HookPath::CallSync,
        HookValue::Used,
        NO_DISCREPANCY,
        "$TS/api/trade/drivePartnerTrade.ts; $S/drive_partner_trade.rs:214-240",
    ),
    hook(
        &["partner-trade"],
        "myOfferReady",
        "Partner trade own-offer readiness predicate reads `{}`",
        HookPath::CallSync,
        HookValue::Used,
        NO_DISCREPANCY,
        "$TS/api/trade/drivePartnerTrade.ts; $S/drive_partner_trade.rs:214-240",
    ),
    hook(
        &["partner-trade"],
        "theirProductMatch",
        "Partner trade incoming-product predicate reads `{}`",
        HookPath::CallSync,
        HookValue::Used,
        NO_DISCREPANCY,
        "$TS/api/trade/drivePartnerTrade.ts; $S/drive_partner_trade.rs:214-240",
    ),
    hook(
        &["partner-trade"],
        "receiverCanAccept",
        "Partner trade receiver-capacity predicate reads `{}`",
        HookPath::CallSync,
        HookValue::Used,
        NO_DISCREPANCY,
        "$TS/api/trade/drivePartnerTrade.ts; $S/drive_partner_trade.rs:214-240",
    ),
    hook(
        &["partner-trade"],
        "baseline",
        "Partner trade baseline projection reads `{}`",
        HookPath::CallSync,
        HookValue::Used,
        NO_DISCREPANCY,
        "$TS/api/trade/drivePartnerTrade.ts; $S/drive_partner_trade.rs:214-240",
    ),
    hook(
        &["cake_stall"],
        "abort",
        "CakeStall abort predicate; `{}` is truthy",
        HookPath::CallSync,
        HookValue::Used,
        NO_DISCREPANCY,
        "$TS/api/thieving/CakeStall.ts:69,146; $S/cake_stall/runtime.rs:727-749",
    ),
    hook(
        &["cake_stall"],
        "shouldEat",
        "CakeStall eat predicate; `{}` is truthy",
        HookPath::CallSync,
        HookValue::Used,
        NO_DISCREPANCY,
        "$TS/api/thieving/CakeStall.ts:69,146; $S/cake_stall/runtime.rs:727-749",
    ),
    hook(
        &["cake_stall"],
        "lockedOutUntil",
        "CakeStall lockout deadline; `{}` is truthy",
        HookPath::CallSync,
        HookValue::Used,
        NO_DISCREPANCY,
        "$TS/api/thieving/CakeStall.ts:69,146; $S/cake_stall/runtime.rs:727-749",
    ),
    // Call-sync / ignored.
    hook(
        &["periodic_bank"],
        "setStatus",
        "PeriodicBank status callback",
        HookPath::CallSync,
        HookValue::Ignored,
        NO_DISCREPANCY,
        "$S/periodic_bank.rs:474-493,512-517",
    ),
    hook(
        SYNC_LOG_FAMILIES,
        "log",
        "Synchronous family log notifications",
        HookPath::CallSync,
        HookValue::Ignored,
        NO_DISCREPANCY,
        "$S/periodic_bank.rs:385-388,474-493; $S/banking_open.rs:352-353; $S/bank_access.rs:713-714,874-875; $S/dialog.rs:166-170; $S/reach.rs:202-205; $S/reach_entity.rs:479-483,994-996; $S/walk.rs:1533-1536,1810-1812; $S/boat_fare.rs:1352-1357; $S/anchor_return.rs:101-104",
    ),
    hook(
        &["bank_nearest"],
        "log",
        "BankNearest synchronous log callback",
        HookPath::CallSync,
        HookValue::Ignored,
        NO_DISCREPANCY,
        "$S/periodic_bank.rs:380-410",
    ),
    hook(
        &["partner-trade"],
        "setStatus",
        "Partner trade ordered notification queue",
        HookPath::CallSync,
        HookValue::Ignored,
        NO_DISCREPANCY,
        "$S/drive_partner_trade.rs:201-203,214-240",
    ),
    hook(
        &["partner-trade"],
        "log",
        "Partner trade ordered notification queue",
        HookPath::CallSync,
        HookValue::Ignored,
        NO_DISCREPANCY,
        "$S/drive_partner_trade.rs:201-203,214-240",
    ),
    hook(
        &["partner-trade"],
        "onMissingPartner",
        "Partner trade missing-partner notification",
        HookPath::CallSync,
        HookValue::Ignored,
        NO_DISCREPANCY,
        "$S/drive_partner_trade.rs:201-203,214-240",
    ),
    hook(
        &["partner-trade"],
        "onComplete",
        "Partner trade completion notification",
        HookPath::CallSync,
        HookValue::Ignored,
        NO_DISCREPANCY,
        "$S/drive_partner_trade.rs:201-203,214-240",
    ),
    hook(
        &["partner-trade"],
        "onDecline",
        "Partner trade decline notification",
        HookPath::CallSync,
        HookValue::Ignored,
        NO_DISCREPANCY,
        "$S/drive_partner_trade.rs:201-203,214-240",
    ),
    hook(
        &["cake_stall"],
        "setStatus",
        "CakeStall status notification",
        HookPath::CallSync,
        HookValue::Ignored,
        NO_DISCREPANCY,
        "$S/cake_stall/runtime.rs:727-749",
    ),
    hook(
        &["cake_stall"],
        "log",
        "CakeStall log notification",
        HookPath::CallSync,
        HookValue::Ignored,
        NO_DISCREPANCY,
        "$S/cake_stall/runtime.rs:727-749",
    ),
    hook(
        &["cake_stall"],
        "onSteal",
        "CakeStall steal notification",
        HookPath::CallSync,
        HookValue::Ignored,
        NO_DISCREPANCY,
        "$S/cake_stall/runtime.rs:727-749",
    ),
    hook(
        &["cake_stall"],
        "onReset",
        "CakeStall reset notification",
        HookPath::CallSync,
        HookValue::Ignored,
        NO_DISCREPANCY,
        "$S/cake_stall/runtime.rs:727-749",
    ),
    // Ask / used. HOOKS is shared by every named Hunt family; invocation varies by stepper.
    hook(
        HUNT_FAMILIES,
        "died",
        "Hunt state getter",
        HookPath::Ask,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/hunt.rs:50-88,152-222; $S/hunt_fight.rs:2229-2243",
    ),
    hook(
        HUNT_FAMILIES,
        "targetIdx",
        "Hunt state getter",
        HookPath::Ask,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/hunt.rs:50-88,203-212; $S/hunt_fight.rs:2229-2243",
    ),
    hook(
        HUNT_FAMILIES,
        "hpFraction",
        "Hunt state getter",
        HookPath::Ask,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/hunt.rs:50-88,184-190; $S/hunt_fight.rs:2229-2243",
    ),
    hook(
        HUNT_FAMILIES,
        "panicHp",
        "Hunt state getter",
        HookPath::Ask,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/hunt.rs:50-88,184-190; $S/hunt_fight.rs:2229-2243",
    ),
    hook(
        HUNT_FAMILIES,
        "retreatHp",
        "Hunt state getter",
        HookPath::Ask,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/hunt.rs:50-88,184-190; $S/hunt_fight.rs:2229-2243",
    ),
    hook(
        HUNT_FAMILIES,
        "hasFood",
        "Hunt state getter",
        HookPath::Ask,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/hunt.rs:50-88,168-181; $S/hunt_fight.rs:2229-2243",
    ),
    hook(
        HUNT_FAMILIES,
        "needEat",
        "Hunt state getter",
        HookPath::Ask,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/hunt.rs:50-88,168-181; $S/hunt_fight.rs:2229-2243",
    ),
    hook(
        HUNT_FAMILIES,
        "style",
        "Hunt state getter",
        HookPath::Ask,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/hunt.rs:50-88,192-200; $S/hunt_fight.rs:2229-2243",
    ),
    hook(
        HUNT_FAMILIES,
        "safespotIndex",
        "Hunt state getter",
        HookPath::Ask,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/hunt.rs:50-88,203-212; $S/hunt_fight.rs:2229-2243",
    ),
    hook(
        HUNT_FAMILIES,
        "buryBones",
        "Hunt state getter",
        HookPath::Ask,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/hunt.rs:50-88,168-181; $S/hunt_fight.rs:2229-2243",
    ),
    hook(
        HUNT_FAMILIES,
        "boneName",
        "Hunt state getter",
        HookPath::Ask,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/hunt.rs:50-88,192-200; $S/hunt_fight.rs:2229-2243",
    ),
    hook(
        HUNT_FAMILIES,
        "shieldReady",
        "Hunt state getter",
        HookPath::Ask,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/hunt.rs:50-88,176-182; $S/hunt_fight.rs:2229-2243",
    ),
    hook(
        HUNT_FAMILIES,
        "parked",
        "Hunt state getter",
        HookPath::Ask,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/hunt.rs:50-88,152-222",
    ),
    hook(
        HUNT_FAMILIES,
        "leaveByWalk",
        "Hunt state getter",
        HookPath::Ask,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/hunt.rs:50-88,152-222; $S/hunt_leave.rs:972-977",
    ),
    hook(
        HUNT_FAMILIES,
        "foodName",
        "Hunt state getter",
        HookPath::Ask,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/hunt.rs:50-88,192-200; $S/hunt_bank.rs:2163-2167",
    ),
    hook(
        HUNT_FAMILIES,
        "foodWithdraw",
        "Hunt state getter",
        HookPath::Ask,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/hunt.rs:50-88,184-190; $S/hunt_bank.rs:2163-2167",
    ),
    hook(
        HUNT_FAMILIES,
        "weaponName",
        "Hunt state getter",
        HookPath::Ask,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/hunt.rs:50-88,192-200; $S/hunt_bank.rs:2163-2167",
    ),
    hook(
        HUNT_FAMILIES,
        "ammoName",
        "Hunt state getter",
        HookPath::Ask,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/hunt.rs:50-88,192-200",
    ),
    hook(
        HUNT_FAMILIES,
        "spellName",
        "Hunt state getter",
        HookPath::Ask,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/hunt.rs:50-88,192-200",
    ),
    hook(
        HUNT_FAMILIES,
        "keepExtra",
        "Hunt state getter",
        HookPath::Ask,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/hunt.rs:50-88,152-222",
    ),
    hook(
        HUNT_FAMILIES,
        "inArea",
        "Hunt site-area predicate",
        HookPath::Ask,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/hunt.rs:323-356; $S/hunt/acquire.rs:107; $S/hunt/teleport_out.rs:137; $S/hunt_fight.rs:2229-2243",
    ),
    hook(
        HUNT_FAMILIES,
        "cond",
        "Hunt wait-fed condition",
        HookPath::Ask,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/hunt/wait.rs:40-44",
    ),
    // Ask / ignored. These are the current Q5 row-21 discrepancy, not an approved path change.
    hook(
        HUNT_FAMILIES,
        "countKill",
        "Hunt count-kill progress notification",
        HookPath::Ask,
        HookValue::Ignored,
        HUNT_NOTIFICATION_PATH_NOT_APPROVED,
        "$TS/api/combat/hunting/combat.ts:521-522; $S/hunt.rs:783-807",
    ),
    hook(
        HUNT_FAMILIES,
        "countBurial",
        "Hunt burial progress notification",
        HookPath::Ask,
        HookValue::Ignored,
        HUNT_NOTIFICATION_PATH_NOT_APPROVED,
        "$S/hunt.rs:50-88,612-617,783-807",
    ),
    hook(
        HUNT_FAMILIES,
        "countBankTrip",
        "Hunt bank-trip progress notification",
        HookPath::Ask,
        HookValue::Ignored,
        HUNT_NOTIFICATION_PATH_NOT_APPROVED,
        "$S/hunt.rs:50-88,612-617,783-807",
    ),
    hook(
        HUNT_FAMILIES,
        "setSafespotIndex",
        "Hunt safespot progress notification",
        HookPath::Ask,
        HookValue::Ignored,
        HUNT_NOTIFICATION_PATH_NOT_APPROVED,
        "$S/hunt.rs:50-88,612-617,783-807",
    ),
    hook(
        HUNT_FAMILIES,
        "setTarget",
        "Hunt target progress notification",
        HookPath::Ask,
        HookValue::Ignored,
        HUNT_NOTIFICATION_PATH_NOT_APPROVED,
        "$S/hunt.rs:50-88,612-617,783-807",
    ),
    hook(
        HUNT_FAMILIES,
        "parkFor",
        "Hunt park progress notification",
        HookPath::Ask,
        HookValue::Ignored,
        HUNT_NOTIFICATION_PATH_NOT_APPROVED,
        "$S/hunt.rs:50-88,612-617,783-807",
    ),
    hook(
        HUNT_FAMILIES,
        "pickWeapon",
        "Hunt weapon-pick progress notification",
        HookPath::Ask,
        HookValue::Ignored,
        HUNT_NOTIFICATION_PATH_NOT_APPROVED,
        "$S/hunt.rs:50-88,618-621,783-807",
    ),
    hook(
        HUNT_FAMILIES,
        "setStatus",
        "Hunt status progress notification",
        HookPath::Ask,
        HookValue::Ignored,
        HUNT_NOTIFICATION_PATH_NOT_APPROVED,
        "$S/hunt.rs:50-88,612-617,783-807",
    ),
    hook(
        HUNT_FAMILIES,
        "vlog",
        "Hunt verbose-log progress notification",
        HookPath::Ask,
        HookValue::Ignored,
        HUNT_NOTIFICATION_PATH_NOT_APPROVED,
        "$S/hunt.rs:50-88,794-807",
    ),
    hook(
        HUNT_FAMILIES,
        "log",
        "Hunt log progress notification",
        HookPath::Ask,
        HookValue::Ignored,
        HUNT_NOTIFICATION_PATH_NOT_APPROVED,
        "$S/hunt.rs:50-88,794-807",
    ),
    hook(
        &["clue_duel_travel"],
        "log",
        "Duel travel log forwarding",
        HookPath::Ask,
        HookValue::Ignored,
        NO_DISCREPANCY,
        "$S/duel.rs:174-178",
    ),
    hook(
        &["clue"],
        "log",
        "Clue nested duel-travel log forwarding",
        HookPath::Ask,
        HookValue::Ignored,
        CLUE_PATH_NOT_APPROVED,
        "$S/clue/family.rs:26-33; $TS/api/ai/clues/SolveClue.ts:121,193,304",
    ),
    // Clue's ordinary callbacks use Step::Call and are a separate Q5 row-14 discrepancy.
    hook(
        &["clue"],
        "enabled",
        "Clue enabled decision callback",
        HookPath::Awaited,
        HookValue::Used,
        CLUE_PATH_NOT_APPROVED,
        "$S/clue/family.rs:43-49,100-113; $TS/api/ai/clues/SolveClue.ts:121,193,304",
    ),
    hook(
        &["clue"],
        "log",
        "Clue ordinary log callback (distinct from nested duel-travel forwarding)",
        HookPath::Awaited,
        HookValue::Ignored,
        CLUE_PATH_NOT_APPROVED,
        "$S/clue/family.rs:43-49,73-85,114-123; $TS/api/ai/clues/SolveClue.ts:121,193,304",
    ),
    hook(
        &["clue"],
        "setStatus",
        "Clue status callback",
        HookPath::Awaited,
        HookValue::Ignored,
        CLUE_PATH_NOT_APPROVED,
        "$S/clue/family.rs:43-49,73-85,124-133; $TS/api/ai/clues/SolveClue.ts:121,193,304",
    ),
];
