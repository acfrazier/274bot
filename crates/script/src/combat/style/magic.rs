//! Cast mechanics only; upkeep and safety remain in the shared planner.
use crate::combat::arbiter;
use crate::combat::frame::Frame;
use crate::combat::request::{ActorKind, ActorRef, CombatRequest};
use crate::combat::tables::{CombatTab, CombatTables};
use api::game_data::{AutocastControls, SpellFact};
use api::snapshot::SnapshotView;

pub const CAST_TICKS: u16 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastMode {
    Autocast,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Runes,
    Spell,
}

/// Spell-specific refusal never masquerades as rune exhaustion.
pub fn refusal(line: &str, spell: &SpellFact) -> Option<Refusal> {
    if line.starts_with("You do not have enough ") && line.contains("Runes to cast this spell") {
        Some(Refusal::Runes)
    } else if line.contains("Your Magic level is not high enough for this spell")
        || line.contains("This spell only affects skeletons, zombies, ghosts and shades")
        || line.contains("You have no charges left on the staff")
        || line.contains("You need to be on a members' server to use this spell")
        || spell
            .worn_reqmessage
            .as_deref()
            .is_some_and(|message| line == message)
    {
        Some(Refusal::Spell)
    } else {
        None
    }
}

pub(crate) fn resolve(
    request: &CombatRequest,
    tables: &CombatTables,
    rhand: Option<i32>,
) -> CastMode {
    if request.spells.is_none()
        && rhand
            .and_then(|id| tables.weapon_style(id))
            .is_some_and(|row| row.tab == Some(CombatTab::Staff))
    {
        CastMode::Autocast
    } else {
        CastMode::Manual
    }
}

pub(crate) fn spell_index(tables: &CombatTables, alias: &str) -> Option<u8> {
    let alias = if alias.eq_ignore_ascii_case("ibans_blast") {
        "iban_blast"
    } else {
        alias.trim()
    };
    tables
        .selected()
        .spells()
        .iter()
        .position(|spell| {
            spell.source_row.eq_ignore_ascii_case(alias)
                || spell
                    .source_row
                    .strip_prefix("magic_spell_")
                    .is_some_and(|row| row.eq_ignore_ascii_case(alias))
                || spell
                    .spellcom
                    .split_once(':')
                    .is_some_and(|(_, com)| com.eq_ignore_ascii_case(alias))
                || spell.name.eq_ignore_ascii_case(alias)
        })
        .and_then(|index| u8::try_from(index).ok())
}

/// Whether the active, visible staff tab reports this spell as selected.
/// The widget text is set by the content's `set_autocast_spell` script.
pub(crate) fn observed_autocast_spell_matches(
    snapshot: SnapshotView<'_>,
    controls: &AutocastControls,
    active_side_tab: i32,
    combat_tab_root: i32,
    spell: &SpellFact,
) -> bool {
    if active_side_tab != 0 || combat_tab_root != controls.staff_tab_root {
        return false;
    }

    snapshot.side_tabs().is_some_and(|tabs| {
        tabs.value.iter().any(|tab| {
            tab.index == 0
                && tab.available
                && tab.active
                && tab.visible
                && tab.root_component_id == controls.staff_tab_root
                && tab.widgets.iter().any(|widget| {
                    widget.component_id == controls.spell_text_component
                        && widget.root_component_id == controls.staff_tab_root
                        && !widget.hidden
                        && widget.text.as_deref() == Some(spell.name.as_str())
                })
        })
    })
}

pub(crate) fn staff_provides(tables: &CombatTables, rhand: i32, rune: i32, members: bool) -> bool {
    (members
        || tables
            .selected()
            .item_by_id(rhand)
            .is_some_and(|item| !item.members))
        && tables
            .selected()
            .staves()
            .iter()
            .find(|row| row.id == rhand)
            .is_some_and(|row| row.runes.iter().any(|provided| provided.id == rune))
}

pub(crate) fn applicable(
    spell: &SpellFact,
    frame: &Frame<'_>,
    tables: &CombatTables,
    target: Option<ActorRef>,
) -> bool {
    if spell.spellcom == "magic:crumble_undead" {
        target.is_some_and(|actor| {
            actor.kind == ActorKind::Npc
                && frame
                    .npcs
                    .iter()
                    .find(|row| row.index == usize::from(actor.index))
                    .and_then(|row| row.r#type)
                    .and_then(|id| tables.npc(id as i32))
                    .is_some_and(|row| row.undead == Some(1))
        })
    } else {
        true
    }
}

pub(crate) fn eligible(
    spell: &SpellFact,
    frame: &Frame<'_>,
    tables: &CombatTables,
    target: Option<ActorRef>,
) -> bool {
    let rhand = frame
        .equipment
        .iter()
        .find(|row| row.slot == 3)
        .map(|row| row.def.id);
    (!spell.members || frame.world.members)
        && spell.level <= arbiter::stat(frame, 6).0
        && spell.component_id >= 0
        && spell.wornrequired.as_ref().is_none_or(|alias| {
            tables
                .selected()
                .item_by_alias(alias)
                .is_some_and(|item| Some(item.id) == rhand)
        })
        && applicable(spell, frame, tables, target)
}

pub(crate) fn castable(
    spell: &SpellFact,
    frame: &Frame<'_>,
    tables: &CombatTables,
    target: Option<ActorRef>,
) -> bool {
    let rhand = frame
        .equipment
        .iter()
        .find(|row| row.slot == 3)
        .map_or(-1, |row| row.def.id);
    eligible(spell, frame, tables, target)
        && spell.runes.iter().all(|rune| {
            staff_provides(tables, rhand, rune.id, frame.world.members)
                || arbiter::count(frame, rune.id) >= rune.count
        })
}

/// One consumed rune is enough to acknowledge a cast, including a splash.
pub(crate) fn rune_count(
    spell: &SpellFact,
    frame: &Frame<'_>,
    tables: &CombatTables,
) -> Option<(i32, i32)> {
    let rhand = frame
        .equipment
        .iter()
        .find(|row| row.slot == 3)
        .map_or(-1, |row| row.def.id);
    spell
        .runes
        .iter()
        .find(|rune| !staff_provides(tables, rhand, rune.id, frame.world.members))
        .map(|rune| (arbiter::count(frame, rune.id), rune.count.max(1)))
}

pub(crate) fn strongest(
    frame: &Frame<'_>,
    tables: &CombatTables,
    target: Option<ActorRef>,
    mode: CastMode,
    rejected: u32,
) -> Option<u8> {
    tables
        .selected()
        .spells()
        .iter()
        .enumerate()
        .filter(|(index, spell)| {
            *index < 32
                && rejected & (1 << index) == 0
                && (mode == CastMode::Manual || spell.autocast_selectable)
                && castable(spell, frame, tables, target)
        })
        .max_by_key(|(_, spell)| (spell.maxhit, spell.level))
        .map(|(index, _)| index as u8)
}
