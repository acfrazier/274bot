//! Melee-only attack onset detection; the schedule remains owned by Combat.
use super::super::request::ActorRef;
use super::super::tables::{CombatTables, StyleMask};
use api::snapshot::ActorView;

/// Consume a local attack animation onset using the caller-owned id/frame
/// stamps. Same-sequence restarts are detected by the frame moving backwards.
/// The target check excludes an auto-retaliate animation aimed at an intruder.
pub fn onset(
    local: &ActorView,
    engaged: Option<ActorRef>,
    tables: &CombatTables,
    last_id: &mut i32,
    last_frame: &mut i32,
) -> bool {
    let animation = local.animation;
    let frame = local.animation_frame;
    let changed = animation != *last_id || (animation == *last_id && frame < *last_frame);
    *last_id = animation;
    *last_frame = frame;
    if !changed || animation < 0 {
        return false;
    }
    if local
        .target
        .is_some_and(|target| !engaged.is_some_and(|actor| actor.matches(target)))
    {
        return false;
    }
    tables
        .style_seq(animation)
        .is_some_and(|mask| mask.contains(StyleMask::MELEE))
}
