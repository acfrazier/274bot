---------------------------- MODULE HoldTrace ----------------------------
(***************************************************************************)
(* TLA+ trace validation of HoldModel against traces of the product's own   *)
(* tests (the `HoldRig` frame tests in crates/host-play/src/                *)
(* script_progress_tests.rs, which run the real `Host::run_client`, the     *)
(* real `script_observe_cached`, a real slot and a real modal-wait lease).  *)
(*                                                                          *)
(* A trace is a sequence of frames.  For each frame the real run records    *)
(*   - the inputs: PLAYER_INFO edge, the modal, wall time slept, and the    *)
(*     operator's Stop, Pause, Resume;                                      *)
(*   - the owner's lease events (begin with its bound, end, drop);          *)
(*   - the host's outputs: the hold after the update (`eff`), and whether   *)
(*     the press went out.                                                  *)
(* TLC accepts the trace only if the model, started from Init, has a        *)
(* behaviour whose steps are, frame by frame, HoldModel's own actions       *)
(* (TickArrive, Drain, Obs1, Obs2, Update, Press, Pause, Resume, Stop,      *)
(* WallAdv, ActAdv), with the model's hold and press equal to the recorded  *)
(* ones at every frame.  `Accepted` is reached only then.  Bookkeeping      *)
(* steps (finishing a sleep, counting frames) change only the trace         *)
(* variables, so they are stuttering steps of HoldModel.                    *)
(*                                                                          *)
(* The real rig passes no evidence-wake flag, so the recorded owner polls     *)
(* come on tick edges only; the model's Drain also marks a modal change as    *)
(* dirty (a wake poll).  The trace does not forbid that wake: it only needs   *)
(* PollOn to admit a recorded begin.                                          *)
(*                                                                          *)
(* WHAT IS NOT A MODEL STEP.  The owner's Poll is data-driven: the owner's  *)
(* lease events come from the trace (`TPoll`), not from HoldModel's         *)
(* OwnerOpts.  The test machine is not the Dialogue, so the trace checks    *)
(* the host and sampling half (Obs2, Update, Press, Pause, Stop, the        *)
(* clocks) of the model against real code, and checks that the model's      *)
(* poll conditions (`PollOn`) admit the recorded owner events.              *)
(*                                                                          *)
(* TIME.  The real run sleeps in milliseconds.  One model unit is `U` ms    *)
(* (the converter divides).  Each frame may also take up to `E` units more  *)
(* than its sleep (the real frame takes some milliseconds of CPU time).     *)
(*                                                                          *)
(* PRESS.  The model presses on every frame without a hold and a modal      *)
(* (HoldModel A4).  The product presses once, then `run_on` stays true.     *)
(* So a model press without a recorded press is accepted only after a       *)
(* recorded press (`ron`).                                                  *)
(***************************************************************************)
EXTENDS HoldModel, Sequences

CONSTANTS Trace,   \* the recorded frames
          E        \* extra time units per frame (CPU time of the real frame)

VARIABLES ti,      \* index of the current frame
          oi,      \* index of the next operator op of the current frame
          w0, a0,  \* wall time and active clock at the start of the frame
          ron      \* a recorded press has happened

tvars == <<s, ti, oi, w0, a0, ron>>
bk == <<ti, oi, w0, a0, ron>>

Fr == Trace[ti]
Live == ti <= Len(Trace)

TInit ==
    /\ Init
    /\ ti = 1 /\ oi = 1 /\ w0 = 0 /\ a0 = 0 /\ ron = FALSE

\* Operator ops of a frame, in the order of the real probe: sleep, stop, pause, resume.
OpsLeft == Live /\ oi <= Len(Fr.ops)
OpsDone == Live /\ oi > Len(Fr.ops)

TSleepDone ==
    /\ OpsLeft /\ pc = "Drain" /\ Fr.ops[oi] = "sleep"
    /\ wall - w0 >= Fr.sl
    /\ (clkOpen => actV - a0 >= Fr.sl)
    /\ oi' = oi + 1
    /\ UNCHANGED <<s, ti, w0, a0, ron>>

TPause == /\ OpsLeft /\ Fr.ops[oi] = "pause" /\ Pause /\ oi' = oi + 1 /\ UNCHANGED <<ti, w0, a0, ron>>
TResume == /\ OpsLeft /\ Fr.ops[oi] = "resume" /\ Resume /\ oi' = oi + 1 /\ UNCHANGED <<ti, w0, a0, ron>>
TStop == /\ OpsLeft /\ Fr.ops[oi] = "stop" /\ Stop /\ oi' = oi + 1 /\ UNCHANGED <<ti, w0, a0, ron>>

TWall == /\ Live /\ wall + 1 - w0 <= Fr.sl + E /\ WallAdv /\ UNCHANGED bk
TAct == /\ Live /\ actV + 1 - a0 <= Fr.sl + E /\ ActAdv /\ UNCHANGED bk

TTick == /\ OpsDone /\ Fr.edge /\ TickArrive /\ UNCHANGED bk

TDrain ==
    /\ OpsDone /\ (Fr.edge => tp)
    /\ Drain
    /\ tickEdge' = Fr.edge /\ s'.tick = Fr.tick /\ s'.modal = Fr.modal
    /\ ~bnd' /\ held' = held /\ deadMsg' = deadMsg
    /\ UNCHANGED bk

TObs1 == /\ Live /\ Obs1 /\ UNCHANGED bk

\* The owner's lease events of this frame, applied in order to (L, kc, txnN).
RECURSIVE ApplyOwner(_, _)
ApplyOwner(u, evs) ==
    IF evs = <<>> THEN u
    ELSE LET e == Head(evs)
             b == IF e.k = "A" THEN [k |-> "A", v |-> actV + e.v] ELSE [k |-> "T", v |-> tick + e.v]
             lease == [kind |-> "M", key |-> <<"M", u.kc["M"] + 1>>, txn |-> u.txnN + 1, ph |-> "Open",
                       own |-> b, st |-> b, since |-> wall, wdl |-> 0]
         IN ApplyOwner(IF e.ev = "begin"
                       THEN [L |-> lease, kc |-> [u.kc EXCEPT !["M"] = u.kc["M"] + 1], txnN |-> u.txnN + 1]
                       ELSE [u EXCEPT !.L = NoLease],
                       Tail(evs))

\* Data-driven owner poll.  A begin needs a frame on which the model says the
\* owner polls; an end or a drop may also come from a Stop (L is already gone).
TPoll ==
    /\ Live /\ pc = "Poll"
    /\ LET evs == Fr.owner
           hasBegin == \E i \in 1..Len(evs) : evs[i].ev = "begin"
           u == ApplyOwner([L |-> L, kc |-> kc, txnN |-> txnN], evs)
       IN /\ hasBegin => PollOn
          /\ Set([pc |-> "Obs2", polled |-> PollOn, L |-> u.L, kc |-> u.kc, txnN |-> u.txnN,
                  act |-> "Poll(trace)"])
    /\ UNCHANGED bk

TObs2 == /\ Live /\ Obs2 /\ UNCHANGED bk

TUpdate ==
    /\ Live /\ Update
    /\ (hw' # NoHold /\ ~hw'.rel) = Fr.eff
    /\ UNCHANGED bk

TPress ==
    /\ Live /\ pc = "Press"
    /\ LET fire == ~modal /\ ~Eff
       IN /\ (Fr.press => fire)
          /\ ((fire /\ ~Fr.press) => ron)
          /\ ron' = (ron \/ Fr.press)
    /\ Press
    /\ ti' = ti + 1 /\ oi' = 1 /\ w0' = wall /\ a0' = actV

TNext ==
    \/ TSleepDone \/ TPause \/ TResume \/ TStop
    \/ TWall \/ TAct \/ TTick \/ TDrain \/ TObs1 \/ TPoll \/ TObs2 \/ TUpdate \/ TPress

TSpec == TInit /\ [][TNext]_tvars

\* The spec's own Next is the abstract step: every step of TSpec is a step of
\* HoldModel's `Next`, or changes only the trace variables, except TPoll (see above).
Done == ti = Len(Trace) + 1
NotDone == ~Done

=============================================================================
