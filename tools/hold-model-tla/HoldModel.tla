---------------------------- MODULE HoldModel ----------------------------
(***************************************************************************)
(* The auto-run hold of 274bot at commit 178a002005a62b49b650544653bae9f8   *)
(* (branch nav-prereq-closure).  HOLD-MODEL-AB arm T.                        *)
(*                                                                          *)
(* WHAT IS MODELLED.  One slot.  One host frame is six stages:              *)
(*   Drain  : packets (tick, modal open/close, relog, death message);       *)
(*            host `after_drain` releases the hold at a session boundary    *)
(*            (crates/host/src/lib.rs:1536-1543, :279-283).                 *)
(*   Obs1   : slot-side session boundary revoke (play_slots.rs:1546-1563)   *)
(*            and `sync_compiled_clue`: the active clock observes its       *)
(*            eligibility and the native quiet read checks its wall bound   *)
(*            (slot.rs:1952-1958, ledger.rs:349-367).                       *)
(*   Poll   : the owner's poll, only if the slot is Running, not held, and  *)
(*            this frame has a tick edge or an evidence wake                *)
(*            (script_observe.rs:815-827, :1120-1126).                      *)
(*   Obs2   : the host samples the lease, under the slot lock               *)
(*            (slot.rs:2259-2294, script_observe.rs:1324-1325).             *)
(*   Update : `sync_runtime` and `observe_script_wait` with the sampled     *)
(*            values, AFTER the slot lock is released                       *)
(*            (script_observe.rs:1327-1338, host lib.rs:203-267).           *)
(*   Press  : `apply_auto_run` (host lib.rs:1548-1588).                     *)
(* Stop, Pause, Resume, Start and Hang can occur between any two stages,    *)
(* except while the slot lock is held (between Poll and Obs2).  Wall time   *)
(* and the active clock advance anywhere.                                   *)
(*                                                                          *)
(* STATE.  One record `s`; each field is read through an alias below, and   *)
(* an action writes only the fields it names (`Set`).                       *)
(*                                                                          *)
(* ABSTRACTIONS (each with its reason).                                     *)
(* A1 Clock values are small naturals, not milliseconds.  Wall time, the    *)
(*    owner's active clock and observed ticks are three counters that       *)
(*    advance independently (a tick is a server event, at most one per      *)
(*    frame; it may come slower or faster than wall time).  The active      *)
(*    clock advances only while its span is open, and the span follows      *)
(*    eligibility with the one-frame lag of `sync_compiled_clue`.  The code *)
(*    only compares these counters with `>=`, `<` and a +1 margin, so the   *)
(*    order relations are all that matter.                                  *)
(* A2 One lease at a time.  The slot has one foreground owner, and the      *)
(*    sample reports Modal first, then Quiet, then Compat                   *)
(*    (slot.rs:2269-2293); two live leases would only change which one is   *)
(*    reported.                                                             *)
(* A3 The owner is a relation, not code.  MStep/QStep/CStep list what the   *)
(*    Dialogue, the native journal machine and the JS journal machine can   *)
(*    do at one poll, with the code's own order (an acknowledgement is      *)
(*    checked before the deadline, dialogue.rs:526-545).  Page content and  *)
(*    item state are not modelled; "modal" is one boolean, and an           *)
(*    acknowledgement is a free choice.                                     *)
(* A4 Press conditions other than the hold and the modal check (energy,     *)
(*    run_on, ready) are taken as true: the model presses on every frame    *)
(*    that is not held and has no modal, which is the strongest case for    *)
(*    S1.  A press is not gated on `held`: apply_auto_run does not read     *)
(*    the guardian hold.                                                    *)
(* A5 Keys are `<<kind, n>>` with per-kind counters.  The code tags a u64   *)
(*    (1<<62 modal, 1<<63 compat, none for quiet); the tag is the kind      *)
(*    field here.  A ghost `txn` numbers every lease begin, so a key that   *)
(*    names two transactions can be seen (S4).                              *)
(* A6 The host frame `tick` is the observed-tick counter; the u64 wrap is   *)
(*    out of scope.                                                         *)
(* A7 Packets that change no modelled state are not modelled, except that   *)
(*    any drain may mark evidence dirty (an evidence wake).                 *)
(* A8 Horizon.  With Room = TRUE a lease starts only while the room guards *)
(*    RoomT/RoomW/RoomA hold, so every bound can be reached inside          *)
(*    MaxTick/MaxWall/MaxAct.  The liveness run needs this: it keeps the    *)
(*    check finite without a clock frozen at a cap.  Safety runs use        *)
(*    Room = FALSE; the Bound constraint alone cuts them.                   *)
(*                                                                          *)
(* MUTATIONS.  `Mut` selects a transition-only variant; "head" is the      *)
(* product at 178a00200.  The invariants never read `Mut`.  Phase 1 sanity  *)
(* variants (they show that an invariant can fail):                         *)
(*   NoSessionRelease  host after_drain does not release at a boundary      *)
(*                     (equivalent while the slot revokes the lease)        *)
(*   NoSlotRevoke      the slot does not revoke leases at a boundary        *)
(*   NoBoundary        both of the above                                    *)
(*   StopKeeps         sync_runtime(None) does not clear the hold           *)
(*                     (equivalent while Stop revokes the lease)            *)
(*   StopNoRevoke      Stop does not revoke the lease                       *)
(*   KeyReuseModal     every modal lease takes the same key                  *)
(*   NoTerminal        the host never releases a hold on its own (no        *)
(*                     deadline-plus-margin release)                        *)
(* Phase 2 (earlier designs, transitions only; "+Fix" adds FixRearm):      *)
(*   M1, M2   no leases: the hold ends at a modal or 2 / 3 ticks after the  *)
(*            click, whichever is first                                     *)
(*   M3       fixed release 2 ticks (code analogue: 25) from the first      *)
(*            sample of a key, whatever the owner's deadline                *)
(*   M4       M3 plus one constant key for every compat lease (and a        *)
(*            compat read may end and the next begin in one poll)           *)
(*   M5       the active-clock bound is converted once to a wall deadline   *)
(*   M6       the compat hold is not renewed at the click                   *)
(*   BackToBackC  head plus a compat read that ends and the next one       *)
(*            begins in one poll (a transition M4 needs; head must pass)    *)
(*   FixRearm          proposed fix of D1: a renewed deadline re-arms a     *)
(*                     terminally released hold of the same key             *)
(* A margin of 0 is the constant Margin = 0.                                *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets, TLC

CONSTANTS
    MaxTick,    \* horizon, observed ticks
    MaxWall,    \* horizon, wall units
    MaxAct,     \* horizon, owner active-clock units
    MaxRelogs,  \* number of session changes
    MaxStops,   \* number of Stop events (each may be followed by a Start)
    MaxDeaths,  \* number of death messages
    MaxToggles, \* number of Pause and freeze changes
    MaxTxn,     \* number of lease begins (all kinds)
    HB,         \* compat hold bound, wall units (code: 10 s)
    QW,         \* native quiet read wall bound, wall units (code: 10 s)
    AckB,       \* dialogue page/ack bound, active-clock units (code: 3 s / 8 s)
    GapT,       \* dialogue end gap, ticks (code: up to 30)
    WinT,       \* journal window, ticks (code: 5)
    Margin,     \* host margin, ticks (code: SCRIPT_WAIT_MARGIN_TICKS = 1)
    Room,       \* TRUE: leases start only with room to reach their bounds (liveness runs)
    Wrap,       \* TRUE: at MaxTick a tick slides every stored tick value down by 1 (liveness runs)
    Mut         \* mutation name

ASSUME /\ MaxTick \in Nat /\ MaxWall \in Nat /\ MaxAct \in Nat /\ Margin \in Nat
       /\ GapT >= WinT

\* "Nothing" values have the same shape as the real ones, so TLC can compare them.
NoKey   == <<"-", 0>>
NoBound == [k |-> "-", v |-> 0]
NoLease == [kind |-> "-", key |-> NoKey, txn |-> 0, ph |-> "-",
            own |-> NoBound, st |-> NoBound, since |-> 0, wdl |-> 0]
NoHold  == [key |-> NoKey, rel |-> FALSE, txn |-> 0, reldt |-> 0, st |-> 0]  \* reldt: deadline at release (read only by Mut = "FixRearm")
NoSmp   == [key |-> NoKey, dt |-> 0, txn |-> 0, opass |-> FALSE]
NoPress == [blocked |-> FALSE]     \* ghost: the last press went out with a live lease
NoTerm  == [bad |-> FALSE]         \* ghost: the last terminal release broke S3
NoPend  == [txnMax |-> 0, n |-> 0] \* ghost: presses held by an old-transaction hold (capped at 2)

VARIABLE s

\* Field aliases.
pc == s.pc
tick == s.tick          \* observed game ticks (host frame tick)
wall == s.wall          \* wall time
actV == s.actV          \* the owner's active clock, as the sample reads it
clkOpen == s.clkOpen    \* the clock's eligible span is open
alive == s.alive        \* a script runtime exists (Starting/Running/Paused)
rtg == s.rtg            \* runtime generation
want == s.want          \* operator run intent (false = Paused)
held == s.held          \* hold, not ready, not up, recovery hold, blocked: no dispatch
hung == s.hung          \* the owner stopped polling for good
modal == s.modal        \* a modal page is open on the client
tp == s.tp              \* a PLAYER_INFO is queued for the next drain
tickEdge == s.tickEdge  \* this frame's drain had a PLAYER_INFO
dirty == s.dirty        \* this frame's drain changed some evidence family
bnd == s.bnd            \* this frame's drain was a session boundary
deadMsg == s.deadMsg    \* a death message the owner has not seen
L == s.L                \* the owner's lease
kc == s.kc              \* per-kind key counters
txnN == s.txnN          \* ghost: lease begins so far
hrt == s.hrt            \* host: ScriptRunPolicy.runtime_generation (0 = None)
hw == s.hw              \* host: ScriptRunPolicy.script_wait
smp == s.smp            \* the sample taken at Obs2, applied at Update
smpRt == s.smpRt        \* the runtime generation read with it (0 = none)

Max2(a, b) == IF a >= b THEN a ELSE b

\* Write the fields named in the record `u`; every other field is unchanged.
Set(u) == s' = [f \in DOMAIN s |-> IF f \in DOMAIN u THEN u[f] ELSE s[f]]

ActNow == actV

\* The clock is eligible: Running by operator intent, not held, instance alive
\* (slot.rs:2013-2032 `dispatch_open && !held`).
Eligible == alive /\ want /\ ~held

\* The hold is in force (`should_defer_auto_run`, lib.rs:272-274).
Eff == hw # NoHold /\ ~hw.rel

\* The owner polls on this frame (script_observe.rs:815-827 and 1120-1126).
PollOn == /\ alive /\ want /\ ~held /\ ~hung
          /\ (tickEdge \/ (tick > 0 /\ dirty))

RoomT == ~Room \/ Wrap \/ tick + GapT + Margin + 1 <= MaxTick
RoomW == ~Room \/ wall + Max2(HB, QW) + 1 <= MaxWall
RoomA == ~Room \/ actV + AckB + 1 <= MaxAct

\* The owner's own deadline has passed, in the owner's own clock.
OwnerPassed(l) ==
    CASE l.kind = "M" -> IF l.own.k = "T" THEN tick >= l.own.v ELSE actV >= l.own.v
      [] l.kind = "Q" -> tick >= l.own.v \/ wall - l.since >= QW
      [] l.kind = "C" -> wall >= l.own.v

\* Phase 2 mutation groups (see the header).
FixOn      == Mut \in {"FixRearm", "BackToBackC+Fix", "M5+Fix", "M6+Fix"}
NoLeaseHost == Mut \in {"M1", "M2"}            \* M1, M2: no leases; the host counts ticks from the click
FixedHost  == Mut \in {"M3", "M4"}              \* M3, M4: fixed release counted from the first sample
WallConv   == Mut \in {"M5", "M5+Fix"}          \* M5: the active bound is converted to wall time once
NoClickRenew == Mut \in {"M6", "M6+Fix"}        \* M6: the compat hold is not renewed at the click
ConstC     == Mut = "M4"                        \* M4: every compat lease has one key
BackToBackC == Mut \in {"M4", "BackToBackC", "BackToBackC+Fix"}    \* a compat read may end and the next begin in one poll
M1Ticks == IF Mut = "M1" THEN 2 ELSE 3         \* M1 releases 2 ticks after the click, M2 3 ticks
M3Limit == 2                                    \* M3, M4: fixed release after 2 ticks (code analogue: 25 against gaps up to 30)

MkKey(kind) ==
    IF Mut = "KeyReuseModal" /\ kind = "M" THEN <<kind, 0>>
    ELSE IF ConstC /\ kind = "C" THEN <<kind, 0>>
    ELSE <<kind, kc[kind] + 1>>

NewLease(kind, ph, own) ==
    [kind |-> kind, key |-> MkKey(kind), txn |-> txnN + 1, ph |-> ph,
     own |-> own, st |-> own, since |-> wall,
     wdl |-> IF WallConv /\ own.k = "A" THEN wall + (own.v - actV) ELSE 0]

\* Updates for a lease begin: the lease, the kind's key counter, the ghost counter.
BeginU(l) == [L |-> l, kc |-> [kc EXCEPT ![l.kind] = kc[l.kind] + 1], txnN |-> txnN + 1]

\* Sliding tick window (liveness runs, Wrap = TRUE).  The code only compares the
\* tick with stored ticks (`>=`, `<`, +margin), so one more tick is the same as
\* one fewer on every stored tick value; values clip at 0 (a bound that is far in
\* the past stays passed).  With Wrap the tick counter is unbounded in effect
\* and the state space is still finite.
Sub1(v) == IF v > 0 THEN v - 1 ELSE 0
Slide == Wrap /\ tick = MaxTick
ShiftB(b) == IF b.k = "T" THEN [b EXCEPT !.v = Sub1(b.v)] ELSE b
ShiftL(l) == [l EXCEPT !.own = ShiftB(l.own), !.st = ShiftB(l.st)]

\* The lease is kept, only its fields change (same key, same transaction).
RenewU(l) == [L |-> l]

EndU == [L |-> NoLease]

-----------------------------------------------------------------------------
\* Initial state

Init ==
    s = [pc |-> "Drain", tick |-> 0, wall |-> 0, actV |-> 0, clkOpen |-> FALSE,
         alive |-> TRUE, rtg |-> 1, want |-> TRUE, held |-> FALSE, hung |-> FALSE,
         modal |-> FALSE, tp |-> FALSE, tickEdge |-> FALSE, dirty |-> FALSE,
         bnd |-> FALSE, deadMsg |-> FALSE,
         L |-> NoLease, kc |-> [k \in {"M", "Q", "C"} |-> 0], txnN |-> 0,
         hrt |-> 0, hw |-> NoHold, smp |-> NoSmp, smpRt |-> 0,
         lastPress |-> NoPress, lastTerm |-> NoTerm,
         pendStop |-> NoPend, pendBnd |-> NoPend, pendDead |-> NoPend, pendDeadAny |-> NoPend,
         polled |-> FALSE, clickBad |-> FALSE,
         nRelogs |-> 0, nStops |-> 0, nDeaths |-> 0, nToggles |-> 0,
         act |-> "init"]

-----------------------------------------------------------------------------
\* Environment: time, ticks, operator, failures.

\* Where the world can change between two stages.  Time passes at a frame
\* boundary (before the sync and the poll) and between the poll and the sample
\* (the poll read its clocks earlier than the sample, script_observe.rs:1141 vs
\* :1325).  The operator and failure events cannot happen while the slot lock is
\* held (Poll to Obs2); they are tried at the frame boundary and between the
\* sample and the host update (the sample is then stale).  Other positions give
\* no new orders: the stages between them read none of these.
Unlocked == pc \in {"Drain", "Update"}
TimeAt == pc \in {"Drain", "Obs2"}

WallAdv ==
    /\ TimeAt
    /\ wall < MaxWall
    /\ Set([wall |-> wall + 1, act |-> "Wall"])

\* The owner's active clock runs only while its span is open.
ActAdv ==
    /\ TimeAt
    /\ clkOpen /\ actV < MaxAct
    /\ Set([actV |-> actV + 1, act |-> "ActClock"])

TickArrive ==
    /\ pc = "Drain"
    /\ ~tp /\ (Wrap \/ tick < MaxTick)
    /\ Set([tp |-> TRUE, act |-> "TickArrive"])

\* Operator Pause: want_run false; the clock span closes at once (slot.rs:1526-1530).
Pause ==
    /\ Unlocked /\ alive /\ want /\ s.nToggles < MaxToggles
    /\ Set([want |-> FALSE, clkOpen |-> FALSE, nToggles |-> s.nToggles + 1, act |-> "Pause"])

Resume ==
    /\ Unlocked /\ alive /\ ~want
    /\ Set([want |-> TRUE, act |-> "Resume"])

\* Stop: the runtime ends; its native leases and the compat lease are revoked
\* (slot.rs:1620-1701, isolate.rs:1515).  Start takes a new generation.
Stop ==
    /\ Unlocked /\ alive /\ s.nStops < MaxStops
    /\ Set([alive |-> FALSE, L |-> IF Mut = "StopNoRevoke" THEN L ELSE NoLease,
            hung |-> FALSE, deadMsg |-> FALSE,
            clkOpen |-> FALSE, nStops |-> s.nStops + 1,
            pendStop |-> [txnMax |-> txnN, n |-> 0], act |-> "Stop"])

Start ==
    /\ Unlocked /\ ~alive
    /\ Set([alive |-> TRUE, rtg |-> rtg + 1, want |-> TRUE, act |-> "Start"])

\* The owner stops polling and never ends its wait (a hung or frozen owner).
Hang ==
    /\ Unlocked /\ alive /\ ~hung /\ L # NoLease
    /\ Set([hung |-> TRUE, act |-> "Hang"])

\* The freeze ends (fair, see FairSpec).
Unhold ==
    /\ Unlocked /\ held /\ ~bnd
    /\ Set([held |-> FALSE, act |-> "Unhold"])

-----------------------------------------------------------------------------
\* Stage Drain.  One drain: optional tick, modal change, relog, death, dirt,
\* freeze change, energy change.  The host's `after_drain` releases the hold at
\* a session boundary (lib.rs:1538-1543).

Drain ==
    /\ pc = "Drain"
    /\ \E doTick \in BOOLEAN, flip \in BOOLEAN, relog \in BOOLEAN, die \in BOOLEAN,
          dirt \in BOOLEAN, hold2 \in BOOLEAN :
       /\ doTick = tp  \* a queued PLAYER_INFO is read by the next drain
       /\ relog => s.nRelogs < MaxRelogs
       /\ die => s.nDeaths < MaxDeaths
       /\ (~relog /\ hold2 # held) => s.nToggles < MaxToggles
       /\ Set([pc |-> "Obs1",
               tick |-> IF doTick /\ ~Slide THEN tick + 1 ELSE tick,
               L |-> IF doTick /\ Slide /\ L # NoLease THEN ShiftL(L) ELSE L,
               smp |-> IF doTick /\ Slide /\ smp # NoSmp THEN [smp EXCEPT !.dt = Sub1(smp.dt)] ELSE smp,
               tp |-> IF doTick THEN FALSE ELSE tp,
               tickEdge |-> doTick,
               modal |-> IF relog THEN FALSE ELSE (IF flip THEN ~modal ELSE modal),
               dirty |-> (dirt \/ flip \/ relog \/ die),
               bnd |-> relog,
               deadMsg |-> (deadMsg \/ die),
               held |-> IF relog THEN TRUE ELSE hold2,
               nRelogs |-> IF relog THEN s.nRelogs + 1 ELSE s.nRelogs,
               nDeaths |-> IF die THEN s.nDeaths + 1 ELSE s.nDeaths,
               nToggles |-> IF (~relog /\ hold2 # held) THEN s.nToggles + 1 ELSE s.nToggles,
               hw |-> IF relog /\ hw # NoHold /\ Mut \notin {"NoSessionRelease", "NoBoundary"}
                      THEN [hw EXCEPT !.rel = TRUE, !.reldt = MaxTick + MaxTick] ELSE
                      IF doTick /\ Slide /\ hw # NoHold THEN [hw EXCEPT !.reldt = Sub1(hw.reldt)] ELSE hw,
               pendBnd |-> IF relog THEN [txnMax |-> txnN, n |-> 0] ELSE s.pendBnd,
               pendDead |-> IF die THEN [txnMax |-> txnN, n |-> 0] ELSE s.pendDead,
               pendDeadAny |-> IF die THEN [txnMax |-> txnN, n |-> 0] ELSE s.pendDeadAny,
               act |-> "Drain"])

\* Stage Obs1.  Slot-side boundary revoke, clock sync, quiet-read wall bound.
Obs1 ==
    /\ pc = "Obs1"
    /\ LET lB == IF bnd /\ Mut \notin {"NoSlotRevoke", "NoBoundary"} THEN NoLease ELSE L
           lQ == IF lB # NoLease /\ lB.kind = "Q" /\ wall - lB.since >= QW
                 THEN NoLease ELSE lB
       IN Set([pc |-> "Poll", L |-> lQ,
               clkOpen |-> (alive /\ want /\ ~held /\ ~bnd), act |-> "Obs1"])

-----------------------------------------------------------------------------
\* Stage Poll.  The owner.  Each disjunct has the code's own guard.  An
\* obligation (an owner whose bound has passed ends the wait) is encoded by
\* offering no "stay" disjunct once the bound has passed.

When(g, u) == IF g THEN {u} ELSE {}

Nop == [act |-> "Poll"]

\* Dialogue (dialogue.rs:385-645, sync_modal_wait :361-383).  Phases: Open = the
\* first page after a Talk; Ack = the acknowledgement of a Continue or answer;
\* Gap = the quiet end gap after the chat closed.
MOpts ==
    LET newSend == When(txnN < MaxTxn /\ RoomA,
                        BeginU(NewLease("M", "Ack", [k |-> "A", v |-> ActNow + AckB])))
    IN
    UNION {
         \* ---- Open
         IF L.ph # "Open" THEN {} ELSE
            (IF modal THEN newSend \cup {EndU} \* page seen: a new send, or no page awaited
             ELSE IF OwnerPassed(L) THEN {EndU} \* the page never came: Failed
             ELSE {Nop}),
         \* ---- Ack
         IF L.ph # "Ack" THEN {} ELSE
            (\* Acknowledged with the chat closed: the end gap starts.  The epoch
             \* (steps, rearms) is unchanged, so the SAME lease is renewed and its bound
             \* moves from the active clock to observed ticks (dialogue.rs:372-381).  The
             \* acknowledgement is checked BEFORE the deadline (dialogue.rs:526-545), so
             \* this may happen after the active bound has passed.
             When(~modal /\ RoomT,
                  RenewU([L EXCEPT !.ph = "Gap",
                                   !.own = [k |-> "T", v |-> tick + GapT],
                                   !.st = [k |-> "T", v |-> tick + GapT]]))
             \cup (IF modal THEN newSend \cup {EndU} ELSE {})
             \cup (IF OwnerPassed(L) THEN {EndU} ELSE {Nop})),
         \* ---- Gap
         IF L.ph # "Gap" THEN {} ELSE
            (IF modal THEN newSend \cup {EndU}  \* a scripted page reopened: drive, a new send, or end
             ELSE When(txnN < MaxTxn /\ RoomT,  \* re-armed gap: epoch changes, new lease
                       BeginU(NewLease("M", "Gap", [k |-> "T", v |-> tick + GapT])))
                  \cup (IF OwnerPassed(L) THEN {EndU} ELSE {Nop}))
       }

\* Native journal machine (quest_journal/native.rs:152-322).
QOpts ==
    IF tick >= L.own.v THEN {EndU}   \* window over: "journal modal timeout"
    ELSE {EndU, Nop}                 \* observed close, or still reading
         \cup When(RoomT,            \* next phase: arm_window renews the same lease
                   RenewU([L EXCEPT !.own = [k |-> "T", v |-> tick + WinT],
                                    !.st = [k |-> "T", v |-> tick + WinT]]))

\* Compat journal machine (compat/quest_journal/isolate.rs:319-332, 452-603).
COpts ==
    IF L.ph = "R"
    THEN {EndU}   \* abort before the click
         \cup When(RoomW,  \* click: a fresh hold bound from now, also after the old one
                   IF NoClickRenew   \* M6: the bound stays the one from the begin
                   THEN [L |-> [L EXCEPT !.ph = "A"],
                         clickBad |-> (wall >= L.st.v)]
                   ELSE [L |-> [L EXCEPT !.ph = "A",
                                         !.own = [k |-> "W", v |-> wall + HB],
                                         !.st = [k |-> "W", v |-> wall + HB]],
                         clickBad |-> FALSE])
    ELSE (IF wall >= L.own.v THEN {EndU}   \* modal-timeout
          ELSE {EndU, Nop})                \* read done, or still reading
         \cup (IF BackToBackC /\ txnN < MaxTxn /\ RoomW /\ ~modal
               THEN {BeginU(NewLease("C", "R", [k |-> "W", v |-> wall + HB]))}  \* read done, next begins at once
               ELSE {})

BeginOpts ==
    IF txnN >= MaxTxn THEN {}
    ELSE UNION {
           When(RoomA, BeginU(NewLease("M", "Open", [k |-> "A", v |-> ActNow + AckB]))),
           When(~modal /\ RoomT /\ RoomW,
                BeginU(NewLease("Q", "Q", [k |-> "T", v |-> tick + WinT]))),
           When(~modal /\ RoomW,
                BeginU(NewLease("C", "R", [k |-> "W", v |-> wall + HB])))
         }

OwnerOpts ==
    IF deadMsg
    THEN {[L |-> NoLease, deadMsg |-> FALSE]}  \* death: the runner cancels its step (nav.md:644)
    ELSE IF L = NoLease THEN {Nop} \cup BeginOpts
    ELSE IF L.kind = "M" THEN MOpts
    ELSE IF L.kind = "Q" THEN QOpts
    ELSE COpts

Poll ==
    /\ pc = "Poll"
    /\ \E u \in (IF PollOn THEN OwnerOpts ELSE {Nop}) :
          Set(u @@ [pc |-> "Obs2", polled |-> PollOn,
                    act |-> IF PollOn THEN "Poll" ELSE "PollSkipped"])

-----------------------------------------------------------------------------

\* Stage Obs2.  The host reads the lease (slot.rs:2259-2294).  The sample can
\* change the lease's stored deadline (native.rs:922-931 `owner_tick`) and
\* ends a quiet read at its wall bound (ledger.rs:349-367).

SampleResult ==
    IF L = NoLease THEN [lease |-> NoLease, s |-> NoSmp]
    ELSE CASE L.kind = "M" ->
           LET dt == IF L.st.k = "T" THEN L.st.v ELSE tick
               st2 == IF L.st.k = "A" /\ (IF WallConv THEN wall >= L.wdl ELSE actV >= L.st.v)
                      THEN [k |-> "T", v |-> tick] ELSE L.st
           IN [lease |-> [L EXCEPT !.st = st2],
               s |-> [key |-> L.key, dt |-> dt, txn |-> L.txn, opass |-> OwnerPassed(L)]]
      [] L.kind = "Q" ->
           IF wall - L.since >= QW THEN [lease |-> NoLease, s |-> NoSmp]
           ELSE [lease |-> L,
                 s |-> [key |-> L.key, dt |-> L.st.v, txn |-> L.txn, opass |-> OwnerPassed(L)]]
      [] L.kind = "C" ->
           IF wall >= L.st.v THEN [lease |-> L, s |-> NoSmp]
           ELSE [lease |-> L,
                 s |-> [key |-> L.key, dt |-> tick, txn |-> L.txn, opass |-> OwnerPassed(L)]]

Obs2 ==
    /\ pc = "Obs2"
    /\ Set([pc |-> "Update", L |-> SampleResult.lease, smp |-> SampleResult.s,
            smpRt |-> IF alive THEN rtg ELSE 0, act |-> "Sample"])

-----------------------------------------------------------------------------
\* Stage Update.  `sync_runtime` then `observe_script_wait` (lib.rs:203-267).

Update ==
    /\ pc = "Update"
    /\ LET hw0 == IF hrt # smpRt /\ ~(Mut = "StopKeeps" /\ smpRt = 0) THEN NoHold ELSE hw
           fresh == [key |-> smp.key, rel |-> FALSE, txn |-> smp.txn, reldt |-> 0,
                     st |-> IF NoLeaseHost \/ FixedHost THEN tick ELSE 0]
           hw1 == IF NoLeaseHost
                  THEN \* M1, M2: no leases.  A new key is a new click; the hold stays until it ends.
                       (IF smp # NoSmp /\ (hw0 = NoHold \/ hw0.key # smp.key) THEN fresh ELSE hw0)
                  ELSE IF smp = NoSmp THEN NoHold
                  ELSE IF hw0 # NoHold /\ hw0.key = smp.key
                       THEN (IF FixOn /\ hw0.rel /\ smp.dt > hw0.reldt
                             THEN [hw0 EXCEPT !.rel = FALSE] ELSE hw0)
                  ELSE fresh
           \* M1, M2: ends when a modal is open or M1Ticks ticks after the click.
           hw2 == IF NoLeaseHost /\ hw1 # NoHold /\ ~hw1.rel /\ (modal \/ tick >= hw1.st + M1Ticks)
                  THEN [hw1 EXCEPT !.rel = TRUE] ELSE hw1
           terminal == /\ ~NoLeaseHost /\ smp # NoSmp /\ ~hw2.rel
                       /\ (IF FixedHost THEN tick >= hw2.st + M3Limit
                            ELSE tick >= smp.dt + Margin)
                       /\ Mut # "NoTerminal"
       IN Set([pc |-> "Press", hrt |-> smpRt,
               hw |-> IF terminal THEN [hw2 EXCEPT !.rel = TRUE, !.reldt = smp.dt] ELSE hw2,
               lastTerm |-> IF terminal
                            THEN [bad |-> ~(smp.opass /\ tick >= smp.dt + 1)]
                            ELSE s.lastTerm,
               act |-> IF terminal THEN "Update:TERMINAL-RELEASE" ELSE "Update"])

-----------------------------------------------------------------------------
\* Stage Press.  `apply_auto_run` (lib.rs:1548-1588): no modal, no hold.

PendTick(p, counts) ==
    IF counts /\ Eff /\ hw.txn <= p.txnMax THEN [p EXCEPT !.n = IF p.n >= 2 THEN 2 ELSE p.n + 1] ELSE p

Press ==
    /\ pc = "Press"
    /\ LET fire == ~modal /\ ~Eff
       IN Set([pc |-> "Drain",
               lastPress |-> IF fire
                             THEN [blocked |-> (L # NoLease /\ ~OwnerPassed(L))]
                             ELSE s.lastPress,
               pendStop |-> PendTick(s.pendStop, TRUE),
               pendBnd |-> PendTick(s.pendBnd, TRUE),
               pendDead |-> PendTick(s.pendDead, s.polled),
               pendDeadAny |-> PendTick(s.pendDeadAny, TRUE),
               act |-> IF fire THEN "PRESS" ELSE IF Eff THEN "Press-held" ELSE "Press-none"])

-----------------------------------------------------------------------------
Next ==
    \/ Drain \/ Obs1 \/ Poll \/ Obs2 \/ Update \/ Press
    \/ WallAdv \/ ActAdv \/ TickArrive \/ Pause \/ Resume \/ Stop \/ Start \/ Hang

NextF == Next \/ Unhold

Spec == Init /\ [][Next]_s

\* Fairness (justified in README.md).  Weak fairness: the six frame stages run.
\* Strong fairness (these actions are enabled only at some stages, so weak
\* fairness would not force them): ticks and wall time keep coming, the active
\* clock runs while its span is open, and the operator eventually resumes a
\* paused script and the freeze ends.  Stop, Hang, relog and death are NOT forced.
FairSpec == Init /\ [][NextF]_s
            /\ WF_s(Drain) /\ WF_s(Obs1) /\ WF_s(Poll) /\ WF_s(Obs2)
            /\ WF_s(Update) /\ WF_s(Press)
            /\ SF_s(WallAdv) /\ SF_s(ActAdv) /\ SF_s(TickArrive)
            /\ SF_s(Resume) /\ SF_s(Unhold)

\* TLC VIEW: the trace label `act` is not part of a state's identity.
View == [f \in (DOMAIN s) \ {"act"} |-> s[f]]

\* State constraint: bounds a run.
Bound == /\ s.txnN <= MaxTxn /\ tick <= MaxTick /\ wall <= MaxWall /\ actV <= MaxAct

-----------------------------------------------------------------------------
\* SAFETY INVARIANTS (none reads `Mut`)

TypeOK ==
    /\ pc \in {"Drain", "Obs1", "Poll", "Obs2", "Update", "Press"}
    /\ tick \in 0..MaxTick /\ wall \in 0..MaxWall /\ actV \in 0..MaxAct
    /\ hw.rel \in BOOLEAN

\* S1.  No press goes out while a lease is live and its owner's own deadline, in
\*      the owner's own clock, has not passed.  A relog and a Stop revoke the
\*      lease, and a death makes the owner drop it, so such a lease is no longer live.
S1 == ~s.lastPress.blocked

\* S2.  After Stop, a session change or a death message, a hold of an old
\*      transaction does not outlive a bounded number of presses.
\*      Stop: Update runs after the slot lock is released, so a sample taken
\*      before the Stop is applied after it, and the next frame clears it: bound 1.
\*      Session change: lib.rs:1542 releases at the drain: bound 0.
\*      Death: the owner drops the lease at its next poll, and the death drain is
\*      itself a wake (dirty): bound 0 on every frame where the owner polls.
\*      `S2DeadAny` counts every frame, also those where the owner cannot poll
\*      (Paused, held, hung): it is NOT an invariant of the product.
S2Stop    == s.pendStop.n <= 1
S2Bnd     == s.pendBnd.n = 0
S2Dead    == s.pendDead.n = 0
S2DeadAny == s.pendDeadAny.n = 0

\* S3.  The host releases without the script only as the logged terminal
\*      failure, never before the owner's deadline plus the margin (1 tick), and
\*      only when the owner's own deadline has passed in the owner's own clock.
S3 == ~s.lastTerm.bad

\* S4.  A lease's identity never carries a released state or a start tick over
\*      to a new transaction.  (a) the host hold's key and the sampled lease's
\*      key name the same transaction; (b) a released hold on a lease that the
\*      sampler still reports belongs to a deadline that has passed.  (b) is
\*      stated where the host's view is fresh, after its Update and before the
\*      next poll (stages Press and Drain): between a poll and the host's
\*      sample the host is one sample behind by construction.
S4a == (hw # NoHold /\ L # NoLease /\ hw.key = L.key) => hw.txn = L.txn
S4b == (pc \in {"Press", "Drain"} /\ hw # NoHold /\ L # NoLease /\ hw.key = L.key /\ hw.rel
        /\ L.kind # "C" /\ ~(L.kind = "Q" /\ wall - L.since >= QW))
       => OwnerPassed(L)

\* S5Click (extra, added in phase 2, NOT one of the frozen S1 to S4/LB).  A compat
\*      journal click never goes out with its hold already past its bound.
S5Click == ~s.clickBad

\* LB (bounded liveness, as a state invariant).  An effective hold on a
\* lease whose stored bound is in ticks does not outlive that bound plus the margin.
LB == (Eff /\ L # NoLease /\ hw.key = L.key /\ L.st.k = "T")
      => tick < L.st.v + Margin + 1

-----------------------------------------------------------------------------
\* TEMPORAL PROPERTY (checked with FairSpec).  Every hold ends: if the owner's
\* clock keeps running, an effective hold is eventually not effective.  An
\* active-clock lease needs the owner's clock to run (a paused owner keeps its
\* hold, docs/api/nav.md:653-656), hence the antecedent <>[]Eligible.
Holding == Eff
HoldEnds == (<>[]Eligible) => [](Holding => <>(~Holding))

=============================================================================
