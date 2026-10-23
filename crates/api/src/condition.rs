//! Compact condition facts and the pure poison-state reducer.

use thiserror::Error;

const POISON_PERIOD: u16 = 30;
const LAG: u16 = 2;

/// The observed condition of one account.
///
/// The owner publishes this value only after a complete live frame. Its size is
/// bounded so a slot can retain it without per-frame allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BotCondition {
    /// Snapshot tick of the frame that produced this fact.
    pub tick: u32,
    /// Session state observed by the owner.
    pub session: SessionMark,
    /// Effective hitpoints from the unique hitpoints stat row.
    pub hp: u8,
    /// Base hitpoints from the unique hitpoints stat row.
    pub hp_max: u8,
    /// Whether the hitpoints row was available and representable.
    pub hp_known: bool,
    /// The current poison-state fact.
    pub poison: PoisonState,
    /// Aggregate held ordinary food facts.
    pub food: FoodSummary,
    /// Held cure-dose facts.
    pub cures: CureDoses,
}

impl Default for BotCondition {
    fn default() -> Self {
        Self {
            tick: 0,
            session: SessionMark::Boundary,
            hp: 0,
            hp_max: 0,
            hp_known: false,
            poison: PoisonState::default(),
            food: FoodSummary::default(),
            cures: CureDoses::default(),
        }
    }
}

const _: () = assert!(std::mem::size_of::<BotCondition>() <= 48);

/// Missing input that prevents a complete condition fact.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ConditionError {
    /// The snapshot has no local player.
    #[error("local player is unavailable")]
    NoLocalPlayer,
    /// The snapshot has no unique, usable hitpoints stat row.
    #[error("hitpoints stats are unavailable")]
    StatsUnavailable,
    /// The snapshot has no observed inventory packet.
    #[error("inventory is unavailable")]
    InventoryUnavailable,
}

/// The owner-observed session state.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum SessionMark {
    /// No complete live session frame is available.
    #[default]
    Boundary,
    /// The local player is alive in a complete session frame.
    Live,
    /// The local player has zero hitpoints in a complete session frame.
    Dead,
}

/// Aggregate held ordinary-food facts.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FoodSummary {
    /// Total held count of ordinary food, saturated at `u8::MAX`.
    pub ordinary_count: u8,
    /// Sum of `count * heal` over ordinary food, saturated at `u16::MAX`.
    pub ordinary_heal_total: u16,
    /// Greatest nominal heal among held ordinary food.
    pub best_heal: u8,
    /// Least nominal heal among held ordinary food.
    pub smallest_heal: u8,
    /// Held count of food rows that permit a food-and-skill combo.
    pub combo_count: u8,
    /// Whether the inventory contains more than one ordinary food kind.
    pub more_kinds: bool,
}

/// Held cure-dose counts.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CureDoses {
    /// Held ordinary antipoison doses, saturated at `u8::MAX`.
    pub antipoison: u8,
    /// Held super-antipoison doses, saturated at `u8::MAX`.
    pub super_antipoison: u8,
    /// Held strange fruit count, saturated at `u8::MAX`.
    pub strange_fruit: u8,
}

/// Observed poison state for one session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoisonState {
    /// Poison state is not known after login, reconnect, or an unsupported mark.
    Unknown {
        /// Tick that began the unknown interval.
        since: u16,
    },
    /// The server poison value is known to be nonpositive.
    Clear,
    /// A poison line arrived before the first poison mark.
    Onset {
        /// Tick of the poison line.
        since: u16,
        /// Pulse attributed to the poison source, or a conservative pulse bound.
        q: u8,
        /// A hit arrived after the poison line.
        hit_since_line: bool,
        /// The owner completed or rejected the source-attribution window.
        latched: bool,
    },
    /// A poison mark established the pulse at its latest observed fire.
    Poisoned {
        /// Damage pulse per poison fire.
        per_tick: u8,
        /// Tick of the latest poison mark.
        last_mark: u16,
        /// A hit arrived at or after the latest poison mark.
        hit_since_mark: bool,
    },
    /// A proven cure supplies a lower-bound immunity credit.
    Immune {
        /// Last tick through which the credited immunity is guaranteed.
        until_lower: u16,
    },
}

