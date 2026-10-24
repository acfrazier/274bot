//! Local selected-bank catalog queries. Reachable selection runs in the native
//! seat and delegates its route costs to the bounded host bank-pick service.
pub(crate) use crate::native_bank::bank_value;
use crate::native_bank::{air_distance_squared, nearest_eligible_bank};
use crate::observed;
use api::named_banks::{BankPreferences, NamedBank, NamedBankFacts};
use api::snapshot::WorldTile;
use serde::Deserialize;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::sync::Arc;

thread_local! {
    static BANKS: RefCell<Arc<NamedBankFacts>> = RefCell::new(Arc::new(NamedBankFacts::empty()));
    static PREFERENCES: Cell<BankPreferences> = Cell::default();
}

pub(crate) fn install(facts: Arc<NamedBankFacts>) {
    BANKS.with(|banks| *banks.borrow_mut() = facts);
}

pub(crate) fn settings(bag: &serde_json::Map<String, Value>) {
    PREFERENCES.set(BankPreferences {
        use_mage_bank: bag
            .get("useMageBank")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        use_zanaris_bank: bag
            .get("useZanarisBank")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    });
}

pub(crate) fn banks() -> Vec<NamedBank> {
    BANKS.with(|banks| banks.borrow().banks().to_vec())
}
/// Resolve one selected catalog bank without allocating the full bank list.
pub(crate) fn bank_tile(name: &str) -> Option<WorldTile> {
    BANKS.with(|banks| {
        banks
            .borrow()
            .banks()
            .iter()
            .find(|bank| bank.name == name)
            .map(|bank| bank.tile)
    })
}

fn eligible(bank: &NamedBank, preferences: BankPreferences) -> bool {
    observed::with(|scene| {
        let view = scene.since_login();
        bank.eligible(
            |id| (id == 10).then(|| view.stats().and_then(|skills| skills.fishing).map(|skill| skill.base)).flatten(),
            |name| matches!(view.quest_statuses(), Some(observed::QuestTab::Bound(rows))
                if rows.iter().any(|row| row.name.as_ref() == name && row.status.as_ref() == "complete")),
            preferences,
        )
    })
}

pub(crate) fn unlocked(name: &str, tile: WorldTile) -> bool {
    BANKS.with(|banks| {
        banks
            .borrow()
            .banks()
            .iter()
            .any(|bank| bank.name == name && bank.tile == tile && eligible(bank, PREFERENCES.get()))
    })
}

/// Frozen `bankUnlocked(bank)` (`api/bank/BankLocations.ts:145-164`) for a
/// roster bank, under the account facts and preferences Rust holds.
pub(crate) fn bank_unlocked(bank: &NamedBank) -> bool {
    eligible(bank, PREFERENCES.get())
}

/// Frozen `lockedBankAt`: the first same-plane bank within Chebyshev radius
/// 14 whose account requirements are not met.
pub(crate) fn locked_bank_at(tile: WorldTile) -> Value {
    BANKS.with(|banks| {
        banks
            .borrow()
            .banks()
            .iter()
            .find(|bank| {
                within_locked_bank_radius(bank.tile, tile) && !eligible(bank, PREFERENCES.get())
            })
            .map(bank_value)
            .unwrap_or(Value::Null)
    })
}

fn within_locked_bank_radius(bank: WorldTile, tile: WorldTile) -> bool {
    let dx = (i64::from(bank.x) - i64::from(tile.x)).abs();
    let dz = (i64::from(bank.z) - i64::from(tile.z)).abs();
    bank.level == tile.level && dx.max(dz) <= 14
}

pub(crate) fn selected_bank(index: i32) -> Option<NamedBank> {
    BANKS.with(|banks| {
        usize::try_from(index)
            .ok()
            .and_then(|index| banks.borrow().banks().get(index).copied())
    })
}

pub(crate) fn selected(index: i32) -> Value {
    selected_bank(index)
        .as_ref()
        .map(bank_value)
        .unwrap_or(Value::Null)
}

pub(crate) fn nearest_bank(from: WorldTile) -> Option<NamedBank> {
    BANKS.with(|banks| nearest_in(&banks.borrow(), from, PREFERENCES.get()))
}

fn nearest_in(
    facts: &NamedBankFacts,
    from: WorldTile,
    preferences: BankPreferences,
) -> Option<NamedBank> {
    nearest_eligible_bank(facts, from, |bank| eligible(bank, preferences))
}

pub(crate) fn nearest(from: WorldTile) -> Value {
    nearest_bank(from)
        .as_ref()
        .map(bank_value)
        .unwrap_or(Value::Null)
}

pub(crate) fn ranked(from: WorldTile) -> Vec<NamedBank> {
    let mut banks = banks();
    banks.retain(|bank| eligible(bank, PREFERENCES.get()));
    banks.sort_by_key(|bank| air_distance_squared(from, bank.air_tile()));
    banks
}

#[derive(Deserialize)]
pub(crate) struct FromTile {
    pub x: i32,
    pub z: i32,
    #[serde(default)]
    pub level: i32,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locked_bank_radius_matches_game_tile_distance() {
        let bank = WorldTile {
            x: 100,
            z: 200,
            level: 0,
        };
        assert!(within_locked_bank_radius(
            bank,
            WorldTile {
                x: 114,
                z: 186,
                level: 0,
            }
        ));
        assert!(!within_locked_bank_radius(
            bank,
            WorldTile {
                x: 115,
                z: 200,
                level: 0,
            }
        ));
        assert!(!within_locked_bank_radius(
            bank,
            WorldTile { level: 1, ..bank }
        ));
    }
}
