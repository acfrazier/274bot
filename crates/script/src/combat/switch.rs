//! PvP slice V2 (PVP-DESIGN-1026 §4): one loadout per style and one executable
//! style, the pending-style switch state (`SwitchState`), the triggers T1-T4,
//! the pending Ranged need and the legal capabilities.
//!
//! A switch adds no producer and no plan shape. It records the pending style in
//! [`SwitchState::Converging`]; the existing wear repair converges the kit of the
//! pending style over as many admitted plans as the five-event budget allows
//! (`arbiter.rs` `TickPlan::push`). `active` (the executable style, whose
//! `StyleState` exists) changes only when the switch completes or, for a switch
//! that needs no wear, on the trigger tick.
use super::*;
use std::cmp::Reverse;

/// Ticks added to the old weapon's attack interval before a settle ends.
///
/// Content: after the wear the server's cooldown can still run for up to
/// `rate(old)` ticks (`pvp_melee.rs2:9-12` returns without a swing while
/// `%action_delay > map_clock`; the interval is installed at `:17-21` from the
/// right hand worn at that decision), and the first new-weapon decision then
/// installs `rate(new)`. One more tick for the input and one for its
/// observation: a click sent from the frame of tick N is read in cycle N + 1
/// (`engine/src/engine/World.ts:355`, `processClientsIn`, before
/// `processPlayers` at `:378`) and is first visible in that cycle's frame. The
/// content gives no upper bound on input lateness (PRAYER-TICK-1), so the 2 is a
/// design constant that the live receipt tunes (design §4.4, §12).
pub const SETTLE_SLACK: u8 = 2;

/// Why a switch started (PVP-DESIGN-1026 §4.6; V4 adds the special causes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchCause {
    /// T1: the active style's attack became impossible.
    Exhausted,
    /// T2: the opponent protects against the active style (the mask, bits 3..5).
    Protect(u8),
    /// T3: back to the first preferred style.
    Prefer,
    /// T4: melee with the opponent out of reach.
    Reach,
}

/// The switch as a pending state inside the existing receipt model (§4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchState {
    Idle,
    Converging {
        to: Style,
        cause: SwitchCause,
        since: u16,
        /// The opponent's protection bits when the switch started.
        mask: u8,
        /// The attack interval of the style being left, read before any wear.
        rate_old: u8,
    },
    Settling {
        since: u16,
        rate_old: u8,
    },
}

impl SwitchState {
    /// The pending style: the kit the repair row converges toward. `None`
    /// outside `Converging`.
    pub const fn pending(self) -> Option<Style> {
        match self {
            Self::Converging { to, .. } => Some(to),
            _ => None,
        }
    }
}

/// The mandatory ranged capability of a pending kit (Codex F1). `Thrown`
/// carries no ammo id: the weapon is its ammo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangedNeed {
    Thrown { weapon: i32 },
    Quiver { weapon: i32, ammo: i32 },
}

/// Whether `style` may be executed at all under this request's rules (§4.5).
///
/// A ranged weapon cannot be worn when the right-hand slot is locked (the duel
/// rule `NoWeapons` locks slot 3), so Ranged is never legal then. Melee is
/// legal unarmed (`pvp_melee.rs2:19-20`, rate 4) and Mage manually without a
/// staff; both only lose their right-hand row to the lock. A style the duel
/// forbids (`NoMelee`, `NoRanged`, `NoMagic`) is expressed by its absent kit.
pub(super) fn legal_style(request: &CombatRequest, style: Style) -> bool {
    style != Style::Ranged || request.allow.locked_slots & (1 << 3) == 0
}

/// The style the machine opens with (PVP-DESIGN-1026 §4.5).
///
/// A request that declares no kit is a single-style request, as before
/// loadouts existed: it opens with `request.style` when that style is legal,
/// otherwise with the first legal style of `prefer`.
///
/// A request that declares kits opens with the first style of `prefer` that has
/// a kit, is legal and is usable now (`usable`: its weapon, ammunition or
/// runes exist). When no declared style is usable it opens with the first
/// legal declared style of `prefer`, so preparation reports the missing item
/// (`PrepFailed`, `Unprotected(NoRunes)`) exactly as it did for one kit.
/// `request.style` does not outrank `prefer` there: callers that build a
/// request from one style use [`SwitchPolicy::for_style`], which puts it first.
pub(super) fn opening_style(
    request: &CombatRequest,
    usable: impl Fn(Style) -> bool,
) -> Option<Style> {
    let any_kit = Style::ALL
        .iter()
        .any(|style| request.kits.get(*style).is_some());
    let prefer = request.switch.prefer;
    if !any_kit {
        return Some(request.style)
            .filter(|style| legal_style(request, *style))
            .or_else(|| {
                prefer
                    .into_iter()
                    .find(|style| legal_style(request, *style))
            });
    }
    let declared =
        |style: &Style| legal_style(request, *style) && request.kits.get(*style).is_some();
    prefer
        .into_iter()
        .find(|style| declared(style) && usable(*style))
        .or_else(|| prefer.into_iter().find(declared))
}

