//! Single descriptor authority for compiled Browse, preparation and Start.
use crate::native::CompiledCard;

/// A picker id: the exact string the picker shows and Start keys on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompiledId(pub &'static str);

static CARDS: &[CompiledCard] = &[
    crate::gatherer::CARD,
    crate::quester::CARD,
    #[cfg(feature = "load")]
    crate::sherlock::CARD,
];

/// Whale names: real 274 scripts out of scope this campaign. Recognized so
/// the panel can explain why a name is not listed.
const WHALE_IDS: &[&str] = &[
    "GatheringBot",
    "Woodcutter",
    "AIOQuester",
    "AIOTeleport",
    "ClueSolver",
    "AutoFighter",
    "GreenDragon",
    "FireGiant",
    "MossGiant",
    "ChaosDruidKiller",
    "RockCrab",
    "HillGiant",
    "ChickenKiller",
    "CowKiller",
    "BrimhavenAgility",
    "WildyAgility",
    "DuelArena",
    "NatureCrafter",
    "RuneCrafter",
    "MuleCrafter",
    "FlaxRunner",
    "ShopRunner",
];

/// Static metadata exists independently of any account or active run.
pub fn compiled_cards() -> &'static [CompiledCard] {
    CARDS
}

pub fn compiled_card(id: CompiledId) -> Option<&'static CompiledCard> {
    CARDS.iter().find(|card| card.id == id)
}

pub fn compiled_id(name: &str) -> Option<CompiledId> {
    CARDS
        .iter()
        .find(|card| card.id.0 == name)
        .map(|card| card.id)
}

/// True when `name` is a known whale script (listed in the 274 client but
/// out of scope for this host).
pub fn is_whale(name: &str) -> bool {
    WHALE_IDS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whale_gate_keeps_listed_whales_out_and_real_cards_in() {
        for whale in ["Woodcutter", "RockCrab", "NatureCrafter", "ShopRunner"] {
            assert!(is_whale(whale), "{whale} must stay whale-gated");
        }
        for listed in [
            crate::gatherer::CARD.id.0,
            crate::quester::CARD.id.0,
            "BoneBurier",
            "",
        ] {
            assert!(!is_whale(listed), "{listed} must not be whale-gated");
        }
    }
}
