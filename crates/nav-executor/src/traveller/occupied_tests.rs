//! Agility crossings the server refuses because another player holds the
//! obstacle, and the transport clock that counts observed game ticks.

use super::*;
use crate::TransportRefusal;
use std::cell::RefCell;
use std::rc::Rc;

const LEDGE_BUSY: &str = "The ledge is being used";
const ROPE_BUSY: &str = "The rope swing is being used";

/// The Yanille ledge bakes 9 ticks (`agility_dungeon.rs2`: 8 forced tiles
/// and a final `p_delay`); the rope swings bake 2. The crossing lands 8
/// tiles from the stand, as the ledge does.
fn crossing_edge(ticks: i32) -> TransportEdge {
    TransportEdge {
        kind: TransportKind::AgilityShortcut,
        ticks,
        to: WorldTile {
            x: 3210,
            z: 3204,
            level: 0,
        },
        ..ladder_edge()
    }
}

/// The agility loc at scene (3, 4), a player standing next to it, and the
/// follow run's state.
struct Crossing {
    c: Client,
    snap: GameSnapshot,
    rec: FollowRec,
    t: Traveller,
    route: Route,
}

impl Crossing {
    fn at(edge: TransportEdge, stand: (i32, i32)) -> Self {
        let mut c = scene_client();
        plant_ladder(&mut c, Some("Walk-across"));
        let snap = snap_at(&mut c, stand.0, stand.1);
        Self {
            rec: FollowRec {
                route: Some(stand),
                ..FollowRec::default()
            },
            t: Traveller::new(),
            route: web_route(edge),
            c,
            snap,
        }
    }

    fn new(edge: TransportEdge) -> Self {
        Self::at(edge, (2, 4))
    }

    /// One poll of the follow on the current snapshot.
    fn poll(&mut self, options: &mut TravelOptions<'_>) -> Option<TravelOutcome> {
        self.t
            .follow(&mut self.rec, &self.snap, self.route.clone(), options)
    }

    /// Observe the next game tick.
    fn tick(&mut self) {
        bump_rebuild(&mut self.c, &mut self.snap);
    }

    /// Observe the next `packets` game ticks in one snapshot rebuild: a
    /// follower that polls less often than the game ticks.
    fn tick_batch(&mut self, packets: u32) {
        self.c.bump_gens(ServerProt::REBUILD_NORMAL);
        for _ in 0..packets {
            self.c.bump_gens(ServerProt::PLAYER_INFO);
        }
        self.snap.rebuild(&self.c);
    }

    /// Observe the next game tick, then poll once.
    fn next_poll(&mut self, options: &mut TravelOptions<'_>) -> Option<TravelOutcome> {
        self.tick();
        self.poll(options)
    }

    /// The server accepts the crossing: the player lands on `to`, and the
    /// run ends once the packed delay has elapsed.
    fn land(&mut self, options: &mut TravelOptions<'_>) -> TravelOutcome {
        plant_player(&mut self.c, 10, 4);
        drive(
            &mut self.t,
            &mut self.rec,
            &mut self.c,
            &mut self.snap,
            &self.route,
            options,
            |_| {},
        )
    }
}

