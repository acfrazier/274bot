//! Compact actor-aware attack evidence and residual danger.
use super::frame::Frame;
use super::request::{ActorKind, ActorRef};
use super::select;
use super::tables::{CombatTables, StyleWhere};
use api::snapshot::{
    ActorTargetView, ActorView, HitmarkView, HitmarksView, NpcView, PlayerView, ProjectileView,
    WorldTile,
};

pub const THREAT_TTL: u16 = 10;
pub const HITMARK_BLOCK: i32 = 0;
pub const HITMARK_DAMAGE: i32 = 1;
pub const HITMARK_POISON: i32 = 2;
/// Actor-local evidence for an attack on the local player. Health-bar state is
/// intentionally not part of this decision.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LocalAttackEvidence {
    pub targets_local: bool,
    pub attack_animation: bool,
    pub attack_spot: bool,
    pub local_hitmark_recent: bool,
}

impl LocalAttackEvidence {
    pub const fn is_live(self) -> bool {
        targets_local_with_attack_evidence(
            self.targets_local,
            self.attack_animation,
            self.attack_spot,
            self.local_hitmark_recent,
        )
    }
}

/// Shared pure threat predicate for product and offline proof evaluation.
#[inline]
pub const fn targets_local_with_attack_evidence(
    targets_local: bool,
    attack_animation: bool,
    attack_spot: bool,
    local_hitmark_recent: bool,
) -> bool {
    targets_local && (attack_animation || attack_spot || local_hitmark_recent)
}

/// Derive the exact health-bar-independent attack evidence for one NPC.
pub fn npc_local_attack_evidence(
    npc: &NpcView,
    local_player_index: usize,
    hitmarks: Option<&HitmarksView>,
    tables: &CombatTables,
) -> LocalAttackEvidence {
    local_attack_evidence(
        npc.target,
        npc.animation,
        npc.spot_animation,
        npc.spot_animation_stamp,
        local_player_index,
        hitmarks,
        tables,
    )
}

/// Derive health-bar-independent attack evidence for any actor view.
pub fn actor_local_attack_evidence(
    actor: &ActorView,
    local_player_index: usize,
    hitmarks: Option<&HitmarksView>,
    tables: &CombatTables,
) -> LocalAttackEvidence {
    local_attack_evidence(
        actor.target,
        actor.animation,
        actor.spot_animation,
        actor.spot_animation_stamp,
        local_player_index,
        hitmarks,
        tables,
    )
}

fn local_attack_evidence(
    target: Option<ActorTargetView>,
    animation: i32,
    spot_animation: i32,
    spot_animation_stamp: i32,
    local_player_index: usize,
    hitmarks: Option<&HitmarksView>,
    tables: &CombatTables,
) -> LocalAttackEvidence {
    let targets_local = faces_us(target, local_player_index);
    let attack_animation = animation >= 0
        && tables
            .style_seq(animation)
            .is_some_and(|mask| mask.bits() != 0);
    let attack_spot = targets_local
        && hitmarks.is_some_and(|marks| {
            cycle_recent(marks.loop_cycle, spot_animation_stamp)
                && tables
                    .style_spotanim(spot_animation)
                    .is_some_and(|row| row.where_ == StyleWhere::Attacker)
        });
    let local_hitmark_recent = hitmarks.is_some_and(|marks| {
        marks.marks.iter().any(|mark| {
            mark.cycle > 0
                && cycle_recent(marks.loop_cycle, mark.cycle)
                && matches!(mark.kind, HITMARK_BLOCK | HITMARK_DAMAGE)
        })
    });
    LocalAttackEvidence {
        targets_local,
        attack_animation,
        attack_spot,
        local_hitmark_recent,
    }
}

const THREAT_VALID: u16 = 1;
const THREAT_FACT_LIVE: u16 = 1 << 1;
const PRIORITY_SHIFT: u16 = 2;
const PRIORITY_MASK: u16 = 0b111 << PRIORITY_SHIFT;
const HISTORY_SHIFT: u16 = 5;
const HISTORY_MASK: u16 = 0b1111 << HISTORY_SHIFT;
const THREAT_HAS_DUE: u16 = 1 << 9;
const FALLBACK_SHIFT: u16 = 10;
const FALLBACK_MASK: u16 = 0b111 << FALLBACK_SHIFT;
const FAMILY_SHIFT: u16 = 13;
const FAMILY_MASK: u16 = 0b111 << FAMILY_SHIFT;
const GLOBAL_OVERFLOW: u8 = 1;
const GLOBAL_UNKNOWN: u8 = 2;
const PRIORITY_IMPACT: u8 = 1;
const PRIORITY_ANIMATION: u8 = 2;
const PRIORITY_SPOT: u8 = 3;
const PRIORITY_PROJECTILE: u8 = 4;

/// Resolved incoming-projectile family. The due slot is deliberately just
/// evidence state; S3a does not add ranged/magic planner branches.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProjectileFamily {
    #[default]
    Unknown,
    NpcRanged,
    NpcSpell,
    PlayerRanged,
    PlayerThrown,
    PlayerSpell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum StyleObs {
    #[default]
    Unknown = 0,
    Melee = 1,
    Ranged = 2,
    Magic = 3,
    Dragonfire = 4,
}
#[inline]
fn decode_style(bits: u16) -> StyleObs {
    match bits {
        1 => StyleObs::Melee,
        2 => StyleObs::Ranged,
        3 => StyleObs::Magic,
        4 => StyleObs::Dragonfire,
        _ => StyleObs::Unknown,
    }
}

#[inline]
fn decode_family(bits: u16) -> ProjectileFamily {
    match bits {
        1 => ProjectileFamily::NpcRanged,
        2 => ProjectileFamily::NpcSpell,
        3 => ProjectileFamily::PlayerRanged,
        4 => ProjectileFamily::PlayerThrown,
        5 => ProjectileFamily::PlayerSpell,
        _ => ProjectileFamily::Unknown,
    }
}

/// One threat row. Four rows plus the shared onset consumer fit in 176 bytes.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Threat {
    pub actor: ActorRef,
    /// NPC type id, or FNV-1a of the observed player name.
    pub ident: i32,
    last_cycle: i32,
    animation_seen: i32,
    recent: u32,
    pub last_seen: u16,
    pub last_decision: u16,
    fallback_since: u16,
    /// The launcher's main-family hit-queue tick, retained after visual expiry.
    pub due_tick: u16,
    max_hit: i16,
    animation_frame: i16,
    pub rate: u8,
    pub style: StyleObs,
    meta: u16,
}

impl Threat {
    const EMPTY: Self = Self {
        actor: ActorRef {
            kind: ActorKind::Npc,
            index: 0,
        },
        ident: 0,
        last_cycle: -1,
        animation_seen: -1,
        recent: 0,
        last_seen: 0,
        last_decision: 0,
        fallback_since: u16::MAX,
        due_tick: 0,
        max_hit: -1,
        animation_frame: -1,
        rate: 0,
        style: StyleObs::Unknown,
        meta: 0,
    };

    fn new(
        actor: ActorRef,
        ident: i32,
        style: StyleObs,
        rate: u8,
        max_hit: Option<u8>,
        tick: u16,
    ) -> Self {
        let mut row = Self {
            actor,
            ident,
            style,
            rate,
            max_hit: max_hit.map_or(-1, i16::from),
            last_seen: tick,
            last_decision: tick,
            ..Self::EMPTY
        };
        row.meta = THREAT_VALID;
        row.set_fallback_style(StyleObs::Melee);
        row
    }

    #[inline]
    pub fn is_live(&self, tick: u16) -> bool {
        self.valid() && (self.fact_live() || tick_age(tick, self.last_seen) <= THREAT_TTL)
    }

    #[inline]
    pub fn next_decision(&self) -> u16 {
        self.last_decision.wrapping_add(u16::from(self.rate))
    }
    #[inline]
    pub fn max_hit_est(&self) -> Option<u8> {
        u8::try_from(self.max_hit).ok()
    }

