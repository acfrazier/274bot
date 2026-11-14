use std::sync::Arc;

use nav_planner::tile::Tile;

use super::map_host::{self, Play, SharedClientTemplate, SlotArm, SlotStatus};

pub fn play(template: &Arc<SharedClientTemplate>, origin: Tile) -> Play {
    let mut play = map_host::run_prepared_template(
        template.validate_for_play().unwrap(),
        false,
        vec![],
        |_| (None, None),
        |_, _, _| {},
    )
    .unwrap();
    play.attach_arm("alice", SlotArm::new(1, false));
    play.focus("alice");
    play.statuses.lock().unwrap().push(SlotStatus {
        username: "alice".into(),
        ingame: true,
        tile_x: origin.x,
        tile_z: origin.z,
        tile_level: origin.level,
        ..Default::default()
    });
    play
}
