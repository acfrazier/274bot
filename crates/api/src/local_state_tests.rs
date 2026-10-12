//! Behaviour tests for the neutral local-state values: every test fails on
//! a plausible wrong definition (a receipt that ignores its account, a
//! version that wraps, an edit that overwrites, a code that misclassifies,
//! a lease that forgets its rows, a tombstone that drops its version, a
//! payload that skips its schema check).

use super::*;
use crate::identity::{Cancellation, RequestId, Settlement};
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

fn request() -> RequestId {
    RequestId::from_raw(7).unwrap()
}

fn knowledge_payload(schema_version: u32, bytes: &[u8]) -> ValidatedKnowledgePayload {
    validate_knowledge_payload(schema_version, bytes).unwrap()
}

fn knowledge_record(expected: Option<StoreVersion>) -> KnowledgeRecord {
    KnowledgeRecord::try_new(
        account("local-289", "alice"),
        scope(),
        expected,
        "cook:stage",
        KnowledgeProvenance::QuestList,
        KnowledgeStatus::Known,
        knowledge_payload(KNOWLEDGE_SCHEMA_VERSION, b"stage=2"),
        at(10),
    )
    .unwrap()
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
fn every_error_code_carries_its_d13_recovery_class() {
    assert_eq!(
        ErrorCode::Busy {
            domain: LocalStateDomain::BankHints
        }
        .recoverability(),
        Some(Recoverability::TransientCapacity),
        "bounded capacity pressure alone retries against a capacity queue"
    );
    assert_eq!(
        ErrorCode::Conflict {
            expected: StoreVersion::INITIAL,
            found: StoreVersion::INITIAL.checked_next().unwrap(),
        }
        .recoverability(),
        None,
        "a conflict retries against the found version, never against a capacity queue"
    );
    assert_eq!(
        ErrorCode::NotReady {
            domain: LocalStateDomain::Preferences
        }
        .recoverability(),
        None,
        "a cold domain retries when the domain is ready, not when a worker queue drains"
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
    for reason in FailureReason::ALL {
        assert_eq!(
            ErrorCode::Failed {
                domain: LocalStateDomain::BankHints,
                reason,
            }
            .recoverability(),
            None,
            "a dirty store stays dirty until an explicit recovery runs"
        );
    }
    for code in ProcessServiceCode::ALL {
        assert_eq!(
            ErrorCode::ProcessService(code).recoverability(),
            Some(Recoverability::RestartRequired),
            "{code} must never heal without a process restart"
        );
    }
}

#[test]
fn durable_failures_are_not_validation_conflict_or_outage() {
    let failed = ErrorCode::Failed {
        domain: LocalStateDomain::BankHints,
        reason: FailureReason::Io,
    };
    assert!(matches!(failed, ErrorCode::Failed { .. }));
    assert!(
        !matches!(failed, ErrorCode::Validation { .. }),
        "a post-admission store failure is not bad input"
    );
    assert!(
        !matches!(failed, ErrorCode::Conflict { .. }),
        "a post-admission store failure is not a lost version race"
    );
    assert!(
        !matches!(failed, ErrorCode::ProcessService(_)),
        "a dirty store is not a latched process outage"
    );
    assert_eq!(failed.domain(), Some(LocalStateDomain::BankHints));
    assert_eq!(failed.operator_text(), None);
    assert_eq!(
        failed.to_string(),
        "durable bank-hints failure: store I/O error"
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
    assert_eq!(conflict.cancellation(), None);

    let failure: WriteSettlement = Settlement::Failed(ErrorCode::Failed {
        domain: LocalStateDomain::BankHints,
        reason: FailureReason::Dirty,
    });
    assert!(!failure.is_completed());
    assert_eq!(failure.cancellation(), None);

    let withdrawn: WriteSettlement = Settlement::Cancelled(Cancellation::ScopeInvalidated);
    assert_eq!(
        withdrawn.cancellation(),
        Some(Cancellation::ScopeInvalidated)
    );

    let superseded: WriteSettlement = Settlement::Cancelled(Cancellation::Superseded);
    assert_eq!(superseded.cancellation(), Some(Cancellation::Superseded));
}

#[test]
fn handed_out_leases_keep_their_rows_after_a_newer_commit() {
    let id = identity(LocalStateDomain::Preferences, "alice");
    let rows = vec![
        PreferenceRow::try_new("world", "auto").unwrap(),
        PreferenceRow::try_new("loot", "keep").unwrap(),
    ];
    let lease = DomainLease::try_new(
        id.clone(),
        StoreVersion::INITIAL,
        at(5),
        LeaseSnapshot::preferences(rows),
    )
    .unwrap();
    let newer = DurableReceipt::new(
        id.clone(),
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
    assert_eq!(
        lease.snapshot().as_preferences().unwrap(),
        &[
            PreferenceRow::try_new("world", "auto").unwrap(),
            PreferenceRow::try_new("loot", "keep").unwrap(),
        ],
        "a newer commit never rewrites the rows a lease already handed out"
    );
    assert_eq!(
        DomainLease::try_new(
            id,
            StoreVersion::INITIAL,
            at(5),
            LeaseSnapshot::bank_hints(vec![(379, 12)]).unwrap(),
        )
        .unwrap_err(),
        ErrorCode::Validation {
            domain: LocalStateDomain::Preferences,
            reason: ValidationReason::Identity,
        },
        "a lease carries the snapshot of its own domain, never another's"
    );
}

/// A hydrating test port: cold until told otherwise, like the T5.2 service
/// while an import or readback is pending.
struct HydratingPort {
    ready: bool,
    version: StoreVersion,
}

impl LocalStatePort for HydratingPort {
    fn lease(&self, identity: &DomainIdentity) -> Result<DomainLease, ErrorCode> {
        if !self.ready {
            return Err(ErrorCode::NotReady {
                domain: identity.domain(),
            });
        }
        DomainLease::try_new(
            identity.clone(),
            self.version,
            at(1),
            LeaseSnapshot::preferences(Vec::new()),
        )
    }

    fn submit(&self, edit: DomainEdit) -> SubmitReceipt {
        SubmitReceipt::Refused(ErrorCode::NotReady {
            domain: edit.identity().domain(),
        })
    }

    fn observe(&self, _request: RequestId) -> Option<WriteSettlement> {
        None
    }
}

#[test]
fn a_cold_domain_is_not_ready_not_an_empty_lease() {
    let id = identity(LocalStateDomain::Preferences, "alice");
    let cold = HydratingPort {
        ready: false,
        version: StoreVersion::INITIAL,
    };
    assert_eq!(
        cold.lease(&id),
        Err(ErrorCode::NotReady {
            domain: LocalStateDomain::Preferences
        }),
        "a cold domain reports typed NotReady, never an empty snapshot"
    );
    let hydrated = HydratingPort {
        ready: true,
        version: StoreVersion::INITIAL.checked_next().unwrap(),
    };
    let lease = hydrated.lease(&id).unwrap();
    assert_eq!(
        lease.version(),
        StoreVersion::INITIAL.checked_next().unwrap()
    );
    assert!(
        lease.snapshot().as_preferences().unwrap().is_empty(),
        "an empty domain that has been read is an empty snapshot at a real version"
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
        ErrorCode::Validation {
            domain: LocalStateDomain::BankHints,
            reason: ValidationReason::Unsorted,
        }
    );
    assert_eq!(
        BankHint::try_new(id.clone(), None, vec![(379, 1), (379, 2)], at(8)).unwrap_err(),
        ErrorCode::Validation {
            domain: LocalStateDomain::BankHints,
            reason: ValidationReason::DuplicateId,
        }
    );
    assert_eq!(
        BankHint::try_new(id.clone(), None, vec![(379, 0)], at(8)).unwrap_err(),
        ErrorCode::Validation {
            domain: LocalStateDomain::BankHints,
            reason: ValidationReason::NonPositiveCount,
        }
    );
    assert_eq!(
        BankHint::try_new(id, None, vec![(1, 1); MAX_HINT_ROWS + 1], at(8)).unwrap_err(),
        ErrorCode::Validation {
            domain: LocalStateDomain::BankHints,
            reason: ValidationReason::TooManyRows,
        }
    );
    assert_eq!(
        ValidationReason::from(HintRowsError::TooManyRows),
        ValidationReason::TooManyRows
    );
}

#[test]
fn bank_hints_refuse_a_foreign_identity() {
    let prefs = identity(LocalStateDomain::Preferences, "alice");
    assert!(
        BankHint::try_new(prefs.clone(), None, vec![(379, 12)], at(8)).is_err(),
        "a preferences identity wrapped in a bank edit stages nothing"
    );
    assert_eq!(
        BankHint::try_new(prefs, None, vec![(379, 12)], at(8)).unwrap_err(),
        ErrorCode::Validation {
            domain: LocalStateDomain::Preferences,
            reason: ValidationReason::Identity,
        },
        "a preferences identity wrapped in a bank edit is an identity mismatch"
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
    let record = knowledge_record(None);
    assert_eq!(record.identity().domain(), LocalStateDomain::Knowledge);
    assert_eq!(record.status(), KnowledgeStatus::Known);
    assert_eq!(record.schema_version(), KNOWLEDGE_SCHEMA_VERSION);
    assert_eq!(record.payload(), b"stage=2");
    let tombstone = record.invalidated(at(11));
    assert_eq!(tombstone.status(), KnowledgeStatus::Unknown);
    assert_eq!(tombstone.provenance(), KnowledgeProvenance::Operator);
    assert_eq!(tombstone.key(), "cook:stage");
    assert!(tombstone.payload().is_empty());
    assert_eq!(tombstone.observed_at(), at(11));
    assert!(
        KnowledgeRecord::try_new(
            account("local-289", "alice"),
            scope(),
            None,
            "",
            KnowledgeProvenance::Journal,
            KnowledgeStatus::Partial,
            knowledge_payload(KNOWLEDGE_SCHEMA_VERSION, b""),
            at(10),
        )
        .is_err()
    );
}

#[test]
fn invalidation_keeps_its_expected_version() {
    let current = StoreVersion::INITIAL
        .checked_next()
        .unwrap()
        .checked_next()
        .unwrap();
    let record = knowledge_record(Some(StoreVersion::INITIAL));
    assert!(record.applies_to(StoreVersion::INITIAL));
    let tombstone = record.invalidated(at(11));
    assert_eq!(tombstone.expected(), Some(StoreVersion::INITIAL));
    assert!(
        !tombstone.applies_to(current),
        "a tombstone built from a stale fact must not overwrite a newer commit"
    );
}

#[test]
fn knowledge_refuses_unknown_schemas_before_any_record_exists() {
    assert_eq!(
        validate_knowledge_payload(3, b"stage=2").unwrap_err(),
        ErrorCode::Validation {
            domain: LocalStateDomain::Knowledge,
            reason: ValidationReason::Schema,
        },
        "an unknown schema is a schema refusal, never a Known record"
    );
    assert_eq!(
        validate_knowledge_payload(
            KNOWLEDGE_SCHEMA_VERSION,
            &[0u8; MAX_KNOWLEDGE_BYTES + 1]
        )
        .unwrap_err(),
        ErrorCode::Validation {
            domain: LocalStateDomain::Knowledge,
            reason: ValidationReason::TooLong,
        }
    );
    let payload = knowledge_payload(KNOWLEDGE_SCHEMA_VERSION, b"stage=2");
    assert_eq!(payload.schema_version(), KNOWLEDGE_SCHEMA_VERSION);
    assert_eq!(payload.bytes(), b"stage=2");
}

#[test]
fn the_audit_record_names_the_commit_and_no_secret() {
    let id = identity(LocalStateDomain::LoadoutsReferences, "alice");
    let committed = StoreVersion::INITIAL.checked_next().unwrap();
    let audit = AuditCommit::after_commit(
        id,
        CommitAction::LoadoutWrite,
        StoreVersion::INITIAL,
        committed,
        AuditOrigin::Submitted { request: request() },
        at(1_759_700_000),
    );
    assert_eq!(
        audit.to_string(),
        "audit domain=loadouts-references profile=local-289 account=alice \
         action=loadout-write 0→1 at=1759700000 request=7"
    );
    let import = AuditCommit::after_commit(
        identity(LocalStateDomain::BankHints, "alice"),
        CommitAction::BankHintWrite,
        StoreVersion::INITIAL,
        StoreVersion::INITIAL,
        AuditOrigin::Import,
        at(8),
    );
    let line = import.to_string();
    assert!(line.contains("action=bank-hint-write"), "{line}");
    assert!(line.contains("0→0"), "{line}");
    assert!(line.contains("origin=import"), "{line}");
    assert!(!line.contains("request="), "{line}");
    assert!(!line.contains("stage=2"), "{line}");
}