impl Default for PoisonState {
    fn default() -> Self {
        Self::Unknown { since: 0 }
    }
}

const _: () = assert!(std::mem::size_of::<PoisonState>() <= 8);

/// One explicit event applied to [`PoisonMemory`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoisonEvent {
    /// No poison event was observed.
    None,
    /// A new game session began.
    Reconnect {
        /// Tick at the session boundary.
        tick: u16,
    },
    /// A raw local-player hitmark was observed.
    Hitmark {
        /// Engine hitmark kind.
        kind: u16,
        /// Hitmark value, including zero.
        value: u8,
        /// Tick of the frame that exposed the hitmark.
        tick: u16,
    },
    /// The server reported that the player was poisoned.
    PoisonChat {
        /// Tick of the frame that consumed the line.
        tick: u16,
    },
    /// The owner completed the onset source-attribution window.
    LatchComplete {
        /// Pulse bound selected by the owner.
        q: u8,
    },
    /// A new antipoison drink line proved that its effect ran.
    ProvenDrink {
        /// Tick that owns the server effect.
        origin: u16,
    },
    /// A count drop and matching consume line proved a strange-fruit effect.
    ProvenFruit {
        /// Tick that owns the server effect.
        origin: u16,
    },
    /// The owner observed a later game tick without a stronger poison event.
    Tick {
        /// Current session tick.
        tick: u16,
    },
    /// The owner observed zero hitpoints.
    DeathObserved,
    /// The owner observed the local player after respawn.
    RespawnObserved,
}

/// Reducer state and session-only evidence.
///
/// This value performs no observation, I/O, clock reads, or host operations.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PoisonMemory {
    /// The current reduced poison state.
    pub state: PoisonState,
    /// An unsupported mark invalidated poison inference for this session.
    pub session_invalid: bool,
    /// The owner observed death and awaits a respawn observation.
    pub death_seen: bool,
}

impl PoisonMemory {
    /// Create reducer state from a poison fact.
    pub const fn from_state(state: PoisonState) -> Self {
        Self {
            state,
            session_invalid: false,
            death_seen: false,
        }
    }

