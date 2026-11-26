//! The driver of the L7 stated workload, `l7_workload.luau` (ADR 0023 item
//! 32, Promotion Condition 2).
//!
//! The driver plays the host for one product run: each step is one game tick
//! with its paint. The tile, the run energy and the combat flag change on each
//! tick. The driver answers each request one tick after the run queued it, as
//! a quick native machine does: a walk arrives, a fight kills its target, a
//! bank operation is done. So the script goes through all of its phases on
//! every trip: four walk legs, four fights with a loot pick-up after each kill,
//! then open, deposit and close at the bank.

use api::identity::RequestId;
use script::api_combat::{CombatEnd, CombatSummary, ReportEnd};
use script::interact::InteractReq;
use script::native::request::NativeKind;
use script::native::seat::{SeatResult, SeatTerminal};
use script::native::seat_rows::{SeatPageInput, SeatResultValue};
use script::native::snapshot_source::{NativeFactsInput, QuestStatusInput, TileInput};
use script::native::SettingsBag;
use script::surface::{LentOutcome, SurfaceRun, SurfaceWake};

/// The workload source, with its manifest statement.
pub const SOURCE: &str = include_str!("l7_workload.luau");

/// The steps of one trip. A request takes two steps: the run queues it on
/// one tick and resumes with its answer on the next, then its tick entry
/// ends. Four walks and four fights take eight entries of two steps each. The
/// bank entry queues three requests in a row and takes four steps.
pub const TICKS_PER_TRIP: u64 = 20;
/// The tick entries of one trip: four walks, four fights, one bank visit.
pub const ENTRIES_PER_TRIP: u64 = 9;

const QUESTS: &[QuestStatusInput<'static>] = &[
    QuestStatusInput {
        name: "Cook's Assistant",
        status: "inProgress",
        component_id: Some(7333),
    },
    QuestStatusInput {
        name: "Sheep Shearer",
        status: "complete",
        component_id: Some(7344),
    },
];

/// The settings bag of a Start. It holds the manifest defaults.
pub fn settings() -> SettingsBag {
    serde_json::from_value(serde_json::json!({
        "target": "Cow",
        "loot": "Cowhide",
        "lootId": 1739,
        "tripKills": 4,
        "paintRoute": true,
        "quest": "Cook's Assistant",
    }))
    .expect("workload settings")
}

/// The request that the run waits for.
enum Owed {
    Nothing,
    Walk { id: u64, x: i32, z: i32, level: i32 },
    Seat { id: RequestId, kind: NativeKind },
}

/// The requests that the driver answered, by kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Answered {
    pub walks: u64,
    pub fights: u64,
    pub banks: u64,
    pub pickups: u64,
    pub logs: u64,
}

pub struct Driver {
    tick: u64,
    owed: Owed,
    walk_seq: u64,
    seat_seq: u64,
    bank_value: Vec<u8>,
    pub answered: Answered,
}

impl Default for Driver {
    fn default() -> Self {
        Self::new()
    }
}

impl Driver {
    pub fn new() -> Self {
        Self {
            tick: 0,
            owed: Owed::Nothing,
            walk_seq: 0,
            seat_seq: 0,
            bank_value: script::channel::encode(&serde_json::json!(true))
                .expect("bank result value"),
            answered: Answered::default(),
        }
    }

    /// One game tick with its paint, then the host's own drains.
    pub fn step(&mut self, run: &mut dyn SurfaceRun) {
        self.tick += 1;
        let tick = self.tick;
        let mut input = crate::support::snapshot(tick);
        input.here = Some(TileInput {
            x: 3240 + (tick % 24) as i32,
            z: 3260 + (tick / 24 % 30) as i32,
            level: 0,
        });
        input.run_energy = (tick % 101) as i32;
        input.in_combat = tick.is_multiple_of(3);
        input.my_name = Some("alice");

        let mut native = NativeFactsInput {
            quest_statuses: Some(QUESTS),
            ..Default::default()
        };
        let combat_end;
        let terminal;
        let values;
        match std::mem::replace(&mut self.owed, Owed::Nothing) {
            Owed::Nothing => {}
            Owed::Walk { id, x, z, level } => {
                self.walk_seq += 1;
                self.answered.walks += 1;
                native.walk_outcome_seq = self.walk_seq;
                native.walk_outcome_request_id = id;
                native.walk_outcome_x = x;
                native.walk_outcome_z = z;
                native.walk_outcome_level = level;
                native.local_player_moving = Some(false);
            }
            Owed::Seat { id, kind } => {
                self.seat_seq += 1;
                terminal = [SeatResult {
                    id,
                    seq: self.seat_seq,
                    outcome: SeatTerminal::Done,
                    prior_parent: None,
                    session: None,
                    authority_revoked: false,
                }];
                let page = SeatPageInput::new(&[], &terminal, &[], 0).expect("seat page");
                match kind {
                    NativeKind::Combat => {
                        self.answered.fights += 1;
                        combat_end = CombatEnd::Fought {
                            token: id.get(),
                            report: CombatSummary {
                                end: ReportEnd::Killed,
                                reason: None,
                                npc_type: 81,
                                ticks: 14,
                                swings: 4,
                                casts: 0,
                                damage_taken: 1,
                                food: 0,
                                prayer_doses: 0,
                                boost_doses: 0,
                                antifire_doses: 0,
                                protect_switches: 0,
                                pvp_source: None,
                            },
                        };
                        native.api_combat_outcome = Some(&combat_end);
                        native.seat_page = Some(page);
                    }
                    NativeKind::BankPlan => {
                        self.answered.banks += 1;
                        values = [SeatResultValue {
                            id,
                            value: &self.bank_value,
                        }];
                        native.seat_page =
                            Some(page.with_result_values(&values).expect("bank result value"));
                    }
                    other => panic!("the workload starts no {other:?} request"),
                }
            }
        }

        let outcome = run.dispatch_lent(
            SurfaceWake::Tick { held: false },
            tick,
            tick,
            &input,
            native,
        );
        assert_eq!(outcome, LentOutcome::Accepted, "workload tick {tick}");
        for queued in run.drain_seat_batches() {
            for start in &queued.batch.starts {
                self.owe(Owed::Seat {
                    id: start.id,
                    kind: start.kind,
                });
            }
            for row in &queued.batch.reqs {
                match &row.request {
                    InteractReq::Walk {
                        request_id,
                        x,
                        z,
                        level,
                        ..
                    } => self.owe(Owed::Walk {
                        id: *request_id,
                        x: *x,
                        z: *z,
                        level: *level,
                    }),
                    InteractReq::Obj { .. } => self.answered.pickups += 1,
                    _ => {}
                }
            }
        }
        self.answered.logs += run.drain_logs().len() as u64;
        drop(run.drain_tick_outcomes());
        assert!(run.paint().is_some(), "workload tick {tick} painted");
    }

    fn owe(&mut self, owed: Owed) {
        assert!(
            matches!(self.owed, Owed::Nothing),
            "the workload queued two requests in one tick"
        );
        self.owed = owed;
    }
}

/// VM used bytes of a live run.
pub fn used_bytes(run: &dyn SurfaceRun) -> u64 {
    run.memory_metrics().expect("live VM")["used_bytes"]
        .as_u64()
        .expect("used bytes")
}
