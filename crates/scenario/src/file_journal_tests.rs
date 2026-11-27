use super::*;
use api::snapshot::{ChatLineView, GameSnapshot, StatView};

fn stats(xp: i32, base: i32, effective: i32) -> GameSnapshot {
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_stats(vec![StatView {
        index: 0,
        name: "attack".into(),
        effective,
        base,
        xp,
        used: true,
    }]);
    snapshot
}

#[test]
fn value_observation_records_only_sampled_endpoint_differences() {
    let mut journal = FileJournal::new();
    let start = stats(100, 1, 1);
    journal.observe(&start, 7).unwrap();

    // An A→B→A batch that reaches the snapshot only as A has no value record.
    journal.observe(&start, 7).unwrap();
    assert_eq!(journal.record_count(RecordKind::Stat), 0);

    // The journal sees only the posted endpoint, so it emits one A→C record.
    let end = stats(300, 3, 4);
    journal.observe(&end, 7).unwrap();
    let rows: Vec<_> = journal
        .records()
        .filter(|record| record.kind() == RecordKind::Stat)
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].session, 7);
    assert_eq!(rows[0].tick, Some(0));
    assert!(matches!(
        rows[0].data,
        JournalData::Stat {
            skill: 0,
            before: Some(StatValues {
                base: Some(1),
                effective: Some(1),
                xp: Some(100)
            }),
            after: StatValues {
                base: Some(3),
                effective: Some(4),
                xp: Some(300)
            },
        }
    ));
}

#[test]
fn unchanged_observation_allocates_nothing_after_baseline() {
    let mut journal = FileJournal::new();
    let snapshot = stats(100, 1, 1);
    journal.observe(&snapshot, 9).unwrap();
    let capacity = journal.capacity();

    let measured = allocation_counter::measure(|| {
        for _ in 0..128 {
            journal.observe(&snapshot, 9).unwrap();
        }
    });
    assert_eq!(measured.count_total, 0);
    assert_eq!(measured.bytes_total, 0);
    assert_eq!(journal.capacity(), capacity);
}

#[test]
fn packet_records_keep_each_intermediate_and_repeated_value() {
    let mut journal = FileJournal::new();
    journal.set_packet_provider(PacketKinds {
        stats: true,
        inventories: false,
        vars: true,
    });
    for (source_serial, application_serial, value) in [(1, 10, 0), (2, 11, 1), (3, 12, 0)] {
        journal
            .observe_packet(PacketRecord::Varp {
                id: 17,
                value,
                source_serial,
                application_serial,
                client_session: 4,
            })
            .unwrap();
    }

    let rows: Vec<_> = journal
        .records()
        .filter(|record| record.kind() == RecordKind::Varp)
        .collect();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows.iter().map(|record| record.serial).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert!(rows
        .iter()
        .all(|record| record.source == RecordSource::Packet));
    assert!(rows.iter().all(|record| record.tick.is_none()));
}

#[test]
fn packet_gap_is_watermarked_without_advancing_serial_and_later_marks_are_fresh() {
    let mut journal = FileJournal::new();
    journal.set_packet_provider(PacketKinds {
        stats: true,
        inventories: true,
        vars: true,
    });
    let snapshot = stats(100, 1, 1);
    journal.observe(&snapshot, 7).unwrap();
    journal.mark(1, &snapshot, 7, 0).unwrap();
    let serial = journal.last_serial();

    journal.note_packet_gap(PacketKinds {
        stats: true,
        inventories: true,
        vars: true,
    });
    assert_eq!(journal.last_serial(), serial);
    assert_eq!(journal.packet_gaps.map(|gap| gap.serial), [serial; 3]);
    assert!(journal.has_packet_gap_since(journal.mark_any(1).unwrap(), RecordKind::Stat));

    journal.mark(2, &snapshot, 7, 0).unwrap();
    assert!(!journal.has_packet_gap_since(journal.mark_any(2).unwrap(), RecordKind::Stat));
}

#[test]
fn mark_watermark_is_exclusive_and_evicted_history_fails_closed() {
    let mut journal = FileJournal::new();

    journal.set_packet_provider(PacketKinds {
        stats: true,
        inventories: false,
        vars: false,
    });
    let snapshot = stats(100, 1, 1);
    journal.observe(&snapshot, 3).unwrap();
    journal
        .observe_packet(PacketRecord::Stat {
            skill: 0,
            level: 2,
            xp: 200,
            source_serial: 1,
            application_serial: 1,
            client_session: 3,
        })
        .unwrap();
    let mark = journal.mark(1, &snapshot, 3, 0).unwrap();
    assert_eq!(mark.serial, 1);
    assert_eq!(
        journal.records_after(RecordKind::Stat, mark.serial).count(),
        0
    );

    for serial in 2..=(MAX_RECORDS as u64 + 2) {
        journal
            .observe_packet(PacketRecord::Stat {
                skill: 0,
                level: 2,
                xp: serial as i32,
                source_serial: serial,
                application_serial: serial,
                client_session: 3,
            })
            .unwrap();
    }
    assert_eq!(journal.record_count(RecordKind::Stat), MAX_RECORDS);
    assert!(journal.history_evicted(RecordKind::Stat, mark.serial));
}

#[test]
fn value_varps_are_not_inferred_from_allocated_snapshot_rows() {
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_varps(vec![api::snapshot::VarpView {
        index: 17,
        value: 0,
    }]);
    let mut journal = FileJournal::new();
    journal.observe(&snapshot, 5).unwrap();
    assert_eq!(journal.record_count(RecordKind::Varp), 0);
}

#[test]
fn new_chat_lines_keep_their_sequence_and_exact_payload() {
    let mut journal = FileJournal::new();
    let mut before = GameSnapshot::new();
    before.seed_ingame(2);
    before.seed_chat_lines(vec![ChatLineView {
        type_: 0,
        username: None,
        text: "old".into(),
        sequence: 4,
    }]);
    journal.observe(&before, 2).unwrap();

    let mut after = GameSnapshot::new();
    after.seed_ingame(2);
    after.seed_chat_lines(vec![
        ChatLineView {
            type_: 0,
            username: None,
            text: "newer".into(),
            sequence: 6,
        },
        ChatLineView {
            type_: 1,
            username: Some("alice".into()),
            text: "new".into(),
            sequence: 5,
        },
        ChatLineView {
            type_: 0,
            username: None,
            text: "old".into(),
            sequence: 4,
        },
    ]);
    journal.observe(&after, 2).unwrap();

    let rows: Vec<_> = journal
        .records()
        .filter_map(|record| match &record.data {
            JournalData::Message { sequence, .. } => Some(*sequence),
            _ => None,
        })
        .collect();
    assert_eq!(rows, vec![5, 6]);
}

#[test]
fn session_change_is_a_record_and_restarts_the_sample_baseline() {
    let mut journal = FileJournal::new();
    journal.observe(&stats(100, 1, 1), 1).unwrap();
    journal.observe(&stats(900, 9, 9), 2).unwrap();

    assert_eq!(journal.record_count(RecordKind::Stat), 0);
    assert_eq!(journal.record_count(RecordKind::Session), 1);
    assert_eq!(journal.records().last().unwrap().session, 2);
}
