use std::collections::HashMap;
#[cfg(feature = "load")]
use std::path::Path;

use super::{
    find_matching_bracket, lookup_source, parse_string_array, quoted_after, scan_quoted,
    ItemOptionCandidate, ItemOptionSpec, SettingDef,
};

/// File-Load SETTINGS: static `export const SETTINGS = { … }` parse.
/// No V8. Identifier-valued fields that the scanner cannot inline stay
/// empty rather than aborting the walk; an unresolvable object spread inside
/// a setting definition fails closed.
pub fn settings_schema_from_source(src: &str) -> Vec<SettingDef> {
    try_settings_schema_from_source(src).unwrap_or_default()
}

/// Fallible static `export const SETTINGS = { … }` parse. Refuses any setting
/// definition whose object spread cannot be resolved.
pub fn try_settings_schema_from_source(src: &str) -> Result<Vec<SettingDef>, String> {
    parse_settings_export(src, "SETTINGS", None)
}

/// Parse `export const NAME = { … }` into setting definitions.
/// Catalog golds type the export (`export const SETTINGS: SettingsSchema = {`).
pub(super) fn parse_settings_export(
    file_src: &str,
    export_name: &str,
    sources: Option<&HashMap<String, String>>,
) -> Result<Vec<SettingDef>, String> {
    let Some(obj) = settings_export_object(file_src, export_name) else {
        return Ok(Vec::new());
    };
    parse_settings_object(obj, file_src, sources)
}

fn setting_object_body(file_src: &str, export_name: &str) -> Option<String> {
    settings_export_object(file_src, export_name)
        .or_else(|| {
            let rhs = const_eq_rhs(file_src, export_name)?.trim_start();
            if !rhs.starts_with('{') {
                return None;
            }
            let obj_end = find_matching_bracket(rhs, '{', '}')?;
            Some(&rhs[..=obj_end])
        })
        .map(str::to_string)
}

/// Body of `export const NAME[: Type] = { … }`, including the braces.
fn settings_export_object<'a>(file_src: &'a str, export_name: &str) -> Option<&'a str> {
    let needle = format!("export const {export_name}");
    let idx = file_src.find(&needle)?;
    let mut after = file_src[idx + needle.len()..].trim_start();
    if after.starts_with(':') {
        after = skip_ts_type_to_eq(after)?;
    }
    let after = after.strip_prefix('=')?.trim_start();
    let obj_end = find_matching_bracket(after, '{', '}')?;
    Some(&after[..=obj_end])
}

/// `s` starts with `:`. Return the suffix at the assignment `=`, or `None`.
fn skip_ts_type_to_eq(s: &str) -> Option<&str> {
    let mut angle = 0i32;
    let mut paren = 0i32;
    let mut square = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '<' => angle += 1,
            '>' => angle = angle.saturating_sub(1),
            '(' => paren += 1,
            ')' => paren = paren.saturating_sub(1),
            '[' => square += 1,
            ']' => square = square.saturating_sub(1),
            '=' if i > 0 && angle == 0 && paren == 0 && square == 0 => {
                return Some(&s[i..]);
            }
            _ => {}
        }
    }
    None
}

fn take_ident(s: &str) -> Option<(&str, &str)> {
    let n = s
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
        .count();
    if n == 0 {
        None
    } else {
        Some((&s[..n], &s[n..]))
    }
}

/// Name-map only: inlined `export const` in the same file, or the host's
/// `LOADOUT_SETTING` shim. Do not evaluate TypeScript identifiers.
fn resolve_setting_ident(file_src: &str, ident: &str) -> Option<String> {
    if ident == "LOADOUT_SETTING" {
        return setting_object_body(
            include_str!("../shim/loadout_setting.js"),
            "LOADOUT_SETTING",
        );
    }
    if ident == "PERIODIC_BANK_SETTINGS" {
        return setting_object_body(include_str!("../shim/banking.js"), "PERIODIC_BANK_SETTINGS");
    }
    // Frozen `src/bot/api/combat/rangedSettings.ts:3-6`: the imported
    // static settings spread is shared with the shim's name map.
    if ident == "CUSTOM_RANGED_SETTINGS" {
        return setting_object_body(
            include_str!("../shim/ranged_settings.js"),
            "CUSTOM_RANGED_SETTINGS",
        );
    }
    setting_object_body(file_src, ident)
}

/// Frozen-source equipment / drop tables that W1 must publish. The host
/// must not invent names here; empty options are the honest schema.
pub(crate) fn is_revision_fact_option_ident(ident: &str) -> bool {
    matches!(
        ident,
        "STAFFS"
            | "BOWS"
            | "CROSSBOWS"
            | "DARTS"
            | "ARROWS"
            | "BOLTS"
            | "AXES"
            | "MELEE_WEAPONS"
            | "ROCK_CRAB_RANGED_WEAPONS"
            | "FOOD_OPTIONS"
            | "DROP_DB"
    )
}

/// W1c `equipment_names` families backing a frozen settings `options` /
/// `optionsFrom` ident. `RANGED_WEAPONS` and `ROCK_CRAB_RANGED_WEAPONS` mirror
/// `.superpowers/release-0.1.9/reference/rs2b0t-00d39a17e0/src/bot/api/combat/ranged.ts`
/// (`[...BOWS, ...DARTS]`). `AXES` and `DROP_DB` stay outside W1c.
pub(crate) fn w1c_equipment_option_families(ident: &str) -> Option<&'static [&'static str]> {
    Some(match ident {
        "STAFFS" => &["staffs"],
        "BOWS" => &["bows"],
        "CROSSBOWS" => &["crossbows"],
        "DARTS" => &["darts"],
        "ARROWS" => &["arrows"],
        "BOLTS" => &["bolts"],
        "MELEE_WEAPONS" => &["melee_weapons"],
        "RANGED_WEAPONS" | "ROCK_CRAB_RANGED_WEAPONS" => &["bows", "darts"],
        _ => return None,
    })
}

/// Host-owned finite option tables for imported / parent-dir identifiers
/// whose bodies are not in the same-directory settings blob. Values are
/// copied from the frozen rs2b0t pin; this is not a JS evaluator.
///
/// `BANK_LOCATIONS` / `BANK_LOCATION_OPTIONS` are the exception: a bank name
/// is drawn from the same frozen roster as runtime `BANK_LOCATIONS`, including
/// gated entries (eligibility is evaluated when selecting a destination).
pub(crate) fn catalog_option_table(ident: &str) -> Option<&'static [&'static str]> {
    Some(match ident {
        "COOK_LOCATION_OPTIONS" => &[
            "Auto",
            "Varrock East",
            "Varrock West",
            "Al Kharid",
            "Draynor",
            "Falador East",
            "Falador West",
            "Edgeville",
            "Seers",
            "Catherby",
            "Yanille",
            "Ardougne West",
            "Ardougne East",
            "Canifis",
            "Shilo Village",
            "Fishing Guild",
            "Shantay Pass",
            "Mage Arena",
            "Grand Tree",
            "Duel Arena",
            "Custom",
        ],
        "LOG_LEVELS" => &[
            "Logs",
            "Oak logs",
            "Willow logs",
            "Maple logs",
            "Yew logs",
            "Magic logs",
        ],
        "FIRE_SPOTS" => &["Varrock East", "Varrock West", "Draynor", "Seers"],
        "HERB_OPTIONS" => &[
            "Guam leaf",
            "Marrentill",
            "Tarromin",
            "Harralander",
            "Ranarr weed",
            "Toadflax",
            "Irit leaf",
            "Avantoe",
            "Kwuarm",
            "Snapdragon",
            "Cadantine",
            "Lantadyme",
            "Dwarf weed",
            "Torstol",
            "Snake weed",
            "Ardrigal",
            "Sito foil",
            "Volencia moss",
            "Rogues purse",
        ],
        "RUNE_OPTIONS" => &[
            "Air rune",
            "Mind rune",
            "Water rune",
            "Earth rune",
            "Fire rune",
            "Body rune",
            "Cosmic rune",
            "Chaos rune",
            "Nature rune",
            "Law rune",
            "Death rune",
        ],
        "BANK_LOCATIONS" | "BANK_LOCATION_OPTIONS" => bank_location_names(ident),
        "PRODUCT_OPTIONS" => &[
            "Gold ring",
            "Sapphire ring",
            "Emerald ring",
            "Ruby ring",
            "Diamond ring",
            "Dragonstone ring",
            "Gold necklace",
            "Sapphire necklace",
            "Emerald necklace",
            "Ruby necklace",
            "Diamond necklace",
            "Dragonstone necklace",
            "Gold amulet",
            "Sapphire amulet",
            "Emerald amulet",
            "Ruby amulet",
            "Diamond amulet",
            "Dragonstone amulet",
        ],
        "JEWEL_OPTIONS" => &[
            "Sapphire ring",
            "Sapphire necklace",
            "Sapphire amulet",
            "Emerald ring",
            "Emerald amulet",
            "Ruby ring",
            "Ruby amulet",
            "Diamond ring",
            "Diamond amulet",
            "Dragonstone ring",
            "Dragonstone amulet",
        ],
        _ => return None,
    })
}