/// The wear repair's slot order: while a switch converges the right hand
/// first, then the quiver, then the shield, then the rest in slot order
/// (§4.4); otherwise plain slot order.
pub(super) const fn wear_order(converging: bool) -> [u8; 14] {
    if converging {
        [3, 13, 5, 0, 1, 2, 4, 6, 7, 8, 9, 10, 11, 12]
    } else {
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]
    }
}

/// The melee weapon to wield with no kit (§4.2): among the held and worn objs
/// whose `weapon_styles` category is melee, the one with the highest attack
/// tier the bot's base Attack (and Strength, for the gated halberds) allows;
/// ties by the lower attack rate, then the lower obj id. A tier is the level of
/// the obj's `levelrequire` script (`levelrequire/scripts/tier*.rs2`); an obj
/// with no tier script is tier 0 and never excluded
/// (`skill_combat/configs/combat.param:81-83`); a quest-gated tier is usable
/// only with its quest varp, which the machine cannot see, so it never wins.
/// The display name is never read.
pub(super) fn best_melee_weapon(tables: &CombatTables, frame: &Frame<'_>) -> i32 {
    let attack = arbiter::stat(frame, 0).1;
    let strength = arbiter::stat(frame, 2).1;
    let mut best: Option<(u8, Reverse<u8>, Reverse<i32>)> = None;
    for row in frame.equipment.iter().chain(frame.inventory) {
        if row.count <= 0 {
            continue;
        }
        let Some(tier) = tables.melee_weapon_tier(row.def.id) else {
            continue;
        };
        if tier.quest_gated || i32::from(tier.tier) > attack || i32::from(tier.strength) > strength
        {
            continue;
        }
        let rate = tables
            .weapon_style(row.def.id)
            .map_or(4, |style| style.attackrate);
        let key = (tier.tier, Reverse(rate), Reverse(row.def.id));
        if best.is_none_or(|have| key > have) {
            best = Some(key);
        }
    }
    best.map_or(-1, |(_, _, Reverse(id))| id)
}

impl Combat {
    /// The executable style.
    pub fn active_style(&self) -> Style {
        self.active
    }
    /// The switch state (PVP-DESIGN-1026 §4.4).
    pub fn switch_state(&self) -> SwitchState {
        self.switch
    }
    /// Completed style switches so far.
    pub fn switches(&self) -> u8 {
        self.switches
    }
    /// Switches that ended `Failed` so far.
    pub fn switch_failures(&self) -> u8 {
        self.switch_failed
    }
    /// Refusals counted against the wear of `slot` (0 to 3): the budget a
    /// switch's `Failed` is judged by.
    pub fn wear_refusals(&self, slot: u8) -> u8 {
        ((self.prep_failures >> (u32::from(slot) * 3)) & 3) as u8
    }

    /// The `StyleState` of `style` (the one `Box` of a switch lives here).
    /// `ammo_pick` is the ranged ammunition the state starts with, `local_update`
    /// the local player's current animation serial so an old shot never counts.
    pub(super) fn new_style_state(
        request: &CombatRequest,
        tables: &CombatTables,
        style: Style,
        local_update: Option<u32>,
        ammo_pick: i32,
    ) -> Result<StyleState, ActionError> {
        Ok(match style {
            Style::Ranged => StyleState::Ranged(Box::new(RangedState {
                ammo_pick,
                anim_update: local_update,
                pickup_baseline: 0,
                sweep_attempts: 0,
                sweep_done: false,
                readiness_wait: RangedReadinessWait::Inactive,
            })),
            Style::Melee => StyleState::Melee(MeleeState {
                anim_id: -1,
                anim_frame: -1,
                mode_fallback: None,
            }),
            Style::Mage => {
                let mut state = Box::<MagicState>::default();
                if let Some(order) = &request.spells {
                    if order.is_empty() || order.len() > u8::MAX as usize {
                        return Err(unavailable("combat spell order"));
                    }
                    for (slot, spell) in order.iter().enumerate() {
                        let Some(index) = magic::spell_index(tables, &spell.alias) else {
                            return Err(unavailable("combat spell order"));
                        };
                        state.spell_order[slot] = index;
                        state.ordered_mask |= 1u32 << index;
                    }
                    state.spell_order_len = order.len() as u8;
                }
                StyleState::Magic(state)
            }
        })
    }

