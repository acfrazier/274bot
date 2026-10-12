//! Complete §28.6 Q5 inventory. Data only: no admission or family activation.
//! §27 §8 approves the table except 8a. Withdrawn row 29 is not an entry.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DivergenceStatus {
    Existing,
    Pinned,
    FamilyCharacterization,
    NoFrozenCounterpart,
    HostChange,
    NewDivergence,
    OutsideAbi,
    OpenRow8a,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Divergence {
    id: &'static str,
    behavior: &'static str,
    status: DivergenceStatus,
    source: &'static str,
}
impl Divergence {
    pub const fn id(self) -> &'static str {
        self.id
    }
    pub const fn behavior(self) -> &'static str {
        self.behavior
    }
    pub const fn status(self) -> DivergenceStatus {
        self.status
    }
    pub const fn source(self) -> &'static str {
        self.source
    }
    pub const fn approved(self) -> bool {
        !matches!(self.status, DivergenceStatus::OpenRow8a)
    }
}
macro_rules! row {
    ($id:literal, $status:ident, $behavior:literal, $source:literal) => {
        Divergence {
            id: $id,
            behavior: $behavior,
            status: DivergenceStatus::$status,
            source: $source,
        }
    };
}
/// All rows, each citing its normative source and relevant product evidence.
/// T2 with T1 must extend/re-review the model before Hunt I-family activates.
pub const DIVERGENCES: &[Divergence] = &[
    row!("1", Existing, "Read hook members once at start; later mutation does not change callbacks", "§28.6 row 1; §25 R5-6; load/machine_v8.rs hook rooting"),
    row!("2", Pinned, "Awaited pending Promise is polled, never re-invoked", "§28.6 row 2; machine.rs:1293-1296,1364-1366"),
    row!("3", Pinned, "One notification per typed event in sequence order, reposts deduplicated", "§28.6 row 3; §25 R5-6.3.2; hunt.rs:798-821"),
    row!("4", FamilyCharacterization, "Keep each family's frozen log/vlog/setStatus invocation point", "§28.6 row 4; §25 R5-6.2"),
    row!("5", Pinned, "Await decision; HookThrew publishes Failed and detached children survive", "§28.6 row 5; §25 R5-7; machine.rs:1351-1356"),
    row!("6", FamilyCharacterization, "Synchronous call point, arguments and path/value Promise/throw semantics stay per member", "§28.6 row 6; §25 R5-6.1"),
    row!("7", NoFrozenCounterpart, "Second live Session root is refused Busy", "§28.6 row 7; §28.4 A2-A4"),
    row!("8a", OpenRow8a, "Model currently refuses top-level Capability beside Hunt; operator requires coexistence before Hunt activation", "§28.6 row 8a; §27 §8 final Q5 answer (2026-10-11)"),
    row!("8b", HostChange, "New Capability root under Gather/Combat/progress is Busy rather than silent gameplay drop", "§28.6 row 8b; slot/api.rs:145-149; slot.rs:1788-1795"),
    row!("9a", NewDivergence, "Fifth root or 17th live entry is Busy; projected replacement may reuse capacity", "§28.6 row 9a; §28.3 projected replacement"),
    row!("9b", NewDivergence, "Second Started child is ParentBusy; adopted children reserve leaves but not the Started slot", "§28.6 row 9b; §28.3 started-child rule"),
    row!("9c", NewDivergence, "Child cannot reopen an ancestor exclusive group", "§28.6 row 9c; §28.4 A2-A4"),
    row!("10", Pinned, "Reconnect drops queued one-shot clicks without replay", "§28.6 row 10; load/isolate/thread.rs:1838"),
    row!("11", Pinned, "In-flight request resumes saved state, never restarts", "§28.6 row 11; §25 R5-8.5; machine.rs:121-127"),
    row!("12", HostChange, "Gather/Combat sub-operation interrupts on reconnect while Session remains live", "§28.6 row 12; slot/combat.rs:343-355; slot/api.rs:547-556"),
    row!("13", HostChange, "ManualWalk is Done(false) with authority revocation; descendants ParentAborted leaf-first", "§28.6 row 13; §25 R5-7; walk.rs:1585-1591"),
    row!("14", Existing, "Clue enabled/log/setStatus currently await Promises despite frozen synchronous calls", "§28.6 row 14; clue/family.rs:43-49,100-133"),
    row!("15", Existing, "Hunt structural ask/property mapping remains; awaited sustain still waits", "§28.6 row 15; §25 R5-6.2; hunt.rs:152-222,525-530"),
    row!("16", NewDivergence, "Revocation drops accepted gameplay; ordinary completion preserves captured authority subject to foreground", "§28.6 row 16; §28.2; §27 §6 accepted-authority exception"),
    row!("17", Pinned, "Detached children survive completion; promote all root children, adopt all non-root children", "§28.6 row 17; §28.3; row 24 separately discloses later ancestor cancellation"),
    row!("18", Pinned, "bankNearest and PeriodicBank can be Capability children when bounds permit", "§28.6 row 18; §25 R5-4.6; periodic_bank.rs:380-392,473-496"),
    row!("19", OutsideAbi, "Derived AsyncContext preserves lineage; unrelated replacement is Foreign with once-per-run diagnostic", "§28.6 row 19; §25 R5-2.2.5, R5-2.3"),
    row!("20", NewDivergence, "Credit-deferred start remains custody; pending gameplay needs admission witness and never replays", "§28.6 row 20; §28.4; pending-count cap 32"),
    row!("21", Existing, "Hunt ask/ignored Promise currently fails not-impl; frozen notification returns ignored", "§28.6 row 21; machine.rs:464-469; hunt.rs:798-821"),
    row!("22", HostChange, "Releases bypass foreground and survive guardian hold; controls execute after resume", "§28.6 row 22; §27 §6 guardian hold; slot.rs:1788-1795; load/isolate/thread.rs:1475-1491"),
    row!("23", NewDivergence, "A fifth reserved future root leaf is refused Busy", "§28.6 row 23; §28.3 root reservation"),
    row!("24", NewDivergence, "Adopted subtree is cancelled ParentAborted when adoptive ancestor later revokes", "§28.6 row 24; §28.3 later ancestor revocation"),
    row!("25", NewDivergence, "Unpinned lookup below history/generation floor is HistoryExpired, not root or revocation", "§28.6 row 25; §28.2 history ring capacity 64"),
    row!("26", NewDivergence, "Start charge over 64 KiB or over 32 callback handles is local ArgsTooLarge; Gather settings still 4 KiB", "§28.6 row 26; §28.4; isolate_fb.rs:3131-3144"),
    row!("27", NewDivergence, "Reserve Walk release before arming; coalesce per Walk id and singleton GatherStop/CombatStop", "§28.6 row 27; §28.5"),
    row!("28", NewDivergence, "Pre-ack parent revocation retains start+cancel unit and publishes ParentAborted only with credit", "§28.6 row 28; §28.4 hybrid cancellation and retirement"),
];