fn bank_location_names(ident: &str) -> &'static [&'static str] {
    /// The complete frozen roster, in the runtime payload's stable order.
    const BANKS: [&str; api::named_banks::BANK_CATALOG.len()] = {
        let mut names = [""; api::named_banks::BANK_CATALOG.len()];
        let mut i = 0;
        while i < names.len() {
            names[i] = api::named_banks::BANK_CATALOG[i].name;
            i += 1;
        }
        names
    };
    const WITH_NEAREST: [&str; api::named_banks::BANK_CATALOG.len() + 1] = {
        let mut names = ["Nearest"; api::named_banks::BANK_CATALOG.len() + 1];
        let mut i = 0;
        while i < api::named_banks::BANK_CATALOG.len() {
            names[i + 1] = api::named_banks::BANK_CATALOG[i].name;
            i += 1;
        }
        names
    };
    if ident == "BANK_LOCATION_OPTIONS" {
        &WITH_NEAREST
    } else {
        &BANKS
    }
}

pub(super) fn catalog_option_values(ident: &str) -> Option<Vec<String>> {
    catalog_option_table(ident).map(|rows| rows.iter().map(|s| (*s).to_string()).collect())
}

/// Static `const NAME = ['a', 'b']` (or a same-file / shim alias of one).
/// Does not evaluate `Object.keys(...)` or other TypeScript.
fn resolve_string_array_ident(file_src: &str, ident: &str) -> Option<Vec<String>> {
    resolve_string_array_ident_visited(file_src, ident, &mut Vec::new())
}

fn resolve_string_array_ident_visited(
    file_src: &str,
    ident: &str,
    stack: &mut Vec<String>,
) -> Option<Vec<String>> {
    if stack.iter().any(|s| s == ident) {
        return None;
    }
    stack.push(ident.to_string());
    if let Some(arr) = string_array_literal_in(file_src, ident) {
        return Some(arr);
    }
    if let Some(alias) = string_array_alias_in(file_src, ident) {
        return resolve_string_array_ident_visited(file_src, &alias, stack);
    }
    for shim in [
        include_str!("../shim/banking.js"),
        include_str!("../shim/combat_style.js"),
        include_str!("../shim/data/pickpocket_targets.js"),
        include_str!("../shim/food.js"),
        include_str!("../shim/steal_rules.js"),
    ] {
        if let Some(arr) = string_array_literal_in(shim, ident) {
            return Some(arr);
        }
        if let Some(alias) = string_array_alias_in(shim, ident) {
            return resolve_string_array_ident_visited(shim, &alias, stack);
        }
        if let Some(arr) = const_eq_rhs(shim, ident)
            .and_then(|rhs| parse_mapped_or_keys_rhs(rhs.trim_start(), shim))
        {
            return Some(arr);
        }
    }
    if let Some(rhs) = const_eq_rhs(file_src, ident) {
        let trimmed = rhs.trim_start();
        if let Some(arr) = parse_mapped_or_keys_rhs(trimmed, file_src) {
            return Some(arr);
        }
        if let Some(labels) = object_array_field_values(trimmed, "label", file_src) {
            return Some(labels);
        }
    }
    catalog_option_values(ident)
}

fn parse_mapped_or_keys_rhs(rhs: &str, file_src: &str) -> Option<Vec<String>> {
    if let Some(keys) = object_keys_call(rhs, file_src) {
        return Some(keys);
    }
    if let Some((base, after)) = take_ident(rhs) {
        if after.trim_start().starts_with(".map") {
            if let Some((vals, _)) = take_map_call(base, after.trim_start(), file_src) {
                return Some(vals);
            }
        }
    }
    None
}

fn const_eq_rhs<'a>(src: &'a str, ident: &str) -> Option<&'a str> {
    let needles = [format!("export const {ident}"), format!("const {ident}")];
    for needle in needles {
        let mut search = src;
        while let Some(idx) = search.find(&needle) {
            let abs = src.len() - search.len() + idx;
            let after = &search[idx + needle.len()..];
            let cont = after
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
            if cont || !const_decl_is_live(src, abs) {
                search = &search[idx + 1..];
                continue;
            }
            let mut rest = after.trim_start();
            if rest.starts_with(':') {
                rest = skip_ts_type_to_eq(rest)?;
            }
            let rest = rest.strip_prefix('=')?.trim_start();
            return Some(rest);
        }
    }
    None
}

/// `const` / `export const` at `idx` is a live declaration, not a comment.
fn const_decl_is_live(src: &str, idx: usize) -> bool {
    let line_start = src[..idx].rfind('\n').map(|i| i + 1).unwrap_or(0);
    if !src[line_start..idx].trim().is_empty() {
        return false;
    }
    !in_unclosed_block_comment(&src[..idx])
}

fn in_unclosed_block_comment(before: &str) -> bool {
    let bytes = before.as_bytes();
    let mut i = 0;
    let mut in_block = false;
    let mut in_line = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_line {
            if b == b'\n' {
                in_line = false;
            }
            i += 1;
            continue;
        }
        if in_block {
            if b == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                in_block = false;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            in_line = true;
            i += 2;
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            in_block = true;
            i += 2;
            continue;
        }
        i += 1;
    }
    in_block
}

fn string_array_literal_in(src: &str, ident: &str) -> Option<Vec<String>> {
    let rhs = const_eq_rhs(src, ident)?;
    parse_string_array_with_consts(rhs, src)
}

/// Parse a string array whose elements are literals, string consts, or a
/// recognized finite `...IDENT` / `...IDENT.map(p => p.field)` spread.
/// `.map(... .key)` stays unresolved so high-alchemy `item_option_spec`
/// keeps ownership. Anything else dynamic remains unresolved.
fn parse_string_array_with_consts(rhs: &str, file_src: &str) -> Option<Vec<String>> {
    let s = rhs.trim_start();
    if !s.starts_with('[') {
        return None;
    }
    let end = find_matching_bracket(s, '[', ']')?;
    let mut rest = s[1..end].trim_start();
    let mut out = Vec::new();
    while !rest.is_empty() {
        if rest.starts_with("...") {
            rest = rest[3..].trim_start();
            let (ident, after) = take_ident(rest)?;
            rest = after.trim_start();
            if rest.starts_with(".map") {
                let (vals, after_map) = take_map_call(ident, rest, file_src)?;
                out.extend(vals);
                rest = after_map.trim_start();
            } else {
                out.extend(resolve_string_array_ident(file_src, ident)?);
            }
        } else if let Some((value, n)) = scan_quoted(rest) {
            out.push(value);
            rest = rest[n..].trim_start();
        } else if let Some((ident, after)) = take_ident(rest) {
            rest = after.trim_start();
            if rest.starts_with(".map") {
                let (vals, after_map) = take_map_call(ident, rest, file_src)?;
                out.extend(vals);
                rest = after_map.trim_start();
            } else if let Some(after_dot) = rest.strip_prefix('.') {
                let after_dot = after_dot.trim_start();
                let (field, after_field) = take_ident(after_dot)?;
                out.push(quoted_object_const_field(file_src, ident, field)?);
                rest = after_field.trim_start();
            } else {
                out.push(resolve_string_const_ident(file_src, ident)?);
            }
        } else {
            return None;
        }
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
        } else if !rest.is_empty() {
            return None;
        }
    }
    Some(out)
}

