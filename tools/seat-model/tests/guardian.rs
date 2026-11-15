//! C6 ownership model: the chosen rules pass, and every rejected design and
//! found defect stays a mutation that a named invariant catches (ADR 0017).
use seat_model::ExploreLimits;
use seat_model::guardian::{
    Family, GUARDIAN_PROFILES, GuardianMutation, GuardianProfile, GuardianStep, ScriptExit,
    explore_guardian_profile, run_guardian_trace,
};

const SEAT_MUTATIONS: [GuardianMutation; 2] = [
    GuardianMutation::GlobalUnhold,
    GuardianMutation::HoldNotAppliedToSeat,
];

fn profile_for(mutation: GuardianMutation) -> GuardianProfile {
    if SEAT_MUTATIONS.contains(&mutation) {
        GuardianProfile::OwnershipSeat
    } else if mutation == GuardianMutation::GuardNeverExpires {
        GuardianProfile::OwnershipEnt
    } else {
        GuardianProfile::Ownership
    }
}

fn tick() -> GuardianStep {
    GuardianStep::Tick { blocked: false }
}

fn appear(family: Family) -> GuardianStep {
    GuardianStep::Appear {
        family,
        forced_host: false,
    }
}

#[test]
fn chosen_profiles_close_without_a_cap() {
    for profile in [
        GuardianProfile::Ownership,
        GuardianProfile::OwnershipSeat,
        GuardianProfile::OwnershipEnt,
    ] {
        let result =
            explore_guardian_profile(profile, GuardianMutation::None, ExploreLimits::default());
        assert!(
            result.complete && result.failure.is_none() && result.cap_reached.is_none(),
            "{}: {:?}",
            profile.name(),
            result.failure
        );
        assert!(
            result.states > 1000,
            "{} explored too little",
            profile.name()
        );
        assert!(
            result.depth_reached < profile.horizon(),
            "{} did not close inside its horizon",
            profile.name()
        );
        // ADR 0017 / design-r3 section 3.4: 1,000,000 states, 250,000
        // frontier entries and 512 MiB for the 32-transition profile.
        assert!(result.states <= 1_000_000, "{}", profile.name());
        assert!(result.max_frontier <= 250_000, "{}", profile.name());
        assert!(
            result.estimated_peak_bytes <= 512 * 1024 * 1024,
            "{}: {} bytes",
            profile.name(),
            result.estimated_peak_bytes
        );
    }
}

#[test]
fn a_reached_cap_is_incomplete_not_a_pass() {
    let result = explore_guardian_profile(
        GuardianProfile::Ownership,
        GuardianMutation::None,
        ExploreLimits {
            state_cap: 100,
            ..ExploreLimits::default()
        },
    );
    assert!(!result.complete);
    assert!(result.cap_reached.is_some());
    assert_eq!(
        result.failure.as_ref().map(|(failure, _)| failure.name),
        Some("exploration-cap")
    );
    let result = explore_guardian_profile(
        GuardianProfile::Ownership,
        GuardianMutation::None,
        ExploreLimits {
            frontier_cap: 1,
            ..ExploreLimits::default()
        },
    );
    assert!(result.cap_reached.is_some() && !result.complete);
}

#[test]
fn every_mutation_is_caught_by_its_invariant() {
    for (mutation, expected) in GuardianMutation::ALL {
        let profile = profile_for(mutation);
        let result = explore_guardian_profile(profile, mutation, ExploreLimits::default());
        assert_eq!(result.cap_reached, None, "{mutation:?} was capped");
        let (failure, trace) = result
            .failure
            .as_ref()
            .unwrap_or_else(|| panic!("{mutation:?} was not caught on {}", profile.name()));
        assert_eq!(
            failure.name, expected,
            "{mutation:?} reached {} ({}) instead of {expected}\n{trace:#?}",
            failure.name, failure.detail
        );
        assert!(!trace.is_empty(), "{mutation:?} failed before any step");
        // The explorer is breadth first: the trace is a shortest one.
        let replay = run_guardian_trace(profile, mutation, trace)
            .expect_err("the counterexample must replay to its failure");
        assert_eq!(replay.0, trace.len() - 1, "{mutation:?} replay diverged");
        assert_eq!(replay.1.name, expected);
    }
}

#[test]
fn every_profile_is_listed_and_unique() {
    let mut names: Vec<_> = GUARDIAN_PROFILES.iter().map(|p| p.name()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), GUARDIAN_PROFILES.len());
}