    /// Apply one explicit event and optional complete accepted-click evidence.
    ///
    /// Silence does not reduce a known poison reserve. An accepted-click window
    /// can clear only an `Unknown` state.
    pub fn transition(
        mut self,
        event: PoisonEvent,
        accepted_click: Option<AcceptedClickWindow>,
    ) -> Self {
        match event {
            PoisonEvent::None => {}
            PoisonEvent::Reconnect { tick } => {
                self.state = PoisonState::Unknown { since: tick };
                self.session_invalid = false;
                self.death_seen = false;
            }
            PoisonEvent::Hitmark {
                kind: 2,
                value,
                tick,
            } => {
                if !self.session_invalid {
                    self.state = PoisonState::Poisoned {
                        per_tick: value,
                        last_mark: tick,
                        hit_since_mark: false,
                    };
                }
            }
            PoisonEvent::Hitmark {
                kind: 0 | 1, tick, ..
            } if !self.session_invalid => match self.state {
                PoisonState::Poisoned {
                    per_tick,
                    last_mark,
                    ..
                } if tick_at_or_after(tick, last_mark) => {
                    self.state = PoisonState::Poisoned {
                        per_tick,
                        last_mark,
                        hit_since_mark: true,
                    };
                }
                PoisonState::Onset {
                    since, q, latched, ..
                } if tick_after(tick, since) => {
                    self.state = PoisonState::Onset {
                        since,
                        q,
                        hit_since_line: true,
                        latched,
                    };
                }
                _ => {}
            },
            PoisonEvent::Hitmark { kind: 0 | 1, .. } => {}
            PoisonEvent::Hitmark { tick, .. } => {
                self.state = PoisonState::Unknown { since: tick };
                self.session_invalid = true;
                self.death_seen = false;
            }
            PoisonEvent::PoisonChat { tick } if !self.session_invalid => {
                self.state = PoisonState::Onset {
                    since: tick,
                    q: 0,
                    hit_since_line: false,
                    latched: false,
                };
                self.death_seen = false;
            }
            PoisonEvent::PoisonChat { .. } => {}
            PoisonEvent::LatchComplete { q } if !self.session_invalid => {
                if let PoisonState::Onset {
                    since,
                    hit_since_line,
                    latched: false,
                    ..
                } = self.state
                {
                    self.state = PoisonState::Onset {
                        since,
                        q,
                        hit_since_line,
                        latched: true,
                    };
                }
            }
            PoisonEvent::LatchComplete { .. } => {}
            PoisonEvent::ProvenDrink { origin } if !self.session_invalid => {
                let existing = match self.state {
                    PoisonState::Immune { until_lower } => Some(until_lower),
                    _ => None,
                };
                self.state = PoisonState::Immune {
                    until_lower: ImmunityCredit::until_lower(120, origin, existing),
                };
                self.death_seen = false;
            }
            PoisonEvent::ProvenDrink { .. } => {}
            PoisonEvent::ProvenFruit { origin } if !self.session_invalid => {
                let existing = match self.state {
                    PoisonState::Immune { until_lower } => Some(until_lower),
                    _ => None,
                };
                self.state = PoisonState::Immune {
                    until_lower: ImmunityCredit::until_lower(0, origin, existing),
                };
                self.death_seen = false;
            }
            PoisonEvent::ProvenFruit { .. } => {}
            PoisonEvent::Tick { tick } if !self.session_invalid => {
                if let PoisonState::Immune { until_lower } = self.state {
                    if tick_after(tick, until_lower) {
                        self.state = PoisonState::Clear;
                    }
                }
            }
            PoisonEvent::Tick { .. } => {}
            PoisonEvent::DeathObserved => self.death_seen = true,
            PoisonEvent::RespawnObserved if self.death_seen && !self.session_invalid => {
                self.state = PoisonState::Clear;
                self.death_seen = false;
            }
            PoisonEvent::RespawnObserved => {}
        }

        if matches!(event, PoisonEvent::None)
            && !self.session_invalid
            && !self.death_seen
            && matches!(self.state, PoisonState::Unknown { .. })
            && accepted_click.is_some_and(|window| {
                let PoisonState::Unknown { since } = self.state else {
                    return false;
                };
                window.proves_timer_ran(since)
            })
        {
            self.state = PoisonState::Clear;
        }
        self
    }
}

/// Complete evidence for the accepted-click window that can clear `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcceptedClickWindow {
    /// Tick at the end of the observed window.
    pub t: u16,
    /// Tick at which the click was sent.
    pub click_sent_at: u16,
    /// The map aimed at the current tile at tick `t`.
    pub map_aim_at_t: bool,
    /// Tile progress occurred at tick `t`.
    pub tile_progress_at_t: bool,
    /// The main modal was clear at tick `t`.
    pub main_clear_t: bool,
    /// The main modal was clear at tick `t + 1`.
    pub main_clear_t_plus_1: bool,
    /// Chat was clear at tick `t`.
    pub chat_clear_t: bool,
    /// Chat was clear at tick `t + 1`.
    pub chat_clear_t_plus_1: bool,
    /// No held input existed at tick `t`.
    pub no_hold_t: bool,
    /// No held input existed at tick `t + 1`.
    pub no_hold_t_plus_1: bool,
    /// First snapshot tick in the observed sequence.
    pub snapshots_from: u16,
    /// Last snapshot tick in the observed sequence.
    pub snapshots_through: u16,
    /// Every expected snapshot was observed consecutively.
    pub snapshots_consecutive: bool,
    /// A poison mark was observed before the window ended.
    pub poison_mark_seen_through_end: bool,
}