/// `IDENT.map(param => param.field)` of a same-file object-literal array.
/// Field `key` is reserved for the high-alchemy item-option walker.
fn take_map_call<'a>(
    ident: &str,
    after_ident: &'a str,
    file_src: &str,
) -> Option<(Vec<String>, &'a str)> {
    let rest = after_ident.trim_start().strip_prefix('.')?.trim_start();
    let rest = rest.strip_prefix("map")?.trim_start();
    let rest = rest.strip_prefix('(')?.trim_start();
    let rest = rest.strip_prefix('(').unwrap_or(rest).trim_start();
    let (param, after_param) = take_ident(rest)?;
    let rest = after_param.trim_start();
    let rest = rest.strip_prefix(')').unwrap_or(rest).trim_start();
    let rest = rest.strip_prefix("=>")?.trim_start();
    let (recv, after_recv) = take_ident(rest)?;
    if recv != param {
        return None;
    }
    let rest = after_recv.trim_start().strip_prefix('.')?.trim_start();
    let (field, after_field) = take_ident(rest)?;
    if field == "key" {
        return None;
    }
    let rest = after_field.trim_start();
    let rest = rest.strip_prefix(')').unwrap_or(rest).trim_start();
    let rest = rest.strip_prefix(')').unwrap_or(rest).trim_start();
    let vals = if let Some(rhs) = const_eq_rhs(file_src, ident) {
        object_array_field_values(rhs, field, file_src)?
    } else {
        catalog_option_values(ident)?
    };
    Some((vals, rest))
}

fn quoted_object_const_field(file_src: &str, ident: &str, field: &str) -> Option<String> {
    let rhs = const_eq_rhs(file_src, ident)?.trim_start();
    if !rhs.starts_with('{') {
        return None;
    }
    quoted_field_in_object(rhs, field)
}

/// Quoted `field:` values from a same-file `[ { … }, … ]`. Constructor
/// elements (`row(...)`) fail closed so metadata can own those tables.
fn object_array_field_values(rhs: &str, field: &str, file_src: &str) -> Option<Vec<String>> {
    let _ = file_src;
    let s = rhs.trim_start();
    if !s.starts_with('[') {
        return None;
    }
    let end = find_matching_bracket(s, '[', ']')?;
    let mut rest = s[1..end].trim_start();
    let mut out = Vec::new();
    while !rest.is_empty() {
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
            continue;
        }
        if !rest.starts_with('{') {
            return None;
        }
        let obj_end = find_matching_bracket(rest, '{', '}')?;
        out.push(quoted_field_in_object(&rest[..=obj_end], field)?);
        rest = rest[obj_end + 1..].trim_start();
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
        } else if !rest.is_empty() {
            return None;
        }
    }
    Some(out)
}

fn quoted_field_in_object(obj: &str, field: &str) -> Option<String> {
    let s = obj.trim_start();
    if !s.starts_with('{') {
        return None;
    }
    let end = find_matching_bracket(s, '{', '}')?;
    let inner = s[1..end].trim();
    let mut rest = inner;
    while !rest.is_empty() {
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
            continue;
        }
        if rest.starts_with('[') {
            let e = find_matching_bracket(rest, '[', ']')?;
            rest = skip_object_value(rest[e + 1..].trim_start())?;
            continue;
        }
        let (key, after_key) = if let Some((quoted, n)) = scan_quoted(rest) {
            (quoted, rest[n..].trim_start())
        } else {
            let (ident, after) = take_ident(rest)?;
            (ident.to_string(), after.trim_start())
        };
        let after_colon = after_key.strip_prefix(':')?.trim_start();
        if key == field {
            let (value, _) = scan_quoted(after_colon)?;
            return Some(value);
        }
        rest = skip_object_value(after_colon)?;
    }
    None
}

fn skip_object_value(after_colon: &str) -> Option<&str> {
    let rest = skip_expression(after_colon)?;
    let rest = rest.trim_start();
    if let Some(stripped) = rest.strip_prefix(',') {
        Some(stripped.trim_start())
    } else {
        Some(rest)
    }
}

fn string_array_alias_in(src: &str, ident: &str) -> Option<String> {
    let rhs = const_eq_rhs(src, ident)?;
    if rhs.starts_with('[') {
        return None;
    }
    ident_alias_rhs(rhs)
}

/// Quoted string `const NAME = 'x'` (or a same-file alias of one).
fn resolve_string_const_ident(file_src: &str, ident: &str) -> Option<String> {
    resolve_string_const_ident_visited(file_src, ident, &mut Vec::new())
}

fn resolve_string_const_ident_visited(
    file_src: &str,
    ident: &str,
    stack: &mut Vec<String>,
) -> Option<String> {
    if stack.iter().any(|s| s == ident) {
        return None;
    }
    stack.push(ident.to_string());
    if let Some(value) = string_const_literal_in(file_src, ident) {
        return Some(value);
    }
    if let Some(alias) = string_const_alias_in(file_src, ident) {
        return resolve_string_const_ident_visited(file_src, &alias, stack);
    }
    None
}

fn string_const_literal_in(src: &str, ident: &str) -> Option<String> {
    quoted_after(const_eq_rhs(src, ident)?)
}

fn string_const_alias_in(src: &str, ident: &str) -> Option<String> {
    let rhs = const_eq_rhs(src, ident)?;
    if quoted_after(rhs).is_some() || rhs.starts_with('[') || rhs.starts_with('{') {
        return None;
    }
    ident_alias_rhs(rhs)
}

fn ident_alias_rhs(rhs: &str) -> Option<String> {
    let (alias, rest) = take_ident(rhs)?;
    if alias == "Object" || alias == "new" {
        return None;
    }
    let rest = rest.trim_start();
    if rest.starts_with('.') || rest.starts_with('(') {
        return None;
    }
    Some(alias.to_string())
}

fn object_keys_call(after: &str, file_src: &str) -> Option<Vec<String>> {
    let rest = after.strip_prefix("Object.keys(")?.trim_start();
    let (ident, _) = take_ident(rest)?;
    if let Some(keys) = object_keys_in(file_src, ident) {
        if !keys.is_empty() {
            return Some(keys);
        }
    }
    if let Some(keys) = object_keys_in(include_str!("../shim/data/spelldb.js"), ident) {
        if !keys.is_empty() {
            return Some(keys);
        }
    }
    catalog_option_values(ident)
}

fn object_keys_in(src: &str, ident: &str) -> Option<Vec<String>> {
    let rhs = const_eq_rhs(src, ident)?;
    quoted_keys_in_object(rhs)
}

fn quoted_keys_in_object(rhs: &str) -> Option<Vec<String>> {
    if !rhs.starts_with('{') {
        return None;
    }
    let end = find_matching_bracket(rhs, '{', '}')?;
    let mut rest = rhs[1..end].trim_start();
    let mut keys = Vec::new();
    while !rest.is_empty() {
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
            continue;
        }
        if rest.starts_with('[') {
            let e = find_matching_bracket(rest, '[', ']')?;
            rest = rest[e + 1..].trim_start().strip_prefix(':')?.trim_start();
            rest = skip_expression(rest)?.trim_start();
            if rest.starts_with(',') {
                rest = rest[1..].trim_start();
            }
            continue;
        }
        let (k, after_key) = if let Some((quoted, n)) = scan_quoted(rest) {
            (quoted, rest[n..].trim_start())
        } else if let Some((ident, after)) = take_ident(rest) {
            (ident.to_string(), after.trim_start())
        } else {
            break;
        };
        let after_colon = after_key.strip_prefix(':')?.trim_start();
        keys.push(k);
        rest = skip_expression(after_colon)?.trim_start();
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
        }
    }
    Some(keys)
}