    #[inline]
    pub fn fallback_style(&self) -> StyleObs {
        decode_style((self.meta & FALLBACK_MASK) >> FALLBACK_SHIFT)
    }

    pub fn projectile_family(&self) -> ProjectileFamily {
        decode_family((self.meta & FAMILY_MASK) >> FAMILY_SHIFT)
    }

    #[inline]
    pub fn has_event(&self) -> bool {
        self.priority() != 0
    }

    #[inline]
    pub fn recent_count(&self, style: StyleObs) -> u8 {
        let wanted = style as u32;
        let len = self.history_len();
        (0..len)
            .filter(|slot| ((self.recent >> (u32::from(*slot) * 3)) & 0b111) == wanted)
            .count() as u8
    }
    #[inline]
    pub fn recent_position(&self, style: StyleObs) -> Option<u8> {
        let wanted = style as u32;
        (0..self.history_len())
            .find(|slot| ((self.recent >> (u32::from(*slot) * 3)) & 0b111) == wanted)
    }

    #[inline]
    fn has_due(&self) -> bool {
        self.meta & THREAT_HAS_DUE != 0
    }
    #[inline]
    pub fn history_len(&self) -> u8 {
        ((self.meta & HISTORY_MASK) >> HISTORY_SHIFT) as u8
    }

    #[inline]
    fn valid(&self) -> bool {
        self.meta & THREAT_VALID != 0
    }

    #[inline]
    fn fact_live(&self) -> bool {
        self.meta & THREAT_FACT_LIVE != 0
    }

    #[inline]
    fn priority(&self) -> u8 {
        ((self.meta & PRIORITY_MASK) >> PRIORITY_SHIFT) as u8
    }
    fn set_priority(&mut self, value: u8) {
        self.meta = (self.meta & !PRIORITY_MASK) | (u16::from(value.min(7)) << PRIORITY_SHIFT);
    }

    fn set_fallback_style(&mut self, style: StyleObs) {
        self.meta = (self.meta & !FALLBACK_MASK) | ((style as u16) << FALLBACK_SHIFT);
    }

    fn set_projectile_family(&mut self, family: ProjectileFamily) {
        self.meta = (self.meta & !FAMILY_MASK) | ((family as u16) << FAMILY_SHIFT);
    }

    fn set_fact_live(&mut self, live: bool) {
        if live {
            self.meta |= THREAT_FACT_LIVE;
        } else {
            self.meta &= !THREAT_FACT_LIVE;
        }
    }

    fn mark_event(&mut self) {
        self.set_fact_live(false);
    }

    fn add_style_event(&mut self, style: StyleObs) {
        let len = self.history_len();
        self.recent = ((self.recent << 3) | u32::from(style as u8)) & 0x00ff_ffff;
        self.meta = (self.meta & !HISTORY_MASK)
            | (u16::from(len.saturating_add(1).min(8)) << HISTORY_SHIFT);
    }
    fn refine_latest_style(&mut self, style: StyleObs) {
        if self.history_len() == 0 {
            self.add_style_event(style);
        } else {
            self.recent = (self.recent & !0b111) | u32::from(style as u8);
        }
    }
    fn add_due(&mut self, tick: u16, family: ProjectileFamily) {
        self.due_tick = tick;
        self.set_projectile_family(family);
        self.meta |= THREAT_HAS_DUE;
    }
    fn clear_due(&mut self) {
        self.meta &= !THREAT_HAS_DUE;
        self.due_tick = 0;
        self.set_projectile_family(ProjectileFamily::Unknown);
    }

    fn set_max_hit(&mut self, hit: Option<u8>) {
        if let Some(hit) = hit {
            self.max_hit = self.max_hit.max(i16::from(hit));
        }
    }

    fn weight(&self) -> u16 {
        self.max_hit_est().map_or(u16::MAX, u16::from)
    }
}

/// Shared stateful hitmark-onset consumer. It accepts only Block and Damage;
/// poison and unknown future hitmark kinds are consumed but never attributed.
#[derive(Debug, Clone, Copy)]
pub struct HitOnset {
    pub hit_seen: [i32; 4],
}

impl Default for HitOnset {
    fn default() -> Self {
        Self::new()
    }
}

impl HitOnset {
    pub const fn new() -> Self {
        Self { hit_seen: [-1; 4] }
    }

    pub fn observe(&mut self, hitmarks: &[HitmarkView; 4], loop_cycle: i32) -> HitOnsets {
        let mut events = HitOnsets::empty();
        for (slot, hitmark) in hitmarks.iter().enumerate() {
            let cycle = hitmark.cycle;
            if cycle <= loop_cycle || cycle == self.hit_seen[slot] {
                continue;
            }
            self.hit_seen[slot] = cycle;
            if matches!(hitmark.kind, HITMARK_BLOCK | HITMARK_DAMAGE) {
                events.push(slot as u8, hitmark.value, hitmark.kind);
            }
        }
        events
    }
}

#[derive(Debug, Clone, Copy)]
struct HitEvent {
    slot: u8,
    value: i32,
    kind: i32,
}

/// Owned, allocation-free iterator returned by [`HitOnset::observe`].
#[derive(Debug, Clone, Copy)]
pub struct HitOnsets {
    rows: [HitEvent; 4],
    len: u8,
    next: u8,
}

impl HitOnsets {
    const EMPTY_EVENT: HitEvent = HitEvent {
        slot: 0,
        value: 0,
        kind: -1,
    };

    const fn empty() -> Self {
        Self {
            rows: [Self::EMPTY_EVENT; 4],
            len: 0,
            next: 0,
        }
    }

    fn push(&mut self, slot: u8, value: i32, kind: i32) {
        let index = usize::from(self.len);
        self.rows[index] = HitEvent { slot, value, kind };
        self.len += 1;
    }
}

impl Iterator for HitOnsets {
    type Item = (usize, i32, i32);

