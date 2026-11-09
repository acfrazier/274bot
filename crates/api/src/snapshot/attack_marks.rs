//! Player-side marks that an attack on the local player leaves in the snapshot.
//!
//! Every attack handler in the pinned content sends at least one of these in
//! the cycle that it closes the player's chat, because the engine writes
//! `PLAYER_INFO` (animation, spot animation, hitmarks) and the zone update
//! (projectiles) before it flushes the interface packets
//! (`World.ts:1105-1123`):
//!
//! - melee: the defend animation (`npc_combat.rs2:70`), then `if_close` and the
//!   hit (`npc_combat_melee.rs2:36-47`). A melee NPC on the default handler at
//!   0 hitpoints sends the animation and no hit
//!   (`npc_combat_melee.rs2:40-43`);
//! - ranged: `if_close`, the defend animation and a projectile
//!   (`npc_combat_ranged.rs2:38,48,52`). The hit follows after the flight;
//! - magic: `if_close`, then a spot animation on the player for a hit or a
//!   splash (`npc_combat_magic.rs2:92,109,119,149,159`). A splash queues no hit;
//! - a breath that is launched from range: `if_close`, a projectile and a
//!   delayed hit (`metal_dragon.rs2:36-70`);
//! - poison: `if_close` and a poison hitmark (`poison.rs2:36-40`).
//!
//! Two documented cases leave none of these marks, and there the interruption
//! is seen only when the bounded dialogue wait ends:
//!
//! - an NPC at 0 hitpoints whose own handler sends no defend animation: a
//!   separate melee handler can omit the default handler's animation, and the
//!   zero-hitpoint return then also skips the hit mark
//!   (`shadow_spider.rs2:1-20` with `npc_combat_melee.rs2:40-43`);
//! - a player under persistent animation protection, which discards the
//!   defend animation (`Player.ts:1922-1932`).
//!
//! Outside those two cases, a close without any of them is the content's own:
//! a script that ends, or a `~chatnpc` that follows `if_close`. The marks are
//! a stamp, not a counter: a caller keeps the marks of its start and asks
//! whether any later mark differs ([`AttackMarks::after`]).

use std::num::NonZeroU64;

/// The local player's attack marks at one observation, packed in a non-zero
/// `u64`: the low 32 bits fold the newest hitmark stamp (every kind, poison
/// included), the local animation serial and the local spot animation stamp;
/// the high 32 bits hold the newest start cycle of a projectile that targets
/// the local player (0 when none is live). The packing is also the wire form
/// (`local_attack_marks`), where 0 reads as "no observation".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttackMarks(NonZeroU64);

impl AttackMarks {
    /// Build the marks from their four sources.
    ///
    /// - `hit_stamp`: the largest expiry cycle of the four hitmark slots. The
    ///   client stamps a new hit `loop_cycle + 70`, so a later hit always has a
    ///   larger stamp and an expired slot keeps its old one.
    /// - `animation_serial`: the reception serial of the newest local
    ///   animation instruction. A repeated instruction with the same sequence
    ///   still moves it.
    /// - `spot_stamp`: the packet cycle of the newest local spot animation.
    /// - `projectile_start`: the newest start cycle among live projectiles that
    ///   target the local player, or 0. A live projectile leaves the client's
    ///   list when it lands, so only a start later than the baseline counts.
    pub fn new(
        hit_stamp: i32,
        animation_serial: u32,
        spot_stamp: i32,
        projectile_start: i32,
    ) -> Self {
        let mut fold: u32 = 0x811c_9dc5;
        for word in [hit_stamp as u32, animation_serial, spot_stamp as u32] {
            for byte in word.to_le_bytes() {
                fold = (fold ^ u32::from(byte)).wrapping_mul(0x0100_0193);
            }
        }
        let events = u64::from(fold.max(1));
        let start = u64::from(projectile_start.max(0) as u32) << 32;
        Self(NonZeroU64::new(start | events).expect("the folded events are at least 1"))
    }

    /// Read the packed wire form. `0` is the wire's "no observation".
    pub fn from_wire(raw: u64) -> Option<Self> {
        NonZeroU64::new(raw).map(Self)
    }

    /// The packed wire form, never zero.
    pub fn to_wire(self) -> u64 {
        self.0.get()
    }

    fn events(self) -> u32 {
        self.0.get() as u32
    }

    fn projectile_start(self) -> u32 {
        (self.0.get() >> 32) as u32
    }

    /// Whether a mark landed after `baseline`: a hit, an animation, a spot
    /// animation, or a projectile that started later than any the baseline
    /// saw. Reads of the same state are equal.
    pub fn after(self, baseline: Self) -> bool {
        self.events() != baseline.events() || self.projectile_start() > baseline.projectile_start()
    }
}

#[cfg(test)]
mod tests {
    use super::AttackMarks;

    #[test]
    fn equal_state_has_no_mark_after_the_baseline() {
        let marks = AttackMarks::new(900, 4, 700, 0);
        assert!(!marks.after(marks));
        assert!(!AttackMarks::new(900, 4, 700, 0).after(marks));
    }

    #[test]
    fn each_source_moves_the_marks() {
        let base = AttackMarks::new(900, 4, 700, 5000);
        assert!(AttackMarks::new(1100, 4, 700, 5000).after(base), "a hit");
        assert!(
            AttackMarks::new(900, 5, 700, 5000).after(base),
            "an animation"
        );
        assert!(
            AttackMarks::new(900, 4, 1500, 5000).after(base),
            "a spot animation"
        );
        assert!(
            AttackMarks::new(900, 4, 700, 5400).after(base),
            "a projectile"
        );
    }

    #[test]
    fn an_expired_projectile_is_not_a_mark() {
        let base = AttackMarks::new(900, 4, 700, 5000);
        assert!(!AttackMarks::new(900, 4, 700, 0).after(base));
        assert!(!AttackMarks::new(900, 4, 700, 4000).after(base));
    }

    #[test]
    fn the_wire_form_is_never_zero_and_round_trips() {
        let marks = AttackMarks::new(0, 0, 0, 0);
        assert_ne!(marks.to_wire(), 0);
        assert_eq!(AttackMarks::from_wire(marks.to_wire()), Some(marks));
        assert_eq!(AttackMarks::from_wire(0), None);
    }
}