fn scan_key_show_if(obj: &str, file_src: &str) -> Option<String> {
    let raw = setting_raw_value(&setting_property_value(obj, "showIf")?)?;
    if raw.starts_with('{') {
        return Some(rewrite_show_if_any_of_idents(&raw, file_src));
    }
    if let Some(rhs) = const_eq_rhs(file_src, &raw) {
        if rhs.starts_with('{') {
            if let Some(end) = find_matching_bracket(rhs, '{', '}') {
                return Some(rewrite_show_if_any_of_idents(&rhs[..=end], file_src));
            }
        }
    }
    Some(raw)
}

/// Rewrite resolvable string-const idents inside `anyOf: […]`. Unknown or
/// computed elements leave the object unchanged so the condition is not dropped
/// and is not a partial successful parse.
fn rewrite_show_if_any_of_idents(raw: &str, file_src: &str) -> String {
    let Some(any_idx) = raw.find("anyOf:") else {
        return raw.to_string();
    };
    let after_key = &raw[any_idx + "anyOf:".len()..];
    let trimmed = after_key.trim_start();
    if !trimmed.starts_with('[') {
        return raw.to_string();
    }
    let Some(end) = find_matching_bracket(trimmed, '[', ']') else {
        return raw.to_string();
    };
    let Some(new_inner) = rewrite_any_of_elements(&trimmed[1..end], file_src) else {
        return raw.to_string();
    };
    let pad = after_key.len() - trimmed.len();
    format!(
        "{}{}[{}]{}",
        &raw[..any_idx + "anyOf:".len()],
        &after_key[..pad],
        new_inner,
        &trimmed[end + 1..]
    )
}

fn rewrite_any_of_elements(inner: &str, file_src: &str) -> Option<String> {
    let mut parts = Vec::new();
    let mut rest = inner;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if let Some((value, n)) = scan_quoted(rest) {
            parts.push(format!("'{value}'"));
            rest = &rest[n..];
        } else if let Some((ident, after)) = take_ident(rest) {
            let value = resolve_string_const_ident(file_src, ident)?;
            parts.push(format!("'{value}'"));
            rest = after;
        } else {
            return None;
        }
        rest = rest.trim_start();
        if rest.starts_with(',') {
            rest = &rest[1..];
        } else if rest.is_empty() {
            break;
        } else {
            return None;
        }
    }
    Some(parts.join(", "))
}

fn parse_settings_object(
    obj: &str,
    file_src: &str,
    sources: Option<&HashMap<String, String>>,
) -> Result<Vec<SettingDef>, String> {
    let inner = obj.trim();
    let inner = inner.strip_prefix('{').unwrap_or(inner);
    let inner = inner.strip_suffix('}').unwrap_or(inner);
    let mut out = Vec::new();
    let mut rest = inner;
    while !rest.trim().is_empty() {
        rest = rest.trim_start();
        if rest.starts_with('}') || rest.is_empty() {
            break;
        }
        if let Some(after_dots) = rest.strip_prefix("...") {
            let expr = after_dots.trim_start();
            if let Some(body) = resolve_spread_settings(expr, file_src) {
                out.extend(parse_settings_object(&body, file_src, sources)?);
            }
            let Some(after_expr) = skip_expression(expr) else {
                break;
            };
            rest = after_expr.trim_start();
            if rest.starts_with(',') {
                rest = &rest[1..];
            }
            continue;
        }
        let Some(colon) = rest.find(':') else {
            break;
        };
        let id = rest[..colon]
            .trim()
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '$')
            .to_string();
        if id.is_empty() {
            break;
        }
        let after_colon = rest[colon + 1..].trim_start();
        if after_colon.starts_with('{') {
            let Some(end) = find_matching_bracket(after_colon, '{', '}') else {
                break;
            };
            out.push(parse_setting_def(
                &id,
                &after_colon[..=end],
                file_src,
                sources,
            )?);
            rest = &after_colon[end + 1..];
        } else if let Some((ident, after_ident)) = take_ident(after_colon) {
            let leftover = after_ident.trim_start();
            if leftover.starts_with('.') || leftover.starts_with('(') {
                let Some(after_expr) = skip_expression(after_colon) else {
                    break;
                };
                rest = after_expr;
            } else if let Some(body) = resolve_setting_ident(file_src, ident) {
                out.push(parse_setting_def(&id, &body, file_src, sources)?);
                rest = after_ident;
            } else {
                let Some(after_expr) = skip_expression(after_colon) else {
                    break;
                };
                rest = after_expr;
            }
        } else {
            let Some(after_expr) = skip_expression(after_colon) else {
                break;
            };
            rest = after_expr;
        }
        rest = rest.trim_start();
        if rest.starts_with(',') {
            rest = &rest[1..];
        }
    }
    Ok(out)
}

/// Bare `...IDENT` or `...Object.fromEntries(Object.entries(IDENT)…)`.
/// Unknown call expressions stay unresolved; the caller still skips them.
fn resolve_spread_settings(expr: &str, file_src: &str) -> Option<String> {
    let expr = expr.trim_start();
    if let Some((ident, after)) = take_ident(expr) {
        let after = after.trim_start();
        if !after.starts_with('.') && !after.starts_with('(') {
            return resolve_setting_ident(file_src, ident);
        }
    }
    resolve_from_entries_settings(expr, file_src)
}

fn resolve_from_entries_settings(expr: &str, file_src: &str) -> Option<String> {
    let rest = expr.strip_prefix("Object")?.trim_start();
    let rest = rest.strip_prefix('.')?.trim_start();
    let rest = rest.strip_prefix("fromEntries")?.trim_start();
    let rest = rest.strip_prefix('(')?.trim_start();
    let rest = rest.strip_prefix("Object")?.trim_start();
    let rest = rest.strip_prefix('.')?.trim_start();
    let rest = rest.strip_prefix("entries")?.trim_start();
    let rest = rest.strip_prefix('(')?.trim_start();
    let (ident, _) = take_ident(rest)?;
    resolve_setting_ident(file_src, ident)
}

/// Advance past one primary expression and its `.ident` / `(…)` / `[…]`
/// postfix so an unsupported spread cannot swallow the next field.
fn skip_expression(s: &str) -> Option<&str> {
    let mut rest = s.trim_start();
    if rest.is_empty() {
        return None;
    }
    if let Some(after_new) = rest.strip_prefix("new ") {
        rest = after_new.trim_start();
    }
    if rest.starts_with('{') {
        let end = find_matching_bracket(rest, '{', '}')?;
        rest = &rest[end + 1..];
    } else if rest.starts_with('[') {
        let end = find_matching_bracket(rest, '[', ']')?;
        rest = &rest[end + 1..];
    } else if rest.starts_with('(') {
        let end = find_matching_bracket(rest, '(', ')')?;
        rest = &rest[end + 1..];
    } else if let Some((_, n)) = scan_quoted(rest) {
        rest = &rest[n..];
    } else if let Some((_, after)) = take_ident(rest) {
        rest = after;
    } else if rest.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '.') {
        rest = rest.trim_start_matches(|c: char| {
            c.is_ascii_digit() || c == '.' || c == '-' || c == 'e' || c == 'E'
        });
    } else {
        return None;
    }
    loop {
        rest = rest.trim_start();
        if let Some(after_dot) = rest.strip_prefix('.') {
            let after_dot = after_dot.trim_start();
            let Some((_, after_ident)) = take_ident(after_dot) else {
                break;
            };
            rest = after_ident;
            continue;
        }
        if rest.starts_with('(') {
            let end = find_matching_bracket(rest, '(', ')')?;
            rest = &rest[end + 1..];
            continue;
        }
        if rest.starts_with('[') {
            let end = find_matching_bracket(rest, '[', ']')?;
            rest = &rest[end + 1..];
            continue;
        }
        break;
    }
    Some(rest)
}

