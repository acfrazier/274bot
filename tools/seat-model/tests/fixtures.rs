use seat_model::{
    Bounds, Class, Id, MAX_START_CHARGE, Origin, Outcome, Refusal, Rules, START_METADATA,
    SessionFamily, State, Step, Trace, Trigger, run_trace,
};

struct Fixture {
    bounds: Bounds,
    builder: State,
    steps: Vec<Step>,
}

impl Fixture {
    fn new(bounds: Bounds) -> Self {
        Self {
            bounds,
            builder: State::new(Rules::Chosen, bounds),
            steps: Vec::new(),
        }
    }

    fn apply(&mut self, step: Step) {
        if let Err(error) = self.builder.apply(&step) {
            panic!("invalid fixture step {step:?}: {error}");
        }
        self.steps.push(step);
    }

    fn start(
        &mut self,
        parent: Option<Id>,
        local_parent: Option<Id>,
        class: Class,
        group: u8,
        bytes: usize,
        hooks: u8,
    ) -> Id {
        let id = self.builder.next_id();
        self.apply(Step::Start {
            parent,
            local_parent,
            class,
            group,
            bytes,
            hooks,
        });
        id
    }

    fn mirror(&mut self, ancestor: Option<Id>, parent: Option<Id>) -> Id {
        let id = self.builder.next_id();
        self.apply(Step::Mirror { ancestor, parent });
        id
    }

    fn admitted(&mut self, parent: Option<Id>, class: Class, group: u8) -> Id {
        let id = self.start(parent, None, class, group, 0, 0);
        self.apply(Step::Admit(id));
        id
    }

    fn child(&mut self, parent: Id, group: u8) -> Id {
        self.admitted(Some(parent), Class::Capability, group)
    }

    fn trace(&self, rules: Rules) -> Trace {
        run_trace(rules, self.bounds, &self.steps)
    }
}

fn expect_r5_failure(fixture: &Fixture, name: &str) -> Trace {
    let trace = fixture.trace(Rules::R5);
    assert!(
        trace.failure.is_some(),
        "expected an R5 invariant failure named {name}\n{}",
        trace.text
    );
    let failure = trace.failure.as_ref().expect("failure was asserted above");
    assert_eq!(failure.name, name, "{}", trace.text);
    trace
}

fn expect_chosen_pass(fixture: &Fixture) -> Trace {
    let trace = fixture.trace(Rules::Chosen);
    assert!(
        trace.failure.is_none(),
        "unexpected chosen-rule failure\n{}",
        trace.text
    );
    trace
}

fn v5_01_fixture() -> (Fixture, Id, Id) {
    let mut fixture = Fixture::new(Bounds::production());
    let session = fixture.admitted(None, Class::Session(SessionFamily::Gather), 1);
    let child = fixture.child(session, 2);
    fixture.apply(Step::Emit {
        of: Some(child),
        release: false,
    });
    fixture.apply(Step::Complete(child));
    fixture.apply(Step::Take);
    fixture.apply(Step::Dispatch);
    (fixture, session, child)
}

#[test]
fn r5_counterexample_v5_01() {
    let (fixture, session, child) = v5_01_fixture();
    let trace = expect_r5_failure(&fixture, "accepted-emission-authority");
    assert!(trace.final_state.entry(session).is_some());
    assert_eq!(trace.final_state.outcome(child), Some(Outcome::Done));
    assert_eq!(trace.final_state.dispatched(), 0);
}

#[test]
fn chosen_counterexample_v5_01() {
    let (fixture, session, child) = v5_01_fixture();
    let trace = expect_chosen_pass(&fixture);
    assert!(trace.final_state.entry(session).is_some());
    assert_eq!(trace.final_state.outcome(child), Some(Outcome::Done));
    assert_eq!(trace.final_state.dispatched(), 1);
}

fn branching_promotion_fixture(root_count: usize) -> (Fixture, Vec<Id>, Id, Id, Id) {
    let mut fixture = Fixture::new(Bounds::production());
    let mut roots = Vec::with_capacity(root_count);
    for index in 0..root_count {
        roots.push(fixture.admitted(None, Class::Capability, ((index + 1) * 16) as u8));
    }

    let root = roots[0];
    let base = 16;
    let parent = fixture.child(root, base + 1);
    let adopted = fixture.child(parent, base + 2);
    fixture.apply(Step::Complete(parent));
    let sibling = fixture.child(root, base + 3);
    fixture.apply(Step::Complete(root));
    (fixture, roots, root, adopted, sibling)
}

#[test]
fn r5_counterexample_v5_02() {
    let (fixture, _, _, adopted, sibling) = branching_promotion_fixture(4);
    let trace = expect_r5_failure(&fixture, "root-bound");
    assert_eq!(trace.final_state.roots(), 5);
    assert_eq!(
        trace.final_state.entry(adopted).map(|entry| entry.parent),
        Some(None)
    );
    assert_eq!(
        trace.final_state.entry(sibling).map(|entry| entry.parent),
        Some(None)
    );
}

#[test]
fn chosen_counterexample_v5_02() {
    let (at_capacity, roots, root, adopted, sibling) = branching_promotion_fixture(4);
    let trace = expect_chosen_pass(&at_capacity);
    assert_eq!(trace.final_state.roots(), 4);
    assert_eq!(trace.final_state.leaves(), 4);
    assert_eq!(trace.final_state.entry(root), None);
    assert_eq!(
        trace.final_state.entry(adopted).map(|entry| entry.parent),
        Some(None)
    );
    assert_eq!(
        trace.final_state.entry(adopted).map(|entry| entry.origin),
        Some(Origin::Adopted)
    );
    assert_eq!(trace.final_state.entry(sibling), None);
    assert_eq!(
        trace.final_state.outcome(sibling),
        Some(Outcome::Refused(Refusal::Busy))
    );
    assert!(
        roots
            .iter()
            .skip(1)
            .all(|id| trace.final_state.entry(*id).is_some())
    );

    let (with_reserved_root, roots, root, adopted, sibling) = branching_promotion_fixture(3);
    let trace = expect_chosen_pass(&with_reserved_root);
    assert_eq!(trace.final_state.roots(), 4);
    assert_eq!(trace.final_state.leaves(), 4);
    assert_eq!(
        trace.final_state.entry(adopted).map(|entry| entry.parent),
        Some(None)
    );
    assert_eq!(
        trace.final_state.entry(adopted).map(|entry| entry.origin),
        Some(Origin::Adopted)
    );
    assert_eq!(
        trace.final_state.entry(sibling).map(|entry| entry.parent),
        Some(None)
    );
    assert_eq!(
        trace.final_state.entry(sibling).map(|entry| entry.origin),
        Some(Origin::Started)
    );
    assert!(trace.final_state.entry(root).is_none());
    assert!(
        roots
            .iter()
            .skip(1)
            .all(|id| trace.final_state.entry(*id).is_some())
    );
}

fn hybrid_completion_fixture(root_parent: bool) -> (Fixture, Id, Id, Id) {
    let mut fixture = Fixture::new(Bounds::production());
    if root_parent {
        let completing = fixture.admitted(None, Class::Capability, 3);
        let mirror = fixture.mirror(Some(completing), None);
        fixture.apply(Step::Complete(completing));
        fixture.apply(Step::Page);
        fixture.apply(Step::Watermark);
        return (fixture, completing, completing, mirror);
    }

    let session = fixture.admitted(None, Class::Session(SessionFamily::Gather), 1);
    let completing = fixture.child(session, 2);
    let mirror = fixture.mirror(Some(completing), None);
    fixture.apply(Step::Complete(completing));
    fixture.apply(Step::Page);
    fixture.apply(Step::Watermark);
    (fixture, session, completing, mirror)
}