/// 0.2.4: a toggle flipped off with a Host solver in flight releases the slot.
#[test]
fn stuck_gatherer_toggle_off_releases_the_hold() {
    let steps = [
        appear(Family::Solver),
        tick(),
        GuardianStep::SetEnabled(false),
        tick(),
    ];
    let state =
        run_guardian_trace(GuardianProfile::Ownership, GuardianMutation::None, &steps).unwrap();
    assert!(!state.publication().hold && !state.publication().ours);
    let caught = run_guardian_trace(
        GuardianProfile::Ownership,
        GuardianMutation::StuckGathererHold,
        &steps,
    )
    .unwrap_err();
    assert_eq!(caught.1.name, "toggle-off-publishes");
}

/// 0.2.7: a paused, stopped or failed script returns a held box at once.
#[test]
fn a_script_that_leaves_returns_its_claim_within_one_tick() {
    for exit in [ScriptExit::Pause, ScriptExit::Stop, ScriptExit::Fail] {
        let steps = [
            appear(Family::Solver),
            GuardianStep::ScriptStart {
                wants: true,
                live: false,
            },
            tick(),
            GuardianStep::ScriptExit(exit),
            tick(),
        ];
        let state =
            run_guardian_trace(GuardianProfile::Ownership, GuardianMutation::None, &steps).unwrap();
        assert!(!state.publication().claim_script, "{exit:?}");
        let caught = run_guardian_trace(
            GuardianProfile::Ownership,
            GuardianMutation::ClaimKeptOnExit(exit),
            &steps,
        )
        .unwrap_err();
        assert_eq!(caught.1.name, "owner-without-script", "{exit:?}");
    }
}

/// 0.2.5: a stunned flee tick does not spend the budget.
#[test]
fn blocked_ticks_do_not_spend_the_wait() {
    let steps = [
        appear(Family::Solver),
        tick(),
        GuardianStep::Tick { blocked: true },
        GuardianStep::Tick { blocked: true },
        GuardianStep::Tick { blocked: true },
    ];
    let state =
        run_guardian_trace(GuardianProfile::Ownership, GuardianMutation::None, &steps).unwrap();
    assert!(state.publication().hold, "still waiting for its outcome");
    let caught = run_guardian_trace(
        GuardianProfile::Ownership,
        GuardianMutation::FleeInterrupted,
        &steps,
    )
    .unwrap_err();
    assert_eq!(caught.1.name, "wait-early");
}

/// 0.2.7: host-owned work survives a script that stops, with no second send.
#[test]
fn host_work_survives_a_script_stop_without_a_second_send() {
    let steps = [
        GuardianStep::ScriptStart {
            wants: false,
            live: true,
        },
        appear(Family::Solver),
        tick(),
        GuardianStep::ScriptExit(ScriptExit::Stop),
        tick(),
    ];
    let state =
        run_guardian_trace(GuardianProfile::Ownership, GuardianMutation::None, &steps).unwrap();
    assert!(!state.emitted(), "the pending operation is not sent again");
    assert!(state.publication().hold);
}

/// A forced-host event (Swarm, the angry plant) never goes to a script claim.
#[test]
fn forced_host_events_never_go_to_a_script() {
    let steps = [
        GuardianStep::ScriptStart {
            wants: true,
            live: true,
        },
        GuardianStep::Appear {
            family: Family::Solver,
            forced_host: true,
        },
        tick(),
    ];
    let state =
        run_guardian_trace(GuardianProfile::Ownership, GuardianMutation::None, &steps).unwrap();
    assert!(!state.publication().claim_script && state.publication().hold);
    let caught = run_guardian_trace(
        GuardianProfile::Ownership,
        GuardianMutation::ForcedHostIgnored,
        &steps,
    )
    .unwrap_err();
    assert_eq!(caught.1.name, "owner-forced-host");
}

/// An exhausted episode cannot mask an independent event.
#[test]
fn an_exhausted_episode_does_not_mask_the_next_event() {
    let steps = [
        appear(Family::Solver),
        tick(),
        tick(),
        tick(),
        GuardianStep::Replace {
            family: Family::Solver,
            forced_host: false,
        },
        tick(),
    ];
    let state =
        run_guardian_trace(GuardianProfile::Ownership, GuardianMutation::None, &steps).unwrap();
    assert!(state.publication().hold, "the replacement is the host's");
    let caught = run_guardian_trace(
        GuardianProfile::Ownership,
        GuardianMutation::OverlapMasksBox,
        &steps,
    )
    .unwrap_err();
    assert_eq!(caught.1.name, "unowned-event");
}