    /// Whether the request's explicit spell order resolves against the spell
    /// table, checked without building a `MagicState`: a bad order fails at
    /// `begin`, not at a later switch, and `begin` allocates nothing for it.
    pub(super) fn validate_spell_order(
        request: &CombatRequest,
        tables: &CombatTables,
    ) -> Result<(), ActionError> {
        let Some(order) = &request.spells else {
            return Ok(());
        };
        if order.is_empty()
            || order.len() > u8::MAX as usize
            || order
                .iter()
                .any(|spell| magic::spell_index(tables, &spell.alias).is_none())
        {
            return Err(unavailable("combat spell order"));
        }
        Ok(())
    }

    /// The item `style`'s kit wants in `slot`, or the style's pick when it has
    /// no kit. A locked slot has no wanted item. The slot-13 entry of Ranged is
    /// the ammunition of its [`RangedNeed`], never `self.ranged()`.
    pub(super) fn desired_for(&self, style: Style, slot: u8, frame: &Frame<'_>) -> Option<i32> {
        if self.request.allow.locked_slots & (1 << slot) != 0 {
            return None;
        }
        let literal = |kit: &CompiledKit| {
            kit.worn
                .iter()
                .find(|(wanted, _)| *wanted == slot)
                .map(|(_, id)| *id)
        };
        if slot == 5 && self.flags & SHIELD_OVERRIDE != 0 {
            self.tables
                .selected()
                .item_by_alias("antidragonbreathshield")
                .map(|row| row.id)
        } else if style == Style::Ranged && slot == 13 {
            match self.pending_ranged_need(style, frame) {
                Some(RangedNeed::Quiver { ammo, .. }) => Some(ammo),
                Some(RangedNeed::Thrown { .. }) => None,
                None => self.request.kits.get(style).and_then(|kit| literal(kit)),
            }
        } else if let Some(kit) = self.request.kits.get(style) {
            literal(kit)
        } else if slot == 3 && self.rhand_pick[style.index()] >= 0 {
            Some(self.rhand_pick[style.index()])
        } else {
            None
        }
    }

    /// The right-hand weapon of `style`: its desired row, else what is worn.
    pub(super) fn weapon_for(&self, style: Style, frame: &Frame<'_>) -> Option<i32> {
        self.desired_for(style, 3, frame).or_else(|| {
            frame
                .equipment
                .iter()
                .find(|row| row.slot == 3 && row.count > 0)
                .map(|row| row.def.id)
        })
    }