fn pending_cancel_before_admit_fixture() -> (Fixture, Id, Id) {
    let mut fixture = Fixture::new(Bounds::production());
    let local_parent = fixture.mirror(None, None);
    let child = fixture.start(None, Some(local_parent), Class::Capability, 5, 0, 0);
    fixture.apply(Step::Revoke {
        of: local_parent,
        trigger: Trigger::HostRevoke,
    });
    fixture.apply(Step::Batch);
    fixture.apply(Step::Page);
    fixture.apply(Step::Watermark);
    (fixture, local_parent, child)
}

fn deferred_cancel_fixture() -> (Fixture, Id, Id, Id) {
    let mut bounds = Bounds::production();
    bounds.roots = 2;
    bounds.live = 2;
    bounds.credits = 2;
    let mut fixture = Fixture::new(bounds);
    let first = fixture.admitted(None, Class::Capability, 1);
    let second = fixture.admitted(None, Class::Capability, 2);
    let local_parent = fixture.mirror(None, None);
    let child = fixture.start(None, Some(local_parent), Class::Capability, 3, 0, 0);
    fixture.apply(Step::Admit(child));
    fixture.apply(Step::Revoke {
        of: local_parent,
        trigger: Trigger::HostRevoke,
    });
    fixture.apply(Step::Revoke {
        of: second,
        trigger: Trigger::HostRevoke,
    });
    fixture.apply(Step::Page);
    fixture.apply(Step::Watermark);
    fixture.apply(Step::Batch);
    fixture.apply(Step::Page);
    fixture.apply(Step::Watermark);
    (fixture, first, local_parent, child)
}

fn lost_admitted_cancel_fixture() -> (Fixture, Id, Id) {
    let mut fixture = Fixture::new(Bounds::production());
    let local_parent = fixture.mirror(None, None);
    let child = fixture.start(None, Some(local_parent), Class::Capability, 7, 0, 0);
    fixture.apply(Step::Admit(child));
    fixture.apply(Step::Revoke {
        of: local_parent,
        trigger: Trigger::HostRevoke,
    });
    fixture.apply(Step::Batch);
    fixture.apply(Step::Page);
    fixture.apply(Step::Watermark);
    (fixture, local_parent, child)
}

#[test]
fn r5_counterexample_v5_03() {
    let (fixture, session, completing, mirror) = hybrid_completion_fixture(false);
    let trace = expect_r5_failure(&fixture, "hybrid-ancestry-custody");
    assert!(trace.final_state.entry(session).is_some());
    assert_eq!(trace.final_state.entry(completing), None);
    assert_eq!(
        trace.final_state.mirror_ancestor(mirror),
        Some(Some(completing))
    );

    let (fixture, local_parent, child) = pending_cancel_before_admit_fixture();
    let trace = expect_r5_failure(&fixture, "cancel-before-start");
    assert_eq!(trace.final_state.entry(local_parent), None);
    assert!(trace.final_state.entry(child).is_some());
    assert_eq!(trace.final_state.outcome(child), None);

    let (fixture, first, local_parent, child) = deferred_cancel_fixture();
    let trace = expect_r5_failure(&fixture, "admission-progress");
    assert!(trace.final_state.entry(first).is_some());
    assert_eq!(trace.final_state.entry(local_parent), None);
    assert_eq!(trace.final_state.entry(child), None);
    assert_eq!(trace.final_state.pending(), 1);
    assert_eq!(trace.final_state.outcome(child), None);

    let (fixture, _, child) = lost_admitted_cancel_fixture();
    let trace = fixture.trace(Rules::R5);
    assert!(
        trace.failure.is_none(),
        "unexpected R5 lost-admission failure\n{}",
        trace.text
    );
    assert_eq!(
        trace.final_state.outcome(child),
        Some(Outcome::Aborted(Trigger::ParentAborted))
    );
    assert_eq!(trace.final_state.settlements(child), 1);
}

#[test]
fn chosen_counterexample_v5_03() {
    let (fixture, session, completing, mirror) = hybrid_completion_fixture(false);
    let trace = expect_chosen_pass(&fixture);
    assert!(trace.final_state.entry(session).is_some());
    assert_eq!(trace.final_state.entry(completing), None);
    assert_eq!(
        trace.final_state.mirror_ancestor(mirror),
        Some(Some(session))
    );

    let (fixture, root, _, mirror) = hybrid_completion_fixture(true);
    let trace = expect_chosen_pass(&fixture);
    assert_eq!(trace.final_state.entry(root), None);
    assert_eq!(trace.final_state.mirror_ancestor(mirror), Some(None));

    let (fixture, local_parent, child) = pending_cancel_before_admit_fixture();
    let trace = expect_chosen_pass(&fixture);
    assert_eq!(trace.final_state.entry(local_parent), None);
    assert_eq!(trace.final_state.entry(child), None);
    assert_eq!(
        trace.final_state.outcome(child),
        Some(Outcome::Aborted(Trigger::ParentAborted))
    );
    assert_eq!(trace.final_state.settlements(child), 1);

    let (fixture, first, local_parent, child) = deferred_cancel_fixture();
    let trace = expect_chosen_pass(&fixture);
    assert!(trace.final_state.entry(first).is_some());
    assert_eq!(trace.final_state.entry(local_parent), None);
    assert_eq!(trace.final_state.entry(child), None);
    assert_eq!(trace.final_state.pending(), 0);
    assert_eq!(
        trace.final_state.outcome(child),
        Some(Outcome::Aborted(Trigger::ParentAborted))
    );
    assert_eq!(trace.final_state.settlements(child), 1);

    let (fixture, local_parent, child) = lost_admitted_cancel_fixture();
    let trace = expect_chosen_pass(&fixture);
    assert_eq!(trace.final_state.entry(local_parent), None);
    assert_eq!(trace.final_state.entry(child), None);
    assert_eq!(
        trace.final_state.outcome(child),
        Some(Outcome::Aborted(Trigger::ParentAborted))
    );
    assert_eq!(trace.final_state.settlements(child), 1);
}

fn full_replacement_fixture() -> (Fixture, Id, Id, Id) {
    let mut fixture = Fixture::new(Bounds::production());
    let mut roots = Vec::with_capacity(4);
    for index in 0..4 {
        roots.push(fixture.admitted(None, Class::Capability, ((index + 1) * 16) as u8));
    }

    let mut first_chain = Vec::with_capacity(3);
    for (index, root) in roots.iter().copied().enumerate() {
        let base = ((index + 1) * 16) as u8;
        let mut parent = root;
        for depth in 1..=3 {
            parent = fixture.child(parent, base + depth as u8);
            if index == 0 {
                first_chain.push(parent);
            }
        }
    }
    let replacement_parent = first_chain[1];
    let victim = first_chain[2];
    let replacement = fixture.start(
        Some(replacement_parent),
        None,
        Class::Capability,
        16 + 3,
        0,
        0,
    );
    fixture.apply(Step::Admit(replacement));
    (fixture, replacement_parent, victim, replacement)
}

