use super::*;

fn thiever_watch() -> CoreWatch {
    let watch = CoreWatch::default();
    watch.configure(CoreCase::Thiever, "catalogtest");
    watch
}

#[test]
fn paired_proofs_refuse_to_run_without_the_pair_gate() {
    for case in PairCase::headed_cells() {
        let name = case.scenario_name();
        let error = LiveCore::resolve(name, 2, false, false).unwrap_err();
        assert!(error.contains("pair core gate"), "{name}: {error}");
        assert_eq!(
            LiveCore::resolve(name, 2, false, true),
            Ok(LiveCore::Pair(case))
        );
        assert!(
            LiveCore::resolve(name, 2, true, false).is_err(),
            "{name}: the catalog gate cannot stand in for the pair witness"
        );
    }
}

#[test]
fn live_core_resolves_single_gates_and_rejects_mixed_or_mis_sized_runs() {
    assert_eq!(
        LiveCore::resolve("thiever", 1, false, false),
        Ok(LiveCore::Off)
    );
    assert_eq!(
        LiveCore::resolve("thiever", 1, true, false),
        Ok(LiveCore::Catalog(CoreCase::Thiever))
    );
    assert!(LiveCore::resolve("thiever", 1, true, true)
        .unwrap_err()
        .contains("mutually exclusive"));
    assert!(LiveCore::resolve("thiever", 2, true, false)
        .unwrap_err()
        .contains("exactly one driven slot"));
    assert!(LiveCore::resolve("nature_crafter_air", 1, false, true)
        .unwrap_err()
        .contains("exactly two driven slots"));
    assert!(LiveCore::resolve("not_a_core_case", 1, true, false).is_err());
}

#[test]
fn catalog_gate_rejects_scenario_only_pass_and_fails_at_the_deadline() {
    let watch = thiever_watch();
    let deadline = Instant::now() + Duration::from_secs(30);

    assert_eq!(
        catalog_core_gate(Some(&watch), Some(deadline), Instant::now()),
        CoreGate::Pending
    );
    let CoreGate::Failed(message) = catalog_core_gate(
        Some(&watch),
        Some(deadline),
        deadline + Duration::from_secs(1),
    ) else {
        panic!("an unqualified core must fail at its deadline");
    };
    assert!(
        message.contains("catalog core did not qualify"),
        "{message}"
    );
    assert_eq!(
        catalog_core_gate(Some(&CoreWatch::default()), None, Instant::now()),
        CoreGate::Disabled
    );
}

#[test]
fn core_deadline_prefers_the_soak_budget_and_needs_a_configured_watch() {
    let now = Instant::now();
    let deadline = Duration::from_secs(300);
    let budget = Duration::from_secs(900);
    let idle = CoreWatch::default();
    assert_eq!(
        core_deadline(
            Some(&idle),
            Some(&PairWatch::default()),
            None,
            deadline,
            now
        ),
        None
    );
    let watch = thiever_watch();
    assert_eq!(
        core_deadline(Some(&watch), None, None, deadline, now),
        Some(now + deadline)
    );
    assert_eq!(
        core_deadline(Some(&watch), None, Some(budget), deadline, now),
        Some(now + budget)
    );
}

#[test]
fn pending_core_holds_pass_without_starting_the_clean_stop_grace() {
    const STOP: &str = "cell qualification complete";
    let start = Instant::now();
    let mut grace = None;
    // A Pending witness holds PASS however long it takes; the grace clock
    // must not run meanwhile.
    for elapsed in [0, 30, 120] {
        let now = start + Duration::from_secs(elapsed);
        assert_eq!(
            pass_hold(
                &[&CoreGate::Pending, &CoreGate::Disabled],
                Some(STOP),
                |_| false,
                &mut grace,
                now
            ),
            PassHold::Core
        );
        assert_eq!(grace, None);
    }
    // Once the witness qualifies the grace starts from that poll, so a
    // slow witness cannot eat the card's clean-stop time.
    let settled = start + Duration::from_secs(120);
    let qualified = CoreGate::Qualified(None);
    assert_eq!(
        pass_hold(&[&qualified], Some(STOP), |_| false, &mut grace, settled),
        PassHold::CleanStop
    );
    assert_eq!(grace, Some(settled));
    assert_eq!(
        pass_hold(
            &[&qualified],
            Some(STOP),
            |_| false,
            &mut grace,
            settled + SCRIPT_STOP_WAIT - Duration::from_millis(1)
        ),
        PassHold::CleanStop
    );
    let PassHold::TimedOut(message) = pass_hold(
        &[&qualified],
        Some(STOP),
        |_| false,
        &mut grace,
        settled + SCRIPT_STOP_WAIT,
    ) else {
        panic!("the grace lapses after SCRIPT_STOP_WAIT");
    };
    assert!(message.contains(STOP), "{message}");
}

#[test]
fn clean_stop_or_no_named_stop_releases_pass() {
    let now = Instant::now();
    let mut grace = None;
    assert_eq!(
        pass_hold(&[&CoreGate::Disabled], None, |_| false, &mut grace, now),
        PassHold::Release { clean_stop: false }
    );
    assert_eq!(
        pass_hold(
            &[&CoreGate::Qualified(None)],
            Some("walk spot qualification complete"),
            |needle| needle == "walk spot qualification complete",
            &mut grace,
            now
        ),
        PassHold::Release { clean_stop: true }
    );
    assert_eq!(grace, None, "an observed stop never starts the grace");
}

#[test]
fn script_self_stop_requires_idle_and_receipt_reason() {
    let receipt = script::ScriptLifecycleReceipt {
        runtime_generation: 1,
        state: script::ScriptTerminalState::Stopped,
        tick: 9,
        reason: "confirmed loaded current-generation bank exhaustion".into(),
    };
    assert!(script_self_stop_observed(
        script::RunState::Idle,
        Some(&receipt),
        "confirmed loaded current-generation bank exhaustion"
    ));
    assert!(!script_self_stop_observed(
        script::RunState::Running,
        Some(&receipt),
        "confirmed loaded current-generation bank exhaustion"
    ));
    assert!(!script_self_stop_observed(
        script::RunState::Idle,
        Some(&receipt),
        "stalled while bury"
    ));
    assert!(!script_self_stop_observed(
        script::RunState::Idle,
        None,
        "confirmed"
    ));
}