/// The next hostile actor inherits a pending flee: the hold stays through the
/// replacement, and the wait starts again. Replacing any other solver releases
/// the work for the pass.
#[test]
fn a_hostile_actor_replaced_by_another_keeps_the_flee_and_its_hold() {
    let replace = |family| GuardianStep::Replace {
        family,
        forced_host: false,
    };
    let steps = [
        appear(Family::Evade),
        tick(),
        replace(Family::Evade),
        tick(),
    ];
    let state =
        run_guardian_trace(GuardianProfile::Ownership, GuardianMutation::None, &steps).unwrap();
    let out = state.publication();
    assert!(
        out.hold && out.acting,
        "the successor has the flee: {out:?}"
    );
    let caught = run_guardian_trace(
        GuardianProfile::Ownership,
        GuardianMutation::EvadeReleasesOnReplace,
        &steps,
    )
    .unwrap_err();
    assert_eq!(caught.1.name, "evade-handoff");
    // Any other solver releases its work when its event is replaced.
    let steps = [
        appear(Family::Solver),
        tick(),
        replace(Family::Solver),
        tick(),
    ];
    run_guardian_trace(GuardianProfile::Ownership, GuardianMutation::None, &steps).unwrap();
    // A flee replaced by another family is released as well.
    let steps = [appear(Family::Evade), tick(), replace(Family::Trap), tick()];
    run_guardian_trace(GuardianProfile::Ownership, GuardianMutation::None, &steps).unwrap();
}

/// `acting` is a published field: a solver in flight acts, a dialog handle
/// does not.
#[test]
fn acting_is_published_for_a_solver_and_not_for_a_dialog() {
    let state = run_guardian_trace(
        GuardianProfile::Ownership,
        GuardianMutation::None,
        &[appear(Family::Solver), tick()],
    )
    .unwrap();
    assert!(state.publication().acting && !state.publication().handling);
    let state = run_guardian_trace(
        GuardianProfile::Ownership,
        GuardianMutation::None,
        &[appear(Family::Dialog), tick()],
    )
    .unwrap();
    assert!(state.publication().handling && !state.publication().acting);
}

/// The Ent guard is host work with no detected event: it holds while the Ent
/// stands, the toggle is on and the claim is the host's, for a bounded time.
#[test]
fn the_ent_guard_holds_for_a_bounded_time_and_follows_the_toggle() {
    let profile = GuardianProfile::OwnershipEnt;
    let held = run_guardian_trace(
        profile,
        GuardianMutation::None,
        &[GuardianStep::EntAppear, tick()],
    )
    .unwrap();
    let out = held.publication();
    assert!(out.hold && !out.ours && !out.acting && !out.claim_script);
    let off = run_guardian_trace(
        profile,
        GuardianMutation::None,
        &[
            GuardianStep::EntAppear,
            tick(),
            GuardianStep::SetEnabled(false),
            tick(),
        ],
    )
    .unwrap();
    assert!(!off.publication().hold, "the toggle off releases the guard");
    // The bound is two fresh ticks in this profile: the third pass lets go,
    // and the Ent stays excluded until it is observed gone.
    let mut steps = vec![GuardianStep::EntAppear];
    steps.extend([tick(), tick(), tick(), tick()]);
    let spent = run_guardian_trace(profile, GuardianMutation::None, &steps).unwrap();
    assert!(!spent.publication().hold, "the guard is bounded");
    steps.extend([
        GuardianStep::EntRemove,
        tick(),
        GuardianStep::EntAppear,
        tick(),
    ]);
    let again = run_guardian_trace(profile, GuardianMutation::None, &steps).unwrap();
    assert!(again.publication().hold, "a new Ent is guarded again");
    let caught =
        run_guardian_trace(profile, GuardianMutation::GuardNeverExpires, &steps).unwrap_err();
    assert_eq!(caught.1.name, "guard-unbounded");
}