fn parse_setting_def(
    id: &str,
    obj: &str,
    file_src: &str,
    sources: Option<&HashMap<String, String>>,
) -> Result<SettingDef, String> {
    let expanded = expand_setting_object(id, obj, file_src)?;
    let obj = expanded.as_str();
    let ty = setting_property_value(obj, "type")
        .and_then(|value| quoted_after(&value))
        .unwrap_or_default();
    if ty.is_empty() {
        return Err(format!("setting '{id}': missing setting type"));
    }
    let options_value = setting_property_value(obj, "options");
    let options_ident = options_value.as_deref().and_then(options_ident);
    let mixed = options_value
        .as_deref()
        .and_then(revision_fact_spread_options);
    let options = mixed
        .as_ref()
        .map(|(_, values)| values.clone())
        .unwrap_or_else(|| {
            options_value
                .as_deref()
                .map(|value| scan_setting_options(value, file_src, sources))
                .unwrap_or_default()
        });
    let item_option_spec = if options.is_empty() && mixed.is_none() {
        options_ident.and_then(|ident| scan_item_option_spec(ident, file_src))
    } else {
        None
    };
    let options_from = mixed
        .as_ref()
        .map(|(from, _)| from.clone())
        .or_else(|| {
            setting_property_value(obj, "optionsFrom")
                .and_then(|value| parse_setting_identifier(&value))
        })
        .or_else(|| inferred_options_from(options_ident, &options, item_option_spec.as_ref()));
    Ok(SettingDef {
        id: id.to_string(),
        ty,
        default: setting_property_value(obj, "default")
            .and_then(|value| scan_setting_literal(&value, Some(file_src))),
        label: setting_property_value(obj, "label").and_then(|value| quoted_after(&value)),
        min: setting_property_value(obj, "min")
            .and_then(|value| scan_setting_literal(&value, None)),
        max: setting_property_value(obj, "max")
            .and_then(|value| scan_setting_literal(&value, None)),
        step: setting_property_value(obj, "step")
            .and_then(|value| scan_setting_literal(&value, None)),
        options,
        option_labels: setting_property_value(obj, "optionLabels")
            .map(|value| scan_setting_option_labels(&value, file_src))
            .unwrap_or_default(),
        group: setting_property_value(obj, "group").and_then(|value| quoted_after(&value)),
        show_if: scan_key_show_if(obj, file_src),
        options_from,
        csv_toggle: setting_property_value(obj, "csvToggle")
            .and_then(|value| setting_raw_value(&value)),
        help: setting_property_value(obj, "help").and_then(|value| quoted_after(&value)),
        item_option_spec,
    })
}

/// Expand `...IDENT` spreads inside a setting definition `{ … }` in
/// left-to-right declaration order so later keys override spread keys and
/// later spreads override earlier keys (JS object-spread semantics).
/// Fails closed on any unresolvable or cyclic spread.
fn expand_setting_object(id: &str, obj: &str, file_src: &str) -> Result<String, String> {
    let mut entries = Vec::new();
    let mut visited = Vec::new();
    collect_setting_entries(id, obj, file_src, &mut visited, &mut entries)?;
    let joined = entries
        .into_iter()
        .map(|(_, raw_kv)| raw_kv)
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!("{{ {joined} }}"))
}

fn collect_setting_entries(
    id: &str,
    obj: &str,
    file_src: &str,
    visited: &mut Vec<String>,
    entries: &mut Vec<(String, String)>,
) -> Result<(), String> {
    let raw_entries = split_top_level_object_entries(obj)
        .ok_or_else(|| format!("setting '{id}': malformed setting object"))?;
    for raw_entry in raw_entries {
        let entry = raw_entry.trim();
        if entry.is_empty() {
            continue;
        }
        if let Some(after_dots) = entry.strip_prefix("...") {
            let spread_expr = after_dots.trim();
            let Some((ident, rest)) = take_ident(spread_expr) else {
                return Err(format!(
                    "setting '{id}': unresolvable spread '...{spread_expr}'"
                ));
            };
            if !rest.trim().is_empty() {
                return Err(format!(
                    "setting '{id}': unresolvable spread '...{spread_expr}'"
                ));
            }
            if matches!(ident, "PERIODIC_BANK_SETTINGS" | "CUSTOM_RANGED_SETTINGS") {
                return Err(format!("setting '{id}': unresolvable spread '...{ident}'"));
            }
            if visited.iter().any(|seen| seen == ident) {
                return Err(format!("setting '{id}': cyclic spread '...{ident}'"));
            }
            let Some(spread_body) = resolve_setting_ident(file_src, ident) else {
                return Err(format!("setting '{id}': unresolvable spread '...{ident}'"));
            };
            visited.push(ident.to_string());
            collect_setting_entries(id, &spread_body, file_src, visited, entries)?;
            visited.pop();
            continue;
        }
        let (key, after_key) = if let Some((quoted, n)) = scan_quoted(entry) {
            (quoted, &entry[n..])
        } else if let Some((ident, after)) = take_ident(entry) {
            (ident.to_string(), after)
        } else {
            return Err(format!("setting '{id}': malformed property '{entry}'"));
        };
        let Some(after_colon) = after_key.trim_start().strip_prefix(':') else {
            return Err(format!("setting '{id}': malformed property '{entry}'"));
        };
        let val_slice = after_colon.trim();
        upsert_setting_entry(entries, key.clone(), format!("{key}: {val_slice}"));
    }
    Ok(())
}

fn upsert_setting_entry(entries: &mut Vec<(String, String)>, key: String, raw_kv: String) {
    if let Some(pos) = entries.iter().position(|(k, _)| k == &key) {
        entries.remove(pos);
    }
    entries.push((key, raw_kv));
}

fn split_top_level_object_entries(obj: &str) -> Option<Vec<String>> {
    let trimmed = obj.trim();
    let inner = trimmed.strip_prefix('{')?.strip_suffix('}')?;
    let mut out = Vec::new();
    let mut current = String::new();
    let mut brace_depth = 0i32;
    let mut bracket_depth = 0i32;
    let mut paren_depth = 0i32;
    let mut in_str: Option<char> = None;
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(q) = in_str {
            current.push(c);
            if c == '\\' {
                let escaped = chars.next()?;
                current.push(escaped);
            } else if c == q {
                in_str = None;
            }
            continue;
        }
        if c == '/' && chars.peek() == Some(&'/') {
            chars.next();
            for nc in chars.by_ref() {
                if nc == '\n' {
                    current.push('\n');
                    break;
                }
            }
            continue;
        }
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut closed = false;
            while let Some(nc) = chars.next() {
                if nc == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    current.push(' ');
                    closed = true;
                    break;
                }
            }
            if !closed {
                return None;
            }
            continue;
        }
        match c {
            '\'' | '"' | '`' => {
                in_str = Some(c);
                current.push(c);
            }
            '{' => {
                brace_depth += 1;
                current.push(c);
            }
            '}' => {
                brace_depth -= 1;
                if brace_depth < 0 {
                    return None;
                }
                current.push(c);
            }
            '[' => {
                bracket_depth += 1;
                current.push(c);
            }
            ']' => {
                bracket_depth -= 1;
                if bracket_depth < 0 {
                    return None;
                }
                current.push(c);
            }
            '(' => {
                paren_depth += 1;
                current.push(c);
            }
            ')' => {
                paren_depth -= 1;
                if paren_depth < 0 {
                    return None;
                }
                current.push(c);
            }
            ',' if brace_depth == 0 && bracket_depth == 0 && paren_depth == 0 => {
                let entry = current.trim();
                if !entry.is_empty() {
                    out.push(entry.to_string());
                }
                current.clear();
            }
            _ => current.push(c),
        }
    }
    if in_str.is_some() || brace_depth != 0 || bracket_depth != 0 || paren_depth != 0 {
        return None;
    }
    let tail = current.trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    Some(out)
}

