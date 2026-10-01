//! Server input, next-food, interaction and swing clocks are independent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OpKind { Eat, Drink, Prayer, Wear, Retaliate, Attack, Cast, Arm }
impl OpKind { pub const fn index(self) -> usize { self as usize } }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Interaction { Installed, Cleared, #[default] Unknown }
#[derive(Debug, Clone, Copy, Default)]
pub struct Cycle { pub deadline: u16, pub known: bool }
/// All short windows are less than half the u16 range. Unlike comparing
/// raw tick numbers, this remains correct across the low-16-bit wrap.
pub fn reached(tick: u16, deadline: u16) -> bool { tick.wrapping_sub(deadline) < 0x8000 }
pub fn elapsed(tick: u16, since: u16) -> u16 { tick.wrapping_sub(since) }
#[derive(Debug, Clone, Copy, Default)]
pub struct Schedule {
    earliest: [u16; 8],
    pending_since: [u16; 8],
    pub pending_base: [i16; 8],
    pub unsettled: [u8; 8],
    ready_mask: u8,
    pending_mask: u8,
    pub cycle: Cycle,
    pub input_lock: u16,
    lock_valid: bool,
    pub interaction: Interaction,
    pub restore_due: u16,
    pub restore_owed: bool,
    pub last_clear: u16,
    pub clear_valid: bool,
    pub last_attack: u16,
    pub attack_valid: bool,
    pub last_swing: u16,
    pub swing_valid: bool,
}
impl Schedule {
    pub fn ready(&self, kind: OpKind, tick: u16) -> bool {
        let bit = 1 << kind.index();
        self.ready_mask & bit == 0 || reached(tick, self.earliest[kind.index()])
    }
    pub fn pending(&self, kind: OpKind) -> bool { self.pending_mask & (1 << kind.index()) != 0 }
    pub fn pending_age(&self, kind: OpKind, tick: u16) -> u16 { elapsed(tick, self.pending_since[kind.index()]) }
    pub fn settle(&mut self, kind: OpKind) {
        self.pending_mask &= !(1 << kind.index());
        self.unsettled[kind.index()] = 0;
    }
    pub fn timeout(&mut self, kind: OpKind) {
        self.pending_mask &= !(1 << kind.index());
        self.unsettled[kind.index()] = self.unsettled[kind.index()].saturating_add(1);
    }
    pub fn locked(&self, tick: u16) -> bool { self.lock_valid && !reached(tick, self.input_lock) }
    pub fn unlock_observed(&mut self, tick: u16) {
        if self.lock_valid && reached(tick, self.input_lock) { self.lock_valid = false; }
    }
    /// Apply only after successful host admission, never after a refused emit.
    pub fn admitted(&mut self, kind: OpKind, tick: u16, baseline: i16, rate: u8, fight: bool, elemental_shield: bool) {
        let index = kind.index();
        let delay = match kind {
            OpKind::Eat | OpKind::Drink | OpKind::Prayer => 3,
            OpKind::Wear | OpKind::Retaliate => 2,
            OpKind::Attack => u16::from(rate.max(1)) + 1,
            OpKind::Cast => 5,
            OpKind::Arm => 1,
        };
        self.earliest[index] = tick.wrapping_add(delay);
        self.ready_mask |= 1 << index;
        self.pending_since[index] = tick;
        self.pending_base[index] = baseline;
        self.pending_mask |= 1 << index;
        match kind {
            OpKind::Eat => {
                if self.cycle.known { self.cycle.deadline = self.cycle.deadline.wrapping_add(3); }
                self.clear(tick, 1, fight);
            }
            OpKind::Drink => {
                self.input_lock = tick.wrapping_add(3);
                self.lock_valid = true;
                self.clear(tick, 3, fight);
            }
            OpKind::Wear => {
                if elemental_shield { self.input_lock = tick.wrapping_add(2); self.lock_valid = true; }
                self.clear(tick, if elemental_shield { 2 } else { 1 }, fight);
            }
            OpKind::Prayer | OpKind::Retaliate | OpKind::Arm => self.clear(tick, 1, fight),
            OpKind::Attack => {
                self.last_attack = tick;
                self.attack_valid = true;
                self.restore_owed = false;
                self.interaction = Interaction::Unknown;
            }
            OpKind::Cast => {}
        }
    }
    pub fn clear(&mut self, tick: u16, wait: u16, fight: bool) {
        self.interaction = Interaction::Cleared;
        self.last_clear = tick;
        self.clear_valid = true;
        self.restore_owed = fight;
        self.restore_due = tick.wrapping_add(wait);
    }
    pub fn observe_swing(&mut self, tick: u16, rate: u8) {
        self.cycle = Cycle { deadline: tick.wrapping_add(u16::from(rate)), known: true };
        self.last_swing = tick;
        self.swing_valid = true;
        if !self.clear_valid || (reached(tick, self.last_clear) && tick != self.last_clear) {
            self.interaction = Interaction::Installed;
            self.settle(OpKind::Attack);
        }
    }
    pub fn restore_ready(&self, tick: u16) -> bool {
        self.restore_owed && self.interaction == Interaction::Cleared && reached(tick, self.restore_due)
    }
    pub fn stale_ready(&self, tick: u16, rate: u8) -> bool {
        if !self.ready(OpKind::Attack, tick) || (self.cycle.known && !reached(tick, self.cycle.deadline)) { return false; }
        if self.interaction == Interaction::Cleared { return false; }
        let since = if self.swing_valid && (!self.attack_valid || reached(self.last_swing, self.last_attack)) {
            self.last_swing
        } else { self.last_attack };
        !self.attack_valid || elapsed(tick, since) >= u16::from(rate) + 1
    }
    pub fn terminal(&mut self) { self.restore_owed = false; }
}