    fn next(&mut self) -> Option<Self::Item> {
        if self.next >= self.len {
            return None;
        }
        let event = self.rows[usize::from(self.next)];
        self.next += 1;
        Some((usize::from(event.slot), event.value, event.kind))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = usize::from(self.len - self.next);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for HitOnsets {}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ThreatEvents {
    pub damage: u16,
    pub positive_protected: u8,
}

/// Fixed-size attack evidence. No actor names or view rows are copied into it.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DueAttribution {
    None,
    Unique(ActorRef),
    Ambiguous,
}
#[derive(Debug, Clone, Copy)]
pub struct ThreatSet {
    rows: [Threat; 4],
    hit_onset: HitOnset,
    impact_seen: i32,
    overflow_until: u16,
    unknown_tick: u16,
    unknown_until: u16,
    flags: u8,
}

impl Default for ThreatSet {
    fn default() -> Self {
        Self {
            rows: [Threat::EMPTY; 4],
            hit_onset: HitOnset::new(),
            impact_seen: -1,
            overflow_until: 0,
            unknown_tick: 0,
            unknown_until: 0,
            flags: 0,
        }
    }
}

const _: () = assert!(std::mem::size_of::<Threat>() <= 40);
const _: () = assert!(std::mem::size_of::<ThreatSet>() <= 176);

impl ThreatSet {
    /// Consume all newly observed attack evidence in a single allocation-free pass.
    pub fn observe(&mut self, frame: &Frame<'_>, tables: &CombatTables, tick: u16) -> ThreatEvents {
        self.expire_unknown(tick);
        self.expire_rows(tick);
        self.clear_npc_hints();

        self.observe_projectiles(frame, tables, tick);
        let hitmarks = HitmarksView {
            marks: frame.hitmarks,
            loop_cycle: frame.loop_cycle,
        };
        for npc in frame.npcs {
            self.observe_npc(npc, frame, &hitmarks, tables, tick);
        }
        for player in frame.players {
            self.observe_player(player, frame, tables, tick);
        }
        self.observe_impact_spot(frame, tables, tick);

        let active_protect = select::active_protect(frame, tables);
        self.update_unknown_fallbacks(tables, tick, active_protect);
        let live_before_hits = self.iter(tick).count();
        let facing = sole_facing(frame);
        let onsets = self.hit_onset.observe(&frame.hitmarks, frame.loop_cycle);
        let mut events = ThreatEvents::default();
        for (_, value, kind) in onsets {
            let source = match self.due_source(tick) {
                DueAttribution::Unique(actor) => Some(actor),
                DueAttribution::Ambiguous => None,
                DueAttribution::None if !self.has_unattributed_event(tick) => facing,
                DueAttribution::None => None,
            };
            let Some(source) = source else {
                self.mark_unknown(tick);
                continue;
            };
            let Some(index) = self.ensure_actor(source, frame, tables, tick) else {
                self.mark_unknown(tick);
                continue;
            };
            let row = &mut self.rows[index];
            Self::record_impact(row, tick);
            if kind == HITMARK_DAMAGE && value > 0 {
                events.damage = events
                    .damage
                    .saturating_add(u16::try_from(value).unwrap_or(u16::MAX));
                if active_protect.is_some() {
                    events.positive_protected = events.positive_protected.saturating_add(1);
                }
                if source.kind == ActorKind::Player && live_before_hits == 1 {
                    let protected = active_protect == Some(row.style);
                    row.set_max_hit(select::facts::player_max_hit_floor(value, protected));
                }
                self.advance_fallback(index, tick, active_protect, live_before_hits);
            }
        }
        for row in &mut self.rows {
            if row.has_due() && row.due_tick == tick {
                row.clear_due();
            }
        }
        self.retire_expired(tick);
        events
    }
    /// Hunt's isolate publishes the same actor identity, target, and
    /// in-combat facts in compact scene rows. Retain native fact-backed
    /// threat hints without reconstructing `NpcView` or a `Frame`.
    pub fn observe_hunt(
        &mut self,
        npcs: impl IntoIterator<Item = (i32, i32, bool, i32, i32)>,
        local_player_slot: i32,
        tables: &CombatTables,
        tick: u16,
    ) {
        self.expire_unknown(tick);
        self.expire_rows(tick);
        self.clear_npc_hints();
        for (npc_index, ident, in_combat, target_kind, target_index) in npcs {
            if !in_combat || target_kind != 2 || target_index != local_player_slot {
                continue;
            }
            let Ok(index) = u16::try_from(npc_index) else {
                continue;
            };
            let actor = ActorRef {
                kind: ActorKind::Npc,
                index,
            };
            self.forget_reused_slot(actor, ident);
            let Some(row_index) = self.ensure_npc(actor, ident, tables, tick) else {
                continue;
            };
            self.rows[row_index].set_fact_live(true);
        }
        self.retire_expired(tick);
    }

    /// Live rows, including an NPC's current in-combat hint and recent event rows.
    pub fn iter(&self, tick: u16) -> impl Iterator<Item = &Threat> {
        self.rows.iter().filter(move |row| row.is_live(tick))
    }

    /// Sum one attack event's residual damage. `None` is explicitly Unknown.
    pub fn danger(
        &self,
        frame: &Frame<'_>,
        tables: &CombatTables,
        tick: u16,
        shield: bool,
        antifire: bool,
    ) -> Option<i32> {
        self.danger_with(
            tables,
            tick,
            shield,
            antifire,
            select::active_protect(frame, tables),
        )
    }

    /// The input phase installs a planned protection before NPC decisions.
    pub(crate) fn danger_with(
        &self,
        tables: &CombatTables,
        tick: u16,
        shield: bool,
        antifire: bool,
        protect: Option<StyleObs>,
    ) -> Option<i32> {
        self.danger_except(tables, tick, (shield, antifire), protect, None)
    }

    /// A confirmed corpse can retain an event row until its threat TTL expires.
    pub(crate) fn danger_except(
        &self,
        tables: &CombatTables,
        tick: u16,
        (shield, antifire): (bool, bool),
        protect: Option<StyleObs>,
        corpse: Option<ActorRef>,
    ) -> Option<i32> {
        if self.has_unknown(tick) {
            return None;
        }
        let mut total = 0i32;
        for threat in self
            .iter(tick)
            .filter(|threat| Some(threat.actor) != corpse)
        {
            let style = if threat.style == StyleObs::Unknown {
                threat.fallback_style()
            } else {
                threat.style
            };
            let residual = select::residual(threat, style, protect, tables, shield, antifire)?;
            total = total.saturating_add(residual);
        }
        Some(total)
    }

    pub fn has_overflow(&self, tick: u16) -> bool {
        self.flags & GLOBAL_OVERFLOW != 0 && tick_until(self.overflow_until, tick)
    }

    pub fn has_unattributed_event(&self, tick: u16) -> bool {
        self.flags & GLOBAL_UNKNOWN != 0
            && tick_reached(tick, self.unknown_tick)
            && (tick == self.unknown_until || tick_until(self.unknown_until, tick))
    }

    fn has_unknown(&self, tick: u16) -> bool {
        self.has_overflow(tick) || self.has_unattributed_event(tick)
    }

    fn mark_unknown(&mut self, tick: u16) {
        self.mark_unknown_window(tick, tick, tick);
    }

    fn mark_unknown_window(&mut self, now: u16, start: u16, until: u16) {
        let pending = self.flags & GLOBAL_UNKNOWN != 0
            && (self.unknown_until == now || tick_until(self.unknown_until, now));
        if pending {
            let old_start = if tick_reached(now, self.unknown_tick) {
                0
            } else {
                self.unknown_tick.wrapping_sub(now)
            };
            let new_start = if tick_reached(now, start) {
                0
            } else {
                start.wrapping_sub(now)
            };
            let old_end = self.unknown_until.wrapping_sub(now);
            let new_end = until.wrapping_sub(now);
            self.unknown_tick = now.wrapping_add(old_start.min(new_start));
            self.unknown_until = now.wrapping_add(old_end.max(new_end));
        } else {
            self.unknown_tick = start;
            self.unknown_until = until;
        }
        self.flags |= GLOBAL_UNKNOWN;
    }

    fn expire_unknown(&mut self, tick: u16) {
        if self.flags & GLOBAL_UNKNOWN != 0
            && tick != self.unknown_until
            && tick_reached(tick, self.unknown_until)
        {
            self.flags &= !GLOBAL_UNKNOWN;
        }
    }
    fn mark_projectile_unknown(&mut self, projectile: &ProjectileView, tick: u16, loop_cycle: i32) {
        match projectile_due_window(projectile, tick, loop_cycle) {
            Some((start, until)) if until == tick || tick_until(until, tick) => {
                self.mark_unknown_window(tick, start, until);
            }
            Some(_) => {}
            None => self.mark_unknown(tick),
        }
    }
    fn mark_overflow(&mut self, tick: u16) {
        self.flags |= GLOBAL_OVERFLOW;
        self.overflow_until = tick.wrapping_add(THREAT_TTL);
    }

    fn expire_rows(&mut self, tick: u16) {
        for row in &mut self.rows {
            if row.valid() && !row.is_live(tick) {
                *row = Threat::EMPTY;
            }
        }
        for row in &mut self.rows {
            if row.has_due() && row.due_tick != tick && tick_reached(tick, row.due_tick) {
                row.clear_due();
            }
        }
        if self.flags & GLOBAL_OVERFLOW != 0 && !tick_until(self.overflow_until, tick) {
            self.flags &= !GLOBAL_OVERFLOW;
        }
    }

    fn retire_expired(&mut self, tick: u16) {
        for row in &mut self.rows {
            if row.valid() && !row.is_live(tick) {
                *row = Threat::EMPTY;
            }
        }
    }