fn setting_property_value(obj: &str, key: &str) -> Option<String> {
    for entry in split_top_level_object_entries(obj)? {
        let entry = entry.trim();
        let (entry_key, after_key) = if let Some((quoted, n)) = scan_quoted(entry) {
            (quoted, &entry[n..])
        } else if let Some((ident, after)) = take_ident(entry) {
            (ident.to_string(), after)
        } else {
            continue;
        };
        if entry_key != key {
            continue;
        }
        return after_key
            .trim_start()
            .strip_prefix(':')
            .map(|value| value.trim().to_string());
    }
    None
}

fn scan_setting_literal(value: &str, file_src: Option<&str>) -> Option<String> {
    let after = value.trim_start();
    if let Some(value) = quoted_after(after) {
        return Some(value);
    }
    if after.starts_with("true") {
        return Some("true".to_string());
    }
    if after.starts_with("false") {
        return Some("false".to_string());
    }
    let num: String = after
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();
    if !num.is_empty() {
        return Some(num);
    }
    if let Some(src) = file_src {
        if let Some((ident, _)) = take_ident(after) {
            if let Some(value) = resolve_string_const_ident(src, ident) {
                return Some(value);
            }
            if let Some(arr) = resolve_string_array_ident(src, ident) {
                return Some(serde_json::to_string(&arr).expect("string array default json"));
            }
        }
    }
    None
}

fn setting_raw_value(value: &str) -> Option<String> {
    let value = value.trim_start();
    if let Some(value) = quoted_after(value) {
        return Some(value);
    }
    if value.starts_with('{') {
        let end = find_matching_bracket(value, '{', '}')?;
        return Some(value[..=end].to_string());
    }
    if value.starts_with('[') {
        let end = find_matching_bracket(value, '[', ']')?;
        return Some(value[..=end].to_string());
    }
    let atom = value
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
        .collect::<String>();
    (!atom.is_empty()).then_some(atom)
}

fn parse_setting_identifier(value: &str) -> Option<String> {
    let value = value.trim_start();
    quoted_after(value).or_else(|| take_ident(value).map(|(ident, _)| ident.to_string()))
}

/// Frozen `FireGiant.ts:78-81` and the other ranged cards combine selected
/// equipment families with a literal `Other`. Keep source order and fail
/// closed on an unknown spread rather than publishing only the literal tail.
fn revision_fact_spread_options(raw: &str) -> Option<(String, Vec<String>)> {
    let inner = raw.strip_prefix('[')?.strip_suffix(']')?;
    if !inner.contains("...") {
        return None;
    }
    let mut families = Vec::new();
    let mut literals = Vec::new();
    for item in inner.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(ident) = item.strip_prefix("...") {
            w1c_equipment_option_families(ident)?;
            families.push(ident);
            if !literals.is_empty() {
                return None;
            }
        } else if let Some((value, n)) = scan_quoted(item) {
            if n != item.len() {
                return None;
            }
            literals.push(value);
        } else {
            return None;
        }
    }
    if families.is_empty() {
        return None;
    }
    Some((families.join(","), literals))
}

fn inferred_options_from(
    options_ident: Option<&str>,
    options: &[String],
    item_option_spec: Option<&ItemOptionSpec>,
) -> Option<String> {
    if !options.is_empty() || item_option_spec.is_some() {
        return None;
    }
    let ident = options_ident?;
    if is_revision_fact_option_ident(ident) || catalog_option_table(ident).is_some() {
        return Some(ident.to_string());
    }
    None
}
fn scan_setting_options(
    value: &str,
    file_src: &str,
    sources: Option<&HashMap<String, String>>,
) -> Vec<String> {
    let value = value.trim_start();
    if let Some(arr) = parse_string_array_with_consts(value, file_src) {
        return arr;
    }
    if let Some(arr) = parse_string_array(value) {
        return arr;
    }
    if let Some(keys) = object_keys_call(value, file_src) {
        return keys;
    }
    if let Some((ident, leftover)) = take_ident(value) {
        let leftover = leftover.trim_start();
        if leftover.starts_with(".map") {
            return take_map_call(ident, leftover, file_src)
                .map(|(vals, _)| vals)
                .unwrap_or_default();
        }
        if ident == "presetBuyableNames" {
            if take_empty_call_expression(leftover).is_some() {
                return resolve_preset_buyable_names(file_src, sources).unwrap_or_default();
            }
            return Vec::new();
        }
        if leftover.starts_with('(') || leftover.starts_with('.') {
            return Vec::new();
        }
        if is_revision_fact_option_ident(ident) {
            return Vec::new();
        }
        return resolve_string_array_ident(file_src, ident).unwrap_or_default();
    }
    Vec::new()
}

fn scan_setting_option_labels(value: &str, file_src: &str) -> Vec<String> {
    let value = value.trim_start();
    if let Some(labels) = parse_string_array(value) {
        return labels;
    }
    if let Some((ident, _)) = take_ident(value) {
        if let Some(rhs) = const_eq_rhs(file_src, ident) {
            if let Some(labels) = parse_string_record(rhs, file_src) {
                return labels;
            }
        }
    }
    Vec::new()
}

/// Values from a static `Record<string, string>` object, in declaration order.
/// Computed keys/values are accepted only when they resolve to string consts.
fn parse_string_record(rhs: &str, file_src: &str) -> Option<Vec<String>> {
    let s = rhs.trim_start();
    if !s.starts_with('{') {
        return None;
    }
    let end = find_matching_bracket(s, '{', '}')?;
    let mut rest = s[1..end].trim_start();
    let mut out = Vec::new();
    while !rest.is_empty() {
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
            continue;
        }
        let (_key, after_key) = if let Some((key, n)) = scan_quoted(rest) {
            (key, &rest[n..])
        } else if let Some(after_open) = rest.strip_prefix('[') {
            let (ident, after_ident) = take_ident(after_open.trim_start())?;
            let after_ident = after_ident.trim_start();
            let after_close = after_ident.strip_prefix(']')?.trim_start();
            (resolve_string_const_ident(file_src, ident)?, after_close)
        } else {
            let (key, after) = take_ident(rest)?;
            (key.to_string(), after)
        };
        let after_key = after_key.trim_start().strip_prefix(':')?.trim_start();
        let (value, n) = if let Some((value, n)) = scan_quoted(after_key) {
            (value, n)
        } else {
            let (ident, after) = take_ident(after_key)?;
            let value = resolve_string_const_ident(file_src, ident)?;
            (value, after_key.len() - after.len())
        };
        out.push(value);
        rest = after_key[n..].trim_start();
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
        } else if !rest.is_empty() {
            return None;
        }
    }
    Some(out)
}

/// Recognized high-alchemy `options: IDENT` whose sibling RHS is
/// `[prefix, ...items.map(i => i.key)]` and `items` is the FODDER.flatMap
/// / ITEM_DB / Math.floor(cost * 0.6) / sort chain. Unknown maps stay
/// unresolved. Never evaluates TypeScript.
fn scan_item_option_spec(ident: &str, file_src: &str) -> Option<ItemOptionSpec> {
    let rhs = const_eq_rhs(file_src, ident)?;
    let (prefix_token, items_ident, sort_keys_by_label) = parse_alch_options_rhs(rhs)?;
    let prefix = match prefix_token {
        AlchPrefix::Quoted(value) => value,
        AlchPrefix::Ident(name) => resolve_string_const_ident(file_src, &name)?,
    };
    if prefix.is_empty() {
        return None;
    }
    let items_rhs = const_eq_rhs(file_src, &items_ident)?;
    let fodder_ident = high_alchemy_fodder_ident(file_src, items_rhs)?;
    let fodder_rhs = const_eq_rhs(file_src, fodder_ident)?;
    let candidates = parse_fodder_candidates(fodder_rhs)?;
    Some(ItemOptionSpec {
        prefix: vec![prefix],
        candidates,
        sort_keys_by_label,
    })
}