impl AcceptedClickWindow {
    fn proves_timer_ran(self, since: u16) -> bool {
        let elapsed = self.t.wrapping_sub(since);
        let expected_from = self.t.wrapping_sub(1);
        let expected_through = self.t.wrapping_add(1).wrapping_add(LAG);
        (POISON_PERIOD..0x8000).contains(&elapsed)
            && self.click_sent_at == self.t.wrapping_sub(1)
            && self.map_aim_at_t
            && self.tile_progress_at_t
            && self.main_clear_t
            && self.main_clear_t_plus_1
            && self.chat_clear_t
            && self.chat_clear_t_plus_1
            && self.no_hold_t
            && self.no_hold_t_plus_1
            && self.snapshots_from == expected_from
            && self.snapshots_through == expected_through
            && self.snapshots_consecutive
            && !self.poison_mark_seen_through_end
    }
}

/// A lower-bound immunity interval earned by a proven cure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImmunityCredit;

impl ImmunityCredit {
    /// Return the later of the existing lower bound and `origin + credit`.
    pub const fn until_lower(credit: u16, origin: u16, existing: Option<u16>) -> u16 {
        let proposed = origin.wrapping_add(credit);
        match existing {
            Some(current) if tick_after(current, proposed) => current,
            _ => proposed,
        }
    }
}

/// The caller's current poison-exposure evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exposure {
    /// No threat row can currently poison the player.
    None,
    /// Every live row is a selected poison-capable melee or ranged NPC.
    Attributable {
        /// Greatest poison pulse among all live rows.
        max_pulse: u8,
    },
    /// The caller cannot prove the source set is complete and poison-safe.
    Unknown,
}

/// Conservative poison damage and timing outlook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoisonOutlook {
    /// Maximum remaining poison damage.
    pub total: u16,
    /// Maximum number of future poison fires, saturated at `u8::MAX`.
    pub fires: u8,
    /// Next expected poison-fire tick, for display only.
    pub next_fire_at: u16,
    /// Maximum damage pulse at the next fire.
    pub per_fire_now: u8,
    /// Whether the onset source-attribution window remains open.
    pub pending: bool,
}

/// Pure, saturating worst-case poison arithmetic.
pub mod poison_math {
    use super::{tick_before, Exposure, PoisonOutlook, PoisonState, POISON_PERIOD};

    /// Return the conservative remaining poison outlook.
    pub fn remaining(
        state: PoisonState,
        exposure: Exposure,
        now: u16,
        q_unknown: u8,
    ) -> PoisonOutlook {
        let empty = PoisonOutlook {
            total: 0,
            fires: 0,
            next_fire_at: now,
            per_fire_now: 0,
            pending: false,
        };
        match state {
            PoisonState::Unknown { .. } | PoisonState::Clear | PoisonState::Immune { .. } => empty,
            PoisonState::Poisoned {
                per_tick,
                last_mark,
                hit_since_mark,
            } => {
                let widened = hit_since_mark || exposure == Exposure::Unknown;
                let marked_total = post_mark_total(per_tick);
                let unknown_total = unknown_total(q_unknown);
                let total = if widened {
                    marked_total.max(unknown_total)
                } else {
                    marked_total
                };
                let marked_fires = post_mark_fires(per_tick);
                let unknown_fires = unknown_fires(q_unknown);
                let fires = if widened {
                    marked_fires.max(unknown_fires)
                } else {
                    marked_fires
                };
                let per_fire_now = if widened {
                    per_tick.max(q_unknown)
                } else {
                    per_tick
                };
                let next = last_mark.wrapping_add(POISON_PERIOD);
                PoisonOutlook {
                    total,
                    fires,
                    next_fire_at: if tick_before(now, next) { next } else { now },
                    per_fire_now,
                    pending: false,
                }
            }
            PoisonState::Onset {
                since,
                q,
                hit_since_line,
                latched,
            } => {
                let widened = hit_since_line || exposure == Exposure::Unknown || !latched;
                let q_effective = if widened { q_unknown } else { q };
                PoisonOutlook {
                    total: unknown_total(q_effective),
                    fires: unknown_fires(q_effective),
                    next_fire_at: since.wrapping_add(POISON_PERIOD),
                    per_fire_now: q_effective,
                    pending: !latched,
                }
            }
        }
    }

    fn post_mark_total(pulse: u8) -> u16 {
        let severity = u32::from(pulse) * 5;
        let damage = r0(severity.saturating_sub(1));
        saturating_u16(damage)
    }

