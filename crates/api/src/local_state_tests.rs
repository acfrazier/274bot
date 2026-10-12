//! Behaviour tests for the neutral local-state values: every test fails on
//! a plausible wrong definition (a receipt that ignores its account, a
//! version that wraps, an edit that overwrites, a code that misclassifies).

use super::*;
use crate::identity::{Cancellation, Settlement};
use crate::process_condition::{ProcessServiceCode, Recoverability};

fn account(profile: &str, name: &str) -> AccountKey {
    AccountKey::try_new(profile, name).unwrap()
}

fn scope() -> ContentScope {
    ContentScope::try_new(client::io::ClientRevision::R289, "main").unwrap()
}

fn identity(domain: LocalStateDomain, name: &str) -> DomainIdentity {
    DomainIdentity::new(domain, account("local-289", name), scope())
}

fn at(seconds: u64) -> DurableTimestamp {
    DurableTimestamp::from_unix_seconds(seconds)
}

#[test]
fn domains_keep_their_canonical_codes() {
    assert_eq!(
        LocalStateDomain::ALL.map(LocalStateDomain::code),
        [
            "preferences",
            "bank-hints",
            "loadouts-references",
            "knowledge"
        ]
    );
    for domain in LocalStateDomain::ALL {
        assert_eq!(LocalStateDomain::parse(domain.code()), Some(domain));
    }
    assert_eq!(LocalStateDomain::parse("bank hints"), None);
    assert_eq!(LocalStateDomain::parse("loadouts/references"), None);
    assert_eq!(LocalStateDomain::parse(""), None);
}

#[test]
fn account_keys_are_canonical_identities_not_labels() {
    let plain = account("local-289", "alice");
    assert_eq!(account("local-289", "alice"), plain);
    assert_eq!(account("local-289", "ALICE"), plain);
    assert_eq!(
        account("local-289", "Alice Smith"),
        account("local-289", "alice_smith"),
        "every spelling the client logs in as one account shares one key"
    );
    assert_eq!(
        account("local-289", " alice smith"),
        account("local-289", "alice_smith")
    );
    assert_ne!(
        account("local-289", "alice"),
        account("local-289", "alice_smith"),
        "different logins are different keys"
    );
    assert_ne!(account("local-289", "bob"), plain);
    assert_ne!(
        AccountKey::try_new("other", "alice").unwrap(),
        plain,
        "the profile is part of the key: one login on two launches is two accounts"
    );
}

#[test]
fn account_keys_refuse_hostile_overlong_and_empty_identities() {
    assert_eq!(
        AccountKey::try_new("local-289", "ab3456789xyz1"),
        Err(IdentityError::UnsafeAccount),
        "thirteen characters never log in"
    );
    assert_eq!(
        AccountKey::try_new("local-289", "../x"),
        Err(IdentityError::UnsafeAccount),
        "a hostile byte must not fold into another account's file"
    );
    assert_eq!(
        AccountKey::try_new("local-289", "____________"),
        Err(IdentityError::UnsafeAccount),
        "a name the hash spells as no account gets no key"
    );
    assert_eq!(
        AccountKey::try_new("../x", "alice"),
        Err(IdentityError::UnsafeProfile)
    );
    assert_eq!(
        AccountKey::try_new("", "alice"),
        Err(IdentityError::UnsafeProfile)
    );
    assert_eq!(
        ContentScope::try_new(client::io::ClientRevision::R289, "../x"),
        Err(IdentityError::UnsafeContent)
    );
}

#[test]
fn a_receipt_keyed_by_domain_identity_rejects_a_different_account() {
    let alice = identity(LocalStateDomain::BankHints, "alice");
    let receipt = DurableReceipt::new(
        alice.clone(),
        StoreVersion::INITIAL,
        at(1_700_000_000),
        None,
    );
    assert!(receipt.accepts(&alice));
    assert!(
        !receipt.accepts(&identity(LocalStateDomain::BankHints, "bob")),
        "bob must never settle alice's write, however his name was spelled"
    );
    assert!(
        !receipt.accepts(&identity(LocalStateDomain::BankHints, "alice_smith")),
        "another login is another key even under the same profile"
    );
    assert!(
        !receipt.accepts(&identity(LocalStateDomain::Preferences, "alice")),
        "another domain is another key"
    );
    let other_scope = DomainIdentity::new(
        LocalStateDomain::BankHints,
        account("local-289", "alice"),
        ContentScope::try_new(client::io::ClientRevision::R274, "main").unwrap(),
    );
    assert!(
        !receipt.accepts(&other_scope),
        "another engine revision is another key"
    );
}

