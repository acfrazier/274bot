//! Neutral operator loadout values (F0 item 7: contract §26 §1 item 7, D10,
//! `21-census-ownership-dag.md` §4.1).
//!
//! Frozen here, moved unchanged from `script::loadouts_store`
//! (`crates/script/src/loadouts_store.rs:59-222`):
//!
//! - the neutral values [`CarryEntry`] and [`Loadout`];
//! - their neutral worn-slot identifiers and validation: [`WORN_SLOTS`] and
//!   [`is_worn_slot`];
//! - their existing wire-compatible serde forms with the current wire shape;
//! - the shallow loadout-file probe [`loadouts_have_pre_0_2_8_shape`], the
//!   borrowed row predicate [`is_pre_0_2_8_loadout_shape`], and the lenient
//!   [`parse_loadout`] current-row parse.
//!
//! The wire shape is `{ name, worn: {slot: name}, carry: [{item, qty}],
//! unassigned? }`. Earlier `{ worn: [name], carry: [name] }` rows are not
//! decoded as current loadouts.
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
use std::fmt;

use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};
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

/// Whether this JSON row uses a loadout shape not written by 0.2.8:
/// array-valued `worn` or string-valued entries in `carry`. A persisted-file
/// reader uses this borrowed check to refuse the whole file rather than
/// dropping older data row by row.
pub fn is_pre_0_2_8_loadout_shape(value: &serde_json::Value) -> bool {
    matches!(value.get("worn"), Some(serde_json::Value::Array(_)))
        || value
            .get("carry")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|rows| rows.iter().any(serde_json::Value::is_string))
}

/// Parse one current-schema loadout row leniently: `None` when the row has
/// no usable name or uses an older array-worn/string-carry shape. Otherwise
/// unknown slots stay as unassigned gear, quantities retain their current
/// lenient decoding, and unusable rows are skipped.
pub fn parse_loadout(value: &serde_json::Value) -> Option<Loadout> {
    if is_pre_0_2_8_loadout_shape(value) {
        return None;
    }
    let name = value.get("name")?.as_str()?.trim();
    if name.is_empty() {
        return None;
    }
    let mut worn = BTreeMap::new();
    let mut unassigned = Vec::new();
    if let Some(serde_json::Value::Object(map)) = value.get("worn") {
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

#[derive(Clone, Copy)]
enum JsonKind {
    Array,
    String,
}

struct JsonKindVisitor(JsonKind);

impl<'de> Visitor<'de> for JsonKindVisitor {
    type Value = bool;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self::Value, E> {
        Ok(false)
    }

    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Self::Value, E> {
        Ok(false)
    }

    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self::Value, E> {
        Ok(false)
    }

    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self::Value, E> {
        Ok(false)
    }

    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Self::Value, E> {
        Ok(matches!(self.0, JsonKind::String))
    }

    fn visit_borrowed_str<E: serde::de::Error>(self, _: &'de str) -> Result<Self::Value, E> {
        Ok(matches!(self.0, JsonKind::String))
    }

    fn visit_string<E: serde::de::Error>(self, _: String) -> Result<Self::Value, E> {
        Ok(matches!(self.0, JsonKind::String))
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(false)
    }

    fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(false)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        while sequence.next_element::<IgnoredAny>()?.is_some() {}
        Ok(matches!(self.0, JsonKind::Array))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(false)
    }
}

struct WornShape(bool);

impl<'de> Deserialize<'de> for WornShape {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer
            .deserialize_any(JsonKindVisitor(JsonKind::Array))
            .map(Self)
    }
}

struct StringValue(bool);

impl<'de> Deserialize<'de> for StringValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer
            .deserialize_any(JsonKindVisitor(JsonKind::String))
            .map(Self)
    }
}

struct CarryShapeVisitor;

impl<'de> Visitor<'de> for CarryShapeVisitor {
    type Value = bool;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON carry value")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let mut found = false;
        while let Some(is_string) = sequence.next_element::<StringValue>()? {
            found |= is_string.0;
        }
        Ok(found)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(false)
    }

    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self::Value, E> {
        Ok(false)
    }

    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Self::Value, E> {
        Ok(false)
    }

    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self::Value, E> {
        Ok(false)
    }

    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self::Value, E> {
        Ok(false)
    }

    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Self::Value, E> {
        Ok(false)
    }

    fn visit_borrowed_str<E: serde::de::Error>(self, _: &'de str) -> Result<Self::Value, E> {
        Ok(false)
    }

    fn visit_string<E: serde::de::Error>(self, _: String) -> Result<Self::Value, E> {
        Ok(false)
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(false)
    }

    fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(false)
    }
}

