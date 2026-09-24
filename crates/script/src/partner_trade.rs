//! v1 `PartnerTrade.js` predicates and first-screen decisions: partner-name
//! matching, settings parsers and the receiver/giver offer-screen answer.
//! Trade HUD actions stay on the Trade family and `drive_partner_trade`.

/// Frozen `DEFAULT_TRADE_RANGE`.
pub const DEFAULT_TRADE_RANGE: i32 = 2;

/// Frozen `MULE_MODE_OPTIONS` settings labels.
pub const MULE_MODE_OPTIONS: [&str; 5] = ["Off", "Gatherer", "Mule", "Cooker", "Supplier"];

/// GatheringBot partner role (`MuleMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MuleMode {
    Off,
    Gatherer,
    Mule,
    Cooker,
    Supplier,
}

impl MuleMode {
    /// A `MuleMode` value as the frozen `===` checks see it; anything else is no role.
    pub fn from_mode(mode: &str) -> Self {
        match mode {
            "gatherer" => Self::Gatherer,
            "mule" => Self::Mule,
            "cooker" => Self::Cooker,
            "supplier" => Self::Supplier,
            _ => Self::Off,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Gatherer => "gatherer",
            Self::Mule => "mule",
            Self::Cooker => "cooker",
            Self::Supplier => "supplier",
        }
    }
}

/// Comma-separated settings string to trimmed, non-empty partner names.
pub fn parse_partner_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

/// Trimmed, case-insensitive name equality.
pub fn names_match(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// `name` is one of `partners` (trimmed, case-insensitive); no name never is.
pub fn is_configured_partner(name: Option<&str>, partners: &[String]) -> bool {
    let Some(name) = name.filter(|name| !name.trim().is_empty()) else {
        return false;
    };
    partners.iter().any(|partner| names_match(partner, name))
}

/// One offer slot: a name (or none) and its count.
pub struct OfferSlot {
    pub name: Option<String>,
    pub count: f64,
}

/// Sum of slot counts (at least 1 each) whose name equals `item_name`.
/// A NaN count poisons the sum, as JS `Math.max(1, NaN)` does.
pub fn count_offer_by_name(items: &[OfferSlot], item_name: &str) -> f64 {
    items
        .iter()
        .filter(|slot| names_match(slot.name.as_deref().unwrap_or(""), item_name))
        .map(|slot| {
            if slot.count.is_nan() {
                f64::NAN
            } else {
                slot.count.max(1.0)
            }
        })
        .sum()
}

/// Receiver side of the first trade screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiverOfferDecision {
    WaitHeader,
    Decline(String),
    WaitOffer,
    Accept,
}

pub fn decide_receiver_offer_screen(
    partner_header: Option<&str>,
    partners: &[String],
    my_offer_slots: f64,
    their_product_count: f64,
) -> ReceiverOfferDecision {
    let Some(header) = partner_header else {
        return ReceiverOfferDecision::WaitHeader;
    };
    if !is_configured_partner(Some(header), partners) {
        return ReceiverOfferDecision::Decline(format!("not a configured partner ({header})"));
    }
    if my_offer_slots > 0.0 {
        return ReceiverOfferDecision::Decline("safety: own offer not empty".to_string());
    }
    // `<= 0` is false for NaN in JS, so an unknown count falls through to accept.
    if their_product_count <= 0.0 {
        return ReceiverOfferDecision::WaitOffer;
    }
    ReceiverOfferDecision::Accept
}

/// Giver side of the first trade screen: offer the haul, then accept.
pub fn decide_giver_offer_screen(my_offer_slots: f64) -> &'static str {
    if my_offer_slots <= 0.0 {
        "offer"
    } else {
        "accept"
    }
}

/// Settings string to a role, trimmed and case-insensitive.
pub fn parse_mule_mode(raw: &str) -> MuleMode {
    MuleMode::from_mode(&raw.trim().to_lowercase())
}

/// Gatherer hands off instead of banking (power mode forces bank/drop).
pub fn mule_gatherer_handoff_active(mode: MuleMode, partners: usize, power_mode: bool) -> bool {
    mode == MuleMode::Gatherer && partners > 0 && !power_mode
}

/// Mule accepts a haul, then banks it.
pub fn mule_receiver_active(mode: MuleMode, partners: usize) -> bool {
    mode == MuleMode::Mule && partners > 0
}

