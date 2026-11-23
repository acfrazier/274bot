//! The native walk's tick deadline is derived from the Traveller's maximum
//! (`script::native::walk::WALK_DEADLINE_TICKS`): it must not lapse while the
//! host's own follow may still legitimately run, so the host's terminal
//! receipt always comes first. This test recomputes the maximum from the
//! Traveller's public defaults, so a change to them fails here.

use nav_executor::TravelOptions;
use script::native::walk::WALK_DEADLINE_TICKS;

/// A hop aims inside the 104-tile loaded scene (`pick_aim_in_scene`), so it
/// plans at most 104 route steps.
const HOP_MAX_STEPS: u32 = 104;

#[test]
fn the_walk_deadline_covers_the_traveller_maximum() {
    let options = TravelOptions::default();
    assert_eq!(options.max_hops, 60);
    assert_eq!(options.budget_ticks_per_hop, 60);
    // A hop's budget is max(minimum, planned steps); while the player keeps
    // moving it gets a quarter more (`traveller/walk.rs:231-235`).
    let budget = options.budget_ticks_per_hop.max(HOP_MAX_STEPS);
    let moving = budget + budget.div_ceil(4);
    // Transport legs wait at most one hop budget each and are not counted by
    // `max_hops`; walk and transport legs alternate, so at most one more.
    let maximum = options.max_hops * moving + (options.max_hops + 1) * options.budget_ticks_per_hop;
    assert_eq!(WALK_DEADLINE_TICKS, maximum);
}