#[test]
fn r5_counterexample_v5_04() {
    let (fixture, parent, victim, replacement) = full_replacement_fixture();
    let trace = expect_r5_failure(&fixture, "reusable-capacity");
    assert_eq!(trace.final_state.live(), 16);
    assert!(trace.final_state.entry(parent).is_some());
    assert!(trace.final_state.entry(victim).is_some());
    assert_eq!(trace.final_state.entry(replacement), None);
    assert_eq!(
        trace.final_state.outcome(replacement),
        Some(Outcome::Refused(Refusal::Busy))
    );
}

#[test]
fn chosen_counterexample_v5_04() {
    let (fixture, parent, victim, replacement) = full_replacement_fixture();
    let trace = expect_chosen_pass(&fixture);
    assert_eq!(trace.final_state.live(), 16);
    assert_eq!(trace.final_state.roots(), 4);
    assert_eq!(trace.final_state.leaves(), 4);
    assert!(trace.final_state.entry(parent).is_some());
    assert_eq!(trace.final_state.entry(victim), None);
    assert_eq!(
        trace.final_state.outcome(victim),
        Some(Outcome::Aborted(Trigger::Superseded))
    );
    assert_eq!(
        trace
            .final_state
            .entry(replacement)
            .map(|entry| entry.parent),
        Some(Some(parent))
    );
    assert_eq!(trace.final_state.outcome(replacement), None);
}

fn below_floor_completion_fixture() -> (Fixture, Id) {
    let mut fixture = Fixture::new(Bounds::production());
    let owner = fixture.admitted(None, Class::Capability, 0);
    for index in 0..65 {
        let child = fixture.child(owner, (index + 1) as u8);
        fixture.apply(Step::Revoke {
            of: child,
            trigger: Trigger::HostRevoke,
        });
        fixture.apply(Step::Page);
        fixture.apply(Step::Watermark);
    }
    fixture.apply(Step::Emit {
        of: Some(owner),
        release: false,
    });
    fixture.apply(Step::Complete(owner));
    fixture.apply(Step::Take);
    fixture.apply(Step::Dispatch);
    (fixture, owner)
}

#[test]
fn r5_counterexample_v5_05() {
    let (fixture, owner) = below_floor_completion_fixture();
    let trace = expect_r5_failure(&fixture, "accepted-emission-authority");
    assert!(trace.final_state.history_floor() > owner);
    assert_eq!(trace.final_state.outcome(owner), Some(Outcome::Done));
    assert_eq!(trace.final_state.dispatched(), 0);
}

#[test]
fn chosen_counterexample_v5_05() {
    let (fixture, owner) = below_floor_completion_fixture();
    let trace = expect_chosen_pass(&fixture);
    assert!(trace.final_state.history_floor() > owner);
    assert_eq!(trace.final_state.outcome(owner), Some(Outcome::Done));
    assert_eq!(trace.final_state.dispatched(), 1);
}

fn oversized_non_gather_fixture() -> (Fixture, Id) {
    let mut fixture = Fixture::new(Bounds::production());
    let bytes = MAX_START_CHARGE - START_METADATA + 1;
    let id = fixture.start(None, None, Class::Capability, 41, bytes, 0);
    (fixture, id)
}

fn overloaded_legacy_release_fixture() -> Fixture {
    let mut fixture = Fixture::new(Bounds::production());
    for _ in 0..300 {
        fixture.apply(Step::Emit {
            of: None,
            release: false,
        });
    }
    fixture.apply(Step::Emit {
        of: None,
        release: true,
    });
    fixture.apply(Step::Take);
    fixture.apply(Step::Dispatch);
    fixture
}

#[test]
fn r5_counterexample_v5_06() {
    let (oversized, id) = oversized_non_gather_fixture();
    let trace = expect_r5_failure(&oversized, "pending-byte-custody");
    assert_eq!(trace.final_state.pending(), 1);
    assert_eq!(trace.final_state.outcome(id), None);

    let release = overloaded_legacy_release_fixture();
    let trace = expect_r5_failure(&release, "encoder-release");
    assert_eq!(trace.final_state.release_dispatched(), 0);
}

#[test]
fn chosen_counterexample_v5_06() {
    let (oversized, id) = oversized_non_gather_fixture();
    let trace = expect_chosen_pass(&oversized);
    assert_eq!(trace.final_state.pending(), 0);
    assert_eq!(
        trace.final_state.outcome(id),
        Some(Outcome::Refused(Refusal::ArgsTooLarge))
    );

    let release = overloaded_legacy_release_fixture();
    let trace = expect_chosen_pass(&release);
    assert_eq!(trace.final_state.release_dispatched(), 1);
    assert_eq!(trace.final_state.dispatched(), 141);
}

fn tree_fixture(root_class: Class) -> (Fixture, Id, Id, Id) {
    let mut fixture = Fixture::new(Bounds::production());
    let root = fixture.admitted(None, root_class, 80);
    let child = fixture.child(root, 81);
    let leaf = fixture.child(child, 82);
    (fixture, root, child, leaf)
}

#[test]
fn chosen_r5_7_leaf_first_boundaries() {
    for (trigger, root_outcome) in [
        (Trigger::Superseded, Outcome::Aborted(Trigger::Superseded)),
        (Trigger::HostRevoke, Outcome::Aborted(Trigger::HostRevoke)),
        (
            Trigger::ParentAborted,
            Outcome::Aborted(Trigger::ParentAborted),
        ),
    ] {
        let (mut fixture, root, child, leaf) = tree_fixture(Class::Capability);
        fixture.apply(Step::Revoke { of: root, trigger });
        let trace = expect_chosen_pass(&fixture);
        assert_eq!(trace.final_state.terminal_order(), vec![leaf, child, root]);
        assert_eq!(
            trace.final_state.outcome(leaf),
            Some(Outcome::Aborted(Trigger::ParentAborted))
        );
        assert_eq!(
            trace.final_state.outcome(child),
            Some(Outcome::Aborted(Trigger::ParentAborted))
        );
        assert_eq!(trace.final_state.outcome(root), Some(root_outcome));
    }

    let (mut manual_walk, root, child, leaf) = tree_fixture(Class::Capability);
    manual_walk.apply(Step::Revoke {
        of: root,
        trigger: Trigger::ManualWalk,
    });
    let trace = expect_chosen_pass(&manual_walk);
    assert_eq!(trace.final_state.terminal_order(), vec![leaf, child, root]);
    assert_eq!(
        trace.final_state.outcome(leaf),
        Some(Outcome::Aborted(Trigger::ParentAborted))
    );
    assert_eq!(
        trace.final_state.outcome(child),
        Some(Outcome::Aborted(Trigger::ParentAborted))
    );
    assert_eq!(trace.final_state.outcome(root), Some(Outcome::Done));

    let (mut manual_combat, root, child, leaf) =
        tree_fixture(Class::Session(SessionFamily::Gather));
    manual_combat.apply(Step::Revoke {
        of: root,
        trigger: Trigger::ManualCombat,
    });
    let trace = expect_chosen_pass(&manual_combat);
    assert_eq!(trace.final_state.terminal_order(), vec![leaf, child, root]);
    assert_eq!(
        trace.final_state.outcome(leaf),
        Some(Outcome::Aborted(Trigger::ParentAborted))
    );
    assert_eq!(
        trace.final_state.outcome(child),
        Some(Outcome::Aborted(Trigger::ParentAborted))
    );
    assert_eq!(
        trace.final_state.outcome(root),
        Some(Outcome::Aborted(Trigger::ManualCombat))
    );

    let (mut cancel, root, child, leaf) = tree_fixture(Class::Capability);
    cancel.apply(Step::Cancel {
        of: root,
        throw: false,
    });
    cancel.apply(Step::Controls);
    let trace = expect_chosen_pass(&cancel);
    assert_eq!(trace.final_state.terminal_order(), vec![leaf, child, root]);
    assert!(
        [leaf, child, root]
            .iter()
            .all(|id| trace.final_state.outcome(*id)
                == Some(Outcome::Aborted(Trigger::ParentAborted)))
    );

    for trigger in [Trigger::GatherSignal, Trigger::Pause] {
        let (mut held, root, _, _) = tree_fixture(Class::Capability);
        held.apply(Step::Revoke { of: root, trigger });
        let trace = expect_chosen_pass(&held);
        assert_eq!(trace.final_state.live(), 3);
        assert!(trace.final_state.terminal_order().is_empty());
    }

    for (throw, expected) in [(false, Outcome::Done), (true, Outcome::Failed)] {
        let mut fixture = Fixture::new(Bounds::production());
        let root = fixture.admitted(None, Class::Capability, 90);
        let parent = fixture.child(root, 91);
        let adopted = fixture.child(parent, 92);
        fixture.apply(if throw {
            Step::Throw(parent)
        } else {
            Step::Complete(parent)
        });
        let trace = expect_chosen_pass(&fixture);
        assert_eq!(trace.final_state.outcome(parent), Some(expected));
        assert_eq!(
            trace.final_state.entry(adopted).map(|entry| entry.parent),
            Some(Some(root))
        );
        assert_eq!(
            trace.final_state.entry(adopted).map(|entry| entry.origin),
            Some(Origin::Adopted)
        );
        assert_eq!(trace.final_state.terminal_order(), vec![parent]);
    }
}

