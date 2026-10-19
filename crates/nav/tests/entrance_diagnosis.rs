use std::num::{NonZeroU16, NonZeroU32};
use std::sync::Arc;
use std::time::Duration;

use api::identity::Deadline;
use api::quest_progress::{EvidenceProvider, EvidenceStamp};
use api::selected::{FactKey, QuestGate, RunKey, Truth};
use api::snapshot::WorldTile;
use api::work::{
    InspectionPhase, SharedCancellation, SliceBudget, StepContext, SystemClock,
};
use nav::contract::NavLease;
use nav::grid::StepGrid;
use nav::producer;
use nav::quest_gates::{QuestEvidence, QuestFamilyId, QuestGates};
use nav::router::{
    AvoidRect, CursorStep, FindOptions, SearchCompletion, SearchCursor, SearchOutcome,
    SearchRequest, SearchRequestShape,
};
use nav::world::NavWorld;
use nav::world_state::{EntranceRequirement, WorldState};

const FROM: WorldTile = WorldTile {
    x: 1,
    z: 0,
    level: 0,
};
const TO: WorldTile = WorldTile {
    x: 3,
    z: 0,
    level: 0,
};
const FORBIDDEN_HELD: i32 = 8001;
const FORBIDDEN_WORN: i32 = 8002;
const POSITIVE_ITEM: i32 = 8003;
const MAX_SETTLES: usize = 128;
const MAX_SLICES: usize = 4096;

fn fixture(configure: impl FnOnce(&mut NavWorld)) -> NavLease {
    let mut world = NavWorld::from_grid(&StepGrid::fixture_door_corridor());
    world.graph.edges[0].forbidden_held = vec![FORBIDDEN_HELD];
    configure(&mut world);
    let world = Arc::new(world);
    producer::lease(&world).expect("lease the entrance fixture")
}

fn observed_state() -> WorldState {
    WorldState {
        inventory_ready: true,
        equipment_ready: true,
        ..WorldState::empty()
    }
}

fn entrance_request() -> SearchRequest {
    SearchRequest::EntranceRequirements {
        from: FROM,
        targets: Box::new([TO]),
        options: FindOptions::default(),
        avoid: Box::<[AvoidRect]>::default(),
        max_settles: MAX_SETTLES,
    }
}

fn drive_cursor(mut cursor: SearchCursor) -> SearchCompletion {
    let clock = SystemClock;
    let cancellation = SharedCancellation::new();
    let units_per_slice = NonZeroU32::new(64).expect("nonzero slice size");

    for _ in 0..MAX_SLICES {
        let budget = SliceBudget::new(
            units_per_slice,
            Deadline::after(clock.now(), Duration::from_secs(5)),
        );
        let mut context = StepContext::<InspectionPhase>::new(&clock, budget, &cancellation);
        match cursor.advance(&mut context) {
            CursorStep::Complete(completion) => return completion,
            CursorStep::Yielded => {}
            CursorStep::Cancelled(reason) => panic!("unexpected cancellation: {reason:?}"),
            CursorStep::Failed(error) => panic!("cooperative search failed: {error:?}"),
        }
    }
    panic!("entrance diagnosis exceeded its finite {MAX_SLICES}-slice limit")
}

fn run_entrance(lease: NavLease, state: WorldState) -> Option<EntranceRequirement> {
    let cursor = SearchCursor::new(entrance_request(), lease, Arc::new(state))
        .expect("construct entrance diagnosis cursor");
    let (outcome, _) = drive_cursor(cursor).into_parts();
    let SearchOutcome::EntranceRequirements(requirement) = outcome else {
        panic!("entrance diagnosis returned the wrong result shape")
    };
    requirement
}