    fn observe_npc(
        &mut self,
        npc: &NpcView,
        frame: &Frame<'_>,
        hitmarks: &HitmarksView,
        tables: &CombatTables,
        tick: u16,
    ) {
        let Some(index) = u16::try_from(npc.index).ok() else {
            return;
        };
        let actor = ActorRef {
            kind: ActorKind::Npc,
            index,
        };
        let ident = npc
            .r#type
            .and_then(|id| i32::try_from(id).ok())
            .unwrap_or(-1);
        let evidence = npc_local_attack_evidence(npc, frame.me(), Some(hitmarks), tables);
        let faces = evidence.targets_local;
        let hint = faces && npc.in_combat;
        let existing = self.find_identity(actor, ident);
        let animation = npc.animation;
        let anim_mask = tables.style_seq(animation);
        let spot = tables.style_spotanim(npc.spot_animation);
        if existing.is_none() && !hint && !evidence.is_live() {
            self.forget_reused_slot(actor, ident);
            return;
        }
        self.forget_reused_slot(actor, ident);
        let Some(row_index) = self.ensure_npc(actor, ident, tables, tick) else {
            return;
        };
        let row = &mut self.rows[row_index];
        row.set_fact_live(hint);
        let old_animation = row.animation_seen;
        let old_frame = row.animation_frame;
        let frame_now = clamp_frame(npc.animation_frame);
        let anim_onset =
            animation != old_animation || (animation == old_animation && frame_now < old_frame);
        if faces && anim_onset && animation >= 0 {
            let style = anim_mask.map_or(StyleObs::Unknown, select::style_from_mask);
            if style == StyleObs::Unknown {
                Self::record_unknown_onset(row, tick, frame.loop_cycle);
            } else {
                Self::record_event(
                    row,
                    style,
                    PRIORITY_ANIMATION,
                    tick,
                    frame.loop_cycle,
                    Some(tick),
                );
            }
        }
        row.animation_seen = animation;
        row.animation_frame = frame_now;
        if faces
            && cycle_recent(frame.loop_cycle, npc.spot_animation_stamp)
            && cycle_newer(npc.spot_animation_stamp, row.last_cycle)
        {
            if let Some(style) = spot
                .filter(|value| value.where_ == StyleWhere::Attacker)
                .map(|value| select::style_from_mask(value.style))
            {
                let event_tick = cycle_to_tick(tick, frame.loop_cycle, npc.spot_animation_stamp);
                if style == StyleObs::Unknown {
                    Self::record_unknown_onset(row, event_tick, npc.spot_animation_stamp);
                } else {
                    Self::record_event(
                        row,
                        style,
                        PRIORITY_SPOT,
                        event_tick,
                        npc.spot_animation_stamp,
                        Some(event_tick),
                    );
                }
            }
        }
        row.rate = tables.npc(ident).map_or(0, |fact| tables.npc_rate(fact));
    }

    fn observe_player(
        &mut self,
        player: &PlayerView,
        frame: &Frame<'_>,
        tables: &CombatTables,
        tick: u16,
    ) {
        let Some(index) = u16::try_from(player.index).ok() else {
            return;
        };
        let actor = ActorRef {
            kind: ActorKind::Player,
            index,
        };
        let ident = player
            .actor
            .name
            .as_deref()
            .map(select::ident::fnv1a)
            .unwrap_or(0);
        let faces = faces_us(player.actor.target, frame.me());
        let existing = self.find_identity(actor, ident);
        let animation = player.actor.animation;
        let anim_mask = tables.style_seq(animation);
        let has_attack_animation =
            animation >= 0 && anim_mask.as_ref().is_some_and(|mask| mask.bits() != 0);
        let spot = tables.style_spotanim(player.actor.spot_animation);
        let has_attack_spot = faces
            && cycle_recent(frame.loop_cycle, player.actor.spot_animation_stamp)
            && spot.is_some_and(|row| row.where_ == StyleWhere::Attacker);
        if existing.is_none() && !(faces && has_attack_animation) && !has_attack_spot {
            self.forget_reused_slot(actor, ident);
            return;
        }
        self.forget_reused_slot(actor, ident);
        let Some(row_index) = self.ensure_player(player, actor, ident, tables, tick) else {
            return;
        };
        let row = &mut self.rows[row_index];
        let (visible_style, rate, family) = player_style(player, tables);
        let weapon_changed = family != row.projectile_family() || rate != row.rate;
        if !row.has_event() || weapon_changed {
            row.style = visible_style;
        }
        row.rate = rate;
        if !row.has_due() {
            row.set_projectile_family(family);
        }
        let old_animation = row.animation_seen;
        let old_frame = row.animation_frame;
        let frame_now = clamp_frame(player.actor.animation_frame);
        let anim_onset =
            animation != old_animation || (animation == old_animation && frame_now < old_frame);
        if faces && anim_onset && animation >= 0 {
            let style = anim_mask.map_or(StyleObs::Unknown, select::style_from_mask);
            if style == StyleObs::Unknown {
                Self::record_unknown_onset(row, tick, frame.loop_cycle);
            } else {
                Self::record_event(
                    row,
                    style,
                    PRIORITY_ANIMATION,
                    tick,
                    frame.loop_cycle,
                    Some(tick),
                );
            }
        }
        row.animation_seen = animation;
        row.animation_frame = frame_now;
        if faces
            && cycle_recent(frame.loop_cycle, player.actor.spot_animation_stamp)
            && cycle_newer(player.actor.spot_animation_stamp, row.last_cycle)
        {
            if let Some(style) = spot
                .filter(|value| value.where_ == StyleWhere::Attacker)
                .map(|value| select::style_from_mask(value.style))
            {
                let event_tick =
                    cycle_to_tick(tick, frame.loop_cycle, player.actor.spot_animation_stamp);
                if style == StyleObs::Unknown {
                    Self::record_unknown_onset(row, event_tick, player.actor.spot_animation_stamp);
                } else {
                    Self::record_event(
                        row,
                        style,
                        PRIORITY_SPOT,
                        event_tick,
                        player.actor.spot_animation_stamp,
                        Some(event_tick),
                    );
                }
            }
        }
    }

    fn take_launcher(
        &self,
        frame: &Frame<'_>,
        src: WorldTile,
        max_distance: u32,
    ) -> Result<Option<ActorRef>, ()> {
        let mut source = None;
        for npc in frame.npcs {
            if npc_launch_distance(npc, src) > max_distance {
                continue;
            }
            let Some(index) = u16::try_from(npc.index).ok() else {
                continue;
            };
            let actor = ActorRef {
                kind: ActorKind::Npc,
                index,
            };
            let ident = npc
                .r#type
                .and_then(|id| i32::try_from(id).ok())
                .unwrap_or(-1);
            if !(faces_us(npc.target, frame.me()) || self.find_identity(actor, ident).is_some()) {
                continue;
            }
            if source.replace(actor).is_some() {
                return Err(());
            }
        }
        for player in frame.players {
            if tile_chebyshev(player.actor.tile, src) > max_distance {
                continue;
            }
            let Some(index) = u16::try_from(player.index).ok() else {
                continue;
            };
            let actor = ActorRef {
                kind: ActorKind::Player,
                index,
            };
            let ident = player
                .actor
                .name
                .as_deref()
                .map(select::ident::fnv1a)
                .unwrap_or(0);
            if !(faces_us(player.actor.target, frame.me())
                || self.find_identity(actor, ident).is_some())
            {
                continue;
            }
            if source.replace(actor).is_some() {
                return Err(());
            }
        }
        Ok(source)
    }