/// A claim belongs to the script runtime that made it. A compiled script that
/// replaces the claiming one before the next observation, and never claims the
/// event itself, does not inherit the claim, and is not called back for it
/// (round-2 review probe `runtime-generation-model-probe.diff`).
#[test]
fn a_replaced_compiled_runtime_does_not_inherit_a_claim() {
    let steps = [
        GuardianStep::ScriptStart {
            wants: true,
            live: false,
        },
        appear(Family::Solver),
        tick(),
        GuardianStep::ScriptExit(ScriptExit::Stop),
        GuardianStep::ScriptStart {
            wants: false,
            live: false,
        },
        tick(),
    ];
    let state =
        run_guardian_trace(GuardianProfile::Ownership, GuardianMutation::None, &steps).unwrap();
    assert!(
        !state.publication().claim_script,
        "a replacement runtime inherited the old claim: {}",
        state.describe()
    );
    assert!(state.publication().hold, "the host takes the event");
    // A successor that would claim is not called back for the old event.
    let mut claiming = steps.clone();
    claiming[4] = GuardianStep::ScriptStart {
        wants: true,
        live: false,
    };
    let state = run_guardian_trace(
        GuardianProfile::Ownership,
        GuardianMutation::None,
        &claiming,
    )
    .unwrap();
    assert!(!state.publication().claim_script, "{}", state.describe());
    let caught = run_guardian_trace(
        GuardianProfile::Ownership,
        GuardianMutation::ClaimKeptAcrossRuntime,
        &steps,
    )
    .unwrap_err();
    assert_eq!(caught.1.name, "stale-runtime");
    // Pause and Resume keep the runtime: the claim comes back to the host on
    // the pause and is not taken again without a new event.
    let paused = [
        GuardianStep::ScriptStart {
            wants: true,
            live: false,
        },
        appear(Family::Solver),
        tick(),
        GuardianStep::ScriptExit(ScriptExit::Pause),
        GuardianStep::ScriptResume,
        tick(),
    ];
    let state =
        run_guardian_trace(GuardianProfile::Ownership, GuardianMutation::None, &paused).unwrap();
    assert!(state.publication().claim_script, "{}", state.describe());
}

/// The Evade escape budget is the content's clock: a script claim does not
/// pause it, and a spent budget is detect-only for the script too (round-2
/// review probe `review_evade_script_owned_deadline_conforms_to_the_model`).
#[test]
fn an_evade_budget_runs_whoever_owns_the_event() {
    let profile = GuardianProfile::Ownership;
    let wait = profile.bounds().wait as usize;
    let mut steps = vec![
        GuardianStep::ScriptStart {
            wants: true,
            live: false,
        },
        appear(Family::Evade),
    ];
    steps.extend(std::iter::repeat_n(tick(), wait));
    let state = run_guardian_trace(profile, GuardianMutation::None, &steps).unwrap();
    let out = state.publication();
    assert!(out.claim_script && out.ours, "{}", state.describe());
    steps.push(tick());
    let state = run_guardian_trace(profile, GuardianMutation::None, &steps).unwrap();
    let out = state.publication();
    assert!(
        out.claim_script && !out.ours && !out.hold,
        "the budget ran out under the script: {}",
        state.describe()
    );
    // The toggle off does not pause it either, and the host that takes the
    // event back does not begin it again.
    let mut steps = vec![
        appear(Family::Evade),
        tick(),
        GuardianStep::SetEnabled(false),
    ];
    steps.extend(std::iter::repeat_n(tick(), wait));
    steps.push(GuardianStep::SetEnabled(true));
    steps.push(tick());
    let state = run_guardian_trace(profile, GuardianMutation::None, &steps).unwrap();
    let out = state.publication();
    assert!(
        !out.ours && !out.hold && !out.acting,
        "{}",
        state.describe()
    );
    // A stunned tick spends nothing.
    let mut steps = vec![appear(Family::Evade)];
    steps.extend(std::iter::repeat_n(tick(), wait));
    steps.push(GuardianStep::Tick { blocked: true });
    let state = run_guardian_trace(profile, GuardianMutation::None, &steps).unwrap();
    assert!(state.publication().hold, "{}", state.describe());
    let caught =
        run_guardian_trace(profile, GuardianMutation::FleeInterrupted, &steps).unwrap_err();
    assert_eq!(caught.1.name, "wait-early");
}