#[test]
fn diagnosis_reports_exact_held_worn_and_unobserved_entrances() {
    let held_lease = fixture(|_| {});
    let mut held_state = observed_state();
    held_state.inv.insert(FORBIDDEN_HELD, 1);
    assert_eq!(
        run_entrance(held_lease, held_state),
        Some(EntranceRequirement::ForbiddenHeld {
            id: FORBIDDEN_HELD,
            at: FROM,
        })
    );

    let worn_lease = fixture(|world| {
        world.graph.edges[0].forbidden_worn = vec![FORBIDDEN_WORN];
    });
    let mut worn_state = observed_state();
    worn_state.worn.insert(FORBIDDEN_WORN);
    assert_eq!(
        run_entrance(worn_lease, worn_state),
        Some(EntranceRequirement::ForbiddenWorn {
            id: FORBIDDEN_WORN,
            at: FROM,
        })
    );

    let inventory_unobserved_lease = fixture(|_| {});
    let inventory_unobserved = WorldState {
        equipment_ready: true,
        ..WorldState::empty()
    };
    assert_eq!(
        run_entrance(inventory_unobserved_lease, inventory_unobserved),
        Some(EntranceRequirement::InventoryNotObserved { at: FROM })
    );

    let equipment_unobserved_lease = fixture(|world| {
        world.graph.edges[0].forbidden_worn = vec![FORBIDDEN_WORN];
    });
    let equipment_unobserved = WorldState {
        inventory_ready: true,
        ..WorldState::empty()
    };
    assert_eq!(
        run_entrance(equipment_unobserved_lease, equipment_unobserved),
        Some(EntranceRequirement::EquipmentNotObserved { at: FROM })
    );
}

#[test]
fn prior_search_modes_remain_strict_about_exclusions() {
    let lease = fixture(|_| {});
    let mut state = observed_state();
    state.inv.insert(FORBIDDEN_HELD, 1);
    let cursor = SearchCursor::new(
        SearchRequest::First {
            from: FROM,
            targets: Box::new([TO]),
            fallback: Box::default(),
            options: FindOptions::default(),
            avoid: Box::default(),
            max_settles: MAX_SETTLES,
        },
        lease,
        Arc::new(state),
    )
    .expect("construct strict first-goal cursor");
    let (outcome, _) = drive_cursor(cursor).into_parts();
    let SearchOutcome::First { search } = outcome else {
        panic!("strict first-goal search returned the wrong result shape")
    };
    assert!(search.route().is_err(), "the old mode must refuse the exclusion");
}

#[derive(Clone, Copy)]
enum PositiveGate {
    Item,
    Consumed,
    WornAny,
    WornAll,
    Skill,
    Members,
    CompletedQuest,
    Varp,
    UnknownQuest,
    FalseQuest,
}

fn family() -> QuestFamilyId {
    QuestFamilyId::new(
        [0x5a; 32],
        NonZeroU16::new(1).expect("nonzero quest extractor schema"),
    )
}

fn install_positive_gate(world: &mut NavWorld, gate: PositiveGate) {
    if matches!(gate, PositiveGate::UnknownQuest | PositiveGate::FalseQuest) {
        let family = family();
        world.graph.quest_family = Some(family);
        world.graph.edges[0].quest_gates = Some(
            QuestGates::new(
                family,
                [QuestGate::Complete(FactKey::new("quest:entrance-test"))],
            )
            .expect("valid stage gate"),
        );
        return;
    }

    let edge = &mut world.graph.edges[0];
    match gate {
        PositiveGate::Item => edge.item_req = vec![(POSITIVE_ITEM, 1)],
        PositiveGate::Consumed => edge.consumed_req = vec![(POSITIVE_ITEM, 1)],
        PositiveGate::WornAny => edge.worn_req = vec![POSITIVE_ITEM],
        PositiveGate::WornAll => edge.worn_all_req = vec![POSITIVE_ITEM],
        PositiveGate::Skill => edge.skill_req = vec![(1, 99)],
        PositiveGate::Members => edge.members_req = true,
        PositiveGate::CompletedQuest => edge.quest_req = vec![String::from("Test Quest")],
        PositiveGate::Varp => edge.varp_req = vec![(99, 1)],
        PositiveGate::UnknownQuest | PositiveGate::FalseQuest => {
            unreachable!("quest gates were installed before borrowing the edge")
        }
    }
}

struct FixedEvidence(Truth);

impl EvidenceProvider for FixedEvidence {
    fn test_gate(&self, _: &QuestGate, _: EvidenceStamp) -> Truth {
        self.0
    }
}

