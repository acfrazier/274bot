use super::*;

#[allow(dead_code)]
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

#[allow(dead_code)]
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
    let plan_change = plan as isize - previous_plan as isize;
    let run_change = run as isize - previous_run as isize;
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
    println!(
        "production family state sizes: MakeTrigger={trigger}; MakePlan={plan} ({plan_change:+} from {previous_plan}); MakeRun={run} ({run_change:+} from {previous_run}) bytes"
    );
}
