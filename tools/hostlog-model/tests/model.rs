//! Tests of the stderr-queue model: every profile ends clean and exhausted,
//! and every mutation fails the invariant that must catch it.

use hostlog_model::{Limits, Mutation, explore_profile, profiles};

fn limits() -> Limits {
    Limits {
        state_cap: 5_000_000,
        frontier_cap: 2_000_000,
        byte_cap: 2 * 1024 * 1024 * 1024,
    }
}

#[test]
fn every_profile_ends_clean_and_exhausted() {
    for profile in profiles() {
        let result = explore_profile(&profile, Mutation::None, None, limits(), true);
        assert!(
            result.cap.is_none(),
            "{}: a cap stopped the run",
            profile.name
        );
        assert!(
            result.failure.is_none() && result.failures.is_empty(),
            "{}: {:?}",
            profile.name,
            result.failure.as_ref().map(|(v, trace)| (v, trace))
        );
        assert!(result.complete, "{}: incomplete", profile.name);
        // The state space ends before the horizon: the bound is not what
        // stops the search.
        assert!(
            result.deepest < profile.bounds.horizon,
            "{}: reached depth {} of {}",
            profile.name,
            result.deepest,
            profile.bounds.horizon
        );
        assert!(
            result.states > 50,
            "{}: {} states",
            profile.name,
            result.states
        );
    }
}

#[test]
fn every_mutation_fails_the_invariant_that_must_catch_it() {
    for mutation in Mutation::ALL {
        let Some(expected) = mutation.caught_by() else {
            continue;
        };
        let mut caught = false;
        for profile in profiles() {
            let result = explore_profile(&profile, mutation, None, limits(), true);
            caught |= result
                .failure
                .iter()
                .chain(&result.failures)
                .any(|(v, _)| v.name == expected);
        }
        assert!(
            caught,
            "{} was not caught by `{expected}` in any profile",
            mutation.name()
        );
    }
}

#[test]
fn a_counterexample_is_the_shortest_trace() {
    // An unchecked reservation overfills a cap of 2 with three entries: three
    // acquires and three reserves cannot be shorter than six steps, and the
    // search reports no longer one.
    let profile = profiles().into_iter().find(|p| p.name == "three").unwrap();
    let result = explore_profile(
        &profile,
        Mutation::ReserveWithoutCheck,
        None,
        limits(),
        false,
    );
    let (violation, trace) = result.failure.expect("a violation");
    assert_eq!(violation.name, "bound");
    assert_eq!(trace.len(), 6, "{trace:?}");
}
