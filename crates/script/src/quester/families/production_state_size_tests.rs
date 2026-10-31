use super::*;

struct PreviousMakePlan {
    tile: WorldTile,
    loc_id: i32,
    loc_name: Arc<str>,
    op: Arc<str>,
    product: BankItem,
    menu_id: i32,
    qty: QuantityPlan,
    make_x: bool,
}

struct PreviousMakeRun {
    tile: WorldTile,
    loc_id: i32,
    loc_name: Arc<str>,
    op: Arc<str>,
    request: MakeRequest,
    walk: Option<ActionHandle<Walk>>,
    trigger: Option<ActionHandle<reach::Reach>>,
    make: Option<ActionHandle<MakeMachine>>,
}

#[test]
fn production_state_sizes_stay_within_explicit_bounds() {
    let trigger = std::mem::size_of::<MakeTrigger>();
    let plan = std::mem::size_of::<MakePlan>();
    let run = std::mem::size_of::<MakeRun>();
    let previous_plan = std::mem::size_of::<PreviousMakePlan>();
    let previous_run = std::mem::size_of::<PreviousMakeRun>();
    let previous_plan_offsets = [
        std::mem::offset_of!(PreviousMakePlan, tile),
        std::mem::offset_of!(PreviousMakePlan, loc_id),
        std::mem::offset_of!(PreviousMakePlan, loc_name),
        std::mem::offset_of!(PreviousMakePlan, op),
        std::mem::offset_of!(PreviousMakePlan, product),
        std::mem::offset_of!(PreviousMakePlan, menu_id),
        std::mem::offset_of!(PreviousMakePlan, qty),
        std::mem::offset_of!(PreviousMakePlan, make_x),
    ];
    let previous_run_offsets = [
        std::mem::offset_of!(PreviousMakeRun, tile),
        std::mem::offset_of!(PreviousMakeRun, loc_id),
        std::mem::offset_of!(PreviousMakeRun, loc_name),
        std::mem::offset_of!(PreviousMakeRun, op),
        std::mem::offset_of!(PreviousMakeRun, request),
        std::mem::offset_of!(PreviousMakeRun, walk),
        std::mem::offset_of!(PreviousMakeRun, trigger),
        std::mem::offset_of!(PreviousMakeRun, make),
    ];
    let plan_change = plan as isize - previous_plan as isize;
    let run_change = run as isize - previous_run as isize;
    println!(
        "production family state sizes: MakeTrigger={trigger}; MakePlan={plan} ({plan_change:+} from {previous_plan}); MakeRun={run} ({run_change:+} from {previous_run}) bytes"
    );
    println!(
        "previous field offsets: MakePlan={previous_plan_offsets:?}; MakeRun={previous_run_offsets:?}"
    );
    assert!(trigger <= 64, "MakeTrigger grew to {trigger} bytes");
    assert!(plan <= 192, "MakePlan grew to {plan} bytes");
    assert!(
        plan <= previous_plan + 16,
        "MakePlan grew by {} bytes",
        plan.saturating_sub(previous_plan)
    );
    assert!(
        run <= previous_run + 16,
        "MakeRun grew by {} bytes",
        run.saturating_sub(previous_run)
    );
}
