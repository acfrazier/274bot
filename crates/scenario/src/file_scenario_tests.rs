use super::*;
use serde_json::json;

#[test]
fn header_checks_all_roles_before_admitting_the_selected_case() {
    let header = json!({
        "name": "walk-check",
        "revision": 289,
        "cases": ["north", "south"],
        "accounts": {"main": {"mainland": true}, "peer": {"preamble": "tutorial"}},
        "proof": {"is": "bank_closed"},
        "seeds": "confirmed-only"
    });
    let plan = file_scenario(header.clone(), Some("south"), 289, false).unwrap();
    assert_eq!(plan.case, "south");
    assert_eq!(plan.deadline, Duration::from_secs(180));
    assert!(plan.roles["main"].mainland);
    assert_eq!(plan.roles["peer"].preamble, Some(FilePreamble::Tutorial));
    assert!(plan.confirmed_only);
    assert!(file_scenario(header.clone(), Some("east"), 289, false).is_err());
    assert!(file_scenario(header, Some("south"), 377, false).is_err());
}

#[test]
fn static_proof_rejects_unwitnessed_varps_table_state_and_unknown_keys() {
    for proof in [
        json!({"is": "script_idle"}),
        json!({"is": "varp", "id": 1, "min": 0}),
        json!({"is": "varp_exact", "id": 1, "value": 0}),
        json!({"is": "arrived", "x": 3220, "z": 3218, "level": 0, "wrong": 1}),
        json!({"is": "action_sent", "after": 1}),
        json!({"is": "all", "of": []}),
    ] {
        assert!(file_scenario(
            json!({"name": "walk-check", "proof": proof}),
            None,
            289,
            false
        )
        .is_err());
    }
}

#[test]
fn file_proof_uses_native_snapshot_predicates_without_a_table_runner() {
    let proof = json!({"is": "all", "of": [
        {"is": "arrived_ring", "x": 3220, "z": 3218, "level": 0, "min": 1, "max": 2},
        {"is": "bank_item_id", "id": 995, "count": 7},
        {"is": "npc_at", "type": 1, "x": 3220, "z": 3218},
        "tutorial_closed"
    ]});
    assert!(FileCheck::from_literal(proof).unwrap().terminal());
    assert!(FileCheck::from_literal(json!({
        "is": "arrived_ring", "x": 3220, "z": 3218, "level": 0, "min": 3, "max": 2
    }))
    .is_err());
}

#[test]
fn static_named_bank_proof_requires_exact_stand_and_distinct_totals() {
    let standing =
        json!({"is": "standing_bank_closed", "stand": {"x": 3221, "z": 3218, "level": 0}});
    let exact = json!({"is": "bank_exact", "expected": [{"id": 995, "count": 7}]});
    let plan = file_scenario(
        json!({"name": "bank-check", "proof": {"is": "all", "of": [standing, exact]}}),
        None,
        289,
        false,
    );
    assert!(plan.is_ok());
    for proof in [
        json!({"is": "standing_bank_closed", "stand": {"x": 3221, "z": 3218, "level": 4}}),
        json!({"is": "bank_exact", "expected": [{"id": 995, "count": 7}, {"id": 995, "count": 3}]}),
        json!({"is": "bank_exact", "expected": [{"id": 995, "count": 0}]}),
    ] {
        assert!(file_scenario(
            json!({"name": "bank-check", "proof": proof}),
            None,
            289,
            false
        )
        .is_err());
    }
}

#[test]
fn marks_do_not_accept_a_first_send_on_a_foreign_session() {
    let mut log = FileJournal::new();
    let snapshot = GameSnapshot::new();
    log.mark(1, &snapshot, 4, 83).unwrap();
    log.record_action(5, 90, true).unwrap();
    assert!(log.action_sent(1, 5).is_err());
    assert!(log.action_sent(1, 4).is_err());
    assert_eq!(log.mark_at(1).unwrap().source_serial, 83);
}

#[test]
fn action_proof_requires_exactly_one_root_that_actually_sent() {
    let mut log = FileJournal::new();
    log.mark(1, &GameSnapshot::new(), 2, 11).unwrap();
    assert!(!log.action_sent(1, 2).unwrap());
    log.record_action(2, 7, false).unwrap();
    assert!(!log.action_sent(1, 2).unwrap());
    log.note_sent(2, 7).unwrap();
    assert!(log.action_sent(1, 2).unwrap());
    log.mark(2, &GameSnapshot::new(), 2, 12).unwrap();
    log.record_action(2, 8, true).unwrap();
    assert!(log.action_sent(2, 2).unwrap());
    log.record_action(2, 9, true).unwrap();
    assert!(log.action_sent(2, 2).is_err());
}
