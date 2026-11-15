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
    for profile in [GuardianProfile::Ownership, GuardianProfile::OwnershipSeat] {
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