/// `~mesbox` page: a chat modal with the text and its continue button.
fn plant_mesbox(c: &mut Client, text: &str) {
    let root = 100;
    c.set_iface(
        102,
        IfType {
            id: 102,
            layer_id: root,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    c.set_iface_mut(
        102,
        IfTypeMut {
            text: text.into(),
            ..Default::default()
        },
    );
    plant_continue_dialog(c);
    c.set_iface(
        root as usize,
        IfType {
            id: root,
            layer_id: root,
            children: Some(vec![101, 102]),
            ..Default::default()
        },
    );
    c.bump_gens(ServerProt::IF_OPENCHAT);
}

#[test]
fn busy_ledge_is_retried_after_its_content_hold_and_arrives() {
    let mut x = Crossing::new(crossing_edge(9));
    let mut options = TravelOptions::default();
    assert!(x.poll(&mut options).is_none());
    assert_eq!(x.rec.loc_ops, 1, "the crossing is sent once");

    // `agility_dungeon.rs2:53-56`: another player holds the ledge.
    x.c.add_chat(0, LEDGE_BUSY, "");
    assert!(x.next_poll(&mut options).is_none(), "the refusal is seen");
    // The reservation lasts at most 8 ticks past its accept: the retry
    // waits 9 observed ticks from the refusal.
    for held in 1..9 {
        assert!(x.next_poll(&mut options).is_none());
        assert_eq!(x.rec.loc_ops, 1, "still holding at tick {held} of 9");
    }
    assert!(x.next_poll(&mut options).is_none());
    assert_eq!(x.rec.loc_ops, 2, "sent again when the hold lapsed");

    let outcome = x.land(&mut options);
    assert_eq!(
        outcome,
        TravelOutcome::Arrived {
            at: crossing_edge(9).to
        }
    );
    assert_eq!(x.rec.loc_ops, 2, "the accepted retry is not sent twice");
}

#[test]
fn busy_ledge_past_the_budget_ends_with_the_actual_try_count_and_last_refusal() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&events);
    let mut options = TravelOptions {
        on_event: Some(Box::new(move |event| {
            if let TravelEvent::TransportBusy {
                message,
                resume_tick,
                tries,
                ..
            } = event
            {
                sink.borrow_mut().push((message, resume_tick, tries));
            }
        })),
        ..TravelOptions::default()
    };
    let edge = crossing_edge(9);
    let mut x = Crossing::new(edge.clone());
    let mut seen = 0;
    let mut ticks = 0;
    let outcome = loop {
        if let Some(outcome) = x.poll(&mut options) {
            break outcome;
        }
        if x.rec.loc_ops > seen {
            seen = x.rec.loc_ops;
            x.c.add_chat(0, LEDGE_BUSY, "");
        }
        x.tick();
        ticks += 1;
        assert!(ticks < 400, "the retries must be finite");
    };
    // 60 ticks of budget, 9 per refusal: six waits fit, the seventh refusal
    // ends the hop. Seven sends reached the ledge.
    assert_eq!(
        outcome,
        TravelOutcome::Stalled {
            at: WorldTile {
                x: 3202,
                z: 3204,
                level: 0
            },
            aiming: edge.to,
            why: HopFailure::Dropped,
            tries: 7,
            last_refusal: Some(TransportRefusal::Busy(LEDGE_BUSY)),
        }
    );
    assert_eq!(x.rec.loc_ops, 7);
    let events = events.borrow();
    assert_eq!(events.len(), 7);
    assert!(events[..6]
        .iter()
        .enumerate()
        .all(|(i, (message, resume, tries))| *message == Some(LEDGE_BUSY)
            && resume.is_some()
            && *tries == i as u32 + 1));
    assert_eq!(
        events[6],
        (Some(LEDGE_BUSY), None, 7),
        "the last refusal names the message and that no retry remains"
    );
}

#[test]
fn busy_rope_swing_page_is_dismissed_and_retried_after_its_hold() {
    let mut x = Crossing::new(crossing_edge(2));
    let mut options = TravelOptions::default();
    assert!(x.poll(&mut options).is_none());
    assert_eq!(x.rec.loc_ops, 1);

    // `shortcuts.rs2:130-132`: a `~mesbox` page, not a chat line.
    plant_mesbox(&mut x.c, ROPE_BUSY);
    assert!(x.next_poll(&mut options).is_none());
    assert_eq!(x.rec.pause_buttons, 1, "the page's continue is pressed");
    x.c.chat_modal_id = -1;
    // `map_clock + 4` is never refreshed: held for at most 5 ticks.
    for held in 1..5 {
        assert!(x.next_poll(&mut options).is_none());
        assert_eq!(x.rec.loc_ops, 1, "still holding at tick {held} of 5");
    }
    assert!(x.next_poll(&mut options).is_none());
    assert_eq!(x.rec.loc_ops, 2, "sent again when the hold lapsed");
    assert!(matches!(
        x.land(&mut options),
        TravelOutcome::Arrived { .. }
    ));
}

#[test]
fn agility_hop_that_never_moves_and_says_nothing_is_sent_again_after_its_crossing_time() {
    let mut x = Crossing::new(crossing_edge(2));
    let mut options = TravelOptions::default();
    assert!(x.poll(&mut options).is_none());
    // The baked crossing time is 2 ticks; a crossing that began would have
    // moved the player by now.
    for tick in 2..=4 {
        assert!(x.next_poll(&mut options).is_none());
        assert_eq!(x.rec.loc_ops, 1, "tick {tick}: inside the crossing time");
    }
    assert!(x.next_poll(&mut options).is_none());
    assert_eq!(x.rec.loc_ops, 2, "the silent drop is sent again at once");
    assert!(matches!(
        x.land(&mut options),
        TravelOutcome::Arrived { .. }
    ));
}

