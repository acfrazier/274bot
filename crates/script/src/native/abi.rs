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
//! execute in place. Hunt notes run in event order before the decision, while standalone
//! `log`/`vlog`/`status` effect hooks run after notes and before the effect is applied; the step's
//! ops are placed at their queue position, then a `Step::Call` runs and a tick-step checkpoint
//! can settle its Promise. A kick has no checkpoint and polls on the next drive. Current sources:
//! `machine.rs:1293-1296,1313-1318,1357-1370`; `load/machine_v8.rs:257-307`;
//! `hunt.rs:583-584,610-612,798-821`. This does not assert seat-side `last_seq`, lineage,
//! Decide/carry, or child-before-parent scheduling: those are not active in this in-isolate machine host.
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

/// Ordering of one callback relative to the row pass and sibling notifications.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HookOrder {
    /// Poll an existing awaited Promise before stepping the row again.
    AwaitedPollBeforeStep,
    /// Invoke at the synchronous call site in the current row step.
    SynchronousAtCallSite,
    /// Deliver queued Hunt notes in event order before the decision.
    HuntNotificationsBeforeDecision,
    /// Deliver queued notifications before the next decision.
    QueuedNotificationsBeforeDecision,
    /// Deliver queued Hunt notes before the decision and standalone hooks after their notes.
    HuntNotificationsAndEffects,
    /// Deliver an effect hook after its notes and before applying the effect.
    HuntEffectAfterNotes,
}

