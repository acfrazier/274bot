#![cfg(feature = "load")]

mod common;

use api::quest_progress::{EvidenceStamp, ProgressFlag};
use api::selected::{FactKey, Gap, Knowledge, RunKey, Truth};
use api::snapshot::QuestListStatus;
use script::api_progress::{ProgressPage, QuestPathRow, QuestProgressRow};
use script::isolate_fb::{
    encode_snapshot_delta_with_native, NativeFactsInput, Snapshot, SnapshotFingerprint,
};
use script::observed;
use std::sync::Arc;

fn progress_row() -> QuestProgressRow {
    QuestProgressRow {
        quest: Arc::from("cook"),
        display: Arc::from("Cook's Assistant"),
        colour: QuestListStatus::InProgress,
        stage: Knowledge::Known(Arc::from("cook:2")),
        rule: Knowledge::Unknown(Gap {
            code: Arc::from("journal-no-match"),
            sources: Arc::from([]),
        }),
        complete: Truth::False,
        flags: vec![
            ProgressFlag {
                flag: FactKey::new("delivered-ingredients"),
                truth: Truth::True,
                count: None,
            },
            ProgressFlag {
                flag: FactKey::new("collected-eggs"),
                truth: Truth::True,
                count: Some(999_999_999),
            },
        ]
        .into(),
        evidence: EvidenceStamp {
            run: RunKey {
                slot: 0,
                run: 7,
                session: 3,
            },
            tick: 41,
            sequence: 52,
        },
        journal_read: true,
        binding: Arc::from("cook"),
        role: Some(Arc::from("cook-assistant")),
    }
}

fn encode_page(
    tick: u64,
    page: Option<&ProgressPage>,
    last: Option<&SnapshotFingerprint>,
) -> (Vec<u8>, SnapshotFingerprint) {
    let mut input = common::ingame_snapshot();
    input.tick = tick;
    encode_snapshot_delta_with_native(
        last,
        &input,
        NativeFactsInput {
            api_progress: page,
            ..NativeFactsInput::default()
        },
        false,
    )
}

#[test]
fn progress_pages_round_trip_and_public_json_uses_the_design_shape() {
    let row = Arc::new(progress_row());
    let page = ProgressPage::Done {
        token: 17,
        row: Arc::clone(&row),
    };
    let (bytes, _) = encode_page(1, Some(&page), None);
    let snapshot = Snapshot::from_bytes(&bytes).expect("progress keyframe verifies");
    assert!(snapshot.has_api_progress());
    let wire = snapshot.api_progress().expect("progress page present");
    assert_eq!((wire.request_id(), wire.kind()), (17, 2));
    let wire_row = wire.row().expect("done page contains row");
    assert_eq!(wire_row.quest(), Some("cook"));
    assert_eq!(wire_row.display(), Some("Cook's Assistant"));
    assert_eq!(wire_row.colour(), 2);
    assert_eq!(wire_row.stage(), Some("cook:2"));
    assert_eq!(wire_row.stage_gap(), Some(""));
    assert_eq!(wire_row.rule(), Some(""));
    assert_eq!(wire_row.rule_gap(), Some("journal-no-match"));
    assert_eq!(wire_row.complete(), 2);
    assert_eq!(
        (
            wire_row.evidence_run(),
            wire_row.evidence_session(),
            wire_row.evidence_tick(),
            wire_row.evidence_sequence(),
        ),
        (7, 3, 41, 52)
    );
    let flags = wire_row.flags().expect("flag vector present");
    assert_eq!(flags.len(), 2);
    assert_eq!(flags.get(0).count(), -1, "no count uses the -1 wire sentinel");
    assert_eq!(flags.get(1).count(), 999_999_999, "nine-digit bound fits");

    observed::on_reset();
    observed::apply(&snapshot);
    assert_eq!(
        observed::with(|scene| scene.since_login().api_progress().cloned()),
        Some(page)
    );

    let public = serde_json::Value::from(ProgressPage::Done { token: 17, row });
    assert_eq!(
        public,
        serde_json::json!({
            "end": "done",
            "token": 17,
            "row": {
                "quest": "cook",
                "display": "Cook's Assistant",
                "colour": "inProgress",
                "stage": {"state": "known", "value": "cook:2"},
                "complete": "false",
                "rule": {"state": "unknown", "gap": "journal-no-match"},
                "flags": [
                    {"flag": "delivered-ingredients", "truth": "true", "count": null},
                    {"flag": "collected-eggs", "truth": "true", "count": 999_999_999},
                ],
                "evidence": {"run": 7, "session": 3, "tick": 41, "sequence": 52},
                "journal_read": true,
                "binding": "cook",
                "role": "cook-assistant",
            }
        })
    );
}