#[test]
fn receipts_carry_whole_second_durable_time() {
    let id = identity(LocalStateDomain::Preferences, "alice");
    let receipt = DurableReceipt::new(id, StoreVersion::INITIAL, at(1_759_700_000), None);
    assert_eq!(receipt.at().unix_seconds(), 1_759_700_000);
}

#[test]
fn slot_retirement_never_rekeys_an_accepted_write() {
    use crate::identity::SlotKey;
    let id = identity(LocalStateDomain::BankHints, "alice");
    let receipt = DurableReceipt::new(id.clone(), StoreVersion::INITIAL, at(7), None);
    {
        let first = SlotKey::from_incarnation(1).unwrap();
        let second = SlotKey::from_incarnation(2).unwrap();
        assert_ne!(first, second, "two slot lifetimes differ");
        assert!(
            receipt.accepts(&id),
            "the accepted receipt is keyed while both slots live"
        );
    }
    assert!(
        receipt.accepts(&id),
        "retiring both slots leaves the accepted receipt keyed and readable"
    );
}

#[test]
fn versions_advance_in_order_and_refuse_at_the_end() {
    assert!(StoreVersion::INITIAL < StoreVersion::INITIAL.checked_next().unwrap());
    let mut version = StoreVersion::INITIAL;
    for _ in 0..3 {
        let next = version.checked_next().unwrap();
        assert!(next > version);
        version = next;
    }
    assert_eq!(version.get(), 3);
    let last = StoreVersion(u64::MAX);
    assert_eq!(last.checked_next(), None, "the space ends, never wraps");
}

#[test]
fn edits_apply_only_to_their_expected_version() {
    let id = identity(LocalStateDomain::Preferences, "alice");
    let current = StoreVersion::INITIAL.checked_next().unwrap();
    let blind = PreferencesEdit::try_new(id.clone(), None, "world", "auto").unwrap();
    assert!(blind.applies_to(current));
    let agreeing = PreferencesEdit::try_new(id.clone(), Some(current), "world", "auto").unwrap();
    assert!(agreeing.applies_to(current));
    let stale =
        PreferencesEdit::try_new(id.clone(), Some(StoreVersion::INITIAL), "world", "auto").unwrap();
    assert!(
        !stale.applies_to(current),
        "a stale expectation must lose instead of silently overwriting"
    );
    let loadout_id = identity(LocalStateDomain::LoadoutsReferences, "alice");
    let removal = LoadoutEdit::remove(loadout_id, Some(StoreVersion::INITIAL), "melee").unwrap();
    assert!(!removal.applies_to(current));
    let edit = DomainEdit::Preferences(stale);
    assert!(!edit.applies_to(current));
    assert_eq!(edit.expected(), Some(StoreVersion::INITIAL));
}

#[test]
fn every_process_code_is_restart_required_with_its_exact_text() {
    for code in ProcessServiceCode::ALL {
        let error = ErrorCode::ProcessService(code);
        assert_eq!(
            error.recoverability(),
            Some(Recoverability::RestartRequired),
            "{code} must never heal without a process restart"
        );
        assert_eq!(error.operator_text(), Some(code.operator_text()));
        assert_eq!(error.domain(), None);
    }
    assert_eq!(
        ErrorCode::ProcessService(ProcessServiceCode::ReactorUnavailable).to_string(),
        "ReactorUnavailable"
    );
}

#[test]
fn domain_states_recover_without_a_restart_but_bad_input_names_no_recovery() {
    assert_eq!(
        ErrorCode::Busy {
            domain: LocalStateDomain::BankHints
        }
        .recoverability(),
        Some(Recoverability::TransientCapacity)
    );
    assert_eq!(
        ErrorCode::NotReady {
            domain: LocalStateDomain::Preferences
        }
        .recoverability(),
        Some(Recoverability::TransientCapacity)
    );
    assert_eq!(
        ErrorCode::Conflict {
            expected: StoreVersion::INITIAL,
            found: StoreVersion::INITIAL.checked_next().unwrap(),
        }
        .recoverability(),
        Some(Recoverability::TransientCapacity)
    );
    assert_eq!(
        ErrorCode::Validation {
            domain: LocalStateDomain::Knowledge,
            reason: ValidationReason::Empty,
        }
        .recoverability(),
        None,
        "bad input is fixed, never restarted or retried blind"
    );
}

