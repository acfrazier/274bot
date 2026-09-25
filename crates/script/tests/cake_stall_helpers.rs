//! Exercise the public native helper surface in both default and no-default builds.
use api::cake_stall::{StallLoc, BAKER_STALL};
use script::cake_stall::{counts_as_stall_food, needs_cake_restock, select_baker_stall};

fn loc<'a>(
    id: i32,
    name: Option<&'a str>,
    x: i32,
    z: i32,
    distance: i32,
    actions: &'a [&'a str],
) -> StallLoc<'a> {
    StallLoc {
        id,
        name,
        x,
        z,
        level: 0,
        distance,
        actions,
    }
}

#[test]
fn closer_door_does_not_beat_farther_qualifying_stall() {
    let steal = ["Steal from"];
    let open = ["Open"];
    let locs = [
        loc(1530, Some("Door"), 2668, 3312, 0, &open),
        loc(2561, Some("Baker's stall"), 2667, 3310, 2, &steal),
    ];
    let hit = select_baker_stall(Some(&BAKER_STALL), &locs).expect("stall");
    assert_eq!(hit.id, 2561);
    assert_eq!((hit.x, hit.z), (2667, 3310));
}

#[test]
fn other_same_id_stall_is_not_the_selected_target() {
    let steal = ["Steal from"];
    let locs = [
        loc(2561, Some("Baker's stall"), 2655, 3311, 1, &steal),
        loc(2561, Some("Baker's stall"), 2667, 3310, 11, &steal),
    ];
    let hit = select_baker_stall(Some(&BAKER_STALL), &locs).expect("market stall");
    assert_eq!((hit.x, hit.z, hit.distance), (2667, 3310, 11));
}

#[test]
fn depleted_or_wrong_op_or_missing_facts_select_nothing() {
    let examine = ["Examine"];
    let steal = ["Steal from"];
    let depleted = [loc(2561, Some("Baker's stall"), 2667, 3310, 1, &examine)];
    assert!(select_baker_stall(Some(&BAKER_STALL), &depleted).is_none());
    let wrong_name = [loc(2561, Some("Bakery stall"), 2667, 3310, 1, &steal)];
    assert!(select_baker_stall(Some(&BAKER_STALL), &wrong_name).is_none());
    let wrong_id = [loc(99, Some("Baker's stall"), 2667, 3310, 1, &steal)];
    assert!(select_baker_stall(Some(&BAKER_STALL), &wrong_id).is_none());
    assert!(select_baker_stall(None, &depleted).is_none());
    assert!(select_baker_stall(Some(&BAKER_STALL), &[]).is_none());
}

#[test]
fn restock_honors_capacity_and_requested_count() {
    assert!(needs_cake_restock(0, Some(1), false));
    assert!(!needs_cake_restock(0, Some(1), true));
    assert!(!needs_cake_restock(0, Some(0), false));
    assert!(!needs_cake_restock(4, Some(4), false));
    assert!(needs_cake_restock(0, None, false));
}

#[test]
fn stall_food_matches_frozen_substring_patterns() {
    // Exact posted names and consumed partials.
    assert!(counts_as_stall_food("Cake"));
    assert!(counts_as_stall_food("2/3 cake"));
    assert!(counts_as_stall_food("Slice of cake"));
    assert!(counts_as_stall_food("Bread"));
    assert!(counts_as_stall_food("chocolate slice"));
    // Frozen also accepts any name that merely contains a pattern.
    assert!(counts_as_stall_food("Chocolate cake"));
    assert!(counts_as_stall_food("Bread roll"));
    // Unrelated inventory stays out.
    assert!(!counts_as_stall_food("Lobster"));
    assert!(!counts_as_stall_food("Chocolate bar"));
    assert!(!counts_as_stall_food("Rune scimitar"));
    assert!(!counts_as_stall_food(""));
    assert!(!counts_as_stall_food("   "));
}