#[test]
fn chosen_page_and_carry_retirement() {
    let mut decision = Fixture::new(Bounds::production());
    let request = decision.admitted(None, Class::Capability, 100);
    decision.apply(Step::Prompt(request));
    decision.apply(Step::Decide {
        of: request,
        complete: true,
    });
    decision.apply(Step::Controls);
    decision.apply(Step::Decide {
        of: request,
        complete: false,
    });
    decision.apply(Step::Controls);

    let in_flight = expect_chosen_pass(&decision);
    assert_eq!(in_flight.final_state.outcome(request), Some(Outcome::Done));
    assert_eq!(in_flight.final_state.settlements(request), 0);
    assert_eq!(in_flight.final_state.carry_len(), 1);
    assert_eq!(in_flight.final_state.late_controls(), 1);
    assert_eq!(in_flight.final_state.retained(), 1);

    decision.apply(Step::Page);
    decision.apply(Step::Watermark);
    let retired = expect_chosen_pass(&decision);
    assert_eq!(retired.final_state.outcome(request), Some(Outcome::Done));
    assert_eq!(retired.final_state.settlements(request), 1);
    assert_eq!(retired.final_state.carry_len(), 0);
    assert_eq!(retired.final_state.late_controls(), 1);
    assert_eq!(retired.final_state.retained(), 0);

    let mut cancellation = Fixture::new(Bounds::production());
    let request = cancellation.admitted(None, Class::Capability, 101);
    let detached = cancellation.child(request, 102);
    cancellation.apply(Step::Prompt(request));
    cancellation.apply(Step::Decide {
        of: request,
        complete: false,
    });
    cancellation.apply(Step::Cancel {
        of: request,
        throw: true,
    });
    cancellation.apply(Step::Controls);
    let before_page = expect_chosen_pass(&cancellation);
    assert_eq!(
        before_page.final_state.outcome(request),
        Some(Outcome::Failed)
    );
    assert_eq!(before_page.final_state.carry_len(), 1);
    assert_eq!(
        before_page
            .final_state
            .entry(detached)
            .map(|entry| entry.parent),
        Some(None)
    );

    cancellation.apply(Step::Page);
    cancellation.apply(Step::Watermark);
    let after_page = expect_chosen_pass(&cancellation);
    assert_eq!(
        after_page.final_state.outcome(request),
        Some(Outcome::Failed)
    );
    assert_eq!(after_page.final_state.settlements(request), 1);
    assert_eq!(after_page.final_state.carry_len(), 0);
    assert_eq!(
        after_page
            .final_state
            .entry(detached)
            .map(|entry| entry.parent),
        Some(None)
    );
}

#[test]
fn chosen_kept_reset_nonkept_reset_and_stop() {
    let mut kept = Fixture::new(Bounds::production());
    let request = kept.admitted(None, Class::Session(SessionFamily::Gather), 110);
    kept.apply(Step::Prompt(request));
    kept.apply(Step::Decide {
        of: request,
        complete: false,
    });
    kept.apply(Step::Emit {
        of: Some(request),
        release: false,
    });
    kept.apply(Step::Take);
    kept.apply(Step::Reconnect { keep: true });
    let trace = expect_chosen_pass(&kept);
    assert_eq!(trace.final_state.live(), 1);
    assert_eq!(trace.final_state.pending(), 1);
    assert_eq!(trace.final_state.carry_len(), 1);
    assert!(trace.final_state.entry(request).is_some());
    assert_eq!(trace.final_state.outcome(request), None);
    assert_eq!(trace.final_state.dispatched(), 0);

    let mut reset = Fixture::new(Bounds::production());
    let root = reset.admitted(None, Class::Session(SessionFamily::Gather), 111);
    let child = reset.child(root, 112);
    reset.apply(Step::Decide {
        of: root,
        complete: false,
    });
    reset.apply(Step::Emit {
        of: Some(root),
        release: false,
    });
    reset.apply(Step::Take);
    let next_id = reset.builder.next_id();
    reset.apply(Step::Reconnect { keep: false });
    let trace = expect_chosen_pass(&reset);
    assert_eq!(trace.final_state.live(), 0);
    assert_eq!(trace.final_state.retained(), 0);
    assert_eq!(trace.final_state.pending(), 0);
    assert_eq!(trace.final_state.carry_len(), 0);
    assert_eq!(
        trace.final_state.outcome(root),
        Some(Outcome::Aborted(Trigger::Reset))
    );
    assert_eq!(
        trace.final_state.outcome(child),
        Some(Outcome::Aborted(Trigger::Reset))
    );
    assert_eq!(trace.final_state.next_id(), next_id);

    let mut stopped = Fixture::new(Bounds::production());
    let root = stopped.admitted(None, Class::Session(SessionFamily::Gather), 113);
    let child = stopped.child(root, 114);
    stopped.apply(Step::Emit {
        of: Some(root),
        release: false,
    });
    stopped.apply(Step::Stop);
    let trace = expect_chosen_pass(&stopped);
    assert_eq!(trace.final_state.live(), 0);
    assert_eq!(trace.final_state.retained(), 0);
    assert_eq!(trace.final_state.pending(), 0);
    assert_eq!(trace.final_state.carry_len(), 0);
    assert_eq!(trace.final_state.outcome(root), None);
    assert_eq!(trace.final_state.outcome(child), None);
    assert_eq!(trace.final_state.settlements(root), 0);
    assert_eq!(trace.final_state.settlements(child), 0);
    assert_eq!(trace.final_state.dispatched(), 0);
}

