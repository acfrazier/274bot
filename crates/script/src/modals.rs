//! Rust-owned `Modals.close` / `closeIfOpen`, a [`crate::machine`] family.
//!
//! The close verb, the 3000ms identity wait, the pause/hold freeze and the
//! bool-vs-void result are all here; JavaScript starts one close and maps
//! the settled value onto its declared return. Capture, the identity wait
//! and the window read the isolate scene. A queued close is not accepted.

use crate::machine::{Begin, Cx, Family, Step};
use crate::observed::{self, Scene};
use crate::shim::InteractReq;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Frozen `Modals.close` wait for `main !== before`.
pub const CLOSE_TIMEOUT_MS: u64 = 3_000;

/// The posted facts this module decides from, read from the isolate scene.
struct NativeObservation {
    ingame: bool,
    main_modal_id: i32,
}

impl NativeObservation {
    /// A logout forgets the session: only pages posted since login count.
    fn from_scene(scene: &Scene) -> Self {
        let session = scene.since_login();
        Self {
            ingame: session.ingame().unwrap_or(false),
            main_modal_id: session.main_modal_id().unwrap_or(-1),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Kind {
    Close,
    CloseIfOpen,
}

#[derive(Deserialize)]
pub(crate) struct ModalsArgs {
    kind: Kind,
}

/// One settlement: `close` reports its boolean, `closeIfOpen` stays void.
#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct ModalsDone {
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<bool>,
}

impl ModalsDone {
    fn settled(kind: Kind, result: bool) -> Self {
        Self {
            result: matches!(kind, Kind::Close).then_some(result),
        }
    }
}

impl From<ModalsDone> for Value {
    fn from(done: ModalsDone) -> Self {
        serde_json::to_value(done).unwrap_or(Value::Null)
    }
}

/// One close: the verb went out at begin; each step watches the root.
pub(crate) struct Modals {
    kind: Kind,
    before: i32,
}

impl Family for Modals {
    const NAME: &'static str = "modals";
    /// A newer close replaces the one in flight, as the frozen token bump did.
    const EXCLUSIVE: bool = true;
    type Args = ModalsArgs;
    type Output = ModalsDone;

    fn begin(args: ModalsArgs, cx: &mut Cx<'_>) -> Begin<Self> {
        let obs = observed::with(NativeObservation::from_scene);
        if !obs.ingame {
            return Begin::Done(ModalsDone::settled(args.kind, false));
        }
        if obs.main_modal_id == -1 {
            return Begin::Done(ModalsDone::settled(args.kind, true));
        }
        cx.clock().arm(CLOSE_TIMEOUT_MS);
        cx.emit(InteractReq::CloseModal);
        Begin::Run(Self {
            kind: args.kind,
            before: obs.main_modal_id,
        })
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<ModalsDone> {
        let obs = observed::with(NativeObservation::from_scene);
        if !obs.ingame {
            return Step::Done(ModalsDone::settled(self.kind, false));
        }
        if obs.main_modal_id != self.before {
            return Step::Done(ModalsDone::settled(self.kind, true));
        }
        if cx.clock().bound_reached() {
            return Step::Done(ModalsDone::settled(self.kind, false));
        }
        Step::Wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::{self, Called, Js, Outcome, Pending, Reply, Started, Take};
    use serde_json::json;
    use std::thread;
    use std::time::Duration;

    /// The modals family never calls a script callback.
    struct NoJs;

    impl Js for NoJs {
        fn queue_len(&mut self) -> usize {
            0
        }

        fn call(
            &mut self,
            _hook: Option<&crate::load::callback_v8::HeldCallback>,
            _args: &[Value],
        ) -> Called {
            panic!("the modals family calls no script callback");
        }

        fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
            panic!("the modals family calls no script callback");
        }
    }

    fn observe(ingame: bool, main_modal_id: i32) {
        observed::post(0, |post| {
            post.session(ingame).main_modal_id(main_modal_id);
        });
    }

    fn reset() {
        machine::on_reset();
        observed::on_reset();
    }

    fn tick() {
        machine::step(&mut NoJs);
    }

    fn drain() -> Vec<InteractReq> {
        machine::merge_ops(Vec::new())
    }

    fn start(kind: &str) -> Started {
        machine::start("modals", json!({ "kind": kind }), Vec::new(), 0)
    }

    fn running(started: Started) -> machine::Handle {
        match started {
            Started::Running(handle) => handle,
            other => panic!("expected a running row, got {other:?}"),
        }
    }

    /// The settled envelope exactly as JS receives it.
    fn done(handle: machine::Handle) -> Value {
        match machine::take(handle) {
            Take::Settled(Outcome::Done(value)) => value,
            other => panic!("expected a done outcome, got {other:?}"),
        }
    }

    #[test]
    fn frozen_close_timeout_is_3000ms() {
        assert_eq!(CLOSE_TIMEOUT_MS, 3_000);
    }

    #[test]
    fn absent_close_is_true_without_a_verb() {
        reset();
        observe(true, -1);
        assert_eq!(
            start("close"),
            Started::Settled(Outcome::Done(json!({ "result": true })))
        );
        assert!(drain().is_empty());
    }

    #[test]
    fn present_close_emits_one_close_modal_and_settles_on_identity_change() {
        reset();
        observe(true, 6675);
        let handle = running(start("close"));
        assert_eq!(drain(), vec![InteractReq::CloseModal]);

        observe(true, 6675);
        tick();
        assert_eq!(machine::take(handle), Take::Pending);
        assert!(drain().is_empty(), "one action only");

        observe(true, -1);
        tick();
        assert_eq!(done(handle), json!({ "result": true }));
        assert_eq!(
            machine::take(handle),
            Take::Settled(Outcome::Aborted(machine::AbortReason::Unknown)),
            "a settled close is handed out once"
        );

        reset();
        observe(true, 6675);
        let handle = running(start("close"));
        assert_eq!(drain(), vec![InteractReq::CloseModal]);
        observe(true, 3824);
        tick();
        assert_eq!(
            done(handle),
            json!({ "result": true }),
            "a newer root is a settled close, not another close"
        );
        assert!(drain().is_empty());
    }

    #[test]
    fn unchanged_root_times_out_false_without_reclose() {
        reset();
        observe(true, 6675);
        let handle = running(start("close"));
        assert_eq!(drain(), vec![InteractReq::CloseModal]);
        tick();
        assert_eq!(
            machine::take(handle),
            Take::Pending,
            "unchanged is not success"
        );

        thread::sleep(Duration::from_millis(CLOSE_TIMEOUT_MS + 100));
        tick();
        assert_eq!(done(handle), json!({ "result": false }));
        tick();
        assert!(drain().is_empty(), "timeout must not re-close");
    }

    #[test]
    fn close_if_open_absent_is_void_and_pause_reset_do_not_close() {
        reset();
        observe(true, -1);
        assert_eq!(
            start("closeIfOpen"),
            Started::Settled(Outcome::Done(json!({})))
        );

        observe(true, 6675);
        let handle = running(start("closeIfOpen"));
        assert_eq!(drain(), vec![InteractReq::CloseModal]);
        machine::on_pause();
        observe(true, -1);
        tick();
        assert_eq!(
            machine::take(handle),
            Take::Pending,
            "paused rows do not step"
        );
        machine::on_resume();
        machine::on_hold(true);
        tick();
        assert_eq!(
            machine::take(handle),
            Take::Pending,
            "held rows do not step"
        );
        machine::on_hold(false);
        machine::on_reset();
        assert_eq!(
            machine::take(handle),
            Take::Settled(Outcome::Aborted(machine::AbortReason::Reset))
        );
        tick();
        assert!(drain().is_empty(), "a reset close never re-closes");
    }

    #[test]
    fn a_logout_settles_false_without_faking_a_close() {
        reset();
        observe(true, 6675);
        let handle = running(start("close"));
        assert_eq!(drain(), vec![InteractReq::CloseModal]);
        observe(false, 6675);
        tick();
        assert_eq!(done(handle), json!({ "result": false }));
        assert!(drain().is_empty());
    }
}