    /// §4.4 (Codex F1): the mandatory ranged capability of the **pending** kit
    /// `kits.get(to)` (`to == Ranged`), from the weapon's `RangedWeaponFact`:
    /// a thrown weapon (`ranged::thrown`) needs its own quantity in the pack or
    /// worn, and nothing in slot 13 (the content reads the ammunition from the
    /// right hand, `pvp_rune_thrownaxe.rs2:77-79`); any other ranged weapon
    /// needs accepted ammunition (`ranged::accepts`): the kit's slot-13 row when
    /// it is held or worn and accepted, else the first worn (slot 13) or held
    /// row that is accepted. Reads the frame's rows only; never the old style's
    /// `RangedState` and never a removed request field. `None` when the need
    /// cannot be met from the held and worn rows.
    pub(super) fn pending_ranged_need(&self, to: Style, frame: &Frame<'_>) -> Option<RangedNeed> {
        if to != Style::Ranged {
            return None;
        }
        let weapon_id = self.weapon_for(to, frame)?;
        let weapon = ranged::weapon(&self.tables, weapon_id)?;
        if ranged::thrown(weapon) {
            let quantity = frame
                .inventory
                .iter()
                .chain(frame.equipment)
                .filter(|row| row.def.id == weapon_id)
                .fold(0i32, |sum, row| sum.saturating_add(row.count));
            return (quantity >= 1 || self.in_flight(weapon_id))
                .then_some(RangedNeed::Thrown { weapon: weapon_id });
        }
        let held_or_worn = |id: i32| {
            self.in_flight(id)
                || frame
                    .equipment
                    .iter()
                    .chain(frame.inventory)
                    .any(|row| row.def.id == id && row.count > 0)
        };
        let literal = self.request.kits.get(to).and_then(|kit| {
            kit.worn
                .iter()
                .find(|(slot, _)| *slot == 13)
                .map(|(_, id)| *id)
        });
        let ammo = literal
            .filter(|id| held_or_worn(*id) && ranged::accepts(&self.tables, weapon, *id))
            .or_else(|| {
                frame
                    .equipment
                    .iter()
                    .filter(|row| row.slot == 13)
                    .chain(frame.inventory)
                    .find(|row| row.count > 0 && ranged::accepts(&self.tables, weapon, row.def.id))
                    .map(|row| row.def.id)
            })
            // Ammunition the machine itself is wearing whose pack and worn
            // updates have not both arrived: the fallback pick of an earlier
            // frame, still awaiting its echo (see `in_flight`).
            .or_else(|| self.in_flight_ammo(weapon))?;
        Some(RangedNeed::Quiver {
            weapon: weapon_id,
            ammo,
        })
    }

    /// The ammunition of a quiver wear row that still awaits its echo and that
    /// `weapon` accepts.
    fn in_flight_ammo(&self, weapon: &api::game_data::RangedWeaponFact) -> Option<i32> {
        self.pending
            .iter()
            .find(|pending| {
                pending.row.kind == RowKind::Wear
                    && pending.row.aux == 13
                    && ranged::accepts(&self.tables, weapon, pending.row.id)
            })
            .map(|pending| pending.row.id)
    }

    /// Whether the explicit spell order, when the request has one, lets the
    /// spell at `index` be cast.
    fn spell_ordered(&self, index: usize) -> bool {
        self.request.spells.as_ref().is_none_or(|order| {
            self.request.fallback_spells
                || order.iter().any(|named| {
                    magic::spell_index(&self.tables, &named.alias)
                        .is_some_and(|found| usize::from(found) == index)
                })
        })
    }

    /// Whether `spell` (the spell table's row `index`) can be cast now with
    /// `rhand` in the right hand: the number of its runes the staff supplies
    /// when it can, `None` when it cannot (level, members, worn requirement,
    /// target, or a rune neither held nor supplied).
    fn spell_castable_with(
        &self,
        rhand: i32,
        index: usize,
        spell: &api::game_data::SpellFact,
        frame: &Frame<'_>,
    ) -> Option<u32> {
        let (magic_level, members) = (arbiter::stat(frame, 6).0, frame.world.members);
        let eligible = self.spell_ordered(index)
            && (!spell.members || members)
            && spell.level <= magic_level
            && spell.component_id >= 0
            && spell.wornrequired.as_ref().is_none_or(|alias| {
                self.tables
                    .selected()
                    .item_by_alias(alias)
                    .is_some_and(|item| item.id == rhand)
            })
            && magic::applicable(spell, frame, &self.tables, self.engaged);
        if !eligible {
            return None;
        }
        let mut supplied = 0u32;
        for rune in &spell.runes {
            if magic::staff_provides(&self.tables, rhand, rune.id, members) {
                supplied += 1;
            } else if arbiter::count(frame, rune.id) < rune.count {
                return None;
            }
        }
        Some(supplied)
    }

    /// Whether some castable spell has its runes held (a staff only when the
    /// kit names one, else the staff worn); honours an explicit spell order.
    fn mage_usable(&self, frame: &Frame<'_>) -> bool {
        let rhand = self.weapon_for(Style::Mage, frame).unwrap_or(-1);
        self.tables
            .selected()
            .spells()
            .iter()
            .enumerate()
            .any(|(index, spell)| {
                self.spell_castable_with(rhand, index, spell, frame)
                    .is_some()
            })
    }

