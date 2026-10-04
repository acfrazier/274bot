//! Cast mechanics only; upkeep and safety remain in the shared planner.
use crate::combat::arbiter;
use crate::combat::frame::Frame;
use crate::combat::request::{ActorKind, ActorRef, CombatRequest};
use crate::combat::tables::{CombatTab, CombatTables};
use api::game_data::SpellFact;
use api::line_of_sight::{has_line_of_sight_local, Footprint};
use api::snapshot::SceneView;

/// Both the server autocast attack range and targeted manual AP range.
pub const CAST_RANGE: i32 = 10;
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

/// Borrowed collision ray, with the actor's full network footprint. Missing
/// collision is unknown, never clear LOS. Open actor ops may approach on the
/// server; safespot consumers use this gate before sending a cast.
pub fn in_reach(
    here: api::WorldTile,
    target: api::WorldTile,
    size: i32,
    scene: Option<&SceneView>,
) -> Option<bool> {
    if here.level != target.level {
        return Some(false);
    }
    let size = size.max(1);
    let dx = (target.x - here.x).max(here.x - target.x - size + 1).max(0);
    let dz = (target.z - here.z).max(here.z - target.z - size + 1).max(0);
    if dx.max(dz) > CAST_RANGE {
        return Some(false);
    }
    let scene = scene.filter(|scene| scene.available && scene.level == here.level)?;
    let lookup = |x: i32, z: i32| {
        if x < 0 || z < 0 || x >= scene.width || z >= scene.height {
            return None;
        }
        scene
            .collision_flags
            .get((x * scene.height + z) as usize)
            .copied()
    };
    Some(has_line_of_sight_local(
        &lookup,
        Footprint {
            lx: here.x - scene.base_x,
            lz: here.z - scene.base_z,
            size: 1,
        },
        Footprint {
            lx: target.x - scene.base_x,
            lz: target.z - scene.base_z,
            size,
        },
    ))
}