#[test]
fn busy_line_from_before_the_send_is_not_a_refusal() {
    let mut x = Crossing::new(crossing_edge(9));
    let mut options = TravelOptions::default();
    x.c.add_chat(0, LEDGE_BUSY, "");
    x.tick();
    assert!(x.poll(&mut options).is_none());
    assert_eq!(x.rec.loc_ops, 1);
    // Ten ticks outlast the 9-tick hold; the silent-drop resend of a
    // motionless hop needs eleven.
    for _ in 0..10 {
        assert!(x.next_poll(&mut options).is_none());
    }
    assert_eq!(x.rec.loc_ops, 1, "an older line never arms a retry");
}

#[test]
fn busy_line_received_while_walking_to_the_stand_is_not_a_refusal_of_the_send() {
    // The player starts two tiles from the stand: the approach walk is sent
    // first, and the interaction only after the stand is reached.
    let mut x = Crossing::at(crossing_edge(9), (0, 0));
    let mut options = TravelOptions::default();
    assert!(x.poll(&mut options).is_none());
    assert_eq!(x.rec.loc_ops, 0, "the approach walk comes first");
    x.c.add_chat(0, LEDGE_BUSY, "");
    plant_player(&mut x.c, 2, 4);
    assert!(x.next_poll(&mut options).is_none());
    assert_eq!(x.rec.loc_ops, 1, "the interaction goes out at the stand");
    for _ in 0..10 {
        assert!(x.next_poll(&mut options).is_none());
    }
    assert_eq!(
        x.rec.loc_ops, 1,
        "the line arrived before the send and never arms a retry"
    );
}

#[test]
fn transport_budget_counts_game_ticks_not_polls() {
    let ladder = TransportEdge {
        kind: TransportKind::Ladder,
        ..crossing_edge(2)
    };
    for edge in [crossing_edge(9), ladder] {
        let mut x = Crossing::new(edge);
        let mut options = TravelOptions {
            budget_ticks_per_hop: 3,
            ..TravelOptions::default()
        };
        assert!(x.poll(&mut options).is_none(), "send");
        for poll in 0..200 {
            assert!(
                x.poll(&mut options).is_none(),
                "poll {poll} of the same game tick must not lapse the budget"
            );
        }
        let mut ticks = 0;
        let outcome = loop {
            ticks += 1;
            if let Some(outcome) = x.next_poll(&mut options) {
                break outcome;
            }
            assert!(ticks < 10);
        };
        assert!(matches!(outcome, TravelOutcome::Stalled { .. }));
        assert_eq!(
            ticks, 4,
            "lapses on the fourth distinct tick after the send"
        );
    }
}

#[test]
fn tick_count_credits_each_packet_and_nothing_for_a_repeated_read() {
    use crate::traveller::TickCount;
    let mut clock = TickCount::sent_at(10);
    assert_eq!(clock.observe(10), 0, "the send's own tick is not counted");
    assert_eq!(clock.observe(11), 1);
    assert_eq!(clock.observe(11), 1, "a repeated read adds nothing");
    assert_eq!(clock.observe(14), 4, "a rebuild that saw 3 packets adds 3");
    assert_eq!(clock.observe(14), 4);
    assert_eq!(clock.observe(15), 5);
    let mut first = TickCount::default();
    assert_eq!(first.observe(40), 1, "the first read counts one tick");
    assert_eq!(
        first.observe(2),
        2,
        "a tick that moved backwards counts one"
    );
    let mut wrapped = TickCount::sent_at(u32::MAX);
    assert_eq!(wrapped.observe(1), 2, "the snapshot clock wraps");
}

#[test]
fn transport_budget_counts_every_packet_of_a_batched_rebuild() {
    let ladder = TransportEdge {
        kind: TransportKind::Ladder,
        ..crossing_edge(2)
    };
    // A follower slower than the game: each rebuild saw `packets` ticks.
    for packets in [2, 3] {
        for edge in [crossing_edge(9), ladder.clone()] {
            let mut x = Crossing::new(edge);
            let mut options = TravelOptions {
                budget_ticks_per_hop: 3,
                ..TravelOptions::default()
            };
            assert!(x.poll(&mut options).is_none(), "send");
            x.tick_batch(packets);
            assert!(
                x.poll(&mut options).is_none(),
                "{packets} ticks of the 3-tick budget are spent"
            );
            x.tick_batch(packets);
            assert!(
                matches!(x.poll(&mut options), Some(TravelOutcome::Stalled { .. })),
                "{} observed ticks lapse the 3-tick budget",
                2 * packets
            );
        }
    }
}