    /// §4.2 Ranged: the held or worn ranged weapon to wield with no kit. Usable
    /// candidates (accepted ammunition held, a thrown weapon being its own; a
    /// Ranged level that allows the weapon, `levelrequire`, the gate the server
    /// applies on wear) outrank the others; each tier ranks by the weapon's
    /// `levelrequire`, then by the best accepted ammunition's, then by the lower
    /// obj id, so the result does not depend on the order of the rows. With no
    /// usable candidate the best of the rest is still picked, so preparation
    /// reports the missing ammunition (`PrepFailed(Ammo)`) as it did before.
    pub(super) fn best_ranged_weapon(&self, frame: &Frame<'_>) -> i32 {
        let level = arbiter::stat(frame, 4).0;
        let mut best: Option<((u8, u8, u8), i32)> = None;
        for row in frame.equipment.iter().chain(frame.inventory) {
            if row.count <= 0 {
                continue;
            }
            let Some(weapon) = ranged::weapon(&self.tables, row.def.id) else {
                continue;
            };
            let allowed = i32::from(weapon.levelrequire) <= level;
            let ammo = if ranged::thrown(weapon) {
                // A thrown weapon is its own ammunition.
                Some(weapon.levelrequire)
            } else {
                frame
                    .equipment
                    .iter()
                    .filter(|held| held.slot == 13)
                    .chain(frame.inventory)
                    .filter(|held| held.count > 0)
                    .filter_map(|held| ranged::ammo_levelrequire(&self.tables, weapon, held.def.id))
                    .max()
            };
            let tier = u8::from(allowed) + u8::from(allowed && ammo.is_some());
            let key = (tier, weapon.levelrequire, ammo.unwrap_or(0));
            if best.is_none_or(|(top, id)| key > top || (key == top && row.def.id < id)) {
                best = Some((key, row.def.id));
            }
        }
        best.map_or(-1, |(_, id)| id)
    }

    /// §4.2 Mage: the held or worn staff to wield with no kit, the one whose
    /// substitutions serve the most castable spells: ranked by how many spells
    /// are castable with it (runes held or supplied), then by how many runes it
    /// supplies across them, then worn over held, then the lower obj id.
    pub(super) fn best_staff(&self, frame: &Frame<'_>) -> i32 {
        let worn = frame
            .equipment
            .iter()
            .filter(|row| row.slot == 3)
            .map(|row| (row, true));
        let held = frame.inventory.iter().map(|row| (row, false));
        let mut best: Option<((u32, u32, bool), i32)> = None;
        for (row, is_worn) in worn.chain(held) {
            if row.count <= 0
                || !self
                    .tables
                    .weapon_style(row.def.id)
                    .is_some_and(|weapon| weapon.tab == Some(CombatTab::Staff))
            {
                continue;
            }
            let (mut castable, mut supplied) = (0u32, 0u32);
            for (index, spell) in self.tables.selected().spells().iter().enumerate() {
                if let Some(runes) = self.spell_castable_with(row.def.id, index, spell, frame) {
                    castable += 1;
                    supplied += runes;
                }
            }
            let key = (castable, supplied, is_worn);
            if best.is_none_or(|(top, id)| key > top || (key == top && row.def.id < id)) {
                best = Some((key, row.def.id));
            }
        }
        best.map_or(-1, |(_, id)| id)
    }

    /// Whether the refusal budget of the style's mandatory ammunition slot is
    /// spent (two refusals, the budget `ranged_prepare` aborts on).
    fn ranged_ammo_refused(&self, frame: &Frame<'_>) -> bool {
        let slot: u32 = match self.pending_ranged_need(Style::Ranged, frame) {
            Some(RangedNeed::Thrown { .. }) => 3,
            _ => 13,
        };
        (self.prep_failures >> (slot * 3)) & 3 >= 2
    }

    /// Whether a wear row for `id` is still awaiting its echo. The server moves
    /// the item out of the pack and into the worn container in the same tick, but
    /// the client applies the two container updates in separate packets, so a
    /// frame can show the item in neither; the row's own window (four ticks,
    /// `settle()`) decides whether it vanished.
    fn in_flight(&self, id: i32) -> bool {
        self.pending_id(RowKind::Wear, id)
    }

    /// §4.3: whether `style` can attack now: its weapon is held or worn and its
    /// resources exist (ranged: its need; magic: a castable spell).
    pub(super) fn style_usable(&self, style: Style, frame: &Frame<'_>) -> bool {
        if self.failed_styles & (1 << style.index()) != 0 {
            return false;
        }
        if let Some(id) = self.desired_for(style, 3, frame) {
            if arbiter::count(frame, id) == 0 && !self.worn(frame, 3, id) && !self.in_flight(id) {
                return false;
            }
        }
        match style {
            Style::Melee => true,
            Style::Ranged => self.pending_ranged_need(style, frame).is_some(),
            Style::Mage => self.mage_usable(frame),
        }
    }