/// Multiplicity of one callback within a row's current execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HookMultiplicity {
    /// One invocation each time the machine reaches this call site.
    PerReachedCallSite,
    /// One invocation per queued Hunt note; notes are not coalesced.
    PerQueuedHuntEvent,
    /// One invocation per queued notification.
    PerQueuedNotification,
    /// One invocation per queued Hunt note or standalone log/status effect.
    PerHuntNotificationOrEffect,
    /// One invocation per occurrence of its Hunt effect.
    PerHuntEffect,
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

    /// Callback order in the current row pass.
    pub fn order(&self, family: &str) -> HookOrder {
        if self.path == HookPath::Awaited {
            HookOrder::AwaitedPollBeforeStep
        } else if self.is_queued_notification(family) {
            HookOrder::QueuedNotificationsBeforeDecision
        } else if self.is_ordered_hunt_notification(family) {
            HookOrder::HuntNotificationsBeforeDecision
        } else if self.is_hunt_notification_and_effect(family) {
            HookOrder::HuntNotificationsAndEffects
        } else if self.is_hunt_effect(family) {
            HookOrder::HuntEffectAfterNotes
        } else {
            HookOrder::SynchronousAtCallSite
        }
    }
    /// Callback multiplicity in the current row pass.
    pub fn multiplicity(&self, family: &str) -> HookMultiplicity {
        if self.is_queued_notification(family) {
            HookMultiplicity::PerQueuedNotification
        } else if self.is_ordered_hunt_notification(family) {
            HookMultiplicity::PerQueuedHuntEvent
        } else if self.is_hunt_notification_and_effect(family) {
            HookMultiplicity::PerHuntNotificationOrEffect
        } else if self.is_hunt_effect(family) {
            HookMultiplicity::PerHuntEffect
        } else {
            HookMultiplicity::PerReachedCallSite
        }
    }

    /// Source-backed frequency statement for this member's current call sites.
    pub fn cadence_note(&self, family: &str) -> &'static str {
        if self.is_hunt_notification_and_effect(family) {
            "Queued Hunt notes are delivered in sequence before the decision; standalone log, vlog and status effects run after notes and before the effect is applied."
        } else if self.is_ordered_hunt_notification(family) {
            "Once per queued Hunt note; notes are delivered in sequence before the decision."
        } else if self.is_queued_notification(family) {
            "Once per queued notification; delivery precedes the next decision."
        } else if self.is_hunt_effect(family) {
            "Once per occurrence of this Hunt effect, after its notes and before applying it."
        } else if self.member == "sustain" && family == "hunt-fight" {
            "The Hunt fight stepper emits sustain at least once per tick."
        } else if self.member == "sustain" && family.starts_with("hunt-") {
            "Per reached Hunt sustain call site; no per-tick minimum is asserted."
        } else if self.member == "sustain" {
            "Non-Hunt movement drivers invoke sustain at reached pump call sites; no Hunt tick cadence applies."
        } else if self.path == HookPath::Awaited {
            "One invocation per reached call site; the same Promise is polled while pending."
        } else {
            "One invocation per reached synchronous call site; sites may be reached repeatedly."
        }
    }
    fn is_queued_notification(&self, family: &str) -> bool {
        self.path == HookPath::CallSync
            && ((family == "partner-trade"
                && matches!(
                    self.member,
                    "setStatus" | "log" | "onComplete" | "onDecline"
                ))
                || (self.member == "log"
                    && matches!(
                        family,
                        "dialog"
                            | "reach-npc-dialog"
                            | "reach-entity-op"
                            | "walk-resilient"
                            | "walk-opening"
                            | "walk-to"
                            | "return-to-anchor"
                            | "walk-hops"
                    )))
    }
    fn is_ordered_hunt_notification(&self, family: &str) -> bool {
        self.path == HookPath::Ask
            && family.starts_with("hunt-")
            && matches!(self.member, "countKill")
    }
    fn is_hunt_notification_and_effect(&self, family: &str) -> bool {
        self.path == HookPath::Ask
            && family.starts_with("hunt-")
            && matches!(self.member, "setStatus" | "vlog" | "log")
    }
    fn is_hunt_effect(&self, family: &str) -> bool {
        self.path == HookPath::Ask
            && family.starts_with("hunt-")
            && matches!(
                self.member,
                "countBurial"
                    | "countBankTrip"
                    | "setSafespotIndex"
                    | "setTarget"
                    | "parkFor"
                    | "pickWeapon"
            )
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
    "bank_open_access",
    "bank_deposit",
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
        "$TS/api/combat/hunting/combat.ts:325; $S/hunt.rs:623",
    ),
    hook(
        HUNT_FAMILIES,
        "leave",
        "Hunt leave effect; fulfillment is compared with true",
        HookPath::Awaited,
        HookValue::Used,
        NO_DISCREPANCY,
        "$TS/api/combat/hunting/supply.ts:549; $S/hunt.rs:700-702",
    ),
    hook(
        &["reach-entity-op"],
        "interact",
        "Reach entity operation; truthy fulfillment means dispatched",
        HookPath::Awaited,
        HookValue::Used,
        NO_DISCREPANCY,
        "$TS/api/walking/Reach.ts:224; $S/reach_entity.rs:603-607",
    ),
    // Awaited / ignored. §28.1 applies V5-07: armSpecial, sustain and afterDeposit are ignored.
    hook(
        HUNT_FAMILIES,
        "armSpecial",
        "Hunt arm-special effect",
        HookPath::Awaited,
        HookValue::Ignored,
        NO_DISCREPANCY,
        "$TS/api/combat/hunting/combat.ts:425,677; $S/hunt.rs:624-629",
    ),
    hook(
        SUSTAIN_FAMILIES,
        "sustain",
        "Hunt and walk sustain pump",
        HookPath::Awaited,
        HookValue::Ignored,
        NO_DISCREPANCY,
        "$TS/api/combat/hunting/combat.ts:717; $S/hunt.rs:627-629; $S/walk.rs:1612-1619; $S/boat_fare.rs:1441-1448; $S/anchor_return.rs:175-180; $S/reach.rs:287-293; $S/reach_entity.rs:1043-1048",
    ),
    hook(
        &["bank_nearest", "periodic_bank"],
        "afterDeposit",
        "BankNearest and PeriodicBank post-deposit continuation",
        HookPath::Awaited,
        HookValue::Ignored,
        NO_DISCREPANCY,
        "$TS/api/bank/Banking.ts:298,313; $S/periodic_bank.rs:268-282,384-410,474-493",
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
        "$TS/api/bank/Bank.ts:331; $TS/api/bank/bankRules.ts:103-105; $S/bank_deposit.rs:458-462",
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
        "$TS/api/thieving/stealRules.ts:37-43; $S/bank_withdraw.rs:190-198,353-357",
    ),
    hook(
        &["shop"],
        "pick",
        "Shop matches.find(pick); `{}` is truthy, so the first row is taken",
        HookPath::CallSync,
        HookValue::Used,
        NO_DISCREPANCY,
        "$TS/api/shop/Shop.ts:124; $S/shop.rs:550-554; crates/script/tests/shop.rs:468",
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
        "$TS/api/tasks/PeriodicBank.ts:49-51; $S/periodic_bank.rs:486-493,518-524,529-536",
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
        "$S/reach_entity.rs:570-579,599-603,613-617,661",
    ),
    hook(
        &["reach-entity-op"],
        "target",
        "Reach entity target closure; `{}` is not a tile",
        HookPath::CallSync,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/reach_entity.rs:578,626",
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
        "$TS/api/thieving/CakeStall.ts:72,83; $S/cake_stall/runtime.rs:727-749",
    ),
    hook(
        &["cake_stall"],
        "shouldEat",
        "CakeStall eat predicate; `{}` is truthy",
        HookPath::CallSync,
        HookValue::Used,
        NO_DISCREPANCY,
        "$TS/api/thieving/CakeStall.ts:72,83; $S/cake_stall/runtime.rs:727-749",
    ),
    hook(
        &["cake_stall"],
        "lockedOutUntil",
        "CakeStall lockout deadline; `{}` is truthy",
        HookPath::CallSync,
        HookValue::Used,
        NO_DISCREPANCY,
        "$TS/api/thieving/CakeStall.ts:72,83; $S/cake_stall/runtime.rs:727-749",
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
        "$TS/api/bank/Banking.ts (openBankAccess); $S/shim/banking.js:81-104; $S/periodic_bank.rs:385-388,474-493; $S/banking_open.rs:345-346; $S/bank_access.rs:689-702,713-714,874-875,951-953; $S/dialog.rs:160-163,323-327,355-358; $S/reach.rs:202-205,260-268; $S/reach_entity.rs:484-490,527-545,994-996,1027-1030; $S/walk.rs:1533-1536,1810-1812; $S/boat_fare.rs:1352-1357; $S/anchor_return.rs:101-104",
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
        "$S/hunt.rs:50-88,184-190; $S/hunt_bank.rs:2165-2166",
    ),
    hook(
        HUNT_FAMILIES,
        "weaponName",
        "Hunt state getter",
        HookPath::Ask,
        HookValue::Used,
        NO_DISCREPANCY,
        "$S/hunt.rs:50-88,192-200; $S/hunt_bank.rs:2168",
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
        "$S/hunt.rs:323-356; $S/hunt/acquire.rs:107; $S/hunt/teleport_out.rs:137",
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
        "$TS/api/combat/hunting/combat.ts:554; $S/hunt.rs:798-821",
    ),
    hook(
        HUNT_FAMILIES,
        "countBurial",
        "Hunt count-burial effect hook",
        HookPath::Ask,
        HookValue::Ignored,
        HUNT_NOTIFICATION_PATH_NOT_APPROVED,
        "$S/hunt.rs:50-88,610-621,798-821",
    ),
    hook(
        HUNT_FAMILIES,
        "countBankTrip",
        "Hunt count-bank-trip effect hook",
        HookPath::Ask,
        HookValue::Ignored,
        HUNT_NOTIFICATION_PATH_NOT_APPROVED,
        "$S/hunt.rs:50-88,610-621,798-821",
    ),
    hook(
        HUNT_FAMILIES,
        "setSafespotIndex",
        "Hunt safespot effect hook",
        HookPath::Ask,
        HookValue::Ignored,
        HUNT_NOTIFICATION_PATH_NOT_APPROVED,
        "$S/hunt.rs:50-88,610-621,798-821",
    ),
    hook(
        HUNT_FAMILIES,
        "setTarget",
        "Hunt target effect hook",
        HookPath::Ask,
        HookValue::Ignored,
        HUNT_NOTIFICATION_PATH_NOT_APPROVED,
        "$S/hunt.rs:50-88,610-621,798-821",
    ),
    hook(
        HUNT_FAMILIES,
        "parkFor",
        "Hunt park-for effect hook",
        HookPath::Ask,
        HookValue::Ignored,
        HUNT_NOTIFICATION_PATH_NOT_APPROVED,
        "$S/hunt.rs:50-88,610-621,798-821",
    ),
    hook(
        HUNT_FAMILIES,
        "pickWeapon",
        "Hunt pick-weapon effect hook",
        HookPath::Ask,
        HookValue::Ignored,
        HUNT_NOTIFICATION_PATH_NOT_APPROVED,
        "$S/hunt.rs:50-88,618-621,798-821",
    ),
    hook(
        HUNT_FAMILIES,
        "setStatus",
        "Hunt queued status note and standalone status effect",
        HookPath::Ask,
        HookValue::Ignored,
        HUNT_NOTIFICATION_PATH_NOT_APPROVED,
        "$S/hunt.rs:50-88,610-612,798-821",
    ),
    hook(
        HUNT_FAMILIES,
        "vlog",
        "Hunt queued verbose-log note and standalone verbose-log effect",
        HookPath::Ask,
        HookValue::Ignored,
        HUNT_NOTIFICATION_PATH_NOT_APPROVED,
        "$S/hunt.rs:50-88,610-612,798-821",
    ),
    hook(
        HUNT_FAMILIES,
        "log",
        "Hunt queued log note and standalone log effect",
        HookPath::Ask,
        HookValue::Ignored,
        HUNT_NOTIFICATION_PATH_NOT_APPROVED,
        "$S/hunt.rs:50-88,610-612,798-821",
    ),
    hook(
        &["clue_duel_travel"],
        "log",
        "Duel travel log forwarding",
        HookPath::Ask,
        HookValue::Ignored,
        NO_DISCREPANCY,
        "$S/duel.rs:174-178,1023",
    ),
    hook(
        &["clue_duel_handshake"],
        "log",
        "Duel handshake log forwarding through flush_logs",
        HookPath::Ask,
        HookValue::Ignored,
        NO_DISCREPANCY,
        "$S/duel.rs:174-178,559",
    ),
    hook(
        &["clue_duel_helper"],
        "log",
        "Duel helper log forwarding through flush_logs",
        HookPath::Ask,
        HookValue::Ignored,
        NO_DISCREPANCY,
        "$S/duel.rs:174-178,694",
    ),
    hook(
        &["clue"],
        "log",
        "Clue nested duel-travel log forwarding",
        HookPath::Ask,
        HookValue::Ignored,
        CLUE_PATH_NOT_APPROVED,
        "$S/clue/family.rs:26-33; $TS/api/ai/clues/SolveClue.ts:126-130,345-346",
    ),
    // Clue's ordinary callbacks use Step::Call and are a separate Q5 row-14 discrepancy.
    hook(
        &["clue"],
        "enabled",
        "Clue enabled decision callback",
        HookPath::Awaited,
        HookValue::Used,
        CLUE_PATH_NOT_APPROVED,
        "$S/clue/family.rs:43-49,100-113; $TS/api/ai/clues/SolveClue.ts:211-213",
    ),
    hook(
        &["clue"],
        "log",
        "Clue ordinary log callback (distinct from nested duel-travel forwarding)",
        HookPath::Awaited,
        HookValue::Ignored,
        CLUE_PATH_NOT_APPROVED,
        "$S/clue/family.rs:43-49,73-85,114-123; $TS/api/ai/clues/SolveClue.ts:232-233,343-346,433-434",
    ),
    hook(
        &["clue"],
        "setStatus",
        "Clue status callback",
        HookPath::Awaited,
        HookValue::Ignored,
        CLUE_PATH_NOT_APPROVED,
        "$S/clue/family.rs:43-49,73-85,124-133; $TS/api/ai/clues/SolveClue.ts:278-280,343-345,433-434",
    ),
];