    fn unknown_total(pulse: u8) -> u16 {
        saturating_u16(r0(u32::from(pulse) * 5))
    }

    fn post_mark_fires(pulse: u8) -> u8 {
        let fires = u16::from(pulse) * 5;
        saturating_u8(fires.saturating_sub(1))
    }

    fn unknown_fires(pulse: u8) -> u8 {
        saturating_u8(u16::from(pulse) * 5)
    }

    fn r0(severity: u32) -> u32 {
        let full = severity / 5;
        let remainder = severity % 5;
        5 * full * (full + 1) / 2 + remainder * (full + 1)
    }

    fn saturating_u16(value: u32) -> u16 {
        if value > u32::from(u16::MAX) {
            u16::MAX
        } else {
            value as u16
        }
    }

    fn saturating_u8(value: u16) -> u8 {
        if value > u16::from(u8::MAX) {
            u8::MAX
        } else {
            value as u8
        }
    }
    #[cfg(test)]
    mod tests {
        use super::super::{Exposure, PoisonEvent, PoisonMemory, PoisonState, POISON_PERIOD};
        use super::{r0, remaining};

        fn outlook(state: PoisonState, now: u16) -> super::super::PoisonOutlook {
            remaining(state, Exposure::None, now, 11)
        }

        fn onset(q: u8) -> PoisonMemory {
            PoisonMemory::from_state(PoisonState::Clear)
                .transition(PoisonEvent::PoisonChat { tick: 0 }, None)
                .transition(PoisonEvent::LatchComplete { q }, None)
        }

        fn hit(memory: PoisonMemory, pulse: u8, tick: u16) -> PoisonMemory {
            memory.transition(
                PoisonEvent::Hitmark {
                    kind: 2,
                    value: pulse,
                    tick,
                },
                None,
            )
        }

        #[test]
        fn poison_math_table_r1_r6_r11() {
            for (pulse, after_mark, total) in [(1, 4, 5), (6, 99, 105), (11, 319, 330)] {
                let state = PoisonState::Onset {
                    since: 0,
                    q: pulse,
                    hit_since_line: false,
                    latched: true,
                };
                let value = outlook(state, 0);
                assert_eq!(value.total, total);
                assert_eq!(value.fires, pulse * 5);
                assert_eq!(value.per_fire_now, pulse);
                assert!(!value.pending);
                let marked = PoisonState::Poisoned {
                    per_tick: pulse,
                    last_mark: 0,
                    hit_since_mark: false,
                };
                assert_eq!(outlook(marked, 0).total, after_mark);
            }
        }

        #[test]
        fn poison_math_r0_before_first_fire_is_total() {
            assert_eq!(r0(27), 87);
            assert_eq!(r0(26), 81);
            for severity in 1..=55 {
                assert_eq!(
                    r0(severity) - r0(severity - 1),
                    severity.div_ceil(5),
                    "severity {severity}"
                );
            }
        }

        #[test]
        fn poison_math_onset_unknown_source_is_rq_330() {
            assert_eq!(
                outlook(
                    PoisonState::Onset {
                        since: 0,
                        q: 0,
                        hit_since_line: false,
                        latched: false,
                    },
                    0,
                )
                .total,
                330
            );
        }

        #[test]
        fn poison_math_onset_latched_scorpion_is_rq_30() {
            let value = remaining(
                PoisonState::Onset {
                    since: 0,
                    q: 3,
                    hit_since_line: false,
                    latched: true,
                },
                Exposure::None,
                0,
                11,
            );
            assert_eq!(value.total, 30);
            assert_eq!(value.per_fire_now, 3);
            assert!(!value.pending);
        }

        #[test]
        fn poison_math_onset_pending_reads_q_unknown() {
            let value = remaining(
                PoisonState::Onset {
                    since: 0,
                    q: 3,
                    hit_since_line: false,
                    latched: false,
                },
                Exposure::None,
                0,
                11,
            );
            assert_eq!(value.total, 330);
            assert_eq!(value.per_fire_now, 11);
            assert!(value.pending);
        }