    /// §4.3 T1: the active style's attack is impossible (the rows that abort
    /// today: `Unprotected(NoAmmo)`, `Unprotected(NoRunes)`).
    fn style_exhausted(&self, frame: &Frame<'_>) -> bool {
        !self.style_usable(self.active, frame)
            || (self.active == Style::Ranged && self.ranged_ammo_refused(frame))
    }

    /// Whether the active style's exhaustion is the switch's to answer, not the
    /// engagement's. A converging switch answers it with its target style. A
    /// settle evaluates no trigger (§4.4), so it holds the exhaustion until
    /// `Idle`, where T1 starts the switch to the first usable style; the
    /// engagement aborts for it only when no such style exists.
    pub(super) fn exhaustion_deferred(&self, frame: &Frame<'_>) -> bool {
        match self.switch {
            SwitchState::Converging { .. } => true,
            SwitchState::Settling { .. } => {
                self.request.switch.on_exhaustion
                    && self.request.allow.equipment
                    && self.target_style(frame, |_| true).is_some()
            }
            SwitchState::Idle => false,
        }
    }

    /// §4.4: the wear set for `to` on this frame (slots 0..14 as a bitmask):
    /// the slots where the kit's row differs from what is worn and the wanted
    /// item is held, minus skipped and locked slots.
    pub(super) fn wear_set(&self, to: Style, frame: &Frame<'_>) -> u16 {
        let mut mask = 0u16;
        for slot in 0..14u8 {
            if self.wear_skipped(slot) {
                continue;
            }
            if let Some(id) = self.desired_for(to, slot, frame) {
                if !self.worn(frame, slot, id) && arbiter::count(frame, id) > 0 {
                    mask |= 1 << slot;
                }
            }
        }
        mask
    }

    /// §4.4: the mandatory capability of `to` observed on this frame. Melee:
    /// the kit's right hand worn (none named: unarmed). Ranged: the need worn
    /// (`Thrown` slot 3 with count >= 1; `Quiver` slot 3 and slot 13). Mage: the
    /// staff the kit names worn, and the runes of a castable spell held.
    pub(super) fn capability_observed(&self, to: Style, frame: &Frame<'_>) -> bool {
        let worn_count = |slot: u8, id: i32| {
            frame
                .equipment
                .iter()
                .any(|row| row.slot == i32::from(slot) && row.def.id == id && row.count >= 1)
        };
        let right_hand = self
            .desired_for(to, 3, frame)
            .is_none_or(|id| worn_count(3, id));
        match to {
            Style::Melee => right_hand,
            Style::Ranged => match self.pending_ranged_need(to, frame) {
                Some(RangedNeed::Thrown { weapon }) => worn_count(3, weapon),
                Some(RangedNeed::Quiver { weapon, ammo }) => {
                    worn_count(3, weapon) && worn_count(13, ammo)
                }
                None => false,
            },
            Style::Mage => right_hand && self.mage_usable(frame),
        }
    }

    /// A legal, kitted, usable style other than the active one for which
    /// `accept` holds, first in `prefer` order.
    fn target_style(&self, frame: &Frame<'_>, accept: impl Fn(Style) -> bool) -> Option<Style> {
        self.request.switch.prefer.into_iter().find(|style| {
            *style != self.active
                && accept(*style)
                && legal_style(&self.request, *style)
                && self.request.kits.get(*style).is_some()
                && self.style_usable(*style, frame)
        })
    }