#[test]
fn r5_and_chosen_pre_cutover_completing_child_authority() {
    let mut fixture = Fixture::new(Bounds::production());
    let session = fixture.admitted(None, Class::Session(SessionFamily::Gather), 1);
    let child = fixture.mirror(Some(session), None);
    fixture.apply(Step::Emit {
        of: Some(child),
        release: false,
    });
    fixture.apply(Step::Complete(child));
    fixture.apply(Step::Take);
    fixture.apply(Step::Dispatch);
    expect_r5_failure(&fixture, "accepted-emission-authority");
    let trace = expect_chosen_pass(&fixture);
    assert_eq!(trace.final_state.dispatched(), 1);
    assert_eq!(trace.final_state.outcome(child), Some(Outcome::Done));
}

#[test]
fn chosen_full_credit_deferral_watermark_and_cancellation() {
    let mut fixture = Fixture::new(Bounds::production());
    let mut ids = Vec::new();
    for _ in 0..32 {
        ids.push(fixture.admitted(None, Class::Capability, 1));
    }
    assert_eq!(
        (fixture.builder.live(), fixture.builder.retained()),
        (1, 31)
    );
    fixture.apply(Step::Page);
    let child = fixture.start(Some(ids[0]), None, Class::Capability, 2, 0, 0);
    fixture.apply(Step::Admit(child));
    let full = expect_chosen_pass(&fixture);
    assert_eq!(
        (
            full.final_state.live(),
            full.final_state.retained(),
            full.final_state.pending()
        ),
        (1, 31, 1)
    );
    assert_eq!(full.final_state.outcome(child), None);
    fixture.apply(Step::Cancel {
        of: ids[31],
        throw: false,
    });
    fixture.apply(Step::Controls);
    let cancelled = expect_chosen_pass(&fixture);
    assert_eq!(
        (
            cancelled.final_state.live(),
            cancelled.final_state.retained()
        ),
        (0, 32)
    );
    fixture.apply(Step::Watermark);
    fixture.apply(Step::Admit(child));
    let resumed = expect_chosen_pass(&fixture);
    assert_eq!(
        resumed.final_state.outcome(child),
        Some(Outcome::Refused(Refusal::ParentNotLive))
    );
    assert_eq!(resumed.final_state.retained(), 2);

    let mut local_limit = Fixture::new(Bounds::production());
    for _ in 0..33 {
        local_limit.start(None, None, Class::Capability, 1, 0, 0);
    }
    let trace = expect_chosen_pass(&local_limit);
    assert_eq!(trace.final_state.pending(), 32);
    assert_eq!(trace.final_state.retained(), 0);
    assert_eq!(
        trace.final_state.outcome(33),
        Some(Outcome::Refused(Refusal::Busy))
    );
}

#[test]
fn chosen_four_walk_heads_publish_sixteen_takeover_results() {
    let mut fixture = Fixture::new(Bounds::production());
    let mut roots = Vec::new();
    let mut chains = Vec::new();
    for group in [10, 20, 30, 40] {
        roots.push(fixture.admitted(None, Class::Capability, group));
    }
    for (index, root) in roots.iter().copied().enumerate() {
        let mut chain = vec![root];
        let mut parent = root;
        for depth in 1..4 {
            parent = fixture.child(parent, ((index + 1) * 10 + depth) as u8);
            chain.push(parent);
        }
        chains.push(chain);
    }
    let mut expected = Vec::new();
    for (root, chain) in roots.iter().copied().zip(&chains) {
        fixture.apply(Step::Emit {
            of: Some(root),
            release: true,
        });
        fixture.apply(Step::Revoke {
            of: root,
            trigger: Trigger::ManualWalk,
        });
        expected.extend(chain.iter().rev().copied());
    }
    let trace = expect_chosen_pass(&fixture);
    assert_eq!(trace.final_state.terminal_order(), expected);
    assert_eq!(
        (trace.final_state.live(), trace.final_state.retained()),
        (0, 16)
    );
    for chain in &chains {
        assert_eq!(trace.final_state.outcome(chain[0]), Some(Outcome::Done));
        for id in &chain[1..] {
            assert_eq!(
                trace.final_state.outcome(*id),
                Some(Outcome::Aborted(Trigger::ParentAborted))
            );
        }
    }
    fixture.apply(Step::Take);
    fixture.apply(Step::Dispatch);
    fixture.apply(Step::Page);
    fixture.apply(Step::Page);
    fixture.apply(Step::Watermark);
    let trace = expect_chosen_pass(&fixture);
    assert_eq!(trace.final_state.release_dispatched(), 4);
    assert_eq!(trace.final_state.retained(), 0);
    for id in expected {
        assert_eq!(trace.final_state.settlements(id), 1);
    }
}

#[test]
fn chosen_bounds_inclusive_ancestry_and_detached_chain() {
    let mut depth = Fixture::new(Bounds::production());
    let root = depth.admitted(None, Class::Capability, 1);
    let p2 = depth.child(root, 2);
    let p3 = depth.child(p2, 3);
    let leaf = depth.child(p3, 4);
    let too_deep = depth.child(leaf, 5);
    let trace = expect_chosen_pass(&depth);
    assert_eq!(
        trace.final_state.outcome(too_deep),
        Some(Outcome::Refused(Refusal::LineageDepth))
    );
    for (complete, parent) in [(p3, Some(p2)), (p2, Some(root)), (root, None)] {
        depth.apply(Step::Complete(complete));
        let trace = expect_chosen_pass(&depth);
        assert_eq!(
            trace.final_state.entry(leaf).map(|e| e.parent),
            Some(parent)
        );
        assert_eq!(trace.final_state.outcome(leaf), None);
    }
    let mut ancestry = Fixture::new(Bounds::production());
    let root = ancestry.admitted(None, Class::Capability, 1);
    let forbidden = ancestry.child(root, 1);
    let child = ancestry.child(root, 2);
    let second = ancestry.child(root, 3);
    let trace = expect_chosen_pass(&ancestry);
    assert_eq!(
        trace.final_state.outcome(forbidden),
        Some(Outcome::Refused(Refusal::Busy))
    );
    assert_eq!(
        trace.final_state.outcome(second),
        Some(Outcome::Refused(Refusal::ParentBusy))
    );
    assert!(trace.final_state.entry(child).is_some());
    let mut roots = Fixture::new(Bounds::production());
    for group in 1..=5 {
        roots.admitted(None, Class::Capability, group);
    }
    let trace = expect_chosen_pass(&roots);
    assert_eq!(trace.final_state.roots(), 4);
    assert_eq!(
        trace.final_state.outcome(5),
        Some(Outcome::Refused(Refusal::Busy))
    );
}