#[test]
fn loc_approach_budget_counts_every_packet_of_a_batched_rebuild() {
    let mut x = Crossing::at(crossing_edge(9), (0, 0));
    let mut options = TravelOptions {
        budget_ticks_per_hop: 3,
        ..TravelOptions::default()
    };
    assert!(x.poll(&mut options).is_none(), "the approach walk is sent");
    assert_eq!(x.rec.loc_ops, 0);
    x.tick_batch(3);
    assert!(x.poll(&mut options).is_none(), "3 of 3 ticks are spent");
    x.tick_batch(3);
    assert!(
        matches!(x.poll(&mut options), Some(TravelOutcome::Stalled { .. })),
        "6 observed ticks lapse the 3-tick approach budget"
    );
}

#[test]
fn landing_delay_counts_every_packet_of_a_batched_rebuild() {
    let mut x = Crossing::new(crossing_edge(4));
    let mut options = TravelOptions::default();
    assert!(x.poll(&mut options).is_none(), "send");
    plant_player(&mut x.c, 10, 4);
    x.tick_batch(2);
    // The landing poll is the delay's first tick; two more rebuilds of two
    // packets each carry it past the 4 packed ticks.
    assert!(x.poll(&mut options).is_none(), "landing poll: 1 of 4");
    x.tick_batch(2);
    assert!(x.poll(&mut options).is_none(), "3 of 4");
    x.tick_batch(2);
    assert!(
        matches!(x.poll(&mut options), Some(TravelOutcome::Arrived { .. })),
        "5 observed ticks end the 4-tick landing delay"
    );
}

#[test]
fn a_late_accepted_retry_shares_the_hop_budget() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&events);
    let mut options = TravelOptions {
        on_event: Some(Box::new(move |event| {
            if let TravelEvent::TransportBusy { tries, .. } = event {
                sink.borrow_mut().push(tries);
            }
        })),
        ..TravelOptions::default()
    };
    let mut x = Crossing::new(crossing_edge(9));
    let mut seen = 0;
    let mut ticks = 0;
    let outcome = loop {
        if let Some(outcome) = x.poll(&mut options) {
            break outcome;
        }
        if x.rec.loc_ops > seen {
            seen = x.rec.loc_ops;
            if seen < 6 {
                x.c.add_chat(0, LEDGE_BUSY, "");
            } else {
                // The sixth send, 50 ticks in, is accepted: the player
                // starts across and then stops short of the landing.
                plant_player(&mut x.c, 6, 4);
            }
        }
        x.tick();
        ticks += 1;
        assert!(ticks < 400, "the hop must end");
    };
    assert_eq!(
        ticks, 61,
        "refused attempts, holds, the accepted attempt and the landing share the 60-tick budget; a resend does not renew it"
    );
    assert_eq!(
        outcome,
        TravelOutcome::Stalled {
            at: WorldTile {
                x: 3206,
                z: 3204,
                level: 0
            },
            aiming: crossing_edge(9).to,
            why: HopFailure::Expired,
            tries: 6,
            last_refusal: Some(TransportRefusal::Busy(LEDGE_BUSY)),
        }
    );
    assert_eq!(events.borrow().len(), 5, "five refusals were retried");
}

#[test]
fn default_callers_get_the_send_count_and_last_refusal_in_the_terminal_outcome() {
    // No `on_event` subscriber: the outcome alone carries the detail.
    let mut options = TravelOptions::default();
    let mut x = Crossing::new(crossing_edge(9));
    let mut seen = 0;
    let mut ticks = 0;
    let outcome = loop {
        if let Some(outcome) = x.poll(&mut options) {
            break outcome;
        }
        if x.rec.loc_ops > seen {
            seen = x.rec.loc_ops;
            x.c.add_chat(0, LEDGE_BUSY, "");
        }
        x.tick();
        ticks += 1;
        assert!(ticks < 400, "the retries must be finite");
    };
    assert!(
        matches!(
            outcome,
            TravelOutcome::Stalled {
                why: HopFailure::Dropped,
                tries: 7,
                last_refusal: Some(TransportRefusal::Busy(LEDGE_BUSY)),
                ..
            }
        ),
        "{outcome:?}"
    );
    assert!(
        format!("{outcome:?}").contains(LEDGE_BUSY),
        "a caller that prints the outcome sees the refusal: {outcome:?}"
    );
}

