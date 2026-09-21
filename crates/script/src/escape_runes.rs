//! Static escape teleport rune facts from selected cache rows (not cast eligibility).

use api::game_data::{SelectedGameData, TeleportSpell};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EscapeRunesFact {
    pub runes: Vec<RuneCost>,
    pub level: i32,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuneCost {
    pub rune: String,
    pub count: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeRunesError {
    MissingSelected,
    UnknownId,
}

/// Resolve by exact `id` → `source_row == magic_spell_teleport_{id}`.
pub fn escape_runes_for(data: &SelectedGameData, id: &str) -> Result<EscapeRunesFact, EscapeRunesError> {
    let wanted = format!("magic_spell_teleport_{id}");
    let Some(spell) = data
        .teleports()
        .iter()
        .find(|row| row.source_row == wanted)
    else {
        return Err(EscapeRunesError::UnknownId);
    };
    Ok(fact_from_spell(spell))
}

pub fn escape_runes_for_optional(
    data: Option<&SelectedGameData>,
    id: &str,
) -> Result<EscapeRunesFact, EscapeRunesError> {
    let Some(data) = data else {
        return Err(EscapeRunesError::MissingSelected);
    };
    escape_runes_for(data, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::io::ClientRevision;

    #[test]
    fn seven_escape_ids_match_both_revisions() {
        for revision in [ClientRevision::R274, ClientRevision::R289] {
            let data = api::game_data::for_revision(revision).expect("selected game data");
            for id in [
                "varrock",
                "lumbridge",
                "falador",
                "camelot",
                "ardougne",
                "watchtower",
                "trollheim",
            ] {
                let fact = escape_runes_for(&data, id).unwrap_or_else(|_| panic!("{id}"));
                assert!(!fact.runes.is_empty(), "{id}");
                assert!(fact.level > 0, "{id}");
                assert!(fact.label.ends_with(" teleport"), "{id}");
            }
            assert_eq!(
                escape_runes_for(&data, "Varrock"),
                Err(EscapeRunesError::UnknownId)
            );
        }
    }
}

fn fact_from_spell(spell: &TeleportSpell) -> EscapeRunesFact {
    EscapeRunesFact {
        runes: spell
            .runes
            .iter()
            .map(|rune| RuneCost {
                rune: rune.name.clone(),
                count: rune.count,
            })
            .collect(),
        level: spell.level,
        label: format!("{} teleport", spell.name),
    }
}