#[test]
fn chosen_adopted_siblings_cancel_in_lower_id_leaf_order() {
    let mut fixture = Fixture::new(Bounds::production());
    let root = fixture.admitted(None, Class::Capability, 1);
    fixture.admitted(None, Class::Capability, 10);
    fixture.admitted(None, Class::Capability, 20);
    let parent = fixture.child(root, 2);
    let adopted = fixture.child(parent, 3);
    fixture.apply(Step::Complete(parent));
    let started = fixture.child(root, 4);
    let replacement = fixture.admitted(None, Class::Capability, 1);
    let trace = expect_chosen_pass(&fixture);
    assert_eq!(
        trace.final_state.terminal_order(),
        vec![parent, adopted, started, root]
    );
    assert_eq!(
        trace.final_state.outcome(adopted),
        Some(Outcome::Aborted(Trigger::ParentAborted))
    );
    assert_eq!(
        trace.final_state.outcome(started),
        Some(Outcome::Aborted(Trigger::ParentAborted))
    );
    assert_eq!(
        trace.final_state.outcome(root),
        Some(Outcome::Aborted(Trigger::Superseded))
    );
    assert!(trace.final_state.entry(replacement).is_some());
}

#[test]
fn chosen_completion_late_push_and_dispatch_revocation_windows() {
    let mut completed = Fixture::new(Bounds::production());
    let session = completed.admitted(None, Class::Session(SessionFamily::Gather), 1);
    let child = completed.child(session, 2);
    completed.apply(Step::Emit {
        of: Some(child),
        release: false,
    });
    completed.apply(Step::Complete(child));
    completed.apply(Step::Emit {
        of: Some(child),
        release: false,
    });
    completed.apply(Step::Take);
    completed.apply(Step::Dispatch);
    let trace = expect_chosen_pass(&completed);
    assert_eq!(trace.final_state.dispatched(), 1);

    let mut revoked = Fixture::new(Bounds::production());
    let owner = revoked.admitted(None, Class::Capability, 1);
    revoked.apply(Step::Emit {
        of: Some(owner),
        release: false,
    });
    revoked.apply(Step::Take);
    revoked.apply(Step::Revoke {
        of: owner,
        trigger: Trigger::HostRevoke,
    });
    revoked.apply(Step::Dispatch);
    revoked.apply(Step::Emit {
        of: Some(owner),
        release: false,
    });
    let late = revoked.start(Some(owner), None, Class::Capability, 2, 0, 0);
    revoked.apply(Step::Admit(late));
    revoked.apply(Step::Take);
    revoked.apply(Step::Dispatch);
    let trace = expect_chosen_pass(&revoked);
    assert_eq!(trace.final_state.dispatched(), 0);
    assert_eq!(
        trace.final_state.outcome(late),
        Some(Outcome::Refused(Refusal::ParentNotLive))
    );
}

#[test]
fn chosen_fifty_late_copies_retire_without_carry_revival() {
    for cancel in [false, true] {
        let mut fixture = Fixture::new(Bounds::production());
        let owner = fixture.admitted(None, Class::Capability, 1);
        fixture.apply(Step::Complete(owner));
        fixture.apply(Step::Page);
        fixture.apply(Step::Watermark);
        for _ in 0..50 {
            fixture.apply(if cancel {
                Step::Cancel {
                    of: owner,
                    throw: false,
                }
            } else {
                Step::Decide {
                    of: owner,
                    complete: false,
                }
            });
            fixture.apply(Step::Batch);
        }
        let trace = expect_chosen_pass(&fixture);
        assert_eq!(trace.final_state.late_controls(), 50);
        assert_eq!(trace.final_state.carry_len(), 0);
        assert_eq!(trace.final_state.settlements(owner), 1);
        assert_eq!(trace.final_state.retained(), 0);
    }
}

#[test]
fn chosen_hybrid_parent_cancels_all_thirty_two_pending_starts() {
    let mut fixture = Fixture::new(Bounds::production());
    let parent = fixture.mirror(None, None);
    let mut children = Vec::new();
    for group in 1..=32 {
        children.push(fixture.start(None, Some(parent), Class::Capability, group, 0, 0));
    }
    fixture.apply(Step::Revoke {
        of: parent,
        trigger: Trigger::HostRevoke,
    });
    let carrying = expect_chosen_pass(&fixture);
    assert_eq!(carrying.final_state.carry_len(), 32);
    fixture.apply(Step::Batch);
    let trace = expect_chosen_pass(&fixture);
    assert_eq!(
        (trace.final_state.live(), trace.final_state.retained()),
        (0, 32)
    );
    fixture.apply(Step::Page);
    fixture.apply(Step::Watermark);
    let trace = expect_chosen_pass(&fixture);
    assert_eq!(
        (
            trace.final_state.pending(),
            trace.final_state.carry_len(),
            trace.final_state.retained()
        ),
        (0, 0, 0)
    );
    for child in children {
        assert_eq!(
            trace.final_state.outcome(child),
            Some(Outcome::Aborted(Trigger::ParentAborted))
        );
        assert_eq!(trace.final_state.settlements(child), 1);
    }
}

#[test]
fn chosen_full_capacity_nested_supersession() {
    let (mut fixture, _, _, nested) = full_replacement_fixture();
    let top_level = fixture.admitted(None, Class::Capability, 19);
    let trace = expect_chosen_pass(&fixture);
    assert_eq!(
        trace.final_state.outcome(top_level),
        Some(Outcome::Refused(Refusal::Busy))
    );
    assert!(trace.final_state.entry(nested).is_some());
    assert_eq!(trace.final_state.live(), 16);
    let replacement_root = fixture.admitted(None, Class::Capability, 16);
    let trace = expect_chosen_pass(&fixture);
    assert_eq!(trace.final_state.roots(), 4);
    assert!(trace.final_state.entry(replacement_root).is_some());
    assert_eq!(
        trace.final_state.outcome(nested),
        Some(Outcome::Aborted(Trigger::ParentAborted))
    );
}

#[test]
fn chosen_full_forest_adoption_needs_future_root_reservation() {
    let (mut fixture, parent, _, adopted) = full_replacement_fixture();
    let grandparent = fixture.builder.entry(parent).unwrap().parent.unwrap();
    let root = fixture.builder.entry(grandparent).unwrap().parent.unwrap();
    fixture.apply(Step::Complete(parent));
    let extra = fixture.child(grandparent, 18);
    let trace = expect_chosen_pass(&fixture);
    assert_eq!(trace.final_state.live(), 15);
    assert_eq!(trace.final_state.leaves(), 4);
    assert_eq!(
        trace.final_state.entry(adopted).unwrap().parent,
        Some(grandparent)
    );
    assert_eq!(
        trace.final_state.outcome(extra),
        Some(Outcome::Refused(Refusal::Busy))
    );
    fixture.child(root, 17);
    let trace = expect_chosen_pass(&fixture);
    assert_eq!(
        trace.final_state.outcome(adopted),
        Some(Outcome::Aborted(Trigger::ParentAborted))
    );
    assert_eq!(
        trace.final_state.outcome(grandparent),
        Some(Outcome::Aborted(Trigger::Superseded))
    );
}

