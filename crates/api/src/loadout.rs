//! Neutral operator loadout values (F0 item 7: contract §26 §1 item 7, D10,
//! `21-census-ownership-dag.md` §4.1).
//!
//! Frozen here, moved unchanged from `script::loadouts_store`
//! (`crates/script/src/loadouts_store.rs:59-222`):
//!
//! - the neutral values [`CarryEntry`] and [`Loadout`];
//! - their neutral worn-slot identifiers and validation: [`WORN_SLOTS`] and
//!   [`is_worn_slot`];
//! - their existing wire-compatible serde forms: the [`Loadout`] serialize
//!   and deserialize impls with legacy handling, and the lenient
//!   [`parse_loadout`] row parse.
//!
//! The wire shape is `{ name, worn: {slot: name}, carry: [{item, qty}],
//! unassigned? }`. Legacy `{ worn: [name], carry: [name] }` files are read
//! without guessing slots or dropping items.
//!
//! What stays in `script` (behaviour, not values): the [`LoadoutsStore`]
//! storage object and its file I/O, loadout selection and name resolution,
//! content-dependent equipment and supply planning, setting-derived option
//! helpers, unique-name and editing operations, and settings merge and
//! coercion (`21-census-ownership-dag.md` §4.1). The worn-slot visual layout
//! and labels (`WORN_SLOT_LAYOUT`, `worn_slot_label`) stay with the
//! presentation owners, not here: they are layout, not identity.
#![warn(missing_docs)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Loadout equipment slots, named as the content wearpos decoder names them.
pub const WORN_SLOTS: [&str; 11] = [
    "hat",
    "back",
    "front",
    "righthand",
    "torso",
    "lefthand",
    "legs",
    "hands",
    "feet",
    "ring",
    "quiver",
];

/// Whether `slot` is a neutral worn-slot identifier.
pub fn is_worn_slot(slot: &str) -> bool {
    WORN_SLOTS.contains(&slot)
}

/// One carried supply with a required positive quantity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CarryEntry {
    /// The carried item name.
    pub item: String,
    /// How many are carried, always at least one.
    pub qty: u32,
}

impl CarryEntry {
    /// A carry row; a zero quantity becomes one, so a row always carries.
    pub fn new(item: impl Into<String>, qty: u32) -> Self {
        Self {
            item: item.into(),
            qty: qty.max(1),
        }
    }
}

/// One named equipment/inventory preset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loadout {
    /// The preset name.
    pub name: String,
    /// Worn gear by worn-slot identifier.
    pub worn: BTreeMap<String, String>,
    /// Gear kept by the preset but in no worn slot.
    pub unassigned: Vec<String>,
    /// Carried supplies.
    pub carry: Vec<CarryEntry>,
}

impl Loadout {
    /// An empty preset called `name`.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            worn: BTreeMap::new(),
            unassigned: Vec::new(),
            carry: Vec::new(),
        }
    }

    /// Wear `item` in `slot`; an unknown slot keeps the item as unassigned
    /// gear instead of dropping it.
    pub fn with_slot(mut self, slot: &str, item: impl Into<String>) -> Self {
        if is_worn_slot(slot) {
            self.worn.insert(slot.to_string(), item.into());
        } else {
            self.unassigned.push(item.into());
        }
        self
    }

    /// Carry `qty` of `item` (at least one).
    pub fn with_carry(mut self, item: impl Into<String>, qty: u32) -> Self {
        self.carry.push(CarryEntry::new(item, qty));
        self
    }

    /// Set `slot` to `item`; an unknown slot is ignored, and an empty item
    /// clears the slot.
    pub fn set_slot(&mut self, slot: &str, item: Option<String>) {
        if !is_worn_slot(slot) {
            return;
        }
        match item {
            Some(name) if !name.trim().is_empty() => {
                self.worn.insert(slot.to_string(), name);
            }
            _ => {
                self.worn.remove(slot);
            }
        }
    }
}

impl Serialize for Loadout {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Wire<'a> {
            name: &'a str,
            worn: &'a BTreeMap<String, String>,
            carry: &'a [CarryEntry],
            #[serde(skip_serializing_if = "slice_empty")]
            unassigned: &'a [String],
        }
        Wire {
            name: &self.name,
            worn: &self.worn,
            carry: &self.carry,
            unassigned: &self.unassigned,
        }
        .serialize(serializer)
    }
}

fn slice_empty(value: &[String]) -> bool {
    value.is_empty()
}

impl<'de> Deserialize<'de> for Loadout {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        parse_loadout(&value).ok_or_else(|| serde::de::Error::custom("invalid loadout"))
    }
}

/// Parse one loadout row leniently: `None` when the row has no usable name,
/// otherwise the row with unknown slots kept as unassigned gear, string
/// carry rows at quantity one, and unusable carry rows skipped. Legacy
/// `{ worn: [...] }` arrays never invent slots.
pub fn parse_loadout(value: &serde_json::Value) -> Option<Loadout> {
    let name = value.get("name")?.as_str()?.trim();
    if name.is_empty() {
        return None;
    }
    let mut worn = BTreeMap::new();
    let mut unassigned = Vec::new();
    match value.get("worn") {
        Some(serde_json::Value::Array(rows)) => {
            for row in rows {
                if let Some(item) = row.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    unassigned.push(item.to_string());
                }
            }
        }
        Some(serde_json::Value::Object(map)) => {
            for (slot, raw) in map {
                let Some(item) = raw.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
                    continue;
                };
                if is_worn_slot(slot) {
                    worn.insert(slot.clone(), item.to_string());
                } else {
                    unassigned.push(item.to_string());
                }
            }
        }
        _ => {}
    }
    if let Some(serde_json::Value::Array(rows)) = value.get("unassigned") {
        for row in rows {
            if let Some(item) = row.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                unassigned.push(item.to_string());
            }
        }
    }
    let mut carry = Vec::new();
    if let Some(serde_json::Value::Array(rows)) = value.get("carry") {
        for row in rows {
            if let Some(item) = row.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                carry.push(CarryEntry::new(item, 1));
                continue;
            }
            let Some(item) = row
                .get("item")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            let qty = row
                .get("qty")
                .and_then(|v| {
                    v.as_u64()
                        .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
                        .or_else(|| v.as_f64().and_then(|n| (n >= 1.0).then_some(n as u64)))
                })
                .unwrap_or(1);
            carry.push(CarryEntry::new(item, qty.min(u32::MAX as u64) as u32));
        }
    }
    Some(Loadout {
        name: name.to_string(),
        worn,
        unassigned,
        carry,
    })
}

#[cfg(test)]
#[path = "loadout_tests.rs"]
mod tests;