    fn observe_projectiles(&mut self, frame: &Frame<'_>, tables: &CombatTables, tick: u16) {
        let me = ActorTargetView {
            kind: ActorKind::Player,
            index: frame.me(),
        };
        for projectile in frame
            .projectiles
            .iter()
            .filter(|projectile| projectile.target == Some(me))
        {
            // t1 is the future visual-flight start, not packet publication.
            // All main families start 32..=51 client cycles after emission.
            if !cycle_recent(frame.loop_cycle, projectile.t1.saturating_sub(32)) {
                continue;
            }
            let duplicate_launch = frame
                .projectiles
                .iter()
                .filter(|candidate| {
                    candidate.target == Some(me)
                        && candidate.src == projectile.src
                        && candidate.t1 == projectile.t1
                })
                .take(2)
                .count()
                > 1;
            if duplicate_launch {
                self.mark_projectile_unknown(projectile, tick, frame.loop_cycle);
                continue;
            }
            let style = tables
                .style_spotanim(projectile.spotanim)
                .filter(|row| row.where_ == StyleWhere::Projectile)
                .map_or(StyleObs::Unknown, |row| select::style_from_mask(row.style));
            // Fine-pixel launch coords often decode one tile off the rendered
            // NPC. Prefer an exact tile/network hit; if none, the unique
            // adjacent facing actor.
            let actor = match self.take_launcher(frame, projectile.src, 0) {
                Ok(Some(actor)) => actor,
                Err(()) => {
                    self.mark_projectile_unknown(projectile, tick, frame.loop_cycle);
                    continue;
                }
                Ok(None) => match self.take_launcher(frame, projectile.src, 1) {
                    Ok(Some(actor)) => actor,
                    Err(()) | Ok(None) => {
                        self.mark_projectile_unknown(projectile, tick, frame.loop_cycle);
                        continue;
                    }
                },
            };
            let Some(index) = self.ensure_actor(actor, frame, tables, tick) else {
                self.mark_projectile_unknown(projectile, tick, frame.loop_cycle);
                continue;
            };
            let family = projectile_family(actor, style, frame, tables);
            let launch_cycle = projectile_launch_cycle(projectile, family)
                .unwrap_or_else(|| projectile.t1.saturating_sub(51).max(0));
            if !cycle_recent(frame.loop_cycle, launch_cycle) {
                continue;
            }
            let launch_tick = cycle_to_tick(tick, frame.loop_cycle, launch_cycle);
            let due = projectile_due_tick(projectile, family, tick, frame.loop_cycle);
            if family == ProjectileFamily::Unknown || due.is_none() {
                self.mark_projectile_unknown(projectile, tick, frame.loop_cycle);
            }
            let mut ambiguous_range = None;
            {
                let row = &mut self.rows[index];
                let new_launch = cycle_newer(launch_cycle, row.last_cycle)
                    || (style != StyleObs::Unknown
                        && launch_cycle == row.last_cycle
                        && tick_age(launch_tick, row.last_seen) <= 2
                        && row.priority() < PRIORITY_PROJECTILE);
                if !new_launch {
                    continue;
                }
                if style == StyleObs::Unknown {
                    Self::record_unknown_onset(row, launch_tick, launch_cycle);
                } else {
                    Self::record_event(
                        row,
                        style,
                        PRIORITY_PROJECTILE,
                        launch_tick,
                        launch_cycle,
                        Some(launch_tick),
                    );
                }
                if let Some(due_tick) =
                    due.filter(|due_tick| *due_tick == tick || tick_until(*due_tick, tick))
                {
                    let overlaps =
                        row.has_due() && (row.due_tick == tick || tick_until(row.due_tick, tick));
                    if overlaps {
                        let old_due = row.due_tick;
                        let first = if old_due.wrapping_sub(tick) <= due_tick.wrapping_sub(tick) {
                            old_due
                        } else {
                            due_tick
                        };
                        let latest = if tick_until(due_tick, old_due) {
                            due_tick
                        } else {
                            old_due
                        };
                        ambiguous_range = Some((first, latest));
                    } else {
                        row.add_due(due_tick, family);
                    }
                }
            }
            if let Some((start, until)) = ambiguous_range {
                self.mark_unknown_window(tick, start, until);
            }
        }
    }

    fn observe_impact_spot(&mut self, frame: &Frame<'_>, tables: &CombatTables, tick: u16) {
        let stamp = frame.local.player.actor.spot_animation_stamp;
        if stamp < 0
            || !cycle_recent(frame.loop_cycle, stamp)
            || !cycle_newer(stamp, self.impact_seen)
        {
            return;
        }
        self.impact_seen = stamp;
        let style = tables
            .style_spotanim(frame.local.player.actor.spot_animation)
            .filter(|row| row.where_ == StyleWhere::OnUs)
            .map_or(StyleObs::Unknown, |row| select::style_from_mask(row.style));
        if style == StyleObs::Unknown {
            return;
        }
        // An old spell's landing belongs to its retained queue, not to the
        // newest facing actor or the newest attack's style chronology.
        match self.due_source(tick) {
            DueAttribution::Unique(actor) => {
                if let Some(index) = self.ensure_actor(actor, frame, tables, tick) {
                    Self::record_impact(&mut self.rows[index], tick);
                }
                return;
            }
            DueAttribution::Ambiguous => {
                self.mark_unknown(tick);
                return;
            }
            DueAttribution::None => {}
        }
        let Some(actor) = sole_facing(frame) else {
            self.mark_unknown(tick);
            return;
        };
        let Some(index) = self.ensure_actor(actor, frame, tables, tick) else {
            self.mark_unknown(tick);
            return;
        };
        let event_tick = cycle_to_tick(tick, frame.loop_cycle, stamp);
        Self::record_event(
            &mut self.rows[index],
            style,
            PRIORITY_IMPACT,
            event_tick,
            stamp,
            None,
        );
    }

    fn update_unknown_fallbacks(
        &mut self,
        tables: &CombatTables,
        tick: u16,
        active_protect: Option<StyleObs>,
    ) {
        for row in &mut self.rows {
            if !row.is_live(tick)
                || row.actor.kind != ActorKind::Npc
                || row.style != StyleObs::Unknown
            {
                continue;
            }
            let is_dragon = tables
                .npc(row.ident)
                .is_some_and(|fact| fact.dragonfire.is_some());
            if is_dragon {
                row.fallback_since = u16::MAX;
                continue;
            }
            if active_protect == Some(row.fallback_style()) {
                if row.fallback_since == u16::MAX {
                    row.fallback_since = tick;
                }
            } else {
                row.fallback_since = u16::MAX;
            }
        }
    }

    fn advance_fallback(
        &mut self,
        row_index: usize,
        tick: u16,
        protect: Option<StyleObs>,
        live: usize,
    ) {
        if live != 1 {
            return;
        }
        let row = &mut self.rows[row_index];
        let fallback = row.fallback_style();
        if row.actor.kind != ActorKind::Npc
            || row.style != StyleObs::Unknown
            || row.fallback_since == u16::MAX
            || protect != Some(fallback)
            || tick_age(tick, row.fallback_since) < 2
        {
            return;
        }
        let next = match fallback {
            StyleObs::Melee => StyleObs::Ranged,
            StyleObs::Ranged => StyleObs::Magic,
            StyleObs::Magic | StyleObs::Dragonfire | StyleObs::Unknown => StyleObs::Melee,
        };
        row.set_fallback_style(next);
        row.fallback_since = tick;
    }

    fn ensure_actor(
        &mut self,
        actor: ActorRef,
        frame: &Frame<'_>,
        tables: &CombatTables,
        tick: u16,
    ) -> Option<usize> {
        match actor.kind {
            ActorKind::Npc => {
                let npc = frame
                    .npcs
                    .iter()
                    .find(|npc| npc.index == usize::from(actor.index))?;
                let ident = npc
                    .r#type
                    .and_then(|id| i32::try_from(id).ok())
                    .unwrap_or(-1);
                self.forget_reused_slot(actor, ident);
                self.ensure_npc(actor, ident, tables, tick)
            }
            ActorKind::Player => {
                let player = frame
                    .players
                    .iter()
                    .find(|player| player.index == usize::from(actor.index))?;
                let ident = player
                    .actor
                    .name
                    .as_deref()
                    .map(select::ident::fnv1a)
                    .unwrap_or(0);
                self.forget_reused_slot(actor, ident);
                self.ensure_player(player, actor, ident, tables, tick)
            }
        }
    }

    fn ensure_npc(
        &mut self,
        actor: ActorRef,
        ident: i32,
        tables: &CombatTables,
        tick: u16,
    ) -> Option<usize> {
        if let Some(index) = self.find_identity(actor, ident) {
            return Some(index);
        }
        let fact = tables.npc(ident);
        let style = fact.map_or(StyleObs::Unknown, npc_main_style);
        let rate = fact.map_or(0, |row| tables.npc_rate(row));
        let maximum = fact.and_then(select::facts::npc_max_hit);
        let row = Threat::new(actor, ident, style, rate, maximum, tick);
        self.insert_row(row, tick)
    }