struct CarryShape(bool);

impl<'de> Deserialize<'de> for CarryShape {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(CarryShapeVisitor).map(Self)
    }
}

#[derive(Deserialize)]
#[serde(field_identifier, rename_all = "lowercase")]
enum LoadoutField {
    Worn,
    Carry,
    #[serde(other)]
    Other,
}

#[derive(Default)]
struct LoadoutShape {
    worn_array: bool,
    string_carry: bool,
}

struct LoadoutShapeVisitor;

impl<'de> Visitor<'de> for LoadoutShapeVisitor {
    type Value = LoadoutShape;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a loadout JSON row")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut shape = LoadoutShape::default();
        while let Some(field) = map.next_key::<LoadoutField>()? {
            match field {
                LoadoutField::Worn => shape.worn_array = map.next_value::<WornShape>()?.0,
                LoadoutField::Carry => shape.string_carry = map.next_value::<CarryShape>()?.0,
                LoadoutField::Other => {
                    let _: IgnoredAny = map.next_value()?;
                }
            }
        }
        Ok(shape)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        while sequence.next_element::<IgnoredAny>()?.is_some() {}
        Ok(LoadoutShape::default())
    }

    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self::Value, E> {
        Ok(LoadoutShape::default())
    }

    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Self::Value, E> {
        Ok(LoadoutShape::default())
    }

    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self::Value, E> {
        Ok(LoadoutShape::default())
    }

    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self::Value, E> {
        Ok(LoadoutShape::default())
    }

    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Self::Value, E> {
        Ok(LoadoutShape::default())
    }

    fn visit_borrowed_str<E: serde::de::Error>(self, _: &'de str) -> Result<Self::Value, E> {
        Ok(LoadoutShape::default())
    }

    fn visit_string<E: serde::de::Error>(self, _: String) -> Result<Self::Value, E> {
        Ok(LoadoutShape::default())
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(LoadoutShape::default())
    }

    fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(LoadoutShape::default())
    }
}

impl<'de> Deserialize<'de> for LoadoutShape {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(LoadoutShapeVisitor)
    }
}

struct LoadoutFileShape(bool);

struct LoadoutFileShapeVisitor;

impl<'de> Visitor<'de> for LoadoutFileShapeVisitor {
    type Value = bool;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON array of loadout rows")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let mut found = false;
        while let Some(shape) = sequence.next_element::<LoadoutShape>()? {
            found |= shape.worn_array || shape.string_carry;
        }
        Ok(found)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(false)
    }

    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self::Value, E> {
        Ok(false)
    }

    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Self::Value, E> {
        Ok(false)
    }

    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self::Value, E> {
        Ok(false)
    }

    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self::Value, E> {
        Ok(false)
    }

    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Self::Value, E> {
        Ok(false)
    }

    fn visit_borrowed_str<E: serde::de::Error>(self, _: &'de str) -> Result<Self::Value, E> {
        Ok(false)
    }

    fn visit_string<E: serde::de::Error>(self, _: String) -> Result<Self::Value, E> {
        Ok(false)
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(false)
    }

    fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(false)
    }
}

impl<'de> Deserialize<'de> for LoadoutFileShape {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer
            .deserialize_any(LoadoutFileShapeVisitor)
            .map(Self)
    }
}

/// Whether a raw `loadouts.json` array contains a row whose shape predates
/// 0.2.8. This streaming check inspects only `worn` and `carry`, ignores all
/// other data without materializing a JSON tree, and returns false for
/// malformed input.
pub fn loadouts_have_pre_0_2_8_shape(bytes: &[u8]) -> bool {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let Ok(LoadoutFileShape(found)) = LoadoutFileShape::deserialize(&mut deserializer) else {
        return false;
    };
    found && deserializer.end().is_ok()
}

#[cfg(test)]
#[path = "loadout_tests.rs"]
mod tests;