/// A content window runs through a toggle off once the host has begun: the
/// host that finds it closed lets go (round-2 review probe
/// `review_lost_tool_toggle_deadline_conforms_to_the_model`). A host that
/// begins after the window has gone by opens a new one.
#[test]
fn a_content_window_runs_through_the_toggle_and_the_claim() {
    let profile = GuardianProfile::Ownership;
    let steps = [
        appear(Family::Window),
        tick(),
        GuardianStep::SetEnabled(false),
        tick(),
        GuardianStep::SetEnabled(true),
        tick(),
    ];
    let state = run_guardian_trace(profile, GuardianMutation::None, &steps).unwrap();
    let out = state.publication();
    assert!(
        !out.ours && !out.hold && !out.acting,
        "the window closed while the toggle was off: {}",
        state.describe()
    );
    // While the toggle is off the work keeps its place.
    let state = run_guardian_trace(profile, GuardianMutation::None, &steps[..4]).unwrap();
    assert!(state.publication().acting && !state.publication().hold);
    // A script that held the event past the window: the host that takes it
    // finds the window closed and lets go in the same pass.
    let late = [
        GuardianStep::ScriptStart {
            wants: true,
            live: false,
        },
        appear(Family::Window),
        tick(),
        tick(),
        tick(),
        tick(),
        GuardianStep::ScriptExit(ScriptExit::Pause),
        tick(),
    ];
    let state = run_guardian_trace(profile, GuardianMutation::None, &late).unwrap();
    let out = state.publication();
    assert!(
        !out.hold && !out.acting && !out.ours,
        "{}",
        state.describe()
    );
    // A pass that sees the toggle off re-arms the spent window once: the host
    // opens a new one when the toggle comes back.
    let mut rearm = steps.to_vec();
    rearm.extend([
        GuardianStep::SetEnabled(false),
        tick(),
        GuardianStep::SetEnabled(true),
        tick(),
    ]);
    let state = run_guardian_trace(profile, GuardianMutation::None, &rearm).unwrap();
    let out = state.publication();
    assert!(out.hold && out.acting && out.ours, "{}", state.describe());
}

/// A hostile actor that stays engaged while another event wins the detector
/// keeps its identity and its budget: the clock runs through the overlap, and
/// the returning actor whose budget ran out is detect-only (round-3 review
/// probe `retained-evade-combat-probe.rs`). An actor with budget left is the
/// host's flee again, with that budget and not a fresh one.
#[test]
fn a_retained_hostile_actor_keeps_its_budget_across_an_overlap() {
    let profile = GuardianProfile::Ownership;
    let wait = profile.bounds().wait as usize;
    let overlap = GuardianStep::Overlap {
        family: Family::Trap,
        forced_host: false,
    };
    // The budget runs out during the overlap.
    let mut spent = vec![appear(Family::Evade), tick(), overlap.clone()];
    spent.extend(std::iter::repeat_n(tick(), wait));
    spent.extend([GuardianStep::Return, tick()]);
    let state = run_guardian_trace(profile, GuardianMutation::None, &spent).unwrap();
    assert_eq!(state.event_id(), Some(0), "the same actor returns");
    let out = state.publication();
    assert!(
        !out.ours && !out.hold && !out.acting,
        "{}",
        state.describe()
    );
    let caught =
        run_guardian_trace(profile, GuardianMutation::RetainedClockLost, &spent).unwrap_err();
    assert_eq!(caught.1.name, "actor-budget");
    assert_eq!(caught.0, spent.len() - 1, "caught at the return's pass");
    // A budget of four: two ticks of the overlap leave one tick after the
    // first observation of the return, which publishes the actor ours; the
    // next pass spends the budget. A fresh budget would run four more.
    let nightly = GuardianProfile::Nightly;
    assert_eq!(nightly.bounds().wait, 4);
    let left = [
        appear(Family::Evade),
        tick(),
        overlap,
        tick(),
        tick(),
        GuardianStep::Return,
    ];
    let mut steps = left.to_vec();
    steps.push(tick());
    let state = run_guardian_trace(nightly, GuardianMutation::None, &steps).unwrap();
    assert_eq!(state.retained_id(), None);
    assert!(state.publication().ours, "{}", state.describe());
    steps.push(tick());
    let state = run_guardian_trace(nightly, GuardianMutation::None, &steps).unwrap();
    let out = state.publication();
    assert!(!out.ours && !out.hold, "{}", state.describe());
    let caught =
        run_guardian_trace(nightly, GuardianMutation::RetainedClockLost, &steps).unwrap_err();
    assert_eq!(caught.1.name, "actor-budget");
    // A retained actor observed gone does not come back.
    let mut gone = left[..4].to_vec();
    gone.extend([GuardianStep::RetainedGone, tick()]);
    let state = run_guardian_trace(nightly, GuardianMutation::None, &gone).unwrap();
    assert_eq!((state.event_id(), state.retained_id()), (Some(1), None));
    assert!(
        run_guardian_trace(
            nightly,
            GuardianMutation::None,
            &[gone, vec![GuardianStep::Return]].concat()
        )
        .is_err(),
        "nothing is retained to return"
    );
}
