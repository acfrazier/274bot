//! Per-account quest decisions. These are runtime input, not Path facts and
//! not part of the process-wide compiled-Path cache.
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrestGauntlets {
    #[default]
    Chaos,
    Cooking,
    Goldsmith,
}

/// One allocation-free home for account-specific quest choices. Handlers
/// borrow it from `StepContext`; a cached plan must not capture these values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QuestChoices {
    pub crest_gauntlets: CrestGauntlets,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gauntlet_choice_is_explicit_and_never_random() {
        assert_eq!(
            QuestChoices::default().crest_gauntlets,
            CrestGauntlets::Chaos
        );
        for (text, choice) in [
            ("chaos", CrestGauntlets::Chaos),
            ("cooking", CrestGauntlets::Cooking),
            ("goldsmith", CrestGauntlets::Goldsmith),
        ] {
            assert_eq!(
                serde_json::from_value::<CrestGauntlets>(serde_json::json!(text)).unwrap(),
                choice
            );
        }
        for text in ["random", "", "gold", "unknown"] {
            assert!(serde_json::from_value::<CrestGauntlets>(serde_json::json!(text)).is_err());
        }
    }
}