        #[test]
        fn poison_math_onset_latched_pulse_survives_weaker_current_set() {
            let value = remaining(
                PoisonState::Onset {
                    since: 0,
                    q: 11,
                    hit_since_line: false,
                    latched: true,
                },
                Exposure::Attributable { max_pulse: 3 },
                0,
                11,
            );
            assert_eq!(value.total, 330);
            assert_eq!(value.per_fire_now, 11);
        }

        #[test]
        fn poison_math_onset_consumer_unknown_exposure_widens() {
            let value = remaining(
                PoisonState::Onset {
                    since: 0,
                    q: 3,
                    hit_since_line: false,
                    latched: true,
                },
                Exposure::Unknown,
                0,
                11,
            );
            assert_eq!(value.total, 330);
            assert_eq!(value.per_fire_now, 11);
        }

        #[test]
        fn poison_math_onset_hit_since_line_uses_q_unknown() {
            let value = remaining(
                PoisonState::Onset {
                    since: 0,
                    q: 3,
                    hit_since_line: true,
                    latched: true,
                },
                Exposure::None,
                0,
                11,
            );
            assert_eq!(value.total, 330);
            assert_eq!(value.per_fire_now, 11);
        }

        #[test]
        fn reducer_smaller_mark_lowers_reserve() {
            let memory = hit(hit(onset(11), 6, 30), 5, 60);
            assert!(matches!(
                memory.state,
                PoisonState::Poisoned { per_tick: 5, .. }
            ));
            assert_eq!(outlook(memory.state, 60).total, 70);
        }

        #[test]
        fn reducer_mark_after_hit_sets_mark_value() {
            let memory = hit(
                onset(11).transition(
                    PoisonEvent::Hitmark {
                        kind: 1,
                        value: 0,
                        tick: 31,
                    },
                    None,
                ),
                11,
                60,
            );
            assert!(matches!(
                memory.state,
                PoisonState::Poisoned {
                    per_tick: 11,
                    hit_since_mark: false,
                    ..
                }
            ));
            assert_eq!(outlook(memory.state, 60).total, 319);
        }

        #[test]
        fn poison_math_zero_hitmark_since_mark_widens() {
            let memory = hit(onset(11), 6, 30).transition(
                PoisonEvent::Hitmark {
                    kind: 0,
                    value: 0,
                    tick: 31,
                },
                None,
            );
            assert_eq!(outlook(memory.state, 31).total, 330);
        }

        #[test]
        fn poison_math_retired_source_keeps_widened_bound() {
            let memory = hit(onset(11), 6, 30).transition(
                PoisonEvent::Hitmark {
                    kind: 1,
                    value: 0,
                    tick: 31,
                },
                None,
            );
            assert_eq!(outlook(memory.state, 32).total, 330);
        }

        #[test]
        fn poison_math_refresh_ignores_attributable_pulse() {
            let memory = hit(onset(11), 6, 30).transition(
                PoisonEvent::Hitmark {
                    kind: 1,
                    value: 0,
                    tick: 31,
                },
                None,
            );
            assert_eq!(
                remaining(
                    memory.state,
                    Exposure::Attributable { max_pulse: 3 },
                    31,
                    11,
                )
                .total,
                330
            );
        }

        #[test]
        fn poison_math_next_mark_clears_refresh() {
            let memory = hit(
                hit(onset(11), 6, 30).transition(
                    PoisonEvent::Hitmark {
                        kind: 1,
                        value: 0,
                        tick: 31,
                    },
                    None,
                ),
                6,
                60,
            );
            assert_eq!(outlook(memory.state, 60).total, 99);
        }

        #[test]
        fn poison_math_magic_exposure_widens_without_hit() {
            let state = hit(onset(11), 6, 30).state;
            assert_eq!(remaining(state, Exposure::Unknown, 31, 11).total, 330);
        }

        #[test]
        fn poison_math_attributable_exposure_without_hit_keeps_r_v() {
            let state = hit(onset(11), 6, 30).state;
            assert_eq!(
                remaining(state, Exposure::Attributable { max_pulse: 6 }, 31, 11,).total,
                99
            );
        }