#[test]
fn chosen_pending_actions_do_not_dispatch_without_admission() {
    for case in 0..3 {
        let mut bounds = Bounds::production();
        if case == 0 {
            bounds.credits = 1;
        }
        let mut fixture = Fixture::new(bounds);
        let session = fixture.admitted(None, Class::Session(SessionFamily::Gather), 1);
        let pending = fixture.start(
            Some(session),
            None,
            if case == 1 {
                Class::Session(SessionFamily::Gather)
            } else {
                Class::Capability
            },
            2,
            0,
            0,
        );
        fixture.apply(Step::Emit {
            of: Some(pending),
            release: false,
        });
        if case == 2 {
            fixture.apply(Step::Cancel {
                of: pending,
                throw: false,
            });
            fixture.apply(Step::Controls);
        }
        fixture.apply(Step::Admit(pending));
        fixture.apply(Step::Take);
        fixture.apply(Step::Dispatch);
        let trace = expect_chosen_pass(&fixture);
        assert_eq!(trace.final_state.dispatched(), 0);
        assert!(trace.final_state.entry(pending).is_none());
        assert_eq!(
            trace.final_state.outcome(pending),
            match case {
                0 => None,
                1 => Some(Outcome::Refused(Refusal::Busy)),
                _ => Some(Outcome::Aborted(Trigger::ParentAborted)),
            }
        );
    }
}

#[test]
fn chosen_encoder_prioritizes_releases_and_preserves_gameplay_order() {
    let mut fixture = Fixture::new(Bounds::production());
    let first = fixture.mirror(None, None);
    let second = fixture.mirror(None, None);
    fixture.apply(Step::Emit {
        of: Some(first),
        release: false,
    });
    fixture.apply(Step::Emit {
        of: Some(second),
        release: false,
    });
    fixture.apply(Step::Emit {
        of: Some(first),
        release: true,
    });
    fixture.apply(Step::Take);
    let trace = expect_chosen_pass(&fixture);
    assert_eq!(
        trace.final_state.encoded_rows(),
        vec![
            (Some(first), true),
            (Some(first), false),
            (Some(second), false)
        ]
    );
}

#[test]
fn chosen_history_expired_is_distinct_from_revoked() {
    let mut fixture = Fixture::new(Bounds::production());
    let completed = fixture.admitted(None, Class::Capability, 0);
    fixture.apply(Step::Complete(completed));
    fixture.apply(Step::Page);
    fixture.apply(Step::Watermark);
    for index in 0..65 {
        let revoked = fixture.admitted(None, Class::Capability, (index + 1) as u8);
        fixture.apply(Step::Revoke {
            of: revoked,
            trigger: Trigger::HostRevoke,
        });
        fixture.apply(Step::Page);
        fixture.apply(Step::Watermark);
    }
    let child = fixture.start(Some(completed), None, Class::Capability, 100, 0, 0);
    fixture.apply(Step::Admit(child));
    fixture.apply(Step::Emit {
        of: Some(completed),
        release: false,
    });
    fixture.apply(Step::Take);
    fixture.apply(Step::Dispatch);

    let trace = expect_chosen_pass(&fixture);
    assert!(trace.final_state.history_floor() > completed);
    assert_eq!(trace.final_state.outcome(completed), Some(Outcome::Done));
    assert_eq!(
        trace.final_state.outcome(child),
        Some(Outcome::Refused(Refusal::HistoryExpired))
    );
    assert_eq!(trace.final_state.dispatched(), 0);
    assert_eq!(trace.final_state.dropped(), 1);
}

#[test]
fn chosen_session_root_rows_follow_family_and_start_order() {
    let families = [
        SessionFamily::Hunt,
        SessionFamily::Gather,
        SessionFamily::Combat,
        SessionFamily::Progress,
    ];
    for family in families {
        let mut capability_first = Fixture::new(Bounds::production());
        let root = capability_first.admitted(None, Class::Capability, 1);
        capability_first.apply(Step::Emit {
            of: Some(root),
            release: false,
        });
        let session = capability_first.admitted(None, Class::Session(family), 2);
        capability_first.apply(Step::Take);
        capability_first.apply(Step::Dispatch);
        capability_first.apply(Step::Emit {
            of: Some(root),
            release: false,
        });
        capability_first.apply(Step::Take);
        capability_first.apply(Step::Dispatch);
        let trace = expect_chosen_pass(&capability_first);
        let dispatched = usize::from(family == SessionFamily::Hunt) * 2;
        assert_eq!(trace.final_state.dispatched(), dispatched, "{family:?}");
        assert_eq!(trace.final_state.dropped(), 2 - dispatched, "{family:?}");
        assert!(trace.final_state.entry(root).is_some(), "{family:?}");
        assert!(trace.final_state.entry(session).is_some(), "{family:?}");
        assert_eq!(
            trace.final_state.foreground_session(),
            family.owns_foreground().then_some(session),
            "{family:?}"
        );

        let mut session_first = Fixture::new(Bounds::production());
        let session = session_first.admitted(None, Class::Session(family), 3);
        let root = session_first.start(None, None, Class::Capability, 4, 0, 0);
        session_first.apply(Step::Admit(root));
        let trace = expect_chosen_pass(&session_first);
        assert_eq!(
            trace.final_state.outcome(root),
            Some(Outcome::Refused(Refusal::Busy)),
            "{family:?}"
        );
        assert!(trace.final_state.entry(root).is_none(), "{family:?}");
        assert!(trace.final_state.entry(session).is_some(), "{family:?}");
        assert_eq!(
            trace.final_state.foreground_session(),
            family.owns_foreground().then_some(session),
            "{family:?}"
        );
    }
}

#[test]
fn chosen_session_capture_handoff_covers_all_family_pairs() {
    let families = [
        SessionFamily::Hunt,
        SessionFamily::Gather,
        SessionFamily::Combat,
        SessionFamily::Progress,
    ];
    for old_family in families {
        for new_family in families {
            let mut fixture = Fixture::new(Bounds::production());
            let root = fixture.admitted(None, Class::Capability, 1);
            let first = fixture.admitted(None, Class::Session(old_family), 2);
            fixture.apply(Step::Emit {
                of: Some(first),
                release: false,
            });
            fixture.apply(Step::Complete(first));
            let second = fixture.admitted(None, Class::Session(new_family), 3);
            fixture.apply(Step::Take);
            fixture.apply(Step::Dispatch);
            let trace = expect_chosen_pass(&fixture);
            let should_dispatch = new_family == SessionFamily::Hunt;
            assert_eq!(
                trace.final_state.dispatched(),
                usize::from(should_dispatch),
                "{old_family:?} -> {new_family:?}"
            );
            assert_eq!(
                trace.final_state.dropped(),
                usize::from(!should_dispatch),
                "{old_family:?} -> {new_family:?}"
            );
            assert_eq!(trace.final_state.outcome(first), Some(Outcome::Done));
            assert!(trace.final_state.entry(second).is_some());
            assert!(trace.final_state.entry(root).is_some());
            assert_eq!(
                trace.final_state.foreground_session(),
                new_family.owns_foreground().then_some(second),
                "{old_family:?} -> {new_family:?}"
            );
        }
    }
}

#[test]
fn chosen_guardian_actions_dispatch_outside_session_queue() {
    let mut fixture = Fixture::new(Bounds::production());
    let root = fixture.admitted(None, Class::Capability, 1);
    let session = fixture.admitted(None, Class::Session(SessionFamily::Gather), 2);
    for (count, action) in [5, 1, 9].into_iter().enumerate() {
        fixture.apply(Step::HostAction { action });
        assert_eq!(fixture.builder.host_actions_dispatched(), count + 1);
        assert_eq!(fixture.builder.last_host_action(), Some(action));
        assert_eq!(fixture.builder.dispatched(), 0);
    }
    assert_eq!(fixture.builder.foreground_session(), Some(session));
    assert!(fixture.builder.entry(root).is_some());
    let trace = expect_chosen_pass(&fixture);
    assert_eq!(trace.final_state.host_actions_dispatched(), 3);
    assert_eq!(trace.final_state.last_host_action(), Some(9));
    assert_eq!(trace.final_state.dispatched(), 0);
    assert!(trace.final_state.entry(root).is_some());
    assert_eq!(trace.final_state.foreground_session(), Some(session));
}