#[test]
fn a_crossing_that_never_began_is_reported_as_such_when_its_retries_end() {
    let mut options = TravelOptions {
        budget_ticks_per_hop: 5,
        ..TravelOptions::default()
    };
    let mut x = Crossing::new(crossing_edge(2));
    let mut ticks = 0;
    let outcome = loop {
        if let Some(outcome) = x.poll(&mut options) {
            break outcome;
        }
        x.tick();
        ticks += 1;
        assert!(ticks < 100, "the retries must be finite");
    };
    assert!(
        matches!(
            outcome,
            TravelOutcome::Stalled {
                why: HopFailure::Dropped,
                tries: 2,
                last_refusal: Some(TransportRefusal::NotStarted),
                ..
            }
        ),
        "{outcome:?}"
    );
}

#[test]
fn a_landing_still_settling_at_the_hop_deadline_ends_the_hop() {
    // Five refusals, then the sixth send is accepted and the player lands
    // 58 ticks in. The ledge's 9-tick landing delay would end after the
    // 60-tick hop budget, so the hop stalls at the deadline instead of
    // reporting the crossing it would have finished later.
    let mut options = TravelOptions::default();
    let mut x = Crossing::new(crossing_edge(9));
    let mut seen = 0;
    let mut ticks = 0;
    let outcome = loop {
        if ticks == 58 {
            plant_player(&mut x.c, 10, 4);
        }
        if let Some(outcome) = x.poll(&mut options) {
            break outcome;
        }
        if x.rec.loc_ops > seen {
            seen = x.rec.loc_ops;
            if seen < 6 {
                x.c.add_chat(0, LEDGE_BUSY, "");
            } else {
                plant_player(&mut x.c, 6, 4);
            }
        }
        x.tick();
        ticks += 1;
        assert!(ticks < 100, "the hop must end");
    };
    assert_eq!(x.rec.loc_ops, 6, "five refused sends and the accepted one");
    assert_eq!(
        (ticks, outcome),
        (
            61,
            TravelOutcome::Stalled {
                at: crossing_edge(9).to,
                aiming: crossing_edge(9).to,
                why: HopFailure::Expired,
                tries: 6,
                last_refusal: Some(TransportRefusal::Busy(LEDGE_BUSY)),
            }
        ),
        "the landing delay draws on the shared 60-tick hop budget"
    );
}

#[test]
fn a_hold_that_ends_after_the_hop_deadline_sends_nothing() {
    let mut options = TravelOptions {
        budget_ticks_per_hop: 12,
        ..TravelOptions::default()
    };
    let mut x = Crossing::new(crossing_edge(9));
    assert!(x.poll(&mut options).is_none(), "the first send");
    x.c.add_chat(0, LEDGE_BUSY, "");
    assert!(
        x.next_poll(&mut options).is_none(),
        "the refusal starts the 9-tick hold"
    );
    assert_eq!(x.rec.loc_ops, 1);
    // The follower next polls 20 game ticks later: the hold has ended, and
    // so has the 12-tick hop budget.
    x.tick_batch(20);
    let outcome = x.poll(&mut options);
    assert_eq!(x.rec.loc_ops, 1, "no send after the hop deadline");
    assert_eq!(
        outcome,
        Some(TravelOutcome::Stalled {
            at: WorldTile {
                x: 3202,
                z: 3204,
                level: 0
            },
            aiming: crossing_edge(9).to,
            why: HopFailure::Dropped,
            tries: 1,
            last_refusal: Some(TransportRefusal::Busy(LEDGE_BUSY)),
        })
    );
}

#[test]
fn a_refusal_read_late_counts_the_ticks_of_its_poll_against_the_deadline() {
    let resumes = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&resumes);
    let mut options = TravelOptions {
        budget_ticks_per_hop: 12,
        on_event: Some(Box::new(move |event| {
            if let TravelEvent::TransportBusy { resume_tick, .. } = event {
                sink.borrow_mut().push(resume_tick);
            }
        })),
        ..TravelOptions::default()
    };
    let mut x = Crossing::new(crossing_edge(9));
    assert!(x.poll(&mut options).is_none(), "the first send");
    // The refusal is read on a poll 4 game ticks after the send: 4 ticks
    // and the 9-tick hold do not fit in the 12-tick budget.
    x.c.add_chat(0, LEDGE_BUSY, "");
    x.tick_batch(4);
    let outcome = x.poll(&mut options);
    assert_eq!(
        *resumes.borrow(),
        vec![None],
        "no retry is announced past the deadline"
    );
    assert!(
        matches!(
            outcome,
            Some(TravelOutcome::Stalled {
                why: HopFailure::Dropped,
                tries: 1,
                last_refusal: Some(TransportRefusal::Busy(LEDGE_BUSY)),
                ..
            })
        ),
        "{outcome:?}"
    );
    assert_eq!(x.rec.loc_ops, 1, "no send after the hop deadline");
}