    /// §4.3: the first trigger that fires, with its target. Only in `Fight`,
    /// only from `Idle`: a settle evaluates no trigger (§4.4), T1 included.
    pub(super) fn switch_trigger(&self, frame: &Frame<'_>) -> Option<(Style, SwitchCause)> {
        let policy = self.request.switch;
        // T1: resource exhausted. Any engagement; the first usable style.
        if policy.on_exhaustion && self.style_exhausted(frame) {
            if let Some(to) = self.target_style(frame, |_| true) {
                return Some((to, SwitchCause::Exhausted));
            }
        }
        let opponent = self.engaged_player(frame)?;
        let (now, prev) = (self.opp_protect, self.opp_protect_prev);
        if policy.on_protect {
            // T2: the opponent protects against the active style on this and the
            // previous observed frame, and its mask differs from the mask at the
            // last completed switch. Rotation: the next style after `active` in
            // the cyclic order of `prefer` whose protect bit is off.
            let bit = self.active.protect_bit();
            if now & bit != 0 && prev & bit != 0 && now != self.last_switch_mask {
                let prefer = policy.prefer;
                let at = prefer
                    .iter()
                    .position(|style| *style == self.active)
                    .unwrap_or(0);
                for step in 1..3 {
                    let to = prefer[(at + step) % 3];
                    if to != self.active
                        && now & to.protect_bit() == 0
                        && self.target_style(frame, |style| style == to).is_some()
                    {
                        return Some((to, SwitchCause::Protect(now)));
                    }
                }
            }
            // T3: back to `prefer[0]` only while the opponent shows no protect at
            // all (so it never undoes a T2 rotation), and, for melee, only with
            // the opponent in reach (so it never undoes a T4 switch).
            let first = policy.prefer[0];
            if first != self.active
                && now == 0
                && prev == 0
                && (first != Style::Melee || distance(opponent.actor.tile, frame.here) <= 1)
                && self.target_style(frame, |style| style == first).is_some()
            {
                return Some((first, SwitchCause::Prefer));
            }
        }
        // T4: melee with the opponent more than one tile away for 2 attack
        // intervals and no swing onset of ours. A melee weapon's range is 1
        // (`combat.param:71-74`); `attackrange <= 1` makes the server walk us
        // (`pvp_combat.rs2:11-16`), and a swing is one decision per interval
        // (`pvp_melee.rs2:9-12`), so two intervals without an onset is a fight
        // the server is not swinging.
        if policy.on_reach
            && self.active == Style::Melee
            && u16::from(self.reach_ticks) >= 2 * u16::from(self.rate(frame))
        {
            if let Some(to) = self.target_style(frame, |style| style != Style::Melee) {
                return Some((to, SwitchCause::Reach));
            }
        }
        None
    }

    /// Evaluate the triggers from `Idle` and start the switch they pick. Called
    /// from the `Fight` plan after the arbiter's rows, before the wear repair.
    pub(super) fn switch_start(&mut self, frame: &Frame<'_>, tick: u16) {
        if self.switch != SwitchState::Idle
            || !self.request.allow.equipment
            || self.engaged.is_none()
        {
            return;
        }
        let Some((to, cause)) = self.switch_trigger(frame) else {
            return;
        };
        let rate_old = self.rate(frame);
        if self.wear_set(to, frame) == 0 && self.capability_observed(to, frame) {
            // Nothing to wear and the capability already observed (an already
            // worn weapon, unarmed melee, manual magic): complete on this tick.
            self.complete_switch(to, frame, tick, rate_old);
        } else {
            self.switch = SwitchState::Converging {
                to,
                cause,
                since: tick,
                mask: self.opp_protect,
                rate_old,
            };
            self.reselect_npc_target(frame, tick);
        }
    }

    /// One observed tick of the switch bookkeeping, before the plan: the
    /// opponent's protect bits, the reach counter, the settle clock and the
    /// convergence outcome.
    pub(super) fn switch_observe(&mut self, frame: &Frame<'_>, tick: u16) {
        let swung = self.flags2 & PVP_SWING != 0;
        self.flags2 &= !PVP_SWING;
        let (mask, tile) = self.engaged_player(frame).map_or((0, None), |row| {
            ((row.headicons & 0b11_1000) as u8, Some(row.actor.tile))
        });
        self.opp_protect_prev = self.opp_protect;
        self.opp_protect = mask;
        self.reach_ticks = match tile {
            Some(tile)
                if self.active == Style::Melee && distance(tile, frame.here) > 1 && !swung =>
            {
                self.reach_ticks.saturating_add(1)
            }
            _ => 0,
        };
        match self.switch {
            SwitchState::Idle => {}
            SwitchState::Settling { since, rate_old } => {
                if swung || elapsed(tick, since) >= u16::from(rate_old) + u16::from(SETTLE_SLACK) {
                    self.switch = SwitchState::Idle;
                }
            }
            SwitchState::Converging { .. } => self.settle_switch(frame, tick),
        }
    }