#[test]
fn progress_page_deltas_omit_keep_replace_and_reset_as_a_keyframe() {
    let reading = ProgressPage::Reading { token: 31 };
    let (keyframe, mut fingerprint) = encode_page(1, Some(&reading), None);
    let keyframe = Snapshot::from_bytes(&keyframe).expect("reading keyframe verifies");
    let wire = keyframe.api_progress().unwrap();
    assert_eq!(wire.kind(), 1);
    assert!(wire.reason().is_none(), "reading page allocates no reason");
    assert!(wire.row().is_none(), "reading page allocates no row");
    observed::on_reset();
    observed::apply(&keyframe);

    let (unchanged, next) = encode_page(2, Some(&reading), Some(&fingerprint));
    fingerprint = next;
    let unchanged = Snapshot::from_bytes(&unchanged).expect("unchanged delta verifies");
    assert!(!unchanged.has_api_progress(), "unchanged page is omitted");
    observed::apply(&unchanged);
    assert_eq!(
        observed::with(|scene| scene.since_login().api_progress().cloned()),
        Some(reading)
    );

    let done = ProgressPage::Done {
        token: 31,
        row: Arc::new(progress_row()),
    };
    let (terminal, next) = encode_page(3, Some(&done), Some(&fingerprint));
    fingerprint = next;
    let terminal = Snapshot::from_bytes(&terminal).expect("terminal replacement verifies");
    assert!(terminal.has_api_progress(), "kind change replaces reading page");
    assert_eq!(terminal.api_progress().unwrap().kind(), 2);
    observed::apply(&terminal);

    let (omitted, next) = encode_page(4, None, Some(&fingerprint));
    fingerprint = next;
    let omitted = Snapshot::from_bytes(&omitted).expect("omitted delta verifies");
    assert!(!omitted.has_api_progress());
    observed::apply(&omitted);
    assert_eq!(
        observed::with(|scene| scene.since_login().api_progress().cloned()),
        Some(done)
    );

    let refused = ProgressPage::Refused {
        token: 32,
        reason: Arc::from("unknown-path"),
    };
    let (replacement, _) = encode_page(5, Some(&refused), Some(&fingerprint));
    let replacement = Snapshot::from_bytes(&replacement).expect("refused replacement verifies");
    assert!(replacement.has_api_progress());
    let wire = replacement.api_progress().unwrap();
    assert_eq!((wire.request_id(), wire.kind()), (32, 3));
    assert_eq!(wire.reason(), Some("unknown-path"));
    assert!(wire.row().is_none(), "refusal page allocates no row");
    observed::apply(&replacement);
    assert_eq!(
        observed::with(|scene| scene.since_login().api_progress().cloned()),
        Some(refused)
    );

    observed::on_reset();
    let (new_keyframe, _) = encode_page(6, None, None);
    let new_keyframe = Snapshot::from_bytes(&new_keyframe).expect("empty keyframe verifies");
    assert!(!new_keyframe.has_api_progress(), "absent page stays absent on keyframe");
    observed::apply(&new_keyframe);
    assert!(observed::with(|scene| scene.since_login().api_progress().is_none()));
}

#[test]
fn progress_refusal_and_path_rows_keep_their_public_shapes() {
    let refused = serde_json::Value::from(ProgressPage::Refused {
        token: 9,
        reason: Arc::from("busy"),
    });
    assert_eq!(
        refused,
        serde_json::json!({"end": "refused", "token": 9, "reason": "busy"})
    );
    assert_eq!(
        serde_json::Value::from(ProgressPage::Reading { token: 9 }),
        serde_json::Value::Null
    );

    let path = QuestPathRow {
        id: Arc::from("cook"),
        display: Arc::from("Cook's Assistant"),
        journal: false,
        stages: vec![Arc::from("cook:0"), Arc::from("cook:1")].into(),
    };
    assert_eq!(
        serde_json::to_value(path).expect("path row serializes"),
        serde_json::json!({
            "id": "cook",
            "display": "Cook's Assistant",
            "journal": false,
            "stages": ["cook:0", "cook:1"]
        })
    );
}
