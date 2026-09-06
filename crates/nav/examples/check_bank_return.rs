//! Inspect the captured bank-return crossing in a selected baked pack.
use api::snapshot::WorldTile;
use nav::{router, world::NavWorld};
fn main() {
    let path = std::env::var("NAV_PACK").expect("NAV_PACK required");
    let world = NavWorld::load_pack(std::path::Path::new(&path)).expect("valid pack");
    let tile = |x, z| WorldTile { x, z, level: 0 };
    assert!(
        !world
            .graph
            .edges
            .iter()
            .any(|e| e.loc_id == 1530 && e.at == tile(2656, 3292) && e.to == tile(2651, 3292)),
        "obsolete crossing retained"
    );
    let route = router::find(
        &world.collision,
        &world.graph,
        tile(2655, 3286),
        tile(2661, 3306),
    )
    .expect("bank return remains routable");
    println!("{route:?}");
}
