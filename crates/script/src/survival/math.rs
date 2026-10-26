//! Pure poison-exposure classification over the caller's threat set and tables.

use api::condition::{poison_math, BotCondition, Exposure, PoisonState};

use crate::combat::request::ActorKind;
use crate::combat::tables::CombatTables;
use crate::combat::threats::{StyleObs, ThreatSet};

/// Classify whether the current live threat set can expose the player to poison.
///
/// The result is conservative. Any incomplete set, non-NPC row, unsupported
/// style, or missing poison fact returns [`Exposure::Unknown`].
pub fn exposure(threats: &ThreatSet, tables: &CombatTables, tick: u16) -> Exposure {
    if threats.has_overflow(tick) || threats.has_unattributed_event(tick) {
        return Exposure::Unknown;
    }

    let mut any = false;
    let mut max_pulse = 0;
    for threat in threats.iter(tick) {
        any = true;
        if threat.actor.kind != ActorKind::Npc
            || !matches!(threat.style, StyleObs::Melee | StyleObs::Ranged)
        {
            return Exposure::Unknown;
        }
        let Some(npc) = tables.npc(threat.ident) else {
            return Exposure::Unknown;
        };
        let Some(pulse) = crate::combat::risk::poison_pulse(npc.poison_severity) else {
            return Exposure::Unknown;
        };
        max_pulse = max_pulse.max(pulse);
    }

    if any {
        Exposure::Attributable { max_pulse }
    } else {
        Exposure::None
    }
}

/// The next poison fire's damage for this frame's condition fact.
///
/// An absent fact gives 0. The exposure comes from the caller's own threat
/// set at the same tick. The unknown-source bound comes from the caller's
/// selected tables. Callers add the result to their danger lines. One fire
/// can land on any accessible tick.
pub fn poison_fire(
    condition: Option<&BotCondition>,
    threats: &ThreatSet,
    tables: &CombatTables,
    tick: u16,
) -> i32 {
    let Some(condition) = condition else {
        return 0;
    };
    // `remaining` gives no fire for the other poison states, whatever the
    // exposure is. Return before the threat pass for them.
    if !matches!(
        condition.poison,
        PoisonState::Onset { .. } | PoisonState::Poisoned { .. }
    ) {
        return 0;
    }
    i32::from(
        poison_math::remaining(
            condition.poison,
            exposure(threats, tables, tick),
            tick,
            tables.unknown_poison_pulse(),
        )
        .per_fire_now,
    )
}