#[test]
fn chosen_guardian_hold_freezes_seat_and_preserves_rows() {
    let mut fixture = Fixture::new(Bounds::production());
    let root = fixture.admitted(None, Class::Capability, 1);
    let session = fixture.admitted(None, Class::Session(SessionFamily::Gather), 2);
    fixture.apply(Step::Emit {
        of: Some(session),
        release: false,
    });
    fixture.apply(Step::Emit {
        of: Some(root),
        release: true,
    });
    fixture.apply(Step::Take);
    fixture.apply(Step::Emit {
        of: Some(session),
        release: false,
    });
    let pending = fixture.start(Some(session), None, Class::Capability, 3, 0, 0);
    fixture.apply(Step::SessionHeld);
    assert!(fixture.builder.session_held());
    assert_eq!(fixture.builder.foreground_session(), Some(session));

    let blocked = [
        Step::Start {
            parent: None,
            local_parent: None,
            class: Class::Capability,
            group: 3,
            bytes: 0,
            hooks: 0,
        },
        Step::Admit(pending),
        Step::AdmitStamped {
            of: pending,
            session: 0,
        },
        Step::Dispatch,
        Step::Complete(session),
        Step::Controls,
        Step::Page,
        Step::Watermark,
        Step::Emit {
            of: Some(session),
            release: false,
        },
        Step::Take,
        Step::Batch,
        Step::Reconnect { keep: true },
        Step::Stop,
        Step::HeldTick,
        Step::Throw(session),
        Step::Prompt(session),
        Step::Cancel {
            of: pending,
            throw: false,
        },
        Step::Revoke {
            of: session,
            trigger: Trigger::HostRevoke,
        },
        Step::Mirror {
            ancestor: Some(session),
            parent: None,
        },
    ];
    for step in blocked {
        let before = fixture.builder.clone();
        assert!(fixture.builder.apply(&step).is_err(), "{step:?}");
        assert_eq!(fixture.builder, before, "{step:?}");
    }

    fixture.apply(Step::HostAction { action: 7 });
    assert_eq!(fixture.builder.host_actions_dispatched(), 1);
    assert_eq!(fixture.builder.last_host_action(), Some(7));
    assert!(fixture.builder.session_held());
    assert_eq!(fixture.builder.foreground_session(), Some(session));
    assert!(fixture.builder.entry(root).is_some());
    fixture.apply(Step::SessionResumed);
    assert!(!fixture.builder.session_held());
    assert_eq!(fixture.builder.foreground_session(), Some(session));
    fixture.apply(Step::Admit(pending));
    fixture.apply(Step::Dispatch);
    fixture.apply(Step::Take);
    fixture.apply(Step::Dispatch);
    let trace = expect_chosen_pass(&fixture);
    assert_eq!(trace.final_state.dispatched(), 2);
    assert_eq!(trace.final_state.release_dispatched(), 1);
    assert_eq!(trace.final_state.dropped(), 0);
    assert_eq!(trace.final_state.host_actions_dispatched(), 1);
    assert_eq!(trace.final_state.foreground_session(), Some(session));
    assert!(trace.final_state.entry(root).is_some());
    assert!(trace.final_state.entry(pending).is_some());
}

#[test]
fn chosen_held_tick_discards_rows_without_settling_live_entries() {
    let mut fixture = Fixture::new(Bounds::production());
    let live = fixture.admitted(None, Class::Capability, 1);
    fixture.apply(Step::Emit {
        of: Some(live),
        release: false,
    });
    fixture.apply(Step::Emit {
        of: Some(live),
        release: false,
    });
    fixture.apply(Step::Take);
    fixture.apply(Step::Emit {
        of: Some(live),
        release: false,
    });
    fixture.apply(Step::HeldTick);
    let trace = expect_chosen_pass(&fixture);
    assert_eq!(trace.final_state.dropped(), 3);
    assert_eq!(trace.final_state.live(), 1);
    assert!(trace.final_state.entry(live).is_some());
    assert_eq!(trace.final_state.outcome(live), None);
    assert_eq!(trace.final_state.dispatched(), 0);
}

#[test]
fn chosen_kept_reconnect_preserves_undelivered_release() {
    let mut fixture = Fixture::new(Bounds::production());
    let walking = fixture.admitted(None, Class::Capability, 1);
    fixture.apply(Step::Emit {
        of: Some(walking),
        release: false,
    });
    fixture.apply(Step::Emit {
        of: Some(walking),
        release: true,
    });
    fixture.apply(Step::Reconnect { keep: true });
    fixture.apply(Step::Take);
    fixture.apply(Step::Dispatch);
    let trace = expect_chosen_pass(&fixture);
    assert_eq!(trace.final_state.live(), 1);
    assert!(trace.final_state.entry(walking).is_some());
    assert_eq!(trace.final_state.release_dispatched(), 1);
    assert_eq!(trace.final_state.dispatched(), 0);
    assert_eq!(trace.final_state.dropped(), 1);
}

#[test]
fn chosen_release_cap_keeps_two_singleton_slots_beyond_walking_owners() {
    let mut fixture = Fixture::new(Bounds::production());
    for _ in 0..16 {
        let walking = fixture.mirror(None, None);
        fixture.apply(Step::Emit {
            of: Some(walking),
            release: true,
        });
    }
    fixture.apply(Step::Emit {
        of: None,
        release: true,
    });
    fixture.apply(Step::Emit {
        of: Some(0),
        release: true,
    });
    for _ in 0..300 {
        fixture.apply(Step::Emit {
            of: None,
            release: false,
        });
    }
    fixture.apply(Step::Take);
    let encoded = fixture.trace(Rules::Chosen);
    assert_eq!(
        encoded
            .final_state
            .encoded_rows()
            .iter()
            .filter(|(_, release)| *release)
            .count(),
        18
    );
    fixture.apply(Step::Dispatch);
    let trace = expect_chosen_pass(&fixture);
    assert_eq!(trace.final_state.release_dispatched(), 18);
    assert_eq!(trace.final_state.dispatched(), 141);
    assert_eq!(trace.final_state.dropped(), 159);
}

#[test]
fn chosen_pending_cancel_records_intent_at_cancel_event() {
    let mut fixture = Fixture::new(Bounds::production());
    let pending = fixture.start(None, None, Class::Capability, 1, 0, 0);
    assert!(
        fixture
            .builder
            .apply(&Step::Cancel {
                of: pending,
                throw: true,
            })
            .is_err()
    );
    fixture.apply(Step::Cancel {
        of: pending,
        throw: false,
    });
    fixture.apply(Step::Controls);
    fixture.apply(Step::Admit(pending));
    let trace = expect_chosen_pass(&fixture);
    assert!(trace.final_state.entry(pending).is_none());
    assert_eq!(
        trace.final_state.outcome(pending),
        Some(Outcome::Aborted(Trigger::ParentAborted))
    );
}
