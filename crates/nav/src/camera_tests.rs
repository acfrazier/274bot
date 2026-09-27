use super::*;

fn t(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

#[test]
fn yaw_north_is_zero() {
    assert_eq!(yaw_toward_delta(0, 1), 0);
}

#[test]
fn yaw_delta_wraps_the_short_way() {
    assert!(yaw_delta(0, 2047).abs() <= 2);
    assert_eq!(yaw_delta(0, 1024), 1024);
}

#[test]
fn ease_yaw_moves_toward_the_target() {
    let (yaw, v) = ease_yaw(0, 200, 0.0);
    assert!(yaw > 0 && yaw < 200, "yaw={yaw}");
    assert!(v > 0.0);
}

#[test]
fn path_facing_stops_at_a_transport_hop() {
    let tiles = [t(0, 0), t(0, 1), t(0, 2), t(50, 50)];
    let hops = [false, false, false, true];
    let yaw = path_facing_yaw(t(0, 0), &tiles, &hops, 12).unwrap();
    assert_eq!(yaw, yaw_toward_delta(0, 2));
}

#[test]
fn path_facing_is_none_when_the_player_is_off_the_path() {
    let tiles = [t(0, 0), t(0, 1), t(0, 2)];
    let hops = [false, false, false];
    assert_eq!(path_facing_yaw(t(10, 10), &tiles, &hops, 12), None);
}

#[test]
fn hold_desired_ignores_small_heading_chatter() {
    assert_eq!(hold_desired(None, 100), 100);
    assert_eq!(hold_desired(Some(100), 110), 100);
    assert_eq!(hold_desired(Some(100), 140), 140);
}
