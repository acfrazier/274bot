-------------------------- MODULE HoldHostTrace --------------------------
(***************************************************************************)
(* Trace validation of the HOST half of HoldModel against the events of     *)
(* `ScriptRunPolicy` and `apply_auto_run`, as the host crate's own tests    *)
(* (crates/host/src/lib_tests.rs) and the compat hand-off test in            *)
(* host-play drive them.  Those tests call the host with synthetic waits    *)
(* (key, deadline tick) and no owner, so the owner and the sampler are      *)
(* not in the trace: the sample is an INPUT event.                          *)
(*                                                                          *)
(* One recorded event is one or two TLC steps.                              *)
(*   boundary : HoldModel's Drain with a relog (the host's                  *)
(*              release_at_session_boundary), then a bookkeeping reset.     *)
(*   sync     : bookkeeping.  It sets the runtime generation that the next  *)
(*              observe passes to the model's Update (the model does        *)
(*              sync_runtime and observe_script_wait in one Update).        *)
(*   observe  : an input step (the sample, the frame tick, `opass` TRUE),   *)
(*              then HoldModel's Update.  The model must agree with the     *)
(*              recorded terminal release (`expired`) and with the hold     *)
(*              after the call (`held`, and `rel` when the host records     *)
(*              its hold).                                                  *)
(*   press    : an input step (the modal, from the host's own modal_free),  *)
(*              then HoldModel's Press.  The model must agree with the      *)
(*              recorded `defer`, and a press that the host sent must be    *)
(*              one the model allows (no hold, no modal).  The converse is  *)
(*              not checked here: the host also withholds a press for       *)
(*              reasons outside the model (title, energy, no local player,  *)
(*              run_on: HoldModel A4), which these tests do not record.     *)
(* Input steps and bookkeeping steps are not HoldModel steps.  Update,      *)
(* Press and the boundary Drain are.                                        *)
(*                                                                          *)
(* LIMIT.  A test that calls sync_runtime and observe_script_wait in        *)
(* separate places (`a_stopped_runtime_releases_the_script_wait`) is not    *)
(* a sequence the model's atomic Update can follow; so is a title or        *)
(* energy test (HoldModel A4).  The converter keeps them out or the run     *)
(* shows the rejection.                                                     *)
(***************************************************************************)
EXTENDS HoldModel, Sequences

CONSTANT Trace
VARIABLES hi, hj   \* event index, step within the event

hvars == <<s, hi, hj>>

Ev == Trace[hi]
HLive == hi <= Len(Trace)

HInit == /\ Init /\ hi = 1 /\ hj = 0

\* Move to the next event.
Next1 == /\ hi' = hi + 1 /\ hj' = 0

\* Bookkeeping: a frame whose press the test did not record ends before the next event.
HSkipPress ==
    /\ HLive /\ hj = 0 /\ Ev.ev = "boundary" /\ pc = "Press"
    /\ Set([pc |-> "Drain", act |-> "skip press"]) /\ hi' = hi /\ hj' = hj

HBoundaryDrain ==
    /\ HLive /\ Ev.ev = "boundary" /\ hj = 0 /\ pc = "Drain"
    /\ Drain /\ bnd' /\ ~tickEdge' /\ s'.tick = tick /\ s'.modal = FALSE
    /\ hi' = hi /\ hj' = 1
HBoundaryReset ==
    /\ HLive /\ Ev.ev = "boundary" /\ hj = 1
    /\ Set([pc |-> "Drain", bnd |-> FALSE, held |-> FALSE, act |-> "reset"]) /\ Next1

HSyncStep ==
    /\ HLive /\ Ev.ev = "sync" /\ hj = 0
    /\ Set([smpRt |-> Ev.rt, act |-> "sync(input)"]) /\ Next1

HObserveInput ==
    /\ HLive /\ Ev.ev = "observe" /\ hj = 0
    /\ Set([pc |-> "Update", tick |-> Ev.tick,
            smp |-> IF Ev.has
                    THEN [key |-> <<"M", Ev.key>>, dt |-> Ev.dl, txn |-> Ev.txn, opass |-> TRUE]
                    ELSE NoSmp,
            act |-> "observe(input)"])
    /\ hi' = hi /\ hj' = 1
HObserveUpdate ==
    /\ HLive /\ Ev.ev = "observe" /\ hj = 1
    /\ Update
    /\ (s'.act = "Update:TERMINAL-RELEASE") = Ev.expired
    /\ (hw' # NoHold /\ ~hw'.rel) = Ev.held
    /\ (Ev.has => (hw' # NoHold /\ hw'.rel = Ev.rel))
    /\ Next1

HPressInput ==
    /\ HLive /\ Ev.ev = "press" /\ hj = 0
    /\ Set([pc |-> "Press", modal |-> ~Ev.mfree, act |-> "press(input)"])
    /\ hi' = hi /\ hj' = 1
HPressStep ==
    /\ HLive /\ Ev.ev = "press" /\ hj = 1
    /\ Eff = Ev.defer
    /\ LET fire == ~modal /\ ~Eff
       IN Ev.sent => fire
    /\ Press
    /\ Next1

HNext == HSkipPress \/ HBoundaryDrain \/ HBoundaryReset \/ HSyncStep \/ HObserveInput \/ HObserveUpdate
         \/ HPressInput \/ HPressStep

HSpec == HInit /\ [][HNext]_hvars

\* S4 on the host side: the sample's key and the hold's key name the same transaction.
\* (Checked after an observe input step, before the model's Update.)
S4h == (pc = "Update" /\ smp # NoSmp /\ hw # NoHold /\ hw.key = smp.key) => hw.txn = smp.txn

Done == hi = Len(Trace) + 1
NotDone == ~Done

=============================================================================