    fn ensure_player(
        &mut self,
        player: &PlayerView,
        actor: ActorRef,
        ident: i32,
        tables: &CombatTables,
        tick: u16,
    ) -> Option<usize> {
        if let Some(index) = self.find_identity(actor, ident) {
            return Some(index);
        }
        let (style, rate, family) = player_style(player, tables);
        let mut row = Threat::new(actor, ident, style, rate, None, tick);
        row.animation_seen = -1;
        row.animation_frame = -1;
        row.set_projectile_family(family);
        self.insert_row(row, tick)
    }

    fn find_identity(&self, actor: ActorRef, ident: i32) -> Option<usize> {
        self.rows
            .iter()
            .position(|row| row.valid() && row.actor == actor && row.ident == ident)
    }

    fn clear_npc_hints(&mut self) {
        for row in &mut self.rows {
            if row.actor.kind == ActorKind::Npc {
                row.set_fact_live(false);
            }
        }
    }

    fn forget_reused_slot(&mut self, actor: ActorRef, ident: i32) {
        for row in &mut self.rows {
            if row.valid()
                && row.actor.kind == actor.kind
                && row.actor.index == actor.index
                && row.ident != ident
            {
                *row = Threat::EMPTY;
            }
        }
    }

    fn insert_row(&mut self, row: Threat, tick: u16) -> Option<usize> {
        if let Some(index) = self.rows.iter().position(|old| !old.valid()) {
            self.rows[index] = row;
            return Some(index);
        }
        let (weakest, weight) = self
            .rows
            .iter()
            .enumerate()
            .min_by_key(|(_, old)| old.weight())
            .map(|(i, old)| (i, old.weight()))?;
        self.mark_overflow(tick);
        if row.weight() > weight {
            self.rows[weakest] = row;
            Some(weakest)
        } else {
            None
        }
    }

    fn record_event(
        row: &mut Threat,
        style: StyleObs,
        priority: u8,
        tick: u16,
        cycle: i32,
        decision_tick: Option<u16>,
    ) {
        let same_event = row.has_event() && tick_age(tick, row.last_seen) <= 2;
        if !same_event {
            row.style = style;
            row.last_seen = tick;
            row.mark_event();
            row.set_priority(priority);
            row.add_style_event(style);
            row.last_cycle = cycle;
        } else {
            let prior_priority = row.priority();
            if priority > prior_priority {
                row.style = style;
                row.set_priority(priority);
                if prior_priority == PRIORITY_IMPACT {
                    row.add_style_event(style);
                } else {
                    row.refine_latest_style(style);
                }
            }
            if cycle_newer(cycle, row.last_cycle) {
                row.last_cycle = cycle;
            }
        }
        if let Some(decision_tick) = decision_tick {
            if tick_until(decision_tick, row.last_decision) {
                row.last_decision = decision_tick;
            }
        }
    }

    fn record_unknown_onset(row: &mut Threat, tick: u16, cycle: i32) {
        if !row.has_event() || tick_age(tick, row.last_seen) > 2 {
            row.last_seen = tick;
            row.mark_event();
            row.set_priority(PRIORITY_IMPACT);
        }
        if tick_until(tick, row.last_decision) {
            row.last_decision = tick;
        }
        if cycle_newer(cycle, row.last_cycle) {
            row.last_cycle = cycle;
        }
    }

    fn record_impact(row: &mut Threat, tick: u16) {
        if !row.has_event() || tick_age(tick, row.last_seen) > 2 {
            row.last_seen = tick;
            row.mark_event();
            row.set_priority(PRIORITY_IMPACT);
        }
    }