/// Cooker accepts raw, cooks and banks the cooked.
pub fn mule_cooker_active(mode: MuleMode, partners: usize) -> bool {
    mode == MuleMode::Cooker && partners > 0
}

/// Supplier withdraws raw and trades at the meet.
pub fn mule_supplier_active(mode: MuleMode, partners: usize, power_mode: bool) -> bool {
    mode == MuleMode::Supplier && partners > 0 && !power_mode
}

/// Any non-gathering partner role.
pub fn mule_non_gatherer_active(mode: MuleMode, partners: usize) -> bool {
    mule_receiver_active(mode, partners)
        || mule_cooker_active(mode, partners)
        || (mode == MuleMode::Supplier && partners > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values are read off the frozen `PartnerTrade.ts`.

    fn list(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn partner_list_and_name_predicates() {
        assert_eq!(
            parse_partner_list(" Alice , ,bob,"),
            list(&["Alice", "bob"])
        );
        assert!(parse_partner_list("").is_empty());
        assert!(names_match(" Jive Maker", "jive maker "));
        assert!(!names_match("Jive", "Jive Maker"));
        let partners = list(&[" Alice ", "Bob"]);
        assert!(is_configured_partner(Some("alice"), &partners));
        assert!(!is_configured_partner(Some("  "), &partners));
        assert!(!is_configured_partner(None, &partners));
        assert!(!is_configured_partner(Some("Alice"), &[]));
    }

    #[test]
    fn offer_count_sums_at_least_one_per_matching_slot() {
        let slots = [
            OfferSlot {
                name: Some("Nature rune".into()),
                count: 25.0,
            },
            OfferSlot {
                name: Some(" nature RUNE ".into()),
                count: 0.0,
            },
            OfferSlot {
                name: None,
                count: 9.0,
            },
            OfferSlot {
                name: Some("Coins".into()),
                count: 5.0,
            },
        ];
        assert_eq!(count_offer_by_name(&slots, "Nature rune"), 26.0);
        assert_eq!(count_offer_by_name(&slots, "Flax"), 0.0);
    }

    #[test]
    fn receiver_screen_decisions_in_frozen_order() {
        let partners = list(&["Alice"]);
        assert_eq!(
            decide_receiver_offer_screen(None, &partners, 0.0, 5.0),
            ReceiverOfferDecision::WaitHeader
        );
        assert_eq!(
            decide_receiver_offer_screen(Some("Mallory"), &partners, 0.0, 5.0),
            ReceiverOfferDecision::Decline("not a configured partner (Mallory)".into())
        );
        assert_eq!(
            decide_receiver_offer_screen(Some("alice"), &partners, 1.0, 5.0),
            ReceiverOfferDecision::Decline("safety: own offer not empty".into())
        );
        assert_eq!(
            decide_receiver_offer_screen(Some("alice"), &partners, 0.0, 0.0),
            ReceiverOfferDecision::WaitOffer
        );
        assert_eq!(
            decide_receiver_offer_screen(Some("alice"), &partners, 0.0, 3.0),
            ReceiverOfferDecision::Accept
        );
        assert_eq!(decide_giver_offer_screen(0.0), "offer");
        assert_eq!(decide_giver_offer_screen(2.0), "accept");
    }

    #[test]
    fn mule_modes() {
        assert_eq!(parse_mule_mode(" Gatherer "), MuleMode::Gatherer);
        assert_eq!(parse_mule_mode("SUPPLIER"), MuleMode::Supplier);
        assert_eq!(parse_mule_mode("banker"), MuleMode::Off);
        assert!(mule_gatherer_handoff_active(MuleMode::Gatherer, 1, false));
        assert!(!mule_gatherer_handoff_active(MuleMode::Gatherer, 1, true));
        assert!(!mule_gatherer_handoff_active(MuleMode::Gatherer, 0, false));
        assert!(mule_non_gatherer_active(MuleMode::Supplier, 1));
        assert!(!mule_supplier_active(MuleMode::Supplier, 1, true));
        assert!(!mule_non_gatherer_active(MuleMode::Gatherer, 1));
        assert!(mule_non_gatherer_active(MuleMode::Cooker, 2));
    }
}