        #[test]
        fn poison_math_delayed_ranged_impact_corrected_by_next_mark() {
            let initial = hit(onset(11), 6, 10);
            assert_eq!(outlook(initial.state, 11).total, 99);

            let after_impact = initial.transition(
                PoisonEvent::Hitmark {
                    kind: 1,
                    value: 0,
                    tick: 12,
                },
                None,
            );
            assert_eq!(outlook(after_impact.state, 12).total, 330);

            let corrected = hit(after_impact, 11, 42);
            assert_eq!(outlook(corrected.state, 42).total, 319);
        }

        #[test]
        fn poison_math_exact_fire_lemma() {
            for severity in 1u32..=55 {
                let pulse = severity.div_ceil(5);
                let bound = r0(5 * pulse - 1);
                let actual = r0(severity - 1);
                assert_eq!(bound - actual, (5 * pulse - severity) * pulse);
                assert_eq!(bound == actual, severity % 5 == 0);
            }
        }

        #[test]
        fn poison_math_false_park_ends_at_next_exact_fire() {
            let mut memory = onset(11);
            assert_eq!(outlook(memory.state, 0).total, 330);

            let mut severity = 27u32;
            let mut hp = 90i32;
            for fire in 1..=3u16 {
                let pulse = severity.div_ceil(5);
                hp -= pulse as i32;
                severity -= 1;
                memory = hit(memory, pulse as u8, fire * POISON_PERIOD);
                let reserve = i32::from(outlook(memory.state, fire * POISON_PERIOD).total);
                let actual = r0(severity) as i32;
                if fire < 3 {
                    assert!(hp <= reserve);
                } else {
                    assert_eq!(severity + 1, 25);
                    assert_eq!(reserve, actual);
                    assert!(hp > reserve);
                    assert!(hp > 1 + pulse as i32);
                }
            }
        }

        #[test]
        fn poison_math_false_park_with_food_ends_at_next_exact_fire() {
            let mut memory = onset(11);
            let mut severity = 22u32;
            let mut hp = 10i32;
            let mut food = 9i32;
            let mut previous_margin = None;
            let mut first_false = None;

            for fire in 1..=8u16 {
                let pulse = severity.div_ceil(5) as i32;
                hp -= pulse;
                severity -= 1;
                memory = hit(memory, pulse as u8, fire * POISON_PERIOD);
                let reserve = i32::from(outlook(memory.state, fire * POISON_PERIOD).total);
                let line = 1 + pulse;
                if hp <= line && food > 0 {
                    hp = (hp + 10).min(10);
                    food -= 1;
                }
                let usable = food * (10 - line).max(0);
                let actual = r0(severity) as i32;
                let margin = hp + usable - actual;
                assert!(previous_margin.is_none_or(|previous| margin >= previous));
                previous_margin = Some(margin);

                let parked = hp + usable <= reserve || hp <= line;
                if fire < 8 {
                    assert!(parked, "fire {fire}");
                } else {
                    assert_eq!(severity + 1, 15);
                    assert_eq!(reserve, actual);
                    assert!(!parked);
                }
                if hp + usable > actual && hp > line {
                    first_false.get_or_insert(fire);
                }
            }
            assert_eq!(first_false, Some(6));
        }

        #[test]
        fn poison_math_unknown_pulse_saturates() {
            let value = remaining(
                PoisonState::Onset {
                    since: 0,
                    q: u8::MAX,
                    hit_since_line: false,
                    latched: true,
                },
                Exposure::None,
                0,
                u8::MAX,
            );
            assert_eq!(value.total, u16::MAX);
            assert_eq!(value.fires, u8::MAX);
            assert_eq!(value.per_fire_now, u8::MAX);
        }
    }
}

const fn tick_after(tick: u16, boundary: u16) -> bool {
    let elapsed = tick.wrapping_sub(boundary);
    elapsed != 0 && elapsed < 0x8000
}

fn tick_at_or_after(tick: u16, boundary: u16) -> bool {
    let elapsed = tick.wrapping_sub(boundary);
    elapsed < 0x8000
}

fn tick_before(tick: u16, boundary: u16) -> bool {
    let elapsed = tick.wrapping_sub(boundary);
    elapsed >= 0x8000
}