    fn due_source(&self, tick: u16) -> DueAttribution {
        if self.has_unattributed_event(tick) {
            return DueAttribution::Ambiguous;
        }
        let mut found = None;
        for row in self.rows.iter().filter(|row| {
            row.is_live(tick)
                && row.has_due()
                && (row.due_tick == tick
                    || (row.projectile_family() == ProjectileFamily::Unknown
                        && tick_until(row.due_tick, tick)))
        }) {
            if row.projectile_family() == ProjectileFamily::Unknown
                || found.replace(row.actor).is_some()
            {
                return DueAttribution::Ambiguous;
            }
        }
        found.map_or(DueAttribution::None, DueAttribution::Unique)
    }
}

/// The sole actor currently facing the local player, regardless of attackability.
pub fn sole_facing(frame: &Frame<'_>) -> Option<ActorRef> {
    let me = frame.me();
    let mut found = None;
    for npc in frame.npcs.iter().filter(|row| faces_us(row.target, me)) {
        if found.is_some() {
            return None;
        }
        found = Some(ActorRef {
            kind: ActorKind::Npc,
            index: u16::try_from(npc.index).ok()?,
        });
    }
    for player in frame
        .players
        .iter()
        .filter(|row| faces_us(row.actor.target, me))
    {
        if found.is_some() {
            return None;
        }
        found = Some(ActorRef {
            kind: ActorKind::Player,
            index: u16::try_from(player.index).ok()?,
        });
    }
    found
}

fn faces_us(target: Option<ActorTargetView>, local_slot: usize) -> bool {
    target.is_some_and(|target| target.kind == ActorKind::Player && target.index == local_slot)
}

fn tile_chebyshev(a: WorldTile, b: WorldTile) -> u32 {
    if a.level != b.level {
        u32::MAX
    } else {
        a.x.abs_diff(b.x).max(a.z.abs_diff(b.z))
    }
}

fn npc_launch_distance(npc: &NpcView, src: WorldTile) -> u32 {
    tile_chebyshev(npc.tile, src).min(tile_chebyshev(npc.network, src))
}

fn npc_main_style(row: &api::game_data::NpcNameRow) -> StyleObs {
    match row.attack_kind {
        Some(api::game_data::NpcAttackKind::Ranged) => StyleObs::Ranged,
        Some(api::game_data::NpcAttackKind::Magic) => StyleObs::Magic,
        Some(api::game_data::NpcAttackKind::Mixed) | None => StyleObs::Melee,
    }
}

fn player_style(player: &PlayerView, tables: &CombatTables) -> (StyleObs, u8, ProjectileFamily) {
    let Some(weapon) = player.weapon else {
        return (StyleObs::Melee, 0, ProjectileFamily::Unknown);
    };
    let Some(fact) = tables.weapon_style(weapon) else {
        return (StyleObs::Melee, 0, ProjectileFamily::Unknown);
    };
    let style = select::style_from_mask(super::tables::StyleMask(fact.style));
    let family = match (style, fact.category) {
        (StyleObs::Magic, _) => ProjectileFamily::PlayerSpell,
        (StyleObs::Ranged, 1 | 2) => ProjectileFamily::PlayerRanged,
        (StyleObs::Ranged, 3 | 4) => ProjectileFamily::PlayerThrown,
        _ => ProjectileFamily::Unknown,
    };
    (
        if style == StyleObs::Unknown {
            StyleObs::Melee
        } else {
            style
        },
        fact.attackrate,
        family,
    )
}
fn projectile_family(
    actor: ActorRef,
    style: StyleObs,
    frame: &Frame<'_>,
    tables: &CombatTables,
) -> ProjectileFamily {
    match (actor.kind, style) {
        (ActorKind::Npc, StyleObs::Ranged) => ProjectileFamily::NpcRanged,
        (ActorKind::Npc, StyleObs::Magic) => ProjectileFamily::NpcSpell,
        (ActorKind::Player, StyleObs::Magic) => ProjectileFamily::PlayerSpell,
        (ActorKind::Player, StyleObs::Ranged) => frame
            .players
            .iter()
            .find(|player| player.index == usize::from(actor.index))
            .map_or(ProjectileFamily::Unknown, |player| {
                player_style(player, tables).2
            }),
        _ => ProjectileFamily::Unknown,
    }
}

/// Recover emission chronology from the family-specific visual start delay.
fn projectile_launch_cycle(projectile: &ProjectileView, family: ProjectileFamily) -> Option<i32> {
    projectile
        .t1
        .checked_sub(projectile_launch_delay(family)?)
        .filter(|cycle| *cycle >= 0)
}

fn projectile_launch_delay(family: ProjectileFamily) -> Option<i32> {
    match family {
        ProjectileFamily::NpcRanged | ProjectileFamily::PlayerThrown => Some(32),
        ProjectileFamily::PlayerRanged => Some(41),
        ProjectileFamily::NpcSpell | ProjectileFamily::PlayerSpell => Some(51),
        ProjectileFamily::Unknown => None,
    }
}

/// Reconstruct the hit-queue tick independently of projectile visual flight.
fn projectile_due_tick(
    projectile: &ProjectileView,
    family: ProjectileFamily,
    tick: u16,
    loop_cycle: i32,
) -> Option<u16> {
    let flight = projectile.t2.checked_sub(projectile.t1)?;
    if flight < 0 {
        return None;
    }
    let launch_delay = projectile_launch_delay(family)?;
    let launch_cycle = projectile_launch_cycle(projectile, family)?;
    let delay = launch_delay.saturating_add(flight);
    let spell_offset = i32::from(family == ProjectileFamily::PlayerSpell);
    let due_after_launch = u16::try_from(delay.div_euclid(30).saturating_add(spell_offset)).ok()?;
    Some(cycle_to_tick(tick, loop_cycle, launch_cycle).wrapping_add(due_after_launch))
}

/// The narrowest launch-relative queue window available when family is unknown.
fn projectile_due_window(
    projectile: &ProjectileView,
    tick: u16,
    loop_cycle: i32,
) -> Option<(u16, u16)> {
    let flight = projectile.t2.checked_sub(projectile.t1)?;
    if !(0..=i32::from(THREAT_TTL) * 30).contains(&flight) {
        return None;
    }
    let mut earliest = None;
    let mut latest = None;
    for family in [
        ProjectileFamily::NpcRanged,
        ProjectileFamily::PlayerRanged,
        ProjectileFamily::NpcSpell,
        ProjectileFamily::PlayerSpell,
    ] {
        let Some(due) = projectile_due_tick(projectile, family, tick, loop_cycle) else {
            continue;
        };
        let offset = due.wrapping_sub(tick) as i16;
        if earliest.is_none_or(|first: u16| offset < first.wrapping_sub(tick) as i16) {
            earliest = Some(due);
        }
        if latest.is_none_or(|last: u16| offset > last.wrapping_sub(tick) as i16) {
            latest = Some(due);
        }
    }
    Some((earliest?, latest?))
}

fn cycle_recent(now: i32, then: i32) -> bool {
    if then < 0 {
        return false;
    }
    let age = now.wrapping_sub(then);
    (0..=i32::from(THREAT_TTL) * 30).contains(&age) || (0..=30).contains(&then.wrapping_sub(now))
}

fn cycle_to_tick(tick: u16, loop_cycle: i32, event_cycle: i32) -> u16 {
    let age = loop_cycle.wrapping_sub(event_cycle).max(0).div_euclid(30);
    tick.wrapping_sub(u16::try_from(age).unwrap_or(u16::MAX))
}

fn tick_reached(now: u16, deadline: u16) -> bool {
    now.wrapping_sub(deadline) < 0x8000
}

fn clamp_frame(frame: i32) -> i16 {
    i16::try_from(frame).unwrap_or(if frame < 0 { -1 } else { i16::MAX })
}

fn tick_age(now: u16, then: u16) -> u16 {
    now.wrapping_sub(then)
}

fn tick_until(until: u16, now: u16) -> bool {
    let remaining = until.wrapping_sub(now);
    remaining != 0 && remaining < 0x8000
}

fn cycle_newer(now: i32, then: i32) -> bool {
    now.wrapping_sub(then) > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(kind: ActorKind, index: u16) -> ActorRef {
        ActorRef { kind, index }
    }

    fn projectile(t1: i32, t2: i32) -> ProjectileView {
        ProjectileView {
            spotanim: 0,
            level: 0,
            src: api::snapshot::WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            target: None,
            t1,
            t2,
        }
    }

    #[test]
    fn hit_onsets_consume_poison_and_unknown_cycles_without_attributing_them() {
        let mut hitmarks = [HitmarkView {
            value: 0,
            kind: -1,
            cycle: 0,
        }; 4];
        hitmarks[0] = HitmarkView {
            value: 0,
            kind: HITMARK_POISON,
            cycle: 110,
        };
        hitmarks[1] = HitmarkView {
            value: 0,
            kind: HITMARK_BLOCK,
            cycle: 111,
        };
        hitmarks[2] = HitmarkView {
            value: 9,
            kind: 99,
            cycle: 112,
        };
        hitmarks[3] = HitmarkView {
            value: 12,
            kind: HITMARK_DAMAGE,
            cycle: 100,
        };
        let mut onset = HitOnset::new();

        let mut events = onset.observe(&hitmarks, 100);
        assert_eq!(events.len(), 1);
        assert_eq!(events.next(), Some((1, 0, HITMARK_BLOCK)));
        assert_eq!(events.next(), None);
        assert_eq!(onset.hit_seen, [110, 111, 112, -1]);

        hitmarks[0] = HitmarkView {
            value: 9,
            kind: HITMARK_DAMAGE,
            cycle: 110,
        };
        assert_eq!(onset.observe(&hitmarks, 100).len(), 0);
        hitmarks[0].cycle = 120;
        hitmarks[0].value = 9;
        let mut events = onset.observe(&hitmarks, 100);
        assert_eq!(events.len(), 1);
        assert_eq!(events.next(), Some((0, 9, HITMARK_DAMAGE)));
    }

    #[test]
    fn unknown_projectile_due_window_is_inclusive_and_wrap_safe() {
        let mut threats = ThreatSet::default();
        threats.mark_unknown_window(u16::MAX - 1, u16::MAX, 0);
        assert!(!threats.has_unattributed_event(u16::MAX - 1));
        assert!(threats.has_unattributed_event(u16::MAX));
        assert!(threats.has_unattributed_event(0));
        threats.expire_unknown(1);
        assert!(!threats.has_unattributed_event(1));
    }

    #[test]
    fn overlapping_due_sources_are_not_attributed_to_one_actor() {
        let mut threats = ThreatSet::default();
        let first = actor(ActorKind::Npc, 1);
        let second = actor(ActorKind::Player, 2);
        let mut row = Threat::new(first, 10, StyleObs::Ranged, 4, Some(7), 10);
        row.add_due(12, ProjectileFamily::NpcRanged);
        threats.rows[0] = row;
        assert_eq!(threats.due_source(12), DueAttribution::Unique(first));

        let mut row = Threat::new(second, 20, StyleObs::Magic, 5, None, 10);
        row.add_due(12, ProjectileFamily::PlayerSpell);
        threats.rows[1] = row;
        assert_eq!(threats.due_source(12), DueAttribution::Ambiguous);

        let mut unknown = ThreatSet::default();
        unknown.mark_unknown_window(10, 12, 13);
        assert_eq!(unknown.due_source(11), DueAttribution::None);
        assert_eq!(unknown.due_source(12), DueAttribution::Ambiguous);
    }

    #[test]
    fn due_projectile_actor_wins_over_current_facing_actor_for_late_hit() {
        let mut threats = ThreatSet::default();
        let launcher = actor(ActorKind::Npc, 1);
        let current_facing = actor(ActorKind::Player, 2);
        let mut launched = Threat::new(launcher, 10, StyleObs::Ranged, 4, Some(7), 10);
        launched.add_due(12, ProjectileFamily::NpcRanged);
        threats.rows[0] = launched;
        threats.rows[1] = Threat::new(current_facing, 11, StyleObs::Melee, 4, Some(3), 12);

        // The projectile visual is no longer needed: its queued impact still
        // identifies its launcher, even if a different actor now faces us.
        assert_eq!(threats.due_source(12), DueAttribution::Unique(launcher));
        ThreatSet::record_impact(&mut threats.rows[0], 12);
        assert!(threats.rows[0].has_event());
        assert!(!threats.rows[1].has_event());
    }

    #[test]
    fn distinct_projectiles_due_together_make_numeric_hit_unattributed() {
        let mut threats = ThreatSet::default();
        let first = actor(ActorKind::Npc, 1);
        let second = actor(ActorKind::Player, 2);
        let mut npc_shot = Threat::new(first, 10, StyleObs::Ranged, 4, Some(7), 10);
        npc_shot.add_due(12, ProjectileFamily::NpcRanged);
        threats.rows[0] = npc_shot;
        let mut player_shot = Threat::new(second, 20, StyleObs::Ranged, 5, None, 10);
        player_shot.add_due(12, ProjectileFamily::PlayerRanged);
        threats.rows[1] = player_shot;

        assert_eq!(threats.due_source(12), DueAttribution::Ambiguous);
    }

    #[test]
    fn reused_actor_slots_discard_old_attack_evidence() {
        let mut threats = ThreatSet::default();
        let actor = actor(ActorKind::Npc, 4);
        let mut row = Threat::new(actor, 100, StyleObs::Ranged, 4, Some(8), 20);
        row.add_due(22, ProjectileFamily::NpcRanged);
        threats.rows[0] = row;

        threats.forget_reused_slot(actor, 101);

        assert_eq!(threats.iter(20).count(), 0);
    }

    #[test]
    fn absent_npc_hint_expires_instead_of_living_forever() {
        let actor = actor(ActorKind::Npc, 1);
        let mut threats = ThreatSet::default();
        let mut row = Threat::new(actor, 10, StyleObs::Melee, 4, Some(8), 0);
        row.set_fact_live(true);
        threats.rows[0] = row;

        threats.clear_npc_hints();

        assert_eq!(threats.iter(THREAT_TTL + 1).count(), 0);
    }

    #[test]
    fn inactive_unknown_style_fallback_does_not_advance() {
        let mut threats = ThreatSet::default();
        threats.rows[0] = Threat::new(
            actor(ActorKind::Npc, 1),
            10,
            StyleObs::Unknown,
            4,
            Some(8),
            20,
        );

        threats.advance_fallback(0, 24, Some(StyleObs::Melee), 1);

        assert_eq!(threats.rows[0].fallback_style(), StyleObs::Melee);
    }

    #[test]
    fn overflow_keeps_stronger_attacker_and_marks_danger_unknown() {
        let mut threats = ThreatSet::default();
        for (index, max_hit) in [1, 2, 3, 4].into_iter().enumerate() {
            let row = Threat::new(
                actor(
                    ActorKind::Npc,
                    u16::try_from(index).expect("four threat rows"),
                ),
                i32::try_from(index).expect("four threat rows"),
                StyleObs::Melee,
                4,
                Some(max_hit),
                30,
            );
            assert!(threats.insert_row(row, 30).is_some());
        }
        let weak = Threat::new(actor(ActorKind::Npc, 8), 8, StyleObs::Melee, 4, Some(0), 30);
        assert_eq!(threats.insert_row(weak, 30), None);
        assert!(threats.has_overflow(30));
        assert!(threats.has_unknown(30));
        assert!(threats.has_overflow(39));
        assert!(!threats.has_overflow(40));

        let strong = Threat::new(actor(ActorKind::Npc, 9), 9, StyleObs::Melee, 4, Some(5), 30);
        assert!(threats.insert_row(strong, 30).is_some());
        assert!(threats.iter(30).any(|row| row.actor.index == 9));
        assert!(!threats.iter(30).any(|row| row.actor.index == 0));
        assert_eq!(threats.iter(30).count(), 4);
    }

    #[test]
    fn stronger_onset_refines_event_without_losing_rate_clock() {
        let actor = actor(ActorKind::Npc, 3);
        let mut row = Threat::new(actor, 30, StyleObs::Unknown, 4, Some(8), 40);
        ThreatSet::record_event(
            &mut row,
            StyleObs::Ranged,
            PRIORITY_ANIMATION,
            40,
            1_200,
            Some(40),
        );
        ThreatSet::record_event(
            &mut row,
            StyleObs::Magic,
            PRIORITY_PROJECTILE,
            41,
            1_230,
            Some(41),
        );

        assert_eq!(row.style, StyleObs::Magic);
        assert_eq!(row.history_len(), 1);
        assert_eq!(row.recent_position(StyleObs::Magic), Some(0));
        assert_eq!(row.next_decision(), 45);
    }

    #[test]
    fn late_impact_does_not_rewind_newer_style_chronology() {
        let mut row = Threat::new(
            actor(ActorKind::Npc, 3),
            30,
            StyleObs::Unknown,
            4,
            Some(8),
            40,
        );
        ThreatSet::record_event(
            &mut row,
            StyleObs::Magic,
            PRIORITY_PROJECTILE,
            41,
            1_230,
            Some(41),
        );
        let history_len = row.history_len();
        let newer_position = row.recent_position(StyleObs::Magic);
        let decision = row.next_decision();

        ThreatSet::record_impact(&mut row, 42);

        assert_eq!(row.style, StyleObs::Magic);
        assert_eq!(row.history_len(), history_len);
        assert_eq!(row.recent_position(StyleObs::Magic), newer_position);
        assert_eq!(row.next_decision(), decision);
    }

    #[test]
    fn ranged_projectile_due_ticks_include_family_delay_and_flight() {
        for (family, visual_delay, flight_due) in [
            (ProjectileFamily::NpcRanged, 32, 51),
            (ProjectileFamily::PlayerRanged, 41, 52),
            (ProjectileFamily::PlayerThrown, 32, 51),
        ] {
            // Emission is at cycle 1,000 / tick 50. The packet's t1 is the
            // future visual start, not the emission clock.
            let visual_start = 1_000 + visual_delay;
            let zero_flight = projectile(visual_start, visual_start);
            assert_eq!(
                projectile_due_tick(&zero_flight, family, 50, 1_000),
                Some(51)
            );
            let flight = projectile(visual_start, visual_start + 19);
            assert_eq!(
                projectile_due_tick(&flight, family, 50, 1_000),
                Some(flight_due)
            );
            assert_eq!(
                projectile_due_tick(&flight, family, 51, 1_030),
                Some(flight_due),
                "later observation must preserve emission-relative due tick"
            );
        }
    }

    #[test]
    fn projectile_family_windows_cover_exact_cycle_boundaries() {
        for (family, visual_delay, spell_offset) in [
            (ProjectileFamily::NpcRanged, 32, 0),
            (ProjectileFamily::PlayerThrown, 32, 0),
            (ProjectileFamily::PlayerRanged, 41, 0),
            (ProjectileFamily::NpcSpell, 51, 0),
            (ProjectileFamily::PlayerSpell, 51, 1),
        ] {
            let shot = projectile(1_000 + visual_delay, 1_029 + visual_delay);
            assert_eq!(projectile_launch_cycle(&shot, family), Some(1_000));
            assert_eq!(
                projectile_due_tick(&shot, family, 50, 1_000),
                Some(52 + spell_offset),
                "{family:?} publication precedes visual flight"
            );
            assert_eq!(
                projectile_due_tick(&shot, family, 52, 1_060),
                Some(52 + spell_offset),
                "{family:?} late observation keeps the original queue"
            );
        }
        let shot = projectile(1_051, 1_080);
        assert_eq!(projectile_due_window(&shot, 50, 1_000), Some((52, 53)));
        assert_eq!(projectile_due_window(&shot, 51, 1_030), Some((52, 53)));
        assert_eq!(projectile_due_window(&shot, 52, 1_060), Some((52, 53)));
        assert_eq!(
            projectile_due_window(&shot, u16::MAX - 1, 1_000),
            Some((0, 1))
        );

        let malformed = projectile(1_000, 999);
        assert_eq!(
            projectile_due_tick(&malformed, ProjectileFamily::NpcRanged, 50, 1_000),
            None
        );
        assert_eq!(
            projectile_due_window(&projectile(1_000, 1_301), 50, 1_000),
            None
        );
    }
}