fn options_ident(value: &str) -> Option<&str> {
    let value = value.trim_start();
    if value.starts_with('[') || value.starts_with("Object.keys(") {
        return None;
    }
    let (ident, _) = take_ident(value)?;
    Some(ident)
}

enum AlchPrefix {
    Quoted(String),
    Ident(String),
}

/// `[PREFIX, ...ITEMS.map(param => param.key)]` or the frozen sort-wrapper
/// spread `[PREFIX, ...[...ITEMS].sort((a,b)=>a.label.localeCompare(b.label)).map(i=>i.key)]`.
fn parse_alch_options_rhs(rhs: &str) -> Option<(AlchPrefix, String, bool)> {
    let s = rhs.trim_start();
    if !s.starts_with('[') {
        return None;
    }
    let end = find_matching_bracket(s, '[', ']')?;
    let mut inner = s[1..end].trim_start();
    let prefix = if let Some((value, n)) = scan_quoted(inner) {
        inner = inner[n..].trim_start();
        AlchPrefix::Quoted(value)
    } else {
        let (ident, after) = take_ident(inner)?;
        inner = after.trim_start();
        AlchPrefix::Ident(ident.to_string())
    };
    inner = inner.strip_prefix(',')?.trim_start();
    inner = inner.strip_prefix("...")?.trim_start();
    let (items, mut inner) = if inner.starts_with('[') {
        let wrap_end = find_matching_bracket(inner, '[', ']')?;
        let inside = inner[1..wrap_end].trim_start();
        let inside = inside.strip_prefix("...")?.trim_start();
        let (items, after_items) = take_ident(inside)?;
        if !after_items.trim().is_empty() {
            return None;
        }
        (items.to_string(), &inner[wrap_end + 1..])
    } else {
        let (items, after) = take_ident(inner)?;
        (items.to_string(), after)
    };
    inner = inner.trim_start();
    let mut sort_keys_by_label = false;
    if let Some(after_sort) = skip_bounded_alch_label_sort(inner) {
        sort_keys_by_label = true;
        inner = after_sort;
    }
    inner = inner.strip_prefix('.')?.trim_start();
    inner = inner.strip_prefix("map")?.trim_start();
    inner = inner.strip_prefix('(')?.trim_start();
    let (param, after) = if let Some(after_paren) = inner.strip_prefix('(') {
        let after_paren = after_paren.trim_start();
        let (param, after) = take_ident(after_paren)?;
        let after = after.trim_start().strip_prefix(')')?.trim_start();
        (param, after)
    } else {
        let (param, after) = take_ident(inner)?;
        (param, after.trim_start())
    };
    inner = after;
    inner = inner.strip_prefix("=>")?.trim_start();
    let (recv, after) = take_ident(inner)?;
    if recv != param {
        return None;
    }
    inner = after.trim_start();
    inner = inner.strip_prefix('.')?.trim_start();
    inner = inner.strip_prefix("key")?.trim_start();
    inner = inner.strip_prefix(')')?.trim_start();
    if !inner.is_empty() {
        return None;
    }
    if !value_expression_complete(&s[end + 1..]) {
        return None;
    }
    Some((prefix, items, sort_keys_by_label))
}

/// Frozen high-alchemy chip order: `.sort((a,b)=>a.label.localeCompare(b.label))`.
fn skip_bounded_alch_label_sort(after: &str) -> Option<&str> {
    let rest = after.strip_prefix('.')?.trim_start();
    let rest = rest.strip_prefix("sort")?.trim_start();
    let rest = rest.strip_prefix('(')?.trim_start();
    let rest = rest.strip_prefix('(')?.trim_start();
    let (a, rest) = take_ident(rest)?;
    let rest = rest.trim_start().strip_prefix(',')?.trim_start();
    let (b, rest) = take_ident(rest)?;
    let rest = rest.trim_start().strip_prefix(')')?.trim_start();
    let rest = rest.strip_prefix("=>")?.trim_start();
    let (lhs, rest) = take_ident(rest)?;
    if lhs != a {
        return None;
    }
    let rest = rest.strip_prefix('.')?.trim_start();
    let (field, rest) = take_ident(rest)?;
    if field != "label" {
        return None;
    }
    let rest = rest.strip_prefix('.')?.trim_start();
    let rest = rest.strip_prefix("localeCompare")?.trim_start();
    let rest = rest.strip_prefix('(')?.trim_start();
    let (cmp_recv, rest) = take_ident(rest)?;
    let rest = rest.strip_prefix('.')?.trim_start();
    let (cmp_field, rest) = take_ident(rest)?;
    if cmp_recv != b || cmp_field != "label" {
        return None;
    }
    let rest = rest.strip_prefix(')')?.trim_start();
    let rest = rest.strip_prefix(')')?.trim_start();
    Some(rest)
}

/// `presetBuyableNames()` with no args and no trailing expression suffix.
fn take_empty_call_expression(after: &str) -> Option<&str> {
    let rest = after.trim_start().strip_prefix('(')?.trim_start();
    let rest = rest.strip_prefix(')')?;
    if value_expression_complete(rest) {
        Some(rest)
    } else {
        None
    }
}

/// Remainder after a finished value: empty, terminator, or the next declaration.
fn value_expression_complete(after: &str) -> bool {
    let rest = after.trim_start();
    if rest.is_empty() || rest.starts_with(';') || rest.starts_with(',') || rest.starts_with('}') {
        return true;
    }
    rest.starts_with("export")
        || rest.starts_with("const ")
        || rest.starts_with("function ")
        || rest.starts_with("let ")
        || rest.starts_with("var ")
        || rest.starts_with("import")
}

struct ShopDbRecord {
    keepers: Vec<String>,
    item_names: Vec<String>,
}

/// `presetBuyableNames()` from frozen `shopPresets.ts` × parsed `SHOP_DB`.
fn resolve_preset_buyable_names(
    file_src: &str,
    sources: Option<&HashMap<String, String>>,
) -> Option<Vec<String>> {
    let presets_rhs = const_eq_rhs(file_src, "SHOP_PRESETS")?;
    let preset_keepers = object_array_field_values(presets_rhs, "keeper", file_src)?;
    let shop_db_src = shop_db_source_text(sources, file_src)?;
    let records = parse_shop_db_records(&shop_db_src)?;
    if records.is_empty() || preset_keepers.is_empty() {
        return None;
    }
    let mut names = std::collections::BTreeSet::new();
    for keeper in preset_keepers {
        let Some(rec) = records
            .iter()
            .find(|r| r.keepers.iter().any(|k| k == &keeper))
        else {
            continue;
        };
        for name in &rec.item_names {
            names.insert(name.clone());
        }
    }
    if names.is_empty() {
        return None;
    }
    Some(names.into_iter().collect())
}

const SHOP_DB_SOURCE_KEYS: [&str; 2] = ["./src/bot/data/shopdb.ts", "./src/bot/data/shopdb.js"];

#[cfg(feature = "load")]
/// Load `SHOP_DB` from the registered catalog root at the known data path.
pub(crate) fn insert_shop_db_from_root(sources: &mut HashMap<String, String>, root: &Path) {
    let ts = root.join("src/bot/data/shopdb.ts");
    let js = root.join("src/bot/data/shopdb.js");
    let text = std::fs::read_to_string(&ts)
        .ok()
        .or_else(|| std::fs::read_to_string(&js).ok());
    let Some(text) = text else {
        return;
    };
    sources
        .entry(SHOP_DB_SOURCE_KEYS[0].into())
        .or_insert_with(|| text.clone());
    sources.entry(SHOP_DB_SOURCE_KEYS[1].into()).or_insert(text);
}