    /// §4.4: evaluated every tick in `Converging`, in the order Failed,
    /// Complete. (An interruption, the exhaustion of `to` before completion, is
    /// a `Failed` through `style_usable`.)
    pub(super) fn settle_switch(&mut self, frame: &Frame<'_>, tick: u16) {
        let SwitchState::Converging { to, rate_old, .. } = self.switch else {
            return;
        };
        if self.switch_failed_now(to, frame) {
            self.fail_switch(to, frame, tick, rate_old);
        } else if self.wear_set(to, frame) == 0 && self.capability_observed(to, frame) {
            self.complete_switch(to, frame, tick, rate_old);
        }
    }

    /// §4.4 `Failed`: the right hand reached its refusal budget (two refusals,
    /// `settle()`'s `FAILED_WEAPON`), or the style is no longer usable (its
    /// weapon is neither worn nor held, its ammunition or runes are gone), or the
    /// mandatory ammunition slot reached two refusals, or another wear of the
    /// kit reached three (`settle()`'s `FAILED_ATTACK`).
    fn switch_failed_now(&self, to: Style, frame: &Frame<'_>) -> bool {
        let refused = |slot: u8| (self.prep_failures >> (u32::from(slot) * 3)) & 3;
        if refused(3) >= 2 || !self.style_usable(to, frame) {
            return true;
        }
        if to == Style::Ranged
            && matches!(
                self.pending_ranged_need(to, frame),
                Some(RangedNeed::Quiver { .. })
            )
            && refused(13) >= 2
        {
            return true;
        }
        let set = self.wear_set(to, frame);
        (0..14u8).any(|slot| set & (1 << slot) != 0 && refused(slot) >= 3)
    }

    /// A refusal budget spent: `active` never moved, the old kit is restored by
    /// the ordinary repair, and the style is not tried again this fight.
    fn fail_switch(&mut self, to: Style, frame: &Frame<'_>, tick: u16, rate_old: u8) {
        self.switch_failed = self.switch_failed.saturating_add(1);
        self.failed_styles |= 1 << to.index();
        // Forget the refusals counted against the pending kit's slots.
        for slot in 0..14u8 {
            if self.desired_for(to, slot, frame).is_some() {
                self.prep_failures &= !(3 << (u32::from(slot) * 3));
            }
        }
        self.switch = SwitchState::Settling {
            since: tick,
            rate_old,
        };
    }

    /// Completion: `active` moves, the new `StyleState` is built (the one `Box`
    /// of a switch), the switch is counted and the settle starts.
    fn complete_switch(&mut self, to: Style, frame: &Frame<'_>, tick: u16, rate_old: u8) {
        let ammo_pick = match self.pending_ranged_need(to, frame) {
            Some(RangedNeed::Thrown { weapon }) => weapon,
            Some(RangedNeed::Quiver { ammo, .. }) => ammo,
            None => -1,
        };
        let local_update = frame.local.animation_update.map(|update| update.serial);
        let Ok(state) =
            Self::new_style_state(&self.request, &self.tables, to, local_update, ammo_pick)
        else {
            self.fail_switch(to, frame, tick, rate_old);
            return;
        };
        self.style_state = state;
        self.active = to;
        self.switches = self.switches.saturating_add(1);
        self.last_switch_mask = self.opp_protect;
        self.failed_styles &= !(1 << to.index());
        self.switch = SwitchState::Settling {
            since: tick,
            rate_old,
        };
        // Target selection follows the style now being executed. A rejected
        // NPC enters the ordinary no-target path instead of retaining a chase.
        let previous = self.engaged;
        self.reselect_npc_target(frame, tick);
        // A different selected actor already resolves worth in engage().
        if self.engaged == previous || self.engaged.is_none() {
            self.resolve_worth(frame);
        }
    }

    /// Plan one tick and undo a `Converging` that carried no wear: a switch is
    /// entered on the tick the plan with its first wear is built, not before
    /// (§4.4), so a tick that cannot wear (a drink, a full plan) starts nothing.
    /// Only `Idle` starts a switch, so a `Converging` entered this tick came
    /// from `Idle`.
    pub(super) fn plan_tick(
        &mut self,
        frame: &Frame<'_>,
        tick: u16,
        cx: &mut ActionContext<'_>,
    ) -> Result<TickPlan, ActionError> {
        let plan = self.plan(frame, tick, cx)?;
        if let SwitchState::Converging { since, .. } = self.switch {
            if since == tick && !plan.contains(RowKind::Wear) {
                self.switch = SwitchState::Idle;
                self.reselect_npc_target(frame, tick);
            }
        }
        Ok(plan)
    }
}