#[cfg(test)]
mod tests {
    use super::{
        FrozenPathStatus, HookCharacterization, HookMultiplicity, HookOrder, HookPath, HookValue,
        PromiseBehavior, ThrowBehavior, HOOK_CHARACTERIZATIONS,
    };
    use serde_json::{json, Value};
    use std::path::PathBuf;

    fn path_name(path: HookPath) -> &'static str {
        match path {
            HookPath::Awaited => "awaited",
            HookPath::CallSync => "call-sync",
            HookPath::Ask => "ask",
        }
    }

    fn value_name(value: HookValue) -> &'static str {
        match value {
            HookValue::Used => "used",
            HookValue::Ignored => "ignored",
        }
    }

    fn order_name(order: HookOrder) -> &'static str {
        match order {
            HookOrder::AwaitedPollBeforeStep => "awaited-poll-before-step",
            HookOrder::SynchronousAtCallSite => "synchronous-at-call-site",
            HookOrder::HuntNotificationsBeforeDecision => "hunt-notifications-before-decision",
            HookOrder::QueuedNotificationsBeforeDecision => {
                "queued-notifications-before-next-decision"
            }
            HookOrder::HuntNotificationsAndEffects => {
                "hunt-notifications-before-decision-and-effects-after-notes"
            }
            HookOrder::HuntEffectAfterNotes => "hunt-effect-after-notes",
        }
    }

    fn multiplicity_name(multiplicity: HookMultiplicity, path: HookPath) -> &'static str {
        match (multiplicity, path) {
            (HookMultiplicity::PerReachedCallSite, HookPath::Ask) => {
                "per-reached-call-site; ASKS_PER_STEP=512 per step"
            }
            (HookMultiplicity::PerQueuedHuntEvent, _) => {
                "per-queued-hunt-event; ASKS_PER_STEP=512 per step"
            }
            (HookMultiplicity::PerHuntNotificationOrEffect, _) => {
                "per-queued-hunt-event-or-effect; ASKS_PER_STEP=512 per step"
            }
            (HookMultiplicity::PerHuntEffect, _) => "per-hunt-effect; ASKS_PER_STEP=512 per step",
            (HookMultiplicity::PerReachedCallSite, _) => {
                "per-reached-call-site; CALLS_PER_TICK=256 per row per tick"
            }
            (HookMultiplicity::PerQueuedNotification, _) => {
                "per-queued-notification; CALLS_PER_TICK=256 per row per tick"
            }
        }
    }

    fn promise_name(promise: PromiseBehavior) -> &'static str {
        match promise {
            PromiseBehavior::PolledSamePromise => "polled-same-promise",
            PromiseBehavior::OpaqueEmptyObject => "opaque-empty-object",
            PromiseBehavior::NotImplementedFailure => "not-impl-failure",
        }
    }

    fn thrown_name(thrown: ThrowBehavior) -> &'static str {
        match thrown {
            ThrowBehavior::FamilyFailsWithThrownValue => "family-fails-with-thrown-value",
            ThrowBehavior::AskFailsWithThrownValue => "ask-fails-with-thrown-value",
        }
    }

    fn frozen_path_value(row: &HookCharacterization) -> Value {
        match row.frozen_path() {
            FrozenPathStatus::MatchesFrozenCallForm => {
                json!({ "status": "matches-frozen-call-form" })
            }
            FrozenPathStatus::ExistingDiscrepancy {
                q5_row,
                proposed_path,
                proposal_approved,
            } => json!({
                "status": "existing-discrepancy",
                "q5_row": q5_row,
                "proposed_path": path_name(proposed_path),
                "proposal_approved": proposal_approved,
            }),
        }
    }

    fn callback_snapshot() -> String {
        let rows = HOOK_CHARACTERIZATIONS
            .iter()
            .map(|row| {
                let family_semantics = row
                    .families()
                    .iter()
                    .map(|family| {
                        json!({
                            "family": family,
                            "order": order_name(row.order(family)),
                            "multiplicity": multiplicity_name(row.multiplicity(family), row.path()),
                            "cadence": row.cadence_note(family),
                        })
                    })
                    .collect::<Vec<_>>();
                json!({
                    "families": row.families(),
                    "family_semantics": family_semantics,
                    "member": row.member(),
                    "call_site": row.call_site(),
                    "path": path_name(row.path()),
                    "value": value_name(row.value()),
                    "promise": promise_name(row.promise()),
                    "thrown": thrown_name(row.thrown()),
                    "frozen_path": frozen_path_value(row),
                    "evidence": row.evidence(),
                })
            })
            .collect::<Vec<_>>();
        let mut output = serde_json::to_string_pretty(&json!({
            "schema_version": 2,
            "source": "crates/script/src/native/abi.rs::HOOK_CHARACTERIZATIONS",
            "rows": rows,
        }))
        .expect("serialize callback ABI census");
        output.push('\n');
        output
    }

    #[test]
    fn callback_snapshot_matches_native_census() {
        assert_eq!(
            callback_snapshot(),
            include_str!("../../compat-js/abi-callbacks.json"),
            "Regenerate with: cargo test -p script --lib regenerate_callback_snapshot -- --ignored"
        );
    }
    fn callback_for(family: &'static str, member: &'static str) -> &'static HookCharacterization {
        HOOK_CHARACTERIZATIONS
            .iter()
            .find(|row| row.member() == member && row.families().contains(&family))
            .unwrap_or_else(|| panic!("missing callback {family}/{member}"))
    }

    #[test]
    fn queued_notifications_and_hunt_effects_keep_their_code_order() {
        for member in ["setStatus", "log", "onComplete", "onDecline"] {
            let row = callback_for("partner-trade", member);
            assert_eq!(
                row.order("partner-trade"),
                HookOrder::QueuedNotificationsBeforeDecision,
                "partner-trade/{member} is drained from the FIFO before the next decision"
            );
            assert_eq!(
                row.multiplicity("partner-trade"),
                HookMultiplicity::PerQueuedNotification
            );
        }
        let missing = callback_for("partner-trade", "onMissingPartner");
        assert_eq!(
            missing.order("partner-trade"),
            HookOrder::SynchronousAtCallSite
        );
        assert_eq!(
            missing.multiplicity("partner-trade"),
            HookMultiplicity::PerReachedCallSite
        );

        for family in [
            "dialog",
            "reach-npc-dialog",
            "reach-entity-op",
            "walk-resilient",
            "walk-opening",
            "walk-to",
            "return-to-anchor",
            "walk-hops",
        ] {
            let row = callback_for(family, "log");
            assert_eq!(
                row.order(family),
                HookOrder::QueuedNotificationsBeforeDecision,
                "{family} drains queued log notifications before the next decision"
            );
            assert_eq!(
                row.multiplicity(family),
                HookMultiplicity::PerQueuedNotification
            );
            assert_eq!(
                row.cadence_note(family),
                "Once per queued notification; delivery precedes the next decision."
            );
        }

        for member in [
            "setSafespotIndex",
            "setTarget",
            "countBurial",
            "countBankTrip",
            "parkFor",
            "pickWeapon",
        ] {
            let row = callback_for("hunt-fight", member);
            assert_eq!(row.order("hunt-fight"), HookOrder::HuntEffectAfterNotes);
            assert_eq!(
                row.multiplicity("hunt-fight"),
                HookMultiplicity::PerHuntEffect
            );
        }
        let count_kill = callback_for("hunt-fight", "countKill");
        assert_eq!(
            count_kill.order("hunt-fight"),
            HookOrder::HuntNotificationsBeforeDecision
        );
        assert_eq!(
            count_kill.multiplicity("hunt-fight"),
            HookMultiplicity::PerQueuedHuntEvent
        );
        for member in ["setStatus", "vlog", "log"] {
            let row = callback_for("hunt-fight", member);
            assert_eq!(
                row.order("hunt-fight"),
                HookOrder::HuntNotificationsAndEffects
            );
            assert_eq!(
                row.multiplicity("hunt-fight"),
                HookMultiplicity::PerHuntNotificationOrEffect
            );
            assert_eq!(
                row.cadence_note("hunt-fight"),
                "Queued Hunt notes are delivered in sequence before the decision; standalone log, vlog and status effects run after notes and before the effect is applied."
            );
        }
    }

    #[test]
    fn walk_sustain_cadence_does_not_inherit_the_hunt_note() {
        let sustain = HOOK_CHARACTERIZATIONS
            .iter()
            .find(|row| row.member() == "sustain")
            .expect("sustain hook row");
        assert_eq!(
            sustain.cadence_note("hunt-fight"),
            "The Hunt fight stepper emits sustain at least once per tick."
        );
        assert_eq!(
            sustain.cadence_note("walk-resilient"),
            "Non-Hunt movement drivers invoke sustain at reached pump call sites; no Hunt tick cadence applies."
        );
        assert_eq!(
            sustain.cadence_note("hunt-bank"),
            "Per reached Hunt sustain call site; no per-tick minimum is asserted."
        );
    }

    #[test]
    #[ignore]
    fn regenerate_callback_snapshot() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("compat-js")
            .join("abi-callbacks.json");
        std::fs::write(&path, callback_snapshot())
            .unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
    }
}