fn shop_db_source_text(
    sources: Option<&HashMap<String, String>>,
    file_src: &str,
) -> Option<String> {
    if let Some(map) = sources {
        for key in SHOP_DB_SOURCE_KEYS {
            if let Some(text) = lookup_source(map, key) {
                return Some(text);
            }
        }
        if const_eq_rhs(file_src, "SHOP_DB").is_some() {
            return Some(file_src.to_string());
        }
        return None;
    }
    if const_eq_rhs(file_src, "SHOP_DB").is_some() {
        return Some(file_src.to_string());
    }
    None
}

fn parse_shop_db_records(src: &str) -> Option<Vec<ShopDbRecord>> {
    let rhs = const_eq_rhs(src, "SHOP_DB")?;
    let s = rhs.trim_start();
    if !s.starts_with('{') {
        return None;
    }
    let end = find_matching_bracket(s, '{', '}')?;
    let mut rest = s[1..end].trim_start();
    let mut out = Vec::new();
    while !rest.is_empty() {
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
            continue;
        }
        let (_, key_end) = scan_quoted(rest)?;
        let after_key = &rest[key_end..];
        let after_colon = after_key.trim_start().strip_prefix(':')?.trim_start();
        if !after_colon.starts_with('{') {
            return None;
        }
        let obj_end = find_matching_bracket(after_colon, '{', '}')?;
        let record_obj = &after_colon[..=obj_end];
        out.push(parse_one_shop_db_record(record_obj)?);
        rest = after_colon[obj_end + 1..].trim_start();
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
        } else if !rest.is_empty() {
            return None;
        }
    }
    Some(out)
}

fn parse_one_shop_db_record(obj: &str) -> Option<ShopDbRecord> {
    let keepers = parse_json_string_array_field(obj, "keepers")?;
    let item_names = parse_shop_items_names(obj)?;
    Some(ShopDbRecord {
        keepers,
        item_names,
    })
}

fn parse_json_string_array_field(obj: &str, field: &str) -> Option<Vec<String>> {
    let needle = format!("\"{field}\":");
    let idx = obj.find(&needle)?;
    let after = obj[idx + needle.len()..].trim_start();
    if !after.starts_with('[') {
        return None;
    }
    let end = find_matching_bracket(after, '[', ']')?;
    parse_string_array(&after[..=end])
}

fn parse_shop_items_names(obj: &str) -> Option<Vec<String>> {
    let needle = "\"items\":";
    let idx = obj.find(needle)?;
    let after = obj[idx + needle.len()..].trim_start();
    if !after.starts_with('[') {
        return None;
    }
    let end = find_matching_bracket(after, '[', ']')?;
    let mut rest = after[1..end].trim_start();
    let mut out = Vec::new();
    while !rest.is_empty() {
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
            continue;
        }
        if !rest.starts_with('{') {
            return None;
        }
        let obj_end = find_matching_bracket(rest, '{', '}')?;
        if let Some(name) = quoted_field_in_object(&rest[..=obj_end], "name") {
            out.push(name);
        }
        rest = rest[obj_end + 1..].trim_start();
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
        } else if !rest.is_empty() {
            return None;
        }
    }
    Some(out)
}

/// `FODDER.flatMap(... ITEM_DB.find ... Math.floor(rec.cost * ALCH_RATE|0.6) ...).sort(...)`.
fn high_alchemy_fodder_ident<'a>(file_src: &str, rhs: &'a str) -> Option<&'a str> {
    let rhs = rhs.trim_start();
    let (fodder, rest) = take_ident(rhs)?;
    if fodder == "ITEM_DB" || fodder == "Object" {
        return None;
    }
    if !rest.trim_start().starts_with('.') {
        return None;
    }
    let compact: String = rhs.chars().filter(|c| !c.is_whitespace()).collect();
    if !compact.contains(".flatMap(") {
        return None;
    }
    if !compact.contains("ITEM_DB.find") {
        return None;
    }
    if !compact.contains(".obj===") && !compact.contains(".obj==") {
        return None;
    }
    if !compact.contains("Math.floor(") && !compact.contains("alchValueOf(") {
        return None;
    }
    if !compact.contains(".sort(") {
        return None;
    }
    if !flat_map_mentions_obj(&compact) {
        return None;
    }
    if !high_alchemy_rate_operand(file_src, &compact)
        && !high_alchemy_alch_value_of(&compact, file_src)
    {
        return None;
    }
    Some(fodder)
}

/// Frozen `AlcherLogic.ts` uses `alchValueOf(rec.cost)` (default `HIGH_ALCH_RATE`).
fn high_alchemy_alch_value_of(compact: &str, file_src: &str) -> bool {
    if !compact.contains("alchValueOf(rec.cost") {
        return false;
    }
    if compact.contains("alchValueOf(rec.cost,HIGH_ALCH_RATE)") {
        let Some(rhs) = const_eq_rhs(file_src, "HIGH_ALCH_RATE") else {
            return false;
        };
        return number_is_point_six(rhs.trim_start());
    }
    if compact.contains("alchValueOf(rec.cost)") {
        let Some(rhs) = const_eq_rhs(file_src, "HIGH_ALCH_RATE") else {
            return const_eq_rhs(file_src, "ALCH_RATE")
                .map(|r| number_is_point_six(r.trim_start()))
                .unwrap_or(false);
        };
        return number_is_point_six(rhs.trim_start());
    }
    false
}

fn flat_map_mentions_obj(compact: &str) -> bool {
    let Some(start) = compact.find(".flatMap(") else {
        return false;
    };
    let after = &compact[start + ".flatMap(".len()..];
    after.contains("obj")
}

fn high_alchemy_rate_operand(file_src: &str, compact: &str) -> bool {
    if compact.contains(".cost*0.6") {
        return rate_token_is_point_six(compact);
    }
    if compact.contains(".cost*ALCH_RATE") {
        let Some(rhs) = const_eq_rhs(file_src, "ALCH_RATE") else {
            return false;
        };
        return number_is_point_six(rhs.trim_start());
    }
    false
}

fn rate_token_is_point_six(compact: &str) -> bool {
    let Some(idx) = compact.find(".cost*0.6") else {
        return false;
    };
    number_is_point_six(&compact[idx + ".cost*".len()..])
}

fn number_is_point_six(s: &str) -> bool {
    let s = s.trim_start();
    let Some(rest) = s.strip_prefix("0.6") else {
        return false;
    };
    !rest.starts_with(|c: char| c.is_ascii_digit())
}

fn parse_fodder_candidates(rhs: &str) -> Option<Vec<ItemOptionCandidate>> {
    let s = rhs.trim_start();
    if !s.starts_with('[') {
        return None;
    }
    let end = find_matching_bracket(s, '[', ']')?;
    let mut rest = s[1..end].trim_start();
    let mut out = Vec::new();
    while !rest.is_empty() {
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
            continue;
        }
        if !rest.starts_with('{') {
            return None;
        }
        let obj_end = find_matching_bracket(rest, '{', '}')?;
        out.push(parse_fodder_object(&rest[..=obj_end])?);
        rest = rest[obj_end + 1..].trim_start();
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
        } else if rest.is_empty() {
            break;
        } else {
            return None;
        }
    }
    Some(out)
}

fn parse_fodder_object(obj: &str) -> Option<ItemOptionCandidate> {
    let inner = obj.trim();
    let inner = inner.strip_prefix('{')?;
    let inner = inner.strip_suffix('}')?;
    let mut rest = inner.trim();
    let mut key = None;
    let mut label = None;
    while !rest.is_empty() {
        let (ident, after) = take_ident(rest)?;
        let after = after.trim_start().strip_prefix(':')?.trim_start();
        let (value, n) = scan_quoted(after)?;
        match ident {
            "obj" if key.is_none() => key = Some(value),
            "label" if label.is_none() => label = Some(value),
            _ => return None,
        }
        rest = after[n..].trim_start();
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
        } else if rest.is_empty() {
            break;
        } else {
            return None;
        }
    }
    Some(ItemOptionCandidate {
        key: key.filter(|k| !k.is_empty())?,
        label,
    })
}