fn evidence(value: Truth) -> QuestEvidence {
    QuestEvidence::new(
        Arc::new(FixedEvidence(value)),
        family(),
        EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 0,
            sequence: 0,
        },
    )
}

#[test]
fn diagnosis_does_not_relax_positive_or_unknown_gates() {
    for (name, gate, quest_result) in [
        ("item", PositiveGate::Item, None),
        ("consumed item", PositiveGate::Consumed, None),
        ("worn alternative", PositiveGate::WornAny, None),
        ("conjunctive worn item", PositiveGate::WornAll, None),
        ("skill", PositiveGate::Skill, None),
        ("world membership", PositiveGate::Members, None),
        ("completed quest", PositiveGate::CompletedQuest, None),
        ("varp", PositiveGate::Varp, None),
        ("unknown stage gate", PositiveGate::UnknownQuest, None),
        ("false stage gate", PositiveGate::FalseQuest, Some(Truth::False)),
    ] {
        let lease = fixture(|world| install_positive_gate(world, gate));
        let mut state = observed_state();
        if let Some(result) = quest_result {
            state = state.with_quest_evidence(evidence(result));
        }
        assert_eq!(
            run_entrance(lease, state),
            None,
            "entrance relaxation must preserve the {name} gate"
        );
    }
}

#[test]
fn proven_quest_gate_stays_open_and_reports_the_route_exclusion() {
    let lease = fixture(|world| install_positive_gate(world, PositiveGate::UnknownQuest));
    let mut state = observed_state();
    state.inv.insert(FORBIDDEN_HELD, 1);
    state = state.with_quest_evidence(evidence(Truth::True));
    assert_eq!(
        run_entrance(lease, state),
        Some(EntranceRequirement::ForbiddenHeld {
            id: FORBIDDEN_HELD,
            at: FROM,
        })
    );
}

#[test]
fn cooperative_admission_and_result_reservations_cover_the_new_request() {
    let lease = fixture(|_| {});
    let mut state = observed_state();
    state.inv.insert(FORBIDDEN_HELD, 1);
    let state = Arc::new(state);
    let request = entrance_request();
    let shape = request.preflight_shape().expect("consistent request shape");
    assert!(matches!(
        shape,
        SearchRequestShape::EntranceRequirements {
            target_capacity: 1,
            avoid_capacity: 0,
            max_settles: MAX_SETTLES,
            ..
        }
    ));

    let preflight = SearchCursor::preflight(shape, &lease, &state)
        .expect("admit the bounded entrance diagnosis");
    assert_eq!(
        preflight.input_bytes(),
        std::mem::size_of::<WorldTile>(),
        "only the boxed target consumes input bytes"
    );
    assert_eq!(
        preflight.reservation().result_bytes,
        std::mem::size_of::<SearchOutcome>() + std::mem::size_of::<SearchCompletion>(),
        "the typed refusal retains no compact route or heap payload"
    );
    assert!(preflight.reservation().peak_bytes > preflight.reservation().result_bytes);

    let cursor = SearchCursor::new_preflighted(request, lease, state, preflight)
        .expect("materialized request fits its admission reservation");
    assert_eq!(cursor.input_bytes(), std::mem::size_of::<WorldTile>());
    assert!(cursor.future_layout_bytes() > 0);
    let (outcome, _) = drive_cursor(cursor).into_parts();
    assert_eq!(
        match outcome {
            SearchOutcome::EntranceRequirements(requirement) => requirement,
            _ => panic!("entrance diagnosis returned the wrong result shape"),
        },
        Some(EntranceRequirement::ForbiddenHeld {
            id: FORBIDDEN_HELD,
            at: FROM,
        })
    );

    let over_budget = SearchRequestShape::EntranceRequirements {
        target_capacity: 1,
        options: FindOptions::default(),
        avoid_capacity: 0,
        max_settles: nav::router::NODE_BUDGET + 1,
    };
    assert!(
        SearchCursor::preflight(over_budget, &lease, &Arc::new(observed_state())).is_none(),
        "the new shape retains the existing cooperative settle cap"
    );
}