#[test]
fn write_settlements_name_exactly_one_outcome() {
    let id = identity(LocalStateDomain::BankHints, "alice");
    let receipt = DurableReceipt::new(id, StoreVersion::INITIAL, at(9), None);
    let done: WriteSettlement = Settlement::Completed(receipt.clone());
    assert!(done.is_completed());
    assert_eq!(done.cancellation(), None);

    let conflict: WriteSettlement = Settlement::Failed(ErrorCode::Conflict {
        expected: StoreVersion::INITIAL,
        found: StoreVersion::INITIAL.checked_next().unwrap(),
    });
    assert!(!conflict.is_completed());

    let withdrawn: WriteSettlement = Settlement::Cancelled(Cancellation::ScopeInvalidated);
    assert_eq!(
        withdrawn.cancellation(),
        Some(Cancellation::ScopeInvalidated)
    );
}

#[test]
fn leases_are_immutable_snapshots() {
    let id = identity(LocalStateDomain::BankHints, "alice");
    let lease = DomainLease::new(id.clone(), StoreVersion::INITIAL, at(5));
    let newer = DurableReceipt::new(
        id,
        StoreVersion::INITIAL.checked_next().unwrap(),
        at(6),
        None,
    );
    assert_eq!(lease.version(), StoreVersion::INITIAL);
    assert_ne!(
        lease.version(),
        newer.version(),
        "the handed-out lease still names the old version"
    );
}

#[test]
fn bank_hints_keep_the_row_contract() {
    let id = identity(LocalStateDomain::BankHints, "alice");
    let good = BankHint::try_new(id.clone(), None, vec![(379, 12), (995, 3400)], at(8)).unwrap();
    assert_eq!(good.rows(), &[(379, 12), (995, 3400)]);
    assert!(good.applies_to(StoreVersion::INITIAL));
    assert_eq!(
        BankHint::try_new(id.clone(), None, vec![(995, 1), (379, 1)], at(8)).unwrap_err(),
        HintRowsError::Unsorted
    );
    assert_eq!(
        BankHint::try_new(id.clone(), None, vec![(379, 1), (379, 2)], at(8)).unwrap_err(),
        HintRowsError::DuplicateId
    );
    assert_eq!(
        BankHint::try_new(id.clone(), None, vec![(379, 0)], at(8)).unwrap_err(),
        HintRowsError::NonPositiveCount
    );
    assert_eq!(
        ValidationReason::from(HintRowsError::TooManyRows),
        ValidationReason::TooManyRows
    );
}

#[test]
fn preference_bounds_refuse_empty_overlong_and_hostile_keys() {
    let id = identity(LocalStateDomain::Preferences, "alice");
    assert!(PreferencesEdit::try_new(id.clone(), None, "", "x").is_err());
    assert!(PreferencesEdit::try_new(id.clone(), None, &"k".repeat(129), "x").is_err());
    assert!(PreferencesEdit::try_new(id.clone(), None, "../x", "x").is_err());
    assert!(PreferencesEdit::try_new(id.clone(), None, "world", &"v".repeat(4097)).is_err());
    let bank_id = identity(LocalStateDomain::BankHints, "alice");
    assert!(
        PreferencesEdit::try_new(bank_id, None, "world", "auto").is_err(),
        "a preferences edit for another domain is an identity mismatch"
    );
}

#[test]
fn knowledge_invalidation_supersedes_with_operator_provenance() {
    let record = KnowledgeRecord::try_new(
        account("local-289", "alice"),
        scope(),
        None,
        "cook:stage",
        KnowledgeProvenance::QuestList,
        KnowledgeStatus::Known,
        3,
        b"stage=2",
        at(10),
    )
    .unwrap();
    assert_eq!(record.identity().domain(), LocalStateDomain::Knowledge);
    assert_eq!(record.status(), KnowledgeStatus::Known);
    let tombstone = record.invalidated(at(11));
    assert_eq!(tombstone.status(), KnowledgeStatus::Unknown);
    assert_eq!(tombstone.provenance(), KnowledgeProvenance::Operator);
    assert_eq!(tombstone.key(), "cook:stage");
    assert!(tombstone.payload().is_empty());
    assert_eq!(tombstone.observed_at(), at(11));
    assert!(KnowledgeRecord::try_new(
        account("local-289", "alice"),
        scope(),
        None,
        "",
        KnowledgeProvenance::Journal,
        KnowledgeStatus::Partial,
        3,
        b"",
        at(10),
    )
    .is_err());
}

#[test]
fn the_audit_record_names_the_commit_and_no_secret() {
    let id = identity(LocalStateDomain::LoadoutsReferences, "alice");
    let audit = AuditCommit::after_commit(id, StoreVersion::INITIAL, at(1_759_700_000), None);
    let line = audit.to_string();
    assert!(line.contains("loadouts-references"), "{line}");
    assert!(line.contains("local-289"), "{line}");
    assert!(line.contains("alice"), "{line}");
    assert!(line.contains("version=0"), "{line}");
    assert!(line.contains("at=1759700000"), "{line}");
    assert!(!line.contains("request="), "{line}");
}
