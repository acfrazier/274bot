//! Path-facing orbit yaw (rs2b0t `cameraFollow.ts`). Pure: the host writes
//! `orbit_camera_yaw` each headed observe. No client opcode.

use api::snapshot::WorldTile;

/// Scene-unit / tile delta → orbit camera yaw (0–2047).
pub fn yaw_toward_delta(dx: i32, dz: i32) -> i32 {
    if dx == 0 && dz == 0 {
        return 0;
    }
    ((f64::atan2(dx as f64, dz as f64) * -325.949) as i32) & 0x7ff
}

/// Shortest signed yaw delta in (-1024, 1024].
pub fn yaw_delta(from: i32, to: i32) -> i32 {
    let mut d = (to - from) & 0x7ff;
    if d > 1024 {
        d -= 2048;
    }
    d
}

/// Ease toward `target`: error × gain blended with damped velocity.
pub fn ease_yaw(current: i32, target: i32, velocity: f32) -> (i32, f32) {
    const GAIN: f32 = 0.14;
    const MAX_SPEED: f32 = 18.0;
    const DAMPING: f32 = 0.72;
    const DEADZONE: i32 = 6;

    let err = yaw_delta(current, target);
    if err.abs() <= DEADZONE && velocity.abs() < 1.0 {
        return (current & 0x7ff, 0.0);
    }
    let mut desired = err as f32 * GAIN;
    desired = desired.clamp(-MAX_SPEED, MAX_SPEED);
    let mut v = velocity * DAMPING + desired * (1.0 - DAMPING);
    if v.abs() < 0.15 {
        v = 0.0;
    }
    let next = (current + (v / 2.0).round() as i32) & 0x7ff;
    (next, v)
}

/// Chebyshev jump treated as a transport / dungeon landing (do not average
/// heading across it).
const TRANSPORT_JUMP: i32 = 32;

fn is_transport_boundary(a: WorldTile, b: WorldTile, b_transport: bool) -> bool {
    if a.level != b.level || b_transport {
        return true;
    }
    (a.x - b.x).abs().max((a.z - b.z).abs()) >= TRANSPORT_JUMP
}

/// Do not retarget until the sampled heading moves this far (rs2b0t
/// `TARGET_RETARGET_MIN`). Stops micro-twitch on a straight corridor.
pub const TARGET_RETARGET_MIN: i32 = 28;

/// Keep `held` until `sampled` differs by at least [`TARGET_RETARGET_MIN`].
pub fn hold_desired(held: Option<i32>, sampled: i32) -> i32 {
    let sampled = sampled & 0x7ff;
    match held {
        None => sampled,
        Some(d) if yaw_delta(d, sampled).abs() >= TARGET_RETARGET_MIN => sampled,
        Some(d) => d & 0x7ff,
    }
}

/// Average heading from `from` across path tiles ahead. `from` must sit on
/// `tiles` (else `None` — do not face the route start when the player is
/// off the packed line). Stops at a transport or level hop so a tele
/// landing does not yank yaw.
pub fn path_facing_yaw(
    from: WorldTile,
    tiles: &[WorldTile],
    transport: &[bool],
    look_ahead: usize,
) -> Option<i32> {
    if tiles.is_empty() {
        return None;
    }
    let idx = tiles.iter().position(|t| *t == from)?;
    let start = idx + 1;
    let end = tiles.len().saturating_sub(1).min(idx + look_ahead.max(2));
    if start > end {
        return None;
    }
    let mut dx = 0i32;
    let mut dz = 0i32;
    let mut n = 0i32;
    let mut prev = from;
    for (i, t) in tiles.iter().enumerate().take(end + 1).skip(start) {
        let hop = transport.get(i).copied().unwrap_or(false);
        if is_transport_boundary(prev, *t, hop) {
            break;
        }
        if t.level != from.level {
            break;
        }
        dx += t.x - from.x;
        dz += t.z - from.z;
        n += 1;
        prev = *t;
    }
    if n == 0 || (dx == 0 && dz == 0) {
        return None;
    }
    Some(yaw_toward_delta(dx, dz))
}

#[cfg(test)]
mod tests {
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
}
